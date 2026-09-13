use crate::app::{App, Focus, THUMB_ROWS};
use crate::ui::input_word_wrap;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

/// One card of the strip: a staged file or a staged sticker.
struct Card {
    label: String,
    slot: Option<crate::app::MediaSlot>,
    /// Drawn in the card's middle row when there is no thumbnail.
    mark: &'static str,
}

/// The cards of the strip: the staged files first, then the staged
/// stickers.
fn strip_cards(app: &App) -> Vec<Card> {
    let mut cards: Vec<Card> = Vec::new();
    for a in &app.pending_attachments {
        cards.push(Card {
            label: format!("{} {}", a.filename, a.size_label()),
            slot: app.staged_thumbnail_slot(a),
            mark: "\u{1F4CE}",
        });
    }
    for sticker in &app.pending_stickers {
        cards.push(Card {
            label: format!("{} sticker", sticker.name),
            slot: app.staged_sticker_slot(sticker),
            mark: "\u{1F5BC}",
        });
    }
    cards
}

/// Rows the staged files and stickers take above the text: their names,
/// under their thumbnails when any of them can be drawn.
pub fn attachment_strip_rows(app: &App) -> u16 {
    let cards = strip_cards(app);
    if cards.is_empty() {
        return 0;
    }
    if cards.iter().any(|c| c.slot.is_some()) {
        THUMB_ROWS + 1
    } else {
        1
    }
}

/// The strip: one card per staged file or sticker, side by side, `width`
/// cells wide. A card is its thumbnail's marker cells (the media overlay
/// draws the picture there) over its name; a file with no picture gets a
/// paper clip. Cards that do not fit are counted at the end.
fn attachment_strip(app: &App, width: u16) -> Vec<Line<'static>> {
    let rows = attachment_strip_rows(app) as usize;
    if rows == 0 {
        return Vec::new();
    }
    let thumb_rows = rows - 1;
    let dim = crate::ui::theme::dim_style();
    let muted = crate::ui::theme::muted_style();
    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new(); rows];
    let mut x = 0usize;
    let mut shown = 0usize;
    let cards = strip_cards(app);
    for card in &cards {
        let (cols, prows) = card
            .slot
            .as_ref()
            .map(|s| (s.cols as usize, s.rows as usize))
            .unwrap_or((0, 0));
        let card_w = cols.max(card.label.width().min(20)).max(6);
        if x + card_w > width as usize {
            break;
        }
        let gap = if shown > 0 { 2 } else { 0 };
        let k = card.slot.clone().map(|s| app.register_media_slot(s));
        for (r, line) in lines.iter_mut().enumerate().take(thumb_rows) {
            line.push(Span::raw(" ".repeat(gap)));
            match k {
                Some(k) if r < prows => {
                    line.push(Span::styled(
                        "\u{2800}".repeat(cols),
                        crate::app::media_marker_style(k, r as u16),
                    ));
                    line.push(Span::raw(" ".repeat(card_w - cols)));
                }
                None if r == thumb_rows / 2 => {
                    let mark_w = card.mark.width();
                    let pad = card_w.saturating_sub(mark_w) / 2;
                    line.push(Span::raw(" ".repeat(pad)));
                    line.push(Span::styled(card.mark, muted));
                    line.push(Span::raw(" ".repeat(card_w.saturating_sub(pad + mark_w))));
                }
                _ => line.push(Span::raw(" ".repeat(card_w))),
            }
        }
        let name = fit(&card.label, card_w);
        let pad = card_w.saturating_sub(name.width());
        let name_line = &mut lines[rows - 1];
        name_line.push(Span::raw(" ".repeat(gap)));
        name_line.push(Span::styled(name, dim));
        name_line.push(Span::raw(" ".repeat(pad)));
        x += gap + card_w;
        shown += 1;
    }
    let left = cards.len().saturating_sub(shown);
    if left > 0 {
        lines[rows - 1].push(Span::styled(format!("  +{left} more"), muted));
    }
    lines.into_iter().map(Line::from).collect()
}

