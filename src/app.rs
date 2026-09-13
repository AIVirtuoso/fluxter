use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::{Protocol, StatefulProtocol};
use std::cell::RefCell;

use crate::api::types::{
    CHANNEL_DM, CHANNEL_DM_PERSONAL_NOTES, CHANNEL_GROUP_DM, CHANNEL_GUILD_CATEGORY,
    CHANNEL_GUILD_LINK, CHANNEL_GUILD_TEXT, CHANNEL_GUILD_VOICE, ChannelPinResponse,
    ChannelResponse, GuildMemberResponse, GuildResponse, MESSAGE_FLAG_SUPPRESS_EMBEDS,
    MESSAGE_NOTIFICATIONS_ALL_MESSAGES, MESSAGE_NOTIFICATIONS_INHERIT,
    MESSAGE_NOTIFICATIONS_NO_MESSAGES, MESSAGE_NOTIFICATIONS_ONLY_MENTIONS, MessageResponse,
    ReadStateResponse, SavedMessageEntryResponse, Snowflake, UserGuildChannelOverride,
    UserGuildMuteConfig, UserGuildSettingsPatch, UserGuildSettingsResponse, UserPartialResponse,
    UserPrivateResponse, UserSettingsResponse, VoiceStateResponse, WellKnownFluxerResponse,
    merge_user_cache, snowflake_sort_key,
};
use crate::api::types::{
    CustomStatusPayload, GuildMemberListUpdateEvent, PresenceRecord, PresenceStatus,
    RELATIONSHIP_BLOCKED, RELATIONSHIP_FRIEND, RELATIONSHIP_INCOMING_REQUEST,
    RELATIONSHIP_OUTGOING_REQUEST, RelationshipResponse,
};
use crate::api::types::{DiscoveryGuildResponse, InviteResponse};
use crate::config::UiSettings;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Servers,
    Channels,
    Messages,
    Input,
}

impl Focus {
    pub fn next(self) -> Self {
        match self {
            Self::Servers => Self::Channels,
            Self::Channels => Self::Messages,
            Self::Messages => Self::Input,
            Self::Input => Self::Servers,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Servers => Self::Input,
            Self::Channels => Self::Servers,
            Self::Messages => Self::Channels,
            Self::Input => Self::Messages,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerSelection {
    DirectMessages,
    Guild(String),
}

impl ServerSelection {
    pub fn id(&self) -> String {
        match self {
            Self::DirectMessages => "@me".to_string(),
            Self::Guild(id) => id.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayStatus {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

impl GatewayStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::Disconnected => "disconnected",
        }
    }
}

#[derive(Debug, Clone)]
pub struct EmojiMatch {
    pub label: String,
    pub insert: String,
    pub is_custom: bool,
    /// Guild emoji id, so the popup can draw its picture.
    pub custom_id: Option<String>,
    pub custom_animated: bool,
}

#[derive(Debug, Clone)]
pub struct EmojiAutocomplete {
    pub matches: Vec<EmojiMatch>,
    pub selected_index: usize,
}

/// One selectable row in @ autocomplete (users vs roles are separate insert targets).
#[derive(Debug, Clone)]
pub enum MentionPick {
    User {
        user_id: String,
        display: String,
        username: String,
    },
    Role {
        role_id: String,
        name: String,
        color: u32,
    },
}

impl MentionPick {
    fn matches_filter(&self, query: &str) -> bool {
        match self {
            MentionPick::User {
                display, username, ..
            } => display.to_lowercase().contains(query) || username.to_lowercase().contains(query),
            MentionPick::Role { name, .. } => name.to_lowercase().contains(query),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MentionAutocomplete {
    pub pool: Vec<MentionPick>,
    pub matches: Vec<usize>,
    pub selected_index: usize,
}

#[derive(Debug, Clone)]
pub struct CommandAutocomplete {
    pub matches: Vec<usize>,
    pub selected_index: usize,
}

pub const MAX_ATTACHMENTS_PER_MESSAGE: usize = 10;
/// What the API accepts on one message.
pub const MAX_STICKERS_PER_MESSAGE: usize = 3;
/// The media proxy clamps a sticker request into this size class.
pub const STICKER_MIN_PX: u32 = 128;
pub const STICKER_MAX_PX: u32 = 512;

/// A sticker staged in the compose box, sent with the next message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedSticker {
    pub id: String,
    pub name: String,
    pub animated: bool,
}

/// One row of the sticker picker: a sticker and the guild that owns it.
#[derive(Debug, Clone)]
pub struct StickerEntry {
    pub guild_id: String,
    pub guild_name: String,
    pub sticker: crate::api::types::GuildStickerResponse,
}

/// The sticker picker (Alt+S, `/sticker`): every sticker the client knows
/// of, the active community's first. The cursor moves with the vim keys,
/// and `/` opens the search that filters the list.
#[derive(Debug, Clone)]
pub struct StickerPicker {
    pub entries: Vec<StickerEntry>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub query: String,
    /// The search is open: keys go to the filter, not to the cursor.
    pub searching: bool,
    /// The filter to put back when the search is cancelled.
    query_before_search: String,
}

impl StickerPicker {
    /// The sticker under the cursor.
    pub fn current(&self) -> Option<&StickerEntry> {
        self.filtered
            .get(self.selected)
            .and_then(|&i| self.entries.get(i))
    }
}

/// Width in cells of an inline custom emoji (one row tall).
pub const CUSTOM_EMOJI_CELLS: u16 = 2;
/// Two braille blanks: not whitespace, so wrapping never trims them, and
/// invisible if the picture fails to land on top.
pub const CUSTOM_EMOJI_PLACEHOLDER: &str = "\u{2800}\u{2800}";

/// A guild emoji as it is known to the message renderer.
pub enum CustomEmojiState {
    Loading,
    Ready(PictureFrames),
    Failed,
}

/// One encoded picture per animation frame (a single frame for still
/// emoji). Animated emoji all run on the app's shared clock, see
/// [`App::custom_emoji_current`].
/// A picture, encoded for the terminal's graphics protocol or kept as
/// pixels for console mode.
#[derive(Clone)]
pub enum Picture {
    Terminal(std::sync::Arc<TerminalPicture>),
    Pixels(std::sync::Arc<image::RgbaImage>),
}

/// A picture as escape sequences for the terminal's graphics protocol,
/// kept in parts so that any run of the block's rows can be printed on
/// its own: a block cut by the pane's edge shows what is on screen. The
/// sequences never enter ratatui's buffer (it would take their length for
/// the width of a character and re-send the rest of the screen on every
/// frame); the backend prints them where a sentinel cell sits.
pub enum TerminalPicture {
    /// Sixel paints bands of six pixel rows. The header up to the height,
    /// the palette and the bands are kept apart, so a run of bands can be
    /// put together under a header with its own height.
    Sixel {
        head: std::sync::Arc<str>,
        palette: std::sync::Arc<str>,
        bands: Vec<std::sync::Arc<str>>,
        area: Rect,
        /// The picture's pixels, so a run of rows that starts inside a
        /// band can be encoded from its own first pixel row.
        pixels: Option<std::sync::Arc<image::RgbaImage>>,
        /// Where no frame of this picture ever draws: those positions are
        /// left undrawn so the terminal's own background shows through
        /// them. Everything another frame paints is painted here too, so
        /// that showing this frame replaces that one outright.
        undrawn: Option<std::sync::Arc<image::RgbaImage>>,
        /// Such runs, by (first row, end row), encoded when first shown.
        cuts: std::sync::Mutex<std::collections::HashMap<(u16, u16), std::sync::Arc<str>>>,
        /// How transparency was dealt with: the colour it was flattened
        /// onto, which a re-encoded run has to flatten onto as well, and
        /// whether the picture is then stopped from drawing there.
        transparent: Option<crate::media::Flatten>,
    },
    /// Kitty places one row of placeholder cells per cell row, each naming
    /// its row of the image, once the image has been transmitted.
    Kitty {
        transmit: std::sync::Arc<str>,
        rows: Vec<std::sync::Arc<str>>,
        area: Rect,
    },
    /// iTerm2: one sequence for the whole block, all or nothing.
    Whole {
        data: std::sync::Arc<str>,
        area: Rect,
    },
}

/// What to print for a run of a picture's rows.
pub struct PicturePrintout {
    /// Sent once per picture before any of its rows (kitty's image data).
    pub transmit: Option<std::sync::Arc<str>>,
    /// (row within the block, the sequence to print at that row's first cell).
    pub rows: Vec<(u16, std::sync::Arc<str>)>,
}

impl TerminalPicture {
    pub fn area(&self) -> Rect {
        match self {
            Self::Sixel { area, .. } | Self::Kitty { area, .. } | Self::Whole { area, .. } => *area,
        }
    }

    /// The sequences that show block rows `r0..r1`, for cells `cell_h`
    /// pixels tall; None when that part cannot be shown on its own. A sixel
    /// run from the top is the bands up to the rows' edge; one cut at the
    /// top is encoded from the pixels at the row's edge (with `picker`),
    /// or, without them, starts at the first whole band inside the rows,
    /// at most five pixels below the edge, and is printed at the row.
    pub fn printout(
        &self,
        r0: u16,
        r1: u16,
        cell_h: u32,
        picker: Option<&ratatui_image::picker::Picker>,
    ) -> Option<PicturePrintout> {
        let rows = self.area().height;
        if r0 >= r1 || r1 > rows {
            return None;
        }
        match self {
            Self::Sixel {
                head,
                palette,
                bands,
                area,
                pixels,
                undrawn,
                cuts,
                transparent,
            } => {
                if r0 > 0
                    && let (Some(pixels), Some(picker)) = (pixels, picker)
                {
                    if let Some(data) = cuts.lock().unwrap().get(&(r0, r1)) {
                        return Some(PicturePrintout {
                            transmit: None,
                            rows: vec![(r0, data.clone())],
                        });
                    }
                    if let Some(data) = encode_sixel_rows(
                        picker,
                        pixels,
                        undrawn.as_deref(),
                        r0,
                        r1,
                        cell_h,
                        area.width,
                        *transparent,
                    ) {
                        cuts.lock().unwrap().insert((r0, r1), data.clone());
                        return Some(PicturePrintout {
                            transmit: None,
                            rows: vec![(r0, data)],
                        });
                    }
                }
                let b0 = (r0 as u32 * cell_h).div_ceil(6) as usize;
                let b1 = ((r1 as u32 * cell_h) / 6) as usize;
                let b1 = b1.min(bands.len());
                if b0 >= b1 {
                    return None;
                }
                let mut data = String::with_capacity(
                    head.len()
                        + palette.len()
                        + bands[b0..b1].iter().map(|b| b.len() + 1).sum::<usize>()
                        + 8,
                );
                data.push_str(head);
                data.push_str(&((b1 - b0) as u32 * 6).to_string());
                data.push_str(palette);
                for (i, band) in bands[b0..b1].iter().enumerate() {
                    if i > 0 {
                        data.push('-');
                    }
                    data.push_str(band);
                }
                data.push_str("\x1b\\");
                Some(PicturePrintout {
                    transmit: None,
                    rows: vec![(r0, std::sync::Arc::from(data))],
                })
            }
            Self::Kitty { transmit, rows, .. } => Some(PicturePrintout {
                transmit: Some(transmit.clone()),
                rows: (r0..r1).map(|r| (r, rows[r as usize].clone())).collect(),
            }),
            Self::Whole { data, .. } => (r0 == 0 && r1 == rows).then(|| PicturePrintout {
                transmit: None,
                rows: vec![(0, data.clone())],
            }),
        }
    }
}

/// Block rows `r0..r1` of a picture as their own sixel, cut to whole bands
/// at the bottom, so the run's first pixel row is the row's edge exactly.
#[allow(clippy::too_many_arguments)]
fn encode_sixel_rows(
    picker: &ratatui_image::picker::Picker,
    pixels: &image::RgbaImage,
    undrawn: Option<&image::RgbaImage>,
    r0: u16,
    r1: u16,
    cell_h: u32,
    cols: u16,
    transparent: Option<crate::media::Flatten>,
) -> Option<std::sync::Arc<str>> {
    let y0 = (r0 as u32 * cell_h).min(pixels.height());
    let h = ((r1 - r0) as u32 * cell_h).min(pixels.height() - y0);
    if h < 6 {
        return None;
    }
    let h = h / 6 * 6;
    // `pixels` is the picture before flattening, so the crop still says
    // which positions were see-through; the encoder is handed a flattened
    // copy and the crop is kept to blank them out of what comes back.
    let crop = image::imageops::crop_imm(pixels, 0, y0, pixels.width(), h).to_image();
    let encoded = transparent.map_or_else(
        || crop.clone(),
        |f| crate::media::composite_over(&crop, f.colour),
    );
    let protocol = picker
        .new_protocol(
            image::DynamicImage::ImageRgba8(encoded),
            Rect::new(0, 0, cols, r1 - r0),
            ratatui_image::Resize::Fit(None),
        )
        .ok()?;
    match protocol {
        Protocol::Sixel(sixel) => Some(std::sync::Arc::from(
            transparent
                .filter(|f| f.drop)
                .and_then(|f| {
                    let mask = undrawn.map_or_else(
                        || crop.clone(),
                        |m| image::imageops::crop_imm(m, 0, y0, m.width(), h).to_image(),
                    );
                    crate::media::sixel_blank_transparent(&sixel.data, &mask, f.colour)
                })
                .unwrap_or_else(|| sixel.data.clone())
                .as_str(),
        )),
        _ => None,
    }
}

/// The cells a protocol picture would put its sequences in, by row: what
/// ratatui-image writes into a scratch buffer.
fn protocol_rows(protocol: &Protocol) -> Vec<(u16, String)> {
    let area = protocol.area();
    let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, area.width, area.height));
    ratatui::widgets::Widget::render(ratatui_image::Image::new(protocol), buf.area, &mut buf);
    (0..area.height)
        .filter_map(|y| {
            let cell = &buf[(0, y)];
            (!cell.skip && cell.symbol().len() > 1).then(|| (y, cell.symbol().to_string()))
        })
        .collect()
}

/// Encode a protocol picture for printing in parts.
pub fn terminal_picture(
    protocol: &Protocol,
    pixels: Option<std::sync::Arc<image::RgbaImage>>,
    undrawn: Option<std::sync::Arc<image::RgbaImage>>,
    transparent: Option<crate::media::Flatten>,
) -> Option<TerminalPicture> {
    let area = protocol.area();
    if area.width == 0 || area.height == 0 {
        return None;
    }
    match protocol {
        Protocol::Sixel(sixel) => {
            // Stopping the picture from drawing wherever it was see-through
            // is what makes transparency real; one that is see-through
            // nowhere is printed exactly as it was encoded.
            let blanked = match (
                transparent.filter(|f| f.drop),
                undrawn.as_deref().or(pixels.as_deref()),
            ) {
                (Some(f), Some(mask)) => {
                    crate::media::sixel_blank_transparent(&sixel.data, mask, f.colour)
                }
                _ => None,
            };
            let data = blanked.as_deref().unwrap_or(sixel.data.as_str());
            // a run cut out of this picture flattens the same way, and stops
            // drawing the same way, so the whole of it is kept
            let kept = transparent;
            parse_sixel(data, area, pixels, undrawn, kept).or_else(|| {
                Some(TerminalPicture::Whole {
                    data: std::sync::Arc::from(data),
                    area,
                })
            })
        }
        Protocol::Kitty(_) => {
            let rows = protocol_rows(protocol);
            if rows.len() != area.height as usize {
                return None;
            }
            // the image data comes before the first row's placeholders
            let (transmit, first) = match rows[0].1.find("\x1b[s") {
                Some(at) => (rows[0].1[..at].to_string(), rows[0].1[at..].to_string()),
                None => (String::new(), rows[0].1.clone()),
            };
            let mut placeholders: Vec<std::sync::Arc<str>> = vec![std::sync::Arc::from(first)];
            placeholders.extend(
                rows[1..]
                    .iter()
                    .map(|(_, s)| std::sync::Arc::from(s.as_str())),
            );
            Some(TerminalPicture::Kitty {
                transmit: std::sync::Arc::from(transmit),
                rows: placeholders,
                area,
            })
        }
        _ => {
            let rows = protocol_rows(protocol);
            let (_, data) = rows.into_iter().next()?;
            Some(TerminalPicture::Whole {
                data: std::sync::Arc::from(data),
                area,
            })
        }
    }
}

/// Take a sixel sequence apart: the DCS header up to the height in the
/// raster attributes, the palette definitions, and the bands.
fn parse_sixel(
    data: &str,
    area: Rect,
    pixels: Option<std::sync::Arc<image::RgbaImage>>,
    undrawn: Option<std::sync::Arc<image::RgbaImage>>,
    transparent: Option<crate::media::Flatten>,
) -> Option<TerminalPicture> {
    let b = data.as_bytes();
    if !data.starts_with("\x1bP") {
        return None;
    }
    let mut i = data.find('q')? + 1;
    if b.get(i) != Some(&b'"') {
        return None;
    }
    i += 1;
    // "Pan;Pad;Ph;Pv: keep everything up to and including the third ';'
    let mut semicolons = 0;
    while i < b.len() && semicolons < 3 {
        match b[i] {
            b';' => semicolons += 1,
            c if c.is_ascii_digit() => {}
            _ => return None,
        }
        i += 1;
    }
    if semicolons < 3 {
        return None;
    }
    let head = &data[..i];
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let palette_start = i;
    // "#n;2;r;g;b" definitions; a "#n" followed by anything but ';' is the
    // body's first colour selection
    while b.get(i) == Some(&b'#') {
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if b.get(j) != Some(&b';') {
            break;
        }
        while j < b.len() && (b[j].is_ascii_digit() || b[j] == b';') {
            j += 1;
        }
        i = j;
    }
    let palette = &data[palette_start..i];
    let body = data[i..].strip_suffix("\x1b\\").unwrap_or(&data[i..]);
    if body.is_empty() {
        return None;
    }
    Some(TerminalPicture::Sixel {
        head: std::sync::Arc::from(head),
        palette: std::sync::Arc::from(palette),
        bands: body.split('-').map(std::sync::Arc::from).collect(),
        area,
        pixels,
        undrawn,
        cuts: std::sync::Mutex::new(std::collections::HashMap::new()),
        transparent,
    })
}

/// Numbers each set of frames, so a sentinel cell can tell pictures apart.
static PICTURE_SERIAL: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(1);

#[derive(Clone)]
pub struct PictureFrames {
    pub frames: Vec<Picture>,
    pub delays: Vec<Duration>,
    pub total: Duration,
    /// Tells this picture from any other one that was on the same cells.
    pub serial: u16,
    /// Frame index drawn last, when it first appeared, and the draw (frame
    /// of the UI) it was decided in: every instance of the emoji on screen
    /// in one draw shows the same frame.
    shown: std::cell::Cell<usize>,
    shown_at: std::cell::Cell<Option<Instant>>,
    decided_in_draw: std::cell::Cell<u64>,
}

impl PictureFrames {
    pub fn new(frames: Vec<Picture>, delays: Vec<Duration>) -> Self {
        let total = delays.iter().sum();
        Self {
            frames,
            delays,
            total,
            serial: PICTURE_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            shown: std::cell::Cell::new(0),
            shown_at: std::cell::Cell::new(None),
            decided_in_draw: std::cell::Cell::new(0),
        }
    }

    pub fn is_animated(&self) -> bool {
        self.delays.len() > 1 && !self.total.is_zero()
    }

    /// The shortest delay between two frames.
    pub fn min_delay(&self) -> Option<Duration> {
        self.delays.iter().copied().min()
    }

    /// The frame to draw at `now`, in UI draw number `draw`. The clock only
    /// runs forward: from the frame shown last and the moment it went up,
    /// the frames whose delay has passed are stepped over, carrying the
    /// remainder, so the cadence stays exact however irregular the draws
    /// are and a frame is never shown again once it is over. A loop shorter
    /// than the time between two draws would land on the same frame; it is
    /// moved one on so it visibly plays. Decided once per draw: every
    /// instance on screen shows the same frame.
    pub fn index_at(&self, now: Instant, draw: u64) -> usize {
        if !self.is_animated() {
            return 0;
        }
        if self.decided_in_draw.get() == draw && self.shown_at.get().is_some() {
            return self.shown.get();
        }
        self.decided_in_draw.set(draw);
        let n = self.delays.len();
        let shown = self.shown.get();
        let Some(mut at) = self.shown_at.get() else {
            self.shown_at.set(Some(now));
            return shown;
        };
        let mut elapsed = now.saturating_duration_since(at);
        let mut stepped = false;
        if elapsed >= self.total {
            // whole loops went by (the pane was hidden, say): keep the phase
            elapsed = Duration::from_nanos((elapsed.as_nanos() % self.total.as_nanos()) as u64);
            at = now - elapsed;
            stepped = true;
        }
        let mut i = shown;
        while elapsed >= self.delays[i] {
            elapsed -= self.delays[i];
            at += self.delays[i];
            i = (i + 1) % n;
            stepped = true;
        }
        if stepped && i == shown {
            i = (i + 1) % n;
        }
        self.shown.set(i);
        self.shown_at.set(Some(at));
        i
    }

    /// The frame to draw now: its index and its picture.
    pub fn current(&self, now: Instant, draw: u64) -> (usize, &Picture) {
        let i = self.index_at(now, draw).min(self.frames.len() - 1);
        (i, &self.frames[i])
    }
}

/// Animation frames kept per custom emoji; enough for the usual short loops.
pub const CUSTOM_EMOJI_MAX_FRAMES: usize = 48;

impl std::fmt::Debug for PictureFrames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PictureFrames({} frames)", self.frames.len())
    }
}

/// Where a reader scrolled up in the history is: the message under the
/// pane's top row and how many rows into it the top row lies. When
/// messages arrive or load while they read, the view is put back on that
/// message instead of drifting with the bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneAnchor {
    pub channel: Option<String>,
    pub message_id: String,
    pub offset: i64,
    /// The content's rows when the anchor was taken: a change means the
    /// content moved under the reader.
    pub total_rows: u32,
}

/// The message pane's view in one draw: which channel, where on screen,
/// how many rows its content had and how far it was scrolled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneView {
    pub channel: Option<String>,
    pub inner: Rect,
    pub total_rows: u16,
    pub top: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// A preview under a message.
    Picture,
    /// A profile picture beside a message.
    Avatar,
}

/// A block of cells showing a picture: what to fetch, at what size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSlot {
    /// Cache key: the URL and the block size, since pictures are prepared
    /// for one exact size.
    pub key: String,
    pub url: String,
    pub cols: u16,
    pub rows: u16,
    pub kind: MediaKind,
}

impl MediaSlot {
    pub fn new(url: String, cols: u16, rows: u16, kind: MediaKind) -> Self {
        Self {
            key: format!("{url}@{cols}x{rows}"),
            url,
            cols,
            rows,
            kind,
        }
    }
}

impl std::fmt::Debug for CustomEmojiState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CustomEmojiState::Loading => "Loading",
            CustomEmojiState::Ready(_) => "Ready(..)",
            CustomEmojiState::Failed => "Failed",
        })
    }
}

#[derive(Debug, Clone)]
pub struct ReplyState {
    pub channel_id: String,
    pub message_id: String,
    pub author_name: String,
    pub source_guild_id: Option<String>,
}

/// The message length limit, in chars, that the compose box enforces.
pub const INPUT_MAX_CHARS: usize = 2000;

#[derive(Debug, Clone)]
pub struct EditState {
    pub channel_id: String,
    pub message_id: String,
}

#[derive(Debug, Clone)]
pub struct PickerEntry {
    pub server: ServerSelection,
    pub channel_id: String,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct ChannelPicker {
    pub query: String,
    pub entries: Vec<PickerEntry>,
    pub filtered: Vec<usize>,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub struct ReadState {
    pub last_message_id: Option<String>,
    pub mention_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationVisibility {
    AllMessages,
    MentionsOnly,
    None,
}

pub enum ImagePreviewState {
    Loading {
        title: String,
    },
    ReadyBitmap {
        title: String,
        protocol: StatefulProtocol,
    },
    ReadyAnimatedGif {
        title: String,
        frames: Vec<DynamicImage>,
        delays: Vec<Duration>,
        frame_idx: usize,
        elapsed: Duration,
        current_protocol: StatefulProtocol,
    },
    ReadyChafa {
        title: String,
        lines: Vec<String>,
        scroll: usize,
    },
    /// Console mode: the frames are blitted by our own renderer.
    ReadyPixels {
        title: String,
        frames: Vec<std::sync::Arc<image::RgbaImage>>,
        delays: Vec<Duration>,
        frame_idx: usize,
        elapsed: Duration,
    },
    Failed {
        message: String,
    },
}

impl std::fmt::Debug for ImagePreviewState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ImagePreviewState::Loading { .. } => "Loading",
            ImagePreviewState::ReadyBitmap { .. } => "ReadyBitmap",
            ImagePreviewState::ReadyAnimatedGif { .. } => "ReadyAnimatedGif",
            ImagePreviewState::ReadyChafa { .. } => "ReadyChafa",
            ImagePreviewState::ReadyPixels { .. } => "ReadyPixels",
            ImagePreviewState::Failed { .. } => "Failed",
        })
    }
}

/// The profile popup: whose profile, and what the API answered.
/// The pings overlay: the messages that mentioned the user, newest
/// first, as the web client's inbox lists them.
#[derive(Debug, Clone)]
pub struct PingsView {
    pub state: PingsState,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub enum PingsState {
    Loading,
    Ready(Vec<MessageResponse>),
    Failed(String),
}

/// One row of the member list as the client keeps it: a heading, or
/// somebody. The gateway sends the two mixed in one array, and the pane
/// draws them the same way.
#[derive(Debug, Clone)]
pub enum MemberRow {
    /// A hoisted role's name, or "Online" / "Offline", with its count.
    Heading {
        label: String,
        count: u32,
    },
    Member {
        user_id: String,
    },
}

/// The member list of one channel, as GUILD_MEMBER_LIST_UPDATE builds it.
///
/// The gateway sends windows rather than the whole list: a SYNC operation
/// replaces one inclusive range of rows. Rows outside every window that
/// has arrived are simply not known yet, so the list is a sparse map by
/// position rather than a vector.
#[derive(Debug, Default)]
pub struct MemberList {
    pub guild_id: String,
    pub channel_id: String,
    pub member_count: u32,
    pub online_count: u32,
    /// Row index to what is there. Sparse until the windows arrive.
    pub rows: HashMap<u32, MemberRow>,
    /// The highest row index any window has covered, so the pane knows
    /// how far it can scroll without asking for more.
    pub known_rows: u32,
    /// The member objects the list carried, by user id.
    pub members: HashMap<String, GuildMemberResponse>,
    pub scroll: u16,
    /// Whether anything has arrived yet, so an empty list can say
    /// "loading" rather than "nobody".
    pub loaded: bool,
}

/// What the client keeps about one person's presence.
#[derive(Debug, Clone, Default)]
pub struct PresenceEntry {
    pub status: PresenceStatus,
    /// Set when their presence came from a phone.
    pub mobile: bool,
    pub custom_status: Option<CustomStatusPayload>,
}

/// What to do to a relationship. The gateway tells the client what came
/// of it, so nothing here writes to the list itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationshipAction {
    Add,
    Accept,
    Block,
    Remove,
}

/// What `+`, `B` and `x` do for one person, given how the reader stands
/// with them.
///
/// The profile's key handler and its footer both read this, so a hint can
/// never offer something the key does not do — which is how `+` came to
/// send a fresh friend request at somebody who had already asked, where
/// the server wants an accept.
#[derive(Debug, Clone, Copy, Default)]
pub struct RelationshipKeys {
    /// `+`
    pub plus: Option<(RelationshipAction, &'static str)>,
    /// `B`
    pub block: Option<(RelationshipAction, &'static str)>,
    /// `x`
    pub undo: Option<(RelationshipAction, &'static str)>,
}

impl RelationshipKeys {
    /// The three of them as hint text, in the order the keys are pressed.
    pub fn hints(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some((_, label)) = self.plus {
            out.push(format!("+ {label}"));
        }
        if let Some((_, label)) = self.undo {
            out.push(format!("x {label}"));
        }
        if let Some((_, label)) = self.block {
            out.push(format!("B {label}"));
        }
        out
    }
}

/// The friends overlay: everybody the reader has a tie to, in the four
/// groups the server sorts them into.
#[derive(Debug)]
pub struct FriendsView {
    pub state: FriendsState,
    pub selected: usize,
    /// Which group the cursor is in decides which keys do anything.
    pub tab: FriendsTab,
    /// Text being typed into the footer, when the reader is part way
    /// through adding somebody or naming a friend.
    pub input: Option<FriendsInput>,
}

/// What the footer is taking, while it is taking anything.
#[derive(Debug, Clone)]
pub enum FriendsInput {
    /// A tag, `name#0001`, of somebody to ask.
    AddTag(String),
    /// A name of the reader's own for a friend. Empty drops the name.
    Nickname { user_id: String, text: String },
}

impl FriendsInput {
    pub fn text(&self) -> &str {
        match self {
            Self::AddTag(text) => text,
            Self::Nickname { text, .. } => text,
        }
    }

    pub fn text_mut(&mut self) -> &mut String {
        match self {
            Self::AddTag(text) => text,
            Self::Nickname { text, .. } => text,
        }
    }

    pub fn prompt(&self) -> &'static str {
        match self {
            Self::AddTag(_) => "Add by tag",
            Self::Nickname { .. } => "Your name for them",
        }
    }
}

/// Joining, making and leaving communities: one overlay for the four
/// things a client needs, each a list and a cursor.
#[derive(Debug)]
pub struct CommunityView {
    pub mode: CommunityMode,
    pub selected: usize,
    /// Text being typed into the footer, when a row asked for some.
    pub input: Option<CommunityInput>,
}

#[derive(Debug, Clone)]
pub enum CommunityMode {
    /// The four things: join, make, browse, leave.
    Menu,
    /// Communities in the directory, with what was searched for.
    Discover { query: String, state: DiscoverState },
    /// A community's own invites, so one can be shared or revoked.
    Invites {
        guild_id: String,
        state: InvitesState,
    },
    /// What an invite leads to, looked up before it is taken, so nobody
    /// joins something they cannot see the name of.
    Preview { code: String, state: PreviewState },
}

#[derive(Debug, Clone)]
pub enum PreviewState {
    Loading,
    Ready(Box<InviteResponse>),
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum DiscoverState {
    Idle,
    Running,
    Ready(Vec<DiscoveryGuildResponse>),
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum InvitesState {
    Loading,
    Ready(Vec<InviteResponse>),
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum CommunityInput {
    /// An invite code or link the reader is pasting or typing.
    JoinCode(String),
    /// The name of a community to make.
    NewName(String),
    /// What to search the directory for.
    Search(String),
}

impl CommunityInput {
    pub fn text(&self) -> &str {
        match self {
            Self::JoinCode(t) | Self::NewName(t) | Self::Search(t) => t,
        }
    }

    pub fn text_mut(&mut self) -> &mut String {
        match self {
            Self::JoinCode(t) | Self::NewName(t) | Self::Search(t) => t,
        }
    }

    pub fn prompt(&self) -> &'static str {
        match self {
            Self::JoinCode(_) => "Invite code or link",
            Self::NewName(_) => "Name it",
            Self::Search(_) => "Look for",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendsTab {
    Friends,
    Incoming,
    Outgoing,
    Blocked,
}

impl FriendsTab {
    pub const ALL: [Self; 4] = [Self::Friends, Self::Incoming, Self::Outgoing, Self::Blocked];

    pub fn label(self) -> &'static str {
        match self {
            Self::Friends => "Friends",
            Self::Incoming => "Wanting",
            Self::Outgoing => "Asked",
            Self::Blocked => "Blocked",
        }
    }

    pub fn relationship_type(self) -> i32 {
        match self {
            Self::Friends => RELATIONSHIP_FRIEND,
            Self::Incoming => RELATIONSHIP_INCOMING_REQUEST,
            Self::Outgoing => RELATIONSHIP_OUTGOING_REQUEST,
            Self::Blocked => RELATIONSHIP_BLOCKED,
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone)]
pub enum FriendsState {
    Loading,
    Ready,
    Failed(String),
}

/// One row of the message actions menu: the keyboard stand-in for the
/// web client's right-click menu on a message. Which rows are offered is
/// decided per message in `App::message_actions_for`, by what the message
/// is and what the channel's permissions allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageAction {
    React,
    ViewReactions,
    ClearReactions,
    Reply,
    Forward,
    Edit,
    Pin,
    Unpin,
    ViewPins,
    Bookmark,
    Unbookmark,
    ViewSaved,
    MarkUnread,
    MarkChannelRead,
    MarkGuildRead,
    SuppressEmbeds,
    ShowEmbeds,
    CopyText,
    CopyLink,
    CopyId,
    RemoveAttachment,
    Delete,
    DeleteMarked,
    Report,
}

impl MessageAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::React => "Add a reaction",
            Self::ViewReactions => "Who reacted",
            Self::ClearReactions => "Clear every reaction",
            Self::Reply => "Reply",
            Self::Forward => "Forward",
            Self::Edit => "Edit",
            Self::Pin => "Pin to the channel",
            Self::Unpin => "Unpin from the channel",
            Self::ViewPins => "Pinned messages",
            Self::Bookmark => "Bookmark",
            Self::Unbookmark => "Remove the bookmark",
            Self::ViewSaved => "Bookmarked messages",
            Self::MarkUnread => "Mark unread from here",
            Self::MarkChannelRead => "Mark the channel read",
            Self::MarkGuildRead => "Mark the community read",
            Self::SuppressEmbeds => "Hide the link previews",
            Self::ShowEmbeds => "Show the link previews",
            Self::CopyText => "Copy the text",
            Self::CopyLink => "Copy a link to it",
            Self::CopyId => "Copy the message id",
            Self::RemoveAttachment => "Remove a file from it",
            Self::Delete => "Delete",
            Self::DeleteMarked => "Delete the marked messages",
            Self::Report => "Report to the moderators",
        }
    }

    /// The key that does the same thing without the menu, where there is
    /// one; shown on the right of the row.
    pub fn hint(self) -> &'static str {
        match self {
            Self::React => "e",
            Self::ViewReactions => "v",
            Self::Reply => "r",
            Self::Forward => "f",
            Self::Edit => "Ctrl+E",
            Self::Pin | Self::Unpin => "P",
            Self::ViewPins => "Alt+P",
            Self::Bookmark | Self::Unbookmark => "b",
            Self::ViewSaved => "Alt+B",
            Self::CopyText => "y",
            Self::CopyLink => "Y",
            Self::Delete => "Ctrl+D",
            _ => "",
        }
    }

    /// Whether choosing it asks for a second press first.
    pub fn needs_confirm(self) -> bool {
        matches!(self, Self::ClearReactions | Self::DeleteMarked)
    }
}

/// What the actions overlay is showing: the actions themselves, or one of
/// the lists an action leads to. Keeping them in one overlay saves three
/// more of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageActionsMode {
    Actions,
    /// Which file to take off the message: (attachment id, filename).
    Attachments(Vec<(String, String)>),
    ReportCategories,
    /// A destructive action waiting for a second press.
    Confirm(MessageAction),
}

/// What choosing a row of the actions menu comes to. The menu itself
/// only decides; `main` does the work, since that is where the HTTP
/// client and the task channel live.
#[derive(Debug, Clone)]
pub enum MessageActionOutcome {
    Run {
        action: MessageAction,
        channel_id: String,
        message_id: String,
        /// The report category, or the attachment id, where the action
        /// needed one picked first.
        argument: Option<String>,
    },
}

#[derive(Debug)]
pub struct MessageActionsView {
    pub channel_id: String,
    pub message_id: String,
    pub mode: MessageActionsMode,
    pub actions: Vec<MessageAction>,
    pub selected: usize,
}

/// Looking after one community channel: the same list-and-cursor shape as
/// the message menu, opened with `a` on the channel list.
#[derive(Debug)]
pub struct ChannelAdminView {
    pub channel_id: String,
    pub guild_id: String,
    /// The channel's name when the menu opened, for the headings and for
    /// the sentence the confirmation asks.
    pub channel_name: String,
    pub mode: ChannelAdminMode,
    pub actions: Vec<ChannelAdminAction>,
    pub selected: usize,
    /// Text being typed into the footer, when a row asked for some.
    pub input: Option<ChannelAdminInput>,
}

#[derive(Debug, Clone)]
pub enum ChannelAdminMode {
    /// The actions that apply to this channel.
    Actions,
    /// Which kind of channel to make, before its name is asked for.
    NewKind,
    /// The second press a deletion asks for.
    ConfirmDelete,
}

/// One row of the channel menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelAdminAction {
    New,
    Rename,
    Topic,
    ClearTopic,
    Slowmode,
    CopyId,
    Delete,
}

impl ChannelAdminAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::New => "Make a channel here",
            Self::Rename => "Rename this channel",
            Self::Topic => "Set the topic",
            Self::ClearTopic => "Clear the topic",
            Self::Slowmode => "Slowmode",
            Self::CopyId => "Copy the channel id",
            Self::Delete => "Delete this channel",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Delete => "for good",
            _ => "",
        }
    }

    pub fn is_destructive(self) -> bool {
        self == Self::Delete
    }
}

/// The kinds of channel a community can hold, in the order the menu
/// offers them.
pub const NEW_CHANNEL_KINDS: [(i32, &str); 4] = [
    (CHANNEL_GUILD_TEXT, "Text channel"),
    (CHANNEL_GUILD_VOICE, "Voice channel"),
    (CHANNEL_GUILD_CATEGORY, "Category"),
    (CHANNEL_GUILD_LINK, "Link channel"),
];

#[derive(Debug, Clone)]
pub enum ChannelAdminInput {
    /// The name of a channel to make, and which kind it will be.
    NewName {
        channel_type: i32,
        text: String,
    },
    Rename(String),
    Topic(String),
    /// Seconds between messages, as typed.
    Slowmode(String),
}

impl ChannelAdminInput {
    pub fn text(&self) -> &str {
        match self {
            Self::NewName { text, .. } => text,
            Self::Rename(t) | Self::Topic(t) | Self::Slowmode(t) => t,
        }
    }

    pub fn text_mut(&mut self) -> &mut String {
        match self {
            Self::NewName { text, .. } => text,
            Self::Rename(t) | Self::Topic(t) | Self::Slowmode(t) => t,
        }
    }

    pub fn prompt(&self) -> &'static str {
        match self {
            Self::NewName { .. } => "Name for the new channel",
            Self::Rename(_) => "New name",
            Self::Topic(_) => "Topic",
            Self::Slowmode(_) => "Seconds between messages (0 turns it off)",
        }
    }
}

/// The categories `POST /reports/message` takes, with the wording the web
/// client puts on them.
pub const REPORT_CATEGORIES: [(&str, &str); 12] = [
    ("harassment", "Harassment or bullying"),
    ("hate_speech", "Hate speech"),
    ("violent_content", "Violence"),
    ("spam", "Spam"),
    ("nsfw_violation", "Adult content in the wrong place"),
    ("illegal_activity", "Illegal activity"),
    ("doxxing", "Private information about somebody"),
    ("self_harm", "Self-harm or suicide"),
    ("child_safety", "Danger to a minor"),
    ("malicious_links", "Malware or phishing links"),
    ("impersonation", "Pretending to be somebody else"),
    ("other", "Something else"),
];

#[derive(Debug)]
pub struct PinsView {
    pub channel_id: String,
    pub state: PinsState,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub enum PinsState {
    Loading,
    Ready(Vec<ChannelPinResponse>),
    Failed(String),
}

#[derive(Debug)]
pub struct SavedView {
    pub state: SavedState,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub enum SavedState {
    Loading,
    Ready(Vec<SavedMessageEntryResponse>),
    Failed(String),
}

/// Who reacted to one message with one emoji. `emoji_api` is the form the
/// reaction routes take (the character, or `name:id` for a custom emoji);
/// `emoji_label` is what to put on the screen.
#[derive(Debug)]
pub struct ReactionUsersView {
    pub channel_id: String,
    pub message_id: String,
    pub emoji_api: String,
    pub emoji_label: String,
    /// Which of the message's reactions is being shown, so Left and Right
    /// can walk them without closing.
    pub reaction_index: usize,
    pub state: ReactionUsersState,
    pub scroll: u16,
}

#[derive(Debug, Clone)]
pub enum ReactionUsersState {
    Loading,
    Ready(Vec<UserPartialResponse>),
    Failed(String),
}

/// The search overlay: a query being typed, the scope it runs in, and
/// whatever came back.
#[derive(Debug)]
pub struct SearchView {
    pub query: String,
    pub scope: crate::search::SearchScope,
    pub state: SearchState,
    pub selected: usize,
    pub page: u32,
    /// Whether the cursor is in the query line or down among the
    /// results. Typing goes to the query, moving goes to the results.
    pub editing: bool,
}

#[derive(Debug, Clone)]
pub enum SearchState {
    /// Nothing asked for yet.
    Idle,
    Running,
    /// The server is still indexing a channel in scope, which is an
    /// answer rather than a failure.
    Indexing,
    Ready {
        messages: Vec<MessageResponse>,
        total: u32,
        page: u32,
        hits_per_page: u32,
    },
    Failed(String),
}

/// Starting a conversation, and looking after one that exists.
///
/// One overlay in two modes: a list of people to start with, and the
/// handful of things a group conversation needs doing to it.
#[derive(Debug)]
pub struct ConversationView {
    pub mode: ConversationMode,
    pub filter: String,
    pub selected: usize,
    /// The people ticked with Space, which is what turns a one-to-one
    /// into a group.
    pub marked: Vec<String>,
    /// Text being typed into the footer, when a row asked for some.
    pub input: Option<ConversationInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationMode {
    /// Pick somebody, or several, to talk to.
    People,
    /// What can be done to the group now open.
    Group { channel_id: String },
    /// Which of a group's people to take out.
    RemoveFrom { channel_id: String },
}

#[derive(Debug, Clone)]
pub enum ConversationInput {
    Rename { channel_id: String, text: String },
}

/// One row of the group menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupAction {
    Rename,
    AddSomebody,
    RemoveSomebody,
    Leave,
}

impl GroupAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rename => "Rename it",
            Self::AddSomebody => "Add somebody",
            Self::RemoveSomebody => "Take somebody out",
            Self::Leave => "Leave it",
        }
    }
}

/// One row of the community menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommunityAction {
    Join,
    Create,
    Discover,
    Invites,
    Leave,
}

impl CommunityAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Join => "Join with an invite",
            Self::Create => "Make a community",
            Self::Discover => "Browse the directory",
            Self::Invites => "Invites to this community",
            Self::Leave => "Leave this community",
        }
    }
}

/// The voice channel this session is in, as far as the client knows.
///
/// Fluxer carries voice media over LiveKit, so the client does the
/// joining, the leaving, the muting and the bookkeeping, and hands the
/// grant to a program on PATH to carry the audio — the same division as
/// the audio player and the notification sender.
#[derive(Debug, Clone)]
pub struct VoiceConnection {
    pub channel_id: String,
    pub guild_id: Option<String>,
    /// The server's name for this connection, needed to change or leave
    /// it. None until the first ack or grant comes back.
    pub connection_id: Option<String>,
    pub self_mute: bool,
    pub self_deaf: bool,
    /// The grant, once VOICE_SERVER_UPDATE has arrived.
    pub grant: Option<VoiceGrant>,
    /// Whether a media program was started for this connection.
    pub media_running: bool,
}

#[derive(Debug, Clone)]
pub struct VoiceGrant {
    pub endpoint: String,
    pub token: String,
    pub e2ee_key: Option<String>,
}

/// Somebody ringing a direct message or group.
#[derive(Debug, Clone)]
pub struct IncomingCall {
    pub channel_id: String,
    /// Who is being rung, from the call event; the reader is in it while
    /// the call is still ringing for them.
    pub ringing: Vec<String>,
}

/// The voice menu.
#[derive(Debug)]
pub struct VoiceView {
    pub selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceAction {
    Join,
    Answer,
    Decline,
    StartCall,
    Mute,
    Unmute,
    Deafen,
    Undeafen,
    Leave,
    CopyGrant,
}

impl VoiceAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Join => "Join this voice channel",
            Self::Answer => "Answer the call",
            Self::Decline => "Turn the call down",
            Self::StartCall => "Ring this conversation",
            Self::Mute => "Mute yourself",
            Self::Unmute => "Unmute yourself",
            Self::Deafen => "Deafen yourself",
            Self::Undeafen => "Undeafen yourself",
            Self::Leave => "Leave",
            Self::CopyGrant => "Copy the connection details",
        }
    }
}

/// Somebody the reader could start talking to.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub user: UserPartialResponse,
    /// Where the client knows them from, for the row's second column.
    pub note: String,
}
#[derive(Debug)]
pub struct ProfileView {
    pub user_id: String,
    /// The guild the message was in: nickname, roles and guild profile
    /// come from there.
    pub guild_id: Option<String>,
    /// What was known before the fetch answered (the message's author),
    /// replaced by the profile's user when it arrives.
    pub user: UserPartialResponse,
    pub state: ProfileState,
    pub scroll: u16,
}

/// One row of the file picker.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: std::path::PathBuf,
    pub is_dir: bool,
    pub size: u64,
}

/// The file picker: a directory's entries, filtered as the user types.
#[derive(Debug)]
pub struct FilePicker {
    pub dir: std::path::PathBuf,
    pub entries: Vec<FileEntry>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub query: String,
}

impl FilePicker {
    /// The entry under the cursor.
    pub fn current(&self) -> Option<&FileEntry> {
        self.filtered
            .get(self.selected)
            .and_then(|&i| self.entries.get(i))
    }
}

/// Size of a staged picture's thumbnail in the compose box, in cells.
pub const THUMB_COLS: u16 = 16;
pub const THUMB_ROWS: u16 = 4;

#[derive(Debug)]
pub enum ProfileState {
    Loading,
    Ready(Box<crate::api::types::UserProfileResponse>),
    Failed(String),
}

#[derive(Debug)]
pub struct App {
    pub discovery: WellKnownFluxerResponse,
    pub me: UserPrivateResponse,
    pub user_settings: Option<UserSettingsResponse>,
    pub user_guild_settings: HashMap<Snowflake, UserGuildSettingsResponse>,
    pub guilds: Vec<GuildResponse>,
    pub private_channels: Vec<ChannelResponse>,
    pub guild_channels: HashMap<Snowflake, Vec<ChannelResponse>>,
    pub guild_members: HashMap<Snowflake, Vec<GuildMemberResponse>>,
    /// Per channel, sorted oldest first. Shared so that a draw can hold the
    /// list without copying it; a write clones only while a draw holds it.
    pub messages: HashMap<Snowflake, std::rc::Rc<Vec<MessageResponse>>>,
    /// Bumped by every change to `messages`.
    pub messages_version: u64,
    pub user_cache: HashMap<Snowflake, UserPartialResponse>,
    pub voice_states: HashMap<Snowflake, HashMap<Snowflake, VoiceStateResponse>>,
    pub guild_emojis: HashMap<Snowflake, Vec<crate::api::types::GuildEmojiResponse>>,
    pub guild_stickers: HashMap<Snowflake, Vec<crate::api::types::GuildStickerResponse>>,
    pub guild_roles: HashMap<Snowflake, Vec<crate::api::types::GuildRoleResponse>>,
    pub emoji_autocomplete: Option<EmojiAutocomplete>,
    pub mention_autocomplete: Option<MentionAutocomplete>,
    pub command_autocomplete: Option<CommandAutocomplete>,
    pub selected_server: ServerSelection,
    pub selected_channel_id: Option<String>,
    pub focus: Focus,
    /// The compose text before the cursor. Typing and the autocompletes
    /// work on its end; see `compose.rs` for the cursor model.
    pub input: String,
    /// The compose text after the cursor.
    pub input_tail: String,
    /// Selection anchor as a byte offset into the full compose text; the
    /// selection runs between it and the cursor.
    pub input_anchor: Option<usize>,
    /// Ctrl+Space set a mark: plain movement keys extend the selection.
    pub input_mark: bool,
    pub input_history: Vec<crate::compose::InputSnapshot>,
    pub input_redo: Vec<crate::compose::InputSnapshot>,
    pub input_last_edit: Option<crate::compose::InputEditKind>,
    /// Text cut or copied from the compose box (Alt+V puts it back).
    pub cut_buffer: String,
    /// Files staged with Ctrl+V or /attach, uploaded with the next message.
    pub pending_attachments: Vec<crate::media::StagedAttachment>,
    /// Stickers staged with the picker, sent with the next message.
    pub pending_stickers: Vec<StagedSticker>,
    pub message_scroll_from_bottom: u16,
    pub message_scroll_max: u16,
    pub selected_message_index: Option<usize>,
    pub reply_to: Option<ReplyState>,
    pub forward_mode: bool,
    pub read_states: HashMap<Snowflake, ReadState>,
    pub typing_users: HashMap<Snowflake, HashMap<Snowflake, Instant>>,
    /// The user's own typing, told to the channel; see [`OwnTyping`].
    pub own_typing: OwnTyping,
    /// The pings overlay while it is open.
    pub pings: Option<PingsView>,
    /// The channels the reader has been in, oldest first, for Alt+Left
    /// and Alt+Right. Capped; see [`App::CHANNEL_HISTORY_MAX`].
    pub channel_history: Vec<(ServerSelection, String)>,
    /// Where in that list the reader is. Walking it does not add to it.
    pub channel_history_pos: usize,
    /// Set while a history step is being applied, so the step is not
    /// recorded as a new visit.
    pub walking_history: bool,
    /// The last community the reader was in, so Alt+L can go back to it
    /// from the conversation list.
    pub last_guild: Option<String>,
    /// Which channel was active last time the client looked, so a change
    /// can be noticed in one place rather than at every call site that
    /// moves the reader.
    pub last_active_channel: Option<String>,
    /// Where the "new messages" line sits in a channel: the id of the
    /// last message that was read when the channel was opened. Kept while
    /// the reader stays, so the line does not slide away under them.
    pub unread_anchor: HashMap<String, String>,
    /// The member list pane while it is open, and what has arrived for it.
    pub member_list: Option<MemberList>,
    /// Who is online, by user id, from READY and PRESENCE_UPDATE. An
    /// account with no entry has never been heard of and counts as
    /// offline; the server only sends presences for people the reader
    /// shares something with.
    pub presences: HashMap<String, PresenceEntry>,
    /// Bumped on every presence change. The message pane caches its
    /// layout by a key, and a presence decides whether an author gets a
    /// dot, so the key has to move when a presence does or the pane keeps
    /// the old one.
    pub presence_version: u64,

    /// The friends overlay while it is open.
    pub friends: Option<FriendsView>,
    /// Everybody the reader has a tie to, by their user id: friends,
    /// requests both ways, and blocked accounts.
    pub relationships: HashMap<String, RelationshipResponse>,
    /// Bumped on every relationship change. Blocking hides somebody's
    /// messages, so the message pane's cached layout has to be dropped
    /// when a block goes on or comes off.
    pub relationships_version: u64,
    /// The message actions menu while it is open.
    pub message_actions: Option<MessageActionsView>,
    /// The channel menu while it is open.
    pub channel_admin: Option<ChannelAdminView>,
    /// The pinned-messages overlay while it is open.
    pub pins: Option<PinsView>,
    /// The bookmarked-messages overlay while it is open.
    pub saved: Option<SavedView>,
    /// The who-reacted overlay while it is open.
    pub reaction_users: Option<ReactionUsersView>,
    /// Messages the user has bookmarked, so a message can be shown as
    /// bookmarked without asking the server. Filled by the saved list and
    /// kept up by the SAVED_MESSAGE_CREATE and _DELETE events.
    pub saved_message_ids: HashSet<String>,
    /// Messages marked with `m` for a bulk delete, per channel.
    pub marked_messages: HashMap<String, HashSet<String>>,
    /// Channels whose pins have changed since they were last looked at,
    /// from CHANNEL_PINS_UPDATE.
    pub channels_with_new_pins: HashSet<String>,
    /// The search overlay while it is open.
    pub search: Option<SearchView>,
    /// Channels a search answer named that the client has no other copy
    /// of, so a hit from a community the reader has not opened can still
    /// say where it came from.
    pub search_channels: HashMap<String, ChannelResponse>,
    /// Starting or managing a conversation, while that overlay is open.
    pub conversation: Option<ConversationView>,
    /// The group somebody is being picked for, when the people list was
    /// opened from a group's "Add somebody" rather than on its own.
    pub pending_group_add: Option<String>,
    /// Conversations the reader has pinned to the top of the list, from
    /// USER_PINNED_DMS_UPDATE.
    pub pinned_dms: HashSet<String>,
    /// Joining, making and leaving communities, while that is open.
    pub community: Option<CommunityView>,
    /// How many the directory said matched the last search, which can be
    /// more than the page it handed back.
    pub discover_total: u32,
    /// The voice menu while it is open.
    pub voice_menu: Option<VoiceView>,
    /// The voice channel this session is in, if any.
    pub voice: Option<VoiceConnection>,
    /// Conversations ringing at the moment, by channel.
    pub incoming_calls: HashMap<String, IncomingCall>,
    /// A message to select once its channel's history is loaded:
    /// (channel, message), set by a jump from the pings overlay.
    pub pending_jump: Option<(String, String)>,
    /// Older pages fetched for the jump so far.
    pub pending_jump_pages: u32,
    pub gateway_status: GatewayStatus,
    /// A READY (or a RESUMED) has been seen on the current connection, so
    /// what the gateway sends with a community has arrived.
    pub gateway_ready_seen: bool,
    pub gateway_lazy_guild_id: Option<String>,
    pub status_message: String,
    status_message_until: Option<Instant>,
    pub should_quit: bool,
    pub should_logout: bool,
    pub loading_channels: HashSet<String>,
    pub loading_members: HashSet<String>,
    pub guild_members_synced: HashSet<String>,
    pub api_backoff_until: HashMap<String, Instant>,
    pub loading_messages: HashSet<String>,
    /// Channels whose history has been fetched. `messages` alone cannot
    /// say: a message arriving over the gateway makes an entry for a
    /// channel that was never opened, and that entry must not stop the
    /// fetch when the channel is.
    pub messages_loaded: HashSet<String>,
    pub loading_emojis: HashSet<String>,
    pub loading_stickers: HashSet<String>,
    pub loading_roles: HashSet<String>,
    pub guild_roles_forbidden: HashSet<String>,
    /// Communities that answered the member list request with 403: asking
    /// again would only fail again.
    pub guild_members_forbidden: HashSet<String>,
    pub messages_older_exhausted: HashSet<String>,
    pub loading_older_messages: HashSet<String>,
    pub show_help: bool,
    pub help_scroll: u16,
    /// The debug panel (`/debug`, F12).
    pub show_debug: bool,
    pub debug_scroll: u16,
    /// A map of the next frame goes to the debug log (`/debug frame`, f
    /// in the panel).
    pub debug_frame_wanted: bool,
    /// What was known at start, for the debug panel: version, terminal,
    /// picture protocol, cell size. Nothing personal.
    pub debug_facts: Vec<(String, String)>,
    /// How long the last frame took to draw.
    pub last_frame_ms: u32,
    /// Whether the terminal can hold a frame back until it is whole (DEC
    /// mode 2026). One that cannot draws each picture as it arrives, so it
    /// must not be asked for animation frames quickly. Assumed until the
    /// terminal says otherwise, so nothing changes where it is not asked.
    pub synchronized_output: bool,
    pub started_at: Instant,
    pub channel_picker: Option<ChannelPicker>,
    pub reaction_target: Option<(String, String)>,
    pub edit_target: Option<EditState>,
    pub input_bar_anim_phase: u8,
    pub input_bar_anim_slow: u8,
    pub image_preview: Option<ImagePreviewState>,
    pub chafa_viewport: (u16, u16),
    pub chafa_preview_cells: (u16, u16),
    pub image_picker: Option<Picker>,
    /// Console mode: pictures are blitted by our own renderer instead of a
    /// terminal graphics protocol.
    pub pixel_mode: bool,
    /// Console mode: where this frame's pictures go (shared with the backend).
    pub pixel_placements: crate::console::backend::SharedPlacements,
    /// Custom (guild) emoji images by id, encoded for the terminal's protocol.
    pub custom_emojis: HashMap<String, CustomEmojiState>,
    /// Per frame: which emoji id each marker slot in the message pane refers to.
    pub custom_emoji_slots: RefCell<Vec<String>>,
    /// Per frame: (id, animated) the renderer met but has no image for yet.
    pub custom_emoji_wanted: RefCell<Vec<(String, bool)>>,
    /// Pictures under messages and avatars, ready to draw, within a byte budget.
    pub media: crate::media::MediaCache,
    /// Per frame: the block behind each media marker slot.
    pub media_slots: RefCell<Vec<MediaSlot>>,
    /// Per frame: blocks on screen with nothing loaded for them yet.
    pub media_wanted: RefCell<Vec<MediaSlot>>,
    /// Per frame: whether an animated block was drawn (so the next tick redraws).
    pub media_animation_seen: std::cell::Cell<bool>,
    /// Pixel size of one cell: the terminal's font size or the console glyph size.
    pub cell_px: (u32, u32),
    /// Downloaded pictures kept between runs.
    pub disk_cache: Option<std::sync::Arc<crate::media::DiskCache>>,
    /// Per frame: the escape sequences to print at each picture's sentinel
    /// cell (shared with the terminal backend).
    pub terminal_pictures: crate::console::backend::SharedPictures,
    /// Per frame: the rendered cells and the pane's scroll, for the
    /// terminal backend to reconcile against what the terminal shows.
    pub terminal_frame: crate::console::backend::SharedFrame,
    /// The message pane's lines as last built, reused while nothing that
    /// shows in them changed.
    pub pane_layout: RefCell<Option<std::rc::Rc<crate::ui::message_pane::PaneLayout>>>,
    /// Bumped when names, nicknames, roles or members change.
    pub roster_version: u64,
    /// Bumped when a custom emoji's picture arrives or fails.
    pub custom_emoji_version: u64,
    /// The message pane as last drawn, to tell a plain scroll from a change.
    pub pane_last: Option<PaneView>,
    /// Where the reader is while scrolled up, kept still as content changes.
    pub pane_anchor: Option<PaneAnchor>,
    /// Set by the message pane when it merely scrolled since the last draw.
    pub pane_scroll_hint: Option<crate::console::backend::RegionScroll>,
    /// Per frame: the shortest frame delay of an animation drawn, which
    /// sets the pace of the ticks.
    pub animation_delay_seen: std::cell::Cell<Option<Duration>>,
    /// Counts UI draws; animated emoji decide their frame once per draw.
    pub draw_serial: std::cell::Cell<u64>,
    pub show_settings: bool,
    pub settings_cursor: usize,
    pub show_server_notifications: bool,
    pub server_notification_cursor: usize,
    pub server_notification_scroll: u16,
    pub ui_settings: UiSettings,
    /// The profile popup, while open.
    pub profile: Option<ProfileView>,
    /// The file picker, while open.
    pub file_picker: Option<FilePicker>,
    /// The sticker picker, while open.
    pub sticker_picker: Option<StickerPicker>,
    /// Where the file picker last was, for the next time.
    pub attach_dir: Option<std::path::PathBuf>,
    /// The external player an audio attachment is playing through.
    pub audio: Option<crate::media::Player>,
    /// `[media] audio_player`: empty picks a player from PATH.
    pub audio_player_cmd: String,
    /// Whether the terminal window (or the VT, in console mode) is the one
    /// the user is looking at; true until the terminal says otherwise.
    pub window_focused: bool,
}

impl App {
    const DM_SETTINGS_KEY: &'static str = "@me";
    const SERVER_MUTE_PRESET_MS: [u64; 5] = [
        15 * 60 * 1000,
        60 * 60 * 1000,
        3 * 60 * 60 * 1000,
        8 * 60 * 60 * 1000,
        24 * 60 * 60 * 1000,
    ];

    #[allow(clippy::too_many_arguments)] // everything READY brings, each of it needed
    pub fn new(
        discovery: WellKnownFluxerResponse,
        me: UserPrivateResponse,
        user_settings: Option<UserSettingsResponse>,
        guilds: Vec<GuildResponse>,
        private_channels: Vec<ChannelResponse>,
        selected_server: ServerSelection,
        selected_channel_id: Option<String>,
        ui_settings: UiSettings,
    ) -> Self {
        let mut user_cache = HashMap::new();
        merge_user_cache(
            &mut user_cache,
            private_channels
                .iter()
                .flat_map(|channel| channel.recipients.clone()),
        );

        let mut app = Self {
            discovery,
            me,
            user_settings,
            user_guild_settings: HashMap::new(),
            guilds,
            private_channels,
            guild_channels: HashMap::new(),
            guild_members: HashMap::new(),
            messages: HashMap::new(),
            messages_version: 0,
            user_cache,
            voice_states: HashMap::new(),
            guild_emojis: HashMap::new(),
            guild_stickers: HashMap::new(),
            guild_roles: HashMap::new(),
            emoji_autocomplete: None,
            mention_autocomplete: None,
            command_autocomplete: None,
            selected_server,
            selected_channel_id,
            focus: Focus::Channels,
            input: String::new(),
            input_tail: String::new(),
            input_anchor: None,
            input_mark: false,
            input_history: Vec::new(),
            input_redo: Vec::new(),
            input_last_edit: None,
            cut_buffer: String::new(),
            pending_attachments: Vec::new(),
            pending_stickers: Vec::new(),
            message_scroll_from_bottom: 0,
            message_scroll_max: 0,
            selected_message_index: None,
            reply_to: None,
            forward_mode: false,
            read_states: HashMap::new(),
            typing_users: HashMap::new(),
            own_typing: OwnTyping::default(),
            pings: None,
            channel_history: Vec::new(),
            channel_history_pos: 0,
            walking_history: false,
            last_guild: None,
            last_active_channel: None,
            unread_anchor: HashMap::new(),
            member_list: None,
            presences: HashMap::new(),
            presence_version: 0,

            friends: None,
            relationships: HashMap::new(),
            relationships_version: 0,
            message_actions: None,
            channel_admin: None,
            pins: None,
            saved: None,
            reaction_users: None,
            saved_message_ids: HashSet::new(),
            marked_messages: HashMap::new(),
            channels_with_new_pins: HashSet::new(),
            search: None,
            search_channels: HashMap::new(),
            conversation: None,
            pending_group_add: None,
            pinned_dms: HashSet::new(),
            community: None,
            discover_total: 0,
            voice_menu: None,
            voice: None,
            incoming_calls: HashMap::new(),
            pending_jump: None,
            pending_jump_pages: 0,
            gateway_status: GatewayStatus::Disconnected,
            gateway_ready_seen: false,
            gateway_lazy_guild_id: None,
            status_message: String::new(),
            status_message_until: None,
            should_quit: false,
            should_logout: false,
            loading_channels: HashSet::new(),
            loading_members: HashSet::new(),
            guild_members_synced: HashSet::new(),
            api_backoff_until: HashMap::new(),
            loading_messages: HashSet::new(),
            messages_loaded: HashSet::new(),
            loading_emojis: HashSet::new(),
            loading_stickers: HashSet::new(),
            loading_roles: HashSet::new(),
            guild_roles_forbidden: HashSet::new(),
            guild_members_forbidden: HashSet::new(),
            messages_older_exhausted: HashSet::new(),
            loading_older_messages: HashSet::new(),
            show_help: false,
            help_scroll: 0,
            show_debug: false,
            debug_scroll: 0,
            debug_frame_wanted: false,
            debug_facts: Vec::new(),
            last_frame_ms: 0,
            synchronized_output: true,
            started_at: Instant::now(),
            channel_picker: None,
            reaction_target: None,
            edit_target: None,
            input_bar_anim_phase: 0,
            input_bar_anim_slow: 0,
            image_preview: None,
            chafa_viewport: (80, 22),
            chafa_preview_cells: (100, 40),
            image_picker: None,
            pixel_mode: false,
            pixel_placements: std::rc::Rc::new(RefCell::new(Vec::new())),
            custom_emojis: HashMap::new(),
            custom_emoji_slots: RefCell::new(Vec::new()),
            custom_emoji_wanted: RefCell::new(Vec::new()),
            media: crate::media::MediaCache::new(64 << 20),
            media_slots: RefCell::new(Vec::new()),
            media_wanted: RefCell::new(Vec::new()),
            media_animation_seen: std::cell::Cell::new(false),
            cell_px: (8, 16),
            disk_cache: None,
            terminal_pictures: std::rc::Rc::new(RefCell::new(HashMap::new())),
            terminal_frame: std::rc::Rc::new(RefCell::new(Default::default())),
            pane_layout: RefCell::new(None),
            roster_version: 0,
            custom_emoji_version: 0,
            pane_last: None,
            pane_anchor: None,
            pane_scroll_hint: None,
            animation_delay_seen: std::cell::Cell::new(None),
            draw_serial: std::cell::Cell::new(0),
            show_settings: false,
            settings_cursor: 0,
            show_server_notifications: false,
            server_notification_cursor: 0,
            server_notification_scroll: 0,
            ui_settings,
            profile: None,
            file_picker: None,
            sticker_picker: None,
            attach_dir: None,
            audio: None,
            window_focused: true,
            audio_player_cmd: String::new(),
        };
        app.normalize_selection();
        app
    }

    pub const UI_SETTINGS_LAST_ROW: usize = 8;
    pub const SERVER_NOTIFICATION_LAST_ROW: usize = 5;
    pub const HISTORY_AUTOLOAD_THRESHOLD_ROWS: u16 = 3;
    pub const TRANSIENT_STATUS_DURATION: Duration = Duration::from_millis(1800);

    fn user_guild_settings_key(guild_id: Option<&str>) -> String {
        guild_id.unwrap_or(Self::DM_SETTINGS_KEY).to_string()
    }

    fn default_user_guild_settings(guild_id: Option<&str>) -> UserGuildSettingsResponse {
        UserGuildSettingsResponse {
            guild_id: guild_id.map(str::to_string),
            message_notifications: if guild_id.is_some() {
                MESSAGE_NOTIFICATIONS_INHERIT
            } else {
                MESSAGE_NOTIFICATIONS_ALL_MESSAGES
            },
            muted: false,
            mute_config: None,
            mobile_push: guild_id.is_some(),
            suppress_everyone: false,
            suppress_roles: false,
            hide_muted_channels: false,
            channel_overrides: HashMap::new(),
            version: 0,
        }
    }

    fn mute_active(muted: bool, mute_config: Option<&UserGuildMuteConfig>) -> bool {
        if !muted {
            return false;
        }
        let Some(end_time) = mute_config.and_then(|config| config.end_time.as_deref()) else {
            return true;
        };
        chrono::DateTime::parse_from_rfc3339(end_time)
            .map(|deadline| deadline.with_timezone(&chrono::Utc) > chrono::Utc::now())
            .unwrap_or(true)
    }

    fn sanitize_user_guild_settings(
        mut settings: UserGuildSettingsResponse,
    ) -> UserGuildSettingsResponse {
        if !Self::mute_active(settings.muted, settings.mute_config.as_ref()) {
            settings.muted = false;
            settings.mute_config = None;
        }
        settings.channel_overrides.retain(|_, override_settings| {
            if Self::mute_active(
                override_settings.muted,
                override_settings.mute_config.as_ref(),
            ) {
                true
            } else {
                override_settings.muted = false;
                override_settings.mute_config = None;
                true
            }
        });
        settings
    }

    pub fn selected_server_name(&self) -> String {
        match &self.selected_server {
            ServerSelection::DirectMessages => "Direct Messages".to_string(),
            ServerSelection::Guild(id) => self
                .guilds
                .iter()
                .find(|guild| guild.id == *id)
                .map(|guild| guild.name.clone())
                .unwrap_or_else(|| id.clone()),
        }
    }

    pub fn selected_server_guild_id(&self) -> Option<String> {
        match &self.selected_server {
            ServerSelection::DirectMessages => None,
            ServerSelection::Guild(id) => Some(id.clone()),
        }
    }

    fn user_guild_settings_for(&self, guild_id: Option<&str>) -> UserGuildSettingsResponse {
        self.user_guild_settings
            .get(Self::user_guild_settings_key(guild_id).as_str())
            .cloned()
            .map(Self::sanitize_user_guild_settings)
            .unwrap_or_else(|| Self::default_user_guild_settings(guild_id))
    }

    fn user_guild_settings_mut_or_default(
        &mut self,
        guild_id: Option<&str>,
    ) -> &mut UserGuildSettingsResponse {
        let key = Self::user_guild_settings_key(guild_id);
        self.user_guild_settings
            .entry(key)
            .or_insert_with(|| Self::default_user_guild_settings(guild_id))
    }

    pub fn selected_server_notification_settings(&self) -> Option<UserGuildSettingsResponse> {
        let guild_id = self.selected_server_guild_id()?;
        Some(self.user_guild_settings_for(Some(&guild_id)))
    }

    pub fn set_user_guild_settings(&mut self, settings: Vec<UserGuildSettingsResponse>) {
        self.user_guild_settings.clear();
        for settings_entry in settings {
            self.upsert_user_guild_settings(settings_entry);
        }
    }

    pub fn upsert_user_guild_settings(&mut self, settings: UserGuildSettingsResponse) {
        let key = Self::user_guild_settings_key(settings.guild_id.as_deref());
        self.user_guild_settings
            .insert(key, Self::sanitize_user_guild_settings(settings));
        self.normalize_selection();
    }

    pub fn apply_user_guild_settings_patch(
        &mut self,
        guild_id: Option<&str>,
        patch: &UserGuildSettingsPatch,
    ) {
        let settings = self.user_guild_settings_mut_or_default(guild_id);
        if let Some(value) = patch.message_notifications {
            settings.message_notifications = value;
        }
        if let Some(value) = patch.muted {
            settings.muted = value;
        }
        if let Some(value) = &patch.mute_config {
            settings.mute_config = value.clone();
        }
        if let Some(value) = patch.mobile_push {
            settings.mobile_push = value;
        }
        if let Some(value) = patch.suppress_everyone {
            settings.suppress_everyone = value;
        }
        if let Some(value) = patch.suppress_roles {
            settings.suppress_roles = value;
        }
        if let Some(value) = patch.hide_muted_channels {
            settings.hide_muted_channels = value;
        }

        let sanitized = Self::sanitize_user_guild_settings(settings.clone());
        *settings = sanitized;
        self.normalize_selection();
    }

    pub fn open_server_notification_settings(&mut self) -> bool {
        let Some(_) = self.selected_server_guild_id() else {
            self.set_status("Select a community to edit its notification settings.");
            return false;
        };
        self.dismiss_image_preview();
        self.show_server_notifications = true;
        self.server_notification_cursor = 0;
        self.server_notification_scroll = 0;
        true
    }

    pub fn current_server_mute_choice_index(&self) -> Option<usize> {
        let settings = self.selected_server_notification_settings()?;
        if !Self::mute_active(settings.muted, settings.mute_config.as_ref()) {
            return Some(0);
        }
        let preset = settings
            .mute_config
            .as_ref()
            .and_then(|config| config.selected_time_window);
        if let Some(window) = preset
            && let Some(index) = Self::SERVER_MUTE_PRESET_MS
                .iter()
                .position(|candidate| *candidate == window)
        {
            return Some(index + 1);
        }
        Some(Self::SERVER_MUTE_PRESET_MS.len() + 1)
    }

    pub fn cycle_server_notification_setting(
        &mut self,
        delta: i32,
    ) -> Option<(String, UserGuildSettingsPatch)> {
        let guild_id = self.selected_server_guild_id()?;
        let settings = self.user_guild_settings_for(Some(&guild_id));

        let patch = match self.server_notification_cursor {
            0 => {
                let option_count = Self::SERVER_MUTE_PRESET_MS.len() as i32 + 2;
                let current = self.current_server_mute_choice_index()? as i32;
                let next = (current + delta).rem_euclid(option_count) as usize;
                if next == 0 {
                    UserGuildSettingsPatch {
                        muted: Some(false),
                        mute_config: Some(None),
                        ..UserGuildSettingsPatch::default()
                    }
                } else if next == Self::SERVER_MUTE_PRESET_MS.len() + 1 {
                    UserGuildSettingsPatch {
                        muted: Some(true),
                        mute_config: Some(None),
                        ..UserGuildSettingsPatch::default()
                    }
                } else {
                    let window = Self::SERVER_MUTE_PRESET_MS[next - 1];
                    UserGuildSettingsPatch {
                        muted: Some(true),
                        mute_config: Some(Some(UserGuildMuteConfig {
                            end_time: Some(
                                (chrono::Utc::now()
                                    + chrono::Duration::milliseconds(window as i64))
                                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                            ),
                            selected_time_window: Some(window),
                        })),
                        ..UserGuildSettingsPatch::default()
                    }
                }
            }
            1 => {
                let options = [
                    MESSAGE_NOTIFICATIONS_ALL_MESSAGES,
                    MESSAGE_NOTIFICATIONS_ONLY_MENTIONS,
                    MESSAGE_NOTIFICATIONS_NO_MESSAGES,
                ];
                let current = self.resolved_message_notifications_for_guild(&guild_id);
                let current_index = options
                    .iter()
                    .position(|candidate| *candidate == current)
                    .unwrap_or(0) as i32;
                let next = (current_index + delta).rem_euclid(options.len() as i32) as usize;
                UserGuildSettingsPatch {
                    message_notifications: Some(options[next]),
                    ..UserGuildSettingsPatch::default()
                }
            }
            2 => UserGuildSettingsPatch {
                suppress_everyone: Some(!settings.suppress_everyone),
                ..UserGuildSettingsPatch::default()
            },
            3 => UserGuildSettingsPatch {
                suppress_roles: Some(!settings.suppress_roles),
                ..UserGuildSettingsPatch::default()
            },
            4 => UserGuildSettingsPatch {
                hide_muted_channels: Some(!settings.hide_muted_channels),
                ..UserGuildSettingsPatch::default()
            },
            5 => UserGuildSettingsPatch {
                mobile_push: Some(!settings.mobile_push),
                ..UserGuildSettingsPatch::default()
            },
            _ => return None,
        };

        self.apply_user_guild_settings_patch(Some(&guild_id), &patch);
        Some((guild_id, patch))
    }

    pub fn toggle_settings_selection(&mut self) {
        match self.settings_cursor {
            0 => {
                self.ui_settings.clock_12h = !self.ui_settings.clock_12h;
            }
            1 => {
                self.ui_settings.show_typing_indicators = !self.ui_settings.show_typing_indicators;
            }
            2 => {
                self.ui_settings.send_typing = !self.ui_settings.send_typing;
            }
            3 => {
                self.ui_settings.performance_mode = !self.ui_settings.performance_mode;
            }
            4 => {
                use crate::config::Theme;
                self.ui_settings.theme = match self.ui_settings.theme {
                    Theme::Terminal => Theme::Fluxer,
                    Theme::Fluxer => Theme::Terminal,
                };
                crate::ui::theme::set_terminal_theme(self.ui_settings.theme == Theme::Terminal);
            }
            5 => {
                self.ui_settings.inline_media = !self.ui_settings.inline_media;
            }
            6 => {
                self.ui_settings.avatars = !self.ui_settings.avatars;
            }
            7 => {
                use crate::config::NotifyMode;
                self.ui_settings.notifications = match self.ui_settings.notifications {
                    NotifyMode::Auto => NotifyMode::Desktop,
                    NotifyMode::Desktop => NotifyMode::Mail,
                    NotifyMode::Mail => NotifyMode::Off,
                    NotifyMode::Off => NotifyMode::Auto,
                };
            }
            8 => {
                self.ui_settings.notify_sound = !self.ui_settings.notify_sound;
            }
            _ => {}
        }
    }

    /// What to announce about a message that just arrived, if anything: a
    /// direct message or a mention, or, when asked for, any message in a
    /// community channel set to all messages; never the user's own, and
    /// nothing for the channel being read while the window is focused
    /// (the message is on screen). A message in that channel is announced
    /// again once the terminal window, or the VT, is out of sight.
    pub fn notification_for(
        &self,
        message: &MessageResponse,
    ) -> Option<crate::notify::Notification> {
        if message.author.id == self.me.id {
            return None;
        }
        let channel = self.channel_by_id(&message.channel_id);
        let reading_it = self.active_channel_id().as_deref() == Some(message.channel_id.as_str());
        if reading_it && self.window_focused {
            return None;
        }
        let wanted = self.message_notifies_me(message)
            || (self.ui_settings.notify_all_messages
                && channel.as_ref().is_some_and(|c| {
                    c.guild_id.is_some()
                        && self.channel_notification_visibility(c)
                            == NotificationVisibility::AllMessages
                }));
        if !wanted {
            return None;
        }
        let guild_id = channel.as_ref().and_then(|c| c.guild_id.clone());
        let author = self.shown_name_for_user(guild_id.as_deref(), &message.author);
        let place = match (&channel, &guild_id) {
            (Some(c), Some(gid)) => {
                let guild = self
                    .guilds
                    .iter()
                    .find(|g| &g.id == gid)
                    .map(|g| g.name.clone())
                    .unwrap_or_default();
                format!("#{} \u{00B7} {}", c.name, guild)
            }
            _ => "Direct message".to_string(),
        };
        let title = match (&channel, &guild_id) {
            (Some(c), Some(_)) => format!("{author} in #{}", c.name),
            _ => format!("{author} (direct message)"),
        };
        let mut body: String = crate::ui::message_markdown::content_lines(&message.content, self)
            .iter()
            .map(|spans| spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        const MAX: usize = 300;
        if body.chars().count() > MAX {
            body = body.chars().take(MAX).collect::<String>() + "\u{2026}";
        }
        if !message.attachments.is_empty() {
            let n = message.attachments.len();
            let files = if n == 1 {
                message.attachments[0].filename.clone()
            } else {
                format!("{n} files")
            };
            if body.is_empty() {
                body = format!("[{files}]");
            } else {
                body = format!("{body} [{files}]");
            }
        }
        if !message.stickers.is_empty() {
            let n = message.stickers.len();
            let stickers = if n == 1 {
                format!("sticker: {}", message.stickers[0].name)
            } else {
                format!("{n} stickers")
            };
            if body.is_empty() {
                body = format!("[{stickers}]");
            } else {
                body = format!("{body} [{stickers}]");
            }
        }
        if body.is_empty() {
            body = "(no text)".to_string();
        }
        Some(crate::notify::Notification { title, body, place })
    }

    pub fn suppress_everyone_enabled(&self, guild_id: Option<&str>) -> bool {
        guild_id
            .map(|id| self.user_guild_settings_for(Some(id)).suppress_everyone)
            .unwrap_or(false)
    }

    pub fn suppress_roles_enabled(&self, guild_id: Option<&str>) -> bool {
        guild_id
            .map(|id| self.user_guild_settings_for(Some(id)).suppress_roles)
            .unwrap_or(false)
    }

    pub fn hide_muted_channels_enabled(&self, guild_id: &str) -> bool {
        self.user_guild_settings_for(Some(guild_id))
            .hide_muted_channels
    }

    pub fn guild_is_muted(&self, guild_id: Option<&str>) -> bool {
        let settings = self.user_guild_settings_for(guild_id);
        Self::mute_active(settings.muted, settings.mute_config.as_ref())
    }

    pub fn channel_override(
        &self,
        guild_id: Option<&str>,
        channel_id: &str,
    ) -> Option<UserGuildChannelOverride> {
        self.user_guild_settings_for(guild_id)
            .channel_overrides
            .get(channel_id)
            .cloned()
    }

    pub fn channel_is_muted_directly(&self, channel: &ChannelResponse) -> bool {
        self.channel_override(channel.guild_id.as_deref(), &channel.id)
            .is_some_and(|override_settings| {
                Self::mute_active(
                    override_settings.muted,
                    override_settings.mute_config.as_ref(),
                )
            })
    }

    pub fn channel_parent_is_muted(&self, channel: &ChannelResponse) -> bool {
        let Some(parent_id) = channel.parent_id.as_deref() else {
            return false;
        };
        self.channel_override(channel.guild_id.as_deref(), parent_id)
            .is_some_and(|override_settings| {
                Self::mute_active(
                    override_settings.muted,
                    override_settings.mute_config.as_ref(),
                )
            })
    }

    pub fn channel_is_muted_effective(&self, channel: &ChannelResponse) -> bool {
        self.guild_is_muted(channel.guild_id.as_deref())
            || self.channel_parent_is_muted(channel)
            || self.channel_is_muted_directly(channel)
    }

    pub fn resolved_message_notifications_for_guild(&self, guild_id: &str) -> i32 {
        let settings = self.user_guild_settings_for(Some(guild_id));
        if settings.message_notifications != MESSAGE_NOTIFICATIONS_INHERIT {
            return settings.message_notifications;
        }
        self.guilds
            .iter()
            .find(|guild| guild.id == guild_id)
            .map(|guild| guild.default_message_notifications)
            .unwrap_or(MESSAGE_NOTIFICATIONS_ALL_MESSAGES)
    }

    pub fn resolved_message_notifications(&self, channel: &ChannelResponse) -> i32 {
        let guild_id = channel.guild_id.as_deref();
        let Some(guild_id) = guild_id else {
            return self
                .channel_override(None, &channel.id)
                .map(|override_settings| override_settings.message_notifications)
                .filter(|level| *level != MESSAGE_NOTIFICATIONS_INHERIT)
                .unwrap_or(MESSAGE_NOTIFICATIONS_ALL_MESSAGES);
        };

        if let Some(override_settings) = self.channel_override(Some(guild_id), &channel.id)
            && override_settings.message_notifications != MESSAGE_NOTIFICATIONS_INHERIT
        {
            return override_settings.message_notifications;
        }

        if let Some(parent_id) = channel.parent_id.as_deref()
            && let Some(parent_override) = self.channel_override(Some(guild_id), parent_id)
            && parent_override.message_notifications != MESSAGE_NOTIFICATIONS_INHERIT
        {
            return parent_override.message_notifications;
        }

        self.resolved_message_notifications_for_guild(guild_id)
    }

    pub fn channel_notification_visibility(
        &self,
        channel: &ChannelResponse,
    ) -> NotificationVisibility {
        let level = self.resolved_message_notifications(channel);
        if level == MESSAGE_NOTIFICATIONS_NO_MESSAGES {
            return NotificationVisibility::None;
        }
        if self.channel_is_muted_effective(channel) || level == MESSAGE_NOTIFICATIONS_ONLY_MENTIONS
        {
            return NotificationVisibility::MentionsOnly;
        }
        NotificationVisibility::AllMessages
    }

    pub fn visible_channel_is_unread(&self, channel_id: &str) -> bool {
        let Some(channel) = self.channel_by_id(channel_id) else {
            return false;
        };
        self.channel_notification_visibility(channel) == NotificationVisibility::AllMessages
            && self.channel_is_unread(channel_id)
    }

    pub fn visible_channel_mention_count(&self, channel_id: &str) -> u64 {
        let Some(channel) = self.channel_by_id(channel_id) else {
            return 0;
        };
        match self.channel_notification_visibility(channel) {
            NotificationVisibility::None => 0,
            NotificationVisibility::AllMessages | NotificationVisibility::MentionsOnly => {
                self.channel_mention_count(channel_id)
            }
        }
    }

    fn channel_hidden_in_sidebar(&self, channel: &ChannelResponse) -> bool {
        let Some(guild_id) = channel.guild_id.as_deref() else {
            return false;
        };
        if !self.hide_muted_channels_enabled(guild_id) {
            return false;
        }
        if self.selected_channel_id.as_deref() == Some(channel.id.as_str()) {
            return false;
        }
        self.channel_is_muted_effective(channel)
    }

    pub const API_FAILURE_BACKOFF_SECS: u64 = 180;

    pub fn api_backoff_can_try(&self, key: &str) -> bool {
        self.api_backoff_until
            .get(key)
            .is_none_or(|until| Instant::now() >= *until)
    }

    pub fn api_backoff_after_failure(&mut self, key: impl Into<String>) {
        self.api_backoff_until.insert(
            key.into(),
            Instant::now() + Duration::from_secs(Self::API_FAILURE_BACKOFF_SECS),
        );
    }

    pub fn api_backoff_clear(&mut self, key: &str) {
        self.api_backoff_until.remove(key);
    }

    pub fn api_backoff_clear_guild(&mut self, guild_id: &str) {
        for prefix in ["members:", "channels:", "emojis:", "roles:"] {
            self.api_backoff_until
                .remove(&format!("{prefix}{guild_id}"));
        }
    }

    pub fn api_backoff_clear_channel_messages(&mut self, channel_id: &str) {
        self.api_backoff_until
            .remove(&format!("messages:{channel_id}"));
    }

    pub fn set_guild_emojis(
        &mut self,
        guild_id: &str,
        emojis: Vec<crate::api::types::GuildEmojiResponse>,
    ) {
        self.guild_emojis.insert(guild_id.to_string(), emojis);
        self.loading_emojis.remove(guild_id);
        self.api_backoff_clear(&format!("emojis:{guild_id}"));
    }

    pub fn set_guild_stickers(
        &mut self,
        guild_id: &str,
        stickers: Vec<crate::api::types::GuildStickerResponse>,
    ) {
        self.guild_stickers.insert(guild_id.to_string(), stickers);
        self.loading_stickers.remove(guild_id);
        self.api_backoff_clear(&format!("stickers:{guild_id}"));
        // the picker holds a copy of the rows: refresh it while it is
        // open, keeping the cursor and any search where they were
        if let Some(picker) = &self.sticker_picker {
            let query = picker.query.clone();
            let at = picker.selected;
            let searching = picker.searching;
            let before = picker.query_before_search.clone();
            self.open_sticker_picker(&query);
            if let Some(picker) = self.sticker_picker.as_mut() {
                picker.selected = at.min(picker.filtered.len().saturating_sub(1));
                picker.searching = searching;
                picker.query_before_search = before;
            }
        }
    }

    pub fn set_guild_roles(
        &mut self,
        guild_id: &str,
        roles: Vec<crate::api::types::GuildRoleResponse>,
    ) {
        self.roster_version = self.roster_version.wrapping_add(1);
        self.guild_roles.insert(guild_id.to_string(), roles);
        self.loading_roles.remove(guild_id);
        self.api_backoff_clear(&format!("roles:{guild_id}"));
    }

    pub fn merge_guild_roles_from_gateway(
        &mut self,
        guild_id: &str,
        incoming: Vec<crate::api::types::GuildRoleResponse>,
    ) {
        self.roster_version = self.roster_version.wrapping_add(1);
        if incoming.is_empty() {
            return;
        }
        let entry = self.guild_roles.entry(guild_id.to_string()).or_default();
        for r in incoming {
            if r.id.is_empty() {
                continue;
            }
            let id_trim = r.id.trim().to_string();
            if let Some(existing) = entry.iter_mut().find(|e| e.id.trim() == id_trim.as_str()) {
                *existing = r;
            } else {
                entry.push(r);
            }
        }
    }

    pub fn remove_guild_role(&mut self, guild_id: &str, role_id: &str) {
        self.roster_version = self.roster_version.wrapping_add(1);
        let Some(roles) = self.guild_roles.get_mut(guild_id) else {
            return;
        };
        let rid = role_id.trim();
        roles.retain(|r| r.id.trim() != rid);
    }

    pub fn server_entries(&self) -> Vec<ServerSelection> {
        let mut entries = vec![ServerSelection::DirectMessages];
        entries.extend(
            self.guilds
                .iter()
                .map(|guild| ServerSelection::Guild(guild.id.clone())),
        );
        entries
    }

    pub fn server_selected_index(&self) -> usize {
        self.server_entries()
            .iter()
            .position(|entry| entry == &self.selected_server)
            .unwrap_or_default()
    }

    pub fn move_server(&mut self, delta: i32) -> bool {
        let entries = self.server_entries();
        if entries.is_empty() {
            return false;
        }
        let current = self.server_selected_index() as i32;
        let next = (current + delta).clamp(0, entries.len() as i32 - 1) as usize;
        if entries[next] == self.selected_server {
            return false;
        }
        self.selected_server = entries[next].clone();
        self.selected_channel_id = None;
        self.message_scroll_from_bottom = 0;
        self.normalize_selection();
        true
    }

    pub fn all_channels_for_server(&self, server: &ServerSelection) -> Vec<ChannelResponse> {
        match server {
            ServerSelection::DirectMessages => {
                let mut dms = self.private_channels.clone();
                // pinned conversations sit above the rest; within each
                // half the newest message comes first
                dms.sort_by(|a, b| {
                    let recency = |c: &ChannelResponse| {
                        c.last_message_id
                            .as_deref()
                            .and_then(|id| id.parse::<u128>().ok())
                            .unwrap_or(0)
                    };
                    self.is_dm_pinned(&b.id)
                        .cmp(&self.is_dm_pinned(&a.id))
                        .then_with(|| recency(b).cmp(&recency(a)))
                });
                dms
            }
            ServerSelection::Guild(guild_id) => {
                let all = self
                    .guild_channels
                    .get(guild_id)
                    .cloned()
                    .unwrap_or_default();

                let mut categories: Vec<&ChannelResponse> = all
                    .iter()
                    .filter(|c| c.channel_type() == CHANNEL_GUILD_CATEGORY)
                    .collect();
                categories.sort_by_key(|c| c.position);

                let mut non_cat: Vec<&ChannelResponse> = all
                    .iter()
                    .filter(|c| c.channel_type() != CHANNEL_GUILD_CATEGORY)
                    .collect();
                non_cat.sort_by(|a, b| a.position.cmp(&b.position).then(a.name.cmp(&b.name)));

                let mut result: Vec<ChannelResponse> = Vec::new();

                let uncategorized: Vec<&ChannelResponse> = non_cat
                    .iter()
                    .filter(|c| c.parent_id.is_none())
                    .copied()
                    .collect();
                for ch in uncategorized {
                    result.push(ch.clone());
                }

                for cat in &categories {
                    let children: Vec<&ChannelResponse> = non_cat
                        .iter()
                        .filter(|c| c.parent_id.as_deref() == Some(cat.id.as_str()))
                        .copied()
                        .collect();
                    result.push((*cat).clone());
                    for ch in children {
                        result.push(ch.clone());
                    }
                }

                result
            }
        }
    }

    pub fn channels_for_server(&self, server: &ServerSelection) -> Vec<ChannelResponse> {
        let all = self.all_channels_for_server(server);
        let ServerSelection::Guild(guild_id) = server else {
            return all;
        };
        if !self.hide_muted_channels_enabled(guild_id) {
            return all;
        }

        let mut visible = Vec::new();
        for channel in all {
            if channel.channel_type() == CHANNEL_GUILD_CATEGORY {
                let has_visible_children = self
                    .guild_channels
                    .get(guild_id)
                    .into_iter()
                    .flat_map(|channels| channels.iter())
                    .any(|candidate| {
                        candidate.parent_id.as_deref() == Some(channel.id.as_str())
                            && !self.channel_hidden_in_sidebar(candidate)
                    });
                if has_visible_children {
                    visible.push(channel);
                }
                continue;
            }
            if !self.channel_hidden_in_sidebar(&channel) {
                visible.push(channel);
            }
        }
        visible
    }

    pub fn channel_entries(&self) -> Vec<ChannelResponse> {
        self.channels_for_server(&self.selected_server)
    }

    pub fn channel_selected_index(&self) -> usize {
        self.channel_entries()
            .iter()
            .position(|channel| Some(channel.id.as_str()) == self.selected_channel_id.as_deref())
            .unwrap_or_default()
    }

    pub fn move_channel(&mut self, delta: i32) -> bool {
        let channels = self.channel_entries();
        if channels.is_empty() {
            self.selected_channel_id = None;
            return false;
        }

        let current = self.channel_selected_index() as i32;
        let mut next = current;
        let len = channels.len() as i32;
        loop {
            next = (next + delta).clamp(0, len - 1);
            if channels[next as usize].channel_type() != CHANNEL_GUILD_CATEGORY {
                break;
            }
            if next == 0 || next == len - 1 {
                break;
            }
        }
        let next = next as usize;
        if channels[next].channel_type() == CHANNEL_GUILD_CATEGORY {
            return false;
        }
        let next_id = channels[next].id.clone();
        if self.selected_channel_id.as_deref() == Some(next_id.as_str()) {
            return false;
        }

        self.selected_channel_id = Some(next_id);
        self.message_scroll_from_bottom = 0;
        self.selected_message_index = None;
        true
    }

    pub fn move_channel_wrapping(&mut self, delta: i32) -> bool {
        let channels: Vec<ChannelResponse> = self
            .channel_entries()
            .into_iter()
            .filter(|c| c.channel_type() != CHANNEL_GUILD_CATEGORY)
            .collect();
        if channels.is_empty() {
            return false;
        }
        let current = self
            .selected_channel_id
            .as_deref()
            .and_then(|sid| channels.iter().position(|c| c.id == sid));
        let idx = match current {
            Some(i) => (i as i32 + delta).rem_euclid(channels.len() as i32) as usize,
            None => 0,
        };
        let next_id = channels[idx].id.clone();
        if self.selected_channel_id.as_deref() == Some(next_id.as_str()) {
            return false;
        }
        self.selected_channel_id = Some(next_id);
        self.message_scroll_from_bottom = 0;
        self.selected_message_index = None;
        true
    }

    pub fn navigable_channel_pairs(&self) -> Vec<(ServerSelection, String)> {
        let mut out = Vec::new();
        for server in self.server_entries() {
            for ch in self.all_channels_for_server(&server) {
                if ch.channel_type() == CHANNEL_GUILD_CATEGORY {
                    continue;
                }
                if matches!(
                    ch.channel_type(),
                    CHANNEL_GUILD_TEXT
                        | CHANNEL_DM
                        | CHANNEL_GROUP_DM
                        | CHANNEL_DM_PERSONAL_NOTES
                        | CHANNEL_GUILD_LINK
                ) {
                    out.push((server.clone(), ch.id.clone()));
                }
            }
        }
        out
    }

    pub fn next_channel_with_activity(&self) -> Option<(ServerSelection, String)> {
        let flat = self.navigable_channel_pairs();
        if flat.len() < 2 {
            return None;
        }
        let pos = flat
            .iter()
            .position(|(s, id)| {
                s == &self.selected_server
                    && Some(id.as_str()) == self.selected_channel_id.as_deref()
            })
            .unwrap_or(0);
        for step in 1..flat.len() {
            let i = (pos + step) % flat.len();
            let (srv, cid) = &flat[i];
            if self.visible_channel_is_unread(cid) || self.visible_channel_mention_count(cid) > 0 {
                return Some((srv.clone(), cid.clone()));
            }
        }
        None
    }

    pub fn can_edit_message(&self, msg: &MessageResponse) -> bool {
        if !self.active_channel_is_text() || !self.can_send_in_active_channel() {
            return false;
        }
        msg.author.id == self.me.id
    }

    pub fn can_delete_message(&self, msg: &MessageResponse) -> bool {
        if !self.active_channel_is_text() {
            return false;
        }
        let p = self.active_channel_permissions();
        if msg.author.id == self.me.id {
            return p & crate::permissions::VIEW_CHANNEL != 0;
        }
        p & crate::permissions::MANAGE_MESSAGES != 0
    }

    pub fn start_edit_message(&mut self, msg: MessageResponse) {
        self.reply_to = None;
        self.forward_mode = false;
        self.edit_target = Some(EditState {
            channel_id: msg.channel_id.clone(),
            message_id: msg.id.clone(),
        });
        self.set_input(msg.content.clone());
    }

    pub fn active_channel(&self) -> Option<ChannelResponse> {
        let active_id = self.selected_channel_id.as_deref()?;
        self.channel_entries()
            .into_iter()
            .find(|channel| channel.id == active_id)
    }

    pub fn active_channel_id(&self) -> Option<String> {
        self.selected_channel_id.clone()
    }

    pub fn guild_id_for_channel(&self, channel_id: &str) -> Option<String> {
        for (guild_id, channels) in &self.guild_channels {
            if channels.iter().any(|c| c.id == channel_id) {
                return Some(guild_id.clone());
            }
        }
        None
    }

    pub fn guild_id_for_active_channel(&self) -> Option<String> {
        let cid = self.selected_channel_id.as_deref()?;
        self.guild_id_for_channel(cid)
    }

    pub fn active_guild_id(&self) -> Option<String> {
        match &self.selected_server {
            ServerSelection::DirectMessages => None,
            ServerSelection::Guild(id) => Some(id.clone()),
        }
    }

    pub fn active_channel_is_text(&self) -> bool {
        self.active_channel()
            .map(|channel| {
                matches!(
                    channel.channel_type(),
                    CHANNEL_GUILD_TEXT
                        | CHANNEL_DM
                        | CHANNEL_GROUP_DM
                        | CHANNEL_DM_PERSONAL_NOTES
                        | CHANNEL_GUILD_LINK
                )
            })
            .unwrap_or(false)
    }

    pub fn active_channel_is_voice(&self) -> bool {
        self.active_channel()
            .map(|channel| channel.channel_type() == CHANNEL_GUILD_VOICE)
            .unwrap_or(false)
    }

    pub fn active_channel_is_link(&self) -> bool {
        self.active_channel()
            .map(|channel| channel.channel_type() == CHANNEL_GUILD_LINK)
            .unwrap_or(false)
    }

    pub fn active_channel_permissions(&self) -> u64 {
        self.active_channel()
            .map(|ch| self.channel_permissions(&ch))
            .unwrap_or(u64::MAX)
    }

    pub fn channel_permissions(&self, channel: &ChannelResponse) -> u64 {
        let Some(guild_id) = channel.guild_id.as_deref() else {
            return u64::MAX;
        };

        let guild = self.guilds.iter().find(|g| g.id == guild_id);
        let guild_base = guild
            .and_then(|g| g.permissions.as_deref())
            .and_then(|p| p.parse::<u64>().ok())
            .unwrap_or(0);
        let owner_id = guild.map(|g| g.owner_id.as_str()).unwrap_or("");

        let member_roles = self
            .guild_members
            .get(guild_id)
            .and_then(|members| members.iter().find(|m| m.user.id == self.me.id))
            .map(|m| m.roles.clone())
            .unwrap_or_default();

        crate::permissions::compute_channel_permissions(
            &self.me.id,
            &member_roles,
            guild_id,
            owner_id,
            guild_base,
            &channel.permission_overwrites,
        )
    }

    pub fn channel_by_id(&self, channel_id: &str) -> Option<&ChannelResponse> {
        self.private_channels
            .iter()
            .find(|c| c.id == channel_id)
            .or_else(|| {
                self.guild_channels
                    .values()
                    .flat_map(|v| v.iter())
                    .find(|c| c.id == channel_id)
            })
    }

    pub fn patch_channel_last_message_id(&mut self, channel_id: &str, message_id: &str) {
        let bump = |last: &Option<String>| match last {
            None => true,
            Some(prev) => snowflake_sort_key(message_id) > snowflake_sort_key(prev),
        };
        for c in &mut self.private_channels {
            if c.id == channel_id && bump(&c.last_message_id) {
                c.last_message_id = Some(message_id.to_string());
                return;
            }
        }
        for channels in self.guild_channels.values_mut() {
            if let Some(c) = channels.iter_mut().find(|c| c.id == channel_id) {
                if bump(&c.last_message_id) {
                    c.last_message_id = Some(message_id.to_string());
                }
                return;
            }
        }
    }

    fn message_notifies_me(&self, message: &MessageResponse) -> bool {
        let Some(channel) = self.channel_by_id(&message.channel_id) else {
            return message.mentions.iter().any(|user| user.id == self.me.id);
        };
        if self.channel_notification_visibility(channel) == NotificationVisibility::None {
            return false;
        }
        if self.message_mentions_me(message) {
            return true;
        }
        channel.guild_id.is_none() && !self.channel_is_muted_effective(channel)
    }

    /// Whether a message mentions the user: by name, through one of their
    /// roles, or with @everyone/@here, the last two unless the community's
    /// notification settings suppress them.
    pub fn message_mentions_me(&self, message: &MessageResponse) -> bool {
        if message.mentions.iter().any(|u| u.id == self.me.id) {
            return true;
        }
        let guild_id = self
            .channel_by_id(&message.channel_id)
            .and_then(|c| c.guild_id.clone());
        if !message.mention_roles.is_empty()
            && !self.suppress_roles_enabled(guild_id.as_deref())
            && let Some(gid) = guild_id.as_deref()
            && let Some(roles) = self
                .guild_members
                .get(gid)
                .and_then(|mems| mems.iter().find(|m| m.user.id == self.me.id))
                .map(|member| member.roles.as_slice())
            && message
                .mention_roles
                .iter()
                .any(|role_id| roles.contains(role_id))
        {
            return true;
        }
        message.mention_everyone && !self.suppress_everyone_enabled(guild_id.as_deref())
    }

    /// Whether a message is shown highlighted, the way the web app marks
    /// what concerns the reader: it mentions them, or it answers one of
    /// their messages. Never the reader's own messages.
    pub fn message_highlights_me(&self, message: &MessageResponse) -> bool {
        message.author.id != self.me.id
            && (self.message_mentions_me(message)
                || message
                    .referenced_message
                    .as_deref()
                    .is_some_and(|original| original.author.id == self.me.id))
    }

    pub fn on_gateway_message_create(&mut self, message: &MessageResponse) {
        self.patch_channel_last_message_id(&message.channel_id, &message.id);

        let channel_id = message.channel_id.as_str();
        let viewing_here = self.active_channel_id().as_deref() == Some(channel_id);
        let from_self = message.author.id == self.me.id;

        if viewing_here {
            self.read_states.insert(
                message.channel_id.clone(),
                ReadState {
                    last_message_id: Some(message.id.clone()),
                    mention_count: 0,
                },
            );
            return;
        }

        if from_self {
            let mc = self
                .read_states
                .get(channel_id)
                .map(|r| r.mention_count)
                .unwrap_or(0);
            self.read_states.insert(
                message.channel_id.clone(),
                ReadState {
                    last_message_id: Some(message.id.clone()),
                    mention_count: mc,
                },
            );
            return;
        }

        self.read_states
            .entry(message.channel_id.clone())
            .or_insert(ReadState {
                last_message_id: None,
                mention_count: 0,
            });

        if self.message_notifies_me(message)
            && let Some(rs) = self.read_states.get_mut(channel_id)
        {
            rs.mention_count = rs.mention_count.saturating_add(1);
        }
    }

    pub fn can_send_in_active_channel(&self) -> bool {
        let p = self.active_channel_permissions();
        p & crate::permissions::VIEW_CHANNEL != 0 && p & crate::permissions::SEND_MESSAGES != 0
    }

    /// The selected channel's messages, oldest first. Kept sorted on every
    /// write, and shared rather than copied: a draw only borrows them.
    pub fn active_messages(&self) -> std::rc::Rc<Vec<MessageResponse>> {
        self.selected_channel_id
            .as_deref()
            .and_then(|id| self.messages.get(id))
            .cloned()
            .unwrap_or_default()
    }

    pub fn active_oldest_message_id(&self) -> Option<String> {
        let channel_id = self.selected_channel_id.as_deref()?;
        self.messages
            .get(channel_id)
            .and_then(|messages| messages.first())
            .map(|message| message.id.clone())
    }

    /// `G`: the newest message, at the bottom of the pane; in selection
    /// mode it is selected as well.
    pub fn jump_to_latest_message(&mut self) {
        self.message_scroll_from_bottom = 0;
        self.pane_anchor = None;
        if self.selected_message_index.is_some() {
            let count = self.active_messages().len();
            self.selected_message_index = count.checked_sub(1);
        }
    }

    pub fn scroll_messages_up(&mut self, amount: u16) {
        self.message_scroll_from_bottom = self.message_scroll_from_bottom.saturating_add(amount);
    }

    pub fn scroll_messages_down(&mut self, amount: u16) {
        self.message_scroll_from_bottom = self.message_scroll_from_bottom.saturating_sub(amount);
    }

    /// Whether the terminal can draw pictures inline (sixel, kitty, iTerm2);
    /// half-block "pictures" two cells wide are not worth it.
    pub fn custom_emoji_inline_supported(&self) -> bool {
        if self.pixel_mode {
            return true;
        }
        self.image_picker
            .as_ref()
            .map(|p| p.protocol_type() != ratatui_image::picker::ProtocolType::Halfblocks)
            .unwrap_or(false)
    }

    /// The media-proxy URL of a custom emoji, as the web app builds it.
    /// Animated ones are asked for with their animation, as animated WebP.
    pub fn custom_emoji_url(&self, id: &str, animated: bool) -> String {
        let base = self.media_base_url();
        if animated {
            format!("{base}/emojis/{id}.webp?size=128&animated=true")
        } else {
            format!("{base}/emojis/{id}.webp?size=128")
        }
    }

    /// The picture of a custom emoji right now: the frames' serial, the
    /// frame index and the picture.
    pub fn custom_emoji_current(&self, id: &str) -> Option<(u16, usize, &Picture)> {
        match self.custom_emojis.get(id)? {
            CustomEmojiState::Ready(frames) => {
                if frames.is_animated() && !self.ui_settings.performance_mode {
                    self.note_animation(frames);
                }
                let (i, picture) = frames.current(Instant::now(), self.draw_serial.get());
                Some((frames.serial, i, picture))
            }
            _ => None,
        }
    }

    /// Whether the last frame drew an animated custom emoji, so the next
    /// tick should redraw to advance it.
    pub fn custom_emoji_animation_visible(&self) -> bool {
        self.custom_emoji_slots.borrow().iter().any(|id| {
            matches!(self.custom_emojis.get(id), Some(CustomEmojiState::Ready(f)) if f.is_animated())
        })
    }

    /// Called by renderers for `<:name:id>`: a marked placeholder span when
    /// the picture is ready, None to fall back to `:name:` text. Unknown ids
    /// are queued for fetching.
    pub fn custom_emoji_placeholder(&self, id: &str, animated: bool) -> Option<Span<'static>> {
        self.custom_emoji_placeholder_inner(id, animated, true)
    }

    /// The compose box is measured before it is drawn; measuring must not
    /// claim overlay slots.
    fn custom_emoji_placeholder_inner(
        &self,
        id: &str,
        animated: bool,
        register: bool,
    ) -> Option<Span<'static>> {
        if !self.pictures_enabled() || id.is_empty() {
            return None;
        }
        match self.custom_emojis.get(id) {
            Some(CustomEmojiState::Ready(_)) => {
                if !register {
                    return Some(Span::raw(CUSTOM_EMOJI_PLACEHOLDER));
                }
                let mut slots = self.custom_emoji_slots.borrow_mut();
                let k = slots.len();
                slots.push(id.to_string());
                Some(Span::styled(
                    CUSTOM_EMOJI_PLACEHOLDER,
                    custom_emoji_marker_style(k),
                ))
            }
            Some(_) => None,
            None => {
                let mut wanted = self.custom_emoji_wanted.borrow_mut();
                if !wanted.iter().any(|(w, _)| w == id) {
                    wanted.push((id.to_string(), animated));
                }
                None
            }
        }
    }

    /// The compose box as it should be displayed: custom emoji tokens show
    /// their picture (or `:name:` while it loads), everything else verbatim.
    /// One `Line` per raw line. `register` claims overlay slots for the
    /// pictures; pass false when only measuring.
    /// The compose text as styled lines: custom emoji tokens become their
    /// placeholder (or `:name:` when the picture is not there), the
    /// selection is highlighted.
    pub fn input_display(&self, register: bool) -> Vec<Line<'static>> {
        let text = self.input_text();
        self.display_compose_text(&text, register, self.input_selection())
    }

    /// `input_display` for any text; `selection` is a byte range of it.
    pub fn display_compose_text(
        &self,
        text: &str,
        register: bool,
        selection: Option<(usize, usize)>,
    ) -> Vec<Line<'static>> {
        let text_style = Style::default().fg(crate::ui::theme::text());
        let name_style = Style::default().fg(crate::ui::theme::emoji_unknown());
        let sel_style = crate::ui::theme::compose_selection_style();
        let mut lines = Vec::new();
        let mut line_offset = 0usize;
        for raw_line in text.split('\n') {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut rest = raw_line;
            let mut at = line_offset;
            let push_text = |spans: &mut Vec<Span<'static>>, seg: &str, at: usize| {
                if seg.is_empty() {
                    return;
                }
                match selection {
                    Some((s, e)) if s < at + seg.len() && e > at => {
                        let s = s.saturating_sub(at).min(seg.len());
                        let e = (e - at).min(seg.len());
                        if s > 0 {
                            spans.push(Span::styled(seg[..s].to_string(), text_style));
                        }
                        spans.push(Span::styled(seg[s..e].to_string(), sel_style));
                        if e < seg.len() {
                            spans.push(Span::styled(seg[e..].to_string(), text_style));
                        }
                    }
                    _ => spans.push(Span::styled(seg.to_string(), text_style)),
                }
            };
            while !rest.is_empty() {
                let Some(lt) = rest.find('<') else {
                    push_text(&mut spans, rest, at);
                    break;
                };
                if lt > 0 {
                    push_text(&mut spans, &rest[..lt], at);
                    at += lt;
                }
                match parse_custom_emoji_token(&rest[lt..]) {
                    Some(tok) => {
                        let selected =
                            matches!(selection, Some((s, e)) if s <= at && e >= at + tok.len);
                        match self.custom_emoji_placeholder_inner(tok.id, tok.animated, register) {
                            Some(ph) => spans.push(ph),
                            None => spans.push(Span::styled(
                                format!(":{}:", tok.name),
                                if selected { sel_style } else { name_style },
                            )),
                        }
                        rest = &rest[lt + tok.len..];
                        at += tok.len;
                    }
                    None => {
                        push_text(&mut spans, "<", at);
                        rest = &rest[lt + 1..];
                        at += 1;
                    }
                }
            }
            lines.push(Line::from(spans));
            line_offset += raw_line.len() + 1;
        }
        lines
    }

    /// `input_display` flattened to text, for width and cursor arithmetic.
    pub fn input_display_plain(&self) -> String {
        let text = self.input_text();
        self.display_plain_of(&text)
    }

    /// The text before the cursor, flattened the same way, so the cursor
    /// can be placed in the wrapped display.
    pub fn input_head_display_plain(&self) -> String {
        self.display_plain_of(&self.input)
    }

    fn display_plain_of(&self, text: &str) -> String {
        self.display_compose_text(text, false, None)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Backspace: a custom emoji token at the end goes away as one unit.
    pub fn input_pop(&mut self) {
        if self.input.ends_with('>')
            && let Some(lt) = self.input.rfind('<')
            && let Some(tok) = parse_custom_emoji_token(&self.input[lt..])
            && lt + tok.len == self.input.len()
        {
            self.input.truncate(lt);
            return;
        }
        self.input.pop();
    }

    /// Ids the last frame asked for; they are marked Loading here so each is
    /// fetched once.
    pub fn take_custom_emoji_wants(&mut self) -> Vec<(String, String)> {
        let wanted = std::mem::take(&mut *self.custom_emoji_wanted.borrow_mut());
        let mut out = Vec::new();
        for (id, animated) in wanted {
            if self.custom_emojis.contains_key(&id) {
                continue;
            }
            let url = self.custom_emoji_url(&id, animated);
            self.custom_emojis
                .insert(id.clone(), CustomEmojiState::Loading);
            out.push((id, url));
        }
        out
    }

    /// Store the decoded frames of a custom emoji (one for a still image,
    /// an empty list when the fetch or decode failed).
    pub fn set_custom_emoji_frames(&mut self, id: String, frames: Vec<(DynamicImage, Duration)>) {
        self.custom_emoji_version = self.custom_emoji_version.wrapping_add(1);
        let state = match self.image_picker.as_ref() {
            _ if self.pixel_mode && !frames.is_empty() => {
                let mut encoded = Vec::with_capacity(frames.len());
                let mut delays = Vec::with_capacity(frames.len());
                for (img, delay) in frames.into_iter().take(CUSTOM_EMOJI_MAX_FRAMES) {
                    // keep them small: they are drawn one text row tall
                    let img = if img.height() > 96 {
                        img.resize(96 * 2, 96, image::imageops::FilterType::Triangle)
                    } else {
                        img
                    };
                    encoded.push(Picture::Pixels(std::sync::Arc::new(img.to_rgba8())));
                    delays.push(delay.max(Duration::from_millis(20)));
                }
                if encoded.is_empty() {
                    CustomEmojiState::Failed
                } else {
                    CustomEmojiState::Ready(PictureFrames::new(encoded, delays))
                }
            }
            Some(picker) if !frames.is_empty() => {
                let mut encoded = Vec::with_capacity(frames.len());
                let mut delays = Vec::with_capacity(frames.len());
                // exactly the cells' pixel size: anything smaller would be
                // padded with black on sixel
                let (cw, ch) = crate::media::block_px(CUSTOM_EMOJI_CELLS, 1, self.cell_px);
                let ch = if picker.protocol_type() == ratatui_image::picker::ProtocolType::Sixel {
                    crate::media::sixel_rows(ch)
                } else {
                    ch
                };
                for (img, delay) in frames.into_iter().take(CUSTOM_EMOJI_MAX_FRAMES) {
                    let img = img.resize_exact(cw, ch, image::imageops::FilterType::Triangle);
                    if let Ok(p) = picker.new_protocol(
                        img,
                        Rect::new(0, 0, CUSTOM_EMOJI_CELLS, 1),
                        ratatui_image::Resize::Fit(None),
                    ) && let Some(tp) = terminal_picture(&p, None, None, None)
                    {
                        encoded.push(Picture::Terminal(std::sync::Arc::new(tp)));
                        delays.push(delay.max(Duration::from_millis(20)));
                    }
                }
                if encoded.is_empty() {
                    CustomEmojiState::Failed
                } else {
                    CustomEmojiState::Ready(PictureFrames::new(encoded, delays))
                }
            }
            _ => CustomEmojiState::Failed,
        };
        self.custom_emojis.insert(id, state);
    }

    /// The media proxy the web app uses, from discovery.
    pub fn media_base_url(&self) -> String {
        let media = self.discovery.endpoints.media.trim_end_matches('/');
        if media.is_empty() {
            "https://fluxerusercontent.com".to_string()
        } else {
            media.to_string()
        }
    }

    /// The web app's static CDN (default avatars), from discovery.
    pub fn static_cdn_url(&self) -> String {
        self.discovery
            .endpoints
            .static_cdn
            .trim_end_matches('/')
            .to_string()
    }

    /// Previews under messages: wanted, and drawable on this terminal.
    pub fn inline_media_enabled(&self) -> bool {
        self.ui_settings.inline_media && self.pictures_enabled()
    }

    /// Whether any picture is drawn at all: the terminal (or the console
    /// renderer) can draw them, and performance mode is off. Decoding and
    /// encoding pictures is the most expensive thing the client does, so
    /// performance mode does without them entirely.
    pub fn pictures_enabled(&self) -> bool {
        !self.ui_settings.performance_mode && self.custom_emoji_inline_supported()
    }

    /// Profile pictures beside messages: wanted, and drawable here.
    pub fn avatars_enabled(&self) -> bool {
        self.ui_settings.avatars && self.pictures_enabled()
    }

    /// Claim a marker slot for a block of cells this draw.
    pub fn register_media_slot(&self, slot: MediaSlot) -> usize {
        let mut slots = self.media_slots.borrow_mut();
        slots.push(slot);
        slots.len() - 1
    }

    /// The avatar block of a message author: their guild avatar, their own
    /// avatar, the web app's default avatar for their id, or (without a
    /// static CDN) a disc in their colour drawn locally.
    pub fn avatar_slot(
        &self,
        guild_id: Option<&str>,
        user: &UserPartialResponse,
        member_avatar: Option<&str>,
    ) -> MediaSlot {
        self.avatar_slot_sized(
            guild_id,
            user,
            member_avatar,
            crate::media::AVATAR_COLS,
            crate::media::AVATAR_ROWS,
        )
    }

    /// The same avatar as a block of any size (the profile popup shows a
    /// larger one).
    pub fn avatar_slot_sized(
        &self,
        guild_id: Option<&str>,
        user: &UserPartialResponse,
        member_avatar: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> MediaSlot {
        let base = self.media_base_url();
        let url = match (guild_id, member_avatar, user.avatar.as_deref()) {
            (Some(_), Some(hash), _) if !hash.is_empty() => {
                crate::media::avatar_url(&base, guild_id, &user.id, hash)
            }
            (_, _, Some(hash)) if !hash.is_empty() => {
                crate::media::avatar_url(&base, None, &user.id, hash)
            }
            _ => {
                let cdn = self.static_cdn_url();
                if cdn.is_empty() {
                    crate::media::default_avatar_key(crate::media::default_avatar_color(user))
                } else {
                    crate::media::default_avatar_url(&cdn, &user.id)
                }
            }
        };
        MediaSlot::new(url, cols, rows, MediaKind::Avatar)
    }

    /// Blocks on screen with nothing loaded yet, marked loading here so
    /// each is fetched once; a few at a time, the rest on a later draw.
    pub fn take_media_wants(&mut self) -> Vec<MediaSlot> {
        let wanted = std::mem::take(&mut *self.media_wanted.borrow_mut());
        let mut out: Vec<MediaSlot> = Vec::new();
        for slot in wanted {
            if out.iter().any(|s| s.key == slot.key) {
                continue;
            }
            if self.media.start(&slot.key) {
                out.push(slot);
            }
        }
        out
    }

    /// Store what a media download produced (None: it failed).
    pub fn set_media_frames(&mut self, key: String, frames: Option<PictureFrames>, bytes: usize) {
        self.media
            .finish(key, frames, bytes, self.draw_serial.get());
    }

    /// Whether the last draw showed an animated picture, so the next tick
    /// should redraw to advance it.
    pub fn media_animation_visible(&self) -> bool {
        self.media_animation_seen.get()
    }

    /// An animation is being drawn this frame: its pace counts for the ticks.
    pub fn note_animation(&self, frames: &PictureFrames) {
        if let Some(d) = frames.min_delay() {
            let seen = self.animation_delay_seen.get();
            self.animation_delay_seen
                .set(Some(seen.map_or(d, |s| s.min(d))));
        }
    }

    /// How long until the next tick: 100 ms, or the shortest frame delay of
    /// an animation on screen, down to 50 ms, so animations play at their
    /// own pace instead of being sampled ten times a second.
    pub fn tick_period(&self) -> Duration {
        if self.ui_settings.performance_mode {
            // nothing animates: the tick only expires statuses and prunes
            // typing state
            return Duration::from_millis(500);
        }
        let floor = Duration::from_millis(50);
        let mut period = Duration::from_millis(100);
        let mut animating = false;
        if let Some(d) = self.animation_delay_seen.get() {
            period = period.min(d.max(floor));
            animating = true;
        }
        match &self.image_preview {
            Some(ImagePreviewState::ReadyAnimatedGif { delays, .. })
            | Some(ImagePreviewState::ReadyPixels { delays, .. }) => {
                if let Some(d) = delays.iter().copied().min() {
                    period = period.min(d.max(floor));
                    animating = true;
                }
            }
            _ => {}
        }
        let _ = animating;
        period
    }

    /// Short label for the input title: what the compose box is carrying,
    /// the staged files first, then the staged stickers, as in
    /// "2 files: a.png, b.jpg \u{00B7} 1 sticker: catspin".
    pub fn attachment_summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let files = self.pending_attachments.len();
        if files > 0 {
            let names: Vec<String> = self
                .pending_attachments
                .iter()
                .map(|a| format!("{} {}", a.filename, a.size_label()))
                .collect();
            parts.push(format!(
                "{files} {}: {}",
                if files == 1 { "file" } else { "files" },
                names.join(", ")
            ));
        }
        let stickers = self.pending_stickers.len();
        if stickers > 0 {
            let names: Vec<String> = self
                .pending_stickers
                .iter()
                .map(|s| s.name.clone())
                .collect();
            parts.push(format!(
                "{stickers} {}: {}",
                if stickers == 1 { "sticker" } else { "stickers" },
                names.join(", ")
            ));
        }
        parts.join(" \u{00B7} ")
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status_message = message.into();
        self.status_message_until = None;
        self.log_status();
    }

    pub fn set_transient_status(&mut self, message: impl Into<String>, duration: Duration) {
        self.status_message = message.into();
        self.status_message_until = Some(Instant::now() + duration);
        self.log_status();
    }

    /// The status line is where errors show, and they are gone a moment
    /// later: the debug log keeps them, without the names or paths of
    /// files.
    fn log_status(&self) {
        if !self.status_message.is_empty() {
            crate::debug::log("status", crate::debug::scrub_private(&self.status_message));
        }
    }

    pub fn clear_status(&mut self) {
        self.status_message.clear();
        self.status_message_until = None;
    }

    pub fn expire_status_if_needed(&mut self) {
        if self
            .status_message_until
            .is_some_and(|until| Instant::now() >= until)
        {
            self.clear_status();
        }
    }

    pub fn should_auto_load_history_on_scroll_up(&self) -> bool {
        self.message_scroll_max
            .saturating_sub(self.message_scroll_from_bottom.min(self.message_scroll_max))
            <= Self::HISTORY_AUTOLOAD_THRESHOLD_ROWS
    }

    pub fn open_help(&mut self) {
        self.help_scroll = 0;
        self.show_help = true;
    }

    pub fn open_debug(&mut self) {
        self.dismiss_image_preview();
        self.debug_scroll = u16::MAX;
        self.show_debug = true;
    }

    /// Open the profile popup for the selected message's author, over any
    /// other popup. Returns whose profile to fetch; None when there is no
    /// selected message or its author has no profile (Fluxerbot).
    pub fn open_profile_of_selected(&mut self) -> Option<(String, Option<String>)> {
        let msg = self.selected_message()?;
        if msg.author.id == crate::slash_commands::FLUXERBOT_ID {
            self.set_status("Fluxerbot has no profile.");
            return None;
        }
        let guild_id = self.guild_id_for_channel(&msg.channel_id);
        let user = self
            .user_cache
            .get(&msg.author.id)
            .cloned()
            .unwrap_or_else(|| msg.author.clone());
        self.show_settings = false;
        self.show_server_notifications = false;
        self.dismiss_image_preview();
        self.profile = Some(ProfileView {
            user_id: user.id.clone(),
            guild_id: guild_id.clone(),
            user,
            state: ProfileState::Loading,
            scroll: 0,
        });
        Some((msg.author.id.clone(), guild_id))
    }

    /// How many pings are asked for; the server allows up to 100.
    pub const PINGS_LIMIT: u32 = 50;
    /// A channel's pins: the server caps a page at 50.
    pub const PINS_LIMIT: u32 = 50;
    pub const SAVED_LIMIT: u32 = 100;
    pub const REACTION_USERS_LIMIT: u32 = 100;

    /// Open the pings overlay, empty until the list arrives.
    pub fn open_pings(&mut self) {
        self.show_settings = false;
        self.show_server_notifications = false;
        self.show_help = false;
        self.dismiss_image_preview();
        self.profile = None;
        self.channel_picker = None;
        self.pings = Some(PingsView {
            state: PingsState::Loading,
            selected: 0,
        });
    }

    pub fn dismiss_pings(&mut self) {
        self.pings = None;
    }

    pub fn set_pings_loaded(&mut self, messages: Vec<MessageResponse>) {
        for message in &messages {
            self.merge_message_embedded_members(message);
            merge_user_cache(&mut self.user_cache, [message.author.clone()]);
        }
        if let Some(view) = &mut self.pings {
            view.state = PingsState::Ready(messages);
            view.selected = 0;
        }
    }

    pub fn set_pings_failed(&mut self, message: String) {
        if let Some(view) = &mut self.pings {
            view.state = PingsState::Failed(message);
        }
    }

    pub fn pings_messages(&self) -> &[MessageResponse] {
        match &self.pings {
            Some(PingsView {
                state: PingsState::Ready(messages),
                ..
            }) => messages,
            _ => &[],
        }
    }

    pub fn pings_selected(&self) -> Option<&MessageResponse> {
        let view = self.pings.as_ref()?;
        self.pings_messages().get(view.selected)
    }

    pub fn pings_move(&mut self, delta: isize) {
        let count = self.pings_messages().len();
        if let Some(view) = &mut self.pings {
            view.selected = if count == 0 {
                0
            } else {
                (view.selected as isize + delta).clamp(0, count as isize - 1) as usize
            };
        }
    }

    /// Where a ping came from: the community, if any, and the channel.
    pub fn ping_location(&self, message: &MessageResponse) -> (Option<String>, String) {
        self.channel_location(&message.channel_id)
    }

    /// Where a channel is: its community's name, where it has one, and
    /// the channel's own. A channel the client does not know is named by
    /// the tail of its id rather than left blank.
    pub fn channel_location(&self, channel_id: &str) -> (Option<String>, String) {
        let channel = self.channel_by_id(channel_id);
        let guild = channel
            .and_then(|c| c.guild_id.clone())
            .or_else(|| self.guild_id_for_channel(channel_id))
            .and_then(|gid| self.guilds.iter().find(|g| g.id == gid))
            .map(|g| g.name.clone());
        let name = match channel {
            Some(c) => crate::ui::sidebar::channel_name(self, c),
            None => format!(
                "unknown-{}",
                &channel_id[channel_id.len().saturating_sub(4)..]
            ),
        };
        (guild, name)
    }

    fn server_for_channel(&self, channel_id: &str) -> Option<ServerSelection> {
        if self.private_channels.iter().any(|c| c.id == channel_id) {
            return Some(ServerSelection::DirectMessages);
        }
        self.guild_id_for_channel(channel_id)
            .map(ServerSelection::Guild)
    }

    /// Go to the selected ping: its community and channel now, the
    /// message itself once the channel's history is there.
    pub fn pings_jump(&mut self) -> bool {
        let Some(message) = self.pings_selected().cloned() else {
            return false;
        };
        let Some(server) = self.server_for_channel(&message.channel_id) else {
            self.set_status("That channel is not on your list any more.");
            return false;
        };
        self.pings = None;
        self.selected_server = server;
        self.selected_channel_id = Some(message.channel_id.clone());
        self.message_scroll_from_bottom = 0;
        self.selected_message_index = None;
        self.normalize_selection();
        self.focus = Focus::Messages;
        self.pending_jump = Some((message.channel_id.clone(), message.id.clone()));
        self.pending_jump_pages = 0;
        self.apply_pending_jump(&message.channel_id);
        true
    }

    /// Older pages a jump fetches before giving up (50 messages each).
    pub const JUMP_MAX_PAGES: u32 = 40;

    /// Whether a jump is waiting for older messages of the active
    /// channel; a jump whose channel the user has left is dropped.
    pub fn jump_wants_older(&mut self) -> bool {
        let Some((channel, _)) = self.pending_jump.clone() else {
            return false;
        };
        if self.active_channel_id().as_deref() != Some(channel.as_str()) {
            self.pending_jump = None;
            return false;
        }
        self.messages_loaded.contains(&channel) && !self.messages_older_exhausted.contains(&channel)
    }

    /// An older page arrived, or failed to, for a channel.
    pub fn older_page_for_jump(&mut self, channel_id: &str, ok: bool) {
        if self
            .pending_jump
            .as_ref()
            .is_none_or(|(c, _)| c != channel_id)
        {
            return;
        }
        if !ok {
            self.pending_jump = None;
            return;
        }
        self.pending_jump_pages += 1;
        self.apply_pending_jump(channel_id);
    }

    /// Select the message a jump was for, if its channel's history is
    /// loaded (called again when it arrives).
    pub fn apply_pending_jump(&mut self, channel_id: &str) {
        let Some((channel, message_id)) = self.pending_jump.clone() else {
            return;
        };
        if channel != channel_id || !self.messages_loaded.contains(channel_id) {
            return;
        }
        let index = self
            .messages
            .get(channel_id)
            .and_then(|messages| messages.iter().position(|m| m.id == message_id));
        if let Some(index) = index {
            self.pending_jump = None;
            self.selected_message_index = Some(index);
            self.clamp_scroll_to_selected_message();
            // the pane would otherwise put the view back where it was,
            // since the history just grew under it
            self.pane_anchor = None;
            if self
                .status_message
                .starts_with("Loading older messages to reach")
            {
                self.set_transient_status("Here is the ping.", Self::TRANSIENT_STATUS_DURATION);
            }
            return;
        }
        // not in the loaded history: older pages are fetched (see
        // `jump_wants_older`) until it turns up or the channel's beginning does
        if self.messages_older_exhausted.contains(channel_id)
            || self.pending_jump_pages >= Self::JUMP_MAX_PAGES
        {
            self.pending_jump = None;
            self.set_status("That message is not in the channel's history any more.");
        } else {
            self.set_status("Loading older messages to reach the ping…");
        }
    }

    /// Take the selected ping off the list; its id, for the server.
    pub fn pings_dismiss_selected(&mut self) -> Option<String> {
        let view = self.pings.as_mut()?;
        let PingsState::Ready(messages) = &mut view.state else {
            return None;
        };
        if view.selected >= messages.len() {
            return None;
        }
        let removed = messages.remove(view.selected);
        if view.selected >= messages.len() {
            view.selected = messages.len().saturating_sub(1);
        }
        Some(removed.id)
    }

    /// Empty the list; the ids, for the server.
    pub fn pings_take_all(&mut self) -> Vec<String> {
        let Some(view) = self.pings.as_mut() else {
            return Vec::new();
        };
        let PingsState::Ready(messages) = &mut view.state else {
            return Vec::new();
        };
        view.selected = 0;
        std::mem::take(messages).into_iter().map(|m| m.id).collect()
    }

    pub fn dismiss_profile(&mut self) {
        self.profile = None;
    }

    /// Open the file picker where it last was, else in the home directory.
    pub fn open_file_picker(&mut self) {
        let dir = self
            .attach_dir
            .clone()
            .filter(|d| d.is_dir())
            .or_else(dirs::home_dir)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("/"));
        self.dismiss_channel_picker();
        self.dismiss_image_preview();
        self.file_picker = Some(FilePicker {
            dir: dir.clone(),
            entries: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            query: String::new(),
        });
        self.file_picker_load(dir);
    }

    pub fn dismiss_file_picker(&mut self) {
        self.file_picker = None;
    }

    /// Read `dir` into the picker: directories first, then files, by name.
    /// Dotfiles are listed only while the filter starts with a dot.
    fn file_picker_load(&mut self, dir: std::path::PathBuf) {
        if self.file_picker.is_none() {
            return;
        }
        let mut entries: Vec<FileEntry> = match std::fs::read_dir(&dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let meta = e.metadata().ok()?;
                    let is_dir =
                        meta.is_dir() || (meta.file_type().is_symlink() && e.path().is_dir());
                    Some(FileEntry {
                        name: e.file_name().to_string_lossy().to_string(),
                        path: e.path(),
                        is_dir,
                        size: meta.len(),
                    })
                })
                .collect(),
            Err(err) => {
                self.set_status(format!("Cannot read {}: {err}", dir.display()));
                return;
            }
        };
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        if let Some(picker) = self.file_picker.as_mut() {
            picker.dir = dir.clone();
            picker.entries = entries;
            picker.query.clear();
            picker.selected = 0;
        }
        self.attach_dir = Some(dir);
        self.filter_file_picker();
    }

    pub fn filter_file_picker(&mut self) {
        let Some(picker) = self.file_picker.as_mut() else {
            return;
        };
        let q = picker.query.to_lowercase();
        let show_hidden = q.starts_with('.');
        picker.filtered = picker
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| show_hidden || !e.name.starts_with('.'))
            .filter(|(_, e)| q.is_empty() || e.name.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        picker.selected = picker.selected.min(picker.filtered.len().saturating_sub(1));
    }

    pub fn file_picker_move(&mut self, delta: i32) {
        if let Some(picker) = self.file_picker.as_mut() {
            let n = picker.filtered.len();
            if n == 0 {
                picker.selected = 0;
                return;
            }
            let at = picker.selected as i64 + delta as i64;
            picker.selected = at.clamp(0, n as i64 - 1) as usize;
        }
    }

    /// Up one directory.
    pub fn file_picker_parent(&mut self) {
        let Some(parent) = self
            .file_picker
            .as_ref()
            .and_then(|p| p.dir.parent().map(|d| d.to_path_buf()))
        else {
            return;
        };
        let from = self.file_picker.as_ref().map(|p| p.dir.clone());
        self.file_picker_load(parent);
        // land on the directory just left
        if let (Some(from), Some(picker)) = (from, self.file_picker.as_mut())
            && let Some(at) = picker
                .filtered
                .iter()
                .position(|&i| picker.entries[i].path == from)
        {
            picker.selected = at;
        }
    }

    /// Enter: descend into a directory (None), or the file to attach.
    pub fn file_picker_confirm(&mut self) -> Option<std::path::PathBuf> {
        let entry = self.file_picker.as_ref()?.current()?.clone();
        if entry.is_dir {
            self.file_picker_load(entry.path);
            None
        } else {
            self.file_picker = None;
            Some(entry.path)
        }
    }

    /// Open the sticker picker, filtered by `query`: every sticker the
    /// client knows of, the active community's first.
    pub fn open_sticker_picker(&mut self, query: &str) {
        let active = self
            .guild_id_for_active_channel()
            .or_else(|| self.active_guild_id());
        let mut guilds: Vec<&crate::api::types::GuildResponse> = self.guilds.iter().collect();
        guilds.sort_by_key(|g| {
            (
                active.as_deref() != Some(g.id.as_str()),
                g.name.to_lowercase(),
            )
        });
        let mut entries: Vec<StickerEntry> = Vec::new();
        for guild in guilds {
            let Some(stickers) = self.guild_stickers.get(&guild.id) else {
                continue;
            };
            let mut owned: Vec<&crate::api::types::GuildStickerResponse> =
                stickers.iter().collect();
            owned.sort_by_key(|s| s.name.to_lowercase());
            for sticker in owned {
                entries.push(StickerEntry {
                    guild_id: guild.id.clone(),
                    guild_name: guild.name.clone(),
                    sticker: sticker.clone(),
                });
            }
        }
        self.dismiss_channel_picker();
        self.dismiss_image_preview();
        self.file_picker = None;
        self.sticker_picker = Some(StickerPicker {
            entries,
            filtered: Vec::new(),
            selected: 0,
            query: query.to_string(),
            searching: false,
            query_before_search: String::new(),
        });
        self.filter_sticker_picker();
    }

    pub fn dismiss_sticker_picker(&mut self) {
        self.sticker_picker = None;
    }

    /// The rows the query keeps: a sticker matches on its name and on any
    /// of its tags.
    pub fn filter_sticker_picker(&mut self) {
        let Some(picker) = self.sticker_picker.as_mut() else {
            return;
        };
        let q = picker.query.trim().to_lowercase();
        picker.filtered = picker
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                q.is_empty()
                    || e.sticker.name.to_lowercase().contains(&q)
                    || e.sticker.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect();
        picker.selected = picker.selected.min(picker.filtered.len().saturating_sub(1));
    }

    pub fn sticker_picker_move(&mut self, delta: i32) {
        if let Some(picker) = self.sticker_picker.as_mut() {
            let n = picker.filtered.len();
            if n == 0 {
                picker.selected = 0;
                return;
            }
            let at = picker.selected as i64 + delta as i64;
            picker.selected = at.clamp(0, n as i64 - 1) as usize;
        }
    }

    /// `/`: start a search. The filter empties, so the whole list is
    /// there to search, and Esc puts back what it was.
    pub fn sticker_picker_start_search(&mut self) {
        if let Some(picker) = self.sticker_picker.as_mut() {
            picker.query_before_search = std::mem::take(&mut picker.query);
            picker.searching = true;
            picker.selected = 0;
        }
        self.filter_sticker_picker();
    }

    /// Leave the search: `keep` for Enter, which keeps the filter and the
    /// sticker under the cursor, false for Esc, which puts the filter the
    /// search started from back.
    pub fn sticker_picker_end_search(&mut self, keep: bool) {
        let Some(picker) = self.sticker_picker.as_mut() else {
            return;
        };
        picker.searching = false;
        if keep {
            picker.query_before_search.clear();
            return;
        }
        picker.query = std::mem::take(&mut picker.query_before_search);
        self.filter_sticker_picker();
    }

    /// A character typed into the search: the list narrows and the cursor
    /// goes to the first sticker that matches.
    pub fn sticker_picker_search_type(&mut self, ch: char) {
        if let Some(picker) = self.sticker_picker.as_mut() {
            picker.query.push(ch);
            picker.selected = 0;
        }
        self.filter_sticker_picker();
    }

    /// Backspace in the search, or Ctrl+Backspace and Ctrl+U for all of it.
    pub fn sticker_picker_search_erase(&mut self, all: bool) {
        if let Some(picker) = self.sticker_picker.as_mut() {
            if all {
                picker.query.clear();
            } else {
                picker.query.pop();
            }
            picker.selected = 0;
        }
        self.filter_sticker_picker();
    }

    /// Enter in the picker: stage the sticker under the cursor and close.
    /// The string is what to tell the user.
    pub fn sticker_picker_confirm(&mut self) -> Option<String> {
        let entry = self.sticker_picker.as_ref()?.current()?.clone();
        self.sticker_picker = None;
        Some(self.stage_sticker(StagedSticker {
            id: entry.sticker.id.clone(),
            name: entry.sticker.name.clone(),
            animated: entry.sticker.animated,
        }))
    }

    /// Put a sticker on the next message; a message carries at most three.
    pub fn stage_sticker(&mut self, sticker: StagedSticker) -> String {
        if self.pending_stickers.iter().any(|s| s.id == sticker.id) {
            return format!("{} is already on this message.", sticker.name);
        }
        if self.pending_stickers.len() >= MAX_STICKERS_PER_MESSAGE {
            return format!(
                "A message carries at most {MAX_STICKERS_PER_MESSAGE} stickers (Ctrl+X drops the last one)."
            );
        }
        let name = sticker.name.clone();
        self.pending_stickers.push(sticker);
        format!("Staged {name}: Enter sends it.")
    }

    /// The media-proxy URL of a sticker. `edge_px` is snapped up to a rung
    /// of the proxy's ladder and clamped to the sticker class; animated
    /// ones are asked for with their animation.
    pub fn sticker_url(&self, id: &str, animated: bool, edge_px: u32) -> String {
        let base = self.media_base_url();
        let size = edge_px.clamp(STICKER_MIN_PX, STICKER_MAX_PX);
        if animated {
            format!("{base}/stickers/{id}.webp?size={size}&animated=true")
        } else {
            format!("{base}/stickers/{id}.webp?size={size}")
        }
    }

    /// The block a sticker takes within `max` cells, asked for at the size
    /// that block holds. None when pictures are not drawn here.
    pub fn sticker_slot(&self, id: &str, animated: bool, max: (u16, u16)) -> Option<MediaSlot> {
        if !self.pictures_enabled() || id.is_empty() || max.0 == 0 || max.1 == 0 {
            return None;
        }
        // A sticker is delivered square, so the block is a square in cells.
        let (cols, rows) = crate::media::picture_cells(
            (STICKER_MAX_PX, STICKER_MAX_PX),
            self.cell_px,
            (max.0, max.1),
        );
        let px = crate::media::block_px(cols, rows, self.cell_px);
        Some(MediaSlot::new(
            self.sticker_url(id, animated, px.0.max(px.1)),
            cols,
            rows,
            MediaKind::Picture,
        ))
    }

    /// The thumbnail of a staged sticker in the compose box.
    pub fn staged_sticker_slot(&self, sticker: &StagedSticker) -> Option<MediaSlot> {
        self.sticker_slot(&sticker.id, sticker.animated, (THUMB_COLS, THUMB_ROWS))
    }

    /// The block a staged picture's thumbnail takes in the compose box,
    /// when the terminal can draw one: an image at its own shape, a
    /// video's first frame letterboxed into a 16:9 box.
    pub fn staged_thumbnail_slot(&self, a: &crate::media::StagedAttachment) -> Option<MediaSlot> {
        if !self.custom_emoji_inline_supported() {
            return None;
        }
        let shape = if let Some(dims) = a.dimensions {
            dims
        } else if a.is_video() {
            // a 16:9 frame, larger than any box so it is only scaled down
            (1600, 900)
        } else {
            return None;
        };
        let (cols, rows) =
            crate::media::picture_cells(shape, self.cell_px, (THUMB_COLS, THUMB_ROWS));
        Some(MediaSlot::new(
            crate::media::staged_url(a.id),
            cols,
            rows,
            MediaKind::Picture,
        ))
    }

    /// The block a file's preview takes in the file picker, within `max`.
    pub fn file_preview_slot(&self, path: &std::path::Path, max: (u16, u16)) -> Option<MediaSlot> {
        if !self.custom_emoji_inline_supported() || max.0 == 0 || max.1 == 0 {
            return None;
        }
        let name = path.file_name()?.to_string_lossy().to_string();
        let shape = if crate::media::is_video("", &name) {
            (1600, 900)
        } else if crate::media::is_image("", &name) {
            crate::media::image_dimensions_of(path)?
        } else {
            return None;
        };
        let (cols, rows) = crate::media::picture_cells(shape, self.cell_px, max);
        Some(MediaSlot::new(
            crate::media::file_url(path),
            cols,
            rows,
            MediaKind::Picture,
        ))
    }

    /// Where a local media slot's bytes come from, for the fetch.
    pub fn local_media_source(&self, url: &str) -> Option<crate::media::LocalSource> {
        if let Some(id) = crate::media::parse_staged_url(url) {
            let a = self.pending_attachments.iter().find(|a| a.id == id)?;
            return Some(crate::media::LocalSource::Bytes {
                filename: a.filename.clone(),
                bytes: a.bytes.clone(),
            });
        }
        crate::media::parse_file_url(url).map(crate::media::LocalSource::Path)
    }

    /// Whether Ctrl+O on this attachment should stop what plays rather
    /// than start it again.
    pub fn audio_playing(&self, key: &str) -> bool {
        self.audio.as_ref().is_some_and(|p| p.key == key)
    }

    pub fn stop_audio(&mut self) {
        if let Some(mut player) = self.audio.take() {
            player.stop();
            self.set_status(format!("Stopped {}", player.label));
        }
    }

    /// Hand downloaded audio to the player; whatever played before stops.
    pub fn play_audio(&mut self, key: String, label: String, bytes: Vec<u8>) {
        if let Some(mut old) = self.audio.take() {
            old.stop();
        }
        let Some(argv) = crate::media::player_command(&self.audio_player_cmd) else {
            self.set_status(
                "No audio player found: install mpv (or ffplay, pw-play, paplay, aplay), \
                 or set [media] audio_player in the config.",
            );
            return;
        };
        match crate::media::Player::start(&argv, bytes, label, key) {
            Ok(player) => {
                self.set_status(format!(
                    "\u{266A} {} via {} (Ctrl+O on it again stops)",
                    player.label, player.program
                ));
                self.audio = Some(player);
            }
            Err(err) => self.set_status(format!("Couldn't start the audio player: {err}")),
        }
    }

    /// Notice a player that ended on its own.
    pub fn reap_audio(&mut self) {
        if self.audio.as_mut().is_some_and(|p| p.finished()) {
            self.audio = None;
        }
    }

    /// The member data for the profile popup's guild: from the profile
    /// once loaded, from the roster before that.
    pub fn profile_member(&self) -> Option<GuildMemberResponse> {
        let view = self.profile.as_ref()?;
        if let ProfileState::Ready(profile) = &view.state
            && let Some(m) = profile.guild_member.as_ref()
        {
            return Some(m.clone());
        }
        let gid = view.guild_id.as_deref()?;
        self.guild_members
            .get(gid)?
            .iter()
            .find(|m| m.user.id == view.user_id)
            .cloned()
    }

    /// The picture the profile popup shows, at full size, with a title for
    /// the preview: the guild avatar, the user's own, or the web app's
    /// default one. None when there is nothing to fetch.
    pub fn profile_picture(&self) -> Option<(String, String)> {
        let view = self.profile.as_ref()?;
        let user = &view.user;
        let member = self.profile_member();
        let base = self.media_base_url();
        let gid = view.guild_id.as_deref();
        let member_avatar = member
            .as_ref()
            .and_then(|m| m.avatar.as_deref())
            .filter(|h| !h.is_empty());
        let url = match (gid, member_avatar, user.avatar.as_deref()) {
            (Some(_), Some(hash), _) => crate::media::avatar_url_sized(
                &base,
                gid,
                &user.id,
                hash,
                crate::media::AVATAR_PREVIEW_PX,
            ),
            (_, _, Some(hash)) if !hash.is_empty() => crate::media::avatar_url_sized(
                &base,
                None,
                &user.id,
                hash,
                crate::media::AVATAR_PREVIEW_PX,
            ),
            _ => {
                let cdn = self.static_cdn_url();
                if cdn.is_empty() {
                    return None;
                }
                crate::media::default_avatar_url(&cdn, &user.id)
            }
        };
        let name = member
            .as_ref()
            .and_then(|m| m.nick.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| display_name(user));
        Some((url, format!("{name} · profile picture")))
    }

    pub fn profile_scroll(&mut self, delta: i32) {
        if let Some(view) = self.profile.as_mut() {
            view.scroll = view.scroll.saturating_add_signed(delta as i16);
        }
    }

    /// A profile arrived: shown if the popup still asks for it; the user
    /// and member data are kept either way.
    pub fn set_profile_loaded(
        &mut self,
        user_id: &str,
        guild_id: Option<&str>,
        profile: crate::api::types::UserProfileResponse,
    ) {
        merge_user_cache(&mut self.user_cache, [profile.user.clone()]);
        if let (Some(gid), Some(member)) = (guild_id, profile.guild_member.clone())
            && self.guild_members.contains_key(gid)
        {
            self.merge_guild_member(gid, member);
        }
        if let Some(view) = self.profile.as_mut()
            && view.user_id == user_id
            && view.guild_id.as_deref() == guild_id
        {
            view.user = profile.user.clone();
            view.state = ProfileState::Ready(Box::new(profile));
        }
    }

    pub fn set_profile_failed(&mut self, user_id: &str, guild_id: Option<&str>, message: String) {
        if let Some(view) = self.profile.as_mut()
            && view.user_id == user_id
            && view.guild_id.as_deref() == guild_id
        {
            view.state = ProfileState::Failed(message);
        }
    }

    pub fn dismiss_image_preview(&mut self) {
        self.image_preview = None;
    }

    pub fn image_preview_scroll(&mut self, delta: i32) {
        let Some(ref mut prev) = self.image_preview else {
            return;
        };
        if let ImagePreviewState::ReadyChafa { scroll, lines, .. } = prev {
            let max = lines.len().saturating_sub(1);
            let ns = (*scroll as i32 + delta).clamp(0, max as i32) as usize;
            *scroll = ns;
        }
    }

    pub fn advance_image_preview_animation(&mut self, dt: Duration) {
        let Some(ref mut prev) = self.image_preview else {
            return;
        };
        if let ImagePreviewState::ReadyPixels {
            frames,
            delays,
            frame_idx,
            elapsed,
            ..
        } = prev
        {
            if frames.len() > 1 && !delays.is_empty() {
                *elapsed += dt;
                while *elapsed >= delays[*frame_idx % delays.len()] {
                    *elapsed -= delays[*frame_idx % delays.len()];
                    *frame_idx = (*frame_idx + 1) % frames.len();
                }
            }
            return;
        }
        let ImagePreviewState::ReadyAnimatedGif {
            frames,
            delays,
            frame_idx,
            elapsed,
            current_protocol,
            ..
        } = prev
        else {
            return;
        };
        if frames.is_empty() {
            return;
        }
        *elapsed += dt;
        let old_idx = *frame_idx;
        loop {
            let lim = delays
                .get(*frame_idx)
                .copied()
                .unwrap_or(Duration::from_millis(100));
            if *elapsed < lim {
                break;
            }
            *elapsed -= lim;
            *frame_idx = (*frame_idx + 1) % frames.len();
        }
        if *frame_idx != old_idx
            && let Some(ref picker) = self.image_picker
        {
            *current_protocol = picker.new_resize_protocol(frames[*frame_idx].clone());
        }
    }

    pub fn start_image_preview_loading(&mut self, title: String) {
        self.image_preview = Some(ImagePreviewState::Loading { title });
    }

    pub const TYPING_TTL: Duration = Duration::from_secs(10);

    pub fn record_typing(&mut self, channel_id: &str, user_id: &str) {
        if channel_id.is_empty() || user_id.is_empty() || user_id == self.me.id {
            return;
        }
        let exp = Instant::now() + Self::TYPING_TTL;
        self.typing_users
            .entry(channel_id.to_string())
            .or_default()
            .insert(user_id.to_string(), exp);
    }

    pub fn clear_typing_for_message(&mut self, channel_id: &str, user_id: &str) {
        if let Some(map) = self.typing_users.get_mut(channel_id) {
            map.remove(user_id);
            if map.is_empty() {
                self.typing_users.remove(channel_id);
            }
        }
    }

    pub fn prune_stale_typing(&mut self) {
        let now = Instant::now();
        self.typing_users.retain(|_, users| {
            users.retain(|_, exp| *exp > now);
            !users.is_empty()
        });
    }

    pub fn clear_all_typing(&mut self) {
        self.typing_users.clear();
    }

    /// After every key: keep the own-typing bout in step with the compose
    /// text. A message being edited and a slash command are not typing.
    pub fn note_own_typing(&mut self) {
        let text = format!("{}{}", self.input, self.input_tail);
        let counts = self.ui_settings.send_typing && self.edit_target.is_none();
        let channel = self.active_channel_id();
        self.own_typing
            .note(&text, channel.as_deref(), counts, Instant::now());
    }

    /// The channel to tell "typing" now, if one is due; the send is
    /// counted as done.
    pub fn own_typing_due(&mut self) -> Option<String> {
        if !self.ui_settings.send_typing {
            self.own_typing.end_bout();
            return None;
        }
        let now = Instant::now();
        let channel = self.own_typing.due(now)?;
        self.own_typing.sent(&channel, now);
        Some(channel)
    }

    pub fn typing_peer_names(&self, channel_id: &str) -> Vec<String> {
        let now = Instant::now();
        let Some(users) = self.typing_users.get(channel_id) else {
            return Vec::new();
        };
        let mut ids: Vec<&String> = users
            .iter()
            .filter(|(_, exp)| **exp > now)
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        let guild = self.guild_id_for_channel(channel_id);
        ids.into_iter()
            .map(|id| {
                self.user_cache
                    .get(id.as_str())
                    .map(|u| self.shown_name_for_user(guild.as_deref(), u))
                    .unwrap_or_else(|| id.clone())
            })
            .collect()
    }

    pub fn others_typing_phrase(&self) -> Option<String> {
        if !self.ui_settings.show_typing_indicators || self.ui_settings.performance_mode {
            return None;
        }
        let ch = self.active_channel_id()?;
        let names = self.typing_peer_names(&ch);
        if names.is_empty() {
            return None;
        }
        Some(fluxer_typing_phrase(&names))
    }

    pub fn others_typing_anim_active(&self) -> bool {
        if !self.ui_settings.show_typing_indicators || self.ui_settings.performance_mode {
            return false;
        }
        self.active_channel_id()
            .is_some_and(|c| !self.typing_peer_names(&c).is_empty())
    }

    pub fn normalize_selection(&mut self) {
        let available_servers = self.server_entries();
        if !available_servers.contains(&self.selected_server) {
            self.selected_server = available_servers
                .first()
                .cloned()
                .unwrap_or(ServerSelection::DirectMessages);
        }

        let channels = self.channel_entries();
        if channels.is_empty() {
            self.selected_channel_id = None;
            return;
        }

        let selected_exists = self
            .selected_channel_id
            .as_deref()
            .map(|selected| channels.iter().any(|channel| channel.id == selected))
            .unwrap_or(false);

        if !selected_exists {
            self.selected_channel_id = channels
                .iter()
                .find(|c| c.channel_type() != CHANNEL_GUILD_CATEGORY)
                .map(|channel| channel.id.clone());
            self.message_scroll_from_bottom = 0;
        }
    }

    pub fn upsert_guild(&mut self, guild: GuildResponse) {
        if guild.id.is_empty() {
            return;
        }
        if let Some(existing) = self
            .guilds
            .iter_mut()
            .find(|existing| existing.id == guild.id)
        {
            let preserved_perms = existing.permissions.clone();
            let preserved_name = existing.name.clone();
            let preserved_owner = existing.owner_id.clone();
            *existing = guild;
            if existing.permissions.is_none() {
                existing.permissions = preserved_perms;
            }
            if existing.name.is_empty() {
                existing.name = preserved_name;
            }
            if existing.owner_id.is_empty() {
                existing.owner_id = preserved_owner;
            }
        } else {
            self.guilds.push(guild);
        }
        self.normalize_selection();
    }

    pub fn remove_guild(&mut self, guild_id: &str) {
        self.roster_version = self.roster_version.wrapping_add(1);
        self.guilds.retain(|guild| guild.id != guild_id);
        self.guild_channels.remove(guild_id);
        self.guild_members.remove(guild_id);
        self.guild_members_synced.remove(guild_id);
        self.api_backoff_clear_guild(guild_id);
        self.guild_emojis.remove(guild_id);
        self.guild_stickers.remove(guild_id);
        self.loading_stickers.remove(guild_id);
        self.guild_roles.remove(guild_id);
        self.guild_roles_forbidden.remove(guild_id);
        self.voice_states.remove(guild_id);
        self.normalize_selection();
    }

    pub fn set_private_channels(&mut self, channels: Vec<ChannelResponse>) {
        self.roster_version = self.roster_version.wrapping_add(1);
        merge_user_cache(
            &mut self.user_cache,
            channels
                .iter()
                .flat_map(|channel| channel.recipients.clone()),
        );
        self.private_channels = channels;
        self.normalize_selection();
    }

    pub fn upsert_private_channel(&mut self, channel: ChannelResponse) {
        if let Some(existing) = self
            .private_channels
            .iter_mut()
            .find(|existing| existing.id == channel.id)
        {
            *existing = channel;
        } else {
            self.private_channels.push(channel);
        }
        self.normalize_selection();
    }

    pub fn remove_private_channel(&mut self, channel_id: &str) {
        self.private_channels
            .retain(|channel| channel.id != channel_id);
        self.pinned_dms.remove(channel_id);
        if self.selected_channel_id.as_deref() == Some(channel_id) {
            self.selected_channel_id = None;
        }
        self.normalize_selection();
    }

    pub fn set_guild_channels(&mut self, guild_id: &str, channels: Vec<ChannelResponse>) {
        self.roster_version = self.roster_version.wrapping_add(1);
        merge_user_cache(
            &mut self.user_cache,
            channels
                .iter()
                .flat_map(|channel| channel.recipients.clone()),
        );
        self.guild_channels.insert(guild_id.to_string(), channels);
        self.loading_channels.remove(guild_id);
        self.api_backoff_clear(&format!("channels:{guild_id}"));
        self.normalize_selection();
    }

    pub fn upsert_channel(&mut self, channel: ChannelResponse) {
        if let Some(guild_id) = channel.guild_id.clone() {
            let entries = self.guild_channels.entry(guild_id).or_default();
            if let Some(existing) = entries
                .iter_mut()
                .find(|existing| existing.id == channel.id)
            {
                *existing = channel;
            } else {
                entries.push(channel);
            }
        } else {
            self.upsert_private_channel(channel);
            return;
        }
        self.normalize_selection();
    }

    pub fn remove_channel(&mut self, channel: &ChannelResponse) {
        if let Some(guild_id) = channel.guild_id.as_deref() {
            if let Some(entries) = self.guild_channels.get_mut(guild_id) {
                entries.retain(|entry| entry.id != channel.id);
            }
        } else {
            self.remove_private_channel(&channel.id);
            return;
        }
        self.normalize_selection();
    }

    pub fn set_guild_members(&mut self, guild_id: &str, members: Vec<GuildMemberResponse>) {
        self.roster_version = self.roster_version.wrapping_add(1);
        merge_user_cache(
            &mut self.user_cache,
            members.iter().map(|member| member.user.clone()),
        );
        self.guild_members.insert(guild_id.to_string(), members);
    }

    /// What the status line says when a community's member list could not
    /// be fetched: plain words, and that @mentions still work from the
    /// members seen so far.
    pub fn members_failure_status(
        &self,
        guild_id: &str,
        failure: crate::api::client::MembersFailure,
        got_some: bool,
        detail: &str,
    ) -> String {
        use crate::api::client::MembersFailure;
        let name = self
            .guilds
            .iter()
            .find(|g| g.id == guild_id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| "this community".to_string());
        let seen_so_far = "@mentions offer the members seen so far";
        let retry_min = Self::API_FAILURE_BACKOFF_SECS.div_ceil(60);
        match failure {
            MembersFailure::Unavailable if got_some => format!(
                "Only part of the member list of {name} arrived before the server timed out; \
                 the rest is tried again in {retry_min} min."
            ),
            MembersFailure::Unavailable => format!(
                "The member list of {name} is unavailable (the server timed out); \
                 {seen_so_far}, and it is tried again in {retry_min} min."
            ),
            MembersFailure::Forbidden => {
                format!("{name} does not let you list its members; {seen_so_far}.")
            }
            MembersFailure::Other => {
                format!("Could not load the member list of {name}: {detail}")
            }
        }
    }

    pub fn ingest_gateway_guild_members(
        &mut self,
        guild_id: &str,
        members: Vec<GuildMemberResponse>,
    ) {
        if members.is_empty() {
            return;
        }
        if self.guild_members_synced.contains(guild_id) {
            for m in members {
                self.merge_guild_member(guild_id, m);
            }
        } else {
            self.set_guild_members(guild_id, members);
        }
    }

    pub fn upsert_message(&mut self, message: MessageResponse) -> bool {
        if message.channel_id.is_empty() {
            return false;
        }
        // a blocked account's messages are not kept at all, which is what
        // keeps them out of the pane, the notifications and the unread
        // counts without a filter in any of those places
        if self.is_blocked(&message.author.id) {
            return false;
        }
        self.merge_message_embedded_members(&message);
        merge_user_cache(&mut self.user_cache, [message.author.clone()]);
        merge_user_cache(&mut self.user_cache, message.mentions.iter().cloned());

        let channel_id = message.channel_id.clone();
        self.messages_version = self.messages_version.wrapping_add(1);
        let entries = std::rc::Rc::make_mut(self.messages.entry(channel_id).or_default());

        if let Some(existing) = entries
            .iter_mut()
            .find(|existing| existing.id == message.id)
        {
            *existing = message;
            false
        } else {
            // almost always the newest: put it where it belongs from the end
            let key = snowflake_sort_key(&message.id);
            let at = entries
                .iter()
                .rposition(|e| snowflake_sort_key(&e.id) <= key)
                .map_or(0, |i| i + 1);
            entries.insert(at, message);
            true
        }
    }

    pub fn set_channel_messages(&mut self, channel_id: &str, mut messages: Vec<MessageResponse>) {
        messages.retain(|message| !self.is_blocked(&message.author.id));
        for message in &messages {
            self.merge_message_embedded_members(message);
            merge_user_cache(&mut self.user_cache, [message.author.clone()]);
        }
        messages.sort_by_key(|message| snowflake_sort_key(&message.id));
        const MAX_MESSAGES: usize = 500;
        if messages.len() < 50 {
            self.messages_older_exhausted.insert(channel_id.to_string());
        } else {
            self.messages_older_exhausted.remove(channel_id);
        }
        // Messages that came over the gateway while the fetch was on its
        // way, or before the channel was opened, and are newer than
        // anything fetched stay; the fetch is the history, not the present.
        if let Some(existing) = self.messages.get(channel_id) {
            let newest = messages.last().map(|m| snowflake_sort_key(&m.id));
            for m in existing.iter() {
                let key = snowflake_sort_key(&m.id);
                if newest.is_none_or(|n| key > n) && !messages.iter().any(|f| f.id == m.id) {
                    messages.push(m.clone());
                }
            }
            messages.sort_by_key(|message| snowflake_sort_key(&message.id));
        }
        if messages.len() > MAX_MESSAGES {
            messages.drain(0..messages.len() - MAX_MESSAGES);
        }
        self.messages_version = self.messages_version.wrapping_add(1);
        self.messages
            .insert(channel_id.to_string(), std::rc::Rc::new(messages));
        self.messages_loaded.insert(channel_id.to_string());
        self.loading_messages.remove(channel_id);
        self.api_backoff_clear(&format!("messages:{channel_id}"));
        self.message_scroll_from_bottom = 0;
    }

    pub fn prepend_channel_messages(&mut self, channel_id: &str, mut older: Vec<MessageResponse>) {
        older.retain(|message| !self.is_blocked(&message.author.id));
        for message in &older {
            self.merge_message_embedded_members(message);
            merge_user_cache(&mut self.user_cache, [message.author.clone()]);
        }
        self.messages_version = self.messages_version.wrapping_add(1);
        let entry = std::rc::Rc::make_mut(self.messages.entry(channel_id.to_string()).or_default());
        for m in older {
            if !entry.iter().any(|e| e.id == m.id) {
                entry.push(m);
            }
        }
        entry.sort_by_key(|m| snowflake_sort_key(&m.id));
        self.loading_older_messages.remove(channel_id);
    }

    pub fn remove_message(&mut self, channel_id: &str, message_id: &str) {
        if let Some(messages) = self.messages.get_mut(channel_id) {
            self.messages_version = self.messages_version.wrapping_add(1);
            std::rc::Rc::make_mut(messages).retain(|message| message.id != message_id);
        }
    }

    pub fn update_voice_state(&mut self, state: VoiceStateResponse) {
        let Some(guild_id) = state.guild_id.clone() else {
            return;
        };

        if let Some(member) = state.member.clone() {
            self.merge_guild_member(guild_id.as_str(), member);
        }

        let guild_states = self.voice_states.entry(guild_id).or_default();
        if state.channel_id.is_none() {
            guild_states.remove(&state.user_id);
        } else {
            guild_states.insert(state.user_id.clone(), state);
        }
    }

    pub fn voice_members_for_active_channel(&self) -> Vec<String> {
        let Some(channel) = self.active_channel() else {
            return Vec::new();
        };
        let Some(guild_id) = channel.guild_id else {
            return Vec::new();
        };
        let Some(states) = self.voice_states.get(&guild_id) else {
            return Vec::new();
        };

        let mut members = states
            .values()
            .filter(|state| state.channel_id.as_deref() == Some(channel.id.as_str()))
            .map(|state| {
                let name = if let Some(m) = state.member.as_ref() {
                    let u = self.user_cache.get(&m.user.id).unwrap_or(&m.user);
                    m.nick
                        .as_ref()
                        .filter(|n| !n.trim().is_empty())
                        .cloned()
                        .unwrap_or_else(|| account_display_name(u))
                } else if let Some(u) = self.user_cache.get(&state.user_id) {
                    self.shown_name_for_user(Some(guild_id.as_str()), u)
                } else {
                    state.user_id.clone()
                };

                let mut badges = Vec::new();
                if state.self_mute {
                    badges.push("self-muted");
                }
                if state.self_deaf {
                    badges.push("self-deaf");
                }
                if state.self_stream {
                    badges.push("streaming");
                }
                if state.self_video {
                    badges.push("video");
                }

                if badges.is_empty() {
                    name
                } else {
                    format!("{name} ({})", badges.join(", "))
                }
            })
            .collect::<Vec<_>>();
        members.sort();
        members
    }
    pub fn start_emoji_autocomplete(&mut self) {
        self.emoji_autocomplete = Some(EmojiAutocomplete {
            matches: Vec::new(),
            selected_index: 0,
        });
        self.update_emoji_filter();
    }

    pub fn update_emoji_filter(&mut self) {
        if self.emoji_autocomplete.is_none() {
            return;
        }

        // Typing the closing colon of a known name completes it in place,
        // like the web composer: ":eyes:" becomes the emoji and the popup closes.
        if self.reaction_target.is_none()
            && self.input.ends_with(':')
            && let Some(open) = self.input[..self.input.len() - 1].rfind(':')
        {
            let name = self.input[open + 1..self.input.len() - 1].to_string();
            // Inside an open code span the text is meant literally.
            let in_code = self.input[..open].matches('`').count() % 2 == 1;
            if !name.is_empty() && !name.contains(char::is_whitespace) && !in_code {
                if let Some(emoji) = crate::emoji::resolve(&name) {
                    self.input.truncate(open);
                    self.input.push_str(emoji);
                }
                // Unknown name: leave the text alone; the new colon may start
                // another shortcode, which reopens the popup on the next key.
                self.emoji_autocomplete = None;
                return;
            }
        }

        let query = self.input.rsplit(':').next().unwrap_or("").to_lowercase();
        let mut results: Vec<EmojiMatch> = Vec::new();

        // guild custom emojis first
        let guild_emojis: Vec<crate::api::types::GuildEmojiResponse> = self
            .guild_id_for_active_channel()
            .and_then(|gid| self.guild_emojis.get(&gid))
            .cloned()
            .unwrap_or_default();

        for e in &guild_emojis {
            if query.is_empty() || e.name.to_lowercase().contains(&query) {
                let prefix = if e.animated { "a" } else { "" };
                results.push(EmojiMatch {
                    label: format!(":{}:", e.name),
                    insert: format!("<{}:{}:{}>", prefix, e.name, e.id),
                    is_custom: true,
                    custom_id: Some(e.id.clone()),
                    custom_animated: e.animated,
                });
            }
            if results.len() >= 12 {
                break;
            }
        }

        // standard unicode emojis, by Fluxer name, best match first
        if results.len() < 12 {
            for c in crate::emoji::search(&query, 12 - results.len()) {
                results.push(EmojiMatch {
                    label: format!("{} :{}:", c.emoji, c.name),
                    insert: c.emoji.to_string(),
                    is_custom: false,
                    custom_id: None,
                    custom_animated: false,
                });
            }
        }

        let auto = self.emoji_autocomplete.as_mut().unwrap();
        auto.matches = results;
        if auto.selected_index >= auto.matches.len() {
            auto.selected_index = auto.matches.len().saturating_sub(1);
        }
        if auto.matches.is_empty() {
            self.emoji_autocomplete = None;
        }
    }

    pub fn dismiss_emoji_autocomplete(&mut self) {
        self.emoji_autocomplete = None;
    }

    pub fn autocomplete_emoji_next(&mut self) {
        if let Some(auto) = &mut self.emoji_autocomplete
            && !auto.matches.is_empty()
        {
            auto.selected_index = (auto.selected_index + 1) % auto.matches.len();
        }
    }

    pub fn autocomplete_emoji_prev(&mut self) {
        if let Some(auto) = &mut self.emoji_autocomplete
            && !auto.matches.is_empty()
        {
            auto.selected_index =
                auto.selected_index.saturating_add(auto.matches.len() - 1) % auto.matches.len();
        }
    }

    pub fn insert_selected_emoji(&mut self) -> bool {
        if let Some(auto) = &self.emoji_autocomplete
            && let Some(emoji) = auto.matches.get(auto.selected_index)
            && let Some(colon_pos) = self.input.rfind(':')
        {
            self.input.truncate(colon_pos);
            self.input.push_str(&emoji.insert);
            self.input.push(' ');
            self.emoji_autocomplete = None;
            return true;
        }
        false
    }

    // rs

    pub fn set_read_states(&mut self, states: Vec<ReadStateResponse>) {
        for s in states {
            if !s.id.is_empty() {
                self.read_states.insert(
                    s.id,
                    ReadState {
                        last_message_id: s.last_message_id,
                        mention_count: s.mention_count,
                    },
                );
            }
        }
    }

    pub fn ack_channel(&mut self, channel_id: &str) {
        let last_msg = self.channel_last_message_id(channel_id);
        if let Some(msg_id) = last_msg {
            self.read_states.insert(
                channel_id.to_string(),
                ReadState {
                    last_message_id: Some(msg_id),
                    mention_count: 0,
                },
            );
        }
    }

    pub fn channel_is_unread(&self, channel_id: &str) -> bool {
        let Some(rs) = self.read_states.get(channel_id) else {
            return false;
        };
        let channel_last = self.channel_last_message_id(channel_id);
        match (&rs.last_message_id, &channel_last) {
            (Some(read), Some(last)) => snowflake_sort_key(read) < snowflake_sort_key(last),
            (None, Some(_)) => true,
            _ => false,
        }
    }

    pub fn channel_mention_count(&self, channel_id: &str) -> u64 {
        self.read_states
            .get(channel_id)
            .map(|rs| rs.mention_count)
            .unwrap_or(0)
    }

    fn channel_counts_toward_server_unread(&self, channel: &ChannelResponse) -> bool {
        channel.channel_type() != CHANNEL_GUILD_CATEGORY
            && (channel.channel_type() != CHANNEL_GUILD_VOICE
                || self.visible_channel_mention_count(&channel.id) > 0)
    }

    pub fn server_unread_channel_count(&self, server: &ServerSelection) -> usize {
        self.all_channels_for_server(server)
            .into_iter()
            .filter(|channel| self.channel_counts_toward_server_unread(channel))
            .filter(|channel| self.visible_channel_is_unread(&channel.id))
            .count()
    }

    pub fn server_mention_count(&self, server: &ServerSelection) -> u64 {
        self.all_channels_for_server(server)
            .into_iter()
            .filter(|channel| channel.channel_type() != CHANNEL_GUILD_CATEGORY)
            .map(|channel| self.visible_channel_mention_count(&channel.id))
            .sum()
    }

    pub(crate) fn channel_last_message_id(&self, channel_id: &str) -> Option<String> {
        let cached_last = self
            .messages
            .get(channel_id)
            .and_then(|msgs| msgs.last())
            .map(|msg| msg.id.clone());
        let channel_last = self
            .private_channels
            .iter()
            .chain(self.guild_channels.values().flat_map(|v| v.iter()))
            .find(|c| c.id == channel_id)
            .and_then(|c| c.last_message_id.clone());

        match (cached_last, channel_last) {
            (Some(cached), Some(channel)) => {
                if snowflake_sort_key(&cached) >= snowflake_sort_key(&channel) {
                    Some(cached)
                } else {
                    Some(channel)
                }
            }
            (Some(cached), None) => Some(cached),
            (None, Some(channel)) => Some(channel),
            (None, None) => None,
        }
    }

    // ms

    pub fn move_selected_message(&mut self, delta: i32) {
        let count = self.active_messages().len();
        if count == 0 {
            self.selected_message_index = None;
            return;
        }
        let current = self
            .selected_message_index
            .unwrap_or(count.saturating_sub(1));
        let next = (current as i32 + delta).clamp(0, count as i32 - 1) as usize;
        self.selected_message_index = Some(next);
        self.clamp_scroll_to_selected_message();
    }

    /// Scroll the pane so the selected message is on it. The pane's
    /// layout depends on the selection -- a message grouped under the one
    /// before it gains a timestamp row while it is selected -- so moving
    /// the selection across the edge of a group changes the content's
    /// height, which is what the reader anchor takes for a message
    /// arriving and puts the view back for. Drop the anchor: the scroll
    /// worked out here is the one to draw, and the next draw anchors on
    /// it again.
    pub fn clamp_scroll_to_selected_message(&mut self) {
        let (w, h) = self.chafa_viewport;
        if w == 0 || h == 0 {
            return;
        }
        if let Some(s) = crate::ui::message_pane::scroll_for_selected_message(
            self,
            w.max(1),
            h.max(1),
            self.message_scroll_from_bottom,
        ) {
            self.message_scroll_from_bottom = s;
            self.pane_anchor = None;
        }
    }

    pub fn selected_message(&self) -> Option<MessageResponse> {
        let msgs = self.active_messages();
        self.selected_message_index
            .and_then(|i| msgs.get(i).cloned())
    }

    /// y or Ctrl+C on a selected message: its text into the cut buffer,
    /// where Alt+V picks it up in the compose box, and into the system
    /// clipboard where a program for one exists. None when the message
    /// carries no text at all; the bool says whether the clipboard took
    /// it, which it never does on the console.
    pub fn copy_selected_message(&mut self) -> Option<bool> {
        let msg = self.selected_message()?;
        let text = message_copy_text(&msg);
        if text.is_empty() {
            return None;
        }
        let to_clipboard = crate::compose::copy_to_system_clipboard(&text);
        self.cut_buffer = text;
        Some(to_clipboard)
    }

    // presence

    /// Somebody's online state. An account the server has said nothing
    /// about is offline: it only sends presences for people the reader
    /// shares a community or a conversation with.
    pub fn presence_status(&self, user_id: &str) -> PresenceStatus {
        if user_id == self.me.id {
            return self.own_status();
        }
        self.presences
            .get(user_id)
            .map(|entry| entry.status)
            .unwrap_or_default()
    }

    pub fn presence_entry(&self, user_id: &str) -> Option<&PresenceEntry> {
        self.presences.get(user_id)
    }

    /// The reader's own status, which is a setting rather than a
    /// presence: the server does not send the reader their own.
    pub fn own_status(&self) -> PresenceStatus {
        self.user_settings
            .as_ref()
            .map(|s| PresenceStatus::parse(&s.status))
            .unwrap_or(PresenceStatus::Online)
    }

    pub fn own_custom_status(&self) -> Option<&CustomStatusPayload> {
        self.user_settings.as_ref()?.custom_status.as_ref()
    }

    /// Take one PRESENCE_UPDATE, or one entry of a ready payload.
    pub fn apply_presence(&mut self, record: PresenceRecord) {
        let user_id = record.user.id.clone();
        if user_id.is_empty() {
            return;
        }
        if !record.user.username.is_empty() {
            merge_user_cache(&mut self.user_cache, [record.user.clone()]);
        }
        let status = record
            .status
            .as_deref()
            .map(PresenceStatus::parse)
            .unwrap_or_default();
        // an offline presence with nothing else on it is the server
        // saying they have gone; keeping the row would only cost memory
        self.presence_version = self.presence_version.wrapping_add(1);
        if status.is_offline() && record.custom_status.is_none() {
            self.presences.remove(&user_id);
            return;
        }
        self.presences.insert(
            user_id,
            PresenceEntry {
                status,
                mobile: record.mobile,
                custom_status: record.custom_status,
            },
        );
    }

    pub fn apply_presences(&mut self, records: Vec<PresenceRecord>) {
        for record in records {
            self.apply_presence(record);
        }
    }

    /// The other person in a one-to-one conversation, whose presence is
    /// what the channel row shows.
    pub fn dm_peer_id(&self, channel: &ChannelResponse) -> Option<String> {
        if channel.channel_type() != CHANNEL_DM {
            return None;
        }
        channel
            .recipients
            .iter()
            .find(|u| u.id != self.me.id)
            .map(|u| u.id.clone())
    }

    /// Set the reader's own status locally, so the screen follows before
    /// the server has answered.
    pub fn set_own_status(&mut self, status: PresenceStatus) {
        if let Some(settings) = &mut self.user_settings {
            settings.status = status.wire().to_string();
        }
    }

    pub fn set_own_custom_status(&mut self, custom: Option<CustomStatusPayload>) {
        if let Some(settings) = &mut self.user_settings {
            settings.custom_status = custom;
        }
    }

    // Alt+M: the member list

    /// How many rows the list asks for at a time. The server takes at
    /// most a hundred per window, which is more than any terminal shows.
    pub const MEMBER_LIST_WINDOW: u32 = 100;

    /// Open the member list on the channel now showing, or shut it. The
    /// caller sends the subscription; this only says what to ask for.
    pub fn toggle_member_list(&mut self) -> Option<(String, Option<String>)> {
        if let Some(list) = self.member_list.take() {
            // giving the list up is a subscription with no channel
            return Some((list.guild_id, None));
        }
        let guild_id = self.active_guild_id()?;
        let channel_id = self.active_channel_id()?;
        self.member_list = Some(MemberList {
            guild_id: guild_id.clone(),
            channel_id: channel_id.clone(),
            ..Default::default()
        });
        Some((guild_id, Some(channel_id)))
    }

    pub fn close_member_list(&mut self) -> Option<String> {
        self.member_list.take().map(|list| list.guild_id)
    }

    /// Follow the channel the reader moved to, so the list is never of
    /// somewhere else. Gives back the subscription to send.
    pub fn member_list_follow_channel(&mut self) -> Option<(String, Option<String>)> {
        let list = self.member_list.as_ref()?;
        let guild_id = self.active_guild_id();
        let channel_id = self.active_channel_id();
        match (guild_id, channel_id) {
            (Some(guild_id), Some(channel_id))
                if guild_id == list.guild_id && channel_id == list.channel_id =>
            {
                None
            }
            (Some(guild_id), Some(channel_id)) => {
                self.member_list = Some(MemberList {
                    guild_id: guild_id.clone(),
                    channel_id: channel_id.clone(),
                    ..Default::default()
                });
                Some((guild_id, Some(channel_id)))
            }
            // a direct message has no member list; the pane closes and
            // the guild's subscription is given up
            _ => {
                let guild_id = list.guild_id.clone();
                self.member_list = None;
                Some((guild_id, None))
            }
        }
    }

    /// Take one GUILD_MEMBER_LIST_UPDATE. Only the list now open is
    /// followed; the server sends at most one per guild anyway.
    pub fn apply_member_list_update(&mut self, event: GuildMemberListUpdateEvent) {
        let channel_id = event.channel_id.clone().unwrap_or_else(|| event.id.clone());
        let Some(list) = self.member_list.as_mut() else {
            return;
        };
        if list.guild_id != event.guild_id || list.channel_id != channel_id {
            return;
        }
        list.member_count = event.member_count;
        list.online_count = event.online_count;
        list.loaded = true;

        let mut presences = Vec::new();
        let mut members = Vec::new();
        for op in &event.ops {
            // SYNC is the only operation the server sends; anything else
            // is for a client that knows more than this one
            if op.op != "SYNC" {
                continue;
            }
            let (Some(start), Some(end)) = (op.range.first().copied(), op.range.get(1).copied())
            else {
                continue;
            };
            if end < start {
                continue;
            }
            // the range is replaced whole: rows it covers that the
            // operation does not fill are gone
            for row in start..=end {
                list.rows.remove(&row);
            }
            for (offset, item) in op.items.iter().enumerate() {
                let row = start + offset as u32;
                if row > end {
                    break;
                }
                if let Some(group) = &item.group {
                    list.rows.insert(
                        row,
                        MemberRow::Heading {
                            label: group.id.clone(),
                            count: group.count,
                        },
                    );
                } else if let Some(entry) = &item.member {
                    let user_id = entry.member.user.id.clone();
                    if user_id.is_empty() {
                        continue;
                    }
                    list.rows.insert(
                        row,
                        MemberRow::Member {
                            user_id: user_id.clone(),
                        },
                    );
                    list.members.insert(user_id, entry.member.clone());
                    members.push(entry.member.clone());
                    if let Some(presence) = &entry.presence {
                        let mut presence = presence.clone();
                        // the list's placeholder presence carries no user
                        if presence.user.id.is_empty() {
                            presence.user = entry.member.user.clone();
                        }
                        presences.push(presence);
                    }
                }
            }
            list.known_rows = list.known_rows.max(end.saturating_add(1));
        }
        let guild_id = event.guild_id.clone();
        for presence in presences {
            self.apply_presence(presence);
        }
        if !members.is_empty() {
            self.ingest_gateway_guild_members(&guild_id, members);
        }
    }

    /// The rows the pane draws, in order, up to what has arrived. A gap
    /// where a window has not come back yet is left out rather than shown
    /// as a blank, so the list never looks like it has holes in it.
    pub fn member_list_rows(&self) -> Vec<MemberRow> {
        let Some(list) = self.member_list.as_ref() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for index in 0..list.known_rows {
            if let Some(row) = list.rows.get(&index) {
                rows.push(row.clone());
            }
        }
        rows
    }

    /// What to call a member list heading. The server sends a hoisted
    /// role's id, or the words `online` and `offline`.
    pub fn member_group_label(&self, guild_id: &str, id: &str) -> String {
        match id {
            "online" => "Online".to_string(),
            "offline" => "Offline".to_string(),
            role_id => self
                .guild_roles
                .get(guild_id)
                .and_then(|roles| roles.iter().find(|r| r.id.trim() == role_id.trim()))
                .map(|r| r.name.clone())
                .unwrap_or_else(|| "Members".to_string()),
        }
    }

    pub fn member_list_scroll(&mut self, delta: i32) {
        if let Some(list) = self.member_list.as_mut() {
            list.scroll = list.scroll.saturating_add_signed(delta as i16);
        }
    }
    // Alt+F: friends, requests and blocked accounts

    pub fn open_friends(&mut self) {
        self.close_conversation_overlays();
        self.friends = Some(FriendsView {
            state: if self.relationships.is_empty() {
                FriendsState::Loading
            } else {
                // what arrived at start is shown at once and refreshed
                // behind it, so the list is never blank for no reason
                FriendsState::Ready
            },
            selected: 0,
            tab: FriendsTab::Friends,
            input: None,
        });
    }

    // Alt+N: starting a conversation, and looking after one

    pub fn open_new_conversation(&mut self) {
        self.close_conversation_overlays();
        self.conversation = Some(ConversationView {
            mode: ConversationMode::People,
            filter: String::new(),
            selected: 0,
            marked: Vec::new(),
            input: None,
        });
    }

    /// The menu for the group now open. None when the channel showing is
    /// not a group.
    pub fn open_group_menu(&mut self) -> bool {
        let Some(channel) = self
            .active_channel_id()
            .and_then(|id| self.channel_by_id(&id).cloned())
        else {
            return false;
        };
        if channel.channel_type() != CHANNEL_GROUP_DM {
            return false;
        }
        self.close_conversation_overlays();
        self.conversation = Some(ConversationView {
            mode: ConversationMode::Group {
                channel_id: channel.id,
            },
            filter: String::new(),
            selected: 0,
            marked: Vec::new(),
            input: None,
        });
        true
    }

    /// Shut every overlay, so opening one never leaves another under it.
    /// Every `open_*` starts here.
    fn close_conversation_overlays(&mut self) {
        self.close_overlays();
    }

    // Alt+C: joining, making and leaving communities

    pub fn open_communities(&mut self) {
        self.close_overlays();
        self.community = Some(CommunityView {
            mode: CommunityMode::Menu,
            selected: 0,
            input: None,
        });
    }

    pub fn dismiss_communities(&mut self) {
        self.community = None;
    }

    pub fn dismiss_friends(&mut self) {
        self.friends = None;
    }

    pub fn set_relationships(&mut self, list: Vec<RelationshipResponse>) {
        merge_user_cache(&mut self.user_cache, list.iter().map(|r| r.user.clone()));
        self.relationships = list
            .into_iter()
            .filter(|r| !r.user.id.is_empty())
            .map(|r| (r.user.id.clone(), r))
            .collect();
        self.relationships_version = self.relationships_version.wrapping_add(1);
        if let Some(view) = &mut self.friends {
            view.state = FriendsState::Ready;
            view.selected = 0;
        }
    }

    pub fn set_relationships_failed(&mut self, message: String) {
        if let Some(view) = &mut self.friends {
            view.state = FriendsState::Failed(message);
        }
    }

    /// Take one RELATIONSHIP_ADD or _UPDATE.
    pub fn upsert_relationship(&mut self, relationship: RelationshipResponse) {
        if relationship.user.id.is_empty() {
            return;
        }
        merge_user_cache(&mut self.user_cache, [relationship.user.clone()]);
        self.relationships
            .insert(relationship.user.id.clone(), relationship);
        self.relationships_version = self.relationships_version.wrapping_add(1);
        self.clamp_friends_selection();
    }

    pub fn remove_relationship(&mut self, user_id: &str) {
        self.relationships.remove(user_id);
        self.relationships_version = self.relationships_version.wrapping_add(1);
        self.clamp_friends_selection();
    }

    /// Blocking somebody takes what they have already said out of every
    /// loaded channel, since new messages are dropped at the door and the
    /// pane would otherwise keep showing the old ones.
    pub fn forget_messages_from(&mut self, user_id: &str) {
        let channels: Vec<String> = self
            .messages
            .iter()
            .filter(|(_, messages)| messages.iter().any(|m| m.author.id == user_id))
            .map(|(id, _)| id.clone())
            .collect();
        if channels.is_empty() {
            return;
        }
        for channel_id in channels {
            if let Some(messages) = self.messages.get_mut(&channel_id) {
                std::rc::Rc::make_mut(messages).retain(|m| m.author.id != user_id);
            }
        }
        self.messages_version = self.messages_version.wrapping_add(1);
        self.normalize_selection();
    }

    /// Unblocking cannot bring back what was thrown away, so the channels
    /// they were in are marked unloaded and fetched again.
    pub fn reload_channels_for(&mut self, user_id: &str) {
        let channels: Vec<String> = self
            .private_channels
            .iter()
            .filter(|c| c.recipients.iter().any(|u| u.id == user_id))
            .map(|c| c.id.clone())
            .collect();
        let here = self.active_channel_id();
        for channel_id in channels.into_iter().chain(here) {
            self.messages.remove(&channel_id);
            self.messages_loaded.remove(&channel_id);
            self.messages_older_exhausted.remove(&channel_id);
            self.loading_messages.remove(&channel_id);
            self.api_backoff_clear_channel_messages(&channel_id);
        }
        self.messages_version = self.messages_version.wrapping_add(1);
        self.normalize_selection();
    }

    /// What the three relationship keys do for somebody, and what to call
    /// each on the hint line. Nothing at all for the reader themselves.
    pub fn relationship_keys_for(&self, user_id: &str) -> RelationshipKeys {
        if user_id == self.me.id {
            return RelationshipKeys::default();
        }
        let block = Some((RelationshipAction::Block, "block"));
        match self.relationships.get(user_id).map(|r| r.relationship_type) {
            None => RelationshipKeys {
                plus: Some((RelationshipAction::Add, "add friend")),
                block,
                undo: None,
            },
            Some(RELATIONSHIP_FRIEND) => RelationshipKeys {
                plus: None,
                block,
                undo: Some((RelationshipAction::Remove, "unfriend")),
            },
            // they asked first, so `+` accepts rather than asking back:
            // the server takes a different call for each
            Some(RELATIONSHIP_INCOMING_REQUEST) => RelationshipKeys {
                plus: Some((RelationshipAction::Accept, "accept")),
                block,
                undo: Some((RelationshipAction::Remove, "turn down")),
            },
            Some(RELATIONSHIP_OUTGOING_REQUEST) => RelationshipKeys {
                plus: None,
                block,
                undo: Some((RelationshipAction::Remove, "take it back")),
            },
            Some(RELATIONSHIP_BLOCKED) => RelationshipKeys {
                plus: None,
                // blocking somebody already blocked is nothing to offer
                block: None,
                undo: Some((RelationshipAction::Remove, "unblock")),
            },
            Some(_) => RelationshipKeys {
                plus: None,
                block,
                undo: Some((RelationshipAction::Remove, "undo")),
            },
        }
    }

    pub fn relationship_with(&self, user_id: &str) -> Option<&RelationshipResponse> {
        self.relationships.get(user_id)
    }

    /// Whether the reader has blocked this account. Their messages are
    /// hidden from the pane and they raise no notification.
    pub fn is_blocked(&self, user_id: &str) -> bool {
        self.relationships
            .get(user_id)
            .is_some_and(|r| r.is_blocked())
    }

    /// The name the reader gave a friend, where they gave one.
    pub fn relationship_nickname(&self, user_id: &str) -> Option<&str> {
        self.relationships
            .get(user_id)?
            .nickname
            .as_deref()
            .filter(|n| !n.trim().is_empty())
    }

    /// The people in one group of the friends overlay, by name.
    pub fn relationships_in(&self, tab: FriendsTab) -> Vec<RelationshipResponse> {
        let wanted = tab.relationship_type();
        let mut out: Vec<RelationshipResponse> = self
            .relationships
            .values()
            .filter(|r| r.relationship_type == wanted)
            .cloned()
            .collect();
        out.sort_by_key(|r| {
            self.relationship_nickname(&r.user.id)
                .map(str::to_string)
                .unwrap_or_else(|| display_name(&r.user))
                .to_lowercase()
        });
        out
    }

    pub fn friends_selected(&self) -> Option<RelationshipResponse> {
        let view = self.friends.as_ref()?;
        self.relationships_in(view.tab).get(view.selected).cloned()
    }

    pub fn friends_move(&mut self, delta: isize) {
        let Some(tab) = self.friends.as_ref().map(|v| v.tab) else {
            return;
        };
        let count = self.relationships_in(tab).len();
        if let Some(view) = &mut self.friends {
            view.selected = if count == 0 {
                0
            } else {
                (view.selected as isize + delta).clamp(0, count as isize - 1) as usize
            };
        }
    }

    /// Whether the user may take other people's things off a message in
    /// this channel: clearing reactions, pinning, deleting in bulk.
    pub fn can_manage_messages(&self) -> bool {
        self.active_channel_permissions() & crate::permissions::MANAGE_MESSAGES != 0
    }

    pub fn is_bookmarked(&self, message_id: &str) -> bool {
        self.saved_message_ids.contains(message_id)
    }

    /// The messages marked with `m` in the channel now open.
    pub fn marked_in_active_channel(&self) -> Vec<String> {
        self.active_channel_id()
            .and_then(|id| self.marked_messages.get(&id).cloned())
            .map(|set| {
                let mut ids: Vec<String> = set.into_iter().collect();
                ids.sort_by_key(|id| snowflake_sort_key(id));
                ids
            })
            .unwrap_or_default()
    }

    pub fn dismiss_conversation(&mut self) {
        self.conversation = None;
        self.pending_group_add = None;
    }

    /// Everybody the client knows well enough to start talking to: the
    /// people already in a conversation, and the members of every
    /// community. The reader themselves is left out, and so is anybody
    /// there is already a one-to-one with, since Enter on them would only
    /// open what the channel list already holds.
    pub fn conversation_candidates(&self) -> Vec<Candidate> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<Candidate> = Vec::new();
        seen.insert(self.me.id.clone());

        for channel in &self.private_channels {
            let note = match channel.channel_type() {
                CHANNEL_GROUP_DM => "in a group with you".to_string(),
                _ => "you talk already".to_string(),
            };
            for user in &channel.recipients {
                if seen.insert(user.id.clone()) {
                    out.push(Candidate {
                        user: user.clone(),
                        note: note.clone(),
                    });
                }
            }
        }
        for guild in &self.guilds {
            let Some(members) = self.guild_members.get(&guild.id) else {
                continue;
            };
            for member in members {
                if seen.insert(member.user.id.clone()) {
                    out.push(Candidate {
                        user: member.user.clone(),
                        note: guild.name.clone(),
                    });
                }
            }
        }
        out.sort_by_key(|c| display_name(&c.user).to_lowercase());
        out
    }

    /// The candidates that match what has been typed, by display name,
    /// username or tag.
    pub fn conversation_matches(&self) -> Vec<Candidate> {
        let Some(view) = self.conversation.as_ref() else {
            return Vec::new();
        };
        let needle = view.filter.trim().to_lowercase();
        let all = self.conversation_candidates();
        if needle.is_empty() {
            return all;
        }
        all.into_iter()
            .filter(|c| {
                display_name(&c.user).to_lowercase().contains(&needle)
                    || c.user.username.to_lowercase().contains(&needle)
                    || format!("{}#{}", c.user.username, c.user.discriminator)
                        .to_lowercase()
                        .contains(&needle)
            })
            .collect()
    }

    /// The people in the group the overlay is about.
    pub fn group_members(&self, channel_id: &str) -> Vec<UserPartialResponse> {
        self.channel_by_id(channel_id)
            .map(|c| {
                c.recipients
                    .iter()
                    .filter(|u| u.id != self.me.id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn is_marked(&self, channel_id: &str, message_id: &str) -> bool {
        self.marked_messages
            .get(channel_id)
            .is_some_and(|set| set.contains(message_id))
    }

    /// `m` on a selected message: mark it, or take the mark off. Marks
    /// are what a bulk delete works on and are kept per channel.
    pub fn toggle_mark_selected(&mut self) -> Option<(bool, usize)> {
        let msg = self.selected_message()?;
        let set = self
            .marked_messages
            .entry(msg.channel_id.clone())
            .or_default();
        let marked = if set.remove(&msg.id) {
            false
        } else {
            set.insert(msg.id.clone());
            true
        };
        let count = set.len();
        if count == 0 {
            self.marked_messages.remove(&msg.channel_id);
        }
        Some((marked, count))
    }

    pub fn clear_marks_for_channel(&mut self, channel_id: &str) {
        self.marked_messages.remove(channel_id);
    }

    /// A link to a message, the shape the web client copies: the web
    /// app's own address, the community (or `@me` in a direct message),
    /// the channel and the message.
    pub fn message_link(&self, channel_id: &str, message_id: &str) -> Option<String> {
        let base = self.discovery.endpoints.webapp.trim_end_matches('/');
        if base.is_empty() {
            return None;
        }
        let guild = self
            .guild_id_for_channel(channel_id)
            .unwrap_or_else(|| "@me".to_string());
        Some(format!("{base}/channels/{guild}/{channel_id}/{message_id}"))
    }

    /// Put text on the system clipboard and always in the cut buffer, the
    /// way `copy_selected_message` does; the bool says whether a
    /// Put text on the system clipboard and always in the cut buffer,
    /// the way `copy_selected_message` does; the bool says whether a
    /// clipboard program took it.
    pub fn copy_text_out(&mut self, text: String) -> bool {
        let to_clipboard = crate::compose::copy_to_system_clipboard(&text);
        self.cut_buffer = text;
        to_clipboard
    }

    // a (as in actions): the message actions menu

    /// Which rows the menu offers for a message. Order follows the web
    /// client's menu: reactions, then the things that write a message,
    /// then the ones that only move it about, then the destructive ones.
    pub fn message_actions_for(&self, msg: &MessageResponse) -> Vec<MessageAction> {
        let mut out = Vec::new();
        let text_channel = self.active_channel_is_text();
        let can_send = self.can_send_in_active_channel();
        let manage = self.can_manage_messages();
        let mine = msg.author.id == self.me.id;

        if text_channel && can_send {
            out.push(MessageAction::React);
        }
        if !msg.reactions.is_empty() {
            out.push(MessageAction::ViewReactions);
            if manage {
                out.push(MessageAction::ClearReactions);
            }
        }
        if text_channel && can_send {
            out.push(MessageAction::Reply);
            out.push(MessageAction::Forward);
        }
        if mine && text_channel && can_send {
            out.push(MessageAction::Edit);
            if msg.embeds.is_empty() && msg.flags & MESSAGE_FLAG_SUPPRESS_EMBEDS != 0 {
                out.push(MessageAction::ShowEmbeds);
            } else if !msg.embeds.is_empty() {
                out.push(MessageAction::SuppressEmbeds);
            }
            if !msg.attachments.is_empty() {
                out.push(MessageAction::RemoveAttachment);
            }
        }
        if manage || (mine && text_channel) {
            out.push(if msg.pinned {
                MessageAction::Unpin
            } else {
                MessageAction::Pin
            });
        }
        out.push(MessageAction::ViewPins);
        out.push(if self.is_bookmarked(&msg.id) {
            MessageAction::Unbookmark
        } else {
            MessageAction::Bookmark
        });
        out.push(MessageAction::ViewSaved);
        out.push(MessageAction::MarkUnread);
        out.push(MessageAction::MarkChannelRead);
        if self.active_guild_id().is_some() {
            out.push(MessageAction::MarkGuildRead);
        }
        if !message_copy_text(msg).is_empty() {
            out.push(MessageAction::CopyText);
        }
        if self.message_link(&msg.channel_id, &msg.id).is_some() {
            out.push(MessageAction::CopyLink);
        }
        out.push(MessageAction::CopyId);
        if self.can_delete_message(msg) {
            out.push(MessageAction::Delete);
        }
        if manage && self.marked_in_active_channel().len() >= 2 {
            out.push(MessageAction::DeleteMarked);
        }
        if !mine {
            out.push(MessageAction::Report);
        }
        out
    }

    /// Which rows the menu offers. The two that act on a community are
    /// only there when one is open.
    pub fn community_actions(&self) -> Vec<CommunityAction> {
        let mut out = vec![
            CommunityAction::Join,
            CommunityAction::Create,
            CommunityAction::Discover,
        ];
        if self.active_guild_id().is_some() {
            out.push(CommunityAction::Invites);
            out.push(CommunityAction::Leave);
        }
        out
    }

    // `a` on the channel list: looking after one community channel

    /// Whether the reader can make, change and delete channels in a
    /// community. MANAGE_CHANNELS is a guild-level permission for making
    /// one and a channel-level permission for changing one, so the menu
    /// reads it on the channel the cursor is on.
    pub fn can_manage_channel(&self, channel: &ChannelResponse) -> bool {
        channel.guild_id.is_some()
            && self.channel_permissions(channel) & crate::permissions::MANAGE_CHANNELS != 0
    }

    /// What the channel menu offers for the channel the cursor is on.
    /// Copying the id is always there; the rest needs MANAGE_CHANNELS.
    pub fn channel_admin_actions(&self, channel: &ChannelResponse) -> Vec<ChannelAdminAction> {
        let mut out = Vec::new();
        let manage = self.can_manage_channel(channel);
        if manage {
            out.push(ChannelAdminAction::New);
            out.push(ChannelAdminAction::Rename);
            let kind = channel.channel_type();
            if matches!(kind, CHANNEL_GUILD_TEXT | CHANNEL_GUILD_VOICE) {
                out.push(ChannelAdminAction::Topic);
                if channel.topic.as_deref().is_some_and(|t| !t.is_empty()) {
                    out.push(ChannelAdminAction::ClearTopic);
                }
                out.push(ChannelAdminAction::Slowmode);
            }
        }
        out.push(ChannelAdminAction::CopyId);
        if manage {
            out.push(ChannelAdminAction::Delete);
        }
        out
    }

    /// Open the menu on the channel the cursor is on. False when there is
    /// no community channel there, which is what makes the key quiet in a
    /// conversation list.
    pub fn open_channel_admin(&mut self) -> bool {
        let Some(channel) = self.active_channel() else {
            return false;
        };
        let Some(guild_id) = channel.guild_id.clone() else {
            return false;
        };
        let actions = self.channel_admin_actions(&channel);
        if actions.is_empty() {
            return false;
        }
        self.close_overlays();
        self.channel_admin = Some(ChannelAdminView {
            channel_id: channel.id.clone(),
            guild_id,
            channel_name: channel.name.clone(),
            mode: ChannelAdminMode::Actions,
            actions,
            selected: 0,
            input: None,
        });
        true
    }

    /// How many rows the channel menu is showing.
    pub fn channel_admin_len(&self) -> usize {
        let Some(view) = &self.channel_admin else {
            return 0;
        };
        match view.mode {
            ChannelAdminMode::Actions => view.actions.len(),
            ChannelAdminMode::NewKind => NEW_CHANNEL_KINDS.len(),
            ChannelAdminMode::ConfirmDelete => 2,
        }
    }

    pub fn channel_admin_move(&mut self, delta: isize) {
        let count = self.channel_admin_len();
        if let Some(view) = &mut self.channel_admin {
            view.selected = if count == 0 {
                0
            } else {
                (view.selected as isize + delta).clamp(0, count as isize - 1) as usize
            };
        }
    }

    /// Esc: out of the typing, then out of a sub-list, then closed.
    pub fn channel_admin_back(&mut self) {
        let Some(view) = &mut self.channel_admin else {
            return;
        };
        if view.input.is_some() {
            view.input = None;
            return;
        }
        if matches!(view.mode, ChannelAdminMode::Actions) {
            self.channel_admin = None;
        } else {
            view.mode = ChannelAdminMode::Actions;
            view.selected = 0;
        }
    }

    pub fn channel_admin_selected_action(&self) -> Option<ChannelAdminAction> {
        let view = self.channel_admin.as_ref()?;
        match view.mode {
            ChannelAdminMode::Actions => view.actions.get(view.selected).copied(),
            _ => None,
        }
    }

    pub fn dismiss_channel_admin(&mut self) {
        self.channel_admin = None;
    }

    pub fn open_message_actions(&mut self) -> bool {
        let Some(msg) = self.selected_message() else {
            return false;
        };
        let actions = self.message_actions_for(&msg);
        if actions.is_empty() {
            return false;
        }
        self.close_overlays();
        self.message_actions = Some(MessageActionsView {
            channel_id: msg.channel_id.clone(),
            message_id: msg.id.clone(),
            mode: MessageActionsMode::Actions,
            actions,
            selected: 0,
        });
        true
    }

    /// How many rows the menu is showing, whichever mode it is in.
    pub fn message_actions_len(&self) -> usize {
        match self.message_actions.as_ref() {
            None => 0,
            Some(view) => match &view.mode {
                MessageActionsMode::Actions => view.actions.len(),
                MessageActionsMode::Attachments(items) => items.len(),
                MessageActionsMode::ReportCategories => REPORT_CATEGORIES.len(),
                MessageActionsMode::Confirm(_) => 2,
            },
        }
    }

    pub fn message_actions_move(&mut self, delta: isize) {
        let count = self.message_actions_len();
        if let Some(view) = &mut self.message_actions {
            view.selected = if count == 0 {
                0
            } else {
                (view.selected as isize + delta).clamp(0, count as isize - 1) as usize
            };
        }
    }

    pub fn community_len(&self) -> usize {
        match self.community.as_ref().map(|v| &v.mode) {
            Some(CommunityMode::Menu) => self.community_actions().len(),
            Some(CommunityMode::Discover {
                state: DiscoverState::Ready(guilds),
                ..
            }) => guilds.len(),
            Some(CommunityMode::Invites {
                state: InvitesState::Ready(invites),
                ..
            }) => invites.len(),
            // nothing to move through while it is still coming, and the
            // preview is one thing rather than a list
            _ => 0,
        }
    }

    pub fn community_move(&mut self, delta: isize) {
        let count = self.community_len();
        if let Some(view) = &mut self.community {
            view.selected = if count == 0 {
                0
            } else {
                (view.selected as isize + delta).clamp(0, count as isize - 1) as usize
            };
        }
    }

    /// What the group menu offers. Renaming and adding are open to
    /// anybody in a group; the server decides the rest.
    pub fn group_actions(&self, channel_id: &str) -> Vec<GroupAction> {
        let mut out = vec![GroupAction::Rename, GroupAction::AddSomebody];
        if !self.group_members(channel_id).is_empty() {
            out.push(GroupAction::RemoveSomebody);
        }
        out.push(GroupAction::Leave);
        out
    }

    /// How many rows the overlay is showing, whichever mode it is in.
    pub fn conversation_len(&self) -> usize {
        match self.conversation.as_ref().map(|v| &v.mode) {
            None => 0,
            Some(ConversationMode::People) => self.conversation_matches().len(),
            Some(ConversationMode::Group { channel_id }) => self.group_actions(channel_id).len(),
            Some(ConversationMode::RemoveFrom { channel_id }) => {
                self.group_members(channel_id).len()
            }
        }
    }

    pub fn conversation_move(&mut self, delta: isize) {
        let count = self.conversation_len();
        if let Some(view) = &mut self.conversation {
            view.selected = if count == 0 {
                0
            } else {
                (view.selected as isize + delta).clamp(0, count as isize - 1) as usize
            };
        }
    }

    pub fn friends_switch_tab(&mut self, forward: bool) {
        if let Some(view) = &mut self.friends {
            view.tab = if forward {
                view.tab.next()
            } else {
                view.tab.previous()
            };
            view.selected = 0;
        }
    }

    pub fn conversation_selected_user(&self) -> Option<UserPartialResponse> {
        let view = self.conversation.as_ref()?;
        match &view.mode {
            ConversationMode::People => self
                .conversation_matches()
                .get(view.selected)
                .map(|c| c.user.clone()),
            ConversationMode::RemoveFrom { channel_id } => {
                self.group_members(channel_id).get(view.selected).cloned()
            }
            ConversationMode::Group { .. } => None,
        }
    }

    pub fn conversation_selected_action(&self) -> Option<GroupAction> {
        let view = self.conversation.as_ref()?;
        match &view.mode {
            ConversationMode::Group { channel_id } => {
                self.group_actions(channel_id).get(view.selected).copied()
            }
            _ => None,
        }
    }

    pub fn community_selected_action(&self) -> Option<CommunityAction> {
        let view = self.community.as_ref()?;
        match &view.mode {
            CommunityMode::Menu => self.community_actions().get(view.selected).copied(),
            _ => None,
        }
    }

    /// Tick or untick the person under the cursor. Ticking anybody is
    /// what turns Enter from "open a conversation" into "make a group".
    pub fn conversation_toggle_mark(&mut self) -> Option<usize> {
        let user = self.conversation_selected_user()?;
        let view = self.conversation.as_mut()?;
        if !matches!(view.mode, ConversationMode::People) {
            return None;
        }
        match view.marked.iter().position(|id| *id == user.id) {
            Some(index) => {
                view.marked.remove(index);
            }
            None => view.marked.push(user.id),
        }
        Some(view.marked.len())
    }

    pub fn conversation_is_marked(&self, user_id: &str) -> bool {
        self.conversation
            .as_ref()
            .is_some_and(|v| v.marked.iter().any(|id| id == user_id))
    }

    pub fn conversation_marked(&self) -> Vec<String> {
        self.conversation
            .as_ref()
            .map(|v| v.marked.clone())
            .unwrap_or_default()
    }

    pub fn conversation_filter_push(&mut self, c: char) {
        if let Some(view) = &mut self.conversation
            && matches!(view.mode, ConversationMode::People)
            && view.filter.chars().count() < 64
        {
            view.filter.push(c);
            view.selected = 0;
        }
    }

    /// Step back out of a list the menu led to, or close it when the
    /// actions themselves are showing.
    pub fn message_actions_back(&mut self) {
        let Some(view) = &mut self.message_actions else {
            return;
        };
        if matches!(view.mode, MessageActionsMode::Actions) {
            self.message_actions = None;
        } else {
            view.mode = MessageActionsMode::Actions;
            view.selected = 0;
        }
    }

    pub fn conversation_filter_pop(&mut self) {
        if let Some(view) = &mut self.conversation {
            view.filter.pop();
            view.selected = 0;
        }
    }

    fn clamp_friends_selection(&mut self) {
        let Some(tab) = self.friends.as_ref().map(|v| v.tab) else {
            return;
        };
        let count = self.relationships_in(tab).len();
        if let Some(view) = &mut self.friends {
            view.selected = view.selected.min(count.saturating_sub(1));
        }
    }
    /// Open a channel the client already knows, the way the channel
    /// picker does.
    pub fn jump_to_channel(&mut self, channel_id: &str) -> bool {
        let Some(server) = self.server_for_channel(channel_id) else {
            self.set_status("That channel is not on your list.");
            return false;
        };
        self.selected_server = server;
        self.selected_channel_id = Some(channel_id.to_string());
        self.message_scroll_from_bottom = 0;
        self.selected_message_index = None;
        self.normalize_selection();
        self.focus = Focus::Messages;
        true
    }

    /// Turn the row under the cursor into something to do. The menu is
    /// left open where the row leads to a list, and closed where it does
    /// not; the caller runs what comes back.
    pub fn message_actions_confirm(&mut self) -> Option<MessageActionOutcome> {
        let view = self.message_actions.as_ref()?;
        let channel_id = view.channel_id.clone();
        let message_id = view.message_id.clone();
        match &view.mode {
            MessageActionsMode::Confirm(action) => {
                let action = *action;
                let go = view.selected == 0;
                self.message_actions = None;
                if go {
                    Some(MessageActionOutcome::Run {
                        action,
                        channel_id,
                        message_id,
                        argument: None,
                    })
                } else {
                    None
                }
            }
            MessageActionsMode::ReportCategories => {
                let category = REPORT_CATEGORIES.get(view.selected)?.0.to_string();
                self.message_actions = None;
                Some(MessageActionOutcome::Run {
                    action: MessageAction::Report,
                    channel_id,
                    message_id,
                    argument: Some(category),
                })
            }
            MessageActionsMode::Attachments(items) => {
                let attachment_id = items.get(view.selected)?.0.clone();
                self.message_actions = None;
                Some(MessageActionOutcome::Run {
                    action: MessageAction::RemoveAttachment,
                    channel_id,
                    message_id,
                    argument: Some(attachment_id),
                })
            }
            MessageActionsMode::Actions => {
                let action = *view.actions.get(view.selected)?;
                if action.needs_confirm() {
                    if let Some(view) = &mut self.message_actions {
                        view.mode = MessageActionsMode::Confirm(action);
                        view.selected = 1;
                    }
                    return None;
                }
                if action == MessageAction::Report {
                    if let Some(view) = &mut self.message_actions {
                        view.mode = MessageActionsMode::ReportCategories;
                        view.selected = 0;
                    }
                    return None;
                }
                if action == MessageAction::RemoveAttachment {
                    let msg = self.message_by_id(&channel_id, &message_id)?;
                    let items: Vec<(String, String)> = msg
                        .attachments
                        .iter()
                        .map(|a| {
                            (
                                a.id.clone(),
                                if a.filename.is_empty() {
                                    a.id.clone()
                                } else {
                                    a.filename.clone()
                                },
                            )
                        })
                        .collect();
                    if items.len() == 1 {
                        let attachment_id = items[0].0.clone();
                        self.message_actions = None;
                        return Some(MessageActionOutcome::Run {
                            action,
                            channel_id,
                            message_id,
                            argument: Some(attachment_id),
                        });
                    }
                    if let Some(view) = &mut self.message_actions {
                        view.mode = MessageActionsMode::Attachments(items);
                        view.selected = 0;
                    }
                    return None;
                }
                self.message_actions = None;
                Some(MessageActionOutcome::Run {
                    action,
                    channel_id,
                    message_id,
                    argument: None,
                })
            }
        }
    }

    /// One message of a channel that is loaded, by id.
    pub fn message_by_id(&self, channel_id: &str, message_id: &str) -> Option<MessageResponse> {
        self.messages
            .get(channel_id)?
            .iter()
            .find(|m| m.id == message_id)
            .cloned()
    }

    /// Shut every overlay, so opening one never leaves another under it.
    ///
    /// The draw chain and the key handler are both an if/else over these,
    /// so a stale one would shadow whatever was just opened. Every
    /// `open_*` calls this first, which is why the list has to name them
    /// all.
    pub fn close_overlays(&mut self) {
        self.show_settings = false;
        self.show_server_notifications = false;
        self.show_help = false;
        self.dismiss_image_preview();
        self.profile = None;
        self.channel_picker = None;
        self.pings = None;
        self.message_actions = None;
        self.channel_admin = None;
        self.pins = None;
        self.saved = None;
        self.reaction_users = None;
        self.friends = None;
        self.conversation = None;
        self.pending_group_add = None;
        self.community = None;
        self.search = None;
        self.voice_menu = None;
    }

    // Alt+V: voice

    pub fn open_voice_menu(&mut self) {
        self.close_overlays();
        self.voice_menu = Some(VoiceView { selected: 0 });
    }

    pub fn dismiss_voice_menu(&mut self) {
        self.voice_menu = None;
    }

    // Alt+P: the channel's pinned messages

    pub fn open_pins(&mut self, channel_id: String) {
        self.close_overlays();
        self.channels_with_new_pins.remove(&channel_id);
        self.pins = Some(PinsView {
            channel_id,
            state: PinsState::Loading,
            selected: 0,
        });
    }

    pub fn dismiss_pins(&mut self) {
        self.pins = None;
    }

    pub fn set_pins_loaded(&mut self, channel_id: &str, items: Vec<ChannelPinResponse>) {
        for pin in &items {
            self.merge_message_embedded_members(&pin.message);
            merge_user_cache(&mut self.user_cache, [pin.message.author.clone()]);
        }
        if let Some(view) = &mut self.pins
            && view.channel_id == channel_id
        {
            view.state = PinsState::Ready(items);
            view.selected = 0;
        }
    }

    pub fn set_pins_failed(&mut self, channel_id: &str, message: String) {
        if let Some(view) = &mut self.pins
            && view.channel_id == channel_id
        {
            view.state = PinsState::Failed(message);
        }
    }

    pub fn pins_items(&self) -> Vec<ChannelPinResponse> {
        match self.pins.as_ref().map(|v| &v.state) {
            Some(PinsState::Ready(items)) => items.clone(),
            _ => Vec::new(),
        }
    }

    pub fn pins_move(&mut self, delta: isize) {
        let count = self.pins_items().len();
        if let Some(view) = &mut self.pins {
            view.selected = if count == 0 {
                0
            } else {
                (view.selected as isize + delta).clamp(0, count as isize - 1) as usize
            };
        }
    }

    pub fn pins_selected(&self) -> Option<MessageResponse> {
        let view = self.pins.as_ref()?;
        let PinsState::Ready(items) = &view.state else {
            return None;
        };
        items.get(view.selected).map(|p| p.message.clone())
    }

    // Alt+B: the messages bookmarked from anywhere

    pub fn open_saved(&mut self) {
        self.close_overlays();
        self.saved = Some(SavedView {
            state: SavedState::Loading,
            selected: 0,
        });
    }

    pub fn dismiss_saved(&mut self) {
        self.saved = None;
    }

    pub fn set_saved_loaded(&mut self, entries: Vec<SavedMessageEntryResponse>) {
        self.saved_message_ids = entries.iter().map(|e| e.message_id.clone()).collect();
        for entry in &entries {
            if let Some(message) = &entry.message {
                self.merge_message_embedded_members(message);
                merge_user_cache(&mut self.user_cache, [message.author.clone()]);
            }
        }
        if let Some(view) = &mut self.saved {
            view.state = SavedState::Ready(entries);
            view.selected = 0;
        }
    }

    pub fn set_saved_failed(&mut self, message: String) {
        if let Some(view) = &mut self.saved {
            view.state = SavedState::Failed(message);
        }
    }

    pub fn saved_entries(&self) -> Vec<SavedMessageEntryResponse> {
        match self.saved.as_ref().map(|v| &v.state) {
            Some(SavedState::Ready(entries)) => entries.clone(),
            _ => Vec::new(),
        }
    }

    pub fn saved_move(&mut self, delta: isize) {
        let count = self.saved_entries().len();
        if let Some(view) = &mut self.saved {
            view.selected = if count == 0 {
                0
            } else {
                (view.selected as isize + delta).clamp(0, count as isize - 1) as usize
            };
        }
    }

    pub fn saved_selected(&self) -> Option<SavedMessageEntryResponse> {
        let view = self.saved.as_ref()?;
        let SavedState::Ready(entries) = &view.state else {
            return None;
        };
        entries.get(view.selected).cloned()
    }

    /// Take an entry out of the open list once the server has dropped it,
    /// so the list does not have to be fetched again.
    pub fn forget_saved_message(&mut self, message_id: &str) {
        self.saved_message_ids.remove(message_id);
        if let Some(view) = &mut self.saved
            && let SavedState::Ready(entries) = &mut view.state
        {
            entries.retain(|e| e.message_id != message_id);
            if view.selected >= entries.len() {
                view.selected = entries.len().saturating_sub(1);
            }
        }
    }

    pub fn remember_saved_message(&mut self, message_id: String) {
        self.saved_message_ids.insert(message_id);
    }

    // v: who reacted

    /// Open the who-reacted list on one of a message's reactions. The
    /// index walks the message's own reaction order.
    pub fn open_reaction_users(
        &mut self,
        message_id: &str,
        reaction_index: usize,
    ) -> Option<(String, String)> {
        let channel_id = self.active_channel_id()?;
        let msg = self.message_by_id(&channel_id, message_id)?;
        let reaction = msg.reactions.get(reaction_index)?;
        let (api, label) = reaction_emoji_forms(reaction);
        self.close_overlays();
        self.reaction_users = Some(ReactionUsersView {
            channel_id: channel_id.clone(),
            message_id: message_id.to_string(),
            emoji_api: api.clone(),
            emoji_label: label,
            reaction_index,
            state: ReactionUsersState::Loading,
            scroll: 0,
        });
        Some((channel_id, api))
    }

    /// Left and Right in the who-reacted list: the message's next or
    /// previous reaction, wrapping, with the fetch to make for it.
    pub fn reaction_users_step(&mut self, delta: isize) -> Option<(String, String, String)> {
        let view = self.reaction_users.as_ref()?;
        let channel_id = view.channel_id.clone();
        let message_id = view.message_id.clone();
        let msg = self.message_by_id(&channel_id, &message_id)?;
        let count = msg.reactions.len();
        if count < 2 {
            return None;
        }
        let index = view.reaction_index as isize + delta;
        let index = index.rem_euclid(count as isize) as usize;
        let reaction = msg.reactions.get(index)?;
        let (api, label) = reaction_emoji_forms(reaction);
        let view = self.reaction_users.as_mut()?;
        view.reaction_index = index;
        view.emoji_api = api.clone();
        view.emoji_label = label;
        view.state = ReactionUsersState::Loading;
        view.scroll = 0;
        Some((channel_id, message_id, api))
    }

    pub fn dismiss_reaction_users(&mut self) {
        self.reaction_users = None;
    }

    pub fn set_reaction_users_loaded(&mut self, emoji: &str, users: Vec<UserPartialResponse>) {
        merge_user_cache(&mut self.user_cache, users.iter().cloned());
        if let Some(view) = &mut self.reaction_users
            && view.emoji_api == emoji
        {
            view.state = ReactionUsersState::Ready(users);
        }
    }

    pub fn set_reaction_users_failed(&mut self, emoji: &str, message: String) {
        if let Some(view) = &mut self.reaction_users
            && view.emoji_api == emoji
        {
            view.state = ReactionUsersState::Failed(message);
        }
    }

    /// Jump the message pane to a message that is in a channel the client
    /// knows, the way the pings list does. False when the channel is gone.
    /// Jump the message pane to a message that is in a channel the client
    /// knows, the way the pings list does. False when the channel is gone
    /// — a search can turn up a hit in a community the reader has since
    /// left, and saying so beats moving the pane somewhere wrong.
    pub fn jump_to_message(&mut self, channel_id: &str, message_id: &str) -> bool {
        let Some(server) = self.server_for_channel(channel_id) else {
            self.set_status("That channel is not on your list any more.");
            return false;
        };
        self.close_overlays();
        self.selected_server = server;
        self.selected_channel_id = Some(channel_id.to_string());
        self.message_scroll_from_bottom = 0;
        self.selected_message_index = None;
        self.normalize_selection();
        self.focus = Focus::Messages;
        self.pending_jump = Some((channel_id.to_string(), message_id.to_string()));
        self.pending_jump_pages = 0;
        self.apply_pending_jump(channel_id);
        true
    }

    /// Open a one-to-one conversation with somebody the reader already
    /// has one with. None when there is none to open yet.
    pub fn dm_channel_with(&self, user_id: &str) -> Option<String> {
        self.private_channels
            .iter()
            .find(|c| {
                c.channel_type() == CHANNEL_DM && c.recipients.iter().any(|u| u.id == user_id)
            })
            .map(|c| c.id.clone())
    }
    /// Set or clear a message's suppress-embeds flag in the loaded copy,
    /// so the pane follows before the gateway says so.
    pub fn set_local_message_flags(&mut self, channel_id: &str, message_id: &str, flags: u64) {
        if let Some(messages) = self.messages.get_mut(channel_id)
            && let Some(msg) = std::rc::Rc::make_mut(messages)
                .iter_mut()
                .find(|m| m.id == message_id)
        {
            msg.flags = flags;
            if flags & MESSAGE_FLAG_SUPPRESS_EMBEDS != 0 {
                msg.embeds.clear();
            }
        }
        self.messages_version = self.messages_version.wrapping_add(1);
    }

    /// Set or clear a message's pinned mark in the loaded copy.
    pub fn set_local_message_pinned(&mut self, channel_id: &str, message_id: &str, pinned: bool) {
        if let Some(messages) = self.messages.get_mut(channel_id)
            && let Some(msg) = std::rc::Rc::make_mut(messages)
                .iter_mut()
                .find(|m| m.id == message_id)
        {
            msg.pinned = pinned;
        }
        self.messages_version = self.messages_version.wrapping_add(1);
    }
    // / : search

    pub fn open_search(&mut self) {
        self.show_settings = false;
        self.show_server_notifications = false;
        self.show_help = false;
        self.dismiss_image_preview();
        self.profile = None;
        self.channel_picker = None;
        self.pings = None;
        // a search opens on the narrowest scope that makes sense here:
        // the channel when there is one, the community otherwise
        let scope = if self.active_channel_id().is_some() {
            crate::search::SearchScope::Channel
        } else if self.active_guild_id().is_some() {
            crate::search::SearchScope::Guild
        } else {
            crate::search::SearchScope::Everything
        };
        self.search = Some(SearchView {
            query: String::new(),
            scope,
            state: SearchState::Idle,
            selected: 0,
            page: 1,
            editing: true,
        });
    }

    pub fn dismiss_search(&mut self) {
        self.search = None;
    }

    /// What to send for the query as it stands, with the `from:` names
    /// turned into ids. `Err` names somebody the client cannot place, so
    /// the reader is told rather than searched for the wrong thing.
    pub fn build_search_request(
        &self,
        page: u32,
    ) -> Option<Result<crate::api::types::MessageSearchRequest, String>> {
        let view = self.search.as_ref()?;
        let parsed = crate::search::parse(&view.query);
        if !crate::search::is_searchable(&parsed) {
            return None;
        }
        let mut author_ids = Vec::new();
        for name in &parsed.authors {
            match self.user_id_for_name(name) {
                Some(id) => author_ids.push(id),
                None => return Some(Err(format!("Nobody here is called \"{name}\"."))),
            }
        }
        Some(Ok(crate::search::build_request(
            &parsed,
            author_ids,
            view.scope,
            self.active_channel_id(),
            self.active_guild_id(),
            page,
        )))
    }

    /// Find somebody by what the reader typed after `from:`: their
    /// nickname here, their display name, or their username, whichever
    /// matches first. A tag with a `#` is matched exactly.
    pub fn user_id_for_name(&self, name: &str) -> Option<String> {
        let needle = name.trim().trim_start_matches('@').to_lowercase();
        if needle.is_empty() {
            return None;
        }
        if let Some((username, discriminator)) = needle.rsplit_once('#') {
            return self
                .user_cache
                .values()
                .find(|u| u.username.to_lowercase() == username && u.discriminator == discriminator)
                .map(|u| u.id.clone());
        }
        let guild_id = self.active_guild_id();
        // a nickname in this community first, since that is the name on
        // the screen, then the account's own
        if let Some(guild_id) = &guild_id
            && let Some(members) = self.guild_members.get(guild_id)
            && let Some(member) = members.iter().find(|m| {
                m.nick
                    .as_deref()
                    .is_some_and(|n| n.to_lowercase() == needle)
            })
        {
            return Some(member.user.id.clone());
        }
        self.user_cache
            .values()
            .find(|u| {
                u.global_name
                    .as_deref()
                    .is_some_and(|n| n.to_lowercase() == needle)
                    || u.username.to_lowercase() == needle
            })
            .map(|u| u.id.clone())
    }

    pub fn set_search_running(&mut self, page: u32) {
        if let Some(view) = &mut self.search {
            view.state = SearchState::Running;
            view.page = page;
            view.selected = 0;
            view.editing = false;
        }
    }

    pub fn set_search_results(
        &mut self,
        messages: Vec<MessageResponse>,
        channels: Vec<ChannelResponse>,
        total: u32,
        page: u32,
        hits_per_page: u32,
    ) {
        for message in &messages {
            self.merge_message_embedded_members(message);
            merge_user_cache(&mut self.user_cache, [message.author.clone()]);
        }
        // the answer carries the channels its hits are in, which is how a
        // result from a community the reader has not opened can still say
        // where it came from
        for channel in channels {
            if !channel.id.is_empty() && self.channel_by_id(&channel.id).is_none() {
                self.search_channels.insert(channel.id.clone(), channel);
            }
        }
        if let Some(view) = &mut self.search {
            view.state = SearchState::Ready {
                messages,
                total,
                page,
                hits_per_page,
            };
            view.selected = 0;
            view.editing = false;
        }
    }

    pub fn set_search_indexing(&mut self) {
        if let Some(view) = &mut self.search {
            view.state = SearchState::Indexing;
        }
    }

    pub fn set_search_failed(&mut self, message: String) {
        if let Some(view) = &mut self.search {
            view.state = SearchState::Failed(message);
        }
    }

    pub fn search_results(&self) -> Vec<MessageResponse> {
        match self.search.as_ref().map(|v| &v.state) {
            Some(SearchState::Ready { messages, .. }) => messages.clone(),
            _ => Vec::new(),
        }
    }

    pub fn search_move(&mut self, delta: isize) {
        let count = self.search_results().len();
        if let Some(view) = &mut self.search {
            if count == 0 {
                view.selected = 0;
                return;
            }
            view.editing = false;
            view.selected = (view.selected as isize + delta).clamp(0, count as isize - 1) as usize;
        }
    }

    pub fn search_selected(&self) -> Option<MessageResponse> {
        let view = self.search.as_ref()?;
        self.search_results().get(view.selected).cloned()
    }

    /// How many pages the answer says there are, so the overlay can say
    /// whether there is another one.
    pub fn search_pages(&self) -> (u32, u32) {
        match self.search.as_ref().map(|v| &v.state) {
            Some(SearchState::Ready {
                total,
                page,
                hits_per_page,
                ..
            }) => {
                let per = (*hits_per_page).max(1);
                (*page, total.div_ceil(per).max(1))
            }
            _ => (1, 1),
        }
    }

    /// Where a search hit came from, including channels the reader has
    /// not opened, whose objects came back with the answer.
    pub fn search_hit_location(&self, channel_id: &str) -> (Option<String>, String) {
        if self.channel_by_id(channel_id).is_some() {
            return self.channel_location(channel_id);
        }
        match self.search_channels.get(channel_id) {
            Some(channel) => {
                let guild = channel
                    .guild_id
                    .as_ref()
                    .and_then(|gid| self.guilds.iter().find(|g| &g.id == gid))
                    .map(|g| g.name.clone());
                (guild, crate::ui::sidebar::channel_name(self, channel))
            }
            None => (
                None,
                format!(
                    "unknown-{}",
                    &channel_id[channel_id.len().saturating_sub(4)..]
                ),
            ),
        }
    }
    /// Take a channel the server just made and put it on the list, so the
    /// reader can be moved into it at once.
    pub fn adopt_private_channel(&mut self, channel: ChannelResponse) {
        if channel.id.is_empty() {
            return;
        }
        merge_user_cache(&mut self.user_cache, channel.recipients.iter().cloned());
        match self
            .private_channels
            .iter_mut()
            .find(|c| c.id == channel.id)
        {
            Some(existing) => *existing = channel,
            None => self.private_channels.push(channel),
        }
    }

    pub fn is_dm_pinned(&self, channel_id: &str) -> bool {
        self.pinned_dms.contains(channel_id)
    }

    pub fn set_pinned_dms(&mut self, ids: Vec<String>) {
        self.pinned_dms = ids.into_iter().collect();
    }

    pub fn set_dm_pinned_local(&mut self, channel_id: &str, pinned: bool) {
        if pinned {
            self.pinned_dms.insert(channel_id.to_string());
        } else {
            self.pinned_dms.remove(channel_id);
        }
    }

    /// Add or drop somebody in a group the client already holds, from
    /// CHANNEL_RECIPIENT_ADD and _REMOVE.
    pub fn set_group_recipient(
        &mut self,
        channel_id: &str,
        user: UserPartialResponse,
        present: bool,
    ) {
        merge_user_cache(&mut self.user_cache, [user.clone()]);
        if let Some(channel) = self
            .private_channels
            .iter_mut()
            .find(|c| c.id == channel_id)
        {
            channel.recipients.retain(|u| u.id != user.id);
            if present {
                channel.recipients.push(user);
            }
        }
    }
    pub fn community_selected_guild(&self) -> Option<DiscoveryGuildResponse> {
        let view = self.community.as_ref()?;
        match &view.mode {
            CommunityMode::Discover {
                state: DiscoverState::Ready(guilds),
                ..
            } => guilds.get(view.selected).cloned(),
            _ => None,
        }
    }

    pub fn community_selected_invite(&self) -> Option<InviteResponse> {
        let view = self.community.as_ref()?;
        match &view.mode {
            CommunityMode::Invites {
                state: InvitesState::Ready(invites),
                ..
            } => invites.get(view.selected).cloned(),
            _ => None,
        }
    }

    /// Step back out of a list the menu led to, or close it.
    pub fn community_back(&mut self) {
        let Some(view) = &mut self.community else {
            return;
        };
        if view.input.is_some() {
            view.input = None;
            return;
        }
        if matches!(view.mode, CommunityMode::Menu) {
            self.community = None;
        } else {
            view.mode = CommunityMode::Menu;
            view.selected = 0;
        }
    }

    pub fn set_discover_running(&mut self, query: String) {
        if let Some(view) = &mut self.community {
            view.mode = CommunityMode::Discover {
                query,
                state: DiscoverState::Running,
            };
            view.selected = 0;
            view.input = None;
        }
    }

    /// How many the directory said there were in all, which can be more
    /// than the page that came back.
    pub fn discover_total(&self) -> u32 {
        self.discover_total
    }

    pub fn set_discover_results(&mut self, guilds: Vec<DiscoveryGuildResponse>) {
        if let Some(view) = &mut self.community
            && let CommunityMode::Discover { state, .. } = &mut view.mode
        {
            *state = DiscoverState::Ready(guilds);
            view.selected = 0;
        }
    }

    pub fn set_discover_failed(&mut self, message: String) {
        if let Some(view) = &mut self.community
            && let CommunityMode::Discover { state, .. } = &mut view.mode
        {
            *state = DiscoverState::Failed(message);
        }
    }

    pub fn open_guild_invites(&mut self, guild_id: String) {
        if let Some(view) = &mut self.community {
            view.mode = CommunityMode::Invites {
                guild_id,
                state: InvitesState::Loading,
            };
            view.selected = 0;
        }
    }

    pub fn set_guild_invites(&mut self, for_guild: &str, invites: Vec<InviteResponse>) {
        if let Some(view) = &mut self.community
            && let CommunityMode::Invites { guild_id, state } = &mut view.mode
            && guild_id == for_guild
        {
            *state = InvitesState::Ready(invites);
            view.selected = 0;
        }
    }

    pub fn set_guild_invites_failed(&mut self, for_guild: &str, message: String) {
        if let Some(view) = &mut self.community
            && let CommunityMode::Invites { guild_id, state } = &mut view.mode
            && guild_id == for_guild
        {
            *state = InvitesState::Failed(message);
        }
    }

    pub fn open_invite_preview(&mut self, code: String) {
        if let Some(view) = &mut self.community {
            view.mode = CommunityMode::Preview {
                code,
                state: PreviewState::Loading,
            };
            view.selected = 0;
            view.input = None;
        }
    }

    pub fn set_invite_preview(&mut self, for_code: &str, invite: InviteResponse) {
        if let Some(view) = &mut self.community
            && let CommunityMode::Preview { code, state } = &mut view.mode
            && code == for_code
        {
            *state = PreviewState::Ready(Box::new(invite));
        }
    }

    pub fn set_invite_preview_failed(&mut self, for_code: &str, message: String) {
        if let Some(view) = &mut self.community
            && let CommunityMode::Preview { code, state } = &mut view.mode
            && code == for_code
        {
            *state = PreviewState::Failed(message);
        }
    }

    /// The invite the preview is showing, once it has arrived.
    pub fn previewed_invite(&self) -> Option<InviteResponse> {
        match self.community.as_ref().map(|v| &v.mode) {
            Some(CommunityMode::Preview {
                state: PreviewState::Ready(invite),
                ..
            }) => Some((**invite).clone()),
            _ => None,
        }
    }

    /// Whether the community now open belongs to the reader, since an
    /// owner cannot leave their own.
    pub fn owns_active_guild(&self) -> bool {
        self.active_guild_id()
            .and_then(|id| self.guilds.iter().find(|g| g.id == id))
            .is_some_and(|g| g.owner_id == self.me.id)
    }

    /// A whole invite link, for putting on the clipboard.
    pub fn invite_link(&self, code: &str) -> String {
        let base = self.discovery.endpoints.webapp.trim_end_matches('/');
        if base.is_empty() {
            code.to_string()
        } else {
            format!("{base}/invite/{code}")
        }
    }

    /// Whether a conversation is ringing for the reader.
    pub fn channel_is_ringing(&self, channel_id: &str) -> bool {
        self.incoming_calls
            .get(channel_id)
            .is_some_and(|call| call.ringing.contains(&self.me.id))
    }

    /// Any conversation ringing for the reader, whether or not it is the
    /// one they are looking at.
    pub fn first_ringing_channel(&self) -> Option<String> {
        self.incoming_calls
            .values()
            .find(|call| call.ringing.contains(&self.me.id))
            .map(|call| call.channel_id.clone())
    }

    /// What the voice menu offers here. The rows depend on whether the
    /// reader is in a call, whether one is ringing, and what kind of
    /// channel is open.
    pub fn voice_actions(&self) -> Vec<VoiceAction> {
        let mut out = Vec::new();
        if self.first_ringing_channel().is_some() {
            out.push(VoiceAction::Answer);
            out.push(VoiceAction::Decline);
        }
        match &self.voice {
            Some(connection) => {
                out.push(if connection.self_mute {
                    VoiceAction::Unmute
                } else {
                    VoiceAction::Mute
                });
                out.push(if connection.self_deaf {
                    VoiceAction::Undeafen
                } else {
                    VoiceAction::Deafen
                });
                if connection.grant.is_some() {
                    out.push(VoiceAction::CopyGrant);
                }
                out.push(VoiceAction::Leave);
            }
            None => {
                if self.active_channel_is_voice() {
                    out.push(VoiceAction::Join);
                } else if self.active_channel_is_private_conversation() {
                    out.push(VoiceAction::StartCall);
                }
            }
        }
        out
    }

    /// Whether the channel now open is a one-to-one or a group, which is
    /// where a call can be started.
    pub fn active_channel_is_private_conversation(&self) -> bool {
        self.active_channel_id()
            .and_then(|id| self.channel_by_id(&id).cloned())
            .is_some_and(|c| matches!(c.channel_type(), CHANNEL_DM | CHANNEL_GROUP_DM))
    }

    pub fn voice_menu_len(&self) -> usize {
        self.voice_actions().len()
    }

    pub fn voice_menu_move(&mut self, delta: isize) {
        let count = self.voice_menu_len();
        if let Some(view) = &mut self.voice_menu {
            view.selected = if count == 0 {
                0
            } else {
                (view.selected as isize + delta).clamp(0, count as isize - 1) as usize
            };
        }
    }

    pub fn voice_selected_action(&self) -> Option<VoiceAction> {
        let view = self.voice_menu.as_ref()?;
        self.voice_actions().get(view.selected).copied()
    }

    /// Remember that the client asked to be in a channel, before the
    /// server has said anything back.
    pub fn set_voice_joining(&mut self, channel_id: String, guild_id: Option<String>) {
        self.voice = Some(VoiceConnection {
            channel_id,
            guild_id,
            connection_id: None,
            self_mute: false,
            self_deaf: false,
            grant: None,
            media_running: false,
        });
    }

    pub fn clear_voice(&mut self) {
        self.voice = None;
    }

    /// Take VOICE_SERVER_UPDATE: the grant for the connection.
    pub fn set_voice_grant(&mut self, event: crate::api::types::VoiceServerUpdateEvent) {
        let Some(connection) = &mut self.voice else {
            return;
        };
        if !event.channel_id.is_empty() && connection.channel_id != event.channel_id {
            return;
        }
        connection.connection_id = Some(event.connection_id);
        // the grant says the scope outright — a community's channel has
        // a guild and a call has none — so it settles what the client
        // guessed when it asked to join
        connection.guild_id = event.guild_id;
        connection.grant = Some(VoiceGrant {
            endpoint: event.endpoint,
            token: event.token,
            e2ee_key: event.e2ee_key,
        });
        // a fresh grant means a fresh connection to carry
        connection.media_running = false;
    }

    pub fn set_voice_media_running(&mut self, running: bool) {
        if let Some(connection) = &mut self.voice {
            connection.media_running = running;
        }
    }

    /// The grant to hand a media program, when there is one waiting to be
    /// carried.
    pub fn voice_grant_to_start(&self) -> Option<(String, VoiceGrant)> {
        let connection = self.voice.as_ref()?;
        if connection.media_running {
            return None;
        }
        let grant = connection.grant.clone()?;
        Some((connection.channel_id.clone(), grant))
    }

    pub fn set_voice_flags(&mut self, self_mute: bool, self_deaf: bool) {
        if let Some(connection) = &mut self.voice {
            connection.self_mute = self_mute;
            // deafening implies not hearing, and the web client mutes
            // with it, so the two move together in that direction
            connection.self_deaf = self_deaf;
            if self_deaf {
                connection.self_mute = true;
            }
        }
    }

    pub fn upsert_incoming_call(&mut self, channel_id: String, ringing: Vec<String>) {
        if ringing.is_empty() {
            self.incoming_calls.remove(&channel_id);
            return;
        }
        self.incoming_calls.insert(
            channel_id.clone(),
            IncomingCall {
                channel_id,
                ringing,
            },
        );
    }

    pub fn clear_incoming_call(&mut self, channel_id: &str) {
        self.incoming_calls.remove(channel_id);
    }

    /// A line for the status bar while a call is on or ringing.
    pub fn voice_status_line(&self) -> Option<String> {
        if let Some(channel_id) = self.first_ringing_channel() {
            let (_, name) = self.channel_location(&channel_id);
            return Some(format!("{name} is ringing (Alt+V)"));
        }
        let connection = self.voice.as_ref()?;
        let (_, name) = self.channel_location(&connection.channel_id);
        let mut what = String::new();
        if connection.self_deaf {
            what.push_str(" deaf");
        } else if connection.self_mute {
            what.push_str(" muted");
        }
        if connection.grant.is_none() {
            what.push_str(" connecting");
        }
        Some(format!("in {name}{what}"))
    }
    // Getting about: slots, history, the last community

    /// How many channels back Alt+Left can walk.
    pub const CHANNEL_HISTORY_MAX: usize = 50;

    /// How long a one-off notice — "sent", "joined", "pinned" — stays
    /// before the key hints come back.
    pub const NOTICE_LIFETIME: Duration = Duration::from_secs(4);

    /// Notice that the reader has moved, and keep the three things that
    /// depend on it: the visited-channel history, the last community,
    /// and where the "new messages" line goes.
    ///
    /// Called once a frame rather than at every place that moves the
    /// reader, of which there are a dozen and counting.
    pub fn note_active_channel(&mut self) {
        let now = self.active_channel_id();
        // the flag is consumed here whatever happens next: a step that
        // landed where the reader already was is still a step, and
        // leaving it set would stop the next real visit being recorded
        let walking = std::mem::take(&mut self.walking_history);
        if now == self.last_active_channel {
            return;
        }
        self.last_active_channel = now.clone();
        if let ServerSelection::Guild(guild_id) = &self.selected_server {
            self.last_guild = Some(guild_id.clone());
        }
        let Some(channel_id) = now else {
            return;
        };
        self.set_unread_anchor(&channel_id);
        if walking {
            // a step through the history is not a new place to go back to
            return;
        }
        let entry = (self.selected_server.clone(), channel_id);
        // going somewhere new throws away whatever was ahead
        self.channel_history.truncate(self.channel_history_pos);
        if self.channel_history.last() == Some(&entry) {
            return;
        }
        self.channel_history.push(entry);
        if self.channel_history.len() > Self::CHANNEL_HISTORY_MAX {
            let drop = self.channel_history.len() - Self::CHANNEL_HISTORY_MAX;
            self.channel_history.drain(0..drop);
        }
        self.channel_history_pos = self.channel_history.len();
    }

    /// Alt+Left and Alt+Right. `back` walks towards the older entries.
    /// False when there is nowhere to go that way.
    pub fn step_channel_history(&mut self, back: bool) -> bool {
        let target = if back {
            if self.channel_history_pos <= 1 {
                return false;
            }
            self.channel_history_pos - 2
        } else {
            if self.channel_history_pos >= self.channel_history.len() {
                return false;
            }
            self.channel_history_pos
        };
        let Some((server, channel_id)) = self.channel_history.get(target).cloned() else {
            return false;
        };
        // a channel that has gone since it was visited is skipped rather
        // than moved to; the entry stays, in case it comes back
        if self.server_for_channel(&channel_id).is_none() {
            self.set_status("That channel is not there any more.");
            return false;
        }
        self.walking_history = true;
        self.channel_history_pos = target + 1;
        self.selected_server = server;
        self.selected_channel_id = Some(channel_id);
        self.message_scroll_from_bottom = 0;
        self.selected_message_index = None;
        self.normalize_selection();
        self.focus = Focus::Messages;
        true
    }

    pub fn can_step_history(&self, back: bool) -> bool {
        if back {
            self.channel_history_pos > 1
        } else {
            self.channel_history_pos < self.channel_history.len()
        }
    }

    /// Alt+1 to Alt+9: slot 1 is the conversation list and slots 2 to 9
    /// are the first eight communities, the way the web client numbers
    /// them.
    pub fn go_to_server_slot(&mut self, slot: usize) -> bool {
        let entries = self.server_entries();
        let Some(server) = slot.checked_sub(1).and_then(|i| entries.get(i)).cloned() else {
            return false;
        };
        self.select_server(server);
        true
    }

    /// Alt+L: back and forth between the community last read and the
    /// conversation list.
    pub fn toggle_guild_and_dms(&mut self) -> bool {
        match &self.selected_server {
            ServerSelection::DirectMessages => {
                let Some(guild_id) = self.last_guild.clone() else {
                    self.set_status("No community to go back to yet.");
                    return false;
                };
                if !self.guilds.iter().any(|g| g.id == guild_id) {
                    self.last_guild = None;
                    self.set_status("That community is not on your list any more.");
                    return false;
                }
                self.select_server(ServerSelection::Guild(guild_id));
                true
            }
            ServerSelection::Guild(_) => {
                self.select_server(ServerSelection::DirectMessages);
                true
            }
        }
    }

    /// Move to a community, or to the conversation list, and open
    /// whatever channel was last read there.
    pub fn select_server(&mut self, server: ServerSelection) {
        if self.selected_server == server {
            return;
        }
        self.selected_server = server;
        self.selected_channel_id = None;
        self.message_scroll_from_bottom = 0;
        self.selected_message_index = None;
        self.normalize_selection();
        self.focus = Focus::Messages;
    }

    /// Step through the server column from anywhere, so the reader does
    /// not have to put the focus on it first.
    pub fn step_server(&mut self, delta: isize) {
        let entries = self.server_entries();
        if entries.len() < 2 {
            return;
        }
        let here = entries
            .iter()
            .position(|s| *s == self.selected_server)
            .unwrap_or(0) as isize;
        let next = (here + delta).rem_euclid(entries.len() as isize) as usize;
        if let Some(server) = entries.get(next).cloned() {
            self.select_server(server);
        }
    }

    // The "new messages" line

    /// Fix where the line goes for a channel the reader has just opened:
    /// after the last message they had read. Nothing unread means no
    /// line, and a channel never opened before gets none either, since a
    /// line above the whole history says nothing.
    pub fn set_unread_anchor(&mut self, channel_id: &str) {
        let last_read = self
            .read_states
            .get(channel_id)
            .and_then(|rs| rs.last_message_id.clone());
        match last_read {
            Some(last_read) if self.channel_is_unread(channel_id) => {
                self.unread_anchor.insert(channel_id.to_string(), last_read);
            }
            _ => {
                self.unread_anchor.remove(channel_id);
            }
        }
        self.messages_version = self.messages_version.wrapping_add(1);
    }

    /// The message the line sits above: the oldest one in the loaded
    /// history that arrived after the anchor. None when there is no line
    /// to draw, or when everything after the anchor is off the top.
    pub fn first_unread_message_id(&self, channel_id: &str) -> Option<String> {
        let anchor = self.unread_anchor.get(channel_id)?;
        let anchor_key = snowflake_sort_key(anchor);
        self.messages
            .get(channel_id)?
            .iter()
            .find(|m| snowflake_sort_key(&m.id) > anchor_key)
            .map(|m| m.id.clone())
    }

    pub fn active_first_unread_message_id(&self) -> Option<String> {
        self.first_unread_message_id(&self.active_channel_id()?)
    }

    /// `U`: put the cursor on the first message that arrived after the
    /// reader last left, and scroll it into view. False when there is no
    /// line to jump to.
    pub fn jump_to_first_unread(&mut self) -> bool {
        let Some(channel_id) = self.active_channel_id() else {
            return false;
        };
        let Some(first_unread) = self.first_unread_message_id(&channel_id) else {
            return false;
        };
        let Some(index) = self
            .messages
            .get(&channel_id)
            .and_then(|messages| messages.iter().position(|m| m.id == first_unread))
        else {
            return false;
        };
        self.focus = Focus::Messages;
        self.selected_message_index = Some(index);
        // moving the selection is a content change, so the reader anchor
        // would put the view back; see the fork's notes on that family
        self.pane_anchor = None;
        self.clamp_scroll_to_selected_message();
        true
    }

    /// Drop the line once the reader has scrolled to the bottom of the
    /// channel with everything read: at that point it has done its job
    /// and would only be in the way next time.
    pub fn clear_unread_anchor_if_caught_up(&mut self) {
        let Some(channel_id) = self.active_channel_id() else {
            return;
        };
        if !self.unread_anchor.contains_key(&channel_id) {
            return;
        }
        if self.message_scroll_from_bottom == 0 && !self.channel_is_unread(&channel_id) {
            self.clear_unread_anchor(&channel_id);
        }
    }

    /// Drop the line: the reader asked to, or has caught up on purpose.
    pub fn clear_unread_anchor(&mut self, channel_id: &str) {
        if self.unread_anchor.remove(channel_id).is_some() {
            self.messages_version = self.messages_version.wrapping_add(1);
        }
    }

    // r (as in reply)

    pub fn start_reply(&mut self) {
        if let Some(msg) = self.selected_message() {
            self.edit_target = None;
            let src_guild = self.guild_id_for_channel(&msg.channel_id);
            self.reply_to = Some(ReplyState {
                channel_id: msg.channel_id.clone(),
                message_id: msg.id.clone(),
                author_name: self.shown_name_for_user(src_guild.as_deref(), &msg.author),
                source_guild_id: src_guild,
            });
            self.forward_mode = false;
            self.focus = Focus::Input;
        }
    }

    /// `e`: aim the emoji picker at a message, so the next emoji chosen
    /// in the compose box becomes a reaction on it.
    pub fn start_reaction_picker(&mut self, channel_id: String, message_id: String) {
        if !self.can_react_in_active_channel() {
            self.set_status("No permission to add reactions here.");
            return;
        }
        self.reaction_target = Some((channel_id, message_id));
        self.focus = Focus::Input;
        self.set_input(":");
        self.start_emoji_autocomplete();
        self.set_status("Pick an emoji, Enter to react (Esc to cancel)");
    }

    /// `f`: carry a message to another channel; the compose box takes an
    /// optional note and Ctrl+K picks where it goes.
    pub fn start_forward(&mut self) {
        let Some(msg) = self.selected_message() else {
            return;
        };
        self.edit_target = None;
        self.forward_mode = true;
        let src_guild = self.guild_id_for_channel(&msg.channel_id);
        self.reply_to = Some(ReplyState {
            channel_id: msg.channel_id.clone(),
            message_id: msg.id.clone(),
            author_name: self.shown_name_for_user(src_guild.as_deref(), &msg.author),
            source_guild_id: src_guild,
        });
        self.set_status("Forward: pick channel (Ctrl+K), type optional note, Enter to send");
    }

    /// The message just above one in a channel's loaded history. Marking
    /// unread acknowledges up to there, so the message picked is the
    /// first thing left unread.
    pub fn message_before(&self, channel_id: &str, message_id: &str) -> Option<String> {
        let messages = self.messages.get(channel_id)?;
        let index = messages.iter().position(|m| m.id == message_id)?;
        messages.get(index.checked_sub(1)?).map(|m| m.id.clone())
    }

    /// How many of the loaded messages from `message_id` onwards mention
    /// the user; the ack carries it so the unread badge is right at once.
    pub fn mention_count_from(&self, channel_id: &str, message_id: &str) -> u32 {
        let Some(messages) = self.messages.get(channel_id) else {
            return 0;
        };
        let Some(index) = messages.iter().position(|m| m.id == message_id) else {
            return 0;
        };
        messages[index..]
            .iter()
            .filter(|m| self.message_mentions_me(m))
            .count() as u32
    }

    pub fn newest_message_id(&self, channel_id: &str) -> Option<String> {
        self.messages.get(channel_id)?.last().map(|m| m.id.clone())
    }

    /// Every unread channel of the community now open, with the newest
    /// message the client knows for each: what a "mark the community
    /// read" call takes. Channels with no loaded history are skipped,
    /// since there is no message to acknowledge up to.
    pub fn unread_channels_with_newest(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for channel in self.all_channels_for_server(&self.selected_server) {
            if channel.channel_type() == CHANNEL_GUILD_CATEGORY {
                continue;
            }
            if !self.channel_is_unread(&channel.id) {
                continue;
            }
            if let Some(newest) = self.newest_message_id(&channel.id) {
                out.push((channel.id.clone(), newest));
            }
        }
        out
    }

    pub fn cancel_reply(&mut self) {
        self.reply_to = None;
        self.forward_mode = false;
        self.edit_target = None;
    }

    pub fn open_channel_picker(&mut self) {
        let mut entries = Vec::new();
        for server in self.server_entries() {
            let server_name = match &server {
                ServerSelection::DirectMessages => "Direct messages".to_string(),
                ServerSelection::Guild(gid) => self
                    .guilds
                    .iter()
                    .find(|g| g.id == *gid)
                    .map(|g| g.name.clone())
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| {
                        let short: String = gid.chars().take(8).collect();
                        format!("guild {short}")
                    }),
            };
            for ch in self.channels_for_server(&server) {
                if ch.channel_type() == CHANNEL_GUILD_CATEGORY {
                    continue;
                }
                if !matches!(
                    ch.channel_type(),
                    CHANNEL_GUILD_TEXT
                        | CHANNEL_DM
                        | CHANNEL_GROUP_DM
                        | CHANNEL_DM_PERSONAL_NOTES
                        | CHANNEL_GUILD_LINK
                ) {
                    continue;
                }
                let ch_label = picker_channel_line(&ch);
                let label = format!("{ch_label} · {server_name}");
                entries.push(PickerEntry {
                    server: server.clone(),
                    channel_id: ch.id.clone(),
                    label,
                });
            }
        }
        let n = entries.len();
        self.channel_picker = Some(ChannelPicker {
            query: String::new(),
            entries,
            filtered: (0..n).collect(),
            selected: 0,
        });
    }

    pub fn filter_channel_picker(&mut self) {
        let Some(p) = self.channel_picker.as_mut() else {
            return;
        };
        let q = p.query.to_lowercase();
        p.filtered = p
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| q.is_empty() || e.label.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        if p.filtered.is_empty() {
            p.selected = 0;
        } else {
            p.selected = p.selected.min(p.filtered.len() - 1);
        }
    }

    pub fn channel_picker_prev(&mut self) {
        let Some(p) = self.channel_picker.as_mut() else {
            return;
        };
        if p.filtered.is_empty() {
            return;
        }
        p.selected = (p.selected + p.filtered.len() - 1) % p.filtered.len();
    }

    pub fn channel_picker_next(&mut self) {
        let Some(p) = self.channel_picker.as_mut() else {
            return;
        };
        if p.filtered.is_empty() {
            return;
        }
        p.selected = (p.selected + 1) % p.filtered.len();
    }

    pub fn channel_picker_confirm(&mut self) -> bool {
        let Some(p) = &self.channel_picker else {
            return false;
        };
        let Some(&ei) = p.filtered.get(p.selected) else {
            return false;
        };
        let Some(entry) = p.entries.get(ei) else {
            return false;
        };
        self.selected_server = entry.server.clone();
        self.selected_channel_id = Some(entry.channel_id.clone());
        self.channel_picker = None;
        self.message_scroll_from_bottom = 0;
        self.selected_message_index = None;
        self.normalize_selection();
        true
    }

    pub fn dismiss_channel_picker(&mut self) {
        self.channel_picker = None;
    }

    pub fn confirm_reaction_emoji(&mut self) -> Option<(String, String, String)> {
        let (ch_id, msg_id) = self.reaction_target.clone()?;
        let auto = self.emoji_autocomplete.as_ref()?;
        let emoji = auto.matches.get(auto.selected_index)?;
        let api = encode_reaction_for_api(&emoji.insert);
        self.reaction_target = None;
        self.emoji_autocomplete = None;
        self.clear_input();
        Some((ch_id, msg_id, api))
    }

    // ma

    pub fn start_mention_autocomplete(&mut self) {
        let pool = self.build_mention_pool();
        if pool.is_empty() {
            return;
        }
        self.mention_autocomplete = Some(MentionAutocomplete {
            pool,
            matches: Vec::new(),
            selected_index: 0,
        });
        self.update_mention_filter();
    }

    /// repopulate @ UI if it was waiting
    pub fn refresh_mention_autocomplete_after_members_load(&mut self, loaded_guild_id: &str) {
        if self.mention_autocomplete.is_none() {
            return;
        }
        if self.active_guild_id().as_deref() != Some(loaded_guild_id) {
            return;
        }
        let pool = self.build_mention_pool();
        if pool.is_empty() {
            self.mention_autocomplete = None;
            return;
        }
        if let Some(auto) = &mut self.mention_autocomplete {
            auto.pool = pool;
            auto.selected_index = 0;
        }
        self.update_mention_filter();
        if self.mention_autocomplete.is_some() {
            self.clear_status();
        }
    }

    const MENTION_FILTER_CAP: usize = 400;
    const MENTION_INITIAL_CAP: usize = 80;

    fn build_mention_pool(&self) -> Vec<MentionPick> {
        let mut users: Vec<MentionPick> = Vec::new();
        let mut roles: Vec<MentionPick> = Vec::new();
        let mut seen_users: HashSet<String> = HashSet::new();

        match &self.selected_server {
            ServerSelection::Guild(guild_id) => {
                if let Some(members) = self.guild_members.get(guild_id) {
                    for m in members {
                        if m.user.id.is_empty() || !seen_users.insert(m.user.id.clone()) {
                            continue;
                        }
                        let cached_user = self.user_cache.get(&m.user.id);
                        let username = if !m.user.username.is_empty() {
                            m.user.username.clone()
                        } else {
                            cached_user.map(|u| u.username.clone()).unwrap_or_default()
                        };
                        let base_display = if !m.user.username.is_empty() {
                            account_display_name(cached_user.unwrap_or(&m.user))
                        } else {
                            cached_user.map(account_display_name).unwrap_or_default()
                        };
                        let nick_display = m
                            .nick
                            .clone()
                            .filter(|n| !n.trim().is_empty())
                            .unwrap_or(base_display);
                        if username.is_empty() && nick_display.is_empty() {
                            continue;
                        }
                        users.push(MentionPick::User {
                            user_id: m.user.id.clone(),
                            display: nick_display,
                            username,
                        });
                    }
                }
                if let Some(rs) = self.guild_roles.get(guild_id) {
                    let mut seen_roles: HashSet<String> = HashSet::new();
                    for r in rs {
                        if r.id.is_empty() || !seen_roles.insert(r.id.clone()) {
                            continue;
                        }
                        let name = if r.name.trim().is_empty() {
                            continue;
                        } else {
                            r.name.clone()
                        };
                        roles.push(MentionPick::Role {
                            role_id: r.id.clone(),
                            name,
                            color: r.color,
                        });
                    }
                }
                roles.sort_by(|a, b| match (a, b) {
                    (MentionPick::Role { name: na, .. }, MentionPick::Role { name: nb, .. }) => {
                        na.to_lowercase().cmp(&nb.to_lowercase())
                    }
                    _ => std::cmp::Ordering::Equal,
                });
            }
            ServerSelection::DirectMessages => {
                if let Some(ch) = self.active_channel() {
                    for r in &ch.recipients {
                        if r.id.is_empty() || !seen_users.insert(r.id.clone()) {
                            continue;
                        }
                        users.push(MentionPick::User {
                            user_id: r.id.clone(),
                            display: display_name(r),
                            username: r.username.clone(),
                        });
                    }
                }
            }
        }

        if let Some(channel_id) = self.selected_channel_id.as_deref() {
            let guild_id = self.guild_id_for_channel(channel_id);
            if let Some(msgs) = self.messages.get(channel_id) {
                for msg in msgs.iter() {
                    if msg.author.id.is_empty() || !seen_users.insert(msg.author.id.clone()) {
                        continue;
                    }
                    let nick = guild_id.as_ref().and_then(|gid| {
                        self.guild_members.get(gid).and_then(|mems| {
                            mems.iter()
                                .find(|m| m.user.id == msg.author.id)
                                .and_then(|m| m.nick.clone())
                        })
                    });
                    let u = self.user_cache.get(&msg.author.id).unwrap_or(&msg.author);
                    let base_display = account_display_name(u);
                    let display = nick
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or(base_display.clone());
                    let username = if !msg.author.username.is_empty() {
                        msg.author.username.clone()
                    } else {
                        self.user_cache
                            .get(&msg.author.id)
                            .map(|u| u.username.clone())
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| base_display.clone())
                    };
                    if username.is_empty() && display.is_empty() {
                        continue;
                    }
                    users.push(MentionPick::User {
                        user_id: msg.author.id.clone(),
                        display,
                        username,
                    });
                }
            }
        }

        users.sort_by(|a, b| match (a, b) {
            (MentionPick::User { display: da, .. }, MentionPick::User { display: db, .. }) => {
                da.to_lowercase().cmp(&db.to_lowercase())
            }
            _ => std::cmp::Ordering::Equal,
        });

        let mut pool = users;
        pool.extend(roles);
        pool
    }

    pub fn update_mention_filter(&mut self) {
        let Some(auto) = &mut self.mention_autocomplete else {
            return;
        };

        let query = self.input.rsplit('@').next().unwrap_or("").to_lowercase();

        auto.matches = if query.is_empty() {
            (0..auto.pool.len().min(Self::MENTION_INITIAL_CAP)).collect()
        } else {
            auto.pool
                .iter()
                .enumerate()
                .filter(|(_, p)| p.matches_filter(&query))
                .map(|(i, _)| i)
                .take(Self::MENTION_FILTER_CAP)
                .collect()
        };

        if auto.selected_index >= auto.matches.len() {
            auto.selected_index = auto.matches.len().saturating_sub(1);
        }
        if auto.matches.is_empty() {
            self.mention_autocomplete = None;
        }
    }

    pub fn dismiss_mention_autocomplete(&mut self) {
        self.mention_autocomplete = None;
    }

    pub fn autocomplete_mention_next(&mut self) {
        if let Some(auto) = &mut self.mention_autocomplete
            && !auto.matches.is_empty()
        {
            auto.selected_index = (auto.selected_index + 1) % auto.matches.len();
        }
    }

    pub fn autocomplete_mention_prev(&mut self) {
        if let Some(auto) = &mut self.mention_autocomplete
            && !auto.matches.is_empty()
        {
            auto.selected_index =
                auto.selected_index.saturating_add(auto.matches.len() - 1) % auto.matches.len();
        }
    }

    pub fn insert_selected_mention(&mut self) -> bool {
        if let Some(auto) = &self.mention_autocomplete
            && let Some(&pool_idx) = auto.matches.get(auto.selected_index)
            && let Some(pick) = auto.pool.get(pool_idx)
            && let Some(at_pos) = self.input.rfind('@')
        {
            self.input.truncate(at_pos);
            match pick {
                MentionPick::User { user_id, .. } => {
                    self.input.push_str(&format!("<@{user_id}> "));
                }
                MentionPick::Role { role_id, .. } => {
                    self.input.push_str(&format!("<@&{role_id}> "));
                }
            }
            self.mention_autocomplete = None;
            return true;
        }
        false
    }

    pub fn self_nick_or_username_in_guild(&self, guild_id: &str) -> String {
        self.shown_name_for_user(Some(guild_id), &me_as_partial(&self.me))
    }

    pub fn shown_name_for_user(
        &self,
        guild_id: Option<&str>,
        user: &UserPartialResponse,
    ) -> String {
        let u = self.user_cache.get(&user.id).unwrap_or(user);
        if let Some(gid) = guild_id
            && let Some(members) = self.guild_members.get(gid)
            && let Some(m) = members.iter().find(|m| m.user.id == user.id)
        {
            let base = self.user_cache.get(&m.user.id).unwrap_or(&m.user);
            return m
                .nick
                .as_ref()
                .filter(|n| !n.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| account_display_name(base));
        }
        account_display_name(u)
    }

    pub fn member_name_color(&self, guild_id: Option<&str>, user_id: &str, is_self: bool) -> Color {
        use crate::api::types::snowflake_sort_key;

        let guild_default = || crate::ui::theme::text();

        if let Some(gid) = guild_id {
            if let Some(members) = self.guild_members.get(gid)
                && let Some(member) = members.iter().find(|m| m.user.id == user_id)
            {
                if let Some(roles) = self.guild_roles.get(gid) {
                    let role_pos = |rid: &str| {
                        let rid = rid.trim();
                        roles
                            .iter()
                            .find(|r| r.id.trim() == rid)
                            .map(|r| r.position)
                            .unwrap_or(i32::MIN)
                    };
                    let mut role_ids: Vec<&str> = member.roles.iter().map(|s| s.as_str()).collect();
                    role_ids.sort_by(|a, b| {
                        role_pos(b).cmp(&role_pos(a)).then_with(|| {
                            snowflake_sort_key(a.trim()).cmp(&snowflake_sort_key(b.trim()))
                        })
                    });
                    for rid in role_ids {
                        let rid = rid.trim();
                        if let Some(r) = roles.iter().find(|rr| rr.id.trim() == rid)
                            && r.color != 0
                        {
                            return crate::ui::theme::rgb_pack_to_color(r.color);
                        }
                    }
                    let gid_trim = gid.trim();
                    if let Some(everyone) = roles.iter().find(|r| r.id.trim() == gid_trim)
                        && everyone.color != 0
                    {
                        return crate::ui::theme::rgb_pack_to_color(everyone.color);
                    }
                }
                return guild_default();
            }
            return guild_default();
        }

        if is_self {
            crate::ui::theme::self_username_color()
        } else {
            crate::ui::theme::username_color(user_id)
        }
    }

    pub fn sync_command_autocomplete(&mut self) {
        if self.focus != Focus::Input || !self.can_send_in_active_channel() {
            self.command_autocomplete = None;
            return;
        }
        let Some(q) = crate::slash_commands::command_name_query(&self.input) else {
            self.command_autocomplete = None;
            return;
        };
        let guild_ch = self
            .active_channel()
            .and_then(|c| c.guild_id.clone())
            .is_some();
        let ch_perms = self.active_channel_permissions();
        let matches = crate::slash_commands::filter_command_indices(q, guild_ch, ch_perms);
        if matches.is_empty() {
            self.command_autocomplete = None;
            return;
        }
        let selected_index = self
            .command_autocomplete
            .as_ref()
            .map(|a| a.selected_index.min(matches.len().saturating_sub(1)))
            .unwrap_or(0);
        self.command_autocomplete = Some(CommandAutocomplete {
            matches,
            selected_index,
        });
    }

    pub fn dismiss_command_autocomplete(&mut self) {
        self.command_autocomplete = None;
    }

    pub fn autocomplete_command_next(&mut self) {
        if let Some(auto) = &mut self.command_autocomplete
            && !auto.matches.is_empty()
        {
            auto.selected_index = (auto.selected_index + 1) % auto.matches.len();
        }
    }

    pub fn autocomplete_command_prev(&mut self) {
        if let Some(auto) = &mut self.command_autocomplete
            && !auto.matches.is_empty()
        {
            auto.selected_index =
                auto.selected_index.saturating_add(auto.matches.len() - 1) % auto.matches.len();
        }
    }

    pub fn insert_selected_slash_command(&mut self) -> bool {
        let Some(auto) = &self.command_autocomplete else {
            return false;
        };
        let Some(&cmd_i) = auto.matches.get(auto.selected_index) else {
            return false;
        };
        let cmd = &crate::slash_commands::SLASH_COMMANDS[cmd_i];
        let Some(slash_pos) = self.input.find('/') else {
            return false;
        };
        let after = &self.input[slash_pos + 1..];
        let token_len = after
            .find(|c: char| c.is_whitespace())
            .unwrap_or(after.len());
        let end = slash_pos + 1 + token_len;
        let trailing_space = cmd.simple_append.is_none();
        let mut new_in = String::new();
        new_in.push_str(&self.input[..slash_pos]);
        new_in.push_str(cmd.name);
        if trailing_space {
            new_in.push(' ');
        }
        new_in.push_str(&self.input[end..]);
        self.input = new_in;
        self.command_autocomplete = None;
        self.sync_command_autocomplete();
        true
    }

    fn merge_single_message_member(&mut self, message: &MessageResponse) {
        let Some(mem) = message.member.as_ref() else {
            return;
        };
        let Some(gid) = self.guild_id_for_channel(&message.channel_id) else {
            return;
        };
        self.merge_guild_member(gid.as_str(), mem.clone());
    }

    fn merge_message_embedded_members(&mut self, message: &MessageResponse) {
        self.merge_single_message_member(message);
        if let Some(r) = message.referenced_message.as_deref() {
            self.merge_message_embedded_members(r);
        }
    }

    /// Take somebody off a community's member list, from
    /// GUILD_MEMBER_REMOVE. Their presence goes with them: the server
    /// stops sending one once they are no longer a member.
    pub fn remove_guild_member(&mut self, guild_id: &str, user_id: &str) {
        if let Some(members) = self.guild_members.get_mut(guild_id) {
            members.retain(|m| m.user.id != user_id);
        }
        // only where nothing else keeps them visible: a friend, or
        // somebody in a conversation, still has a presence of their own
        let elsewhere =
            self.guild_members.iter().any(|(gid, members)| {
                gid != guild_id && members.iter().any(|m| m.user.id == user_id)
            }) || self
                .private_channels
                .iter()
                .any(|c| c.recipients.iter().any(|u| u.id == user_id));
        if !elsewhere {
            self.presences.remove(user_id);
            self.presence_version = self.presence_version.wrapping_add(1);
        }
    }

    pub fn merge_guild_member(&mut self, guild_id: &str, mut member: GuildMemberResponse) {
        self.roster_version = self.roster_version.wrapping_add(1);
        merge_user_cache(&mut self.user_cache, [member.user.clone()]);
        let members = self.guild_members.entry(guild_id.to_string()).or_default();
        if let Some(existing) = members.iter().find(|m| m.user.id == member.user.id) {
            if member.roles.is_empty() && !existing.roles.is_empty() {
                member.roles = existing.roles.clone();
            }
            if member.nick.is_none() && existing.nick.is_some() {
                member.nick = existing.nick.clone();
            }
        }
        if let Some(existing) = members.iter_mut().find(|m| m.user.id == member.user.id) {
            *existing = member;
        } else {
            members.push(member);
        }
    }

    pub fn allocate_local_message_snowflake(&self, channel_id: &str) -> String {
        let msgs = self
            .messages
            .get(channel_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let max_k = msgs
            .iter()
            .map(|m| snowflake_sort_key(&m.id))
            .max()
            .unwrap_or(0);
        const DISCORD_EPOCH_MS: u128 = 1420070400000;
        let ts = chrono::Utc::now().timestamp_millis() as u128;
        let delta = ts.saturating_sub(DISCORD_EPOCH_MS);
        let mut candidate = delta.saturating_mul(1u128 << 22);
        if candidate <= max_k {
            candidate = max_k.saturating_add(1);
        }
        candidate.to_string()
    }

    // permission(orn) helpers

    pub fn can_react_in_active_channel(&self) -> bool {
        self.active_channel_permissions() & crate::permissions::ADD_REACTIONS != 0
    }
}

/// What a copied message hands over: the text as it was written, and
/// the link of every attachment on its own line under it. Stickers,
/// reactions and embeds carry no text of their own and are left out, so
/// a message made of nothing but a sticker copies as nothing.
pub fn message_copy_text(msg: &MessageResponse) -> String {
    let mut out = msg.content.trim_end().to_string();
    for att in &msg.attachments {
        let Some(url) = att
            .url
            .as_deref()
            .or(att.proxy_url.as_deref())
            .map(str::trim)
            .filter(|u| !u.is_empty())
        else {
            continue;
        };
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(url);
    }
    out
}

fn picker_channel_line(ch: &ChannelResponse) -> String {
    match ch.channel_type() {
        CHANNEL_GUILD_TEXT | CHANNEL_GUILD_LINK => {
            if ch.name.is_empty() {
                ch.id.chars().take(6).collect()
            } else {
                format!("#{}", ch.name)
            }
        }
        CHANNEL_DM_PERSONAL_NOTES => "Personal notes".to_string(),
        CHANNEL_DM => ch
            .recipients
            .first()
            .map(display_name)
            .unwrap_or_else(|| "DM".to_string()),
        CHANNEL_GROUP_DM => {
            if !ch.name.trim().is_empty() {
                ch.name.clone()
            } else if !ch.recipients.is_empty() {
                ch.recipients
                    .iter()
                    .map(display_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                "Group DM".to_string()
            }
        }
        _ => {
            if ch.name.is_empty() {
                ch.id.chars().take(6).collect()
            } else {
                ch.name.clone()
            }
        }
    }
}

/// The two spellings of a reaction's emoji: what the reaction routes
/// take (the character, or `name:id` for a custom one) and what to put on
/// the screen (the character, or `:name:`).
pub fn reaction_emoji_forms(
    reaction: &crate::api::types::MessageReactionResponse,
) -> (String, String) {
    match reaction.emoji.id.as_deref() {
        Some(id) if !id.is_empty() => (
            format!("{}:{}", reaction.emoji.name, id),
            format!(":{}:", reaction.emoji.name),
        ),
        _ => (reaction.emoji.name.clone(), reaction.emoji.name.clone()),
    }
}

fn encode_reaction_for_api(insert: &str) -> String {
    let t = insert.trim();
    if t.starts_with('<') && t.ends_with('>') && t.contains(':') {
        let inner = &t[1..t.len() - 1];
        let parts: Vec<&str> = inner.split(':').collect();
        if parts.len() >= 3 {
            let id = parts[parts.len() - 1];
            let name = parts[parts.len() - 2];
            if !name.is_empty() && !id.is_empty() {
                return format!("{name}:{id}");
            }
        }
    }
    t.to_string()
}

pub fn me_as_partial(me: &UserPrivateResponse) -> UserPartialResponse {
    UserPartialResponse {
        id: me.id.clone(),
        username: me.username.clone(),
        discriminator: me.discriminator.clone(),
        global_name: me.global_name.clone(),
        avatar: me.avatar.clone(),
        avatar_color: me.avatar_color,
        bot: me.bot,
        system: me.system,
        flags: 0,
    }
}

pub fn account_display_name(user: &UserPartialResponse) -> String {
    user.global_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| username_handle(user))
}

fn username_handle(user: &UserPartialResponse) -> String {
    if user.discriminator.is_empty() {
        user.username.clone()
    } else {
        format!("{}#{}", user.username, user.discriminator)
    }
}

pub fn display_name(user: &UserPartialResponse) -> String {
    account_display_name(user)
}

/// The user's own typing, told to the channel the way the web client
/// does it: the first POST 1.5 s after the first keystroke of a bout,
/// another one no sooner than 8 s after the previous when keys kept
/// coming, and nothing once 10 s passed without a key. An empty compose
/// box (a sent message) ends the bout; the 8 s spacing holds across
/// bouts in the same channel.
#[derive(Debug, Default)]
pub struct OwnTyping {
    /// The compose text at the last look, to tell a key that changed it
    /// from one that did not.
    last_text: String,
    /// The bout under way: its channel, first and latest change.
    bout: Option<(String, Instant, Instant)>,
    /// The last POST: channel and time.
    last_sent: Option<(String, Instant)>,
}

impl OwnTyping {
    pub const SEND_DELAY: Duration = Duration::from_millis(1500);
    pub const REFRESH: Duration = Duration::from_secs(8);
    pub const IDLE: Duration = Duration::from_secs(10);

    /// `text` is the whole compose text, `channel` where it would go;
    /// `counts` false ends the bout (setting off, editing a message).
    pub fn note(&mut self, text: &str, channel: Option<&str>, counts: bool, now: Instant) {
        let changed = text != self.last_text;
        if changed {
            self.last_text = text.to_string();
        }
        let content = text.trim();
        let typing = counts && !content.is_empty() && !content.starts_with('/');
        let Some(channel) = channel.filter(|_| typing) else {
            self.bout = None;
            return;
        };
        match &mut self.bout {
            Some((c, _, last)) if c == channel => {
                if changed {
                    *last = now;
                }
            }
            // a draft carried into another channel is not typing there
            // until a key changes it
            Some(_) => self.bout = None,
            None if changed => self.bout = Some((channel.to_string(), now, now)),
            None => {}
        }
    }

    /// The channel a POST is due for; call [`Self::sent`] after it.
    pub fn due(&mut self, now: Instant) -> Option<String> {
        let (channel, started, last_change) = self.bout.as_ref()?;
        if now.duration_since(*last_change) >= Self::IDLE {
            self.bout = None;
            return None;
        }
        if now.duration_since(*started) < Self::SEND_DELAY {
            return None;
        }
        let spaced = match &self.last_sent {
            Some((c, sent)) if c == channel => {
                *last_change > *sent && now.duration_since(*sent) >= Self::REFRESH
            }
            _ => true,
        };
        spaced.then(|| channel.clone())
    }

    pub fn sent(&mut self, channel: &str, now: Instant) {
        self.last_sent = Some((channel.to_string(), now));
    }

    pub fn end_bout(&mut self) {
        self.bout = None;
    }
}

fn fluxer_typing_phrase(names: &[String]) -> String {
    const SEVERAL: &str = "Several people are typing...";
    const HANDFUL: &str = "A handful of keyboard warriors are assembling...";
    const SYMPHONY: &str = "A symphony of clacking keys is underway...";
    const FIESTA: &str = "It's a full-blown typing fiesta in here";
    const APOCALYPSE: &str = "Whoa, it's a typing apocalypse";

    match names.len() {
        1 => format!("{} is typing...", names[0]),
        2 => format!("{} and {} are typing...", names[0], names[1]),
        3 => format!("{}, {} and {} are typing...", names[0], names[1], names[2]),
        4 => SEVERAL.to_string(),
        n if (5..=9).contains(&n) => HANDFUL.to_string(),
        n if (10..=14).contains(&n) => SYMPHONY.to_string(),
        n if (15..=19).contains(&n) => FIESTA.to_string(),
        _ => APOCALYPSE.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app(private_channels: Vec<ChannelResponse>) -> App {
        let mut me = UserPrivateResponse::default();
        me.id = "me".to_string();

        App::new(
            WellKnownFluxerResponse::default(),
            me,
            None,
            Vec::new(),
            private_channels,
            ServerSelection::DirectMessages,
            None,
            UiSettings::default(),
        )
    }

    fn guild_app(channels: Vec<ChannelResponse>) -> App {
        let mut app = test_app(Vec::new());
        app.guilds.push(GuildResponse {
            id: "guild-1".to_string(),
            name: "Guild".to_string(),
            ..GuildResponse::default()
        });
        app.guild_channels.insert("guild-1".to_string(), channels);
        app.selected_server = ServerSelection::Guild("guild-1".to_string());
        app
    }

    fn presence(user_id: &str, status: &str) -> PresenceRecord {
        PresenceRecord {
            user: UserPartialResponse {
                id: user_id.to_string(),
                username: user_id.to_string(),
                ..Default::default()
            },
            status: Some(status.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn a_presence_is_kept_and_an_offline_one_forgets_the_row() {
        let mut app = test_app(Vec::new());
        assert_eq!(app.presence_status("u2"), PresenceStatus::Offline);
        app.apply_presence(presence("u2", "online"));
        assert_eq!(app.presence_status("u2"), PresenceStatus::Online);
        app.apply_presence(presence("u2", "idle"));
        assert_eq!(app.presence_status("u2"), PresenceStatus::Idle);
        // going offline drops the row rather than keeping an offline one
        app.apply_presence(presence("u2", "offline"));
        assert!(app.presence_entry("u2").is_none());
        assert_eq!(app.presence_status("u2"), PresenceStatus::Offline);
    }

    #[test]
    fn an_offline_presence_with_a_status_line_is_kept_for_the_line() {
        let mut app = test_app(Vec::new());
        let mut record = presence("u2", "offline");
        record.custom_status = Some(CustomStatusPayload {
            text: Some("back later".to_string()),
            ..Default::default()
        });
        app.apply_presence(record);
        let entry = app.presence_entry("u2").expect("kept");
        assert_eq!(entry.status, PresenceStatus::Offline);
        assert_eq!(
            entry.custom_status.as_ref().and_then(|c| c.text.as_deref()),
            Some("back later")
        );
    }

    #[test]
    fn the_readers_own_status_comes_from_their_settings_not_a_presence() {
        let mut app = test_app(Vec::new());
        // with no settings at all the client assumes online rather than
        // showing the reader as offline to themselves
        assert_eq!(app.presence_status("me"), PresenceStatus::Online);
        app.user_settings = Some(UserSettingsResponse {
            status: "dnd".to_string(),
            ..Default::default()
        });
        assert_eq!(app.own_status(), PresenceStatus::Dnd);
        assert_eq!(app.presence_status("me"), PresenceStatus::Dnd);
        app.set_own_status(PresenceStatus::Invisible);
        assert_eq!(app.own_status(), PresenceStatus::Invisible);
        // and a presence the server sent about the reader does not win
        app.apply_presence(presence("me", "online"));
        assert_eq!(app.presence_status("me"), PresenceStatus::Invisible);
    }

    #[test]
    fn every_presence_change_moves_the_version_the_pane_caches_on() {
        let mut app = test_app(Vec::new());
        let before = app.presence_version;
        app.apply_presence(presence("u2", "online"));
        assert_ne!(app.presence_version, before);
        let mid = app.presence_version;
        app.apply_presence(presence("u2", "offline"));
        assert_ne!(app.presence_version, mid);
    }

    #[test]
    fn a_one_to_one_channel_knows_whose_presence_it_shows() {
        let channel = ChannelResponse {
            id: "dm1".to_string(),
            kind: CHANNEL_DM,
            recipients: vec![
                UserPartialResponse {
                    id: "me".to_string(),
                    ..Default::default()
                },
                UserPartialResponse {
                    id: "u2".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let app = test_app(vec![channel.clone()]);
        assert_eq!(app.dm_peer_id(&channel), Some("u2".to_string()));
        // a group has no single peer, so no dot
        let group = ChannelResponse {
            kind: CHANNEL_GROUP_DM,
            ..channel
        };
        assert_eq!(app.dm_peer_id(&group), None);
    }

    #[test]
    fn a_pinned_conversation_sits_above_a_busier_one() {
        let older = ChannelResponse {
            id: "quiet".to_string(),
            kind: CHANNEL_DM,
            last_message_id: Some("100".to_string()),
            ..Default::default()
        };
        let newer = ChannelResponse {
            id: "busy".to_string(),
            kind: CHANNEL_DM,
            last_message_id: Some("900".to_string()),
            ..Default::default()
        };
        let mut app = test_app(vec![older, newer]);
        // by recency alone the busy one is first
        let order: Vec<String> = app
            .all_channels_for_server(&ServerSelection::DirectMessages)
            .iter()
            .map(|c| c.id.clone())
            .collect();
        assert_eq!(order, vec!["busy", "quiet"]);
        app.set_pinned_dms(vec!["quiet".to_string()]);
        let order: Vec<String> = app
            .all_channels_for_server(&ServerSelection::DirectMessages)
            .iter()
            .map(|c| c.id.clone())
            .collect();
        assert_eq!(order, vec!["quiet", "busy"]);
        assert!(app.is_dm_pinned("quiet"));
    }

    #[test]
    fn somebody_joining_or_leaving_a_group_changes_its_people() {
        let group = ChannelResponse {
            id: "g1".to_string(),
            kind: CHANNEL_GROUP_DM,
            recipients: vec![UserPartialResponse {
                id: "u1".to_string(),
                username: "ada".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut app = test_app(vec![group]);
        let bob = UserPartialResponse {
            id: "u2".to_string(),
            username: "bob".to_string(),
            ..Default::default()
        };
        app.set_group_recipient("g1", bob.clone(), true);
        assert_eq!(app.group_members("g1").len(), 2);
        // adding the same person again does not double them
        app.set_group_recipient("g1", bob.clone(), true);
        assert_eq!(app.group_members("g1").len(), 2);
        app.set_group_recipient("g1", bob, false);
        assert_eq!(app.group_members("g1").len(), 1);
    }

    #[test]
    fn closing_a_conversation_takes_it_off_the_list_and_the_pins() {
        let mut app = test_app(vec![ChannelResponse {
            id: "dm1".to_string(),
            kind: CHANNEL_DM,
            ..Default::default()
        }]);
        app.set_pinned_dms(vec!["dm1".to_string()]);
        app.selected_channel_id = Some("dm1".to_string());
        app.remove_private_channel("dm1");
        assert!(app.private_channels.is_empty());
        assert!(!app.is_dm_pinned("dm1"));
        assert_eq!(app.selected_channel_id, None);
    }

    #[test]
    fn the_reader_is_never_offered_as_somebody_to_talk_to() {
        let mut app = test_app(vec![ChannelResponse {
            id: "g1".to_string(),
            kind: CHANNEL_GROUP_DM,
            recipients: vec![
                UserPartialResponse {
                    id: "me".to_string(),
                    username: "me".to_string(),
                    ..Default::default()
                },
                UserPartialResponse {
                    id: "u1".to_string(),
                    username: "ada".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }]);
        app.open_new_conversation();
        let ids: Vec<String> = app
            .conversation_candidates()
            .iter()
            .map(|c| c.user.id.clone())
            .collect();
        assert_eq!(ids, vec!["u1".to_string()]);
    }

    fn relationship(user_id: &str, kind: i32) -> RelationshipResponse {
        RelationshipResponse {
            id: format!("r{user_id}"),
            relationship_type: kind,
            user: UserPartialResponse {
                id: user_id.to_string(),
                username: user_id.to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn message_from(id: &str, channel_id: &str, author: &str) -> MessageResponse {
        MessageResponse {
            id: id.to_string(),
            channel_id: channel_id.to_string(),
            author: UserPartialResponse {
                id: author.to_string(),
                username: author.to_string(),
                ..Default::default()
            },
            content: format!("from {author}"),
            timestamp: "2026-09-10T10:00:00.000Z".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn a_blocked_accounts_messages_are_never_kept() {
        let mut app = test_app(Vec::new());
        app.upsert_relationship(relationship("bad", RELATIONSHIP_BLOCKED));
        assert!(app.is_blocked("bad"));
        // one arriving over the gateway is dropped at the door
        assert!(!app.upsert_message(message_from("m1", "c1", "bad")));
        assert!(app.messages.get("c1").is_none_or(|m| m.is_empty()));
        // and a fetched page comes back without them
        app.set_channel_messages(
            "c1",
            vec![
                message_from("m2", "c1", "bad"),
                message_from("m3", "c1", "ok"),
            ],
        );
        let kept = app.messages.get("c1").expect("channel");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].author.id, "ok");
        // older history too
        app.prepend_channel_messages("c1", vec![message_from("m0", "c1", "bad")]);
        assert_eq!(app.messages.get("c1").expect("channel").len(), 1);
    }

    #[test]
    fn blocking_somebody_takes_what_they_already_said_out_of_the_pane() {
        let mut app = test_app(Vec::new());
        app.set_channel_messages(
            "c1",
            vec![
                message_from("m1", "c1", "bad"),
                message_from("m2", "c1", "ok"),
                message_from("m3", "c1", "bad"),
            ],
        );
        assert_eq!(app.messages.get("c1").expect("channel").len(), 3);
        app.upsert_relationship(relationship("bad", RELATIONSHIP_BLOCKED));
        app.forget_messages_from("bad");
        let kept = app.messages.get("c1").expect("channel");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].author.id, "ok");
    }

    #[test]
    fn the_four_groups_hold_the_right_people_and_a_nickname_wins() {
        let mut app = test_app(Vec::new());
        app.set_relationships(vec![
            relationship("u1", RELATIONSHIP_FRIEND),
            relationship("u2", RELATIONSHIP_INCOMING_REQUEST),
            relationship("u3", RELATIONSHIP_OUTGOING_REQUEST),
            relationship("u4", RELATIONSHIP_BLOCKED),
        ]);
        assert_eq!(app.relationships_in(FriendsTab::Friends).len(), 1);
        assert_eq!(app.relationships_in(FriendsTab::Incoming).len(), 1);
        assert_eq!(app.relationships_in(FriendsTab::Outgoing).len(), 1);
        assert_eq!(app.relationships_in(FriendsTab::Blocked).len(), 1);
        assert!(app.is_blocked("u4"));
        assert!(!app.is_blocked("u1"));
        // a name the reader gave wins over the account's own
        let mut named = relationship("u1", RELATIONSHIP_FRIEND);
        named.nickname = Some("Ada L".to_string());
        app.upsert_relationship(named);
        assert_eq!(app.relationship_nickname("u1"), Some("Ada L"));
        // and an empty one counts as none
        let mut blank = relationship("u1", RELATIONSHIP_FRIEND);
        blank.nickname = Some("   ".to_string());
        app.upsert_relationship(blank);
        assert_eq!(app.relationship_nickname("u1"), None);
    }

    #[test]
    fn the_group_tabs_go_round_both_ways() {
        let mut app = test_app(Vec::new());
        app.open_friends();
        assert_eq!(
            app.friends.as_ref().map(|v| v.tab),
            Some(FriendsTab::Friends)
        );
        for _ in 0..4 {
            app.friends_switch_tab(true);
        }
        assert_eq!(
            app.friends.as_ref().map(|v| v.tab),
            Some(FriendsTab::Friends)
        );
        app.friends_switch_tab(false);
        assert_eq!(
            app.friends.as_ref().map(|v| v.tab),
            Some(FriendsTab::Blocked)
        );
    }

    fn two_guild_app() -> App {
        let mut app = test_app(vec![ChannelResponse {
            id: "dm1".to_string(),
            kind: CHANNEL_DM,
            ..Default::default()
        }]);
        for (id, name) in [("g1", "One"), ("g2", "Two")] {
            app.guilds.push(GuildResponse {
                id: id.to_string(),
                name: name.to_string(),
                ..Default::default()
            });
            app.guild_channels.insert(
                id.to_string(),
                vec![ChannelResponse {
                    id: format!("{id}-general"),
                    kind: CHANNEL_GUILD_TEXT,
                    guild_id: Some(id.to_string()),
                    name: "general".to_string(),
                    ..Default::default()
                }],
            );
        }
        app
    }

    #[test]
    fn slot_one_is_the_conversations_and_the_rest_are_communities_in_order() {
        let mut app = two_guild_app();
        assert!(app.go_to_server_slot(2));
        assert_eq!(
            app.selected_server,
            ServerSelection::Guild("g1".to_string())
        );
        assert!(app.go_to_server_slot(3));
        assert_eq!(
            app.selected_server,
            ServerSelection::Guild("g2".to_string())
        );
        assert!(app.go_to_server_slot(1));
        assert_eq!(app.selected_server, ServerSelection::DirectMessages);
        // a slot past the end does nothing rather than moving somewhere
        assert!(!app.go_to_server_slot(4));
        assert_eq!(app.selected_server, ServerSelection::DirectMessages);
        assert!(!app.go_to_server_slot(0));
    }

    #[test]
    fn stepping_the_server_column_wraps_both_ways() {
        let mut app = two_guild_app();
        assert_eq!(app.selected_server, ServerSelection::DirectMessages);
        app.step_server(1);
        assert_eq!(
            app.selected_server,
            ServerSelection::Guild("g1".to_string())
        );
        app.step_server(1);
        assert_eq!(
            app.selected_server,
            ServerSelection::Guild("g2".to_string())
        );
        app.step_server(1);
        assert_eq!(app.selected_server, ServerSelection::DirectMessages);
        app.step_server(-1);
        assert_eq!(
            app.selected_server,
            ServerSelection::Guild("g2".to_string())
        );
    }

    #[test]
    fn the_toggle_goes_back_to_the_community_last_read() {
        let mut app = two_guild_app();
        // with nowhere to go back to it says so rather than moving
        assert!(!app.toggle_guild_and_dms());
        app.go_to_server_slot(3);
        app.selected_channel_id = Some("g2-general".to_string());
        app.note_active_channel();
        assert!(app.toggle_guild_and_dms());
        assert_eq!(app.selected_server, ServerSelection::DirectMessages);
        assert!(app.toggle_guild_and_dms());
        assert_eq!(
            app.selected_server,
            ServerSelection::Guild("g2".to_string())
        );
    }

    #[test]
    fn the_history_walks_back_and_forward_and_a_new_place_cuts_the_rest_off() {
        let mut app = two_guild_app();
        let visit = |app: &mut App, server: ServerSelection, channel: &str| {
            app.selected_server = server;
            app.selected_channel_id = Some(channel.to_string());
            app.note_active_channel();
        };
        visit(&mut app, ServerSelection::DirectMessages, "dm1");
        visit(
            &mut app,
            ServerSelection::Guild("g1".to_string()),
            "g1-general",
        );
        visit(
            &mut app,
            ServerSelection::Guild("g2".to_string()),
            "g2-general",
        );
        assert_eq!(app.channel_history.len(), 3);

        assert!(app.can_step_history(true));
        assert!(!app.can_step_history(false));
        assert!(app.step_channel_history(true));
        assert_eq!(app.active_channel_id(), Some("g1-general".to_string()));
        // walking is not itself a visit, so forward is still there
        app.note_active_channel();
        assert!(app.can_step_history(false));
        assert!(app.step_channel_history(false));
        assert_eq!(app.active_channel_id(), Some("g2-general".to_string()));

        // going somewhere new from the middle throws the forward half away
        app.step_channel_history(true);
        app.note_active_channel();
        visit(&mut app, ServerSelection::DirectMessages, "dm1");
        assert!(!app.can_step_history(false));
    }

    #[test]
    fn the_history_does_not_record_standing_still() {
        let mut app = two_guild_app();
        app.selected_channel_id = Some("dm1".to_string());
        for _ in 0..5 {
            app.note_active_channel();
        }
        assert_eq!(app.channel_history.len(), 1);
    }

    #[test]
    fn the_new_messages_line_sits_after_the_last_message_read() {
        let mut app = test_app(vec![ChannelResponse {
            id: "dm1".to_string(),
            kind: CHANNEL_DM,
            last_message_id: Some("40".to_string()),
            ..Default::default()
        }]);
        app.selected_channel_id = Some("dm1".to_string());
        app.set_channel_messages(
            "dm1",
            vec![
                dm_message("10", "dm1"),
                dm_message("20", "dm1"),
                dm_message("30", "dm1"),
                dm_message("40", "dm1"),
            ],
        );
        app.read_states.insert(
            "dm1".to_string(),
            ReadState {
                last_message_id: Some("20".to_string()),
                mention_count: 0,
            },
        );
        app.set_unread_anchor("dm1");
        // the line goes above the oldest message that arrived after it
        assert_eq!(app.first_unread_message_id("dm1"), Some("30".to_string()));
        assert_eq!(app.active_first_unread_message_id(), Some("30".to_string()));
    }

    #[test]
    fn a_channel_with_nothing_unread_gets_no_line() {
        let mut app = test_app(vec![ChannelResponse {
            id: "dm1".to_string(),
            kind: CHANNEL_DM,
            last_message_id: Some("20".to_string()),
            ..Default::default()
        }]);
        app.selected_channel_id = Some("dm1".to_string());
        app.set_channel_messages(
            "dm1",
            vec![dm_message("10", "dm1"), dm_message("20", "dm1")],
        );
        app.read_states.insert(
            "dm1".to_string(),
            ReadState {
                last_message_id: Some("20".to_string()),
                mention_count: 0,
            },
        );
        app.set_unread_anchor("dm1");
        assert_eq!(app.first_unread_message_id("dm1"), None);
    }

    #[test]
    fn a_channel_never_read_before_gets_no_line_either() {
        // a rule above the whole history says nothing worth a row
        let mut app = test_app(vec![ChannelResponse {
            id: "dm1".to_string(),
            kind: CHANNEL_DM,
            last_message_id: Some("20".to_string()),
            ..Default::default()
        }]);
        app.selected_channel_id = Some("dm1".to_string());
        app.set_channel_messages(
            "dm1",
            vec![dm_message("10", "dm1"), dm_message("20", "dm1")],
        );
        app.read_states.insert(
            "dm1".to_string(),
            ReadState {
                last_message_id: None,
                mention_count: 0,
            },
        );
        app.set_unread_anchor("dm1");
        assert_eq!(app.first_unread_message_id("dm1"), None);
    }

    #[test]
    fn the_line_stays_put_while_the_reader_is_in_the_channel() {
        let mut app = test_app(vec![ChannelResponse {
            id: "dm1".to_string(),
            kind: CHANNEL_DM,
            last_message_id: Some("40".to_string()),
            ..Default::default()
        }]);
        app.selected_channel_id = Some("dm1".to_string());
        app.set_channel_messages(
            "dm1",
            vec![dm_message("30", "dm1"), dm_message("40", "dm1")],
        );
        app.read_states.insert(
            "dm1".to_string(),
            ReadState {
                last_message_id: Some("20".to_string()),
                mention_count: 0,
            },
        );
        app.set_unread_anchor("dm1");
        assert_eq!(app.first_unread_message_id("dm1"), Some("30".to_string()));
        // the client acks as the reader looks, which moves the read state;
        // the line must not move with it
        app.read_states.insert(
            "dm1".to_string(),
            ReadState {
                last_message_id: Some("40".to_string()),
                mention_count: 0,
            },
        );
        assert_eq!(app.first_unread_message_id("dm1"), Some("30".to_string()));
        // and it goes once they are at the bottom with nothing unread
        app.message_scroll_from_bottom = 0;
        app.clear_unread_anchor_if_caught_up();
        assert_eq!(app.first_unread_message_id("dm1"), None);
    }

    #[test]
    fn u_puts_the_cursor_on_the_first_unread_message() {
        let mut app = test_app(vec![ChannelResponse {
            id: "dm1".to_string(),
            kind: CHANNEL_DM,
            last_message_id: Some("40".to_string()),
            ..Default::default()
        }]);
        app.selected_channel_id = Some("dm1".to_string());
        app.set_channel_messages(
            "dm1",
            vec![
                dm_message("10", "dm1"),
                dm_message("20", "dm1"),
                dm_message("30", "dm1"),
                dm_message("40", "dm1"),
            ],
        );
        app.read_states.insert(
            "dm1".to_string(),
            ReadState {
                last_message_id: Some("20".to_string()),
                mention_count: 0,
            },
        );
        app.set_unread_anchor("dm1");
        assert!(app.jump_to_first_unread());
        assert_eq!(app.selected_message_index, Some(2));
        assert_eq!(app.focus, Focus::Messages);
        // with no line there is nothing to jump to, and it says so
        app.clear_unread_anchor("dm1");
        assert!(!app.jump_to_first_unread());
    }

    fn dm_message(id: &str, channel_id: &str) -> MessageResponse {
        MessageResponse {
            id: id.to_string(),
            channel_id: channel_id.to_string(),
            author: UserPartialResponse {
                id: "other".to_string(),
                ..Default::default()
            },
            content: format!("message {id}"),
            ..Default::default()
        }
    }

    #[test]
    fn a_gateway_message_does_not_count_as_loaded_history() {
        let channel = ChannelResponse {
            id: "dm-1".to_string(),
            kind: CHANNEL_DM,
            ..Default::default()
        };
        let mut app = test_app(vec![channel]);
        // a message arrives for a channel that was never opened
        assert!(app.upsert_message(dm_message("300", "dm-1")));
        assert!(app.messages.contains_key("dm-1"));
        assert!(
            !app.messages_loaded.contains("dm-1"),
            "the history must still be fetched when the channel is opened"
        );

        // the fetch answers with the history; the newer gateway message stays
        app.set_channel_messages(
            "dm-1",
            vec![dm_message("200", "dm-1"), dm_message("100", "dm-1")],
        );
        assert!(app.messages_loaded.contains("dm-1"));
        let ids: Vec<&str> = app.messages["dm-1"].iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["100", "200", "300"]);

        // a fetch that already has it does not double it
        app.set_channel_messages(
            "dm-1",
            vec![
                dm_message("300", "dm-1"),
                dm_message("200", "dm-1"),
                dm_message("100", "dm-1"),
            ],
        );
        let ids: Vec<&str> = app.messages["dm-1"].iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["100", "200", "300"]);
    }

    #[test]
    fn channel_last_message_id_prefers_newer_channel_metadata() {
        let channel = ChannelResponse {
            id: "dm-1".to_string(),
            kind: CHANNEL_DM,
            last_message_id: Some("300".to_string()),
            ..ChannelResponse::default()
        };
        let mut app = test_app(vec![channel]);
        app.messages.insert(
            "dm-1".to_string(),
            std::rc::Rc::new(vec![
                MessageResponse {
                    id: "100".to_string(),
                    channel_id: "dm-1".to_string(),
                    ..MessageResponse::default()
                },
                MessageResponse {
                    id: "250".to_string(),
                    channel_id: "dm-1".to_string(),
                    ..MessageResponse::default()
                },
            ]),
        );

        assert_eq!(app.channel_last_message_id("dm-1").as_deref(), Some("300"));
    }

    #[test]
    fn ack_channel_uses_channel_metadata_without_cached_messages() {
        let channel = ChannelResponse {
            id: "dm-1".to_string(),
            kind: CHANNEL_DM,
            last_message_id: Some("400".to_string()),
            ..ChannelResponse::default()
        };
        let mut app = test_app(vec![channel]);

        app.ack_channel("dm-1");

        assert_eq!(
            app.read_states
                .get("dm-1")
                .and_then(|state| state.last_message_id.as_deref()),
            Some("400")
        );
        assert_eq!(app.channel_mention_count("dm-1"), 0);
    }

    #[test]
    fn auto_load_history_only_near_top_when_enabled() {
        let mut app = test_app(Vec::new());
        app.message_scroll_max = 24;

        app.message_scroll_from_bottom = 20;
        assert!(!app.should_auto_load_history_on_scroll_up());

        app.message_scroll_from_bottom = 21;
        assert!(app.should_auto_load_history_on_scroll_up());
    }

    #[test]
    fn auto_load_history_uses_scroll_threshold() {
        let mut app = test_app(Vec::new());
        app.message_scroll_max = 24;
        app.message_scroll_from_bottom = 18;

        assert!(!app.should_auto_load_history_on_scroll_up());
    }

    #[test]
    fn server_aggregates_unread_channels_and_mentions() {
        let mut app = test_app(Vec::new());
        app.guilds.push(GuildResponse {
            id: "guild-1".to_string(),
            name: "Guild".to_string(),
            ..GuildResponse::default()
        });
        app.guild_channels.insert(
            "guild-1".to_string(),
            vec![
                ChannelResponse {
                    id: "chan-1".to_string(),
                    guild_id: Some("guild-1".to_string()),
                    name: "alpha".to_string(),
                    kind: CHANNEL_GUILD_TEXT,
                    last_message_id: Some("200".to_string()),
                    ..ChannelResponse::default()
                },
                ChannelResponse {
                    id: "chan-2".to_string(),
                    guild_id: Some("guild-1".to_string()),
                    name: "beta".to_string(),
                    kind: CHANNEL_GUILD_TEXT,
                    last_message_id: Some("300".to_string()),
                    ..ChannelResponse::default()
                },
            ],
        );
        app.read_states.insert(
            "chan-1".to_string(),
            ReadState {
                last_message_id: Some("150".to_string()),
                mention_count: 0,
            },
        );
        app.read_states.insert(
            "chan-2".to_string(),
            ReadState {
                last_message_id: Some("300".to_string()),
                mention_count: 2,
            },
        );

        let server = ServerSelection::Guild("guild-1".to_string());
        assert_eq!(app.server_unread_channel_count(&server), 1);
        assert_eq!(app.server_mention_count(&server), 2);
    }

    #[test]
    fn muted_server_hides_unread_only_activity() {
        let mut app = guild_app(vec![
            ChannelResponse {
                id: "chan-1".to_string(),
                guild_id: Some("guild-1".to_string()),
                name: "alpha".to_string(),
                kind: CHANNEL_GUILD_TEXT,
                position: 1,
                last_message_id: Some("200".to_string()),
                ..ChannelResponse::default()
            },
            ChannelResponse {
                id: "chan-2".to_string(),
                guild_id: Some("guild-1".to_string()),
                name: "beta".to_string(),
                kind: CHANNEL_GUILD_TEXT,
                position: 2,
                last_message_id: Some("300".to_string()),
                ..ChannelResponse::default()
            },
        ]);
        app.read_states.insert(
            "chan-1".to_string(),
            ReadState {
                last_message_id: Some("150".to_string()),
                mention_count: 0,
            },
        );
        app.read_states.insert(
            "chan-2".to_string(),
            ReadState {
                last_message_id: Some("300".to_string()),
                mention_count: 2,
            },
        );
        app.upsert_user_guild_settings(UserGuildSettingsResponse {
            guild_id: Some("guild-1".to_string()),
            muted: true,
            ..UserGuildSettingsResponse::default()
        });

        let server = ServerSelection::Guild("guild-1".to_string());
        assert_eq!(app.server_unread_channel_count(&server), 0);
        assert_eq!(app.server_mention_count(&server), 2);
    }

    #[test]
    fn hide_muted_channels_filters_sidebar_entries() {
        let mut app = guild_app(vec![
            ChannelResponse {
                id: "chan-1".to_string(),
                guild_id: Some("guild-1".to_string()),
                name: "alpha".to_string(),
                kind: CHANNEL_GUILD_TEXT,
                position: 1,
                ..ChannelResponse::default()
            },
            ChannelResponse {
                id: "chan-2".to_string(),
                guild_id: Some("guild-1".to_string()),
                name: "beta".to_string(),
                kind: CHANNEL_GUILD_TEXT,
                position: 2,
                ..ChannelResponse::default()
            },
        ]);
        app.selected_channel_id = Some("chan-2".to_string());
        app.upsert_user_guild_settings(UserGuildSettingsResponse {
            guild_id: Some("guild-1".to_string()),
            hide_muted_channels: true,
            channel_overrides: HashMap::from([(
                "chan-1".to_string(),
                UserGuildChannelOverride {
                    muted: true,
                    ..UserGuildChannelOverride::default()
                },
            )]),
            ..UserGuildSettingsResponse::default()
        });

        let visible = app.channels_for_server(&ServerSelection::Guild("guild-1".to_string()));
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "chan-2");
    }

    #[test]
    fn suppress_everyone_blocks_local_mention_increment() {
        let mut app = guild_app(vec![ChannelResponse {
            id: "chan-1".to_string(),
            guild_id: Some("guild-1".to_string()),
            name: "alpha".to_string(),
            kind: CHANNEL_GUILD_TEXT,
            position: 1,
            last_message_id: Some("200".to_string()),
            ..ChannelResponse::default()
        }]);
        app.upsert_user_guild_settings(UserGuildSettingsResponse {
            guild_id: Some("guild-1".to_string()),
            suppress_everyone: true,
            ..UserGuildSettingsResponse::default()
        });
        app.read_states.insert(
            "chan-1".to_string(),
            ReadState {
                last_message_id: Some("150".to_string()),
                mention_count: 0,
            },
        );

        app.on_gateway_message_create(&MessageResponse {
            id: "250".to_string(),
            channel_id: "chan-1".to_string(),
            mention_everyone: true,
            author: UserPartialResponse {
                id: "other".to_string(),
                ..UserPartialResponse::default()
            },
            ..MessageResponse::default()
        });

        assert_eq!(app.channel_mention_count("chan-1"), 0);
    }
}

/// A `<:name:id>` / `<a:name:id>` token at the start of `s`.
#[derive(Debug, PartialEq, Eq)]
pub struct CustomEmojiToken<'a> {
    pub name: &'a str,
    pub id: &'a str,
    pub animated: bool,
    /// Byte length of the whole token including the angle brackets.
    pub len: usize,
}

pub fn parse_custom_emoji_token(s: &str) -> Option<CustomEmojiToken<'_>> {
    let body = s.strip_prefix('<')?;
    let (animated, body) = match body.strip_prefix("a:") {
        Some(rest) => (true, rest),
        None => (false, body.strip_prefix(':')?),
    };
    let close = body.find('>')?;
    let inner = &body[..close];
    let (name, id) = inner.rsplit_once(':')?;
    if name.is_empty()
        || id.is_empty()
        || !id.bytes().all(|b| b.is_ascii_digit())
        || name.contains(|c: char| c.is_whitespace() || c == '<' || c == ':')
    {
        return None;
    }
    Some(CustomEmojiToken {
        name,
        id,
        animated,
        len: 1 + if animated { 2 } else { 1 } + close + 1,
    })
}

/// Placeholder cells carry their slot number in the underline colour, which
/// survives Paragraph wrapping and is never shown: the backend takes every
/// marker off again before the cell is written, see [`is_marker_underline`].
pub fn custom_emoji_marker_style(slot: usize) -> Style {
    let k = slot.min(u16::MAX as usize) as u16;
    Style::default().underline_color(Color::Rgb(0xEE, (k >> 8) as u8, k as u8))
}

pub fn custom_emoji_marker_slot(style: Style) -> Option<usize> {
    match style.underline_color {
        Some(Color::Rgb(0xEE, hi, lo)) => Some(((hi as usize) << 8) | lo as usize),
        _ => None,
    }
}

/// Media block cells carry their row within the block in the red channel
/// (0xD0 + row) and the slot in green and blue. Four bits hold the row, so
/// a block is at most [`crate::media::BLOCK_MAX_ROWS`] rows tall; beyond
/// that every row would say 15 and the overlay would draw nothing.
pub fn media_marker_style(slot: usize, row: u16) -> Style {
    let k = slot.min(u16::MAX as usize) as u16;
    Style::default().underline_color(Color::Rgb(
        0xD0 | (row.min(15) as u8),
        (k >> 8) as u8,
        k as u8,
    ))
}

pub fn media_marker(style: Style) -> Option<(usize, u16)> {
    match style.underline_color {
        Some(Color::Rgb(r, hi, lo)) if r & 0xF0 == 0xD0 => {
            Some((((hi as usize) << 8) | lo as usize, (r & 0x0F) as u16))
        }
        _ => None,
    }
}

/// The cell a terminal picture is printed at: the backend prints the
/// picture there instead of the cell. The frames' serial and the frame
/// index sit in the underline colour (red 0x80..0xBF), so the cell changes
/// exactly when the picture does: a still picture is sent once, an
/// animation once per frame.
pub fn picture_sentinel_style(style: Style, serial: u16, frame: usize) -> Style {
    style.underline_color(Color::Rgb(
        0x80 | (frame as u8 & 0x3F),
        (serial >> 8) as u8,
        serial as u8,
    ))
}

/// Which picture a sentinel belongs to, if it is one. The frame index is
/// not part of it: the next frame of the same picture is the same picture.
pub fn picture_serial(underline_color: Color) -> Option<u16> {
    match underline_color {
        Color::Rgb(r, hi, lo) if r & 0xC0 == 0x80 => Some((u16::from(hi) << 8) | u16::from(lo)),
        _ => None,
    }
}

pub fn is_picture_sentinel(underline_color: Color) -> bool {
    matches!(underline_color, Color::Rgb(r, _, _) if r & 0xC0 == 0x80)
}

/// Whether an underline colour is one of the markers above rather than a
/// colour to show. They are a side channel within the buffer -- the panes
/// write them, the overlay and the backend read them -- and no terminal is
/// ever meant to be told about one: crossterm sends an underline colour as
/// `ESC[58;2;r;g;b m`, and a terminal without SGR 58 (xterm has no case for
/// it at all) skips the 58 and runs the rest as ordinary parameters, so the
/// green byte's 0 resets every attribute and a slot number in 40..=47 turns
/// into a background colour that stays on every cell written after it.
pub fn is_marker_underline(underline_color: Color) -> bool {
    matches!(underline_color, Color::Rgb(r, _, _)
        if r & 0xC0 == 0x80 || r & 0xF0 == 0xD0 || r == 0xEE)
}

#[cfg(test)]
mod custom_emoji_tests {
    use super::*;

    #[test]
    fn marker_style_round_trips_slot_numbers() {
        for k in [0usize, 1, 7, 255, 256, 4095, 65535] {
            assert_eq!(
                custom_emoji_marker_slot(custom_emoji_marker_style(k)),
                Some(k)
            );
        }
        assert_eq!(custom_emoji_marker_slot(Style::default()), None);
        assert_eq!(
            custom_emoji_marker_slot(Style::default().underline_color(Color::Rgb(1, 2, 3))),
            None
        );
    }

    #[test]
    fn a_sixel_comes_apart_into_bands_and_goes_back_together() {
        use ratatui_image::picker::{Picker, ProtocolType};
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        let mut img = image::RgbaImage::from_pixel(40, 36, image::Rgba([200, 30, 30, 255]));
        for y in 18..36 {
            for x in 0..40 {
                img.put_pixel(x, y, image::Rgba([30, 30, 200, 255]));
            }
        }
        let protocol = picker
            .new_protocol(
                image::DynamicImage::ImageRgba8(img),
                Rect::new(0, 0, 4, 2),
                ratatui_image::Resize::Fit(None),
            )
            .unwrap();
        let original = match &protocol {
            Protocol::Sixel(s) => s.data.clone(),
            _ => panic!("sixel"),
        };
        let picture = terminal_picture(&protocol, None, None, None).unwrap();
        let TerminalPicture::Sixel { bands, .. } = &picture else {
            panic!("kept in bands");
        };
        assert_eq!(bands.len(), 6, "36 px = 6 bands");
        // the whole block prints exactly what the encoder produced
        let whole = picture.printout(0, 2, 20, None).unwrap();
        assert_eq!(whole.rows.len(), 1);
        assert_eq!(&*whole.rows[0].1, original);
        // the second row alone: whole bands from 20 px on (24..36), 12 px tall
        let lower = picture.printout(1, 2, 20, None).unwrap();
        assert_eq!(lower.rows[0].0, 1);
        let data = &*lower.rows[0].1;
        assert!(data.contains("\"1;1;40;12"), "{data:?}");
        assert_eq!(data.matches('-').count(), 1, "two bands: {data:?}");
        assert!(data.ends_with("\x1b\\"));
        // the first row alone: bands 0..3, 18 px, and nothing past the row
        let upper = picture.printout(0, 1, 20, None).unwrap();
        assert!(
            upper.rows[0].1.contains("\"1;1;40;18"),
            "{:?}",
            upper.rows[0].1
        );
        assert!(picture.printout(1, 1, 20, None).is_none());
        assert!(picture.printout(0, 3, 20, None).is_none());
    }

    #[test]
    fn a_kitty_picture_prints_any_run_of_rows_after_one_transmit() {
        use ratatui_image::picker::{Picker, ProtocolType};
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Kitty);
        let img = image::RgbaImage::from_pixel(40, 60, image::Rgba([1, 2, 3, 255]));
        let protocol = picker
            .new_protocol(
                image::DynamicImage::ImageRgba8(img),
                Rect::new(0, 0, 4, 3),
                ratatui_image::Resize::Fit(None),
            )
            .unwrap();
        let picture = terminal_picture(&protocol, None, None, None).unwrap();
        let TerminalPicture::Kitty { transmit, rows, .. } = &picture else {
            panic!("kitty keeps rows");
        };
        assert_eq!(rows.len(), 3);
        assert!(transmit.starts_with("\x1b_G"), "{transmit:?}");
        assert!(
            rows.iter().all(|r| r.starts_with("\x1b[s")),
            "placeholders only"
        );
        let middle = picture.printout(1, 3, 20, None).unwrap();
        assert_eq!(middle.rows.iter().map(|r| r.0).collect::<Vec<_>>(), [1, 2]);
        assert!(middle.transmit.is_some());
    }

    #[test]
    fn a_sixel_run_cut_at_the_top_is_encoded_from_the_rows_edge() {
        use ratatui_image::picker::{Picker, ProtocolType};
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        // red on top, blue from pixel row 20 (the second cell row) down
        let mut img = image::RgbaImage::from_pixel(40, 36, image::Rgba([200, 30, 30, 255]));
        for y in 20..36 {
            for x in 0..40 {
                img.put_pixel(x, y, image::Rgba([30, 30, 200, 255]));
            }
        }
        let pixels = std::sync::Arc::new(img.clone());
        let protocol = picker
            .new_protocol(
                image::DynamicImage::ImageRgba8(img),
                Rect::new(0, 0, 4, 2),
                ratatui_image::Resize::Fit(None),
            )
            .unwrap();
        let picture = terminal_picture(&protocol, Some(pixels), None, None).unwrap();
        // the second row alone starts at pixel row 20: 16 rows left, cut to
        // two whole bands, all blue (no red band from above)
        let lower = picture.printout(1, 2, 20, Some(&picker)).unwrap();
        assert_eq!(lower.rows[0].0, 1);
        let data = &*lower.rows[0].1;
        assert!(data.contains("\"1;1;40;12"), "{data:?}");
        let blue_only = picker
            .new_protocol(
                image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                    40,
                    12,
                    image::Rgba([30, 30, 200, 255]),
                )),
                Rect::new(0, 0, 4, 1),
                ratatui_image::Resize::Fit(None),
            )
            .unwrap();
        let Protocol::Sixel(blue) = &blue_only else {
            panic!("sixel");
        };
        assert_eq!(data, blue.data.as_str(), "the run is the blue rows alone");
        // asked again, the same encoding comes back without more work
        let again = picture.printout(1, 2, 20, Some(&picker)).unwrap();
        assert!(std::sync::Arc::ptr_eq(&again.rows[0].1, &lower.rows[0].1));
        // from the top, the bands are still used, and without a picker the
        // band method stands in for a cut
        let whole = picture.printout(0, 2, 20, Some(&picker)).unwrap();
        assert!(whole.rows[0].1.contains("\"1;1;40;36"));
        let bands = picture.printout(1, 2, 20, None).unwrap();
        assert!(bands.rows[0].1.contains("\"1;1;40;12"));
    }

    #[test]
    fn picture_sentinels_are_not_markers() {
        let s = picture_sentinel_style(Style::default(), 0x1234, 47);
        assert!(is_picture_sentinel(s.underline_color.unwrap()));
        assert_eq!(s.underline_color, Some(Color::Rgb(0x80 | 47, 0x12, 0x34)));
        assert_eq!(media_marker(s), None);
        assert_eq!(custom_emoji_marker_slot(s), None);
        assert!(!is_picture_sentinel(
            media_marker_style(3, 2).underline_color.unwrap()
        ));
        assert!(!is_picture_sentinel(
            custom_emoji_marker_style(3).underline_color.unwrap()
        ));
        assert!(!is_picture_sentinel(Color::Reset));
    }

    #[test]
    fn media_marker_round_trips_slot_and_row() {
        for (k, r) in [(0usize, 0u16), (1, 1), (300, 11), (65535, 15)] {
            assert_eq!(media_marker(media_marker_style(k, r)), Some((k, r)));
        }
        assert_eq!(media_marker(Style::default()), None);
        assert_eq!(media_marker(custom_emoji_marker_style(3)), None);
        assert_eq!(custom_emoji_marker_slot(media_marker_style(3, 0)), None);
    }

    #[test]
    fn custom_emoji_tokens_parse() {
        let t = parse_custom_emoji_token("<:blob:123> tail").unwrap();
        assert_eq!(
            (t.name, t.id, t.animated, t.len),
            ("blob", "123", false, 11)
        );
        let t = parse_custom_emoji_token("<a:party:9>").unwrap();
        assert_eq!((t.name, t.id, t.animated, t.len), ("party", "9", true, 11));
        assert!(parse_custom_emoji_token("<:blob:abc>").is_none());
        assert!(parse_custom_emoji_token("<#123>").is_none());
        assert!(parse_custom_emoji_token("<:no close").is_none());
        assert!(parse_custom_emoji_token("<::1>").is_none());
    }

    #[test]
    fn animation_timeline_loops() {
        let f = PictureFrames::new(
            Vec::new(),
            vec![
                Duration::from_millis(100),
                Duration::from_millis(50),
                Duration::from_millis(150),
            ],
        );
        let t0 = Instant::now();
        let mut draw = 0u64;
        for (ms, want) in [(0u64, 0usize), (100, 1), (150, 2), (300, 0), (1000, 1)] {
            draw += 1;
            assert_eq!(
                f.index_at(t0 + Duration::from_millis(ms), draw),
                want,
                "at {ms} ms"
            );
        }
    }

    #[test]
    fn a_long_frame_is_held_and_never_shown_twice() {
        // three quick frames and a half-second pause: the pause is held
        // for as many draws as it takes, without stepping ahead and back
        let f = PictureFrames::new(
            Vec::new(),
            vec![
                Duration::from_millis(100),
                Duration::from_millis(100),
                Duration::from_millis(100),
                Duration::from_millis(500),
            ],
        );
        let t0 = Instant::now();
        let seen: Vec<usize> = (0..10u64)
            .map(|tick| f.index_at(t0 + Duration::from_millis(100 * tick), tick + 1))
            .collect();
        assert_eq!(seen, vec![0, 1, 2, 3, 3, 3, 3, 3, 0, 1]);
    }

    #[test]
    fn fast_loops_still_advance() {
        // two 50 ms frames: a 100 ms loop that a 100 ms tick would always
        // sample at the same frame
        let f = PictureFrames::new(
            Vec::new(),
            vec![Duration::from_millis(50), Duration::from_millis(50)],
        );
        let t0 = Instant::now();
        let seen: Vec<usize> = (0..6u64)
            .map(|tick| f.index_at(t0 + Duration::from_millis(100 * tick), tick + 1))
            .collect();
        assert_eq!(seen, vec![0, 1, 0, 1, 0, 1]);
        // redraws inside the same frame's time (typing, cursor blink) do not advance
        let now = t0 + Duration::from_millis(520);
        let a = f.index_at(now, 10);
        let b = f.index_at(now + Duration::from_millis(5), 11);
        assert_eq!(a, b);
    }

    #[test]
    fn irregular_draws_keep_the_cadence() {
        // 40 ms frames drawn every 50 ms: five frames go by in every four
        // draws, never the same frame twice in a row, never a frame skipped
        // twice in a row
        let f = PictureFrames::new(Vec::new(), vec![Duration::from_millis(40); 10]);
        let t0 = Instant::now();
        let seen: Vec<usize> = (0..8u64)
            .map(|tick| f.index_at(t0 + Duration::from_millis(50 * tick), tick + 1))
            .collect();
        assert_eq!(seen, vec![0, 1, 2, 3, 5, 6, 7, 8]);
    }

    #[test]
    fn several_instances_in_one_draw_share_a_frame() {
        let f = PictureFrames::new(
            Vec::new(),
            vec![Duration::from_millis(50), Duration::from_millis(50)],
        );
        let t0 = Instant::now();
        let mut per_draw = Vec::new();
        for draw in 1..=6u64 {
            let now = t0 + Duration::from_millis(100 * draw);
            // the same emoji drawn three times in this draw
            let a = f.index_at(now, draw);
            let b = f.index_at(now, draw);
            let c = f.index_at(now, draw);
            assert_eq!(b, a);
            assert_eq!(c, a);
            per_draw.push(a);
        }
        // and it still moves from draw to draw
        assert!(per_draw.windows(2).all(|w| w[0] != w[1]), "{per_draw:?}");
    }

    #[test]
    fn placeholder_is_two_cells_and_not_whitespace() {
        use unicode_width::UnicodeWidthStr;
        assert_eq!(
            CUSTOM_EMOJI_PLACEHOLDER.width(),
            CUSTOM_EMOJI_CELLS as usize
        );
        assert!(!CUSTOM_EMOJI_PLACEHOLDER.chars().any(char::is_whitespace));
    }
}

#[cfg(test)]
mod file_picker_tests {
    use super::*;

    fn app() -> App {
        let me = UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        };
        App::new(
            WellKnownFluxerResponse::default(),
            me,
            None,
            Vec::new(),
            Vec::new(),
            ServerSelection::DirectMessages,
            None,
            UiSettings::default(),
        )
    }

    fn names(app: &App) -> Vec<String> {
        let p = app.file_picker.as_ref().unwrap();
        p.filtered
            .iter()
            .map(|&i| p.entries[i].name.clone())
            .collect()
    }

    #[test]
    fn the_picker_lists_filters_descends_and_attaches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("photos")).unwrap();
        std::fs::write(dir.path().join("photos/cat.png"), b"x").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"hello").unwrap();
        std::fs::write(dir.path().join("archive.zip"), b"zz").unwrap();
        std::fs::write(dir.path().join(".hidden"), b"h").unwrap();
        let mut app = app();
        app.attach_dir = Some(dir.path().to_path_buf());
        app.open_file_picker();
        // directories first, then files by name; dotfiles hidden
        assert_eq!(names(&app), ["photos", "archive.zip", "notes.txt"]);
        // typing filters; a leading dot shows dotfiles
        app.file_picker.as_mut().unwrap().query = "not".into();
        app.filter_file_picker();
        assert_eq!(names(&app), ["notes.txt"]);
        app.file_picker.as_mut().unwrap().query = ".hid".into();
        app.filter_file_picker();
        assert_eq!(names(&app), [".hidden"]);
        app.file_picker.as_mut().unwrap().query.clear();
        app.filter_file_picker();
        // Enter on a directory descends, on a file attaches and closes
        app.file_picker_move(0);
        assert!(app.file_picker_confirm().is_none());
        assert_eq!(names(&app), ["cat.png"]);
        assert_eq!(
            app.file_picker.as_ref().unwrap().dir,
            dir.path().join("photos")
        );
        // back up: the cursor lands on the directory just left
        app.file_picker_parent();
        assert_eq!(app.file_picker.as_ref().unwrap().dir, dir.path());
        assert_eq!(
            app.file_picker.as_ref().unwrap().current().unwrap().name,
            "photos"
        );
        app.file_picker_move(10);
        assert_eq!(
            app.file_picker.as_ref().unwrap().current().unwrap().name,
            "notes.txt"
        );
        let picked = app.file_picker_confirm();
        assert_eq!(picked, Some(dir.path().join("notes.txt")));
        assert!(app.file_picker.is_none());
        // the picker remembers where it was
        assert_eq!(app.attach_dir.as_deref(), Some(dir.path()));
    }

    #[test]
    fn staged_pictures_get_thumbnails_where_pictures_can_be_drawn() {
        let mut app = app();
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(200, 100, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let shot = crate::media::StagedAttachment::new("shot.png".into(), "image/png".into(), png);
        let clip =
            crate::media::StagedAttachment::new("clip.mp4".into(), "video/mp4".into(), vec![0; 8]);
        let doc = crate::media::StagedAttachment::new(
            "doc.pdf".into(),
            "application/pdf".into(),
            vec![0; 8],
        );
        // a terminal without pictures: no thumbnails
        assert!(app.staged_thumbnail_slot(&shot).is_none());
        app.pixel_mode = true;
        app.cell_px = (10, 20);
        let s = app.staged_thumbnail_slot(&shot).unwrap();
        assert_eq!(
            (s.cols, s.rows),
            (16, 4),
            "a 2:1 picture fills the 16x4 box"
        );
        assert_eq!(s.url, crate::media::staged_url(shot.id));
        let v = app.staged_thumbnail_slot(&clip).unwrap();
        assert_eq!((v.cols, v.rows), (14, 4), "16:9 in a 16x4 box");
        assert!(app.staged_thumbnail_slot(&doc).is_none());
        app.pending_attachments = vec![shot.clone(), doc];
        match app.local_media_source(&crate::media::staged_url(shot.id)) {
            Some(crate::media::LocalSource::Bytes { filename, bytes }) => {
                assert_eq!(filename, "shot.png");
                assert_eq!(bytes, shot.bytes);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            app.local_media_source(&crate::media::staged_url(999))
                .is_none()
        );
        assert!(matches!(
            app.local_media_source("file:///tmp/x.png"),
            Some(crate::media::LocalSource::Path(_))
        ));
    }
}

#[cfg(test)]
mod performance_mode_tests {
    use super::*;
    use crate::api::types::{UserPrivateResponse, WellKnownFluxerResponse};
    use crate::config::UiSettings;

    fn app(performance_mode: bool) -> App {
        let mut ui = UiSettings::default();
        ui.performance_mode = performance_mode;
        let mut app = App::new(
            WellKnownFluxerResponse::default(),
            UserPrivateResponse {
                id: "me".to_string(),
                ..UserPrivateResponse::default()
            },
            None,
            Vec::new(),
            Vec::new(),
            ServerSelection::DirectMessages,
            None,
            ui,
        );
        // the console renderer: pictures are drawable
        app.pixel_mode = true;
        app
    }

    #[test]
    fn performance_mode_draws_no_pictures_and_ticks_twice_a_second() {
        let full = app(false);
        assert!(full.pictures_enabled());
        assert!(full.inline_media_enabled());
        assert!(full.avatars_enabled());
        assert_eq!(full.tick_period(), Duration::from_millis(100));

        let mut lean = app(true);
        assert!(!lean.pictures_enabled());
        assert!(!lean.inline_media_enabled());
        assert!(!lean.avatars_enabled());
        assert!(lean.custom_emoji_placeholder("123", false).is_none());
        // and nothing was put on the fetch list
        assert!(lean.take_custom_emoji_wants().is_empty());
        assert_eq!(lean.tick_period(), Duration::from_millis(500));
    }
}

#[cfg(test)]
mod notification_tests {
    use super::*;

    fn app() -> App {
        let me = UserPrivateResponse {
            id: "me".into(),
            username: "me".into(),
            ..Default::default()
        };
        let dm = ChannelResponse {
            id: "dm".into(),
            kind: 1,
            recipients: vec![UserPartialResponse {
                id: "ann".into(),
                username: "ann".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut app = App::new(
            WellKnownFluxerResponse::default(),
            me,
            None,
            vec![GuildResponse {
                id: "g1".into(),
                name: "Linux Hub".into(),
                ..Default::default()
            }],
            vec![dm],
            ServerSelection::Guild("g1".into()),
            Some("general".into()),
            UiSettings::default(),
        );
        app.guild_channels.insert(
            "g1".into(),
            vec![
                ChannelResponse {
                    id: "general".into(),
                    guild_id: Some("g1".into()),
                    name: "general".into(),
                    ..Default::default()
                },
                ChannelResponse {
                    id: "art".into(),
                    guild_id: Some("g1".into()),
                    name: "art".into(),
                    ..Default::default()
                },
            ],
        );
        app.selected_channel_id = Some("general".into());
        app
    }

    fn msg(channel: &str, author: &str, text: &str) -> MessageResponse {
        MessageResponse {
            id: "1".into(),
            channel_id: channel.into(),
            author: UserPartialResponse {
                id: author.into(),
                username: author.into(),
                ..Default::default()
            },
            content: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn direct_messages_and_mentions_are_announced_others_are_not() {
        let app = app();
        let dm = app.notification_for(&msg("dm", "ann", "hey you")).unwrap();
        assert_eq!(dm.title, "ann (direct message)");
        assert_eq!(dm.body, "hey you");
        assert_eq!(dm.place, "Direct message");

        let mut mention = msg("art", "ann", "look <@1234> at this");
        mention.mentions.push(UserPartialResponse {
            id: "me".into(),
            username: "me".into(),
            ..Default::default()
        });
        let n = app.notification_for(&mention).unwrap();
        assert_eq!(n.title, "ann in #art");
        assert_eq!(n.place, "#art \u{00B7} Linux Hub");
        assert!(
            n.body.starts_with("look @") && n.body.ends_with("at this"),
            "{}",
            n.body
        );

        assert!(
            app.notification_for(&msg("art", "ann", "no mention"))
                .is_none()
        );
        // a mention in the channel being read is on screen while the
        // window is focused, and counts once it is not; one's own messages
        // never do
        let mut here = msg("general", "ann", "<@me> here");
        here.mentions.push(UserPartialResponse {
            id: "me".into(),
            ..Default::default()
        });
        assert!(app.notification_for(&here).is_none());
        let mut away = app;
        away.window_focused = false;
        assert!(away.notification_for(&here).is_some());
        assert!(away.notification_for(&msg("dm", "me", "my own")).is_none());
    }

    #[test]
    fn all_messages_can_be_asked_for_and_files_are_named() {
        let mut app = app();
        app.ui_settings.notify_all_messages = true;
        // all messages: except in the channel being read, while it is
        // in sight
        assert!(
            app.notification_for(&msg("general", "ann", "chatter"))
                .is_none()
        );
        app.window_focused = false;
        assert!(
            app.notification_for(&msg("general", "ann", "chatter"))
                .is_some()
        );
        app.window_focused = true;
        assert!(
            app.notification_for(&msg("art", "ann", "chatter"))
                .is_some()
        );
        let mut m = msg("art", "ann", "");
        m.attachments
            .push(crate::api::types::MessageAttachmentResponse {
                filename: "cat.png".into(),
                ..Default::default()
            });
        let n = app.notification_for(&m).unwrap();
        assert_eq!(n.body, "[cat.png]");
        let long = "x".repeat(400);
        let n = app.notification_for(&msg("art", "ann", &long)).unwrap();
        assert_eq!(n.body.chars().count(), 301);
        assert!(n.body.ends_with('\u{2026}'));
    }
}

#[cfg(test)]
mod members_failure_status_tests {
    use super::*;
    use crate::api::client::MembersFailure;
    use crate::api::types::{GuildResponse, UserPrivateResponse, WellKnownFluxerResponse};
    use crate::config::UiSettings;

    fn app_with_guild() -> App {
        let mut app = App::new(
            WellKnownFluxerResponse::default(),
            UserPrivateResponse {
                id: "me".to_string(),
                ..UserPrivateResponse::default()
            },
            None,
            Vec::new(),
            Vec::new(),
            ServerSelection::DirectMessages,
            None,
            UiSettings::default(),
        );
        app.guilds.push(GuildResponse {
            id: "g1".to_string(),
            name: "Linux Hub".to_string(),
            ..GuildResponse::default()
        });
        app
    }

    #[test]
    fn the_status_names_the_community_and_says_what_still_works() {
        let app = app_with_guild();
        let s = app.members_failure_status("g1", MembersFailure::Unavailable, false, "504");
        assert!(
            s.starts_with("The member list of Linux Hub is unavailable"),
            "{s}"
        );
        assert!(s.contains("@mentions offer the members seen so far"), "{s}");
        assert!(s.contains("tried again in 3 min"), "{s}");

        let s = app.members_failure_status("g1", MembersFailure::Unavailable, true, "504");
        assert!(
            s.starts_with("Only part of the member list of Linux Hub"),
            "{s}"
        );

        let s = app.members_failure_status("g1", MembersFailure::Forbidden, false, "403");
        assert_eq!(
            s,
            "Linux Hub does not let you list its members; @mentions offer the members seen so far."
        );

        let s = app.members_failure_status("g1", MembersFailure::Other, false, "400 Bad Request");
        assert_eq!(
            s,
            "Could not load the member list of Linux Hub: 400 Bad Request"
        );

        let s = app.members_failure_status("unknown", MembersFailure::Forbidden, false, "");
        assert!(s.starts_with("this community does not let you"), "{s}");
    }
}

#[cfg(test)]
mod own_typing_tests {
    use super::*;

    fn secs(t0: Instant, s: f32) -> Instant {
        t0 + Duration::from_millis((s * 1000.0) as u64)
    }

    #[test]
    fn the_first_post_comes_after_a_second_and_a_half_of_typing() {
        let t0 = Instant::now();
        let mut own = OwnTyping::default();
        own.note("h", Some("c1"), true, t0);
        assert_eq!(own.due(secs(t0, 1.0)), None);
        own.note("he", Some("c1"), true, secs(t0, 1.2));
        assert_eq!(own.due(secs(t0, 1.5)).as_deref(), Some("c1"));
        own.sent("c1", secs(t0, 1.5));
        // nothing more without keys, and not before eight seconds with them
        assert_eq!(own.due(secs(t0, 5.0)), None);
        own.note("hel", Some("c1"), true, secs(t0, 5.0));
        assert_eq!(own.due(secs(t0, 9.4)), None);
        assert_eq!(own.due(secs(t0, 9.5)).as_deref(), Some("c1"));
        own.sent("c1", secs(t0, 9.5));
        // a key that changed nothing does not count as typing
        own.note("hel", Some("c1"), true, secs(t0, 12.0));
        assert_eq!(own.due(secs(t0, 18.0)), None);
    }

    #[test]
    fn ten_seconds_without_a_key_end_the_bout() {
        let t0 = Instant::now();
        let mut own = OwnTyping::default();
        own.note("h", Some("c1"), true, t0);
        assert_eq!(own.due(secs(t0, 10.0)), None);
        // the next key starts over, with the delay counted afresh
        own.note("hi", Some("c1"), true, secs(t0, 20.0));
        assert_eq!(own.due(secs(t0, 21.0)), None);
        assert_eq!(own.due(secs(t0, 21.5)).as_deref(), Some("c1"));
    }

    #[test]
    fn a_sent_message_ends_the_bout_and_the_spacing_holds_across_bouts() {
        let t0 = Instant::now();
        let mut own = OwnTyping::default();
        own.note("h", Some("c1"), true, t0);
        assert_eq!(own.due(secs(t0, 2.0)).as_deref(), Some("c1"));
        own.sent("c1", secs(t0, 2.0));
        own.note("", Some("c1"), true, secs(t0, 3.0));
        assert_eq!(own.due(secs(t0, 30.0)), None);
        own.note("a", Some("c1"), true, secs(t0, 4.0));
        assert_eq!(own.due(secs(t0, 6.0)), None);
        assert_eq!(own.due(secs(t0, 10.0)).as_deref(), Some("c1"));
        // another channel is not held back by it
        own.note("", Some("c1"), true, secs(t0, 10.0));
        own.note("b", Some("c2"), true, secs(t0, 10.0));
        assert_eq!(own.due(secs(t0, 11.5)).as_deref(), Some("c2"));
    }

    #[test]
    fn slash_commands_edits_and_the_setting_off_are_not_typing() {
        let t0 = Instant::now();
        let mut own = OwnTyping::default();
        own.note("/nick x", Some("c1"), true, t0);
        assert_eq!(own.due(secs(t0, 5.0)), None);
        own.note("hello", Some("c1"), false, secs(t0, 6.0));
        assert_eq!(own.due(secs(t0, 12.0)), None);
        own.note("hello!", Some("c1"), true, secs(t0, 13.0));
        assert_eq!(own.due(secs(t0, 14.5)).as_deref(), Some("c1"));
        own.sent("c1", secs(t0, 14.5));
        own.note("hello!", Some("c1"), false, secs(t0, 15.0));
        own.note("hello!!", Some("c1"), false, secs(t0, 15.5));
        assert_eq!(own.due(secs(t0, 30.0)), None);
    }

    #[test]
    fn a_draft_carried_to_another_channel_waits_for_a_key() {
        let t0 = Instant::now();
        let mut own = OwnTyping::default();
        own.note("h", Some("c1"), true, t0);
        own.note("h", Some("c2"), true, secs(t0, 0.5));
        assert_eq!(own.due(secs(t0, 5.0)), None);
        own.note("hi", Some("c2"), true, secs(t0, 6.0));
        assert_eq!(own.due(secs(t0, 7.5)).as_deref(), Some("c2"));
    }
}

#[cfg(test)]
mod pings_tests {
    use super::*;
    use crate::api::types::{CHANNEL_DM, CHANNEL_GUILD_TEXT, ChannelResponse, GuildResponse};

    fn ping(id: &str, channel_id: &str, content: &str) -> MessageResponse {
        MessageResponse {
            id: id.to_string(),
            channel_id: channel_id.to_string(),
            author: UserPartialResponse {
                id: "u2".to_string(),
                username: "ada".to_string(),
                ..Default::default()
            },
            content: content.to_string(),
            ..Default::default()
        }
    }

    fn app_with_places() -> App {
        let mut app = App::new(
            Default::default(),
            Default::default(),
            None,
            Vec::new(),
            Vec::new(),
            ServerSelection::DirectMessages,
            None,
            Default::default(),
        );
        app.guilds.push(GuildResponse {
            id: "g1".to_string(),
            name: "Lab".to_string(),
            ..Default::default()
        });
        app.guild_channels.insert(
            "g1".to_string(),
            vec![ChannelResponse {
                id: "c1".to_string(),
                guild_id: Some("g1".to_string()),
                name: "general".to_string(),
                kind: CHANNEL_GUILD_TEXT,
                ..Default::default()
            }],
        );
        app.private_channels.push(ChannelResponse {
            id: "dm1".to_string(),
            kind: CHANNEL_DM,
            recipients: vec![UserPartialResponse {
                id: "u2".to_string(),
                username: "ada".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        });
        app
    }

    #[test]
    fn a_ping_knows_its_community_and_channel() {
        let app = app_with_places();
        assert_eq!(
            app.ping_location(&ping("1", "c1", "hi")),
            (Some("Lab".to_string()), "general".to_string())
        );
        let (guild, name) = app.ping_location(&ping("2", "dm1", "hi"));
        assert_eq!(guild, None);
        assert!(name.contains("ada"), "{name}");
        let (_, gone) = app.ping_location(&ping("3", "zz9999", "hi"));
        assert_eq!(gone, "unknown-9999");
    }

    #[test]
    fn enter_goes_to_the_channel_and_selects_the_message_once_loaded() {
        let mut app = app_with_places();
        app.open_pings();
        assert!(matches!(
            app.pings.as_ref().map(|v| &v.state),
            Some(PingsState::Loading)
        ));
        app.set_pings_loaded(vec![ping("20", "c1", "second"), ping("10", "c1", "first")]);
        app.pings_move(1);
        assert_eq!(app.pings_selected().map(|m| m.id.as_str()), Some("10"));
        app.pings_move(5);
        assert_eq!(app.pings_selected().map(|m| m.id.as_str()), Some("10"));
        assert!(app.pings_jump());
        assert!(app.pings.is_none());
        assert_eq!(
            app.selected_server,
            ServerSelection::Guild("g1".to_string())
        );
        assert_eq!(app.selected_channel_id.as_deref(), Some("c1"));
        assert_eq!(app.focus, Focus::Messages);
        // the history is not there yet: the jump waits
        assert_eq!(app.pending_jump, Some(("c1".to_string(), "10".to_string())));
        assert_eq!(app.selected_message_index, None);
        app.set_channel_messages(
            "c1",
            vec![ping("10", "c1", "first"), ping("20", "c1", "second")],
        );
        app.apply_pending_jump("c1");
        assert_eq!(app.pending_jump, None);
        assert_eq!(app.selected_message_index, Some(0));
    }

    #[test]
    fn a_jump_past_the_loaded_history_fetches_older_pages_until_it_finds_the_message() {
        let mut app = app_with_places();
        app.open_pings();
        app.set_pings_loaded(vec![ping("5", "dm1", "old")]);
        app.set_channel_messages("dm1", vec![ping("10", "dm1", "newer")]);
        app.messages_older_exhausted.remove("dm1");
        assert!(app.pings_jump());
        assert_eq!(app.selected_server, ServerSelection::DirectMessages);
        // not there yet: the jump waits for older pages of this channel
        assert!(app.pending_jump.is_some());
        assert!(app.jump_wants_older());
        assert!(
            app.status_message.contains("older messages"),
            "{}",
            app.status_message
        );
        // a page without it keeps waiting
        app.prepend_channel_messages("dm1", vec![ping("7", "dm1", "between")]);
        app.older_page_for_jump("dm1", true);
        assert!(app.pending_jump.is_some());
        assert_eq!(app.pending_jump_pages, 1);
        // the page with it selects it
        app.prepend_channel_messages("dm1", vec![ping("5", "dm1", "old")]);
        app.older_page_for_jump("dm1", true);
        assert_eq!(app.pending_jump, None);
        assert_eq!(app.selected_message_index, Some(0));
    }

    #[test]
    fn a_jump_gives_up_at_the_channel_beginning_or_when_the_user_leaves() {
        let mut app = app_with_places();
        app.open_pings();
        app.set_pings_loaded(vec![ping("5", "dm1", "old"), ping("6", "c1", "x")]);
        // fewer than a page: the history is complete, the message is gone
        app.set_channel_messages("dm1", vec![ping("10", "dm1", "newer")]);
        assert!(app.pings_jump());
        assert_eq!(app.pending_jump, None);
        assert!(
            app.status_message.contains("not in the channel's history"),
            "{}",
            app.status_message
        );
        // a jump still waiting is dropped once the user goes elsewhere
        app.open_pings();
        app.set_pings_loaded(vec![ping("5", "dm1", "old")]);
        app.messages_older_exhausted.remove("dm1");
        assert!(app.pings_jump());
        assert!(app.pending_jump.is_some());
        app.selected_channel_id = Some("c1".to_string());
        assert!(!app.jump_wants_older());
        assert_eq!(app.pending_jump, None);
        // and a failed page ends it too
        app.selected_channel_id = Some("dm1".to_string());
        app.open_pings();
        app.set_pings_loaded(vec![ping("5", "dm1", "old")]);
        assert!(app.pings_jump());
        app.older_page_for_jump("dm1", false);
        assert_eq!(app.pending_jump, None);
    }

    #[test]
    fn dismissing_takes_pings_off_the_list_and_hands_back_their_ids() {
        let mut app = app_with_places();
        app.open_pings();
        app.set_pings_loaded(vec![
            ping("3", "c1", "c"),
            ping("2", "c1", "b"),
            ping("1", "c1", "a"),
        ]);
        app.pings_move(2);
        assert_eq!(app.pings_dismiss_selected().as_deref(), Some("1"));
        assert_eq!(app.pings_selected().map(|m| m.id.as_str()), Some("2"));
        assert_eq!(app.pings_take_all(), vec!["3".to_string(), "2".to_string()]);
        assert!(app.pings_messages().is_empty());
        assert_eq!(app.pings_dismiss_selected(), None);
        assert!(!app.pings_jump());
    }
}

#[cfg(test)]
mod copy_message_tests {
    use super::*;
    use crate::api::types::{CHANNEL_DM, ChannelResponse, MessageAttachmentResponse};

    fn attachment(url: &str) -> MessageAttachmentResponse {
        MessageAttachmentResponse {
            id: "a1".to_string(),
            filename: "shot.png".to_string(),
            url: Some(url.to_string()),
            ..MessageAttachmentResponse::default()
        }
    }

    #[test]
    fn copy_text_is_the_content_and_the_file_links() {
        let plain = MessageResponse {
            content: "hello there  \n".to_string(),
            ..MessageResponse::default()
        };
        assert_eq!(message_copy_text(&plain), "hello there");

        let with_file = MessageResponse {
            content: "look".to_string(),
            attachments: vec![attachment("https://cdn.example/shot.png")],
            ..MessageResponse::default()
        };
        assert_eq!(
            message_copy_text(&with_file),
            "look\nhttps://cdn.example/shot.png"
        );

        let file_only = MessageResponse {
            attachments: vec![attachment("https://cdn.example/shot.png")],
            ..MessageResponse::default()
        };
        assert_eq!(
            message_copy_text(&file_only),
            "https://cdn.example/shot.png"
        );

        assert!(message_copy_text(&MessageResponse::default()).is_empty());
    }

    #[test]
    fn copying_a_message_fills_the_cut_buffer() {
        let channel = ChannelResponse {
            id: "dm-1".to_string(),
            kind: CHANNEL_DM,
            ..ChannelResponse::default()
        };
        let mut app = App::new(
            Default::default(),
            Default::default(),
            None,
            Vec::new(),
            vec![channel],
            ServerSelection::DirectMessages,
            None,
            Default::default(),
        );
        app.selected_channel_id = Some("dm-1".to_string());
        app.messages.insert(
            "dm-1".to_string(),
            std::rc::Rc::new(vec![
                MessageResponse {
                    id: "1".to_string(),
                    channel_id: "dm-1".to_string(),
                    content: "first".to_string(),
                    ..MessageResponse::default()
                },
                MessageResponse {
                    id: "2".to_string(),
                    channel_id: "dm-1".to_string(),
                    content: "second".to_string(),
                    ..MessageResponse::default()
                },
            ]),
        );

        // Nothing selected: nothing to copy.
        assert!(app.copy_selected_message().is_none());
        assert!(app.cut_buffer.is_empty());

        app.selected_message_index = Some(0);
        assert!(app.copy_selected_message().is_some());
        assert_eq!(app.cut_buffer, "first");

        // A message with neither text nor files leaves the buffer alone.
        app.messages.insert(
            "dm-1".to_string(),
            std::rc::Rc::new(vec![MessageResponse {
                id: "3".to_string(),
                channel_id: "dm-1".to_string(),
                ..MessageResponse::default()
            }]),
        );
        assert!(app.copy_selected_message().is_none());
        assert_eq!(app.cut_buffer, "first");
    }
}

#[cfg(test)]
mod sticker_tests {
    use super::*;
    use crate::api::types::{ChannelResponse, GuildResponse};

    fn guild_app() -> App {
        let mut app = App::new(
            Default::default(),
            Default::default(),
            None,
            Vec::new(),
            Vec::new(),
            ServerSelection::Guild("guild-1".to_string()),
            None,
            Default::default(),
        );
        app.guilds.push(GuildResponse {
            id: "guild-1".to_string(),
            name: "Lab".to_string(),
            ..Default::default()
        });
        app.guild_channels
            .insert("guild-1".to_string(), Vec::<ChannelResponse>::new());
        app
    }

    fn guild_sticker(
        id: &str,
        name: &str,
        tags: &[&str],
    ) -> crate::api::types::GuildStickerResponse {
        crate::api::types::GuildStickerResponse {
            id: id.to_string(),
            name: name.to_string(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn the_sticker_picker_lists_the_active_community_first_and_filters_on_tags() {
        let mut app = guild_app();
        app.guilds.push(GuildResponse {
            id: "guild-2".to_string(),
            name: "Other".to_string(),
            ..GuildResponse::default()
        });
        app.set_guild_stickers("guild-2", vec![guild_sticker("20", "wave", &["hello"])]);
        app.set_guild_stickers(
            "guild-1",
            vec![
                guild_sticker("11", "shipit", &["deploy"]),
                guild_sticker("10", "party", &[]),
            ],
        );

        app.open_sticker_picker("");
        let picker = app.sticker_picker.as_ref().unwrap();
        let listed: Vec<&str> = picker
            .entries
            .iter()
            .map(|e| e.sticker.name.as_str())
            .collect();
        assert_eq!(listed, ["party", "shipit", "wave"]);
        assert_eq!(picker.filtered.len(), 3);

        // the filter reads names and tags alike
        app.sticker_picker.as_mut().unwrap().query = "deploy".to_string();
        app.filter_sticker_picker();
        let picker = app.sticker_picker.as_ref().unwrap();
        let matched: Vec<&str> = picker
            .filtered
            .iter()
            .map(|&i| picker.entries[i].sticker.name.as_str())
            .collect();
        assert_eq!(matched, ["shipit"]);
        assert_eq!(
            picker.current().map(|e| e.guild_id.as_str()),
            Some("guild-1")
        );
    }

    /// The names the filter keeps, in the order the picker lists them.
    fn matched(app: &App) -> Vec<String> {
        let picker = app.sticker_picker.as_ref().expect("the picker is open");
        picker
            .filtered
            .iter()
            .map(|&i| picker.entries[i].sticker.name.clone())
            .collect()
    }

    #[test]
    fn the_search_narrows_the_list_and_esc_puts_the_old_filter_back() {
        let mut app = guild_app();
        app.set_guild_stickers(
            "guild-1",
            vec![
                guild_sticker("1", "shipit", &["deploy"]),
                guild_sticker("2", "party", &[]),
                guild_sticker("3", "partycat", &["cat"]),
            ],
        );
        app.open_sticker_picker("party");
        assert_eq!(matched(&app), ["party", "partycat"]);
        assert!(
            !app.sticker_picker.as_ref().unwrap().searching,
            "the picker opens on the list, not in the search"
        );

        // `/` searches the whole list again
        app.sticker_picker_start_search();
        let picker = app.sticker_picker.as_ref().unwrap();
        assert!(picker.searching && picker.query.is_empty());
        assert_eq!(picker.filtered.len(), 3);

        // typing narrows it, and the cursor sits on the first match
        app.sticker_picker_move(2);
        for ch in "cat".chars() {
            app.sticker_picker_search_type(ch);
        }
        assert_eq!(matched(&app), ["partycat"]);
        assert_eq!(app.sticker_picker.as_ref().unwrap().selected, 0);

        // Backspace edits what was typed
        app.sticker_picker_search_erase(false);
        assert_eq!(app.sticker_picker.as_ref().unwrap().query, "ca");

        // Esc gives back the filter the search started from
        app.sticker_picker_end_search(false);
        let picker = app.sticker_picker.as_ref().unwrap();
        assert!(!picker.searching);
        assert_eq!(picker.query, "party");
        assert_eq!(matched(&app), ["party", "partycat"]);

        // Enter keeps what was typed instead
        app.sticker_picker_start_search();
        app.sticker_picker_search_type('s');
        app.sticker_picker_end_search(true);
        let picker = app.sticker_picker.as_ref().unwrap();
        assert!(!picker.searching);
        assert_eq!(picker.query, "s");
        assert_eq!(matched(&app), ["shipit"]);
        // and a later Esc closes the picker rather than restoring anything
        app.dismiss_sticker_picker();
        assert!(app.sticker_picker.is_none());
    }

    #[test]
    fn a_message_carries_three_stickers_and_never_the_same_one_twice() {
        let mut app = guild_app();
        app.set_guild_stickers(
            "guild-1",
            vec![
                guild_sticker("1", "one", &[]),
                guild_sticker("2", "two", &[]),
                guild_sticker("3", "three", &[]),
                guild_sticker("4", "four", &[]),
            ],
        );
        for name in ["one", "two", "three"] {
            app.open_sticker_picker(name);
            assert!(app.sticker_picker_confirm().is_some());
            assert!(app.sticker_picker.is_none(), "Enter closes the picker");
        }
        assert_eq!(app.pending_stickers.len(), 3);

        app.open_sticker_picker("one");
        let said = app.sticker_picker_confirm().unwrap();
        assert!(said.contains("already"), "{said}");
        assert_eq!(app.pending_stickers.len(), 3);

        app.open_sticker_picker("four");
        let said = app.sticker_picker_confirm().unwrap();
        assert!(said.contains("at most"), "{said}");
        assert_eq!(app.pending_stickers.len(), MAX_STICKERS_PER_MESSAGE);
        let ids: Vec<&str> = app.pending_stickers.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["1", "2", "3"]);
    }

    #[test]
    fn a_sticker_url_asks_for_a_size_of_the_sticker_class() {
        let app = guild_app();
        // the proxy clamps to 128..=512; ask inside that range
        assert!(
            app.sticker_url("42", false, 10)
                .ends_with("/stickers/42.webp?size=128"),
            "{}",
            app.sticker_url("42", false, 10)
        );
        assert!(
            app.sticker_url("42", true, 4096)
                .ends_with("/stickers/42.webp?size=512&animated=true"),
            "{}",
            app.sticker_url("42", true, 4096)
        );
    }

    #[test]
    fn a_guild_that_goes_away_takes_its_stickers_with_it() {
        let mut app = guild_app();
        app.set_guild_stickers("guild-1", vec![guild_sticker("1", "one", &[])]);
        app.remove_guild("guild-1");
        assert!(!app.guild_stickers.contains_key("guild-1"));
    }
}
