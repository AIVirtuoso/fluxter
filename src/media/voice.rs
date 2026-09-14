//! Handing a voice call's audio to a program on PATH.
//!
//! Fluxer carries voice media over LiveKit: the gateway hands a session
//! a signalling URL and a token, and everything after that is the
//! LiveKit protocol — WebRTC, with ICE, DTLS-SRTP and an Opus codec.
//!
//! This client does not speak any of that, on purpose. Bringing it in
//! would mean libwebrtc, a C++ toolchain in the build, and a great deal
//! of code that has nothing to do with drawing a terminal. So voice
//! follows the same division as the audio player and the notification
//! sender: the client decides — joins, leaves, mutes, keeps the
//! bookkeeping, knows who is in the channel — and a program named in
//! `[media] voice_command` carries the sound, given the URL and the
//! token the server issued.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};

/// The program looked up on PATH when `[media] voice_command` names none:
/// fluxter-phone, which ships with the client and takes the URL, the
/// token and the key as its three arguments.
pub const DEFAULT_PROGRAM: &str = "fluxter-phone";
const DEFAULT_TEMPLATE: &str = "fluxter-phone {url} {token} {key}";

/// The command template to carry the sound with: the configured one, or
/// fluxter-phone's when nothing is configured and it is on PATH. None
/// means no sound is carried, which is a real state the menu says out
/// loud.
pub fn resolve_template(configured: &str) -> Option<String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    resolve_template_in(configured, &path)
}

fn resolve_template_in(configured: &str, path: &std::ffi::OsStr) -> Option<String> {
    let configured = configured.trim();
    if !configured.is_empty() {
        return Some(configured.to_string());
    }
    std::env::split_paths(path)
        .any(|dir| dir.join(DEFAULT_PROGRAM).is_file())
        .then(|| DEFAULT_TEMPLATE.to_string())
}

/// A voice grant, split into the three things a command line needs.
pub struct GrantParts<'a> {
    pub url: &'a str,
    pub token: &'a str,
    pub key: Option<&'a str>,
}

/// Fill `{url}`, `{token}` and `{key}` into a command template, and
/// split it into a program and its arguments.
///
/// Substitution happens after splitting, so a token can never introduce
/// a new argument however it is punctuated: whatever the server sent
/// lands in exactly one argv slot.
pub fn build_command(template: &str, grant: &GrantParts<'_>) -> Option<Vec<String>> {
    let parts: Vec<String> = template
        .split_whitespace()
        .map(|part| {
            part.replace("{url}", grant.url)
                .replace("{token}", grant.token)
                .replace("{key}", grant.key.unwrap_or(""))
        })
        .collect();
    if parts.is_empty() || parts[0].is_empty() {
        return None;
    }
    Some(parts)
}

/// A running sound program, seen from fluxter's end of its control line.
///
/// Its standard input takes one word per line: `mute`, `unmute`,
/// `deafen`, `undeafen`; closing it asks the program to leave. Its
/// standard output is copied to the debug log a line at a time (the
/// program is asked never to print the token), and its standard error
/// is dropped, since the terminal is busy being the UI.
pub struct VoiceMedia {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl std::fmt::Debug for VoiceMedia {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VoiceMedia(pid {})", self.child.id())
    }
}

impl VoiceMedia {
    /// Start the program.
    pub fn start(argv: &[String]) -> std::io::Result<Self> {
        let mut child = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take();
        if let Some(out) = child.stdout.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    crate::debug::log("voice", format!("program: {line}"));
                }
            });
        }
        Ok(Self { child, stdin })
    }

    /// One control word down the line.
    pub fn send(&mut self, word: &str) {
        if let Some(stdin) = &mut self.stdin {
            let _ = writeln!(stdin, "{word}");
            let _ = stdin.flush();
        }
    }

    /// Whether the program has exited on its own, and how.
    pub fn poll(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    /// Close the control line, which asks the program to leave, and kill
    /// it if it has not gone within a moment. The waiting happens off
    /// the UI thread.
    pub fn stop(mut self) {
        drop(self.stdin.take());
        std::thread::spawn(move || {
            for _ in 0..20 {
                if self.child.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant<'a>(key: Option<&'a str>) -> GrantParts<'a> {
        GrantParts {
            url: "wss://voice.example/livekit",
            token: "eyJhbGciOi.tok.en",
            key,
        }
    }

    #[test]
    fn the_placeholders_are_filled_in() {
        let argv = build_command("livekit-cli join --url {url} --token {token}", &grant(None))
            .expect("a command");
        assert_eq!(
            argv,
            vec![
                "livekit-cli",
                "join",
                "--url",
                "wss://voice.example/livekit",
                "--token",
                "eyJhbGciOi.tok.en"
            ]
        );
    }

    #[test]
    fn a_missing_key_becomes_empty_rather_than_the_placeholder() {
        let argv = build_command("p --key {key}", &grant(None)).expect("a command");
        assert_eq!(argv, vec!["p", "--key", ""]);
        let argv = build_command("p --key {key}", &grant(Some("secret"))).expect("a command");
        assert_eq!(argv, vec!["p", "--key", "secret"]);
    }

    #[test]
    fn a_token_with_spaces_in_it_stays_one_argument() {
        // the template is split first and the values put in after, so
        // nothing the server sends can add an argument of its own
        let sneaky = GrantParts {
            url: "wss://x",
            token: "tok --publish-microphone /etc/passwd",
            key: None,
        };
        let argv = build_command("p --token {token} --end", &sneaky).expect("a command");
        assert_eq!(argv.len(), 4);
        assert_eq!(argv[2], "tok --publish-microphone /etc/passwd");
        assert_eq!(argv[3], "--end");
    }

    #[test]
    fn the_default_program_is_used_only_when_it_is_on_path() {
        let dir = tempfile::tempdir().expect("a directory");
        assert!(resolve_template_in("", dir.path().as_os_str()).is_none());
        std::fs::write(dir.path().join(DEFAULT_PROGRAM), "#!/bin/sh\n").expect("a file");
        assert_eq!(
            resolve_template_in("  ", dir.path().as_os_str()).as_deref(),
            Some(DEFAULT_TEMPLATE)
        );
        // what is configured wins, on PATH or not
        assert_eq!(
            resolve_template_in(" my-phone {url} {token} ", dir.path().as_os_str()).as_deref(),
            Some("my-phone {url} {token}")
        );
    }

    #[test]
    fn the_control_line_reaches_the_program_and_closing_it_ends_it() {
        // a stand-in program: copies its control line to a file and
        // exits when the line is closed, as fluxter-phone does
        let dir = tempfile::tempdir().expect("a directory");
        let log = dir.path().join("words");
        let script = dir.path().join("phone.sh");
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho started\ncat > {}\n", log.display()),
        )
        .expect("a script");
        let mut media = VoiceMedia::start(&["/bin/sh".to_string(), script.display().to_string()])
            .expect("started");
        assert!(media.poll().is_none(), "still running");
        media.send("mute");
        media.send("deafen");
        media.stop();
        // stop waits off the UI thread, so give it a moment
        let mut words = String::new();
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            words = std::fs::read_to_string(&log).unwrap_or_default();
            if words.lines().count() == 2 {
                break;
            }
        }
        assert_eq!(words, "mute\ndeafen\n");
    }

    #[test]
    fn an_empty_template_is_no_command_at_all() {
        assert!(build_command("", &grant(None)).is_none());
        assert!(build_command("   ", &grant(None)).is_none());
    }
}
