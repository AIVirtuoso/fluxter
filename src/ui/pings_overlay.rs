use crate::app::{App, PingsState};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Rows one ping takes: where and when, who and what, a blank.
const ROWS_PER_PING: usize = 3;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(Clear, area);
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(1),
        ])
        .split(area);
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(32),
            Constraint::Length(2),
        ])
        .split(outer[1]);
    let popup = mid[1];
    frame.render_widget(Clear, popup);
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(popup);
    let content = body[0];

    let accent = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);
    let text = Style::default().fg(crate::ui::theme::text());
    let dim = crate::ui::theme::dim_style();
    let muted = crate::ui::theme::muted_style();
    let block = Block::default()
        .title(Line::from(Span::styled(" Pings ", accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    let Some(view) = app.pings.as_ref() else {
        return;
    };
    let (lines, scroll): (Vec<Line>, u16) = match &view.state {
        PingsState::Loading => (vec![Line::from(Span::styled("  Loading…", muted))], 0),
        PingsState::Failed(message) => (
            vec![Line::from(Span::styled(format!("  {message}"), muted))],
            0,
        ),
        PingsState::Ready(messages) if messages.is_empty() => (
            vec![Line::from(Span::styled(
                "  Nobody has mentioned you lately.",
                muted,
            ))],
            0,
        ),
        PingsState::Ready(messages) => {
            let mut lines = Vec::with_capacity(messages.len() * ROWS_PER_PING);
            for (index, message) in messages.iter().enumerate() {
                let selected = index == view.selected;
                let (guild, channel) = app.ping_location(message);
                let place = match guild {
                    Some(guild) => format!("#{channel} · {guild}"),
                    None => channel,
                };
                let when = crate::ui::message_pane::format_timestamp(
                    &message.timestamp,
                    app.ui_settings.clock_12h,
                );
                lines.push(Line::from(vec![
                    Span::styled(if selected { " ▸ " } else { "   " }, accent),
                    Span::styled(when, if selected { accent } else { dim }),
                    Span::styled("  ", text),
                    Span::styled(place, if selected { accent } else { dim }),
                ]));
                let guild_id = app.guild_id_for_channel(&message.channel_id);
                let author = app.shown_name_for_user(guild_id.as_deref(), &message.author);
                let mut row = vec![
                    Span::styled("     ", text),
                    Span::styled(
                        format!("{author}: "),
                        if selected {
                            text.add_modifier(Modifier::BOLD)
                        } else {
                            text
                        },
                    ),
                ];
                row.extend(preview_spans(
                    app,
                    message,
                    if selected { text } else { muted },
                ));
                lines.push(Line::from(row));
                lines.push(Line::from(""));
            }
            let inner = content.height.saturating_sub(2) as usize;
            let bottom = (view.selected + 1) * ROWS_PER_PING;
            (lines, bottom.saturating_sub(inner) as u16)
        }
    };

    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .scroll((scroll, 0))
        .alignment(Alignment::Left);
    frame.render_widget(paragraph, content);

    crate::ui::footer::render(
        frame,
        body[1],
        app,
        "↑/↓ move · Enter go to it · x dismiss · X all · R reload · Esc close",
    );
}

/// The message on one row: its lines joined, mentions and emoji shown as
/// in the chat; attachments counted when there is no text. The row is
/// clipped at the popup's edge.
pub fn preview_spans(
    app: &App,
    message: &crate::api::types::MessageResponse,
    base: Style,
) -> Vec<Span<'static>> {
    let content = message.display_content();
    if content.trim().is_empty() {
        let n = message.all_attachments().count();
        let what = if n == 1 {
            "1 attachment".to_string()
        } else if n > 1 {
            format!("{n} attachments")
        } else {
            "(no text)".to_string()
        };
        return vec![Span::styled(what, base.add_modifier(Modifier::ITALIC))];
    }
    let mut out = Vec::new();
    for (i, line) in crate::ui::message_markdown::content_lines(&content, app)
        .into_iter()
        .enumerate()
    {
        if i > 0 {
            out.push(Span::styled(" ", base));
        }
        out.extend(line.into_iter().map(|span| {
            let style = span.style;
            Span::styled(span.content, base.patch(style))
        }));
    }
    out
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

    fn app() -> App {
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
        app.open_pings();
        app
    }

    fn ping(id: &str, content: &str) -> MessageResponse {
        MessageResponse {
            id: id.to_string(),
            channel_id: "dm1".to_string(),
            author: UserPartialResponse {
                id: "u2".to_string(),
                username: "ada".to_string(),
                ..Default::default()
            },
            content: content.to_string(),
            timestamp: "2026-09-08T10:00:00.000Z".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn the_list_shows_who_where_and_what_and_marks_the_selected_one() {
        let mut app = app();
        assert!(drawn(&app, 80, 20).contains("Loading…"));
        app.set_pings_loaded(vec![ping("2", "second\nline"), ping("1", "first")]);
        let s = drawn(&app, 80, 20);
        assert!(s.contains("Pings"), "{s}");
        assert!(s.contains("ada: second line"), "{s}");
        assert!(s.contains("ada: first"), "{s}");
        let marked: Vec<&str> = s.lines().filter(|l| l.contains("▸")).collect();
        assert_eq!(marked.len(), 1, "{s}");
        assert!(marked[0].contains("ada"), "{s}");
        app.pings_move(1);
        let s = drawn(&app, 80, 20);
        let marked: Vec<&str> = s.lines().filter(|l| l.contains("▸")).collect();
        assert_eq!(marked.len(), 1);
        // the selected ping's rows stay in view on a short terminal
        let many: Vec<MessageResponse> = (0..12).map(|i| ping(&i.to_string(), "m")).collect();
        app.set_pings_loaded(many);
        app.pings_move(11);
        let s = drawn(&app, 80, 12);
        assert!(s.lines().any(|l| l.contains("▸")), "{s}");
    }

    #[test]
    fn an_empty_list_and_a_failure_say_so() {
        let mut app = app();
        app.set_pings_loaded(Vec::new());
        assert!(drawn(&app, 80, 12).contains("Nobody has mentioned you lately."));
        app.set_pings_failed("Failed to load pings: 500".to_string());
        assert!(drawn(&app, 80, 12).contains("Failed to load pings: 500"));
    }
}
