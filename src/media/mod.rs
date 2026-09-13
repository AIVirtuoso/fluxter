mod attachments;
mod audio;
mod cache;
mod chafa;
mod disk_cache;
mod gif_anim;
mod inline;
mod local;
mod open_external;
mod prepare;
pub mod voice;

pub use attachments::{
    ClipboardContent, StagedAttachment, content_type_for_extension, expand_home, from_clipboard,
    from_path, human_size,
};
pub use audio::{Player, attachment_is_audio, format_duration, player_command};
pub use cache::{Lookup, MediaCache};
pub use chafa::chafa_from_bytes;
pub use disk_cache::DiskCache;
pub use gif_anim::{decode_animation, decode_preview_animation};
pub use inline::{
    AVATAR_COLS, AVATAR_PREVIEW_PX, AVATAR_ROWS, BLOCK_MAX_ROWS, Flatten, InlinePicture,
    MAX_PICTURES_PER_MESSAGE, attachment_picture, avatar_url, avatar_url_sized, block_px,
    composite_over, default_avatar_color, default_avatar_key, default_avatar_url, embed_picture,
    parse_default_avatar_key, picture_cells, preview_limits, proxied_url, sixel_blank_transparent,
    sixel_rows,
};
pub use local::{
    LocalSource, file_url, image_dimensions_of, is_image, is_video, parse_file_url,
    parse_staged_url, picture_bytes, staged_url,
};
pub use open_external::{open_file_path, write_temp_video_bytes};
pub use prepare::prepare_pictures;

use crate::api::types::{
    EmbedMediaResponse, MessageAttachmentResponse, MessageEmbedResponse, MessageResponse,
};

fn is_http_url(u: &str) -> bool {
    u.starts_with("http://") || u.starts_with("https://")
}

fn pick_media_url(m: &EmbedMediaResponse) -> Option<String> {
    m.proxy_url
        .clone()
        .or_else(|| m.url.clone())
        .filter(|u| is_http_url(u))
}

fn attachment_is_probably_image(a: &MessageAttachmentResponse) -> bool {
    let mime = a.content_type.as_deref().unwrap_or("");
    if mime.starts_with("image/") {
        return true;
    }
    let n = a.filename.to_lowercase();
    n.ends_with(".png")
        || n.ends_with(".jpg")
        || n.ends_with(".jpeg")
        || n.ends_with(".gif")
        || n.ends_with(".webp")
        || n.ends_with(".bmp")
        || n.ends_with(".avif")
}

fn attachment_is_probably_video(a: &MessageAttachmentResponse) -> bool {
    let mime = a.content_type.as_deref().unwrap_or("");
    if mime.starts_with("video/") {
        return true;
    }
    let n = a.filename.to_lowercase();
    n.ends_with(".mp4")
        || n.ends_with(".webm")
        || n.ends_with(".mov")
        || n.ends_with(".mkv")
        || n.ends_with(".avi")
        || n.ends_with(".m4v")
        || n.ends_with(".ogv")
}

pub fn attachment_image_url(a: &MessageAttachmentResponse) -> Option<String> {
    if !attachment_is_probably_image(a) {
        return None;
    }
    a.proxy_url
        .clone()
        .or_else(|| a.url.clone())
        .filter(|u| is_http_url(u))
}

fn attachment_video_url(a: &MessageAttachmentResponse) -> Option<String> {
    if !attachment_is_probably_video(a) {
        return None;
    }
    a.proxy_url
        .clone()
        .or_else(|| a.url.clone())
        .filter(|u| is_http_url(u))
}

pub fn embed_image_url(embed: &MessageEmbedResponse) -> Option<String> {
    embed
        .image
        .as_ref()
        .and_then(pick_media_url)
        .or_else(|| embed.thumbnail.as_ref().and_then(pick_media_url))
}

fn embed_video_url(embed: &MessageEmbedResponse) -> Option<String> {
    embed.video.as_ref().and_then(pick_media_url)
}

fn embed_label(embed: &MessageEmbedResponse, fallback: &str) -> String {
    embed
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// Last path segment of a URL, percent-decoded: the slug of a GIF
/// provider's page, `https://klipy.com/gifs/linux-kernel-tux` giving
/// `linux-kernel-tux`.
fn url_slug(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let (_, path) = rest.split_once('/')?;
    let end = path.find(['?', '#']).unwrap_or(path.len());
    let slug = path[..end].trim_end_matches('/').rsplit('/').next()?;
    if slug.is_empty() {
        return None;
    }
    Some(
        urlencoding::decode(slug)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| slug.to_string()),
    )
}

