use crate::api::types::{
    CHANNEL_DM, CHANNEL_DM_PERSONAL_NOTES, CHANNEL_GROUP_DM, CHANNEL_GUILD_TEXT, ChannelResponse,
    MessageEmbedResponse,
};
use crate::app::{App, Focus, display_name};
use crate::ui::message_markdown;
use crate::ui::span_wrap;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::collections::HashMap;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

fn clip_url_for_display(url: &str, max_chars: usize) -> String {
    let t = url.trim();
    if t.is_empty() {
        return String::new();
    }
    let n = t.chars().count();
    if n <= max_chars {
        return t.to_string();
    }
    let take = max_chars.saturating_sub(1);
    format!("{}…", t.chars().take(take).collect::<String>())
}

fn embed_display_label(embed: &MessageEmbedResponse) -> (String, bool) {
    let original = embed
        .url
        .as_ref()
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"));
    let t = embed.embed_type.as_str();
    let is_gif = matches!(t, "gifv")
        || original.is_some_and(|u| u.contains("tenor.com") || u.to_lowercase().ends_with(".gif"))
        || embed
            .image
            .as_ref()
            .and_then(|m| m.url.as_ref().or(m.proxy_url.as_ref()))
            .is_some_and(|u| u.to_lowercase().ends_with(".gif"));
    let label = if let Some(u) = original {
        clip_url_for_display(u, 72)
    } else {
        let proxy = embed
            .image
            .as_ref()
            .and_then(|m| m.proxy_url.clone().or_else(|| m.url.clone()))
            .or_else(|| {
                embed
                    .thumbnail
                    .as_ref()
                    .and_then(|m| m.proxy_url.clone().or_else(|| m.url.clone()))
            });
        if let Some(u) = proxy {
            clip_url_for_display(&u, 72)
        } else {
            String::new()
        }
    };
    (label, is_gif)
}

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    app.message_scroll_max = 0;
    if app.active_channel_is_voice() {
        render_voice(frame, area, app);
    } else if app.active_channel_is_link() {
        render_link(frame, area, app);
    } else {
        render_messages(frame, area, app);
    }
}

/// in the beginning there was GOD Just kidding it was HAMPLER.
fn channel_welcome_label(channel: &ChannelResponse) -> String {
    match channel.channel_type() {
        CHANNEL_GUILD_TEXT => format!("#{}", channel.name),
        CHANNEL_DM_PERSONAL_NOTES => "Personal Notes".to_string(),
        CHANNEL_DM => channel
            .recipients
            .first()
            .map(display_name)
            .unwrap_or_else(|| "Direct Message".to_string()),
        CHANNEL_GROUP_DM => {
            if !channel.name.trim().is_empty() {
                channel.name.clone()
            } else if !channel.recipients.is_empty() {
                channel
                    .recipients
                    .iter()
                    .map(display_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                "Group DM".to_string()
            }
        }
        _ => {
            if channel.guild_id.is_some() && !channel.name.is_empty() {
                format!("#{}", channel.name)
            } else {
                channel.name.clone()
            }
        }
    }
}

/// praise satan
fn channel_welcome_lines(_app: &App, channel: &ChannelResponse) -> Vec<Line<'static>> {
    let label = channel_welcome_label(channel);
    let genesis =
        format!("In the beginning, there was nothing. Then, there was {label}. And it was good.");
    vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Welcome to ",
                Style::default()
                    .fg(crate::ui::theme::text())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                label.clone(),
                Style::default()
                    .fg(crate::ui::theme::accent())
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(genesis, crate::ui::theme::dim_style())),
        Line::from(""),
    ]
}

fn message_was_edited(message: &crate::api::types::MessageResponse) -> bool {
    message
        .edited_timestamp
        .as_ref()
        .is_some_and(|s| !s.trim().is_empty())
}

fn edited_span() -> Span<'static> {
    Span::styled(
        "(edited) ",
        crate::ui::theme::dim_style().add_modifier(Modifier::ITALIC),
    )
}

fn sel_prefix_span(is_selected: bool) -> Span<'static> {
    if is_selected {
        Span::styled("\u{25B6} ", Style::default().fg(crate::ui::theme::accent()))
    } else {
        Span::raw("  ")
    }
}

/// What the two columns left of a message say about it.
#[derive(Clone, Copy, Default)]
struct Gutter {
    /// The selected message: an arrow on its first row.
    selected: bool,
    /// A message that mentions the reader or answers them: a bar down
    /// every row that has a margin.
    mention: bool,
    /// The message the selected reply answers: a mark on its first row.
    reply_target: bool,
    /// Marked with `m` for a bulk delete: a cross on its first row, and
    /// the selection arrow turns the same colour where both apply.
    marked: bool,
}

impl Gutter {
    fn span(self, row: usize) -> Span<'static> {
        if row == 0 && self.selected {
            if self.marked {
                return Span::styled("\u{25B6} ", Style::default().fg(crate::ui::theme::danger()));
            }
            return sel_prefix_span(true);
        }
        if row == 0 && self.marked {
            return Span::styled("\u{2716} ", Style::default().fg(crate::ui::theme::danger()));
        }
        if row == 0 && self.reply_target {
            return Span::styled(
                "\u{21A9} ",
                Style::default()
                    .fg(crate::ui::theme::accent())
                    .add_modifier(Modifier::BOLD),
            );
        }
        if self.mention {
            return Span::styled(
                "\u{258E} ",
                Style::default().fg(crate::ui::theme::mention_bar()),
            );
        }
        Span::raw("  ")
    }
}

/// The message the selected message answers, when it is a reply and the
/// original is among the loaded messages.
fn reply_target_index(app: &App, messages: &[crate::api::types::MessageResponse]) -> Option<usize> {
    let selected = messages.get(app.selected_message_index?)?;
    let original_id = selected
        .referenced_message
        .as_deref()
        .map(|m| m.id.as_str())
        .filter(|id| !id.is_empty())
        .or_else(|| {
            selected
                .message_reference
                .as_ref()
                .map(|r| r.message_id.as_str())
                .filter(|id| !id.is_empty())
        })?;
    messages
        .iter()
        .position(|m| m.id == original_id)
        .filter(|&i| Some(i) != app.selected_message_index)
}

fn truncate_to_display_width(s: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(s) <= max_w {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
        if w + cw > max_w.saturating_sub(1) {
            out.push('…');
            break;
        }
        out.push(ch);
        w += cw;
    }
    out
}

fn referenced_body_preview(ref_msg: &crate::api::types::MessageResponse) -> String {
    // a reply to a forward has its text in the forwarded copy, so the
    // preview reads the same effective sets the body does
    let content = ref_msg.display_content();
    let c = content.trim();
    if !c.is_empty() {
        let flat: String = c.chars().filter(|&x| x != '\n' && x != '\r').collect();
        let count = flat.chars().count();
        let mut s: String = flat.chars().take(72).collect();
        if count > 72 {
            s.push('…');
        }
        s
    } else if let Some(first) = ref_msg.all_attachments().next() {
        let n = ref_msg.all_attachments().count();
        if n == 1 {
            format!("[file: {}]", first.filename)
        } else {
            format!("[{n} attachments]")
        }
    } else if let Some(first) = ref_msg.all_stickers().next() {
        let n = ref_msg.all_stickers().count();
        if n == 1 {
            format!("[sticker: {}]", first.name)
        } else {
            format!("[{n} stickers]")
        }
    } else if ref_msg.all_embeds().next().is_some() {
        "[embed]".to_string()
    } else {
        "(no text)".to_string()
    }
}

fn push_fluxer_client_system_message(
    app: &App,
    lines: &mut Vec<Line<'static>>,
    message: &crate::api::types::MessageResponse,
    is_selected_msg: bool,
) {
    let header_style = if is_selected_msg {
        Style::default().bg(crate::ui::theme::bg_tertiary())
    } else {
        Style::default()
    };
    let box_fg = crate::ui::theme::text_muted();
    let ts = format_timestamp(&message.timestamp, app.ui_settings.clock_12h);

    lines.push(
        Line::from(vec![
            sel_prefix_span(is_selected_msg),
            Span::styled("╭─ ", Style::default().fg(box_fg)),
            Span::styled(
                "Fluxerbot",
                Style::default()
                    .fg(crate::ui::theme::text())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" [SYSTEM]", Style::default().fg(crate::ui::theme::accent())),
            Span::styled(format!("  \u{2014} {ts}"), crate::ui::theme::dim_style()),
        ])
        .style(header_style),
    );

    lines.push(
        Line::from(vec![
            sel_prefix_span(is_selected_msg),
            Span::styled("┃", Style::default().fg(box_fg)),
        ])
        .style(header_style),
    );

    for row in message_markdown::content_lines(&message.content, app) {
        let mut r = vec![
            sel_prefix_span(is_selected_msg),
            Span::styled("┃ ", Style::default().fg(box_fg)),
        ];
        if row.is_empty() {
            r.push(Span::raw(" "));
        } else {
            r.extend(row);
        }
        lines.push(Line::from(r).style(header_style));
    }

    lines.push(
        Line::from(vec![
            sel_prefix_span(is_selected_msg),
            Span::styled("┃ ", Style::default().fg(box_fg)),
            Span::styled("\u{1F441}\u{FE0F} ", crate::ui::theme::dim_style()),
            Span::styled(
                "only you can see this message. ",
                crate::ui::theme::dim_style(),
            ),
            Span::styled(
                "dismiss",
                Style::default()
                    .fg(crate::ui::theme::link_color())
                    .add_modifier(Modifier::UNDERLINED),
            ),
        ])
        .style(header_style),
    );

    lines.push(
        Line::from(vec![
            sel_prefix_span(is_selected_msg),
            Span::styled("╰─", Style::default().fg(box_fg)),
        ])
        .style(header_style),
    );
}

/// Rows of one message before its left margin is added.
struct BlockRow {
    spans: Vec<Span<'static>>,
    style: Style,
    kind: RowKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    /// The reply context above the header.
    Context,
    Header,
    Body,
    /// Marker cells of a picture: never wrapped.
    Picture,
}

/// The left margin of a message's rows: the selection mark, then, with
/// avatars on, the avatar's four columns and a space, like the web app.
struct Margin {
    avatars: bool,
}

impl Margin {
    fn width(&self, kind: RowKind) -> usize {
        if self.avatars {
            7
        } else if matches!(kind, RowKind::Context | RowKind::Header) {
            2
        } else {
            0
        }
    }
}

const AVATAR_PLACEHOLDER: &str = "\u{2800}\u{2800}\u{2800}\u{2800}";

/// Lay out one message: wrap its rows to the text width minus the margin,
/// then put the margin in front of every row, the avatar's two rows on the
/// header row and the one after it.
fn finish_block(
    rows: Vec<BlockRow>,
    text_w: u16,
    margin: &Margin,
    gutter: Gutter,
    block_style: Style,
    avatar_slot: Option<usize>,
) -> Vec<Line<'static>> {
    let mut out: Vec<(Vec<Span<'static>>, Style, RowKind)> = Vec::new();
    for row in rows {
        let width = (text_w as usize)
            .saturating_sub(margin.width(row.kind))
            .max(1);
        if row.kind == RowKind::Picture {
            out.push((row.spans, row.style, row.kind));
            continue;
        }
        for spans in span_wrap::wrap_spans(&row.spans, width) {
            out.push((spans, row.style, row.kind));
        }
    }
    if out.is_empty() {
        out.push((Vec::new(), Style::default(), RowKind::Body));
    }
    let header_at = out.iter().position(|(_, _, k)| *k == RowKind::Header);
    if let (Some(_), Some(h)) = (avatar_slot, header_at)
        && h + 1 >= out.len()
    {
        // the avatar's second row needs a row under the header
        out.push((Vec::new(), Style::default(), RowKind::Body));
    }
    let muted = crate::ui::theme::muted_style();
    let mut lines = Vec::with_capacity(out.len());
    for (i, (spans, style, kind)) in out.into_iter().enumerate() {
        let mut line: Vec<Span<'static>> = Vec::with_capacity(spans.len() + 3);
        if margin.avatars {
            line.push(gutter.span(i));
            let avatar_row = match (avatar_slot, header_at) {
                (Some(k), Some(h)) if i == h => Some((k, 0)),
                (Some(k), Some(h)) if i == h + 1 => Some((k, 1)),
                _ => None,
            };
            match avatar_row {
                Some((k, r)) => line.push(Span::styled(
                    AVATAR_PLACEHOLDER,
                    crate::app::media_marker_style(k, r),
                )),
                None if kind == RowKind::Context && avatar_slot.is_some() => {
                    line.push(Span::styled(" \u{256D}\u{2500} ", muted));
                }
                None => line.push(Span::raw("    ")),
            }
            line.push(Span::raw(" "));
        } else if matches!(kind, RowKind::Context | RowKind::Header) {
            line.push(gutter.span(i));
        }
        line.extend(spans);
        lines.push(Line::from(line).style(block_style.patch(style)));
    }
    lines
}

