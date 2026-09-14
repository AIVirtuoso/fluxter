// fluxter-phone carries the sound of a Fluxer voice call for fluxter.
//
// fluxter joins, leaves, mutes and keeps the bookkeeping; this program
// is what it names in `[media] voice_command`, and it does one thing:
// given the LiveKit address and the grant the server issued, it sends
// the microphone into the room and plays what the others say.
//
// The sound itself goes through programs on PATH, in the same spirit as
// the rest of fluxter: ffmpeg captures the microphone as Ogg Opus, and
// ffplay (or mpv) plays each remote voice from a pipe. Either can be
// replaced with FLUXTER_PHONE_MIC and FLUXTER_PHONE_PLAYER.
//
//	fluxter-phone URL TOKEN [KEY]
//
// KEY is the channel's end-to-end key where it has one; the frames are
// then encrypted and decrypted the way the web client does it.
//
// Standard input is the control line: `mute`, `unmute`, `deafen`,
// `undeafen`, `video`, `novideo`, `screen`, `noscreen` and `quit`, one
// per line; end of input quits as well. `screen` shares a screen or a
// window: the desktop portal's own chooser picks which, and GStreamer
// encodes the PipeWire stream it hands back (FLUXTER_PHONE_SCREEN
// replaces that command).
//
//	fluxter-phone screen-test SECONDS FILE
//
// tries the screen capture on its own, outside any call, writing raw
// H.264 to FILE. Video (cameras and screen shares) is not subscribed to
// until `video` is asked for; each track then gets a window of its own
// through ffplay or mpv, replaceable with FLUXTER_PHONE_VIDEO_PLAYER.
// Standard output reports what happens, one line each, and never the
// token or the key.
package main

import (
	"bufio"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/livekit/protocol/livekit"
	lksdk "github.com/livekit/server-sdk-go/v2"
	"github.com/livekit/server-sdk-go/v2/e2ee"
	e2eetypes "github.com/livekit/server-sdk-go/v2/e2ee/types"
	"github.com/pion/rtp"
	"github.com/pion/webrtc/v4"
	"github.com/pion/webrtc/v4/pkg/media/h264writer"
	"github.com/pion/webrtc/v4/pkg/media/ivfwriter"
	"github.com/pion/webrtc/v4/pkg/media/oggwriter"
)

// The microphone, as Ogg Opus on standard output: 48 kHz mono at the
// voice profile, 20 ms frames one to a page so they can be sent as they
// come. `{capture}` is pulse (which PipeWire answers to) or alsa.
var defaultMic = []string{
	"ffmpeg", "-loglevel", "error", "-nostdin",
	"-f", "{capture}", "-i", "default",
	"-ac", "1", "-ar", "48000",
	"-c:a", "libopus", "-b:a", "32k", "-application", "voip",
	"-frame_duration", "20", "-page_duration", "20000",
	"-f", "ogg", "pipe:1",
}

// Players tried in turn, each reading Ogg Opus from standard input with
// as little buffering as it allows.
var defaultPlayers = [][]string{
	{"ffplay", "-nodisp", "-autoexit", "-loglevel", "error",
		"-fflags", "nobuffer", "-flags", "low_delay", "-i", "pipe:0"},
	{"mpv", "--no-terminal", "--no-video", "--really-quiet",
		"--cache=no", "--demuxer-lavf-o=fflags=+nobuffer", "-"},
}

// Video players tried in turn, each reading a stream from standard
// input: `{format}` is `ivf` for VP8, VP9 and AV1 and `h264` for H.264,
// `{title}` names the window after who is showing what.
// Probing is turned down to nothing: a live pipe has no history to read
// ahead in, and a raw stream would otherwise wait for several seconds of
// it before the first picture.
var defaultVideoPlayers = [][]string{
	{"ffplay", "-loglevel", "error", "-fflags", "nobuffer", "-flags", "low_delay", "-framedrop",
		"-probesize", "32", "-analyzeduration", "0",
		"-window_title", "{title}", "-f", "{format}", "-i", "pipe:0"},
	{"mpv", "--no-terminal", "--really-quiet", "--profile=low-latency",
		"--demuxer-lavf-probesize=32", "--demuxer-lavf-analyzeduration=0",
		"--title={title}", "--demuxer-lavf-format={format}", "-"},
}

