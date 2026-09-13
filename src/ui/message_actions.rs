//! The message actions menu: what the web client puts on a right-click,
//! as a list the keyboard walks. One overlay serves four states — the
//! actions, the files of a message, the report categories, and the second
//! press a destructive action asks for — because each of them is a list
//! of rows and a cursor, and three more overlays would say nothing new.

use crate::app::{App, MessageActionsMode, REPORT_CATEGORIES};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.message_actions.as_ref() else {
        return;
    };

    let accent = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);
    let text = Style::default().fg(crate::ui::theme::text());
    let dim = crate::ui::theme::dim_style();

    let (title, rows, footer): (&str, Vec<(String, String, bool)>, &str) = match &view.mode {
        MessageActionsMode::Actions => (
            " Message ",
            view.actions
                .iter()
                .map(|a| (a.label().to_string(), a.hint().to_string(), false))
                .collect(),
            "↑/↓ move  ·  Enter do it  ·  Esc close",
        ),
        MessageActionsMode::Attachments(items) => (
            " Remove which file? ",
            items
                .iter()
                .map(|(_, name)| (name.clone(), String::new(), false))
                .collect(),
            "↑/↓ move  ·  Enter remove it  ·  Esc back",
        ),
        MessageActionsMode::ReportCategories => (
            " Report this message as ",
            REPORT_CATEGORIES
                .iter()
                .map(|(_, label)| ((*label).to_string(), String::new(), false))
                .collect(),
            "↑/↓ move  ·  Enter send the report  ·  Esc back",
        ),
        MessageActionsMode::UserReportCategories => (
            " Report this account as ",
            crate::app::USER_REPORT_CATEGORIES
                .iter()
                .map(|(_, label)| ((*label).to_string(), String::new(), false))
                .collect(),
            "↑/↓ move  ·  Enter send the report  ·  Esc back",
        ),
        MessageActionsMode::Confirm(action) => (
            " Are you sure? ",
            vec![
                (
                    format!("Yes, {}", lower_first(action.label())),
                    String::new(),
                    true,
                ),
                ("No, leave it alone".to_string(), String::new(), false),
            ],
            "↑/↓ move  ·  Enter choose  ·  Esc back",
        ),
    };

    // the popup is only as tall as it needs to be, and never taller than
    // the screen: a menu that filled the terminal would hide the message
    // it is about
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
                // border, cursor, hint and a gap
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

    crate::ui::footer::render(frame, body[1], app, footer);
}

/// "Clear every reaction" reads badly after "Yes, "; lower the first
/// letter unless the label starts with a name.
fn lower_first(label: &str) -> String {
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
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
    use crate::api::types::{CHANNEL_DM, ChannelResponse, MessageResponse, UserPartialResponse};
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

    fn app_with_message() -> App {
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
        app.selected_channel_id = Some("dm1".to_string());
        app.set_channel_messages(
            "dm1",
            vec![MessageResponse {
                id: "m1".to_string(),
                channel_id: "dm1".to_string(),
                author: UserPartialResponse {
                    id: "u2".to_string(),
                    username: "ada".to_string(),
                    ..Default::default()
                },
                content: "hello".to_string(),
                timestamp: "2026-09-10T10:00:00.000Z".to_string(),
                ..Default::default()
            }],
        );
        app.selected_message_index = Some(0);
        // a link to a message needs the instance's web app address
        app.discovery.endpoints.webapp = "https://fluxer.example".to_string();
        app
    }

    #[test]
    fn the_menu_lists_the_actions_and_marks_the_cursor() {
        let mut app = app_with_message();
        assert!(app.open_message_actions());
        let s = drawn(&app, 80, 30);
        assert!(s.contains("Message"), "{s}");
        assert!(s.contains("Copy a link to it"), "{s}");
        assert!(s.contains("Bookmark"), "{s}");
        // somebody else's message can be reported, and cannot be edited
        assert!(s.contains("Report this message to the moderators"), "{s}");
        assert!(s.contains("Report the account to the moderators"), "{s}");
        assert!(!s.contains("Edit"), "{s}");
        let marked: Vec<&str> = s.lines().filter(|l| l.contains("▸")).collect();
        assert_eq!(marked.len(), 1, "{s}");
    }

    #[test]
    fn choosing_report_swaps_the_menu_for_the_categories() {
        let mut app = app_with_message();
        assert!(app.open_message_actions());
        let report = app
            .message_actions
            .as_ref()
            .unwrap()
            .actions
            .iter()
            .position(|a| *a == crate::app::MessageAction::Report)
            .expect("report row");
        app.message_actions_move(report as isize);
        assert!(app.message_actions_confirm().is_none());
        let s = drawn(&app, 80, 30);
        assert!(s.contains("Report this message as"), "{s}");
        assert!(s.contains("Spam"), "{s}");
        // Esc steps back to the actions rather than closing outright
        app.message_actions_back();
        assert!(drawn(&app, 80, 30).contains("Copy a link to it"));
    }
}
