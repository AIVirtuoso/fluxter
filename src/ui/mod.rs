pub mod ansi_line;
pub mod channel_admin;
pub mod channel_picker;
pub mod command_popup;
pub mod community_overlay;
pub mod conversation_overlay;
pub mod debug_overlay;
pub mod emoji_popup;
pub mod file_picker;
pub mod footer;
pub mod friends_overlay;
pub mod help_overlay;
pub mod image_preview;
pub mod input_bar;
pub(crate) mod input_word_wrap;
pub mod member_pane;
pub mod mention_popup;
pub mod message_actions;
pub mod message_markdown;
pub mod message_pane;
pub mod pings_overlay;
pub mod pins_overlay;
pub mod presence;
pub mod profile_overlay;
pub mod reaction_users_overlay;
pub mod saved_overlay;
pub mod search_overlay;
pub mod server_notifications_overlay;
pub mod settings_overlay;
pub mod sidebar;
pub(crate) mod span_wrap;
pub mod status_bar;
pub mod sticker_picker;
pub mod theme;
pub mod voice_overlay;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::{Clear, Paragraph};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.chafa_preview_cells = image_preview::overlay_chafa_cells(area);
    const MIN_W: u16 = 28;
    const MIN_H: u16 = 8;
    if area.width < MIN_W || area.height < MIN_H {
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(format!(
                "Terminal too small (need at least {MIN_W}×{MIN_H}). Enlarge the window or reduce font size."
            ))
            .style(Style::default().fg(crate::ui::theme::text())),
            area,
        );
        return;
    }

    app.custom_emoji_slots.borrow_mut().clear();
    app.media_slots.borrow_mut().clear();
    app.media_animation_seen.set(false);
    app.animation_delay_seen.set(None);
    app.terminal_pictures.borrow_mut().clear();
    app.pane_scroll_hint = None;
    app.pixel_placements.borrow_mut().clear();
    app.draw_serial.set(app.draw_serial.get().wrapping_add(1));
    let inner_w = area.width.saturating_sub(2).max(1);
    let input_lines = input_bar::input_display_row_count(app, inner_w);
    let input_block_h = input_lines.saturating_add(2).clamp(3, 40);

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(input_block_h),
        ])
        .split(area);

    status_bar::render(frame, root[0], app);

    let sidebar_width = sidebar_width(area.width);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(sidebar_width), Constraint::Min(1)])
        .split(root[1]);

    let server_height = server_list_height(body[0].height);
    let sidebar = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(server_height), Constraint::Min(1)])
        .split(body[0]);

    sidebar::render_servers(frame, sidebar[0], app);
    sidebar::render_channels(frame, sidebar[1], app);
    // the member column takes its width off the messages, and is dropped
    // rather than squeezing them on a narrow terminal
    let members_w = member_pane::width(app, area.width);
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(members_w)])
        .split(body[1]);
    app.chafa_viewport = (
        main[0].width.saturating_sub(2).max(12),
        main[0].height.saturating_sub(2).max(6),
    );
    message_pane::render(frame, main[0], app);
    if members_w > 0 {
        member_pane::render(frame, main[1], app);
    }

    if app.emoji_autocomplete.is_some() {
        emoji_popup::render(frame, root[1], app);
    }
    if app.mention_autocomplete.is_some() {
        mention_popup::render(frame, root[1], app);
    }
    if app.command_autocomplete.is_some() {
        command_popup::render(frame, root[1], app);
    }

    let cursor = input_bar::render(frame, root[2], app);
    if let Some(cursor) = cursor {
        frame.set_cursor_position(cursor);
    }

    if app.show_debug {
        debug_overlay::render(frame, area, app);
    } else if app.show_help {
        help_overlay::render(frame, area, app);
    } else if app.show_settings {
        settings_overlay::render(frame, area, app);
    } else if app.show_server_notifications {
        server_notifications_overlay::render(frame, area, app);
    } else if app.pings.is_some() {
        pings_overlay::render(frame, area, app);
    } else if app.friends.is_some() {
        friends_overlay::render(frame, area, app);
    } else if app.pins.is_some() {
        pins_overlay::render(frame, area, app);
    } else if app.saved.is_some() {
        saved_overlay::render(frame, area, app);
    } else if app.reaction_users.is_some() {
        reaction_users_overlay::render(frame, area, app);
    } else if app.message_actions.is_some() {
        message_actions::render(frame, area, app);
    } else if app.channel_admin.is_some() {
        channel_admin::render(frame, area, app);
    } else if app.search.is_some() {
        search_overlay::render(frame, area, app);
    } else if app.conversation.is_some() {
        conversation_overlay::render(frame, area, app);
    } else if app.community.is_some() {
        community_overlay::render(frame, area, app);
    } else if app.voice_menu.is_some() {
        voice_overlay::render(frame, area, app);
    } else if app.image_preview.is_some() {
        image_preview::render(frame, area, app);
    } else if app.profile.is_some() {
        profile_overlay::render(frame, area, app);
    } else if app.file_picker.is_some() {
        file_picker::render(frame, area, app);
    } else if app.sticker_picker.is_some() {
        sticker_picker::render(frame, area, app);
    } else if app.channel_picker.is_some() {
        channel_picker::render(frame, area, app);
    }

    // Custom emoji pictures go on top of whatever placed their marker cells
    // this frame: message pane, compose box, emoji popup.
    message_pane::overlay_custom_emojis(frame, area, app);
    // Pictures under messages and avatars: the blocks of marker cells the
    // message pane laid out this frame.
    message_pane::overlay_media(frame, area, app);

    if std::mem::take(&mut app.debug_frame_wanted) {
        log_frame_map(frame, app, root[2], input_lines, cursor);
    }

    // The terminal backend reconciles this frame against what the terminal
    // shows. A scroll of the pane is only worth doing when nothing is drawn
    // over it.
    if !app.pixel_mode {
        let popup = app.show_help
            || app.show_debug
            || app.show_settings
            || app.show_server_notifications
            || app.pings.is_some()
            || app.profile.is_some()
            || app.image_preview.is_some()
            || app.file_picker.is_some()
            || app.sticker_picker.is_some()
            || app.channel_picker.is_some()
            || app.emoji_autocomplete.is_some()
            || app.mention_autocomplete.is_some()
            || app.command_autocomplete.is_some();
        let scroll = if popup {
            None
        } else {
            app.pane_scroll_hint.take()
        };
        *app.terminal_frame.borrow_mut() = crate::console::backend::FrameInfo {
            buffer: Some(frame.buffer_mut().clone()),
            scroll,
        };
    }
    blank_placeholders(frame);
}