fn context_row(
    width: usize,
    lead: &'static str,
    body: &str,
    body_style: Style,
    style: Style,
) -> BlockRow {
    let budget = width.saturating_sub(UnicodeWidthStr::width(lead)).max(1);
    BlockRow {
        spans: vec![
            Span::styled(lead, crate::ui::theme::muted_style()),
            Span::styled(truncate_to_display_width(body, budget), body_style),
        ],
        style,
        kind: RowKind::Context,
    }
}

/// The "new messages" line: a rule across the pane with the words in the
/// middle, in the same amber the mention bar uses, since both mean "this
/// is the part meant for you".
fn unread_divider_line(text_w: u16) -> Line<'static> {
    const LABEL: &str = " new messages ";
    let style = Style::default().fg(crate::ui::theme::mention_bar());
    let width = text_w as usize;
    let rule = width.saturating_sub(LABEL.chars().count());
    // a pane too narrow for the words keeps the rule, so the line is
    // still there to see
    if rule < 4 {
        return Line::from(Span::styled("\u{2500}".repeat(width.max(1)), style));
    }
    let left = rule / 2;
    let right = rule - left;
    Line::from(vec![
        Span::styled("\u{2500}".repeat(left), style),
        Span::styled(LABEL, style.add_modifier(Modifier::BOLD)),
        Span::styled("\u{2500}".repeat(right), style),
    ])
}

fn header_row(
    message: &crate::api::types::MessageResponse,
    author: &str,
    name_color: ratatui::style::Color,
    style: Style,
    clock_12h: bool,
    // `presence` is set only when the client has been told of one that is
    // not offline: a hollow dot on every other message would be noise,
    // and would claim they are away where the truth is that the server
    // has said nothing about them.
    presence: Option<crate::api::types::PresenceStatus>,
) -> BlockRow {
    let timestamp = format_timestamp(&message.timestamp, clock_12h);
    let mut spans = vec![Span::styled(
        format!("[{timestamp}] "),
        crate::ui::theme::dim_style(),
    )];
    if message_was_edited(message) {
        spans.push(edited_span());
    }
    if let Some(status) = presence {
        spans.push(crate::ui::presence::dot_prefix(status));
    }
    spans.push(Span::styled(
        author.to_string(),
        Style::default().fg(name_color).add_modifier(Modifier::BOLD),
    ));
    BlockRow {
        spans,
        style,
        kind: RowKind::Header,
    }
}

fn body_row(spans: Vec<Span<'static>>) -> BlockRow {
    BlockRow {
        spans,
        style: Style::default(),
        kind: RowKind::Body,
    }
}

fn markdown_rows(rows: &mut Vec<BlockRow>, text: &str, app: &App, lead: Option<Span<'static>>) {
    for row in message_markdown::content_lines(text, app) {
        let mut spans = Vec::with_capacity(row.len() + 1);
        if let Some(lead) = &lead {
            spans.push(lead.clone());
        }
        // A blank line stays empty: ratatui draws a row of blanks alone as
        // two rows, an empty row as one.
        spans.extend(row);
        rows.push(body_row(spans));
    }
}

/// The marker rows of one block of cells; the block is registered for this
/// draw and fetched when it comes on screen.
fn push_slot_rows(
    app: &App,
    rows: &mut Vec<BlockRow>,
    slot: crate::app::MediaSlot,
    left: &mut usize,
) {
    if *left == 0 {
        return;
    }
    let (cols, prows) = (slot.cols, slot.rows);
    let k = app.register_media_slot(slot);
    for r in 0..prows {
        rows.push(BlockRow {
            spans: vec![Span::styled(
                "\u{2800}".repeat(cols as usize),
                crate::app::media_marker_style(k, r),
            )],
            style: Style::default(),
            kind: RowKind::Picture,
        });
    }
    *left -= 1;
}

/// The marker rows of one preview, at the size the picture's shape takes
/// within `max`.
fn push_picture_rows(
    app: &App,
    rows: &mut Vec<BlockRow>,
    pic: &crate::media::InlinePicture,
    max: (u16, u16),
    left: &mut usize,
) {
    let (cols, prows) = crate::media::picture_cells((pic.width, pic.height), app.cell_px, max);
    let url = crate::media::proxied_url(pic, crate::media::block_px(cols, prows, app.cell_px));
    push_slot_rows(
        app,
        rows,
        crate::app::MediaSlot::new(url, cols, prows, crate::app::MediaKind::Picture),
        left,
    );
}

