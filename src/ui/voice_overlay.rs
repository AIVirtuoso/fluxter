//! The voice menu: joining a channel, answering a call, muting, leaving.
//!
//! It also says plainly whether the sound is actually being carried,
//! because this client does the joining and hands the audio to a program
//! on PATH (see `media::voice`), and "in the channel but silent" is a
//! real state somebody can be in.

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.voice_menu.as_ref() else {
        return;
    };
    frame.render_widget(Clear, area);
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(60),
            Constraint::Percentage(20),
        ])
        .split(area);
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(15),
            Constraint::Percentage(70),
            Constraint::Percentage(15),
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
    let muted = crate::ui::theme::muted_style();
    let danger = Style::default().fg(crate::ui::theme::danger());

    let title = match app.voice.as_ref() {
        Some(connection) => {
            let (_, name) = app.channel_location(&connection.channel_id);
            format!(" Voice · {name} ")
        }
        None => " Voice ".to_string(),
    };

    let mut lines: Vec<Line> = Vec::new();

    // where the sound stands, before the things that can be done
    if let Some(connection) = app.voice.as_ref() {
        // two short lines rather than one long one: the popup is narrow
        // and this is the sentence a reader most needs to finish
        let (state, hints, style): (&str, &[&str], _) = if connection.grant.is_none() {
            ("waiting for the connection details", &[], muted)
        } else if connection.media_exited {
            (
                "the sound program stopped",
                &["see the debug log, then", "leave and join again"],
                danger,
            )
        } else if !connection.media_running {
            (
                "no sound is being carried",
                &["install fluxter-phone", "or set [media] voice_command"],
                danger,
            )
        } else {
            ("sound is being carried", &[], muted)
        };
        lines.push(Line::from(vec![
            Span::styled("  ", text),
            Span::styled(state, style),
        ]));
        for hint in hints {
            lines.push(Line::from(vec![
                Span::styled("  ", text),
                Span::styled(*hint, muted),
            ]));
        }
        let members = app.voice_members_for_active_channel();
        if !members.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  with ", muted),
                Span::styled(members.join(", "), text),
            ]));
        }
        lines.push(Line::from(""));
    }

    let actions = app.voice_actions();
    if actions.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Nothing to do here: open a voice channel or a conversation.",
            muted,
        )));
    }
    for (index, action) in actions.iter().enumerate() {
        let selected = index == view.selected;
        let style = match action {
            crate::app::VoiceAction::Leave | crate::app::VoiceAction::Decline => danger,
            _ if selected => text.add_modifier(Modifier::BOLD),
            _ => text,
        };
        lines.push(Line::from(vec![
            Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
            Span::styled(action.label(), style),
        ]));
    }

    let block = Block::default()
        .title(Line::from(Span::styled(title, accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .alignment(Alignment::Left),
        content,
    );

    crate::ui::footer::render(
        frame,
        body[1],
        app,
        "\u{2191}/\u{2193} move  ·  Enter do it  ·  Esc close",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{
        CHANNEL_DM, CHANNEL_GUILD_TEXT, CHANNEL_GUILD_VOICE, ChannelResponse, GuildResponse,
        UserPrivateResponse, VoiceServerUpdateEvent,
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
        let mut me = UserPrivateResponse::default();
        me.id = "me".to_string();
        let mut app = App::new(
            Default::default(),
            me,
            None,
            Vec::new(),
            vec![ChannelResponse {
                id: "dm1".to_string(),
                kind: CHANNEL_DM,
                ..Default::default()
            }],
            ServerSelection::DirectMessages,
            None,
            Default::default(),
        );
        app.guilds.push(GuildResponse {
            id: "g1".to_string(),
            name: "Guild".to_string(),
            ..Default::default()
        });
        app.guild_channels.insert(
            "g1".to_string(),
            vec![
                ChannelResponse {
                    id: "vc1".to_string(),
                    kind: CHANNEL_GUILD_VOICE,
                    guild_id: Some("g1".to_string()),
                    name: "General Voice".to_string(),
                    ..Default::default()
                },
                ChannelResponse {
                    id: "tc1".to_string(),
                    kind: CHANNEL_GUILD_TEXT,
                    guild_id: Some("g1".to_string()),
                    name: "general".to_string(),
                    ..Default::default()
                },
            ],
        );
        app
    }

    #[test]
    fn a_voice_channel_offers_joining_and_a_text_one_offers_nothing() {
        let mut app = app();
        app.selected_server = ServerSelection::Guild("g1".to_string());
        app.selected_channel_id = Some("vc1".to_string());
        app.open_voice_menu();
        assert!(drawn(&app, 60, 14).contains("Join this voice channel"));

        app.selected_channel_id = Some("tc1".to_string());
        let s = drawn(&app, 60, 14);
        assert!(s.contains("Nothing to do here"), "{s}");
    }

    #[test]
    fn a_conversation_offers_ringing_it() {
        let mut app = app();
        app.selected_channel_id = Some("dm1".to_string());
        app.open_voice_menu();
        assert!(drawn(&app, 60, 14).contains("Ring this conversation"));
    }

    #[test]
    fn being_in_a_channel_says_whether_the_sound_is_actually_carried() {
        let mut app = app();
        app.selected_server = ServerSelection::Guild("g1".to_string());
        app.selected_channel_id = Some("vc1".to_string());
        app.set_voice_joining("vc1".to_string(), Some("g1".to_string()));
        app.open_voice_menu();
        // before the grant lands
        assert!(drawn(&app, 70, 16).contains("waiting for the connection details"));
        app.set_voice_grant(VoiceServerUpdateEvent {
            token: "t".to_string(),
            endpoint: "wss://x".to_string(),
            connection_id: "c1".to_string(),
            channel_id: "vc1".to_string(),
            ..Default::default()
        });
        // the grant is in but nothing is carrying it, which is a state
        // worth saying out loud rather than looking connected
        let s = drawn(&app, 70, 16);
        assert!(s.contains("no sound"), "{s}");
        assert!(s.contains("voice_command"), "{s}");
        app.set_voice_media_running(true);
        assert!(drawn(&app, 70, 16).contains("sound is being carried"));
    }

    #[test]
    fn a_call_ringing_offers_answering_and_turning_it_down_first() {
        let mut app = app();
        app.selected_channel_id = Some("dm1".to_string());
        app.upsert_incoming_call("dm1".to_string(), vec!["me".to_string()]);
        app.open_voice_menu();
        let s = drawn(&app, 60, 14);
        assert!(s.contains("Answer the call"), "{s}");
        assert!(s.contains("Turn the call down"), "{s}");
        // and a call ringing for somebody else is not the reader's
        app.upsert_incoming_call("dm1".to_string(), vec!["someone".to_string()]);
        assert!(!drawn(&app, 60, 14).contains("Answer the call"));
    }

    #[test]
    fn muting_and_deafening_swap_their_rows_once_they_are_on() {
        let mut app = app();
        app.set_voice_joining("vc1".to_string(), Some("g1".to_string()));
        app.open_voice_menu();
        let s = drawn(&app, 60, 16);
        assert!(s.contains("Mute yourself"), "{s}");
        assert!(s.contains("Deafen yourself"), "{s}");
        app.set_voice_flags(true, false);
        assert!(drawn(&app, 60, 16).contains("Unmute yourself"));
        // deafening implies muting, so both rows flip
        app.set_voice_flags(false, true);
        let s = drawn(&app, 60, 16);
        assert!(s.contains("Undeafen yourself"), "{s}");
        assert!(s.contains("Unmute yourself"), "{s}");
    }
}
