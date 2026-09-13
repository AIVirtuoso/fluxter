//! Finding a member of the open community through the server's own member
//! index: a line to type in and the matches under it. The member list
//! beside the messages shows who is here; this finds somebody who is not
//! on the screen, which is the only way to reach a member of a large
//! community who has not said anything.

use crate::app::{App, MemberSearchState};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.member_search.as_ref() else {
        return;
    };

    let accent = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);
    let text = Style::default().fg(crate::ui::theme::text());
    let dim = crate::ui::theme::dim_style();
    let muted = crate::ui::theme::muted_style();

    let name = app
        .guilds
        .iter()
        .find(|g| g.id == view.guild_id)
        .map(|g| g.name.clone())
        .unwrap_or_default();

    let mut rows: Vec<Line> = Vec::new();
    rows.push(Line::from(vec![
        Span::styled("  Name or nickname: ", muted),
        Span::styled(view.query.clone(), text),
        Span::styled("\u{2588}", accent),
    ]));
    rows.push(Line::from(Span::raw("")));

    match &view.state {
        MemberSearchState::Idle => rows.push(Line::from(Span::styled(
            "  Type a name and press Enter.",
            muted,
        ))),
        MemberSearchState::Running => {
            rows.push(Line::from(Span::styled("  Looking…", muted)));
        }
        MemberSearchState::Indexing => rows.push(Line::from(Span::styled(
            "  The community's member index is still being built; try again shortly.",
            muted,
        ))),
        MemberSearchState::Failed(message) => {
            rows.push(Line::from(Span::styled(format!("  {message}"), muted)));
        }
        MemberSearchState::Ready(members) if members.is_empty() => {
            rows.push(Line::from(Span::styled("  Nobody matched.", muted)));
        }
        MemberSearchState::Ready(members) => {
            for (index, member) in members.iter().enumerate() {
                let selected = index == view.selected;
                // the nickname is what they are called here, with the
                // account's own name beside it when the two differ
                let shown = member.shown_name();
                let tag = format!("{}#{}", member.username, member.discriminator);
                let joined = chrono::DateTime::from_timestamp(member.joined_at, 0)
                    .map(|t| {
                        crate::ui::message_pane::format_timestamp(
                            &t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                            app.ui_settings.clock_12h,
                        )
                    })
                    .unwrap_or_default();
                let roles = match member.role_ids.len() {
                    0 => String::new(),
                    1 => "   1 role".to_string(),
                    n => format!("   {n} roles"),
                };
                let mut spans = vec![
                    Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                    Span::styled(
                        shown,
                        if selected {
                            text.add_modifier(Modifier::BOLD)
                        } else {
                            text
                        },
                    ),
                    Span::styled(format!("   {tag}"), dim),
                ];
                if member.is_bot {
                    spans.push(Span::styled("   bot".to_string(), muted));
                }
                spans.push(Span::styled(roles, muted));
                spans.push(Span::styled(format!("   joined {joined}"), muted));
                rows.push(Line::from(spans));
            }
        }
    }

    let footer = "type a name  \u{b7}  Enter search  \u{b7}  \u{2191}/\u{2193} move  \u{b7}  u profile  \u{b7}  d message them  \u{b7}  Esc close";
    let width = 86.min(area.width.saturating_sub(4)).max(36);
    let height = (rows.len() as u16 + 3)
        .min(area.height.saturating_sub(2))
        .max(7);
    let popup = centred(area, width, height);
    frame.render_widget(Clear, popup);

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(popup);

    let inner = body[0].height.saturating_sub(2) as usize;
    // the two header rows are always on screen, so only the matches scroll
    let scroll = (view.selected + 3).saturating_sub(inner) as u16;

    let block = Block::default()
        .title(Line::from(Span::styled(
            format!(" Find somebody in {name} "),
            accent,
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    frame.render_widget(
        Paragraph::new(Text::from(rows))
            .block(block)
            .scroll((scroll, 0))
            .alignment(Alignment::Left),
        body[0],
    );
    crate::ui::footer::render(frame, body[1], app, footer);
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
        GuildMemberSearchResponse, GuildMemberSearchResult, GuildResponse, UserPrivateResponse,
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

    fn app_with(permissions: u64) -> App {
        let me = UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        };
        let guild = GuildResponse {
            id: "g".into(),
            name: "ours".into(),
            owner_id: "olive".into(),
            permissions: Some(permissions.to_string()),
            ..Default::default()
        };
        App::new(
            Default::default(),
            me,
            None,
            vec![guild],
            Vec::new(),
            ServerSelection::Guild("g".into()),
            None,
            Default::default(),
        )
    }

    #[test]
    fn an_ordinary_member_is_told_rather_than_shown_a_403() {
        let mut app = app_with(crate::permissions::VIEW_CHANNEL);
        assert_eq!(app.open_member_search(), Some(false));
        assert!(app.member_search.is_none());
    }

    #[test]
    fn any_moderator_permission_opens_it() {
        for permission in [
            crate::permissions::KICK_MEMBERS,
            crate::permissions::BAN_MEMBERS,
            crate::permissions::MANAGE_ROLES,
            crate::permissions::MANAGE_NICKNAMES,
            crate::permissions::MODERATE_MEMBERS,
            crate::permissions::MANAGE_GUILD,
        ] {
            let mut app = app_with(permission);
            assert_eq!(app.open_member_search(), Some(true), "{permission:#x}");
        }
    }

    #[test]
    fn the_matches_show_who_they_are_and_the_cursor_walks_them() {
        let mut app = app_with(crate::permissions::KICK_MEMBERS);
        app.open_member_search();
        if let Some(view) = app.member_search.as_mut() {
            view.query = "ad".into();
        }
        app.set_member_search_results(
            "g",
            GuildMemberSearchResponse {
                members: vec![
                    GuildMemberSearchResult {
                        user_id: "u1".into(),
                        username: "ada".into(),
                        discriminator: "0042".into(),
                        nickname: Some("Countess".into()),
                        role_ids: vec!["r1".into(), "r2".into()],
                        joined_at: 1_755_158_400,
                        ..Default::default()
                    },
                    GuildMemberSearchResult {
                        user_id: "u2".into(),
                        username: "adabot".into(),
                        discriminator: "0001".into(),
                        is_bot: true,
                        ..Default::default()
                    },
                ],
                total_result_count: 2,
                indexing: false,
            },
        );
        let out = drawn(&app, 90, 14);
        assert!(out.contains("Find somebody in ours"), "{out}");
        assert!(out.contains("Countess"), "{out}");
        assert!(out.contains("ada#0042"), "{out}");
        assert!(out.contains("2 roles"), "{out}");
        assert!(out.contains("bot"), "{out}");
        assert_eq!(app.member_search_len(), 2);
        app.member_search_move(1);
        assert_eq!(
            app.member_search_selected().map(|m| m.user_id),
            Some("u2".into())
        );
    }

    /// An index still being built is an answer, not an error.
    #[test]
    fn an_index_being_built_says_so() {
        let mut app = app_with(crate::permissions::KICK_MEMBERS);
        app.open_member_search();
        app.set_member_search_results(
            "g",
            GuildMemberSearchResponse {
                members: Vec::new(),
                total_result_count: 0,
                indexing: true,
            },
        );
        let out = drawn(&app, 90, 10);
        assert!(out.contains("still being built"), "{out}");
        assert_eq!(app.member_search_len(), 0);
    }
}
