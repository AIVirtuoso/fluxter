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
// `undeafen` and `quit`, one per line; end of input quits as well.
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

func main() {
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

	deaf atomic.Bool

	mu      sync.Mutex
	players map[string]*player
}

// One remote voice: the player it is piped into.
type player struct {
	cmd  *exec.Cmd
	in   io.WriteCloser
	ogg  *oggwriter.OggWriter
	done chan struct{}
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
	if pub.Kind() != lksdk.TrackKindAudio {
		// a camera or a screen: nothing here can show it
		pub.SetSubscribed(false)
		return
	}
	argv, err := playerCommand()
	if err != nil {
		say("cannot play %s: %v", rp.Identity(), err)
		return
	}
	cmd := exec.Command(argv[0], argv[1:]...)
	in, err := cmd.StdinPipe()
	if err != nil {
		say("cannot play %s: %v", rp.Identity(), err)
		return
	}
	if err := cmd.Start(); err != nil {
		say("cannot play %s: %v", rp.Identity(), err)
		return
	}
	channels := uint16(track.Codec().Channels)
	if channels == 0 {
		channels = 1
	}
	ogg, err := oggwriter.NewWith(in, 48000, channels)
	if err != nil {
		say("cannot play %s: %v", rp.Identity(), err)
		_ = cmd.Process.Kill()
		return
	}
	pl := &player{cmd: cmd, in: in, ogg: ogg, done: make(chan struct{})}
	p.mu.Lock()
	p.players[track.ID()] = pl
	p.mu.Unlock()
	say("hearing %s (%s)", rp.Identity(), argv[0])
	go p.pump(track, pl)
}

// Copy one remote voice into its player until the track ends.
func (p *phone) pump(track *webrtc.TrackRemote, pl *player) {
	defer close(pl.done)
	defer func() {
		_ = pl.ogg.Close()
		_ = pl.in.Close()
		_ = pl.cmd.Wait()
	}()
	if p.keys != nil {
		dec, err := lksdk.NewFrameDecryptor(p.keys, lksdk.CodecOpus, p.room.SifTrailer())
		if err != nil {
			say("cannot decrypt: %v", err)
			return
		}
		td := e2ee.NewTrackDecryptor(track, dec)
		// decrypted frames are re-packetised, one 20 ms Opus frame each,
		// since the Ogg writer wants RTP
		var seq uint16
		var ts uint32
		for {
			sample, err := td.ReadSample()
			if err != nil {
				return
			}
			if sample == nil {
				continue
			}
			seq++
			ts += 960
			if p.deaf.Load() {
				continue
			}
			pkt := &rtp.Packet{
				Header:  rtp.Header{Version: 2, SequenceNumber: seq, Timestamp: ts},
				Payload: sample.Data,
			}
			if err := pl.ogg.WriteRTP(pkt); err != nil {
				return
			}
		}
	}
	for {
		pkt, _, err := track.ReadRTP()
		if err != nil {
			return
		}
		if p.deaf.Load() {
			continue
		}
		if err := pl.ogg.WriteRTP(pkt); err != nil {
			return
		}
	}
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
		case "quit":
			stop()
			return
		}
	}
	stop()
}

func (p *phone) shutdown() {
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