// The screen capture: the portal's PipeWire stream, given as file
// descriptor `{fd}` and node `{node}`, encoded to raw H.264 on standard
// output.
//
// Three things about the shape of it. The portal sends a frame only when
// the screen changes, so the rate is made a steady 15 a second first (a
// still screen repeats its last frame, which costs the encoder next to
// nothing), and a keyframe comes every second, so a viewer who arrives
// late, or the preview, sees a picture within one. The picture is scaled
// to fit 1280x720, which is what the server admits from an ordinary
// account, and a fraction of the encoding work of a whole desktop. And
// every frame is one slice, with `sliced-threads=false`: the LiveKit SDK
// sends each H.264 slice as a frame, paced 33 ms apart whatever the
// real rate, so five slices a frame would go out five times too slowly
// and the delay would grow without end. That also caps the rate at 30.
var defaultScreen = []string{
	"gst-launch-1.0", "-q",
	"pipewiresrc", "fd={fd}", "path={node}", "do-timestamp=true",
	"!", "queue", "max-size-buffers=2", "leaky=downstream",
	"!", "videorate", "!", "video/x-raw,framerate=15/1",
	"!", "videoscale", "!", "video/x-raw,width=1280,height=720,pixel-aspect-ratio=1/1",
	"!", "videoconvert", "!", "video/x-raw,format=I420",
	"!", "x264enc", "tune=zerolatency", "speed-preset=ultrafast", "sliced-threads=false", "threads=1",
	"key-int-max=15", "bitrate=2000",
	"!", "video/x-h264,stream-format=byte-stream,profile=baseline",
	"!", "fdsink", "fd=1",
}

func main() {
	if len(os.Args) == 4 && os.Args[1] == "screen-test" {
		screenTest(os.Args[2], os.Args[3])
		return
	}
	if len(os.Args) < 3 || len(os.Args) > 4 {
		fmt.Fprintln(os.Stderr, "usage: fluxter-phone URL TOKEN [KEY]")
		os.Exit(2)
	}
	url, token := os.Args[1], os.Args[2]
	key := ""
	if len(os.Args) == 4 {
		key = os.Args[3]
	}

	p := &phone{players: map[string]*player{}}
	if key != "" {
		kp := e2ee.NewExternalKeyProvider()
		if err := kp.SetKeyFromPassphrase(key, 0); err != nil {
			fail("end-to-end key: %v", err)
		}
		p.keys = kp
		say("end-to-end encryption on")
	}

	mic, err := micCommand()
	if err != nil {
		say("no microphone: %v", err)
	}

	room, err := lksdk.ConnectToRoomWithToken(url, token, p.callbacks(), lksdk.WithAutoSubscribe(true))
	if err != nil {
		fail("could not connect: %v", err)
	}
	p.room = room
	say("connected to the room")

	if mic != nil {
		if err := p.publishMicrophone(mic); err != nil {
			say("microphone not published: %v", err)
		}
	}

	// the two ways out: fluxter closing the control line, or a signal
	quit := make(chan struct{})
	var once sync.Once
	stop := func() { once.Do(func() { close(quit) }) }
	p.onLeft = stop
	go p.control(os.Stdin, stop)
	sigs := make(chan os.Signal, 1)
	signal.Notify(sigs, syscall.SIGINT, syscall.SIGTERM, syscall.SIGHUP)
	select {
	case <-quit:
	case <-sigs:
	}
	p.shutdown()
}

type phone struct {
	room   *lksdk.Room
	keys   e2eetypes.KeyProvider
	onLeft func()

	mic    *exec.Cmd
	micPub *lksdk.LocalTrackPublication

	screen    *screenCast
	screenCmd *exec.Cmd
	screenPub *lksdk.LocalTrackPublication
	// a copy of the shared screen for a window of its own, while
	// watching video: the room never sends one's own track back
	screenCopy gate
	preview    *player

	deaf     atomic.Bool
	watching atomic.Bool

	mu      sync.Mutex
	players map[string]*player
}

// One remote track: the player it is piped into and how frames get
// there. `writeRTP` takes packets as they arrive; `writeFrame` takes a
// whole decrypted frame on an encrypted channel, and is nil where the
// SDK cannot decrypt that codec.
type player struct {
	cmd         *exec.Cmd
	in          io.WriteCloser
	done        chan struct{}
	audio       bool
	writeRTP    func(*rtp.Packet) error
	writeFrame  func([]byte) error
	closeWriter func() error
}

