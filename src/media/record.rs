//! Recording a voice message. The client decides when to start and stop
//! and what to do with the result; a program on PATH does the recording,
//! the same division as playing audio and carrying a call.
//!
//! The recorder of choice is ffmpeg, which captures from the microphone
//! and writes Ogg Opus straight away -- the format the web client records
//! in, at a fraction of the bytes -- and is already one of the programs
//! this client uses. Where it is missing, pw-record, parecord and arecord
//! write mono 16 kHz WAV, which is read back without decoding anything.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Recorders tried in turn when no command is configured. `{file}` is
/// where the recording goes; `{capture}` is the microphone's side, pulse
/// (which PipeWire answers to as well) or alsa on a bare console. Opus is
/// asked for at 48 kHz mono, 32 kbit/s, the voice profile.
const RECORDERS: &[&[&str]] = &[
    &[
        "ffmpeg",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "{capture}",
        "-i",
        "default",
        "-ac",
        "1",
        "-ar",
        "48000",
        "-c:a",
        "libopus",
        "-b:a",
        "32k",
        "-application",
        "voip",
        "{file}",
    ],
    &["pw-record", "--rate=16000", "--channels=1", "{file}"],
    &[
        "parecord",
        "--file-format=wav",
        "--rate=16000",
        "--channels=1",
        "--format=s16le",
        "{file}",
    ],
    &[
        "arecord", "-q", "-t", "wav", "-f", "S16_LE", "-r", "16000", "-c", "1", "{file}",
    ],
];

/// The command to record with: the configured one (split on whitespace),
/// else the first recorder found on PATH. None when there is none, which
/// is a real state the status line says out loud.
pub fn recorder_command(configured: &str) -> Option<Vec<String>> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let argv = recorder_command_in(configured, &path)?;
    let capture = capture_input();
    Some(
        argv.into_iter()
            .map(|part| part.replace("{capture}", capture))
            .collect(),
    )
}

/// Which side ffmpeg captures from: pulse where a PipeWire or PulseAudio
/// socket is in the runtime directory, which is every desktop session,
/// and alsa on a bare console with neither.
pub fn capture_input() -> &'static str {
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return "alsa";
    };
    let runtime = Path::new(&runtime);
    if runtime.join("pipewire-0").exists() || runtime.join("pulse").exists() {
        "pulse"
    } else {
        "alsa"
    }
}

/// The extension the recording file gets, which is what ffmpeg picks its
/// container by: `.ogg` for a command that speaks of ffmpeg, ogg or opus,
/// `.wav` for the rest.
pub fn file_suffix(argv: &[String]) -> &'static str {
    let ogg = argv.iter().any(|part| {
        let p = part.to_ascii_lowercase();
        p.ends_with("ffmpeg") || p.contains("ogg") || p.contains("opus")
    });
    if ogg { ".ogg" } else { ".wav" }
}

fn recorder_command_in(configured: &str, path: &OsStr) -> Option<Vec<String>> {
    let configured: Vec<String> = configured.split_whitespace().map(str::to_string).collect();
    if !configured.is_empty() {
        return Some(configured);
    }
    RECORDERS
        .iter()
        .find(|argv| find_on_path(argv[0], path).is_some())
        .map(|argv| argv.iter().map(|s| s.to_string()).collect())
}

fn find_on_path(name: &str, path: &OsStr) -> Option<PathBuf> {
    if name.contains('/') {
        let p = Path::new(name);
        return is_executable(p).then(|| p.to_path_buf());
    }
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// `{file}` filled in with where the recording goes. A command that names
/// no `{file}` gets the path appended, so a bare `arecord -f cd` works.
pub fn fill_file(argv: &[String], file: &Path) -> Vec<String> {
    let file = file.to_string_lossy().to_string();
    let mut out: Vec<String> = argv
        .iter()
        .map(|part| part.replace("{file}", &file))
        .collect();
    if !argv.iter().any(|part| part.contains("{file}")) {
        out.push(file);
    }
    out
}

/// A recorder that is running, and the file it is filling.
#[derive(Debug)]
pub struct Recorder {
    child: std::process::Child,
    /// Kept so the file lives exactly as long as the recording does.
    file: tempfile::NamedTempFile,
    /// The program's name, for the status line.
    pub program: String,
    pub started: std::time::Instant,
}

impl Recorder {
    pub fn start(argv: &[String]) -> io::Result<Self> {
        let file = tempfile::Builder::new()
            .prefix("fluxter-voice-")
            .suffix(file_suffix(argv))
            .tempfile()?;
        let argv = fill_file(argv, file.path());
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty recorder command"))?;
        let child = std::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Self {
            child,
            file,
            program: program.clone(),
            started: std::time::Instant::now(),
        })
    }

    pub fn elapsed_secs(&self) -> i64 {
        self.started.elapsed().as_secs() as i64
    }

    /// Ask the recorder to finish, wait for it, and hand back what it
    /// wrote with how long it ran. **Termination has to be polite**: a
    /// WAV header carries the data length and an Ogg its last page, and
    /// every recorder here writes them on a SIGTERM (ffmpeg then exits
    /// 255, which is how it reports the signal, with the file whole), so
    /// a SIGKILL leaves a file that says nothing about its length.
    pub fn finish(mut self) -> io::Result<Recording> {
        let elapsed_secs = self.elapsed_secs();
        #[cfg(unix)]
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        let _ = self.child.kill();
        let _ = self.child.wait();
        Ok(Recording {
            bytes: std::fs::read(self.file.path())?,
            elapsed_secs,
        })
    }

    /// Stop and throw away what was recorded.
    pub fn cancel(mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What a recorder wrote, and how long it was running: the length to fall
/// back on when the file does not say.
#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    pub bytes: Vec<u8>,
    pub elapsed_secs: i64,
}

/// The container a recording turned out to be, read off its first bytes
/// rather than trusted from the command: a configured recorder writes
/// what it likes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Wav,
    Ogg,
    Unknown,
}

