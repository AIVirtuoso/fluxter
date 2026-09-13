//! Recording a voice message. The client decides when to start and stop
//! and what to do with the result; a program on PATH does the recording,
//! the same division as playing audio and carrying a call.
//!
//! Every recorder here writes a WAV file, mono at 16 kHz, which is the
//! cheapest thing that carries a voice and the one format whose duration
//! and levels can be read back without decoding anything.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Recorders tried in turn when no command is configured. `{file}` is
/// where the recording goes.
const RECORDERS: &[&[&str]] = &[
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
    recorder_command_in(configured, &path)
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
            .suffix(".wav")
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
    /// wrote. **Termination has to be polite**: a WAV header carries the
    /// data length, and every recorder here writes it on close, so a
    /// SIGKILL leaves a file whose header says zero bytes.
    pub fn finish(mut self) -> io::Result<Vec<u8>> {
        #[cfg(unix)]
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        let _ = self.child.kill();
        let _ = self.child.wait();
        std::fs::read(self.file.path())
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

    // one bucket a point, each the loudest sample in it, which is what a
    // waveform is meant to show
    let samples = len / 2;
    let per_bucket = samples.div_ceil(WAVEFORM_POINTS).max(1);
    let mut waveform = Vec::with_capacity(WAVEFORM_POINTS);
    let mut index = 0usize;
    while index < samples {
        let end = (index + per_bucket).min(samples);
        let mut peak = 0i32;
        for s in index..end {
            let at = start + s * 2;
            if at + 1 >= bytes.len() {
                break;
            }
            let value = i16::from_le_bytes([bytes[at], bytes[at + 1]]) as i32;
            peak = peak.max(value.abs());
        }
        waveform.push((peak * 255 / 32768).clamp(0, 255) as u8);
        index = end;
    }
    if waveform.is_empty() {
        waveform.push(0);
    }
    Some(VoiceShape {
        duration_secs,
        waveform,
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
    }
}