fn build_message_lines(
    app: &App,
    messages: &[crate::api::types::MessageResponse],
    text_w: u16,
    pane_rows: u16,
) -> (Vec<Line<'static>>, Vec<(usize, usize)>, Vec<usize>) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut line_ranges = vec![(0usize, 0usize); messages.len()];
    // the message whose header names this one's author: itself, unless it
    // is grouped under another from the same person
    let mut group_head: Vec<usize> = (0..messages.len()).collect();
    let mut prev_author_id: Option<&str> = None;
    let mut prev_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;
    let margin = Margin {
        avatars: app.avatars_enabled(),
    };
    let inline = app.inline_media_enabled();
    let body_w = (text_w as usize)
        .saturating_sub(margin.width(RowKind::Body))
        .max(1) as u16;
    let ctx_w = (text_w as usize)
        .saturating_sub(margin.width(RowKind::Context))
        .max(1);
    let picture_max = crate::media::preview_limits(body_w, pane_rows, app.cell_px);
    let reply_target = reply_target_index(app, messages);
    // the message the "new messages" line sits above, if any
    let first_unread = app.active_first_unread_message_id();

    for (idx, message) in messages.iter().enumerate() {
        let is_selected_msg = app.selected_message_index == Some(idx);

        // the line goes above the first message that arrived after the
        // reader last left, and takes a row of its own
        if first_unread.as_deref() == Some(message.id.as_str()) {
            lines.push(unread_divider_line(text_w));
        }
        let cur_ts = message
            .timestamp
            .parse::<chrono::DateTime<chrono::Utc>>()
            .ok();

        if message.message_type == crate::slash_commands::MESSAGE_TYPE_CLIENT_SYSTEM
            && message.author.id == crate::slash_commands::FLUXERBOT_ID
        {
            if idx > 0 {
                lines.push(Line::from(""));
            }
            let block_start = lines.len();
            push_fluxer_client_system_message(app, &mut lines, message, is_selected_msg);
            line_ranges[idx] = (block_start, lines.len());
            prev_author_id = Some(message.author.id.as_str());
            prev_timestamp = cur_ts;
            continue;
        }

        let gid = app.guild_id_for_channel(message.channel_id.as_str());
        let author = app.shown_name_for_user(gid.as_deref(), &message.author);

        let is_self = message.author.id == app.me.id;
        let name_color = app.member_name_color(gid.as_deref(), &message.author.id, is_self);
        // only somebody who is about is marked; see `header_row`
        let author_presence = {
            let status = app.presence_status(&message.author.id);
            (!status.is_offline()).then_some(status)
        };

        let has_reply_rail =
            message.referenced_message.is_some() || message.message_reference.is_some();
        let same_author = prev_author_id == Some(message.author.id.as_str());
        let within_group = same_author
            && !has_reply_rail
            && match (prev_timestamp, cur_ts) {
                (Some(prev), Some(cur)) => (cur - prev).num_minutes().abs() < 5,
                _ => false,
            };
        if within_group && idx > 0 {
            group_head[idx] = group_head[idx - 1];
        }

        let mention = app.message_highlights_me(message);
        let block_style = if mention {
            crate::ui::theme::mention_block_style()
        } else {
            Style::default()
        };
        let header_style = if is_selected_msg {
            Style::default().bg(crate::ui::theme::bg_tertiary())
        } else {
            block_style
        };
        let gutter = Gutter {
            selected: is_selected_msg,
            mention,
            marked: app.is_marked(&message.channel_id, &message.id),
            reply_target: reply_target == Some(idx),
        };

        if idx > 0 && !within_group {
            lines.push(Line::from(""));
        }

        let block_start = lines.len();
        let mut rows: Vec<BlockRow> = Vec::new();
        let clock_12h = app.ui_settings.clock_12h;

        if let Some(ref_msg) = message.referenced_message.as_deref() {
            let ref_author = app.shown_name_for_user(gid.as_deref(), &ref_msg.author);
            let preview = referenced_body_preview(ref_msg);
            let ctx_body = format!("@{ref_author} - {preview}");
            rows.push(context_row(
                ctx_w,
                "\u{21AA} ",
                &ctx_body,
                crate::ui::theme::dim_style(),
                header_style,
            ));
            rows.push(header_row(
                message,
                &author,
                name_color,
                header_style,
                clock_12h,
                author_presence,
            ));
        } else if message.message_reference.is_some() {
            let (ctx_body, body_style) = if message.is_forward() {
                (
                    "Forwarded",
                    crate::ui::theme::muted_style().add_modifier(Modifier::ITALIC),
                )
            } else {
                (
                    "(original message unavailable)",
                    crate::ui::theme::dim_style(),
                )
            };
            rows.push(context_row(
                ctx_w,
                "\u{21AA} ",
                ctx_body,
                body_style,
                header_style,
            ));
            rows.push(header_row(
                message,
                &author,
                name_color,
                header_style,
                clock_12h,
                author_presence,
            ));
        } else if within_group && !is_selected_msg {
            // grouped under the previous message: no header
        } else {
            // The selected message carries a header of its own even when
            // it is grouped under one from the same person, so the pane
            // always names whoever wrote the message the reader is on:
            // the one above it that would have named them may be off the
            // top of the pane.
            rows.push(header_row(
                message,
                &author,
                name_color,
                header_style,
                clock_12h,
                author_presence,
            ));
        }

        // a forward's text is in its snapshots, not in `content`
        let body_text = message.display_content();
        if !body_text.trim().is_empty() {
            markdown_rows(&mut rows, &body_text, app, None);
        }

        prev_author_id = Some(&message.author.id);
        prev_timestamp = cur_ts;

        let mut pictures_left = crate::media::MAX_PICTURES_PER_MESSAGE;
        for attachment in message.all_attachments() {
            let size_str = match attachment.size {
                Some(s) if s < 1024 => format!("{} B", s),
                Some(s) if s < 1024 * 1024 => format!("{:.1} KB", s as f64 / 1024.0),
                Some(s) => format!("{:.1} MB", s as f64 / (1024.0 * 1024.0)),
                None => "unknown".to_string(),
            };
            let mime = attachment.content_type.as_deref().unwrap_or("unknown");
            let audio = crate::media::attachment_is_audio(attachment);
            let size_str = match attachment.duration.filter(|_| audio) {
                Some(d) => format!("{size_str} \u{00B7} {}", crate::media::format_duration(d)),
                None => size_str,
            };
            rows.push(body_row(vec![
                Span::styled(
                    if audio { "\u{266A} " } else { "\u{1F4CE} " },
                    Style::default().fg(crate::ui::theme::accent_dim()),
                ),
                Span::styled(
                    attachment.filename.clone(),
                    Style::default()
                        .fg(crate::ui::theme::link_color())
                        .add_modifier(Modifier::UNDERLINED),
                ),
                Span::styled(
                    format!(" [{mime} \u{00B7} {size_str}]"),
                    crate::ui::theme::dim_style(),
                ),
            ]));
            if inline && let Some(pic) = crate::media::attachment_picture(attachment) {
                push_picture_rows(app, &mut rows, &pic, picture_max, &mut pictures_left);
            }
        }

        for sticker in message.all_stickers() {
            rows.push(body_row(vec![
                Span::styled(
                    "\u{1F5BC} ",
                    Style::default().fg(crate::ui::theme::accent_dim()),
                ),
                Span::styled(
                    sticker.name.clone(),
                    Style::default().fg(crate::ui::theme::accent()),
                ),
                Span::styled(
                    if sticker.nsfw {
                        " [sticker \u{00B7} explicit]"
                    } else {
                        " [sticker]"
                    },
                    crate::ui::theme::dim_style(),
                ),
            ]));
            if inline
                && let Some(slot) = app.sticker_slot(&sticker.id, sticker.animated, picture_max)
            {
                push_slot_rows(app, &mut rows, slot, &mut pictures_left);
            }
        }

        for embed in message.all_embeds() {
            let has_content = embed.title.is_some()
                || embed.description.is_some()
                || embed.author.is_some()
                || !embed.fields.is_empty();
            let bar_color = embed
                .color
                .map(|c| {
                    ratatui::style::Color::Rgb(
                        ((c >> 16) & 0xFF) as u8,
                        ((c >> 8) & 0xFF) as u8,
                        (c & 0xFF) as u8,
                    )
                })
                .unwrap_or(crate::ui::theme::accent_dim());
            let bar = Span::styled("\u{2502} ", Style::default().fg(bar_color));

            if !has_content {
                let (label, is_gif) = embed_display_label(embed);
                if !label.is_empty() {
                    let mut spans = vec![bar.clone()];
                    if is_gif {
                        spans.push(Span::styled(
                            "[GIF] ",
                            Style::default()
                                .fg(crate::ui::theme::accent())
                                .add_modifier(Modifier::BOLD),
                        ));
                    }
                    spans.push(Span::styled(
                        label,
                        Style::default()
                            .fg(crate::ui::theme::link_color())
                            .add_modifier(Modifier::UNDERLINED),
                    ));
                    rows.push(body_row(spans));
                }
            } else {
                if let Some(author) = &embed.author {
                    rows.push(body_row(vec![
                        bar.clone(),
                        Span::styled(author.name.clone(), crate::ui::theme::dim_style()),
                    ]));
                }
                if let Some(title) = &embed.title {
                    let mut title_spans = vec![bar.clone()];
                    let base = if embed.url.is_some() {
                        Style::default()
                            .fg(crate::ui::theme::link_color())
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(crate::ui::theme::text())
                            .add_modifier(Modifier::BOLD)
                    };
                    for s in message_markdown::parse_message_spans(title, app) {
                        title_spans.push(Span::styled(s.content.to_string(), base.patch(s.style)));
                    }
                    rows.push(body_row(title_spans));
                }
                if let Some(desc) = &embed.description {
                    markdown_rows(&mut rows, desc, app, Some(bar.clone()));
                }
                for field in &embed.fields {
                    rows.push(body_row(vec![
                        bar.clone(),
                        Span::styled(
                            field.name.clone(),
                            Style::default()
                                .fg(crate::ui::theme::text())
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                    markdown_rows(&mut rows, &field.value, app, Some(bar.clone()));
                }
                if let Some(footer) = &embed.footer {
                    rows.push(body_row(vec![
                        bar.clone(),
                        Span::styled(footer.text.clone(), crate::ui::theme::muted_style()),
                    ]));
                }
            }
            if inline && let Some(pic) = crate::media::embed_picture(embed) {
                push_picture_rows(app, &mut rows, &pic, picture_max, &mut pictures_left);
            }
        }

        if !message.reactions.is_empty() {
            let mut reaction_spans: Vec<Span<'static>> = Vec::new();
            for reaction in &message.reactions {
                let style = if reaction.me {
                    Style::default()
                        .fg(crate::ui::theme::accent())
                        .add_modifier(Modifier::BOLD)
                } else {
                    crate::ui::theme::dim_style()
                };
                // custom emoji: the picture where the terminal can draw one
                let picture = reaction
                    .emoji
                    .id
                    .as_deref()
                    .and_then(|id| app.custom_emoji_placeholder(id, reaction.emoji.animated));
                match picture {
                    Some(p) => {
                        reaction_spans.push(Span::styled(" ", style));
                        reaction_spans.push(p);
                        reaction_spans.push(Span::styled(format!(" {}", reaction.count), style));
                    }
                    None => {
                        let emoji_str = if reaction.emoji.id.is_some() {
                            format!(":{}:", reaction.emoji.name)
                        } else {
                            reaction.emoji.name.clone()
                        };
                        reaction_spans.push(Span::styled(
                            format!(" {emoji_str} {}", reaction.count),
                            style,
                        ));
                    }
                }
                reaction_spans.push(Span::raw(" "));
            }
            rows.push(body_row(reaction_spans));
        }

        // the header the avatar's two rows hang off: every message that
        // has one, grouped or not
        let avatar_slot = if margin.avatars && (!within_group || is_selected_msg) {
            Some(app.register_media_slot(app.avatar_slot(
                gid.as_deref(),
                &message.author,
                message.member.as_ref().and_then(|m| m.avatar.as_deref()),
            )))
        } else {
            None
        };
        lines.extend(finish_block(
            rows,
            text_w,
            &margin,
            gutter,
            block_style,
            avatar_slot,
        ));

        line_ranges[idx] = (block_start, lines.len());
    }

    (lines, line_ranges, group_head)
}

/// Rows each line takes at `text_w`. Lines are wrapped before they get
/// here, so nearly all fit in one row and only a wider one is measured
/// the slow way, by wrapping it as the paragraph would. A line of blanks
/// alone is measured that way too: ratatui draws it as two rows.
fn paragraph_line_heights(lines: &[Line<'static>], text_w: u16) -> Vec<u16> {
    lines
        .iter()
        .map(|line| {
            let width: usize = line
                .spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            let content = line.spans.iter().any(|s| !s.content.is_empty());
            let blank_only = content && line.spans.iter().all(|s| s.content.trim().is_empty());
            if width <= text_w as usize && !blank_only {
                1
            } else {
                Paragraph::new(Text::from(vec![line.clone()]))
                    .wrap(Wrap { trim: false })
                    .line_count(text_w) as u16
            }
        })
        .collect()
}

#[cfg(test)]
mod height_tests {
    use super::*;

    /// The shortcut must agree with ratatui's own wrapping for every kind
    /// of line: empty, blanks alone (two rows in ratatui), text with
    /// leading or trailing blanks, wide characters, and lines that wrap.
    #[test]
    fn heights_match_what_ratatui_draws() {
        let lines: Vec<Line<'static>> = [
            "",
            " ",
            "   ",
            "a b",
            "  a",
            "a  ",
            "abcdefghij",
            "abcdefghijk",
            "\u{1F4CE} pic.png",
            "\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}",
            "word longerthanthewidth",
            "\u{2800}\u{2800}\u{2800}\u{2800}",
        ]
        .into_iter()
        .map(Line::from)
        .collect();
        for text_w in [4u16, 10, 20] {
            let fast = paragraph_line_heights(&lines, text_w);
            for (line, h) in lines.iter().zip(fast) {
                let real = Paragraph::new(Text::from(vec![line.clone()]))
                    .wrap(Wrap { trim: false })
                    .line_count(text_w) as u16;
                assert_eq!(h, real, "{:?} at width {text_w}", line.to_string());
            }
        }
    }
}

/// Everything that decides what the message pane's lines look like. While
/// it stays the same from one draw to the next, the lines are reused.
#[derive(Clone, PartialEq, Eq)]
pub struct LayoutKey {
    channel: Option<String>,
    text_w: u16,
    pane_rows: u16,
    selected: Option<usize>,
    /// The message list's version: every change bumps it.
    messages: u64,
    count: usize,
    clock_12h: bool,
    avatars: bool,
    inline: bool,
    theme: crate::config::Theme,
    cell_px: (u32, u32),
    roster: u64,
    emoji: u64,
    presence: u64,
}

/// The message pane's lines as built for one [`LayoutKey`], with what the
/// build registered on the side: the media and emoji slots its marker
/// cells refer to, and the emoji it wanted fetched.
pub struct PaneLayout {
    key: LayoutKey,
    pub lines: Vec<Line<'static>>,
    /// Rows before each line, and after the last: `cum[i]` is where line `i`
    /// starts, `cum[lines.len()]` the body's height.
    pub cum: Vec<u32>,
    pub line_ranges: Vec<(usize, usize)>,
    /// For each message, the one whose header names its author: itself,
    /// or the message it is grouped under.
    pub group_head: Vec<usize>,
    media_slots: Vec<crate::app::MediaSlot>,
    emoji_slots: Vec<String>,
    emoji_wants: Vec<(String, bool)>,
}

impl std::fmt::Debug for PaneLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PaneLayout({} lines)", self.lines.len())
    }
}

/// The pane's lines for these messages, built now or reused from the last
/// draw. Building takes tens of milliseconds for a few hundred messages,
/// which would cap scrolling; a plain scroll changes nothing in the key.
pub fn pane_layout(
    app: &App,
    messages: &[crate::api::types::MessageResponse],
    text_w: u16,
    pane_rows: u16,
) -> std::rc::Rc<PaneLayout> {
    let key = LayoutKey {
        channel: app.selected_channel_id.clone(),
        text_w,
        pane_rows,
        selected: app.selected_message_index,
        messages: app.messages_version,
        presence: app.presence_version,
        count: messages.len(),
        clock_12h: app.ui_settings.clock_12h,
        avatars: app.avatars_enabled(),
        inline: app.inline_media_enabled(),
        theme: app.ui_settings.theme,
        cell_px: app.cell_px,
        roster: app.roster_version,
        emoji: app.custom_emoji_version,
    };
    if let Some(layout) = app.pane_layout.borrow().as_ref()
        && layout.key == key
    {
        // the marker cells in the lines count on these slot tables
        *app.media_slots.borrow_mut() = layout.media_slots.clone();
        *app.custom_emoji_slots.borrow_mut() = layout.emoji_slots.clone();
        let mut wanted = app.custom_emoji_wanted.borrow_mut();
        for want in &layout.emoji_wants {
            if !wanted.iter().any(|w| w.0 == want.0) {
                wanted.push(want.clone());
            }
        }
        return layout.clone();
    }
    app.media_slots.borrow_mut().clear();
    app.custom_emoji_slots.borrow_mut().clear();
    let (lines, line_ranges, group_head) = build_message_lines(app, messages, text_w, pane_rows);
    let heights = paragraph_line_heights(&lines, text_w);
    let mut cum = Vec::with_capacity(lines.len() + 1);
    cum.push(0u32);
    for h in heights {
        cum.push(cum.last().copied().unwrap_or(0) + h as u32);
    }
    let layout = std::rc::Rc::new(PaneLayout {
        key,
        lines,
        cum,
        line_ranges,
        group_head,
        media_slots: app.media_slots.borrow().clone(),
        emoji_slots: app.custom_emoji_slots.borrow().clone(),
        emoji_wants: app.custom_emoji_wanted.borrow().clone(),
    });
    *app.pane_layout.borrow_mut() = Some(layout.clone());
    layout
}

/// The rows of the pane from top to bottom: blank filler when the content
/// is shorter than the pane, the channel welcome, a gap, then the body.
struct RowModel<'a> {
    filler: u32,
    welcome: &'a [Line<'static>],
    welcome_heights: &'a [u16],
    gap: u32,
    body: &'a [Line<'static>],
    body_cum: &'a [u32],
}

impl RowModel<'_> {
    fn welcome_rows(&self) -> u32 {
        self.welcome_heights.iter().map(|&h| h as u32).sum()
    }

    /// Rows before the body.
    fn pre_rows(&self) -> u32 {
        self.filler + self.welcome_rows() + self.gap
    }

    fn body_rows(&self) -> u32 {
        self.body_cum.last().copied().unwrap_or(0)
    }

    fn total(&self) -> u32 {
        self.pre_rows() + self.body_rows()
    }

    /// The lines that cover rows `top..top + rows`, and how many rows of the
    /// first one lie above `top`.
    fn window(&self, top: u32, rows: u16) -> (Vec<Line<'static>>, u16) {
        let need = top + rows as u32;
        let mut out: Vec<Line<'static>> = Vec::new();
        let mut offset = 0u32;
        let mut row = 0u32;
        let pre: Vec<(Line<'static>, u32)> = (0..self.filler)
            .map(|_| (Line::from(""), 1))
            .chain(
                self.welcome
                    .iter()
                    .zip(self.welcome_heights)
                    .map(|(l, &h)| (l.clone(), h as u32)),
            )
            .chain((0..self.gap).map(|_| (Line::from(""), 1)))
            .collect();
        for (line, h) in pre {
            let end = row + h;
            if end > top {
                if out.is_empty() {
                    offset = top.saturating_sub(row);
                }
                out.push(line);
            }
            row = end;
            if row >= need {
                return (out, offset as u16);
            }
        }
        let pre_rows = row;
        let first = if top > pre_rows {
            self.body_cum
                .partition_point(|&c| pre_rows + c <= top)
                .saturating_sub(1)
        } else {
            0
        };
        for i in first..self.body.len() {
            let start = pre_rows + self.body_cum[i];
            let end = pre_rows + self.body_cum[i + 1];
            if end <= top {
                continue;
            }
            if out.is_empty() {
                offset = top.saturating_sub(start);
            }
            out.push(self.body[i].clone());
            if end >= need {
                break;
            }
        }
        (out, offset as u16)
    }
}