/// Name for a GIF embed: its title, else the slug of the provider page
/// (GIF providers send no title), else just "GIF".
fn gif_label(embed: &MessageEmbedResponse) -> String {
    embed
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .or_else(|| embed.url.as_deref().and_then(url_slug))
        .unwrap_or_else(|| "GIF".to_string())
}

#[derive(Debug, Clone)]
pub enum MessagePreviewMedia {
    Image {
        url: String,
        label: String,
    },
    Video {
        url: String,
        label: String,
    },
    /// Played through an external program (see `audio`).
    Audio {
        url: String,
        label: String,
    },
}

fn attachment_audio_url(a: &MessageAttachmentResponse) -> Option<String> {
    if !attachment_is_audio(a) {
        return None;
    }
    a.url
        .clone()
        .or_else(|| a.proxy_url.clone())
        .filter(|u| is_http_url(u))
}

/// What Ctrl+O shows for one embed, if anything.
fn embed_preview_media(embed: &MessageEmbedResponse) -> Option<MessagePreviewMedia> {
    match embed.embed_type.as_str() {
        // GIF providers (KLIPY, Tenor) send the animation itself as the
        // thumbnail, an animated WebP or GIF, plus a WebM/MP4 copy as the
        // video; the embed's own URL is only the provider's web page. Play
        // the animation in the terminal like a GIF attachment, and reach
        // for the video copy only when there is no animation.
        "gifv" => {
            let label = gif_label(embed);
            if let Some(url) = embed_image_url(embed) {
                return Some(MessagePreviewMedia::Image { url, label });
            }
            embed_video_url(embed).map(|url| MessagePreviewMedia::Video { url, label })
        }
        "video" => {
            let video =
                embed_video_url(embed).or_else(|| embed.url.clone().filter(|u| is_http_url(u)));
            if let Some(url) = video {
                return Some(MessagePreviewMedia::Video {
                    url,
                    label: embed_label(embed, "video"),
                });
            }
            embed_image_url(embed).map(|url| MessagePreviewMedia::Image {
                url,
                label: embed_label(embed, "embed image"),
            })
        }
        _ => embed_image_url(embed).map(|url| MessagePreviewMedia::Image {
            url,
            label: embed_label(embed, "embed image"),
        }),
    }
}

/// First image or video suitable for Ctrl+O (images and animations preview
/// in-terminal; videos open externally).
pub fn first_message_preview_media(msg: &MessageResponse) -> Option<MessagePreviewMedia> {
    for a in msg.all_attachments() {
        if let Some(u) = attachment_image_url(a) {
            let label = if a.filename.is_empty() {
                "image".to_string()
            } else {
                a.filename.clone()
            };
            return Some(MessagePreviewMedia::Image { url: u, label });
        }
    }
    for a in msg.all_attachments() {
        if let Some(u) = attachment_video_url(a) {
            let label = if a.filename.is_empty() {
                "video".to_string()
            } else {
                a.filename.clone()
            };
            return Some(MessagePreviewMedia::Video { url: u, label });
        }
    }
    for a in msg.all_attachments() {
        if let Some(u) = attachment_audio_url(a) {
            let label = if a.filename.is_empty() {
                "audio".to_string()
            } else {
                a.filename.clone()
            };
            return Some(MessagePreviewMedia::Audio { url: u, label });
        }
    }

    msg.all_embeds().find_map(embed_preview_media)
}

