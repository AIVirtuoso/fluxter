use crate::app::{App, Focus, ServerSelection};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// The keys that work where the reader is standing.
///
/// The bar is the only place a reader who has not opened F1 finds out
/// what a key does, so every mode says something rather than most of
/// them. An overlay covers this line, and puts its own hints on its
/// footer instead (see `ui::footer`).
pub fn hints_for(app: &App) -> String {
    let mut out = match app.focus {
        Focus::Servers => {
            " · j/k servers · Alt+1-9 slots · n notifications · l channels".to_string()
        }
        Focus::Channels => {
            let mut hints =
                " · j/k channels · Enter open · n notifications · i input · R refresh".to_string();
            if app.selected_server == crate::app::ServerSelection::DirectMessages {
                hints.push_str(" · P keep at top · x close");
            }
            hints
        }
        Focus::Messages => {
            if app.selected_message_index.is_some() {
                " · a actions · r reply · y/Y copy · f forward · e react · Ctrl+E/D edit/del"
                    .to_string()
            } else {
                " · s select · / search · U new messages · Alt+A unread · i input".to_string()
            }
        }
        Focus::Input => {
            if app.input_mark || app.input_selection().is_some() {
                " · selecting: Ctrl+C copy · Ctrl+X cut · Ctrl+B/I/S mark · Esc drop".to_string()
            } else {
                " · Alt+Enter newline · Ctrl+F file · Alt+S sticker · Ctrl+K picker · F1 help"
                    .to_string()
            }
        }
    };
    // the member column has no focus of its own, so the bar is the only
    // place its two keys can be offered
    if app.member_list.is_some() {
        out.push_str(" · Alt+J/K members");
    }
    // the two that depend on where the reader has been, so they are only
    // offered when they would do something
    if !matches!(app.focus, Focus::Input) {
        let back = app.can_step_history(true);
        let forward = app.can_step_history(false);
        match (back, forward) {
            (true, true) => out.push_str(" · Alt+←/→ back/forward"),
            (true, false) => out.push_str(" · Alt+← back"),
            (false, true) => out.push_str(" · Alt+→ forward"),
            (false, false) => {}
        }
    }
    out
}

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let server = match &app.selected_server {
        ServerSelection::DirectMessages => "DMs".to_string(),
        ServerSelection::Guild(_) => app.selected_server_name(),
    };

    let mut status_mid = if app.status_message.is_empty() {
        String::new()
    } else {
        format!(" | {}", app.status_message)
    };
    if let Some(player) = app.audio.as_ref()
        && app.status_message.is_empty()
    {
        status_mid = format!(" | \u{266A} {}", player.label);
    }
    // a call, or one ringing, outranks the rest: it is the thing the
    // reader most needs to know is happening
    if let Some(voice) = app.voice_status_line()
        && app.status_message.is_empty()
    {
        status_mid = format!(" | \u{1F50A} {voice}");
    }
    // a recording outranks everything: it is running on the microphone
    // and the reader has to be able to see that it is, so it is drawn in
    // red beside the dim status rather than as part of it
    let recording = app
        .recording_secs()
        .map(|secs| format!(" | {}", recording_label(secs)));

    let hints = hints_for(app);

    // the reader's own presence sits beside the connection, which is the
    // other thing on the bar that says how the client stands
    let own = app.own_status();
    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(
            app.gateway_status.label(),
            crate::ui::theme::gateway_status_style(app.gateway_status),
        ),
        Span::styled(" ", Style::default()),
        crate::ui::presence::dot(own),
        Span::styled(format!(" {}", own.label()), crate::ui::presence::style(own)),
        Span::styled(
            format!(" | {server}{status_mid}"),
            crate::ui::theme::dim_style(),
        ),
        Span::styled(recording.unwrap_or_default(), recording_style()),
        Span::styled(hints, crate::ui::theme::muted_style()),
    ]))
    .style(Style::default().bg(crate::ui::theme::bg_tertiary()));
    frame.render_widget(paragraph, area);
}

/// What a running recording says of itself: a dot that blinks once a
/// second, the time recorded, and the two keys that end it. The dot is
/// U+25CF, which the console fonts have, rather than a record symbol they
/// do not.
pub fn recording_label(secs: i64) -> String {
    let dot = if secs % 2 == 0 {
        "\u{25CF}"
    } else {
        "\u{25CB}"
    };
    format!(
        "{dot} REC {}  Ctrl+R sends \u{b7} Esc throws it away",
        crate::media::format_duration(secs)
    )
}

/// Red and bold: the one thing on the screen that must not be missed
/// while the microphone is open.
pub fn recording_style() -> Style {
    Style::default()
        .fg(crate::ui::theme::danger())
        .add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod recording_tests {
    use super::recording_label;

    #[test]
    fn the_label_carries_the_time_and_blinks_once_a_second() {
        let a = recording_label(2);
        let b = recording_label(3);
        assert!(a.contains("REC 0:02"), "{a}");
        assert!(b.contains("REC 0:03"), "{b}");
        assert!(a.starts_with('\u{25CF}') && b.starts_with('\u{25CB}'));
        assert!(a.contains("Ctrl+R sends") && a.contains("Esc throws it away"));
    }
}