impl Container {
    pub fn of(bytes: &[u8]) -> Self {
        if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
            Self::Wav
        } else if bytes.starts_with(b"OggS") {
            Self::Ogg
        } else {
            Self::Unknown
        }
    }

    pub fn filename(self) -> &'static str {
        match self {
            Self::Wav => "voice-message.wav",
            Self::Ogg => "voice-message.ogg",
            Self::Unknown => "voice-message",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Wav => "audio/wav",
            Self::Ogg => "audio/ogg",
            Self::Unknown => "application/octet-stream",
        }
    }
}

impl Recording {
    /// The recording's shape: read out of a WAV directly; for an Ogg the
    /// length from its last page and the levels by decoding it with
    /// ffmpeg, flat where ffmpeg is not there; for anything else the time
    /// it was recording for and a flat line. Blocking on the decode, so
    /// call it off the drawing thread.
    pub fn shape(&self) -> VoiceShape {
        match Container::of(&self.bytes) {
            Container::Wav => {
                wav_shape(&self.bytes).unwrap_or_else(|| flat_shape(self.elapsed_secs))
            }
            Container::Ogg => {
                let duration_secs = ogg_duration_secs(&self.bytes).unwrap_or(self.elapsed_secs);
                let waveform = decode_levels(&self.bytes).unwrap_or_else(|| flat_shape(0).waveform);
                VoiceShape {
                    duration_secs,
                    waveform,
                }
            }
            Container::Unknown => flat_shape(self.elapsed_secs),
        }
    }
}

/// How long an Ogg Opus file plays: the granule position of its last
/// page, in 48 kHz samples whatever the stream's own rate, which is the
/// one thing about an Ogg that can be read without a decoder.
pub fn ogg_duration_secs(bytes: &[u8]) -> Option<i64> {
    let last = bytes
        .windows(4)
        .rposition(|w| w == b"OggS")
        .filter(|&i| i + 14 <= bytes.len())?;
    let granule = u64::from_le_bytes(bytes[last + 6..last + 14].try_into().ok()?);
    if granule == u64::MAX {
        return None;
    }
    Some((granule / 48_000) as i64)
}