#[cfg(test)]
mod row_model_tests {
    use super::*;

    fn texts(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn window_walks_filler_welcome_gap_and_body_by_rows() {
        let welcome = [Line::from("w0"), Line::from("w1")];
        let body = [
            Line::from("b0"),
            Line::from("b1 (two rows)"),
            Line::from("b2"),
        ];
        let model = RowModel {
            filler: 2,
            welcome: &welcome,
            welcome_heights: &[1, 1],
            gap: 1,
            body: &body,
            body_cum: &[0, 1, 3, 4],
        };
        assert_eq!(model.pre_rows(), 5);
        assert_eq!(model.total(), 9);
        let (lines, offset) = model.window(0, 4);
        assert_eq!(texts(&lines), ["", "", "w0", "w1"]);
        assert_eq!(offset, 0);
        let (lines, offset) = model.window(4, 3);
        assert_eq!(texts(&lines), ["", "b0", "b1 (two rows)"]);
        assert_eq!(offset, 0);
        let (lines, offset) = model.window(6, 2);
        assert_eq!(texts(&lines), ["b1 (two rows)"]);
        assert_eq!(offset, 0);
        let (lines, offset) = model.window(7, 2);
        assert_eq!(
            texts(&lines),
            ["b1 (two rows)", "b2"],
            "starts inside the tall line"
        );
        assert_eq!(offset, 1);
        let (lines, _) = model.window(8, 5);
        assert_eq!(texts(&lines), ["b2"]);
    }
}

/// The message whose block holds row `top` (else the first one below it),
/// and how far into it `top` lies: where the reader is.
fn anchor_at(
    top: u32,
    pre_rows: u32,
    line_ranges: &[(usize, usize)],
    cum: &[u32],
    ids: &[&str],
) -> Option<(String, i64)> {
    for (i, &(b0, b1)) in line_ranges.iter().enumerate() {
        let (Some(&start), Some(&end)) = (cum.get(b0), cum.get(b1)) else {
            continue;
        };
        let (start, end) = (pre_rows + start, pre_rows + end);
        if top < end || i + 1 == line_ranges.len() {
            let id = ids.get(i)?;
            return Some((id.to_string(), top as i64 - start as i64));
        }
    }
    None
}

/// The top row that puts the anchored message back where it was, in a
/// layout that may have changed around it.
fn top_for_anchor(
    message_id: &str,
    offset: i64,
    pre_rows: u32,
    line_ranges: &[(usize, usize)],
    cum: &[u32],
    ids: &[&str],
    max_scroll: u32,
) -> Option<u32> {
    let i = ids.iter().position(|id| *id == message_id)?;
    let &(b0, _) = line_ranges.get(i)?;
    let start = pre_rows as i64 + *cum.get(b0)? as i64;
    Some((start + offset).clamp(0, max_scroll as i64) as u32)
}

#[cfg(test)]
mod anchor_tests {
    use super::*;

    // three messages of 2, 3 and 1 rows after 4 rows of welcome
    const RANGES: [(usize, usize); 3] = [(0, 2), (2, 5), (5, 6)];
    const CUM: [u32; 7] = [0, 1, 2, 3, 4, 5, 6];
    const IDS: [&str; 3] = ["a", "b", "c"];

    #[test]
    fn the_anchor_is_the_message_under_the_top_row() {
        assert_eq!(anchor_at(0, 4, &RANGES, &CUM, &IDS), Some(("a".into(), -4)));
        assert_eq!(anchor_at(5, 4, &RANGES, &CUM, &IDS), Some(("a".into(), 1)));
        assert_eq!(anchor_at(6, 4, &RANGES, &CUM, &IDS), Some(("b".into(), 0)));
        assert_eq!(anchor_at(8, 4, &RANGES, &CUM, &IDS), Some(("b".into(), 2)));
        assert_eq!(anchor_at(9, 4, &RANGES, &CUM, &IDS), Some(("c".into(), 0)));
        assert_eq!(
            anchor_at(30, 4, &RANGES, &CUM, &IDS),
            Some(("c".into(), 21))
        );
    }

    #[test]
    fn a_new_message_below_does_not_move_the_reader() {
        // the reader is 1 row into "a"; a 4-row message "d" arrives below
        let (id, offset) = anchor_at(5, 4, &RANGES, &CUM, &IDS).unwrap();
        let ranges = [(0, 2), (2, 5), (5, 6), (6, 10)];
        let cum = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let ids = ["a", "b", "c", "d"];
        assert_eq!(
            top_for_anchor(&id, offset, 4, &ranges, &cum, &ids, 100),
            Some(5)
        );
        // older messages loaded above push the anchor down the pane by their rows
        let ranges = [(0, 3), (3, 5), (5, 8), (8, 9)];
        let cum = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let ids = ["old", "a", "b", "c"];
        assert_eq!(
            top_for_anchor(&id, offset, 4, &ranges, &cum, &ids, 100),
            Some(8)
        );
        // never past the end of the content
        assert_eq!(
            top_for_anchor(&id, offset, 4, &ranges, &cum, &ids, 6),
            Some(6)
        );
        assert_eq!(top_for_anchor("gone", 0, 4, &ranges, &cum, &ids, 6), None);
    }
}

#[cfg(test)]
mod overlay_tests {
    use super::*;
    use crate::app::{MediaKind, MediaSlot, Picture, PictureFrames, ServerSelection};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn app_with_picture() -> (App, usize) {
        let mut app = App::new(
            Default::default(),
            Default::default(),
            Default::default(),
            Vec::new(),
            Vec::new(),
            ServerSelection::DirectMessages,
            None,
            Default::default(),
        );
        app.pixel_mode = true;
        app.cell_px = (10, 20);
        // a 4x3 block whose picture is 40x60 px, ready in the cache
        let slot = MediaSlot::new("https://x/p.png".to_string(), 4, 3, MediaKind::Picture);
        let img = image::RgbaImage::from_pixel(40, 60, image::Rgba([9, 9, 9, 255]));
        assert!(app.media.start(&slot.key));
        app.media.finish(
            slot.key.clone(),
            Some(PictureFrames::new(
                vec![Picture::Pixels(std::sync::Arc::new(img))],
                vec![std::time::Duration::ZERO],
            )),
            40 * 60 * 4,
            1,
        );
        let k = app.register_media_slot(slot);
        (app, k)
    }

    /// Draw marker rows `rows` of block `k` at screen rows `at`, run the
    /// overlay, and return the placements.
    fn placements(app: &App, k: usize, rows: &[u16], at: &[u16]) -> Vec<(Rect, u32, u32)> {
        let mut terminal = Terminal::new(TestBackend::new(6, 4)).unwrap();
        terminal
            .draw(|frame| {
                let buf = frame.buffer_mut();
                for (&r, &y) in rows.iter().zip(at) {
                    for x in 1..5u16 {
                        buf[(x, y)].set_style(crate::app::media_marker_style(k, r));
                    }
                }
                app.pixel_placements.borrow_mut().clear();
                overlay_media(frame, Rect::new(0, 0, 6, 4), app);
            })
            .unwrap();
        app.pixel_placements
            .borrow()
            .iter()
            .map(|p| (p.area, p.image.width(), p.image.height()))
            .collect()
    }

