//! Where the account is signed in (Alt+Z): what the server knows about
//! each live session, newest activity first.
//!
//! Only a read. Ending a session needs the server's *sudo mode* -- the
//! account password or a second factor, proved on the request -- and this
//! client signs in by desktop handoff and holds neither, so the overlay
//! says where that can be done rather than offering a key that would fail.

use crate::app::{App, SessionsState};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.sessions.as_ref() else {
        return;
    };
    let accent = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);
    let text = Style::default().fg(crate::ui::theme::text());
    let dim = crate::ui::theme::dim_style();
    let muted = crate::ui::theme::muted_style();

    let rows: Vec<Line> = match &view.state {
        SessionsState::Loading => vec![Line::from(Span::styled("  Loading…", muted))],
        SessionsState::Failed(message) => {
            vec![Line::from(Span::styled(format!("  {message}"), muted))]
        }
        SessionsState::Ready(sessions) if sessions.is_empty() => {
            vec![Line::from(Span::styled("  Nothing signed in.", muted))]
        }
        SessionsState::Ready(sessions) => sessions
            .iter()
            .enumerate()
            .flat_map(|(index, session)| {
                let selected = index == view.selected;
                let info = session.client_info.as_ref();
                // the platform is the closest thing to "what this is",
                // with the browser and the operating system beside it
                let what = info
                    .and_then(|i| i.platform.clone())
                    .or_else(|| info.and_then(|i| i.browser.clone()))
                    .or_else(|| info.and_then(|i| i.os.clone()))
                    .unwrap_or_else(|| "an unrecognised client".to_string());
                let device = info
                    .map(|i| i.device.clone())
                    .filter(|d| !d.is_empty())
                    .unwrap_or_default();
                let when = session
                    .approx_last_used_at
                    .as_deref()
                    .map(|t| {
                        crate::ui::message_pane::format_timestamp(t, app.ui_settings.clock_12h)
                    })
                    .unwrap_or_else(|| "unknown".to_string());
                let where_ = info
                    .and_then(|i| i.location.as_ref())
                    .map(|l| l.label())
                    .filter(|l| !l.is_empty())
                    .unwrap_or_default();
                let mut second = String::from("     ");
                if let Some(ip) = session.masked_ip.as_deref().filter(|ip| !ip.is_empty()) {
                    second.push_str(ip);
                }
                if !where_.is_empty() {
                    if second.trim().is_empty() {
                        second.push_str(&where_);
                    } else {
                        second.push_str(&format!(" \u{b7} {where_}"));
                    }
                }
                vec![
                    Line::from(vec![
                        Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                        Span::styled(
                            what,
                            if selected {
                                text.add_modifier(Modifier::BOLD)
                            } else {
                                text
                            },
                        ),
                        Span::styled(
                            if device.is_empty() {
                                String::new()
                            } else {
                                format!("   {device}")
                            },
                            dim,
                        ),
                        Span::styled(format!("   last used {when}"), muted),
                    ]),
                    Line::from(Span::styled(second, muted)),
                ]
            })
            .collect(),
    };

    let mut lines = rows;
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        "  Ending a session needs your password or a second factor, which this client never holds:",
        muted,
    )));
    lines.push(Line::from(Span::styled(
        "  sign out of the others from the web client's settings.",
        muted,
    )));

    let width = 84.min(area.width.saturating_sub(4)).max(40);
    let height = (lines.len() as u16 + 2)
        .min(area.height.saturating_sub(2))
        .max(7);
    let popup = centred(area, width, height);
    frame.render_widget(Clear, popup);

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(popup);

    let inner = body[0].height.saturating_sub(2) as usize;
    let scroll = (view.selected * 2 + 2).saturating_sub(inner) as u16;

    let block = Block::default()
        .title(Line::from(Span::styled(
            " Where you are signed in ",
            accent,
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .scroll((scroll, 0))
            .alignment(Alignment::Left),
        body[0],
    );
    crate::ui::footer::render(
        frame,
        body[1],
        app,
        "\u{2191}/\u{2193} move  \u{b7}  R reload  \u{b7}  Esc close",
    );
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
    use crate::api::types::{
        AuthSessionResponse, ClientInfoResponse, ClientLocationResponse, UserPrivateResponse,
    };
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
        App::new(
            Default::default(),
            UserPrivateResponse {
                id: "me".into(),
                ..Default::default()
            },
            None,
            Vec::new(),
            Vec::new(),
            ServerSelection::DirectMessages,
            None,
            Default::default(),
        )
    }

    #[test]
    fn each_session_says_what_it_is_and_where_it_was() {
        let mut app = app();
        app.open_sessions();
        app.set_sessions_loaded(vec![
            AuthSessionResponse {
                id_hash: "abc".into(),
                client_info: Some(ClientInfoResponse {
                    platform: Some("Fluxer Linux".into()),
                    os: Some("Linux".into()),
                    browser: None,
                    device: "desktop".into(),
                    location: Some(ClientLocationResponse {
                        city: Some("Madrid".into()),
                        region: None,
                        country: Some("ES".into()),
                    }),
                }),
                masked_ip: Some("81.32.x.x".into()),
                approx_last_used_at: Some("2026-09-13T09:00:00.000Z".into()),
            },
            AuthSessionResponse {
                id_hash: "def".into(),
                ..Default::default()
            },
        ]);
        let out = drawn(&app, 84, 20);
        assert!(out.contains("Where you are signed in"), "{out}");
        assert!(out.contains("Fluxer Linux"), "{out}");
        assert!(out.contains("desktop"), "{out}");
        assert!(out.contains("81.32.x.x"), "{out}");
        assert!(out.contains("Madrid, ES"), "{out}");
        // a session the server could not parse still shows
        assert!(out.contains("an unrecognised client"), "{out}");
        // and the overlay says why there is no key to end one
        assert!(
            out.contains("needs your password or a second factor"),
            "{out}"
        );
        assert_eq!(app.sessions_len(), 2);
        app.sessions_move(1);
        assert_eq!(app.sessions.as_ref().map(|v| v.selected), Some(1));
    }

    #[test]
    fn a_failure_is_shown_as_it_came() {
        let mut app = app();
        app.open_sessions();
        app.set_sessions_failed("Could not read them: 500".into());
        let out = drawn(&app, 84, 12);
        assert!(out.contains("Could not read them: 500"), "{out}");
        assert_eq!(app.sessions_len(), 0);
    }
}