func (p *phone) callbacks() *lksdk.RoomCallback {
	return &lksdk.RoomCallback{
		OnDisconnected: func() {
			say("disconnected")
			if p.onLeft != nil {
				p.onLeft()
			}
		},
		OnReconnecting: func() { say("reconnecting") },
		OnReconnected:  func() { say("reconnected") },
		OnParticipantConnected: func(rp *lksdk.RemoteParticipant) {
			say("joined: %s", rp.Identity())
		},
		OnParticipantDisconnected: func(rp *lksdk.RemoteParticipant) {
			say("left: %s", rp.Identity())
		},
		ParticipantCallback: lksdk.ParticipantCallback{
			OnTrackSubscribed:   p.trackSubscribed,
			OnTrackUnsubscribed: p.trackUnsubscribed,
		},
	}
}

func (p *phone) publishMicrophone(argv []string) error {
	cmd := exec.Command(argv[0], argv[1:]...)
	cmd.Stderr = nil
	out, err := cmd.StdoutPipe()
	if err != nil {
		return err
	}
	if err := cmd.Start(); err != nil {
		return err
	}
	p.mic = cmd

	opts := []lksdk.ReaderSampleProviderOption{
		lksdk.ReaderTrackWithOnWriteComplete(func() { say("microphone ended") }),
	}
	pub := &lksdk.TrackPublicationOptions{
		Name:   "microphone",
		Source: livekit.TrackSource_MICROPHONE,
	}
	if p.keys != nil {
		enc, err := lksdk.NewFrameEncryptor(p.keys, lksdk.CodecOpus)
		if err != nil {
			return err
		}
		opts = append(opts, lksdk.ReaderTrackWithSampleOptions(lksdk.WithFrameEncryptor(enc)))
		pub.Encryption = livekit.Encryption_GCM
	}
	track, err := lksdk.NewLocalReaderTrack(out, webrtc.MimeTypeOpus, opts...)
	if err != nil {
		return err
	}
	publication, err := p.room.LocalParticipant.PublishTrack(track, pub)
	if err != nil {
		return err
	}
	p.micPub = publication
	say("microphone published (%s)", argv[0])
	return nil
}

func (p *phone) trackSubscribed(track *webrtc.TrackRemote, pub *lksdk.RemoteTrackPublication, rp *lksdk.RemoteParticipant) {
	switch pub.Kind() {
	case lksdk.TrackKindAudio:
		p.startAudio(track, pub, rp)
	case lksdk.TrackKindVideo:
		if !p.watching.Load() {
			// a camera or a screen nobody asked to see: not even received
			_ = pub.SetSubscribed(false)
			return
		}
		p.startVideo(track, pub, rp)
	}
}

func (p *phone) startAudio(track *webrtc.TrackRemote, pub *lksdk.RemoteTrackPublication, rp *lksdk.RemoteParticipant) {
	argv, err := playerCommand()
	if err != nil {
		say("cannot play %s: %v", rp.Identity(), err)
		return
	}
	pl, err := startPlayer(argv)
	if err != nil {
		say("cannot play %s: %v", rp.Identity(), err)
		return
	}
	channels := uint16(track.Codec().Channels)
	if channels == 0 {
		channels = 1
	}
	ogg, err := oggwriter.NewWith(pl.in, 48000, channels)
	if err != nil {
		say("cannot play %s: %v", rp.Identity(), err)
		pl.stop()
		return
	}
	pl.audio = true
	pl.writeRTP = ogg.WriteRTP
	// decrypted frames are re-packetised, one 20 ms Opus frame each,
	// since the Ogg writer wants RTP
	var seq uint16
	var ts uint32
	pl.writeFrame = func(frame []byte) error {
		seq++
		ts += 960
		return ogg.WriteRTP(&rtp.Packet{
			Header:  rtp.Header{Version: 2, SequenceNumber: seq, Timestamp: ts},
			Payload: frame,
		})
	}
	pl.closeWriter = ogg.Close
	p.keep(track.ID(), pl)
	say("hearing %s (%s)", describe(pub, rp), argv[0])
	go p.pump(track, pl, lksdk.CodecOpus)
}