    #[test]
    fn a_block_cut_by_the_pane_edge_shows_the_rows_on_screen() {
        let (app, k) = app_with_picture();
        // whole: rows 0..3 at screen rows 1..4
        assert_eq!(
            placements(&app, k, &[0, 1, 2], &[1, 2, 3]),
            [(Rect::new(1, 1, 4, 3), 40, 60)]
        );
        // cut at the top: rows 1..3 visible at screen rows 0..2 -> pixels 20..60
        assert_eq!(
            placements(&app, k, &[1, 2], &[0, 1]),
            [(Rect::new(1, 0, 4, 2), 40, 40)]
        );
        // cut at the bottom: rows 0..2 at screen rows 2..4 -> pixels 0..40
        assert_eq!(
            placements(&app, k, &[0, 1], &[2, 3]),
            [(Rect::new(1, 2, 4, 2), 40, 40)]
        );
        // rows that do not follow each other are not a block on screen
        assert_eq!(placements(&app, k, &[0, 2], &[1, 2]), []);
    }
}

/// When a message is selected, adjust `message_scroll_from_bottom` so the selection stays in view.
pub fn scroll_for_selected_message(
    app: &App,
    text_w: u16,
    pane_visible: u16,
    current_scroll_from_bottom: u16,
) -> Option<u16> {
    let idx = app.selected_message_index?;
    let messages = app.active_messages();
    if messages.is_empty() || idx >= messages.len() {
        return None;
    }
    if app
        .selected_channel_id
        .as_ref()
        .is_some_and(|id| app.loading_messages.contains(id))
    {
        return None;
    }

    let welcome: Vec<Line<'static>> = app
        .active_channel()
        .map(|ch| channel_welcome_lines(app, &ch))
        .unwrap_or_default();
    let welcome_heights = paragraph_line_heights(&welcome, text_w);
    let layout = pane_layout(app, &messages, text_w, pane_visible);
    let mut model = RowModel {
        filler: 0,
        welcome: &welcome,
        welcome_heights: &welcome_heights,
        gap: u32::from(!welcome.is_empty()),
        body: &layout.lines,
        body_cum: &layout.cum,
    };
    let pane = pane_visible as u32;
    model.filler = pane.saturating_sub(model.total());
    let total = model.total();
    if total <= pane {
        return Some(0);
    }

    let max_scroll = total - pane;
    let scroll = (current_scroll_from_bottom as u32).min(max_scroll);
    let mut top = max_scroll - scroll;

    let (b0, b1) = layout.line_ranges.get(idx).copied().unwrap_or((0, 0));
    if b1 >= layout.cum.len() {
        return None;
    }
    let pre = model.pre_rows();
    let rs = pre + layout.cum[b0];
    let re = pre + layout.cum[b1];

    if re.saturating_sub(rs) > pane {
        top = rs;
    } else {
        if rs < top {
            top = rs;
        }
        if re > top + pane {
            top = re - pane;
        }
    }

    top = top.min(max_scroll);
    let new_scroll = max_scroll - top;
    Some(new_scroll.min(u16::MAX as u32) as u16)
}

fn render_messages(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = block("Messages", app.focus == Focus::Messages);
    let inner = block.inner(area);
    let text_w = inner.width.max(1);

    let pane_visible = area.height.saturating_sub(2).max(1);

    let welcome: Vec<Line<'static>> = app
        .active_channel()
        .map(|ch| channel_welcome_lines(app, &ch))
        .unwrap_or_default();
    let welcome_heights = paragraph_line_heights(&welcome, text_w);

    let messages = app.active_messages();
    let loading = app
        .selected_channel_id
        .as_ref()
        .is_some_and(|id| app.loading_messages.contains(id));

    let layout = if loading || messages.is_empty() {
        None
    } else {
        Some(pane_layout(app, &messages, text_w, pane_visible))
    };
    let loading_line = [Line::from(Span::styled(
        "Loading messages...",
        crate::ui::theme::dim_style(),
    ))];
    let (body, body_cum): (&[Line<'static>], &[u32]) = match &layout {
        Some(l) => (&l.lines, &l.cum),
        None if loading => (&loading_line, &[0, 1]),
        None => (&[], &[0]),
    };
    let mut model = RowModel {
        filler: 0,
        welcome: &welcome,
        welcome_heights: &welcome_heights,
        gap: u32::from(!welcome.is_empty()),
        body,
        body_cum,
    };
    let pane = pane_visible as u32;
    model.filler = pane.saturating_sub(model.total());
    let total = model.total();

    let max_scroll = total.saturating_sub(pane);
    app.message_scroll_max = max_scroll.min(u16::MAX as u32) as u16;
    let mut scroll_from_bottom = (app.message_scroll_from_bottom as u32).min(max_scroll);
    let ids: Vec<&str> = messages.iter().map(|m| m.id.as_str()).collect();
    let pre_rows = model.pre_rows();
    // Scrolled up, and the content changed since the last draw (a message
    // arrived, history loaded, an edit): put the reader back on the same
    // message rather than let the bottom drag the view.
    if scroll_from_bottom > 0
        && let (Some(anchor), Some(layout)) = (&app.pane_anchor, &layout)
        && anchor.channel == app.selected_channel_id
        && anchor.total_rows != total
        && let Some(new_top) = top_for_anchor(
            &anchor.message_id,
            anchor.offset,
            pre_rows,
            &layout.line_ranges,
            &layout.cum,
            &ids,
            max_scroll,
        )
    {
        scroll_from_bottom = max_scroll - new_top;
        app.message_scroll_from_bottom = scroll_from_bottom.min(u16::MAX as u32) as u16;
    }
    let top = max_scroll - scroll_from_bottom;
    app.pane_anchor = layout.as_ref().and_then(|layout| {
        let (message_id, offset) =
            anchor_at(top, pre_rows, &layout.line_ranges, &layout.cum, &ids)?;
        Some(crate::app::PaneAnchor {
            channel: app.selected_channel_id.clone(),
            message_id,
            offset,
            total_rows: total,
        })
    });

    // The same content in the same place, just scrolled: the terminal can
    // shift the rows itself and keep the pictures in them.
    let view = crate::app::PaneView {
        channel: app.selected_channel_id.clone(),
        inner,
        total_rows: total.min(u16::MAX as u32) as u16,
        top: top.min(u16::MAX as u32) as u16,
    };
    app.pane_scroll_hint = match &app.pane_last {
        Some(last)
            if last.channel == view.channel
                && last.inner == view.inner
                && last.total_rows == view.total_rows
                && last.top != view.top =>
        {
            Some(crate::console::backend::RegionScroll {
                area: inner,
                rows: view.top as i32 - last.top as i32,
            })
        }
        _ => None,
    };
    app.pane_last = Some(view);

    // Nothing on the pane says who wrote the message it opens on when
    // that message began above the top row: a long one scrolled into, or
    // one grouped under a message that is off the pane. Name them in the
    // pane's own title, where it costs none of the rows -- there are only
    // as many as the terminal gives, and every one of them is a line of
    // somebody's message. Where the header that names them sits depends
    // on the scroll, so the layout cannot answer this.
    let block = match top_author(app, layout.as_deref(), pre_rows, top, &messages) {
        Some((name, color)) => titled_block(
            vec![
                Span::raw(" Messages "),
                Span::styled("\u{2191} ", crate::ui::theme::muted_style()),
                Span::styled(
                    name,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
            ],
            app.focus == Focus::Messages,
        ),
        None => block,
    };

    // only the lines on screen are handed to the paragraph
    let (lines, offset) = model.window(top, pane_visible);
    let paragraph = Paragraph::new(Text::from(lines))
        .block(block.clone())
        .wrap(Wrap { trim: false })
        .scroll((offset, 0));
    frame.render_widget(paragraph, area);

    // The selected message carries a header of its own inside a group,
    // and the author's picture with it, for when the header that names
    // them has been scrolled off the top. While that one is on the pane
    // as well the picture would be the same face twice, so drop the
    // marker cells of this one and leave the margin blank: the layout
    // cannot decide it, having no idea where the pane is scrolled to.
    let margin = Margin {
        avatars: app.avatars_enabled(),
    };
    if let Some(layout) = &layout
        && margin.avatars
        && let Some(selected) = app.selected_message_index
        && layout
            .group_head
            .get(selected)
            .is_some_and(|&head| head != selected)
        && let Some(head_row) = block_top_row(layout, pre_rows, layout.group_head[selected])
        && let Some(selected_row) = block_top_row(layout, pre_rows, selected)
        && head_row >= top
        && let Some(down) = selected_row.checked_sub(top)
        && down < inner.height as u32
    {
        // the avatar hangs off the header row and the one under it
        let right = (inner.x + margin.width(RowKind::Header) as u16).min(inner.x + inner.width);
        let y0 = inner.y + down as u16;
        let buf = frame.buffer_mut();
        for y in y0..y0.saturating_add(2).min(inner.y + inner.height) {
            for x in inner.x..right {
                if crate::app::media_marker(buf[(x, y)].style()).is_some() {
                    buf[(x, y)]
                        .set_style(Style::default().underline_color(ratatui::style::Color::Reset));
                }
            }
        }
    }
}

/// The pane row a message's block starts on.
fn block_top_row(layout: &PaneLayout, pre_rows: u32, index: usize) -> Option<u32> {
    let &(b0, _) = layout.line_ranges.get(index)?;
    Some(pre_rows + layout.cum.get(b0).copied()?)
}

/// The message whose block covers pane row `row`. None where the row is
/// above the first message or on a blank line between two blocks: there
/// the message below it starts with everything it has.
fn message_at_row(layout: &PaneLayout, pre_rows: u32, row: u32) -> Option<usize> {
    layout
        .line_ranges
        .iter()
        .position(|&(b0, b1)| match (layout.cum.get(b0), layout.cum.get(b1)) {
            (Some(&start), Some(&end)) => (pre_rows + start..pre_rows + end).contains(&row),
            _ => false,
        })
}

/// Who wrote the message the pane opens on, where the header that names
/// them is above the top row and the pane itself therefore says nowhere.
fn top_author(
    app: &App,
    layout: Option<&PaneLayout>,
    pre_rows: u32,
    top: u32,
    messages: &[crate::api::types::MessageResponse],
) -> Option<(String, ratatui::style::Color)> {
    let layout = layout?;
    let index = message_at_row(layout, pre_rows, top)?;
    if naming_header_row(app, layout, pre_rows, index)? >= top {
        return None;
    }
    let message = messages.get(index)?;
    let guild = app.guild_id_for_channel(message.channel_id.as_str());
    Some((
        app.shown_name_for_user(guild.as_deref(), &message.author),
        app.member_name_color(
            guild.as_deref(),
            &message.author.id,
            message.author.id == app.me.id,
        ),
    ))
}

/// The pane row of the header that names message `index`: its own block,
/// or the one it is grouped under, which is where its author's name was
/// written. A selected message is named by its own block whatever the
/// grouping, since it is given a header of its own.
fn naming_header_row(app: &App, layout: &PaneLayout, pre_rows: u32, index: usize) -> Option<u32> {
    let named_by = match layout.group_head.get(index) {
        Some(&head) if head != index && app.selected_message_index != Some(index) => head,
        _ => index,
    };
    block_top_row(layout, pre_rows, named_by)
}

/// Draw custom emoji pictures over the marked placeholder cells the paragraph
/// just laid out. Scanning the buffer means wrapping and scrolling are already
/// accounted for.
pub fn overlay_custom_emojis(frame: &mut Frame, inner: Rect, app: &App) {
    let slots = app.custom_emoji_slots.borrow();
    if slots.is_empty() {
        return;
    }
    let w = crate::app::CUSTOM_EMOJI_CELLS;
    let buf = frame.buffer_mut();
    for y in inner.y..inner.y.saturating_add(inner.height) {
        let mut x = inner.x;
        while x.saturating_add(w) <= inner.x.saturating_add(inner.width) {
            let slot = crate::app::custom_emoji_marker_slot(buf[(x, y)].style());
            let Some(k) = slot else {
                x += 1;
                continue;
            };
            // both cells must belong to the same placeholder (a wrap could
            // split one; then draw nothing rather than over a neighbour)
            let whole = (1..w).all(|dx| {
                crate::app::custom_emoji_marker_slot(buf[(x + dx, y)].style()) == Some(k)
            });
            if whole
                && let Some(id) = slots.get(k)
                && let Some((serial, frame_idx, picture)) = app.custom_emoji_current(id)
            {
                let rect = Rect::new(x, y, w, 1);
                match picture {
                    crate::app::Picture::Terminal(tp) => {
                        place_terminal_picture(app, buf, rect, 0, 1, serial, frame_idx, tp);
                    }
                    crate::app::Picture::Pixels(img) => {
                        app.pixel_placements
                            .borrow_mut()
                            .push(crate::console::raster::Placement {
                                area: rect,
                                image: img.clone(),
                            });
                    }
                }
            }
            x += w;
        }
    }
}

/// Put rows `r0..r1` of a terminal picture on the cells `rect` (the rows
/// of the block that are on screen). The cells are skipped, so the text
/// under them is never rewritten while the picture is there; the rows that
/// carry escape sequences get a sentinel cell, and the backend prints the
/// picture at it instead of the cell. The sentinel carries the picture,
/// the frame and the run, so it changes exactly when what to print does.
#[allow(clippy::too_many_arguments)]
fn place_terminal_picture(
    app: &App,
    buf: &mut ratatui::buffer::Buffer,
    rect: Rect,
    r0: u16,
    r1: u16,
    serial: u16,
    frame: usize,
    picture: &crate::app::TerminalPicture,
) {
    if picture.area().width > rect.width {
        // encoded for a wider block than it has: printing it would spill
        return;
    }
    let Some(printout) = picture.printout(r0, r1, app.cell_px.1, app.image_picker.as_ref()) else {
        return;
    };
    let bottom = rect.y.saturating_add(rect.height);
    for y in rect.y..bottom {
        for x in rect.x..rect.x.saturating_add(rect.width) {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_skip(true);
            }
        }
    }
    let transmit = printout
        .transmit
        .map(|seq| (((serial as u32) << 8) | (frame as u32 & 0xFF), seq));
    let mut pictures = app.terminal_pictures.borrow_mut();
    for (dy, data) in printout.rows {
        let y = rect.y.saturating_add(dy.saturating_sub(r0));
        if y >= bottom {
            continue;
        }
        if let Some(cell) = buf.cell_mut((rect.x, y)) {
            let style = crate::app::picture_sentinel_style(cell.style(), serial, frame)
                .fg(ratatui::style::Color::Rgb(0xC0, r0 as u8, r1 as u8));
            cell.set_skip(false);
            cell.set_style(style);
        }
        pictures.insert(
            (rect.x, y),
            crate::console::backend::PicturePrint {
                data,
                area: rect,
                transmit: transmit.clone(),
            },
        );
    }
}

/// The rows of a picture's pixels for block rows `r0..r1`, for the console.
fn crop_rows(
    img: &image::RgbaImage,
    r0: u16,
    r1: u16,
    cell_h: u32,
) -> std::sync::Arc<image::RgbaImage> {
    let y0 = (r0 as u32 * cell_h).min(img.height());
    let h = ((r1 - r0) as u32 * cell_h).min(img.height() - y0).max(1);
    std::sync::Arc::new(image::imageops::crop_imm(img, 0, y0, img.width(), h).to_image())
}

/// Draw the pictures whose marker blocks the message pane laid out this
/// draw, where the whole block is on screen (a block cut by the pane's edge
/// draws nothing until it scrolls fully in), and ask for the ones that are
/// not loaded yet. Scanning the buffer means wrapping and scrolling are
/// already accounted for.
pub fn overlay_media(frame: &mut Frame, area: Rect, app: &App) {
    let slots = app.media_slots.borrow();
    if slots.is_empty() {
        return;
    }
    /// A block's rows found on screen: cut by the pane's edge, only a run
    /// of them may be there.
    struct Seen {
        x: u16,
        y_first: u16,
        r_first: u16,
        r_last: u16,
        consistent: bool,
    }
    let draw = app.draw_serial.get();
    let buf = frame.buffer_mut();
    let mut seen: HashMap<usize, Seen> = HashMap::new();
    for y in area.y..area.y.saturating_add(area.height) {
        let right = area.x.saturating_add(area.width);
        let mut x = area.x;
        while x < right {
            let Some((k, r)) = crate::app::media_marker(buf[(x, y)].style()) else {
                x += 1;
                continue;
            };
            let mut run: u16 = 1;
            while x.saturating_add(run) < right
                && crate::app::media_marker(buf[(x + run, y)].style()) == Some((k, r))
            {
                run += 1;
            }
            if let Some(slot) = slots.get(k)
                && run == slot.cols
                && r < slot.rows
            {
                match seen.get_mut(&k) {
                    None => {
                        seen.insert(
                            k,
                            Seen {
                                x,
                                y_first: y,
                                r_first: r,
                                r_last: r,
                                consistent: true,
                            },
                        );
                    }
                    Some(entry) => {
                        // the rows must follow each other down the screen
                        if entry.x != x
                            || r != entry.r_last + 1
                            || y - entry.y_first != r - entry.r_first
                        {
                            entry.consistent = false;
                        }
                        entry.r_last = r;
                    }
                }
            }
            x = x.saturating_add(run);
        }
    }
    let mut wanted = app.media_wanted.borrow_mut();
    for (k, block) in seen {
        let slot = &slots[k];
        if !block.consistent {
            continue;
        }
        let (r0, r1) = (block.r_first, block.r_last + 1);
        match app.media.lookup(&slot.key, draw) {
            crate::media::Lookup::Ready(frames) => {
                let animated = !app.ui_settings.performance_mode && frames.is_animated();
                let (frame_idx, picture) = if animated {
                    app.note_animation(frames);
                    frames.current(std::time::Instant::now(), draw)
                } else {
                    (0, &frames.frames[0])
                };
                let rect = Rect::new(block.x, block.y_first, slot.cols, r1 - r0);
                match picture {
                    crate::app::Picture::Terminal(tp) => {
                        place_terminal_picture(
                            app,
                            buf,
                            rect,
                            r0,
                            r1,
                            frames.serial,
                            frame_idx,
                            tp,
                        );
                    }
                    crate::app::Picture::Pixels(img) => {
                        let image = if r0 == 0 && r1 == slot.rows {
                            img.clone()
                        } else {
                            crop_rows(img, r0, r1, app.cell_px.1)
                        };
                        app.pixel_placements
                            .borrow_mut()
                            .push(crate::console::raster::Placement { area: rect, image });
                    }
                }
                if animated {
                    app.media_animation_seen.set(true);
                }
            }
            crate::media::Lookup::Missing => {
                if !wanted.iter().any(|w| w.key == slot.key) {
                    wanted.push(slot.clone());
                }
            }
            crate::media::Lookup::Pending => {}
        }
    }
}

fn render_voice(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(channel) = app.active_channel() {
        lines.push(Line::styled(
            format!("\u{1F50A} {}", channel.name),
            Style::default()
                .fg(crate::ui::theme::voice_color())
                .add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "Voice is view-only in this client: you cannot join, transmit, or hear audio here.",
            crate::ui::theme::dim_style(),
        ));
        lines.push(Line::styled(
            "Below is who appears connected from gateway state (informational only).",
            crate::ui::theme::muted_style(),
        ));
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "Members",
            Style::default()
                .fg(crate::ui::theme::accent())
                .add_modifier(Modifier::BOLD),
        ));

        let members = app.voice_members_for_active_channel();
        if members.is_empty() {
            lines.push(Line::styled(
                "Nobody listed.",
                crate::ui::theme::dim_style(),
            ));
        } else {
            for member in members {
                lines.push(Line::styled(
                    format!("  {member}"),
                    Style::default().fg(crate::ui::theme::text()),
                ));
            }
        }
    } else {
        lines.push(Line::styled(
            "Select a voice channel.",
            crate::ui::theme::dim_style(),
        ));
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(block("Voice (read-only)", app.focus == Focus::Messages));
    frame.render_widget(paragraph, area);
}

fn render_link(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(channel) = app.active_channel() {
        lines.push(Line::styled(
            format!("\u{1F517} {}", channel.name),
            Style::default()
                .fg(crate::ui::theme::link_color())
                .add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::from(""));
        if let Some(url) = &channel.url {
            lines.push(Line::styled(
                url.clone(),
                Style::default().fg(crate::ui::theme::accent()),
            ));
            lines.push(Line::from(""));
            lines.push(Line::styled(
                "Press Enter to open this link in your browser.",
                crate::ui::theme::dim_style(),
            ));
        } else {
            lines.push(Line::styled(
                "This link channel has no URL set.",
                crate::ui::theme::dim_style(),
            ));
        }
    } else {
        lines.push(Line::styled(
            "Select a channel.",
            crate::ui::theme::dim_style(),
        ));
    }

    let paragraph =
        Paragraph::new(Text::from(lines)).block(block("Link", app.focus == Focus::Messages));
    frame.render_widget(paragraph, area);
}

pub fn format_timestamp(raw: &str, clock_12h: bool) -> String {
    use chrono::{DateTime, Local, Utc};

    if let Ok(dt) = raw.parse::<DateTime<Utc>>() {
        let local = dt.with_timezone(&Local);
        let now = Local::now();
        if clock_12h {
            if local.date_naive() == now.date_naive() {
                let h = local.format("%I").to_string();
                let h = h.trim_start_matches('0');
                let h = if h.is_empty() { "12" } else { h };
                return format!("{h}{}", local.format(":%M %p"));
            }
            let h = local.format("%I").to_string();
            let h = h.trim_start_matches('0');
            let h = if h.is_empty() { "12" } else { h };
            let tail = local.format(":%M %p").to_string();
            return format!("{}{h}{tail}", local.format("%m/%d "));
        }
        if local.date_naive() == now.date_naive() {
            return local.format("%H:%M").to_string();
        }
        return local.format("%m/%d %H:%M").to_string();
    }

    if raw.len() >= 16 {
        raw[11..16].to_string()
    } else {
        raw.to_string()
    }
}

fn block(title: &str, focused: bool) -> Block<'static> {
    titled_block(vec![Span::raw(format!(" {title} "))], focused)
}

/// The same box with a title of styled spans, for the message pane, which
/// names the author of the message it opens on in its own.
fn titled_block(title: Vec<Span<'static>>, focused: bool) -> Block<'static> {
    Block::default()
        .title(Line::from(title))
        .borders(Borders::ALL)
        .border_style(crate::ui::theme::focused_border(focused))
        .style(Style::default().bg(crate::ui::theme::bg()))
}

#[cfg(test)]
mod bottom_tests {
    use crate::api::types::{
        ChannelResponse, MessageResponse, UserPartialResponse, UserPrivateResponse,
        WellKnownFluxerResponse,
    };
    use crate::app::{App, ServerSelection};
    use crate::config::UiSettings;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn user(id: &str) -> UserPartialResponse {
        UserPartialResponse {
            id: id.into(),
            username: id.into(),
            discriminator: "0001".into(),
            ..Default::default()
        }
    }

    /// Message `n`: text of a length that varies with `n`, some with a
    /// second line or a link, always ending in a token that marks its
    /// last row.
    fn msg(n: u64, channel: &str) -> MessageResponse {
        let base = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor ";
        let len = (n * 37 % 140) as usize;
        let mut s: String = base.chars().cycle().take(len).collect();
        if n.is_multiple_of(5) {
            s.push_str("\nsecond line of the message");
        }
        if n.is_multiple_of(11) {
            s.push_str("\n\nafter a blank line");
        }
        if n.is_multiple_of(7) {
            s = format!("{s} https://example.org/{n}");
        }
        MessageResponse {
            id: n.to_string(),
            channel_id: channel.into(),
            author: user(if n.is_multiple_of(3) { "ann" } else { "bob" }),
            content: format!("m{n} {s} end{n}."),
            timestamp: format!("2026-09-06T10:{:02}:{:02}.000Z", (n / 60) % 60, n % 60),
            ..Default::default()
        }
    }

    fn app_with(channels: &[&str]) -> App {
        let me = UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        };
        let private: Vec<ChannelResponse> = channels
            .iter()
            .map(|c| ChannelResponse {
                id: c.to_string(),
                kind: 1,
                recipients: vec![user("other")],
                ..Default::default()
            })
            .collect();
        App::new(
            WellKnownFluxerResponse::default(),
            me,
            None,
            Vec::new(),
            private,
            ServerSelection::DirectMessages,
            Some(channels[0].to_string()),
            UiSettings::default(),
        )
    }

    fn draw(app: &mut App, w: u16, h: u16) -> Vec<String> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    /// Which rows carry a picture's marker cells. The placeholder glyph
    /// goes to the terminal as a space, so the marker is what says a
    /// block's cells were reserved.
    fn marked_rows(app: &mut App, w: u16, h: u16) -> Vec<bool> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).any(|x| crate::app::media_marker(buf[(x, y)].style()).is_some()))
            .collect()
    }

    /// The pane's last content row: above its bottom border, which sits
    /// above the three-row input box.
    fn bottom_row(rows: &[String]) -> &str {
        &rows[rows.len() - 5]
    }

    fn switch_to(app: &mut App, channel: &str) {
        app.selected_channel_id = Some(channel.to_string());
        app.selected_message_index = None;
        app.message_scroll_from_bottom = 0;
    }

    fn assert_newest_at_bottom(app: &mut App, w: u16, h: u16, n: u64) {
        let rows = draw(app, w, h);
        let token = format!("end{n}.");
        assert!(
            bottom_row(&rows).contains(&token),
            "{w}x{h}: message {n} not on the pane's bottom row:\n{}",
            rows.join("\n")
        );
    }

    #[test]
    fn the_newest_message_ends_on_the_bottom_row() {
        for (w, h) in [(60u16, 20u16), (80, 30), (100, 24), (37, 12)] {
            let mut app = app_with(&["c1"]);
            for n in 1..=40 {
                app.upsert_message(msg(n, "c1"));
            }
            assert_newest_at_bottom(&mut app, w, h, 40);
            for n in 41..=75 {
                app.upsert_message(msg(n, "c1"));
                assert_newest_at_bottom(&mut app, w, h, n);
            }
        }
    }

    /// Both ends of the selected message on the drawn pane: it must hold
    /// the whole of a message that fits on it.
    fn assert_selection_on_screen(app: &mut App, w: u16, h: u16, what: &str) {
        let idx = app.selected_message_index.expect("a selection");
        let id = app.active_messages()[idx].id.clone();
        let rows = draw(app, w, h);
        for token in [format!("m{id} "), format!("end{id}.")] {
            assert!(
                rows.iter().any(|r| r.contains(&token)),
                "{w}x{h}: {what}: message {id} is selected, {token:?} is not on the pane:\n{}",
                rows.join("\n")
            );
        }
    }

    /// Walking the selection through the history keeps the selected
    /// message on the screen. Selecting a message that is grouped under
    /// the one before it adds a timestamp row, so the pane's content
    /// changes height at the edge of every group; the anchor that keeps a
    /// reader in place when a message arrives must not put the view back
    /// and leave the selection off the pane.
    #[test]
    fn the_selection_stays_on_screen_while_walking_the_history() {
        for (w, h) in [(100u16, 30u16), (80, 20), (60, 24)] {
            let mut app = app_with(&["c1"]);
            for n in 1..=60 {
                app.upsert_message(msg(n, "c1"));
            }
            draw(&mut app, w, h);
            // s: select the newest message
            let count = app.active_messages().len();
            app.selected_message_index = Some(count - 1);
            app.clamp_scroll_to_selected_message();
            assert_selection_on_screen(&mut app, w, h, "s");
            for _ in 1..count {
                app.move_selected_message(-1);
                assert_selection_on_screen(&mut app, w, h, "up");
            }
            assert_eq!(app.selected_message_index, Some(0));
            for _ in 1..count {
                app.move_selected_message(1);
                assert_selection_on_screen(&mut app, w, h, "down");
            }
            assert_eq!(app.selected_message_index, Some(count - 1));
        }
    }

    /// Selecting a message that is off the pane brings it on, even when
    /// it is grouped under the message before it and selecting it is
    /// what changes the pane's height.
    #[test]
    fn selecting_a_message_off_the_pane_scrolls_to_it() {
        let (w, h) = (100u16, 30u16);
        let mut app = app_with(&["c1"]);
        for n in 1..=60 {
            app.upsert_message(msg(n, "c1"));
        }
        draw(&mut app, w, h);
        // 32 follows 31 from the same author within five minutes
        let index = app
            .active_messages()
            .iter()
            .position(|m| m.id == "32")
            .expect("message 32");
        app.selected_message_index = Some(index);
        app.clamp_scroll_to_selected_message();
        assert_selection_on_screen(&mut app, w, h, "selected off the pane");
    }

    /// The `endN.` token of the message a row is the last row of.
    fn end_token(row: &str) -> Option<u64> {
        row.match_indices("end").find_map(|(i, _)| {
            let rest = &row[i + 3..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            match rest[digits.len()..].starts_with('.') {
                true => digits.parse().ok(),
                false => None,
            }
        })
    }

    /// Whoever wrote the message the pane opens on is named on it: by a
    /// header of their own where that is on the pane, and by the pane's
    /// title where the message began above the top row. A row of text
    /// with nobody's name against it anywhere is the bug.
    #[test]
    fn the_pane_names_the_author_of_the_message_it_opens_on() {
        for (w, h) in [(100u16, 16u16), (80, 30), (60, 20)] {
            let mut app = app_with(&["c1"]);
            for n in 1..=60 {
                app.upsert_message(msg(n, "c1"));
            }
            draw(&mut app, w, h);
            let sidebar = (w / 4).clamp(22, 50);
            let (pane_x, text_w) = (sidebar as usize + 1, (w - sidebar - 2) as usize);
            let pane = |row: &String| row.chars().skip(pane_x).take(text_w).collect::<String>();
            for scroll in 0..24u16 {
                app.message_scroll_from_bottom = scroll;
                let rows = draw(&mut app, w, h);
                let titled = rows[1].contains('\u{2191}');
                let first = pane(&rows[2]);
                // A header row names its author itself. So does the
                // message under the blank row that separates two of
                // them, which a header always follows -- a blank row
                // inside a message body does not, and there the title
                // has something to say.
                let separator = first.trim().is_empty() && pane(&rows[3]).contains("#0001");
                if first.contains("#0001") || separator {
                    assert!(
                        !titled,
                        "{w}x{h} scrolled {scroll}: the title names an author the pane \
                         already names:\n{}",
                        rows.join("\n")
                    );
                    continue;
                }
                let n = rows[2..]
                    .iter()
                    .find_map(|r| end_token(&pane(r)))
                    .expect("a message ends somewhere on the pane");
                let author = if n.is_multiple_of(3) { "ann" } else { "bob" };
                assert!(
                    rows[1].contains(&format!("\u{2191} {author}#0001")),
                    "{w}x{h} scrolled {scroll}: the pane opens on {author}'s message {n} \
                     and names nobody:\n{}",
                    rows.join("\n")
                );
            }
        }
    }

    /// A message grouped under one from the same person carries no header
    /// -- until it is the selected one. Then it names its author always,
    /// and shows their picture only where the header that already names
    /// them has gone off the top of the pane: one face, never two.
    #[test]
    fn the_selected_message_names_its_author_even_when_grouped() {
        let (w, h) = (100u16, 30u16);
        let mut app = app_with(&["c1"]);
        // pictures drawn by our own renderer: avatars on
        app.pixel_mode = true;
        app.cell_px = (10, 20);
        assert!(app.avatars_enabled());
        for n in 1..=60 {
            app.upsert_message(msg(n, "c1"));
        }
        draw(&mut app, w, h);
        // 32 follows 31 from bob, within five minutes of it
        let index = app
            .active_messages()
            .iter()
            .position(|m| m.id == "32")
            .expect("message 32");
        app.selected_message_index = Some(index);
        app.clamp_scroll_to_selected_message();

        // selected from below, so it sits on the top row and the header
        // naming bob is off the pane: this one needs the picture
        let rows = draw(&mut app, w, h);
        let marked = marked_rows(&mut app, w, h);
        let body = |rows: &[String]| {
            rows.iter()
                .position(|r| r.contains("m32 "))
                .expect("message 32 on the pane")
        };
        let at = body(&rows);
        assert!(
            !rows.iter().any(|r| r.contains("m31 ")),
            "message 31 should be off the top of the pane:\n{}",
            rows.join("\n")
        );
        assert!(
            rows[at - 1].contains("bob#0001"),
            "the row above the selected message does not name its author:\n{}",
            rows.join("\n")
        );
        assert!(
            marked[at - 1] && marked[at],
            "no avatar block where the header naming the author is off the pane:\n{}",
            rows.join("\n")
        );

        // scrolled back until that header is on the pane as well: it
        // carries bob's picture, and the selected message must not
        // repeat it
        app.scroll_messages_up(6);
        let rows = draw(&mut app, w, h);
        let marked = marked_rows(&mut app, w, h);
        let at = body(&rows);
        let head = rows
            .iter()
            .position(|r| r.contains("m31 "))
            .expect("message 31 on the pane");
        assert!(
            rows[at - 1].contains("bob#0001"),
            "the selected message stops naming its author once scrolled:\n{}",
            rows.join("\n")
        );
        assert!(
            marked[head - 1],
            "the header naming the author lost its picture:\n{}",
            rows.join("\n")
        );
        assert!(
            !marked[at - 1] && !marked[at],
            "the author's picture is drawn twice:\n{}",
            rows.join("\n")
        );
    }

    /// A sticker on a message is named in the pane, and takes a block of
    /// cells for its picture where the terminal draws pictures.
    #[test]
    fn a_sticker_is_named_and_takes_a_picture_block() {
        let mut app = app_with(&["c1"]);
        let mut m = msg(1, "c1");
        m.content = String::new();
        m.stickers.push(crate::api::types::MessageStickerResponse {
            id: "77".into(),
            name: "shipit".into(),
            animated: false,
            nsfw: false,
        });
        app.upsert_message(m);
        let rows = draw(&mut app, 80, 20);
        assert!(
            rows.iter()
                .any(|r| r.contains("shipit") && r.contains("[sticker]")),
            "the sticker is not named:\n{}",
            rows.join("\n")
        );
        assert!(
            app.media_slots.borrow().is_empty(),
            "no picture without a terminal that draws one"
        );

        app.pixel_mode = true;
        app.cell_px = (10, 20);
        let rows = draw(&mut app, 80, 20);
        // one block for the sticker, beside the author's avatar
        let slots = app.media_slots.borrow().clone();
        let sticker_slots: Vec<&crate::app::MediaSlot> = slots
            .iter()
            .filter(|s| s.url.contains("/stickers/77.webp?size="))
            .collect();
        assert_eq!(sticker_slots.len(), 1, "{slots:?}");
        assert_eq!(
            sticker_slots[0].cols,
            sticker_slots[0].rows * 2,
            "a square block of cells"
        );
        assert!(
            marked_rows(&mut app, 80, 20).iter().any(|m| *m),
            "the block's marker cells are not on the pane:\n{}",
            rows.join("\n")
        );
        // U+2800 keeps a block's cells from being wrapped away, but a font
        // without it draws a box: xterm's does, so a block of them shows as
        // hatching wherever no picture has been printed over them. None of
        // them may reach the terminal.
        assert!(
            !rows.iter().any(|r| r.contains('\u{2800}')),
            "a placeholder reached the terminal:\n{}",
            rows.join("\n")
        );
    }

    /// A message with the trimmings: a picture attachment, a reaction, a
    /// reply to the one before, a custom emoji.
    fn rich(n: u64, channel: &str) -> MessageResponse {
        use crate::api::types::{
            MessageAttachmentResponse, MessageReactionResponse, MessageReferenceResponse,
            ReactionEmojiResponse,
        };
        let mut m = msg(n, channel);
        if n.is_multiple_of(2) {
            m.attachments.push(MessageAttachmentResponse {
                id: format!("a{n}"),
                filename: "pic.png".into(),
                url: Some(format!("https://x/{n}.png")),
                content_type: Some("image/png".into()),
                size: Some(1000),
                width: Some(400),
                height: Some(300),
                ..Default::default()
            });
        }
        if n.is_multiple_of(3) {
            m.reactions.push(MessageReactionResponse {
                count: 2,
                me: n.is_multiple_of(6),
                emoji: ReactionEmojiResponse {
                    name: "😀".into(),
                    ..Default::default()
                },
            });
        }
        if n.is_multiple_of(4) {
            m.message_reference = Some(MessageReferenceResponse {
                message_id: (n - 1).to_string(),
                channel_id: channel.into(),
                ..Default::default()
            });
            m.referenced_message = Some(Box::new(msg(n - 1, channel)));
        }
        if n.is_multiple_of(5) {
            m.content = format!(
                "{} <:kekw:{}> end{n}.",
                m.content.trim_end_matches(&format!("end{n}.")),
                900 + n
            );
        }
        m
    }

    /// The pane's bottom row must be the last row of the pane's own
    /// layout, rendered without a height limit: anything else means rows
    /// were miscounted or the view is not at the bottom.
    fn assert_bottom_matches_layout(app: &mut App, w: u16, h: u16) {
        use ratatui::text::Text;
        use ratatui::widgets::{Paragraph, Wrap};
        let rows = draw(app, w, h);
        let layout = app
            .pane_layout
            .borrow()
            .clone()
            .expect("a layout was built");
        let sidebar = (w / 4).clamp(22, 50);
        let text_w = w - sidebar - 2;
        let total = (*layout.cum.last().unwrap()).max(1) as u16;
        let mut t = Terminal::new(TestBackend::new(text_w, total)).unwrap();
        t.draw(|f| {
            f.render_widget(
                Paragraph::new(Text::from(layout.lines.clone())).wrap(Wrap { trim: false }),
                f.area(),
            )
        })
        .unwrap();
        let buf = t.backend().buffer().clone();
        // the placeholder goes out as a space, so compare it as one
        let expected: String = (0..text_w)
            .map(|x| buf[(x, total - 1)].symbol().replace('\u{2800}', " "))
            .collect();
        let pane_x = sidebar as usize + 1;
        let actual: String = bottom_row(&rows)
            .chars()
            .skip(pane_x)
            .take(text_w as usize)
            .collect();
        assert_eq!(
            actual.trim_end(),
            expected.trim_end(),
            "{w}x{h}: the pane's bottom row is not the layout's last row:\n{}",
            rows.join("\n")
        );
    }

    #[test]
    fn with_avatars_and_pictures_the_bottom_row_is_the_layouts_last() {
        for (w, h) in [(60u16, 20u16), (80, 30), (100, 40), (40, 14)] {
            let mut app = app_with(&["c1"]);
            // pictures drawn by our own renderer: avatars and previews on
            app.pixel_mode = true;
            app.cell_px = (10, 20);
            assert!(app.avatars_enabled() && app.inline_media_enabled());
            for n in 1..=40 {
                app.upsert_message(rich(n, "c1"));
            }
            assert_bottom_matches_layout(&mut app, w, h);
            for n in 41..=75 {
                app.upsert_message(rich(n, "c1"));
                assert_bottom_matches_layout(&mut app, w, h);
            }
            app.selected_message_index = Some(app.active_messages().len() - 1);
            assert_bottom_matches_layout(&mut app, w, h);
        }
    }

    /// Real messages, replayed one arrival at a time: set
    /// FLUXER_TUI_REPLAY to a JSON array of messages (newest first, as the
    /// API answers) and FLUXER_TUI_REPLAY_CHANNEL to the channel object.
    #[test]
    fn replayed_real_messages_keep_the_bottom_row() {
        let (Some(path), Some(channel_path)) = (
            std::env::var_os("FLUXER_TUI_REPLAY"),
            std::env::var_os("FLUXER_TUI_REPLAY_CHANNEL"),
        ) else {
            return;
        };
        let raw = std::fs::read_to_string(path).unwrap();
        let mut messages: Vec<MessageResponse> = serde_json::from_str(&raw).unwrap();
        messages.reverse();
        let channel: ChannelResponse =
            serde_json::from_str(&std::fs::read_to_string(channel_path).unwrap()).unwrap();
        let gid = channel.guild_id.clone().expect("a guild channel");
        for (w, h, pixel) in [
            (100u16, 30u16, false),
            (100, 30, true),
            (120, 40, true),
            (80, 24, false),
        ] {
            let me = UserPrivateResponse {
                id: "me".into(),
                ..Default::default()
            };
            let mut app = App::new(
                WellKnownFluxerResponse::default(),
                me,
                None,
                vec![crate::api::types::GuildResponse {
                    id: gid.clone(),
                    name: "g".into(),
                    ..Default::default()
                }],
                Vec::new(),
                ServerSelection::Guild(gid.clone()),
                Some(channel.id.clone()),
                UiSettings::default(),
            );
            app.guild_channels
                .insert(gid.clone(), vec![channel.clone()]);
            app.selected_channel_id = Some(channel.id.clone());
            if pixel {
                app.pixel_mode = true;
                app.cell_px = (10, 20);
            }
            let (first, rest) = messages.split_at(messages.len() / 2);
            app.set_channel_messages(&channel.id, first.to_vec());
            assert_bottom_matches_layout(&mut app, w, h);
            for m in rest {
                app.upsert_message(m.clone());
                assert_bottom_matches_layout(&mut app, w, h);
            }
        }
    }

    #[test]
    fn switching_channels_while_messages_arrive_keeps_the_bottom() {
        let (w, h) = (80u16, 30u16);
        let mut app = app_with(&["c1", "c2"]);
        for n in 1..=40 {
            app.upsert_message(msg(n, "c1"));
        }
        for n in 100..=110 {
            app.upsert_message(msg(n, "c2"));
        }
        assert_newest_at_bottom(&mut app, w, h, 40);
        switch_to(&mut app, "c2");
        assert_newest_at_bottom(&mut app, w, h, 110);
        // updates in both channels while c2 is shown
        app.upsert_message(msg(41, "c1"));
        app.upsert_message(msg(111, "c2"));
        assert_newest_at_bottom(&mut app, w, h, 111);
        switch_to(&mut app, "c1");
        assert_newest_at_bottom(&mut app, w, h, 41);
        app.upsert_message(msg(42, "c1"));
        assert_newest_at_bottom(&mut app, w, h, 42);
        // an edit of the newest message, and one of an older one
        let mut edited = msg(42, "c1");
        edited.content = format!("{} and more words after the edit end42.", edited.content);
        app.upsert_message(edited);
        assert_newest_at_bottom(&mut app, w, h, 42);
        let mut older = msg(30, "c1");
        older.content = "short end30.".into();
        app.upsert_message(older);
        assert_newest_at_bottom(&mut app, w, h, 42);
    }

    #[test]
    fn jumping_back_to_the_latest_after_scrolling_shows_it_whole() {
        let (w, h) = (80u16, 30u16);
        let mut app = app_with(&["c1"]);
        for n in 1..=40 {
            app.upsert_message(msg(n, "c1"));
        }
        assert_newest_at_bottom(&mut app, w, h, 40);
        app.scroll_messages_up(3);
        let _ = draw(&mut app, w, h);
        app.upsert_message(msg(41, "c1"));
        let _ = draw(&mut app, w, h);
        app.jump_to_latest_message();
        assert_newest_at_bottom(&mut app, w, h, 41);
        // the pane changes size (the input box grows) and shrinks back
        assert_newest_at_bottom(&mut app, w, h - 4, 41);
        assert_newest_at_bottom(&mut app, w, h, 41);
    }
}