/// U+2800 stands in for the cells a picture is printed over. It is used
/// rather than a space because it survives being wrapped, and it draws as
/// nothing in a font that has it. xterm's does not, and draws a box in its
/// place, so a block of them shows as hatching wherever a picture has not
/// been printed over them yet.
///
/// By this point the layout is settled and nothing reads these cells by
/// their symbol again, so what goes to the terminal can be a space. The
/// frame kept for the debug panel is taken before this, and keeps them.
fn blank_placeholders(frame: &mut Frame) {
    let area = frame.area();
    let buf = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y))
                && cell.symbol() == "\u{2800}"
            {
                cell.set_symbol(" ");
            }
        }
    }
}

/// The frame as drawn, to the debug log: the layout the compose box got
/// against what it asked for, the cursor, then the map of every cell
/// (see `crate::debug::frame_map`).
fn log_frame_map(
    frame: &mut Frame,
    app: &mut App,
    input_area: ratatui::layout::Rect,
    input_lines: u16,
    cursor: Option<(u16, u16)>,
) {
    let area = frame.area();
    let strip_rows = input_bar::attachment_strip_rows(app);
    crate::debug::log(
        "frame",
        format!(
            "{}x{} cells; compose box rows {}..{} ({} rows, asked for {}: {} of text, {} of staged files); cursor {}; {} pictures placed",
            area.width,
            area.height,
            input_area.y,
            input_area.bottom(),
            input_area.height,
            input_lines.saturating_add(2),
            input_lines.saturating_sub(strip_rows),
            strip_rows,
            cursor.map_or("hidden".to_string(), |(x, y)| format!("({x},{y})")),
            app.media_slots.borrow().len()
        ),
    );
    for (y, row) in crate::debug::frame_map(frame.buffer_mut())
        .iter()
        .enumerate()
    {
        crate::debug::log("frame", format!("{y:>3}|{row}|"));
    }
    app.set_status("Frame map written to the debug log (see /debug).");
}

fn sidebar_width(terminal_width: u16) -> u16 {
    let w = terminal_width / 4;
    w.clamp(22, 50)
}

fn server_list_height(sidebar_height: u16) -> u16 {
    const CHANNEL_BLOCK_MIN: u16 = 3;
    if sidebar_height <= CHANNEL_BLOCK_MIN {
        return 1;
    }
    let max_for_server = sidebar_height.saturating_sub(CHANNEL_BLOCK_MIN);
    let desired = (sidebar_height * 3 / 10).clamp(3, 12);
    desired.min(max_for_server).max(1)
}