func (p *phone) startVideo(track *webrtc.TrackRemote, pub *lksdk.RemoteTrackPublication, rp *lksdk.RemoteParticipant) {
	mime := strings.ToLower(track.Codec().MimeType)
	var format string
	var codec lksdk.Codec = -1
	switch mime {
	case strings.ToLower(webrtc.MimeTypeVP8), strings.ToLower(webrtc.MimeTypeVP9), strings.ToLower(webrtc.MimeTypeAV1):
		format = "ivf"
	case strings.ToLower(webrtc.MimeTypeH264):
		format = "h264"
		codec = lksdk.CodecH264
	default:
		say("cannot show %s: %s is not a codec this can pipe", describe(pub, rp), track.Codec().MimeType)
		_ = pub.SetSubscribed(false)
		return
	}
	if p.keys != nil && codec < 0 {
		say("cannot show %s: encrypted %s cannot be decrypted here", describe(pub, rp), track.Codec().MimeType)
		_ = pub.SetSubscribed(false)
		return
	}
	argv, err := videoPlayerCommand(describe(pub, rp), format)
	if err != nil {
		say("cannot show %s: %v", describe(pub, rp), err)
		return
	}
	pl, err := startPlayer(argv)
	if err != nil {
		say("cannot show %s: %v", describe(pub, rp), err)
		return
	}
	if format == "ivf" {
		ivf, err := ivfwriter.NewWith(pl.in, ivfwriter.WithCodec(track.Codec().MimeType))
		if err != nil {
			say("cannot show %s: %v", describe(pub, rp), err)
			pl.stop()
			return
		}
		pl.writeRTP = ivf.WriteRTP
		pl.closeWriter = ivf.Close
	} else {
		h264 := h264writer.NewWith(pl.in)
		pl.writeRTP = h264.WriteRTP
		// a decrypted H.264 frame comes out as Annex B, which is what a
		// raw h264 stream is
		pl.writeFrame = func(frame []byte) error {
			_, err := pl.in.Write(frame)
			return err
		}
		pl.closeWriter = h264.Close
	}
	p.keep(track.ID(), pl)
	say("showing %s (%s)", describe(pub, rp), argv[0])
	go p.pump(track, pl, codec)
}

func startPlayer(argv []string) (*player, error) {
	cmd := exec.Command(argv[0], argv[1:]...)
	in, err := cmd.StdinPipe()
	if err != nil {
		return nil, err
	}
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	return &player{cmd: cmd, in: in, done: make(chan struct{})}, nil
}

func (p *phone) keep(id string, pl *player) {
	p.mu.Lock()
	p.players[id] = pl
	p.mu.Unlock()
}

// Who is showing what, for a window title and the log.
func describe(pub *lksdk.RemoteTrackPublication, rp *lksdk.RemoteParticipant) string {
	what := "voice"
	switch pub.Source() {
	case livekit.TrackSource_CAMERA:
		what = "camera"
	case livekit.TrackSource_SCREEN_SHARE:
		what = "screen"
	case livekit.TrackSource_SCREEN_SHARE_AUDIO:
		what = "screen audio"
	}
	return rp.Identity() + " " + what
}

// Copy one remote track into its player until the track ends.
func (p *phone) pump(track *webrtc.TrackRemote, pl *player, codec lksdk.Codec) {
	defer close(pl.done)
	defer func() {
		if pl.closeWriter != nil {
			_ = pl.closeWriter()
		}
		_ = pl.in.Close()
		_ = pl.cmd.Wait()
	}()
	if p.keys != nil {
		dec, err := lksdk.NewFrameDecryptor(p.keys, codec, p.room.SifTrailer())
		if err != nil {
			say("cannot decrypt: %v", err)
			return
		}
		td := e2ee.NewTrackDecryptor(track, dec)
		for {
			sample, err := td.ReadSample()
			if err != nil {
				return
			}
			if sample == nil || (pl.audio && p.deaf.Load()) {
				continue
			}
			if err := pl.writeFrame(sample.Data); err != nil {
				return
			}
		}
	}
	for {
		pkt, _, err := track.ReadRTP()
		if err != nil {
			return
		}
		if pl.audio && p.deaf.Load() {
			continue
		}
		if err := pl.writeRTP(pkt); err != nil {
			return
		}
	}
}

