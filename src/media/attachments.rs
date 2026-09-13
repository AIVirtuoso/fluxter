//! Staging attachments for the next message: images or files copied to the
//! system clipboard (Ctrl+V), and files named with `/attach <path>` or
//! picked in the file picker. Any kind of file goes.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// One attachment waiting in the compose box to be uploaded with the next
/// message.
#[derive(Debug, Clone)]
pub struct StagedAttachment {
    /// Tells it from every other staged file, for its preview.
    pub id: u64,
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    /// Pixel size of an image, from its header.
    pub dimensions: Option<(u32, u32)>,
}

// A 32-bit counter, not a 64-bit one: powerpc and the other 32-bit targets have no
// `AtomicU64` at all, and no session stages four billion attachments.
static STAGED_IDS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

impl StagedAttachment {
    pub fn new(filename: String, content_type: String, bytes: Vec<u8>) -> Self {
        let dimensions = if super::local::is_image(&content_type, &filename) {
            super::local::image_dimensions(&bytes)
        } else {
            None
        };
        Self {
            id: u64::from(STAGED_IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)),
            filename,
            content_type,
            bytes,
            dimensions,
        }
    }

    pub fn size_label(&self) -> String {
        human_size(self.bytes.len())
    }

    /// An image the client can show a preview of.
    pub fn is_image(&self) -> bool {
        self.dimensions.is_some()
    }

    pub fn is_video(&self) -> bool {
        super::local::is_video(&self.content_type, &self.filename)
    }
}

pub fn human_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// Image MIME types we know how to name, most preferred first.
const IMAGE_TYPES: &[(&str, &str)] = &[
    ("image/png", "png"),
    ("image/jpeg", "jpg"),
    ("image/webp", "webp"),
    ("image/gif", "gif"),
    ("image/bmp", "bmp"),
    ("image/tiff", "tiff"),
    ("image/avif", "avif"),
];

pub fn content_type_for_extension(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "txt" | "log" | "md" => "text/plain",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn extension_for_image_type(mime: &str) -> &'static str {
    IMAGE_TYPES
        .iter()
        .find(|(m, _)| *m == mime)
        .map(|(_, ext)| *ext)
        .unwrap_or("bin")
}

fn pick_image_type<'a>(offered: impl Iterator<Item = &'a str>) -> Option<String> {
    let offered: Vec<String> = offered
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    for (mime, _) in IMAGE_TYPES {
        if offered.iter().any(|t| t == mime) {
            return Some((*mime).to_string());
        }
    }
    offered.into_iter().find(|t| t.starts_with("image/"))
}

fn clipboard_filename(mime: &str) -> String {
    format!(
        "clipboard-{}.{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        extension_for_image_type(mime)
    )
}

fn run(cmd: &str, args: &[&str]) -> Result<std::process::Output> {
    Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("failed to run {cmd}"))
}

const URI_LIST: &str = "text/uri-list";

/// Files copied in a file manager arrive as a `text/uri-list`.
fn offers_files<'a>(mut offered: impl Iterator<Item = &'a str>) -> bool {
    offered.any(|t| t.trim().eq_ignore_ascii_case(URI_LIST))
}

/// Stage the files a `text/uri-list` names, in order, up to the limit.
fn stage_uri_list(text: &str) -> Result<Vec<StagedAttachment>> {
    let paths = super::local::uri_list_paths(text);
    if paths.is_empty() {
        bail!("the clipboard names no files");
    }
    let mut out = Vec::new();
    for path in paths.iter().take(crate::app::MAX_ATTACHMENTS_PER_MESSAGE) {
        if path.is_dir() {
            continue;
        }
        out.push(stage_file(path)?);
    }
    if out.is_empty() {
        bail!("the clipboard names only directories");
    }
    Ok(out)
}

/// What Ctrl+V found on the clipboard.
#[derive(Debug, Clone)]
pub enum ClipboardContent {
    /// Files copied in a file manager, or an image: staged as attachments.
    Files(Vec<StagedAttachment>),
    /// Plain text: goes into the compose box at the cursor.
    Text(String),
}

