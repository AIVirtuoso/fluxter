//! Joining, making, browsing and leaving communities, plus a
//! community's own invites. One overlay in three modes, since each of
//! them is a list and a cursor.

use crate::app::{App, CommunityMode, DiscoverState, InvitesState, WebhooksState};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.community.as_ref() else {
        return;
    };
    frame.render_widget(Clear, area);
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(10),
            Constraint::Percentage(80),
            Constraint::Percentage(10),
        ])
        .split(area);
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(10),
            Constraint::Percentage(80),
            Constraint::Percentage(10),
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

    let (title, rows, footer): (String, Vec<Line>, String) = match &view.mode {
        CommunityMode::Menu => {
            let rows = app
                .community_actions()
                .iter()
                .enumerate()
                .map(|(index, action)| {
                    let selected = index == view.selected;
                    Line::from(vec![
                        Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                        Span::styled(
                            action.label(),
                            if selected {
                                text.add_modifier(Modifier::BOLD)
                            } else {
                                text
                            },
                        ),
                    ])
                })
                .collect();
            (
                " Communities ".to_string(),
                rows,
                "\u{2191}/\u{2193} move  ·  Enter do it  ·  Esc close".to_string(),
            )
        }
        CommunityMode::Discover { query, state } => {
            let found = match state {
                DiscoverState::Ready(_) => app.discover_total(),
                _ => 0,
            };
            let shown = match state {
                DiscoverState::Ready(guilds) => guilds.len() as u32,
                _ => 0,
            };
            let mut title = if query.trim().is_empty() {
                " The directory ".to_string()
            } else {
                format!(" The directory  ·  \"{query}\" ")
            };
            // the directory hands back a page at a time, so a search that
            // found more says so rather than looking like the whole of it
            if found > shown {
                title = format!("{}· {shown} of {found} ", title);
            }
            let rows = match state {
                DiscoverState::Idle => {
                    vec![Line::from(Span::styled("  / looks for something.", muted))]
                }
                DiscoverState::Running => {
                    vec![Line::from(Span::styled("  Looking…", muted))]
                }
                DiscoverState::Failed(message) => {
                    vec![Line::from(Span::styled(format!("  {message}"), muted))]
                }
                DiscoverState::Ready(guilds) if guilds.is_empty() => {
                    vec![Line::from(Span::styled(
                        "  Nothing in the directory matched.",
                        muted,
                    ))]
                }
                DiscoverState::Ready(guilds) => guilds
                    .iter()
                    .enumerate()
                    .flat_map(|(index, guild)| {
                        let selected = index == view.selected;
                        let mut lines = vec![Line::from(vec![
                            Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                            Span::styled(
                                guild.name.clone(),
                                if selected {
                                    text.add_modifier(Modifier::BOLD)
                                } else {
                                    text
                                },
                            ),
                            Span::styled(
                                format!(
                                    "   {} online of {}",
                                    guild.online_count, guild.member_count
                                ),
                                dim,
                            ),
                        ])];
                        if !guild.custom_tags.is_empty() {
                            lines.push(Line::from(vec![
                                Span::styled("     ", text),
                                Span::styled(guild.custom_tags.join(" · "), dim),
                            ]));
                        }
                        if let Some(description) = guild
                            .description
                            .as_deref()
                            .filter(|d| !d.trim().is_empty())
                        {
                            lines.push(Line::from(vec![
                                Span::styled("     ", text),
                                Span::styled(
                                    description.to_string(),
                                    if selected { text } else { muted },
                                ),
                            ]));
                        }
                        lines.push(Line::from(""));
                        lines
                    })
                    .collect(),
            };
            (
                title,
                rows,
                "\u{2191}/\u{2193} move  ·  Enter join  ·  / look for something  ·  Esc back"
                    .to_string(),
            )
        }
        CommunityMode::Preview { code, state } => {
            let rows = match state {
                crate::app::PreviewState::Loading => {
                    vec![Line::from(Span::styled("  Looking the invite up…", muted))]
                }
                crate::app::PreviewState::Failed(message) => {
                    vec![Line::from(Span::styled(format!("  {message}"), muted))]
                }
                crate::app::PreviewState::Ready(invite) => {
                    let mut lines = vec![Line::from(vec![
                        Span::styled("  ", text),
                        Span::styled(invite.destination(), text.add_modifier(Modifier::BOLD)),
                    ])];
                    if let Some(description) = invite
                        .guild
                        .as_ref()
                        .and_then(|g| g.description.as_deref())
                        .filter(|d| !d.trim().is_empty())
                    {
                        lines.push(Line::from(vec![
                            Span::styled("  ", text),
                            Span::styled(description.to_string(), muted),
                        ]));
                    }
                    lines.push(Line::from(""));
                    lines.push(Line::from(vec![
                        Span::styled("  ", text),
                        Span::styled(
                            format!(
                                "{} online of {}",
                                invite.presence_count, invite.member_count
                            ),
                            dim,
                        ),
                    ]));
                    if let Some(inviter) = &invite.inviter {
                        lines.push(Line::from(vec![
                            Span::styled("  invited by ", muted),
                            Span::styled(crate::app::display_name(inviter), text),
                        ]));
                    }
                    if let Some(expires) = invite.expires_at.as_deref() {
                        lines.push(Line::from(vec![
                            Span::styled("  runs out ", muted),
                            Span::styled(
                                crate::ui::message_pane::format_timestamp(
                                    expires,
                                    app.ui_settings.clock_12h,
                                ),
                                dim,
                            ),
                        ]));
                    }
                    if invite.temporary {
                        lines.push(Line::from(Span::styled("  membership is temporary", muted)));
                    }
                    lines
                }
            };
            (
                format!(" Invite {code} "),
                rows,
                "Enter join  ·  Esc back".to_string(),
            )
        }
        CommunityMode::Invites { guild_id, state } => {
            let name = app
                .guilds
                .iter()
                .find(|g| &g.id == guild_id)
                .map(|g| g.name.clone())
                .unwrap_or_default();
            let rows = match state {
                InvitesState::Loading => vec![Line::from(Span::styled("  Loading…", muted))],
                InvitesState::Failed(message) => {
                    vec![Line::from(Span::styled(format!("  {message}"), muted))]
                }
                InvitesState::Ready(invites) if invites.is_empty() => vec![Line::from(
                    Span::styled("  No invites yet (+ makes one to this channel).", muted),
                )],
                InvitesState::Ready(invites) => invites
                    .iter()
                    .enumerate()
                    .map(|(index, invite)| {
                        let selected = index == view.selected;
                        let uses = if invite.max_uses > 0 {
                            format!("{} of {} used", invite.uses, invite.max_uses)
                        } else {
                            format!("{} used", invite.uses)
                        };
                        let where_ = invite
                            .channel
                            .as_ref()
                            .map(|c| format!("#{}", c.name))
                            .unwrap_or_default();
                        Line::from(vec![
                            Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                            Span::styled(
                                invite.code.clone(),
                                if selected {
                                    text.add_modifier(Modifier::BOLD)
                                } else {
                                    text
                                },
                            ),
                            Span::styled(format!("   {where_}"), dim),
                            Span::styled(format!("   {uses}"), muted),
                        ])
                    })
                    .collect(),
            };
            (
                format!(" Invites to {name} "),
                rows,
                "\u{2191}/\u{2193} move  ·  y copy the link  ·  + make one  ·  x revoke  ·  Esc back"
                    .to_string(),
            )
        }
        CommunityMode::ConfirmWebhookDelete { name, .. } => {
            let rows = [
                (
                    format!("Yes, delete {name}; its address stops working"),
                    true,
                ),
                ("No, leave it alone".to_string(), false),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (label, danger))| {
                let selected = index == view.selected;
                let style = if danger {
                    Style::default().fg(crate::ui::theme::danger())
                } else {
                    text
                };
                Line::from(vec![
                    Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                    Span::styled(
                        label,
                        if selected {
                            style.add_modifier(Modifier::BOLD)
                        } else {
                            style
                        },
                    ),
                ])
            })
            .collect();
            (
                format!(" Delete {name}? "),
                rows,
                "\u{2191}/\u{2193} move  \u{b7}  Enter choose  \u{b7}  Esc back".to_string(),
            )
        }
        CommunityMode::Webhooks { guild_id, state } => {
            let name = app
                .guilds
                .iter()
                .find(|g| &g.id == guild_id)
                .map(|g| g.name.clone())
                .unwrap_or_default();
            let rows = match state {
                WebhooksState::Loading => vec![Line::from(Span::styled("  Loading…", muted))],
                WebhooksState::Failed(message) => {
                    vec![Line::from(Span::styled(format!("  {message}"), muted))]
                }
                WebhooksState::Ready(hooks) if hooks.is_empty() => vec![Line::from(Span::styled(
                    "  None yet (+ makes one in the channel now open).",
                    muted,
                ))],
                WebhooksState::Ready(hooks) => hooks
                    .iter()
                    .enumerate()
                    .map(|(index, hook)| {
                        let selected = index == view.selected;
                        // where it posts, and who made it; the token is a
                        // credential and is never drawn
                        let channel = app
                            .channel_by_id(&hook.channel_id)
                            .map(|c| format!("#{}", c.name))
                            .unwrap_or_else(|| "#?".to_string());
                        let by = hook
                            .user
                            .as_ref()
                            .map(crate::app::display_name)
                            .unwrap_or_default();
                        Line::from(vec![
                            Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                            Span::styled(
                                hook.name.clone(),
                                if selected {
                                    text.add_modifier(Modifier::BOLD)
                                } else {
                                    text
                                },
                            ),
                            Span::styled(format!("   {channel}"), dim),
                            Span::styled(
                                if by.is_empty() {
                                    String::new()
                                } else {
                                    format!("   made by {by}")
                                },
                                muted,
                            ),
                        ])
                    })
                    .collect(),
            };
            (
                format!(" Webhooks in {name} "),
                rows,
                "\u{2191}/\u{2193} move  \u{b7}  + make one here  \u{b7}  r rename  \u{b7}  y copy its address  \u{b7}  x delete  \u{b7}  Esc back"
                    .to_string(),
            )
        }
    };

    let block = Block::default()
        .title(Line::from(Span::styled(title, accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    let inner = content.height.saturating_sub(2) as usize;
    let scroll = (view.selected + 1).saturating_sub(inner) as u16;

    frame.render_widget(
        Paragraph::new(Text::from(rows))
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
        None => footer,
    };
    crate::ui::footer::render(frame, body[1], app, &footer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{
        ChannelPartialResponse, DiscoveryGuildResponse, GuildResponse, InviteResponse,
    };
    use crate::app::{CommunityInput, ServerSelection};
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
            Default::default(),
            None,
            Vec::new(),
            Vec::new(),
            ServerSelection::DirectMessages,
            None,
            Default::default(),
        )
    }

    fn guild_app() -> App {
        let mut app = app();
        app.guilds.push(GuildResponse {
            id: "g1".to_string(),
            name: "The Fork".to_string(),
            ..Default::default()
        });
        app.selected_server = ServerSelection::Guild("g1".to_string());
        app
    }

    #[test]
    fn the_menu_offers_more_inside_a_community_than_outside_one() {
        let mut app = app();
        app.open_communities();
        let s = drawn(&app, 70, 16);
        assert!(s.contains("Join with an invite"), "{s}");
        assert!(s.contains("Make a community"), "{s}");
        assert!(s.contains("Browse the directory"), "{s}");
        // nothing to leave or invite to while no community is open
        assert!(!s.contains("Leave this community"), "{s}");

        let mut app = guild_app();
        app.open_communities();
        let s = drawn(&app, 70, 16);
        assert!(s.contains("Invites to this community"), "{s}");
        assert!(s.contains("Leave this community"), "{s}");
    }

    #[test]
    fn the_directory_lists_what_came_back_with_its_counts() {
        let mut app = app();
        app.open_communities();
        app.set_discover_running("rust".to_string());
        assert!(drawn(&app, 70, 16).contains("Looking…"));
        app.set_discover_results(vec![DiscoveryGuildResponse {
            id: "g9".to_string(),
            name: "Rustaceans".to_string(),
            description: Some("we talk about rust".to_string()),
            member_count: 400,
            online_count: 42,
            ..Default::default()
        }]);
        let s = drawn(&app, 70, 16);
        assert!(s.contains("Rustaceans"), "{s}");
        assert!(s.contains("42 online of 400"), "{s}");
        assert!(s.contains("we talk about rust"), "{s}");
        assert!(s.contains("\"rust\""), "{s}");
    }

    #[test]
    fn an_invite_list_shows_the_code_where_it_goes_and_its_use() {
        let mut app = guild_app();
        app.open_communities();
        app.open_guild_invites("g1".to_string());
        assert!(drawn(&app, 76, 16).contains("Loading…"));
        app.set_guild_invites(
            "g1",
            vec![InviteResponse {
                code: "abc123".to_string(),
                channel: Some(ChannelPartialResponse {
                    name: "general".to_string(),
                    ..Default::default()
                }),
                uses: 3,
                max_uses: 10,
                ..Default::default()
            }],
        );
        let s = drawn(&app, 76, 16);
        assert!(s.contains("Invites to The Fork"), "{s}");
        assert!(s.contains("abc123"), "{s}");
        assert!(s.contains("#general"), "{s}");
        assert!(s.contains("3 of 10 used"), "{s}");
    }

    #[test]
    fn typing_takes_over_the_footer() {
        let mut app = app();
        app.open_communities();
        if let Some(view) = app.community.as_mut() {
            view.input = Some(CommunityInput::JoinCode("abc".to_string()));
        }
        assert!(drawn(&app, 70, 16).contains("Invite code or link: abc"));
    }
}
