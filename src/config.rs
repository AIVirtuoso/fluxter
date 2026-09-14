use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_API_BASE_URL: &str = "https://api.fluxer.app/v1";

/// Where colours come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// The terminal's own palette: default foreground/background and the
    /// 16 ANSI colours, so the client matches whatever theme the terminal uses.
    #[default]
    Terminal,
    /// The fixed dark RGB theme modelled on the Fluxer web app.
    Fluxer,
}

/// How messages that concern the user are announced outside the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NotifyMode {
    /// notify-send where there is a display, nothing elsewhere.
    #[default]
    Auto,
    /// libnotify's notify-send.
    Desktop,
    /// GNU mail to the login user (or `notify_mail_to`): for the console,
    /// and only ever by choice.
    Mail,
    Off,
}

/// When to paint the screen ourselves through DRM instead of using the
/// terminal (see src/console).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleMode {
    /// On a Linux virtual console (TERM=linux on a VT); a terminal otherwise.
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConsoleSettings {
    pub mode: ConsoleMode,
    /// DRM device; /dev/dri/card0 when empty.
    pub drm_device: String,
    /// Font files; fontconfig's "monospace", "monospace:bold" and "emoji"
    /// matches when unset.
    pub font: Option<String>,
    pub bold_font: Option<String>,
    pub emoji_font: Option<String>,
    /// Text size in pixels (cell height follows the font's line height).
    pub font_px: f32,
}

impl Default for ConsoleSettings {
    fn default() -> Self {
        Self {
            mode: ConsoleMode::Auto,
            drm_device: String::new(),
            font: None,
            bold_font: None,
            emoji_font: None,
            font_px: 28.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub clock_12h: bool,
    pub theme: Theme,
    #[serde(default, skip_serializing, rename = "image_display")]
    legacy_image_display: Option<String>,
    #[serde(default = "default_true")]
    pub show_typing_indicators: bool,
    /// Tell the channel that you are typing, so others see "… is typing"
    /// the way they do for the web client.
    #[serde(default = "default_true")]
    pub send_typing: bool,
    pub performance_mode: bool,
    /// Pictures and GIFs shown under messages (Ctrl+O still opens them full size).
    #[serde(default = "default_true")]
    pub inline_media: bool,
    /// Profile pictures beside messages.
    #[serde(default = "default_true")]
    pub avatars: bool,
    /// Where notifications go: see [`NotifyMode`].
    pub notifications: NotifyMode,
    /// Recipient of mailed notifications; the login user when empty.
    pub notify_mail_to: String,
    /// The mail program; `mail` when empty.
    pub notify_mail_command: String,
    /// The desktop program; `notify-send` when empty.
    pub notify_desktop_command: String,
    /// Also notify for every message in community channels set to "all
    /// messages", not only mentions and direct messages.
    pub notify_all_messages: bool,
    /// A sound with every notification, played by a program on PATH
    /// (the audio player), so it works on the console too.
    #[serde(default = "default_true")]
    pub notify_sound: bool,
    /// The sound to play; a built-in chime when empty.
    pub notify_sound_file: String,
    /// The program to play it with; `[media] audio_player` or the first
    /// usual player on PATH when empty.
    pub notify_sound_player: String,
    /// What becomes of transparency where the picture protocol has no
    /// alpha channel of its own (sixel, halfblocks). Empty or "auto"
    /// leaves it undrawn on sixel, so the terminal's own background shows
    /// through, and flattens it elsewhere onto the theme's background or,
    /// where the theme fixes none, the terminal's own; "none" flattens
    /// nothing and lets the protocol make of the alpha what it will;
    /// anything else is a colour to flatten onto and draw, `#002b36` or
    /// `rgb:00/2b/36`, for a sixel terminal that paints unset positions.
    pub image_background: String,
}

const fn default_true() -> bool {
    true
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            clock_12h: false,
            theme: Theme::Terminal,
            legacy_image_display: None,
            show_typing_indicators: true,
            send_typing: true,
            performance_mode: false,
            inline_media: true,
            avatars: true,
            notifications: NotifyMode::Auto,
            notify_mail_to: String::new(),
            notify_mail_command: String::new(),
            notify_desktop_command: String::new(),
            notify_all_messages: false,
            notify_sound: true,
            notify_sound_file: String::new(),
            notify_sound_player: String::new(),
            image_background: String::new(),
        }
    }
}