/// Plain text is offered under one of these names (the X11 ones are
/// what xclip lists as TARGETS). Returns the name as the owner spelled
/// it, since the paste tool asks for exactly that string.
fn offers_text<'a>(offered: impl Iterator<Item = &'a str>) -> Option<String> {
    const NAMES: [&str; 5] = [
        "text/plain;charset=utf-8",
        "text/plain",
        "UTF8_STRING",
        "STRING",
        "TEXT",
    ];
    let offered: Vec<&str> = offered.map(str::trim).collect();
    NAMES.iter().find_map(|n| {
        offered
            .iter()
            .find(|t| t.eq_ignore_ascii_case(n))
            .map(|t| (*t).to_string())
    })
}

/// A paste tool's complaint, for the status line.
fn tool_error(tool: &str, out: &std::process::Output) -> anyhow::Error {
    let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if msg.is_empty() {
        anyhow::anyhow!("{tool} exited with {}", out.status)
    } else {
        anyhow::anyhow!("{tool}: {msg}")
    }
}

/// Read what the clipboard holds: files copied in a file manager, else an
/// image, else text. Tries wl-paste (Wayland) first, then xclip (X11).
/// Anything else fails with a message naming what the clipboard holds.
fn read_clipboard() -> Result<ClipboardContent> {
    let mut last_err: Option<anyhow::Error> = None;

    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        match run("wl-paste", &["--list-types"]) {
            Ok(list) if list.status.success() => {
                let types = String::from_utf8_lossy(&list.stdout);
                crate::debug::log(
                    "clipboard",
                    format!(
                        "wl-paste offers: {}",
                        types.lines().map(str::trim).collect::<Vec<_>>().join(" ")
                    ),
                );
                if offers_files(types.lines()) {
                    let out = run("wl-paste", &["--no-newline", "--type", URI_LIST])?;
                    if out.status.success() && !out.stdout.is_empty() {
                        return stage_uri_list(&String::from_utf8_lossy(&out.stdout))
                            .map(ClipboardContent::Files);
                    }
                }
                let Some(mime) = pick_image_type(types.lines()) else {
                    if let Some(name) = offers_text(types.lines()) {
                        let out = run("wl-paste", &["--no-newline", "--type", &name])?;
                        if !out.status.success() {
                            return Err(tool_error("wl-paste", &out));
                        }
                        return Ok(ClipboardContent::Text(
                            String::from_utf8_lossy(&out.stdout).into_owned(),
                        ));
                    }
                    let seen: Vec<&str> = types.lines().take(4).collect();
                    bail!(
                        "no text, image or files on the clipboard (it holds: {})",
                        if seen.is_empty() {
                            "nothing".to_string()
                        } else {
                            seen.join(", ")
                        }
                    );
                };
                let out = run("wl-paste", &["--no-newline", "--type", &mime])?;
                if !out.status.success() || out.stdout.is_empty() {
                    bail!("wl-paste returned no data for {mime}");
                }
                return Ok(ClipboardContent::Files(vec![StagedAttachment::new(
                    clipboard_filename(&mime),
                    mime,
                    out.stdout,
                )]));
            }
            Ok(list) => {
                let msg = String::from_utf8_lossy(&list.stderr).trim().to_string();
                last_err = Some(anyhow::anyhow!("wl-paste: {msg}"));
            }
            Err(e) => last_err = Some(e),
        }
    }

    match run("xclip", &["-selection", "clipboard", "-t", "TARGETS", "-o"]) {
        Ok(list) if list.status.success() => {
            let types = String::from_utf8_lossy(&list.stdout);
            crate::debug::log(
                "clipboard",
                format!(
                    "xclip offers: {}",
                    types.lines().map(str::trim).collect::<Vec<_>>().join(" ")
                ),
            );
            if offers_files(types.lines()) {
                let out = run("xclip", &["-selection", "clipboard", "-t", URI_LIST, "-o"])?;
                if out.status.success() && !out.stdout.is_empty() {
                    return stage_uri_list(&String::from_utf8_lossy(&out.stdout))
                        .map(ClipboardContent::Files);
                }
            }
            let Some(mime) = pick_image_type(types.lines()) else {
                if let Some(name) = offers_text(types.lines()) {
                    let out = run("xclip", &["-selection", "clipboard", "-t", &name, "-o"])?;
                    if !out.status.success() {
                        return Err(tool_error("xclip", &out));
                    }
                    return Ok(ClipboardContent::Text(
                        String::from_utf8_lossy(&out.stdout).into_owned(),
                    ));
                }
                bail!("no text, image or files on the clipboard");
            };
            let out = run("xclip", &["-selection", "clipboard", "-t", &mime, "-o"])?;
            if !out.status.success() || out.stdout.is_empty() {
                bail!("xclip returned no data for {mime}");
            }
            return Ok(ClipboardContent::Files(vec![StagedAttachment::new(
                clipboard_filename(&mime),
                mime,
                out.stdout,
            )]));
        }
        Ok(list) => {
            let msg = String::from_utf8_lossy(&list.stderr).trim().to_string();
            last_err = Some(anyhow::anyhow!("xclip: {msg}"));
        }
        Err(e) => {
            if last_err.is_none() {
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        anyhow::anyhow!("no clipboard tool found (install wl-clipboard or xclip)")
    }))
}