/// The levels of anything ffmpeg can decode, as the waveform: ffmpeg
/// reads the bytes on its stdin and writes 8 kHz mono PCM, which is
/// plenty for 64 points. None where ffmpeg is missing or refuses.
pub fn decode_levels(bytes: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut child = std::process::Command::new("ffmpeg")
        .args([
            "-loglevel",
            "error",
            "-i",
            "-",
            "-f",
            "s16le",
            "-ac",
            "1",
            "-ar",
            "8000",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let input = bytes.to_vec();
    // fed from its own thread, or a long recording deadlocks on the pipe
    let feeder = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let output = child.wait_with_output().ok()?;
    let _ = feeder.join();
    if !output.status.success() || output.stdout.len() < 2 {
        return None;
    }
    Some(peaks(&output.stdout))
}

/// One bucket a point, each the loudest 16-bit sample in it, which is
/// what a waveform is meant to show.
fn peaks(pcm: &[u8]) -> Vec<u8> {
    let samples = pcm.len() / 2;
    let per_bucket = samples.div_ceil(WAVEFORM_POINTS).max(1);
    let mut waveform = Vec::with_capacity(WAVEFORM_POINTS);
    let mut index = 0usize;
    while index < samples {
        let end = (index + per_bucket).min(samples);
        let mut peak = 0i32;
        for s in index..end {
            let value = i16::from_le_bytes([pcm[s * 2], pcm[s * 2 + 1]]) as i32;
            peak = peak.max(value.abs());
        }
        waveform.push((peak * 255 / 32768).clamp(0, 255) as u8);
        index = end;
    }
    if waveform.is_empty() {
        waveform.push(0);
    }
    waveform
}

/// What a voice message has to carry besides the bytes: how long it runs,
/// and the levels the other client draws as a waveform.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceShape {
    pub duration_secs: i64,
    /// One byte a bucket, 0 to 255, as the API's base64 waveform.
    pub waveform: Vec<u8>,
}

/// How many points the waveform holds. The web client draws a few dozen;
/// the field allows 4,096 base64 characters, which is far more than a
/// terminal or a phone ever shows.
pub const WAVEFORM_POINTS: usize = 64;

/// Read a WAV file's shape: its length from the format and data chunks,
/// and a peak per bucket from the samples. None when the bytes are not a
/// PCM WAV, which is how a configured recorder writing something else is
/// noticed.
pub fn wav_shape(bytes: &[u8]) -> Option<VoiceShape> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let u16at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    let u32at = |i: usize| {
        u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize
    };

    let mut channels = 0usize;
    let mut rate = 0usize;
    let mut bits = 0usize;
    let mut data: Option<(usize, usize)> = None;

    // walk the chunks rather than assuming the canonical 44-byte header:
    // pw-record and parecord both write extra ones
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32at(pos + 4);
        let body = pos + 8;
        match id {
            b"fmt " if body + 16 <= bytes.len() => {
                channels = u16at(body + 2);
                rate = u32at(body + 4);
                bits = u16at(body + 14);
            }
            b"data" => {
                // a recorder that was killed can leave a size of zero or
                // one larger than the file: trust the file
                let available = bytes.len().saturating_sub(body);
                let len = if size == 0 || size > available {
                    available
                } else {
                    size
                };
                data = Some((body, len));
                break;
            }
            _ => {}
        }
        pos = body + size + (size & 1);
    }

    let (start, len) = data?;
    if channels == 0 || rate == 0 || bits != 16 || len == 0 {
        return None;
    }
    let byte_rate = rate * channels * 2;
    let duration_secs = (len / byte_rate) as i64;

    let end = (start + len).min(bytes.len());
    Some(VoiceShape {
        duration_secs,
        waveform: peaks(&bytes[start..end]),
    })
}