/// `s` cut to `width` cells, with an ellipsis when something was cut.
fn fit(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('\u{2026}');
    out
}

fn input_span_style() -> Style {
    Style::default().fg(crate::ui::theme::text())
}

fn typing_line_with_dots(phrase: &str, dots: &str) -> String {
    let base = phrase.strip_suffix("...").unwrap_or(phrase);
    format!("{base}{dots}")
}

pub fn input_display_row_count(app: &App, inner_width: u16) -> u16 {
    let can_type = app.active_channel_is_text() && app.can_send_in_active_channel();
    if !can_type {
        return 1;
    }
    let strip = attachment_strip_rows(app);
    if !app.input_is_empty() {
        return input_word_wrap::wrapped_row_count(
            &app.input_display_plain(),
            inner_width,
            input_span_style(),
        )
        .saturating_add(strip);
    }
    1u16.saturating_add(strip)
}

/// The "is typing" line of the compose box, for its bottom border.
/// Nobody typing gives `None`; the border stays a plain line.
///
/// The phrase lives in the border, where the web client puts it under
/// the input, so it costs no row: the box is the same height whether a
/// peer types or not, nothing else on the screen moves when one starts
/// or stops, and it stays in view while a message is being written.
fn typing_border_title(app: &App, width: u16) -> Option<Line<'static>> {
    let phrase = app.others_typing_phrase()?;
    let dots = match app.input_bar_anim_phase % 4 {
        0 => "",
        1 => ".",
        2 => "..",
        _ => "...",
    };
    // the corners and a space on each side of the text
    let room = width.saturating_sub(4) as usize;
    let text = fit(&typing_line_with_dots(&phrase, dots), room);
    Some(Line::from(Span::styled(
        format!(" {text} "),
        Style::default()
            .fg(crate::ui::theme::typing_others())
            .add_modifier(Modifier::ITALIC),
    )))
}