#[cfg(test)]
mod highlight_tests {
    use crate::api::types::{
        ChannelResponse, MessageReferenceResponse, MessageResponse, UserPartialResponse,
        UserPrivateResponse, WellKnownFluxerResponse,
    };
    use crate::app::{App, Focus, ServerSelection};
    use crate::config::UiSettings;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn user(id: &str) -> UserPartialResponse {
        UserPartialResponse {
            id: id.into(),
            username: id.into(),
            discriminator: "0001".into(),
            ..Default::default()
        }
    }

    fn msg(id: &str, author: &str, content: &str) -> MessageResponse {
        MessageResponse {
            id: id.into(),
            channel_id: "c".into(),
            author: user(author),
            content: content.into(),
            timestamp: format!("2026-09-07T10:{:02}:00.000Z", id.parse::<u64>().unwrap()),
            ..Default::default()
        }
    }

    fn reply_to(mut message: MessageResponse, original: &MessageResponse) -> MessageResponse {
        message.message_reference = Some(MessageReferenceResponse {
            channel_id: "c".into(),
            message_id: original.id.clone(),
            ..Default::default()
        });
        message.referenced_message = Some(Box::new(original.clone()));
        message
    }

    fn app_with(messages: Vec<MessageResponse>, avatars: bool) -> App {
        let me = UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        };
        let channel = ChannelResponse {
            id: "c".into(),
            kind: 1,
            recipients: vec![user("bob")],
            ..Default::default()
        };
        let mut ui = UiSettings::default();
        ui.avatars = avatars;
        ui.inline_media = false;
        let mut app = App::new(
            WellKnownFluxerResponse::default(),
            me,
            None,
            Vec::new(),
            vec![channel],
            ServerSelection::DirectMessages,
            Some("c".into()),
            ui,
        );
        app.focus = Focus::Messages;
        for m in messages {
            app.upsert_message(m);
        }
        app
    }

    fn draw(app: &mut App, w: u16, h: u16) -> Vec<String> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    /// The pane rows, without the sidebar and the borders, keyed by a
    /// word each message ends with.
    fn pane_row_starting_with<'a>(rows: &'a [String], marker: &str) -> &'a str {
        rows.iter()
            .find(|r| r.contains(marker))
            .map(|r| r.as_str())
            .unwrap_or_else(|| panic!("no row contains {marker:?}"))
    }

    /// The two gutter columns of a pane row: the first two columns inside
    /// the message pane's left border, found from its title row.
    fn gutter_of(rows: &[String], row: &str) -> String {
        let title = rows
            .iter()
            .find(|r| r.contains("Messages"))
            .expect("the pane has a title row");
        let corner = title
            .find("\u{250C} Messages")
            .expect("the title row has the pane's corner");
        let border_x = title[..corner].chars().count();
        row.chars().skip(border_x + 1).take(2).collect()
    }

    /// The row above the one containing `body`: a message's header when
    /// `body` is its first line.
    fn row_above<'a>(rows: &'a [String], body: &str) -> &'a str {
        let at = rows
            .iter()
            .position(|r| r.contains(body))
            .unwrap_or_else(|| panic!("no row contains {body:?}"));
        &rows[at - 1]
    }

    #[test]
    fn mentions_and_answers_to_me_get_the_bar_my_own_messages_do_not() {
        let mine = msg("1", "me", "mine one");
        let mut mention = msg("2", "bob", "hey mention");
        mention.mentions = vec![user("me")];
        let answer = reply_to(msg("3", "bob", "your point answer"), &mine);
        let mut own_ping = msg("4", "me", "self ping");
        own_ping.mentions = vec![user("me")];
        let plain = msg("5", "ann", "nothing plain");
        let mut app = app_with(vec![mine, mention, answer, own_ping, plain], false);
        let rows = draw(&mut app, 100, 30);

        // without avatars only the header and context rows have a margin
        assert_eq!(
            gutter_of(&rows, row_above(&rows, "hey mention")),
            "\u{258E} "
        );
        assert_eq!(
            gutter_of(&rows, row_above(&rows, "your point answer")),
            "\u{258E} "
        );
        assert_eq!(
            gutter_of(&rows, pane_row_starting_with(&rows, "@me#0001 - mine one")),
            "\u{258E} "
        );
        assert_eq!(gutter_of(&rows, row_above(&rows, "mine one")), "  ");
        assert_eq!(gutter_of(&rows, row_above(&rows, "self ping")), "  ");
        assert_eq!(gutter_of(&rows, row_above(&rows, "nothing plain")), "  ");
    }

    #[test]
    fn with_avatars_every_row_of_a_mention_carries_the_bar() {
        let mut mention = msg("2", "bob", "first row\nsecond row\nthird row");
        mention.mentions = vec![user("me")];
        let mut app = app_with(vec![msg("1", "ann", "before"), mention], true);
        // the console renderer draws pictures itself: avatars are on
        app.pixel_mode = true;
        let rows = draw(&mut app, 100, 24);
        assert_eq!(gutter_of(&rows, row_above(&rows, "first row")), "\u{258E} ");
        for marker in ["first row", "second row", "third row"] {
            assert_eq!(
                gutter_of(&rows, pane_row_starting_with(&rows, marker)),
                "\u{258E} ",
                "{marker}"
            );
        }
        assert_eq!(gutter_of(&rows, row_above(&rows, "before")), "  ");
        assert_eq!(
            gutter_of(&rows, pane_row_starting_with(&rows, "before")),
            "  "
        );
    }

    #[test]
    fn the_message_a_selected_reply_answers_is_marked() {
        let original = msg("1", "ann", "the original");
        let between = msg("2", "bob", "in between");
        let reply = reply_to(msg("3", "bob", "the reply"), &original);
        let mut app = app_with(vec![original, between, reply], false);
        app.selected_message_index = Some(2);
        let rows = draw(&mut app, 100, 24);
        // the original's header carries the mark, the selected reply's
        // first row (its context row) the arrow, the one between nothing
        assert_eq!(
            gutter_of(&rows, row_above(&rows, "the original")),
            "\u{21A9} "
        );
        assert_eq!(gutter_of(&rows, row_above(&rows, "in between")), "  ");
        assert_eq!(
            gutter_of(
                &rows,
                pane_row_starting_with(&rows, "@ann#0001 - the original")
            ),
            "\u{25B6} "
        );

        app.selected_message_index = None;
        let rows = draw(&mut app, 100, 24);
        assert_eq!(gutter_of(&rows, row_above(&rows, "the original")), "  ");
    }
}