// Start or stop receiving video. Subscribing is what makes the SFU send
// a track, so a camera or a screen nobody watches costs nothing.
func (p *phone) setWatching(on bool) {
	p.watching.Store(on)
	if p.room == nil {
		return
	}
	for _, rp := range p.room.GetRemoteParticipants() {
		for _, tp := range rp.TrackPublications() {
			if tp.Kind() != lksdk.TrackKindVideo {
				continue
			}
			if remote, ok := tp.(*lksdk.RemoteTrackPublication); ok {
				_ = remote.SetSubscribed(on)
			}
		}
	}
	if on {
		say("watching video")
	} else {
		say("not watching video")
	}
	p.setPreview()
}

func (p *phone) trackUnsubscribed(track *webrtc.TrackRemote, pub *lksdk.RemoteTrackPublication, rp *lksdk.RemoteParticipant) {
	p.mu.Lock()
	pl := p.players[track.ID()]
	delete(p.players, track.ID())
	p.mu.Unlock()
	if pl != nil {
		pl.stop()
		say("no longer hearing %s", rp.Identity())
	}
}

func (pl *player) stop() {
	_ = pl.in.Close()
	select {
	case <-pl.done:
	case <-time.After(500 * time.Millisecond):
		_ = pl.cmd.Process.Kill()
	}
}

// The control line from fluxter.
func (p *phone) control(r io.Reader, stop func()) {
	sc := bufio.NewScanner(r)
	for sc.Scan() {
		switch strings.TrimSpace(sc.Text()) {
		case "mute":
			if p.micPub != nil {
				p.micPub.SetMuted(true)
			}
			say("muted")
		case "unmute":
			if p.micPub != nil {
				p.micPub.SetMuted(false)
			}
			say("unmuted")
		case "deafen":
			p.deaf.Store(true)
			say("deafened")
		case "undeafen":
			p.deaf.Store(false)
			say("undeafened")
		case "video":
			p.setWatching(true)
		case "novideo":
			p.setWatching(false)
		case "screen":
			go p.startScreen()
		case "noscreen":
			p.stopScreen()
		case "quit":
			stop()
			return
		}
	}
	stop()
}

// A writer that can be pointed at a sink, or at nothing, while the
// stream runs. Errors drop the sink rather than the stream.
type gate struct {
	mu sync.Mutex
	w  io.Writer
}

func (g *gate) Write(b []byte) (int, error) {
	g.mu.Lock()
	w := g.w
	g.mu.Unlock()
	if w != nil {
		if _, err := w.Write(b); err != nil {
			g.set(nil)
		}
	}
	return len(b), nil
}

func (g *gate) set(w io.Writer) {
	g.mu.Lock()
	g.w = w
	g.mu.Unlock()
}

// The last few lines a program wrote to its standard error, for saying
// why it ended.
type tail struct {
	mu   sync.Mutex
	text string
}

func (t *tail) Write(b []byte) (int, error) {
	t.mu.Lock()
	t.text += string(b)
	if len(t.text) > 2000 {
		t.text = t.text[len(t.text)-2000:]
	}
	t.mu.Unlock()
	return len(b), nil
}

func (t *tail) last() string {
	t.mu.Lock()
	defer t.mu.Unlock()
	lines := strings.Split(strings.TrimSpace(t.text), "\n")
	if len(lines) == 0 || lines[len(lines)-1] == "" {
		return ""
	}
	return lines[len(lines)-1]
}