pub fn render(frame: &mut Frame, area: Rect, app: &App) -> Option<(u16, u16)> {
    // a profile row borrows the box wherever it is, voice channel or not
    let profile_row = app.profile_field_editing();
    let can_type =
        profile_row.is_some() || (app.active_channel_is_text() && app.can_send_in_active_channel());
    let no_perms =
        profile_row.is_none() && app.active_channel_is_text() && !app.can_send_in_active_channel();
    let voice_only = profile_row.is_none() && app.active_channel_is_voice();

    // a mode says how to finish it and how to get out of it: the box
    // named the mode and left the reader to guess at Esc
    let profile_title = profile_row.map(|row| {
        format!(
            "{} \u{00B7} Enter saves \u{00B7} Esc keeps it as it was",
            row.prompt()
        )
    });
    let title = if let Some(title) = profile_title.as_deref() {
        title
    } else if voice_only {
        "Input (voice not supported)"
    } else if no_perms {
        "Input (no permission)"
    } else if app.edit_target.is_some() {
        "Edit message \u{00B7} Enter saves \u{00B7} Esc cancels"
    } else if app.forward_mode {
        "Forward \u{00B7} Ctrl+K picks the channel \u{00B7} Enter sends \u{00B7} Esc cancels"
    } else if app.reply_to.is_some() {
        "Reply \u{00B7} Enter sends \u{00B7} Esc drops it"
    } else if can_type {
        "Input"
    } else {
        "Input (disabled)"
    };
    let title: String = if app.pending_attachments.is_empty() && app.pending_stickers.is_empty() {
        title.to_string()
    } else {
        format!("{title} · {}", app.attachment_summary())
    };

    let placeholder: Option<String> = if voice_only || no_perms || !can_type {
        None
    } else if app.input_is_empty() && app.recording_secs().is_some() {
        // the microphone is open: that outranks what the box would say
        // about a reply, which the recording will be anyway
        Some(match app.reply_to.as_ref() {
            Some(reply) => format!(
                "\u{25CF} Recording a voice message as a reply to {}\u{2026}  Ctrl+R sends it, Esc throws it away",
                reply.author_name
            ),
            None => {
                "\u{25CF} Recording a voice message\u{2026}  Ctrl+R sends it, Esc throws it away"
                    .to_string()
            }
        })
    } else if app.input_is_empty() {
        Some(if let Some(ref reply) = app.reply_to {
            if app.forward_mode {
                format!(
                    "Forward from {} - optional note, Enter to send",
                    reply.author_name
                )
            } else {
                format!("Replying to {}...", reply.author_name)
            }
        } else {
            "Type a message…  ( / for commands )".to_string()
        })
    } else {
        None
    };

    let (content, style) = if voice_only {
        (
            "This client cannot join or use voice - text input is disabled here.".to_string(),
            crate::ui::theme::muted_style(),
        )
    } else if no_perms {
        (
            "You do not have permission to send messages here.".to_string(),
            crate::ui::theme::muted_style(),
        )
    } else if can_type && !app.input_is_empty() {
        (String::new(), Style::default().fg(crate::ui::theme::text()))
    } else if can_type {
        (
            placeholder.clone().unwrap_or_default(),
            crate::ui::theme::muted_style(),
        )
    } else {
        (
            "Select a text channel to chat.".to_string(),
            crate::ui::theme::muted_style(),
        )
    };

    // while the microphone is open the box says so instead of its mode,
    // in red, with the clock and the keys that end the recording
    let title_line = match app.recording_secs() {
        Some(secs) => Line::from(Span::styled(
            format!(" {}", crate::ui::status_bar::recording_label(secs)),
            crate::ui::status_bar::recording_style(),
        )),
        None => Line::from(Span::styled(
            format!(" {title}"),
            Style::default()
                .add_modifier(Modifier::BOLD)
                .patch(crate::ui::theme::dim_style()),
        )),
    };

    let focused = app.focus == Focus::Input;
    let mut blk = Block::default()
        .title(title_line)
        .borders(Borders::ALL)
        .border_style(crate::ui::theme::focused_border(focused))
        .style(Style::default().bg(crate::ui::theme::bg_secondary()));
    if can_type && let Some(typing) = typing_border_title(app, area.width) {
        blk = blk.title_bottom(typing);
    }

    if can_type && !app.input_is_empty() {
        let char_count = app.input_char_count();
        let max_chars = 2000;
        let count_str = format!(" {char_count}/{max_chars} ");
        let count_style = if char_count > max_chars {
            Style::default()
                .fg(crate::ui::theme::danger())
                .add_modifier(Modifier::BOLD)
        } else if char_count > max_chars - 100 {
            Style::default()
                .fg(ratatui::style::Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            crate::ui::theme::muted_style()
        };

        let right_title = Line::from(Span::styled(count_str, count_style))
            .alignment(ratatui::layout::Alignment::Right);
        blk = blk.title(right_title);
    }

    // The staged files sit above the text. The strip is drawn as it is,
    // never through the word wrapper: ratatui's wrapper makes two rows of
    // a line that is only spaces, and a thumbnail card has such rows under
    // its picture, which pushed the text below the box.
    let inner_w = area.width.saturating_sub(2).max(1);
    let strip: Vec<Line<'static>> = if can_type {
        attachment_strip(app, inner_w)
    } else {
        Vec::new()
    };
    let strip_rows = strip.len() as u16;
    let mut lines: Vec<Line<'static>> = Vec::new();
    if can_type && !app.input_is_empty() {
        lines.extend(app.input_display(true));
    } else {
        lines.push(Line::from(Span::styled(content, style)));
    }
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    let strip_h = strip_rows.min(inner.height);
    if strip_h > 0 {
        let strip_area = Rect::new(inner.x, inner.y, inner.width, strip_h);
        frame.render_widget(Paragraph::new(Text::from(strip)), strip_area);
    }
    let text_area = Rect::new(
        inner.x,
        inner.y.saturating_add(strip_h),
        inner.width,
        inner.height.saturating_sub(strip_h),
    );
    // The cursor row decides how far the text scrolls when it is taller
    // than the box: the row with the cursor is always in view.
    let cursor = if focused && can_type && !app.input_is_empty() {
        Some(input_word_wrap::cursor_col_row(
            &app.input_display_plain(),
            &app.input_head_display_plain(),
            inner_w,
            input_span_style(),
        ))
    } else {
        None
    };
    let visible_rows = text_area.height.max(1);
    let scroll = match cursor {
        Some((_, row)) if row >= visible_rows => row - visible_rows + 1,
        _ => 0,
    };
    let paragraph = Paragraph::new(Text::from(lines))
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, text_area);

    if let Some((col, row)) = cursor {
        let max_x = area.x + area.width.saturating_sub(2);
        let x = (area.x + 1 + col).min(max_x);
        let y = area.y + 1 + strip_rows + (row - scroll);
        Some((x, y))
    } else if focused && can_type {
        Some((area.x + 1, area.y + 1 + strip_rows))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{ChannelResponse, UserPartialResponse, UserPrivateResponse};
    use crate::app::ServerSelection;

    fn dm_app() -> App {
        let me = UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        };
        let channel = ChannelResponse {
            id: "c1".into(),
            kind: 1,
            recipients: vec![UserPartialResponse {
                id: "o".into(),
                username: "o".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        App::new(
            Default::default(),
            me,
            None,
            Vec::new(),
            vec![channel],
            ServerSelection::DirectMessages,
            Some("c1".into()),
            Default::default(),
        )
    }

    #[test]
    fn staged_files_take_a_strip_above_the_text() {
        let mut app = dm_app();
        assert!(app.active_channel_is_text() && app.can_send_in_active_channel());
        assert_eq!(input_display_row_count(&app, 60), 1);
        app.pending_attachments
            .push(crate::media::StagedAttachment::new(
                "notes.txt".into(),
                "text/plain".into(),
                b"hi".to_vec(),
            ));
        // names only where pictures cannot be drawn
        assert_eq!(attachment_strip_rows(&app), 1);
        assert_eq!(input_display_row_count(&app, 60), 2);
        let lines = attachment_strip(&app, 60);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].to_string().contains("notes.txt 2 B"));

        let mut png = Vec::new();
        image::RgbaImage::from_pixel(40, 40, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        app.pending_attachments
            .push(crate::media::StagedAttachment::new(
                "shot.png".into(),
                "image/png".into(),
                png,
            ));
        app.pixel_mode = true;
        app.cell_px = (10, 20);
        assert_eq!(attachment_strip_rows(&app), THUMB_ROWS + 1);
        app.input = "hello".into();
        assert_eq!(input_display_row_count(&app, 60), THUMB_ROWS + 2);
        let lines = attachment_strip(&app, 60);
        assert_eq!(lines.len() as u16, THUMB_ROWS + 1);
        // the picture's marker cells sit on the thumbnail rows
        let slots = app.media_slots.borrow();
        assert_eq!(slots.len(), 1);
        let marked = lines[0]
            .spans
            .iter()
            .filter(|s| crate::app::media_marker(s.style) == Some((0, 0)))
            .count();
        assert_eq!(marked, 1, "{:?}", lines[0]);
        assert!(lines[THUMB_ROWS as usize].to_string().contains("shot.png"));
    }

    #[test]
    fn a_staged_sticker_gets_a_card_of_its_own() {
        let mut app = dm_app();
        app.pending_stickers.push(crate::app::StagedSticker {
            id: "77".into(),
            name: "shipit".into(),
            animated: false,
        });
        // its name alone where pictures cannot be drawn
        assert_eq!(attachment_strip_rows(&app), 1);
        let lines = attachment_strip(&app, 60);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].to_string().contains("shipit sticker"));

        app.pixel_mode = true;
        app.cell_px = (10, 20);
        assert_eq!(attachment_strip_rows(&app), THUMB_ROWS + 1);
        let lines = attachment_strip(&app, 60);
        let slots = app.media_slots.borrow();
        assert_eq!(slots.len(), 1);
        assert!(
            slots[0].url.contains("/stickers/77.webp?size="),
            "{}",
            slots[0].url
        );
        assert!(
            lines[THUMB_ROWS as usize]
                .to_string()
                .contains("shipit sticker")
        );
    }

    fn draw(app: &mut App, w: u16, h: u16) -> Vec<String> {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
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
    fn who_is_typing_sits_in_the_bottom_border_and_moves_nothing() {
        let mut app = dm_app();
        app.focus = Focus::Input;
        let (w, h) = (80u16, 24u16);
        // nobody typing, empty input: a three-row box at the bottom, the
        // pane's border right above it
        let quiet = draw(&mut app, w, h);
        let box_top = h as usize - 3;
        assert!(quiet[box_top].contains('─'), "{}", quiet.join("\n"));
        assert!(quiet[box_top - 1].contains('─'), "{}", quiet.join("\n"));
        assert!(!quiet[h as usize - 1].contains("typing"));

        // a peer starts typing while a message is being written: the
        // phrase is in the bottom border, no row moved
        app.record_typing("c1", "o");
        app.input = "hello".into();
        let busy = draw(&mut app, w, h);
        assert_eq!(quiet[box_top - 1], busy[box_top - 1]);
        assert!(busy[box_top].contains('─'), "{}", busy.join("\n"));
        assert!(busy[box_top + 1].contains("hello"), "{}", busy.join("\n"));
        let bottom = &busy[h as usize - 1];
        assert!(bottom.contains("└ o is typing"), "{}", busy.join("\n"));
        assert!(bottom.ends_with('┘'), "{}", busy.join("\n"));

        // the same with an empty input, and gone once indicators are off
        app.input.clear();
        let empty = draw(&mut app, w, h);
        assert!(empty[h as usize - 1].contains("o is typing"));
        assert!(empty[box_top + 1].contains("Type a message"));
        app.ui_settings.show_typing_indicators = false;
        let off = draw(&mut app, w, h);
        assert!(
            !off[h as usize - 1].contains("typing"),
            "{}",
            off.join("\n")
        );
    }

    #[test]
    fn the_text_stays_in_the_box_under_a_thumbnail() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut app = dm_app();
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(40, 40, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        app.pending_attachments
            .push(crate::media::StagedAttachment::new(
                "shot.png".into(),
                "image/png".into(),
                png,
            ));
        app.pixel_mode = true;
        app.cell_px = (11, 25);
        app.input = "hello there".into();
        app.focus = Focus::Input;
        let mut terminal = Terminal::new(TestBackend::new(114, 54)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let row = |y: u16| -> String {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect()
        };
        let rows: Vec<String> = (0..buf.area.height).map(row).collect();
        let bottom = buf.area.height - 1;
        assert!(
            rows[bottom as usize].starts_with('└'),
            "{:?}",
            rows[bottom as usize]
        );
        // the 40x40 picture is two rows tall in 11x25 cells: its marker
        // rows, two blank rows, the name, then the text, all inside
        assert!(
            rows[(bottom - 1) as usize].contains("hello there"),
            "{rows:#?}"
        );
        assert!(
            rows[(bottom - 2) as usize].contains("shot.png"),
            "{rows:#?}"
        );
        // the placeholder goes out as a space; the marker is what says the
        // thumbnail's cells were reserved
        let marked = |y: u16| {
            (0..buf.area.width).any(|x| crate::app::media_marker(buf[(x, y)].style()).is_some())
        };
        assert!(marked(bottom - 5), "{rows:#?}");
        assert!(marked(bottom - 6), "{rows:#?}");
        assert!(rows[(bottom - 7) as usize].contains("Input"), "{rows:#?}");
    }

    #[test]
    fn cards_that_do_not_fit_are_counted() {
        let mut app = dm_app();
        for i in 0..6 {
            app.pending_attachments
                .push(crate::media::StagedAttachment::new(
                    format!("a-rather-long-file-name-{i}.bin"),
                    "application/octet-stream".into(),
                    vec![0; 10],
                ));
        }
        let lines = attachment_strip(&app, 50);
        assert!(
            lines[0].to_string().contains("more"),
            "{:?}",
            lines[0].to_string()
        );
    }
}