/// The shape of something that is not a PCM WAV: the length measured
/// while recording, and a flat waveform, since the field is required and
/// nothing here can decode an arbitrary format.
pub fn flat_shape(duration_secs: i64) -> VoiceShape {
    VoiceShape {
        duration_secs,
        waveform: vec![128; 16],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WAV of `secs` seconds at 16 kHz mono, with a sine-ish ramp so the
    /// buckets are not all equal.
    fn wav(secs: usize) -> Vec<u8> {
        let rate = 16000usize;
        let samples = rate * secs;
        let data_len = samples * 2;
        let mut out = Vec::with_capacity(44 + data_len);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&(rate as u32).to_le_bytes());
        out.extend_from_slice(&((rate * 2) as u32).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        for i in 0..samples {
            let value = ((i as f32 / samples as f32) * 30000.0) as i16;
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    #[test]
    fn a_wav_says_how_long_it_is_and_how_loud_it_got() {
        let shape = wav_shape(&wav(3)).unwrap();
        assert_eq!(shape.duration_secs, 3);
        assert_eq!(shape.waveform.len(), WAVEFORM_POINTS);
        // the ramp means the last bucket is the loudest and the first the
        // quietest
        assert!(shape.waveform[0] < shape.waveform[WAVEFORM_POINTS - 1]);
        assert!(*shape.waveform.last().unwrap() > 200);
    }

    /// Chunks before `data` are skipped rather than assumed away: both
    /// pw-record and parecord write them.
    #[test]
    fn an_extra_chunk_before_the_data_is_walked_over() {
        let mut bytes = wav(1);
        let mut padded = bytes[..12].to_vec();
        padded.extend_from_slice(b"LIST");
        padded.extend_from_slice(&4u32.to_le_bytes());
        padded.extend_from_slice(b"INFO");
        padded.extend_from_slice(&bytes[12..]);
        bytes = padded;
        let shape = wav_shape(&bytes).unwrap();
        assert_eq!(shape.duration_secs, 1);
    }

    /// A recorder that died without closing its file leaves a data size of
    /// zero; the bytes that are there are still a recording.
    #[test]
    fn a_header_that_says_zero_bytes_falls_back_to_the_file() {
        let mut bytes = wav(2);
        let data_at = bytes.len() - 16000 * 2 * 2;
        bytes[data_at - 4..data_at].copy_from_slice(&0u32.to_le_bytes());
        let shape = wav_shape(&bytes).unwrap();
        assert_eq!(shape.duration_secs, 2);
    }

    #[test]
    fn anything_that_is_not_a_pcm_wav_is_refused() {
        assert!(wav_shape(b"").is_none());
        assert!(wav_shape(b"not a wav at all, not even close ----------").is_none());
        let mut ogg = vec![0u8; 64];
        ogg[0..4].copy_from_slice(b"OggS");
        assert!(wav_shape(&ogg).is_none());
    }

    /// An Ogg page header on its own is enough to read the length from.
    #[test]
    fn an_oggs_last_page_says_how_long_it_is() {
        let mut ogg = b"OggS\x00\x02".to_vec();
        ogg.extend_from_slice(&(3 * 48_000u64).to_le_bytes());
        ogg.extend_from_slice(&[0u8; 12]);
        assert_eq!(ogg_duration_secs(&ogg), Some(3));
        assert_eq!(Container::of(&ogg), Container::Ogg);
        assert_eq!(Container::of(&wav(1)), Container::Wav);
        assert_eq!(Container::of(b"ID3\x04"), Container::Unknown);
        assert_eq!(ogg_duration_secs(b"OggS"), None);
    }

    /// A recorder that wrote something unreadable still has a length: the
    /// time it was running for.
    #[test]
    fn an_unknown_recording_keeps_the_clock_and_a_flat_line() {
        let shape = Recording {
            bytes: b"ID3\x04 who knows".to_vec(),
            elapsed_secs: 7,
        }
        .shape();
        assert_eq!(shape.duration_secs, 7);
        assert!(shape.waveform.iter().all(|&v| v == 128));
        let shape = Recording {
            bytes: wav(2),
            elapsed_secs: 9,
        }
        .shape();
        assert_eq!(shape.duration_secs, 2);
        assert_eq!(shape.waveform.len(), WAVEFORM_POINTS);
    }

    /// With ffmpeg on PATH: an Ogg Opus made from a tone comes back with
    /// its length off the last page and levels off the decoder.
    #[test]
    fn an_ogg_is_shaped_by_its_pages_and_the_decoder() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("tone.ogg");
        let made = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg("aevalsrc=0.8*sin(440*2*PI*t):s=48000:d=3")
            .args(["-ac", "1", "-ar", "48000", "-c:a", "libopus", "-b:a", "32k"])
            .arg(&out)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !made {
            return;
        }
        let bytes = std::fs::read(&out).unwrap();
        let shape = Recording {
            bytes,
            elapsed_secs: 99,
        }
        .shape();
        assert_eq!(shape.duration_secs, 3);
        assert_eq!(shape.waveform.len(), WAVEFORM_POINTS);
        // eight tenths of full scale, less what Opus shaves off
        assert!(*shape.waveform.iter().max().unwrap() > 150);
    }

    #[test]
    fn the_recording_file_takes_the_extension_the_command_calls_for() {
        assert_eq!(file_suffix(&["ffmpeg".into(), "{file}".into()]), ".ogg");
        assert_eq!(file_suffix(&["/nix/store/x/bin/ffmpeg".into()]), ".ogg");
        assert_eq!(file_suffix(&["pw-record".into(), "{file}".into()]), ".wav");
        assert_eq!(file_suffix(&["myrec".into(), "--opus".into()]), ".ogg");
    }

    #[test]
    fn the_file_goes_where_the_command_says_or_on_the_end() {
        let path = Path::new("/tmp/x.wav");
        assert_eq!(
            fill_file(&["arecord".into(), "{file}".into()], path),
            ["arecord", "/tmp/x.wav"]
        );
        assert_eq!(
            fill_file(&["arecord".into(), "-q".into()], path),
            ["arecord", "-q", "/tmp/x.wav"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_configured_command_wins_and_the_path_is_searched_otherwise() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            recorder_command_in("  my-recorder -x  ", OsStr::new("")).unwrap(),
            ["my-recorder", "-x"]
        );
        assert!(recorder_command_in("", dir.path().as_os_str()).is_none());
        let p = dir.path().join("arecord");
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let argv = recorder_command_in("", dir.path().as_os_str()).unwrap();
        assert_eq!(argv[0], "arecord");
        // ffmpeg beside it wins, and asks for Ogg Opus from the capture side
        let f = dir.path().join("ffmpeg");
        std::fs::write(&f, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        let argv = recorder_command_in("", dir.path().as_os_str()).unwrap();
        assert_eq!(argv[0], "ffmpeg");
        assert!(argv.contains(&"libopus".to_string()));
        assert!(argv.contains(&"{capture}".to_string()));
        assert!(matches!(capture_input(), "pulse" | "alsa"));
    }
}