pub async fn from_clipboard() -> Result<ClipboardContent> {
    tokio::task::spawn_blocking(read_clipboard)
        .await
        .context("clipboard task")?
}

/// `~/x` as an absolute path. Used by the attachment staging and by the
/// profile editor, which both take a path the reader typed.
pub fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

/// Stage a file of any kind from disk.
fn stage_file(path: &Path) -> Result<StagedAttachment> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    if bytes.is_empty() {
        bail!("{} is empty", path.display());
    }
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "file".to_string());
    let ext = Path::new(&filename)
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(StagedAttachment::new(
        filename,
        content_type_for_extension(&ext).to_string(),
        bytes,
    ))
}

pub async fn from_path(raw: &str) -> Result<StagedAttachment> {
    let path = expand_home(raw.trim().trim_matches('"').trim_matches('\''));
    tokio::task::spawn_blocking(move || stage_file(&path))
        .await
        .context("file task")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_kind_of_file_is_staged_with_its_type() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("notes.tar.xz");
        std::fs::write(&p, b"not really an archive").unwrap();
        let a = stage_file(&p).unwrap();
        assert_eq!(a.filename, "notes.tar.xz");
        assert_eq!(a.content_type, "application/octet-stream");
        assert!(!a.is_image() && !a.is_video());
        assert_eq!(a.size_label(), "21 B");

        let mut png = Vec::new();
        image::RgbaImage::from_pixel(8, 4, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let p = dir.path().join("shot.png");
        std::fs::write(&p, &png).unwrap();
        let a = stage_file(&p).unwrap();
        assert_eq!(a.content_type, "image/png");
        assert_eq!(a.dimensions, Some((8, 4)));
        assert!(a.is_image());
        let b = StagedAttachment::new("clip.mp4".into(), "video/mp4".into(), vec![0; 4]);
        assert!(b.is_video() && !b.is_image());
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn copied_files_are_preferred_over_an_image() {
        assert!(offers_files(["image/png", "text/uri-list"].into_iter()));
        assert!(!offers_files(["image/png", "text/plain"].into_iter()));
        assert_eq!(
            offers_text(["image/png", "text/plain;charset=UTF-8"].into_iter()).as_deref(),
            Some("text/plain;charset=UTF-8")
        );
        assert_eq!(
            offers_text(["TARGETS", "UTF8_STRING"].into_iter()).as_deref(),
            Some("UTF8_STRING")
        );
        assert_eq!(offers_text(["image/png"].into_iter()), None);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a b.txt");
        std::fs::write(&p, "x").unwrap();
        let list = format!(
            "file://{}\r\nfile://{}\r\n",
            p.display().to_string().replace(' ', "%20"),
            dir.path().display()
        );
        let staged = stage_uri_list(&list).unwrap();
        assert_eq!(staged.len(), 1, "directories are skipped");
        assert_eq!(staged[0].filename, "a b.txt");
        assert!(stage_uri_list("# nothing\n").is_err());
    }
}