// Share a screen: the portal picks it, a capture program encodes it,
// the room gets it as the screen share source.
func (p *phone) startScreen() {
	p.mu.Lock()
	already := p.screen != nil
	p.mu.Unlock()
	if already {
		return
	}
	sc, err := openScreenCast()
	if err != nil {
		say("screen not shared: %v", err)
		return
	}
	argv := screenCommand(sc)
	cmd := exec.Command(argv[0], argv[1:]...)
	cmd.ExtraFiles = []*os.File{sc.pipe} // fd 3 in the child
	stderr := &tail{}
	cmd.Stderr = stderr
	out, err := cmd.StdoutPipe()
	if err == nil {
		err = cmd.Start()
	}
	if err != nil {
		sc.close()
		say("screen not shared: %s would not start: %v", argv[0], err)
		return
	}
	// the stream goes to the room and, through the gate, to a preview
	stream := struct {
		io.Reader
		io.Closer
	}{io.TeeReader(out, &p.screenCopy), out}
	opts := []lksdk.ReaderSampleProviderOption{
		lksdk.ReaderTrackWithFrameDuration(33 * time.Millisecond),
		lksdk.ReaderTrackWithOnWriteComplete(func() {
			// which end gave up: the capture program, or the reader
			done := make(chan error, 1)
			go func() { done <- cmd.Wait() }()
			select {
			case err := <-done:
				if err != nil {
					say("screen capture ended: %s: %v (%s)", argv[0], err, stderr.last())
				} else {
					say("screen capture ended: %s finished (%s)", argv[0], stderr.last())
				}
			case <-time.After(300 * time.Millisecond):
				say("screen capture ended: the stream could not be read while %s still ran", argv[0])
			}
		}),
	}
	pub := &lksdk.TrackPublicationOptions{
		Name:   "screen",
		Source: livekit.TrackSource_SCREEN_SHARE,
	}
	if p.keys != nil {
		enc, err := lksdk.NewFrameEncryptor(p.keys, lksdk.CodecH264)
		if err != nil {
			_ = cmd.Process.Kill()
			sc.close()
			say("screen not shared: %v", err)
			return
		}
		opts = append(opts, lksdk.ReaderTrackWithSampleOptions(lksdk.WithFrameEncryptor(enc)))
		pub.Encryption = livekit.Encryption_GCM
	}
	track, err := lksdk.NewLocalReaderTrack(stream, webrtc.MimeTypeH264, opts...)
	if err == nil {
		p.screenPub, err = p.room.LocalParticipant.PublishTrack(track, pub)
	}
	if err != nil {
		_ = cmd.Process.Kill()
		sc.close()
		say("screen not shared: %v", err)
		return
	}
	p.mu.Lock()
	p.screen, p.screenCmd = sc, cmd
	p.mu.Unlock()
	say("sharing the screen (%s, node %d)", argv[0], sc.node)
	p.setPreview()
}

// A window showing one's own shared screen, while both sharing and
// watching video; closed as soon as either stops. It joins the stream
// mid-way and shows a picture at the next keyframe, within a second.
func (p *phone) setPreview() {
	p.mu.Lock()
	sharing := p.screen != nil
	current := p.preview
	p.mu.Unlock()
	want := sharing && p.watching.Load()
	if want == (current != nil) {
		return
	}
	if !want {
		p.screenCopy.set(nil)
		p.mu.Lock()
		p.preview = nil
		p.mu.Unlock()
		current.stop()
		say("preview closed")
		return
	}
	argv, err := videoPlayerCommand("your screen", "h264")
	if err != nil {
		say("no preview: %v", err)
		return
	}
	pl, err := startPlayer(argv)
	if err != nil {
		say("no preview: %v", err)
		return
	}
	pl.writeRTP = func(*rtp.Packet) error { return nil }
	go func() {
		_ = pl.cmd.Wait()
		close(pl.done)
	}()
	p.mu.Lock()
	p.preview = pl
	p.mu.Unlock()
	p.screenCopy.set(pl.in)
	say("preview of your screen (%s)", argv[0])
}

func (p *phone) stopScreen() {
	p.mu.Lock()
	sc, cmd, pub := p.screen, p.screenCmd, p.screenPub
	p.screen, p.screenCmd, p.screenPub = nil, nil, nil
	p.mu.Unlock()
	if sc == nil {
		return
	}
	if pub != nil && p.room != nil {
		_ = p.room.LocalParticipant.UnpublishTrack(pub.SID())
	}
	if cmd != nil && cmd.Process != nil {
		_ = cmd.Process.Signal(syscall.SIGTERM)
		done := make(chan struct{})
		go func() { _ = cmd.Wait(); close(done) }()
		select {
		case <-done:
		case <-time.After(time.Second):
			_ = cmd.Process.Kill()
		}
	}
	sc.close()
	say("no longer sharing the screen")
	p.setPreview()
}

func screenCommand(sc *screenCast) []string {
	argv := defaultScreen
	if custom := strings.Fields(os.Getenv("FLUXTER_PHONE_SCREEN")); len(custom) > 0 {
		argv = custom
	}
	out := make([]string, len(argv))
	for i, part := range argv {
		out[i] = strings.ReplaceAll(strings.ReplaceAll(part, "{fd}", "3"), "{node}", fmt.Sprint(sc.node))
	}
	return out
}