/// Caches for pictures shown in chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaSettings {
    /// Downloaded pictures kept on disk (MiB, 0 disables the disk cache).
    pub disk_cache_mb: u32,
    /// Decoded pictures kept in memory (MiB).
    pub memory_cache_mb: u32,
    /// The command audio attachments are piped to (whitespace-separated);
    /// empty picks the first of mpv, ffplay, pw-play, paplay, aplay on PATH.
    pub audio_player: String,
    /// The program that carries a voice call's audio.
    ///
    /// Fluxer's voice media is LiveKit, and this client does not speak
    /// WebRTC itself: it joins, leaves, mutes and keeps the bookkeeping,
    /// and hands the grant to this program, the same way audio and
    /// notifications are handed to one. Empty means fluxter-phone, the
    /// program that ships with the client, when it is on PATH; otherwise
    /// no program, and the client stays in the channel without carrying
    /// any sound.
    ///
    /// The command is whitespace-separated, without quoting, and takes
    /// three placeholders: `{url}` the LiveKit address, `{token}` the
    /// grant, and `{key}` the end-to-end key where the channel has one
    /// (empty otherwise). The program is told `mute`, `unmute`, `deafen`
    /// and `undeafen` on its standard input, one per line, and is asked
    /// to leave by that input being closed; whatever it prints on its
    /// standard output goes to the debug log, so it must never print the
    /// token. The default is the same as:
    ///
    /// ```toml
    /// [media]
    /// voice_command = "fluxter-phone {url} {token} {key}"
    /// ```
    #[serde(default)]
    pub voice_command: String,
    /// The program that records a voice message (whitespace-separated).
    /// `{file}` is where the recording goes, and a command that names no
    /// `{file}` gets the path appended; the file ends in `.ogg` when the
    /// command speaks of ffmpeg, ogg or opus and `.wav` otherwise, which
    /// is how ffmpeg picks its container. Empty picks ffmpeg on PATH,
    /// capturing from PipeWire or PulseAudio (alsa on a bare console)
    /// straight to Ogg Opus, else the first of pw-record, parecord and
    /// arecord, each asked for mono 16 kHz WAV. What the recorder wrote is
    /// read off its first bytes, so a command may write either.
    ///
    /// ```toml
    /// [media]
    /// recorder_command = "ffmpeg -loglevel error -y -f pulse -i default -ac 1 -c:a libopus -b:a 24k {file}"
    /// ```
    #[serde(default)]
    pub recorder_command: String,
}

impl Default for MediaSettings {
    fn default() -> Self {
        Self {
            disk_cache_mb: 64,
            memory_cache_mb: 64,
            audio_player: String::new(),
            voice_command: String::new(),
            recorder_command: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_api_base_url")]
    pub api_base_url: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub last_server_id: Option<String>,
    #[serde(default)]
    pub last_channel_id: Option<String>,
    #[serde(default)]
    pub ui: UiSettings,
    #[serde(default)]
    pub console: ConsoleSettings,
    #[serde(default)]
    pub media: MediaSettings,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            api_base_url: default_api_base_url(),
            token: None,
            last_server_id: None,
            last_channel_id: None,
            ui: UiSettings::default(),
            console: ConsoleSettings::default(),
            media: MediaSettings::default(),
        }
    }
}

pub fn default_api_base_url() -> String {
    DEFAULT_API_BASE_URL.to_string()
}

/// The API address the client will actually talk to, which is always an
/// `https://` one.
///
/// An empty setting is the default address and a bare host is read as https,
/// but an `http://` one is refused instead of quietly upgraded: the login
/// token travels in the headers of every request, so an address written down
/// as cleartext has to fail where it can be read, not turn into a different
/// address behind the reader's back.
pub fn https_api_base_url(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(default_api_base_url());
    }
    let rest = match value.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => rest,
        Some((scheme, rest)) => bail!(
            "api_base_url is {scheme}://{rest}: the client only talks to the API \
             over https, because the login token goes out with every request. \
             Write it as https://{rest}"
        ),
        None => value,
    };
    let rest = rest.trim_end_matches('/');
    if rest.split(['/', '?', '#']).next().unwrap_or("").is_empty() {
        bail!("api_base_url has no host in it: {value}");
    }
    Ok(format!("https://{rest}"))
}

pub fn default_config_path() -> Result<PathBuf> {
    let base = dirs::config_dir().context("could not determine config directory")?;
    Ok(base.join("fluxer-tui").join("config.toml"))
}

pub fn load_config(path: &Path) -> Result<AppConfig> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let config = toml::from_str::<AppConfig>(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(config)
}

pub fn save_config(path: &Path, config: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }

    let serialized = toml::to_string_pretty(config).context("failed to serialize config")?;
    fs::write(path, serialized).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_API_BASE_URL, https_api_base_url};

    #[test]
    fn an_https_address_is_kept_and_tidied() {
        assert_eq!(
            https_api_base_url("https://api.fluxer.app/v1").unwrap(),
            "https://api.fluxer.app/v1"
        );
        assert_eq!(
            https_api_base_url("  https://api.fluxer.app/v1///  ").unwrap(),
            "https://api.fluxer.app/v1"
        );
        assert_eq!(
            https_api_base_url("HTTPS://API.Fluxer.app/v1").unwrap(),
            "https://API.Fluxer.app/v1"
        );
    }

    #[test]
    fn an_address_with_no_scheme_is_read_as_https() {
        assert_eq!(
            https_api_base_url("api.fluxer.app/v1").unwrap(),
            "https://api.fluxer.app/v1"
        );
        assert_eq!(
            https_api_base_url("localhost:3000/v1").unwrap(),
            "https://localhost:3000/v1"
        );
    }

    #[test]
    fn nothing_at_all_is_the_default_address() {
        assert_eq!(https_api_base_url("").unwrap(), DEFAULT_API_BASE_URL);
        assert_eq!(https_api_base_url("   ").unwrap(), DEFAULT_API_BASE_URL);
    }

    #[test]
    fn every_other_scheme_is_refused_by_name() {
        for address in [
            "http://api.fluxer.app/v1",
            "http://localhost:3000/v1",
            "ws://api.fluxer.app/v1",
            "HTTP://api.fluxer.app/v1",
        ] {
            let err = https_api_base_url(address).unwrap_err().to_string();
            assert!(err.contains("api_base_url"), "{address}: {err}");
            assert!(
                err.contains("https://api.fluxer.app/v1")
                    || err.contains("https://localhost:3000/v1"),
                "{address}: {err}"
            );
        }
    }

    #[test]
    fn an_address_with_no_host_is_refused() {
        assert!(https_api_base_url("https://").is_err());
        assert!(https_api_base_url("https:///v1").is_err());
    }
}