/// A forward keeps everything it is showing in `message_snapshots`: no
/// content, no referenced_message, so the pane has to read them or it
/// draws a header over nothing.
#[cfg(test)]
mod forward_tests {
    use crate::api::types::{
        ChannelResponse, MESSAGE_REFERENCE_FORWARD, MessageAttachmentResponse,
        MessageReferenceResponse, MessageResponse, MessageSnapshotResponse, UserPartialResponse,
        UserPrivateResponse, WellKnownFluxerResponse,
    };
    use crate::app::{App, Focus, ServerSelection};
    use crate::config::UiSettings;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn user(id: &str) -> UserPartialResponse {
        UserPartialResponse {
            id: id.into(),
            username: id.into(),
            discriminator: "0001".into(),
            ..Default::default()
        }
    }

    /// What the server sends for a forward: the note the sender typed as
    /// the content (here none at all), the original under it.
    fn forward(note: &str, snapshot: MessageSnapshotResponse) -> MessageResponse {
        MessageResponse {
            id: "20".into(),
            channel_id: "c".into(),
            author: user("bob"),
            content: note.into(),
            timestamp: "2026-09-12T10:20:00.000Z".into(),
            message_reference: Some(MessageReferenceResponse {
                channel_id: "other".into(),
                message_id: "19".into(),
                reference_type: MESSAGE_REFERENCE_FORWARD,
                ..Default::default()
            }),
            message_snapshots: vec![snapshot],
            ..Default::default()
        }
    }