// Caught you
// Sniffing my boxers
// Who the fuck does that at Red Lobster ?
// Creepy
// Like when Tom Cruise laughs
// That's how your finger
// Felt in my ass

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const THUMB: &str =
        "https://fluxerusercontent.com/external/kZ0w/https/static.klipy.com/ii/9d/52/B9ynyBGO.webp";
    const VIDEO: &str = "https://fluxerusercontent.com/external/Z1qO/https/static.klipy.com/ii/9d/52/DkIvrEVx48Lh.webm";

    /// A KLIPY embed as the API sends it (trimmed).
    fn klipy_embed() -> MessageEmbedResponse {
        serde_json::from_value(json!({
            "type": "gifv",
            "url": "https://klipy.com/gifs/linux-kernel-tux",
            "provider": {"name": "KLIPY", "url": "https://klipy.com/"},
            "thumbnail": {
                "url": "https://static.klipy.com/ii/9d/52/B9ynyBGO.webp",
                "proxy_url": THUMB,
                "width": 312, "height": 312,
                "content_type": "image/webp", "flags": 32
            },
            "video": {
                "url": "https://static.klipy.com/ii/9d/52/DkIvrEVx48Lh.webm",
                "proxy_url": VIDEO,
                "width": 312, "height": 312, "duration": 2,
                "content_type": "video/webm", "flags": 0
            },
            "image": null,
            "title": null
        }))
        .expect("embed deserialises")
    }

    fn message_with(embed: MessageEmbedResponse) -> MessageResponse {
        MessageResponse {
            embeds: vec![embed],
            ..Default::default()
        }
    }

    #[test]
    fn klipy_gif_plays_its_animation_in_terminal() {
        let msg = message_with(klipy_embed());
        match first_message_preview_media(&msg) {
            Some(MessagePreviewMedia::Image { url, label }) => {
                assert_eq!(url, THUMB);
                assert_eq!(label, "linux-kernel-tux");
            }
            other => panic!("expected the animated thumbnail, got {other:?}"),
        }
    }

    #[test]
    fn gif_without_animation_falls_back_to_its_video_copy() {
        let mut embed = klipy_embed();
        embed.thumbnail = None;
        match embed_preview_media(&embed) {
            Some(MessagePreviewMedia::Video { url, label }) => {
                assert_eq!(url, VIDEO);
                assert_eq!(label, "linux-kernel-tux");
            }
            other => panic!("expected the video copy, got {other:?}"),
        }
    }

    #[test]
    fn gif_page_url_alone_is_nothing_to_show() {
        let mut embed = klipy_embed();
        embed.thumbnail = None;
        embed.video = None;
        assert!(embed_preview_media(&embed).is_none());
    }

    #[test]
    fn video_embed_prefers_its_video_media_over_the_page() {
        let embed: MessageEmbedResponse = serde_json::from_value(json!({
            "type": "video",
            "url": "https://example.com/watch/1",
            "title": "Clip",
            "thumbnail": {"url": "https://example.com/1.jpg"},
            "video": {"url": "https://example.com/1.mp4"}
        }))
        .unwrap();
        match embed_preview_media(&embed) {
            Some(MessagePreviewMedia::Video { url, label }) => {
                assert_eq!(url, "https://example.com/1.mp4");
                assert_eq!(label, "Clip");
            }
            other => panic!("expected the video, got {other:?}"),
        }
    }

    #[test]
    fn gif_label_uses_title_then_slug() {
        let mut embed = klipy_embed();
        assert_eq!(gif_label(&embed), "linux-kernel-tux");
        embed.title = Some(" Tux ".to_string());
        assert_eq!(gif_label(&embed), "Tux");
        embed.title = None;
        embed.url = Some("https://klipy.com/gifs/happy%20cat/?utm=1#x".to_string());
        assert_eq!(gif_label(&embed), "happy cat");
        embed.url = Some("https://klipy.com/".to_string());
        assert_eq!(gif_label(&embed), "GIF");
        embed.url = None;
        assert_eq!(gif_label(&embed), "GIF");
    }
}

/// Base64, for the one thing that needs it: a picture sent inside a JSON
/// body rather than uploaded. Standard alphabet, padded, no line breaks.
/// Hand-written rather than pulled in, since this is the whole of the job.
pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18) as usize & 0x3F] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3F] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 0x3F] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 0x3F] as char
        } else {
            '='
        });
    }
    out
}

/// A local picture as the `data:` URI a JSON body takes, with the type
/// read from the file's name.
pub fn data_uri_for_file(path: &std::path::Path, bytes: &[u8]) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mime = content_type_for_extension(&ext);
    format!("data:{mime};base64,{}", base64_encode(bytes))
}

#[cfg(test)]
mod base64_tests {
    use super::base64_encode;

    /// The vectors from RFC 4648, which is the whole of the contract.
    #[test]
    fn the_rfc_vectors_come_out_right() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    /// Every byte value, so the alphabet and the shifts are exercised
    /// rather than just ASCII.
    #[test]
    fn the_whole_byte_range_round_trips_against_a_known_answer() {
        let bytes: Vec<u8> = (0u8..=255).collect();
        let encoded = base64_encode(&bytes);
        assert_eq!(encoded.len(), 344);
        assert!(encoded.starts_with("AAECAwQFBgcICQoLDA0ODxAREhMUFRYX"));
        assert!(encoded.ends_with("8PHy8/T19vf4+fr7/P3+/w=="));
    }
}
