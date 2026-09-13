//! The channel menu: `a` on the channel list, for the community channel
//! the cursor is on. One overlay for the three states — the actions, the
//! kinds of channel a new one can be, and the second press a deletion
//! asks for — with a footer that takes a name, a topic or a number when a
//! row needs one, the same shape as the community menu.

use crate::app::{App, ChannelAdminMode, NEW_CHANNEL_KINDS};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.channel_admin.as_ref() else {
        return;
    };

    let accent = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);
    let text = Style::default().fg(crate::ui::theme::text());
    let dim = crate::ui::theme::dim_style();

    let title = match view.mode {
        ChannelAdminMode::Actions => format!(" #{} ", view.channel_name),
        ChannelAdminMode::NewKind => " What kind of channel? ".to_string(),
        ChannelAdminMode::ConfirmDelete => " Are you sure? ".to_string(),
    };
    let (rows, footer): (Vec<(String, String, bool)>, &str) = match view.mode {
        ChannelAdminMode::Actions => (
            view.actions
                .iter()
                .map(|a| {
                    (
                        a.label().to_string(),
                        a.hint().to_string(),
                        a.is_destructive(),
                    )
                })
                .collect(),
            "↑/↓ move  ·  Enter do it  ·  Esc close",
        ),
        ChannelAdminMode::NewKind => (
            NEW_CHANNEL_KINDS
                .iter()
                .map(|(_, label)| ((*label).to_string(), String::new(), false))
                .collect(),
            "↑/↓ move  ·  Enter name it  ·  Esc back",
        ),
        ChannelAdminMode::ConfirmDelete => (
            vec![
                (
                    format!("Yes, delete #{} and everything in it", view.channel_name),
                    String::new(),
                    true,
                ),
                ("No, leave it alone".to_string(), String::new(), false),
            ],
            "↑/↓ move  ·  Enter choose  ·  Esc back",
        ),
    };

    let widest = rows
        .iter()
        .map(|(label, hint, _)| label.chars().count() + hint.chars().count() + 8)
        .max()
        .unwrap_or(24)
        .max(title.chars().count() + 4)
        .max(footer.chars().count() + 4);
    let width = (widest as u16).min(area.width.saturating_sub(4)).max(20);
    let height = (rows.len() as u16 + 3)
        .min(area.height.saturating_sub(2))
        .max(5);

    let popup = centred(area, width, height);
    frame.render_widget(Clear, popup);

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(popup);
    let content = body[0];

    let inner_rows = content.height.saturating_sub(2) as usize;
    let scroll = (view.selected + 1).saturating_sub(inner_rows) as u16;

    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(index, (label, hint, danger))| {
            let selected = index == view.selected;
            let label_style = if *danger {
                Style::default()
                    .fg(crate::ui::theme::danger())
                    .add_modifier(if selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    })
            } else if selected {
                text.add_modifier(Modifier::BOLD)
            } else {
                text
            };
            let mut spans = vec![
                Span::styled(if selected { " ▸ " } else { "   " }, accent),
                Span::styled(label.clone(), label_style),
            ];
            if !hint.is_empty() {
                let pad = (width as usize)
                    .saturating_sub(label.chars().count() + hint.chars().count() + 7);
                spans.push(Span::styled(" ".repeat(pad.max(2)), text));
                spans.push(Span::styled(hint.clone(), dim));
            }
            Line::from(spans)
        })
        .collect();

    let block = Block::default()
        .title(Line::from(Span::styled(title, accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .scroll((scroll, 0))
            .alignment(Alignment::Left),
        content,
    );

    let footer = match &view.input {
        Some(input) => format!(
            "{}: {}\u{2588}  ·  type it  ·  Backspace edits  ·  Enter go  ·  Esc cancel",
            input.prompt(),
            input.text()
        ),
        None => footer.to_string(),
    };
    crate::ui::footer::render(frame, body[1], app, &footer);
}

fn centred(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{CHANNEL_GUILD_TEXT, ChannelResponse, GuildResponse};
    use crate::app::ServerSelection;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn drawn(app: &App, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| render(f, f.area(), app)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A community whose channel the reader can manage: owning it is the
    /// shortest way to every permission.
    fn app_owning_a_channel() -> App {
        let mut me = crate::api::types::UserPrivateResponse::default();
        me.id = "me".into();
        let guild = GuildResponse {
            id: "g".into(),
            name: "ours".into(),
            owner_id: "me".into(),
            ..Default::default()
        };
        let channel = ChannelResponse {
            id: "c".into(),
            kind: CHANNEL_GUILD_TEXT,
            name: "general".into(),
            guild_id: Some("g".into()),
            topic: Some("what we are up to".into()),
            ..Default::default()
        };
        let mut app = App::new(
            Default::default(),
            me,
            None,
            vec![guild],
            Vec::new(),
            ServerSelection::Guild("g".into()),
            Some("c".into()),
            Default::default(),
        );
        app.set_guild_channels("g", vec![channel]);
        app
    }

    #[test]
    fn the_menu_names_the_channel_and_offers_the_rows() {
        let mut app = app_owning_a_channel();
        assert!(app.open_channel_admin());
        let out = drawn(&app, 80, 20);
        assert!(out.contains("#general"), "{out}");
        assert!(out.contains("Make a channel here"), "{out}");
        assert!(out.contains("Clear the topic"), "{out}");
        assert!(out.contains("Delete this channel"), "{out}");
    }

    #[test]
    fn the_deletion_asks_twice_and_names_what_goes() {
        let mut app = app_owning_a_channel();
        assert!(app.open_channel_admin());
        if let Some(view) = app.channel_admin.as_mut() {
            view.mode = ChannelAdminMode::ConfirmDelete;
        }
        let out = drawn(&app, 80, 20);
        assert!(out.contains("Yes, delete #general"), "{out}");
        assert!(out.contains("No, leave it alone"), "{out}");
    }

    #[test]
    fn the_footer_asks_for_the_text_a_row_needs() {
        let mut app = app_owning_a_channel();
        assert!(app.open_channel_admin());
        if let Some(view) = app.channel_admin.as_mut() {
            view.input = Some(crate::app::ChannelAdminInput::Rename("lobby".into()));
        }
        let out = drawn(&app, 80, 20);
        assert!(out.contains("New name: lobby"), "{out}");
    }

    /// Nothing to manage in a conversation, so the key does nothing there.
    #[test]
    fn a_conversation_has_no_channel_menu() {
        let mut app = App::new(
            Default::default(),
            Default::default(),
            None,
            Vec::new(),
            vec![ChannelResponse {
                id: "dm".into(),
                kind: crate::api::types::CHANNEL_DM,
                ..Default::default()
            }],
            ServerSelection::DirectMessages,
            Some("dm".into()),
            Default::default(),
        );
        assert!(!app.open_channel_admin());
        assert!(app.channel_admin.is_none());
    }
}