    fn app_with(message: MessageResponse) -> App {
        let me = UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        };
        let channel = ChannelResponse {
            id: "c".into(),
            kind: 1,
            recipients: vec![user("bob")],
            ..Default::default()
        };
        let mut ui = UiSettings::default();
        ui.avatars = false;
        ui.inline_media = false;
        let mut app = App::new(
            WellKnownFluxerResponse::default(),
            me,
            None,
            Vec::new(),
            vec![channel],
            ServerSelection::DirectMessages,
            Some("c".into()),
            ui,
        );
        app.focus = Focus::Messages;
        app.upsert_message(message);
        app
    }

    fn draw(app: &mut App, w: u16, h: u16) -> Vec<String> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn a_forwarded_message_shows_its_text_and_its_files() {
        let snapshot = MessageSnapshotResponse {
            content: "the kitchen end4.".into(),
            attachments: vec![MessageAttachmentResponse {
                id: "1".into(),
                filename: "floorplan.pdf".into(),
                size: Some(2048),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut app = app_with(forward("", snapshot));
        let rows = draw(&mut app, 100, 24);
        let joined = rows.join("\n");
        assert!(joined.contains("Forwarded"), "{joined}");
        assert!(joined.contains("the kitchen end4."), "{joined}");
        assert!(joined.contains("floorplan.pdf"), "{joined}");
    }

    #[test]
    fn the_note_comes_first_and_the_forwarded_text_under_it() {
        let snapshot = MessageSnapshotResponse {
            content: "forwarded body".into(),
            ..Default::default()
        };
        let message = forward("look at this", snapshot);
        let text = message.display_content();
        assert_eq!(text, "look at this\n\nforwarded body");
        let mut app = app_with(message);
        let rows = draw(&mut app, 100, 24);
        let note = rows.iter().position(|r| r.contains("look at this"));
        let body = rows.iter().position(|r| r.contains("forwarded body"));
        assert!(note.is_some() && body.is_some(), "{rows:?}");
        assert!(note < body, "{rows:?}");
    }

    #[test]
    fn copying_a_forward_hands_over_what_it_shows() {
        let snapshot = MessageSnapshotResponse {
            content: "forwarded body".into(),
            attachments: vec![MessageAttachmentResponse {
                id: "1".into(),
                filename: "cat.png".into(),
                url: Some("https://example.invalid/cat.png".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let text = crate::app::message_copy_text(&forward("", snapshot));
        assert_eq!(text, "forwarded body\nhttps://example.invalid/cat.png");
    }

    /// An ordinary message has no snapshots, and nothing about it changes.
    #[test]
    fn a_plain_message_is_untouched() {
        let mut m = forward("just text", MessageSnapshotResponse::default());
        m.message_reference = None;
        m.message_snapshots.clear();
        assert_eq!(m.display_content(), "just text");
        assert_eq!(m.all_attachments().count(), 0);
    }
}