// `fluxter-phone screen-test SECONDS FILE`: the portal, the capture and
// the encoder on their own, to check the setup without a call.
func screenTest(seconds, file string) {
	n, err := time.ParseDuration(seconds + "s")
	if err != nil {
		fail("seconds: %v", err)
	}
	sc, err := openScreenCast()
	if err != nil {
		fail("portal: %v", err)
	}
	defer sc.close()
	say("portal gave node %d", sc.node)
	f, err := os.Create(file)
	if err != nil {
		fail("%v", err)
	}
	defer f.Close()
	argv := screenCommand(sc)
	cmd := exec.Command(argv[0], argv[1:]...)
	cmd.ExtraFiles = []*os.File{sc.pipe}
	cmd.Stdout = f
	cmd.Stderr = os.Stderr
	if err := cmd.Start(); err != nil {
		fail("%s: %v", argv[0], err)
	}
	say("capturing with %s for %s", argv[0], n)
	time.Sleep(n)
	_ = cmd.Process.Signal(syscall.SIGTERM)
	_ = cmd.Wait()
	info, _ := f.Stat()
	say("wrote %d bytes to %s (play with: ffplay -f h264 %s)", info.Size(), file, file)
}

func (p *phone) shutdown() {
	p.stopScreen()
	if p.room != nil {
		p.room.Disconnect()
	}
	if p.mic != nil && p.mic.Process != nil {
		// SIGTERM rather than kill: ffmpeg ends its output cleanly
		_ = p.mic.Process.Signal(syscall.SIGTERM)
		done := make(chan struct{})
		go func() { _ = p.mic.Wait(); close(done) }()
		select {
		case <-done:
		case <-time.After(time.Second):
			_ = p.mic.Process.Kill()
		}
	}
	p.mu.Lock()
	players := p.players
	p.players = map[string]*player{}
	p.mu.Unlock()
	for _, pl := range players {
		pl.stop()
	}
	say("left")
}

// Which side ffmpeg captures from: pulse where a PipeWire or PulseAudio
// socket is in the runtime directory, alsa on a bare console.
func captureInput() string {
	runtime := os.Getenv("XDG_RUNTIME_DIR")
	if runtime == "" {
		return "alsa"
	}
	for _, name := range []string{"pipewire-0", "pulse"} {
		if _, err := os.Stat(filepath.Join(runtime, name)); err == nil {
			return "pulse"
		}
	}
	return "alsa"
}

func micCommand() ([]string, error) {
	if custom := strings.Fields(os.Getenv("FLUXTER_PHONE_MIC")); len(custom) > 0 {
		return custom, nil
	}
	if _, err := exec.LookPath(defaultMic[0]); err != nil {
		return nil, errors.New("ffmpeg is not on PATH and FLUXTER_PHONE_MIC is not set")
	}
	argv := make([]string, len(defaultMic))
	capture := captureInput()
	for i, part := range defaultMic {
		argv[i] = strings.ReplaceAll(part, "{capture}", capture)
	}
	return argv, nil
}

func videoPlayerCommand(title, format string) ([]string, error) {
	var argv []string
	if custom := strings.Fields(os.Getenv("FLUXTER_PHONE_VIDEO_PLAYER")); len(custom) > 0 {
		argv = custom
	} else {
		for _, candidate := range defaultVideoPlayers {
			if _, err := exec.LookPath(candidate[0]); err == nil {
				argv = candidate
				break
			}
		}
	}
	if argv == nil {
		return nil, errors.New("neither ffplay nor mpv is on PATH and FLUXTER_PHONE_VIDEO_PLAYER is not set")
	}
	out := make([]string, len(argv))
	for i, part := range argv {
		out[i] = strings.ReplaceAll(strings.ReplaceAll(part, "{title}", title), "{format}", format)
	}
	return out, nil
}

func playerCommand() ([]string, error) {
	if custom := strings.Fields(os.Getenv("FLUXTER_PHONE_PLAYER")); len(custom) > 0 {
		return custom, nil
	}
	for _, argv := range defaultPlayers {
		if _, err := exec.LookPath(argv[0]); err == nil {
			return argv, nil
		}
	}
	return nil, errors.New("neither ffplay nor mpv is on PATH and FLUXTER_PHONE_PLAYER is not set")
}

func say(format string, args ...any) {
	fmt.Printf(format+"\n", args...)
}

func fail(format string, args ...any) {
	fmt.Fprintf(os.Stderr, format+"\n", args...)
	say(format, args...)
	os.Exit(1)
}
