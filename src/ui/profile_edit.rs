//! Your own profile: the things about yourself that can be changed
//! without the server's sudo mode, as a list the keyboard walks. The rows
//! show what is stored now, Enter edits one in the footer, and x clears
//! the ones that can be empty.

use crate::app::{App, PROFILE_EDIT_ROWS, ProfileEditRow};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = app.profile_edit.as_ref() else {
        return;
    };

    let accent = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);
    let text = Style::default().fg(crate::ui::theme::text());
    let dim = crate::ui::theme::dim_style();
    let muted = crate::ui::theme::muted_style();

    let rows: Vec<Line> = PROFILE_EDIT_ROWS
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let selected = index == view.selected;
            let value = app.profile_edit_value(*row);
            let shown = if value.is_empty() {
                "not set".to_string()
            } else {
                value
            };
            // the accent colour is drawn in itself, which says more than
            // its six digits do
            let value_style = match row {
                ProfileEditRow::AccentColour => match app.me.accent_color {
                    Some(c) => Style::default().fg(ratatui::style::Color::Rgb(
                        ((c >> 16) & 0xFF) as u8,
                        ((c >> 8) & 0xFF) as u8,
                        (c & 0xFF) as u8,
                    )),
                    None => muted,
                },
                _ => dim,
            };
            Line::from(vec![
                Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                Span::styled(
                    format!("{:<30}", row.label()),
                    if selected {
                        text.add_modifier(Modifier::BOLD)
                    } else {
                        text
                    },
                ),
                Span::styled(shown, value_style),
            ])
        })
        .collect();

    let footer = match view.editing {
        // the row's value is in the compose box, with everything the box
        // can do: paste, a cursor, a selection, undo
        Some(_) => "typing goes to the Input box below  \u{b7}  paste and every editing key work there  \u{b7}  Enter saves  \u{b7}  Esc keeps it as it was".to_string(),
        // x asked about a row: the one key that clears it is named, and
        // every other one keeps it
        None if view.confirm_clear.is_some() => {
            let label = view
                .confirm_clear
                .map(|row| row.label())
                .unwrap_or_default();
            format!("Clear {label}?  \u{b7}  Enter clears it  \u{b7}  any other key keeps it")
        }
        None => {
            let clearable = app
                .profile_edit_selected_row()
                .is_some_and(|row| row.clearable());
            if clearable {
                "\u{2191}/\u{2193} move  \u{b7}  Enter change it  \u{b7}  x clear it  \u{b7}  Esc close"
                    .to_string()
            } else {
                "\u{2191}/\u{2193} move  \u{b7}  Enter cycle it  \u{b7}  Esc close".to_string()
            }
        }
    };

    let width = 72.min(area.width.saturating_sub(4)).max(30);
    let height = (rows.len() as u16 + 3).min(area.height.saturating_sub(2));
    let popup = centred(area, width, height);
    frame.render_widget(Clear, popup);

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(popup);

    let block = Block::default()
        .title(Line::from(Span::styled(" You ", accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    frame.render_widget(
        Paragraph::new(Text::from(rows))
            .block(block)
            .alignment(Alignment::Left),
        body[0],
    );
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
    use crate::api::types::UserPrivateResponse;
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

    fn app_with_me(me: UserPrivateResponse) -> App {
        App::new(
            Default::default(),
            me,
            None,
            Vec::new(),
            Vec::new(),
            ServerSelection::DirectMessages,
            None,
            Default::default(),
        )
    }

    #[test]
    fn the_rows_show_what_is_stored_and_say_when_nothing_is() {
        let me = UserPrivateResponse {
            id: "me".into(),
            username: "ada".into(),
            global_name: Some("Ada L".into()),
            bio: Some("counting".into()),
            accent_color: Some(0x3498db),
            mention_flags: Some(2),
            ..Default::default()
        };
        let mut app = app_with_me(me);
        app.open_profile_edit();
        let out = drawn(&app, 80, 12);
        assert!(out.contains("Display name"), "{out}");
        assert!(out.contains("Ada L"), "{out}");
        assert!(out.contains("counting"), "{out}");
        assert!(out.contains("#3498db"), "{out}");
        // pronouns are unset, and the row says so rather than sitting blank
        assert!(out.contains("not set"), "{out}");
        assert!(out.contains("do not mention me by default"), "{out}");
    }

    #[test]
    fn the_footer_asks_for_the_value_and_says_what_clears_it() {
        let mut app = app_with_me(UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        });
        app.open_profile_edit();
        let out = drawn(&app, 80, 12);
        assert!(out.contains("x clear it"), "{out}");
        app.profile_edit_move(1);
        app.begin_profile_field(crate::app::ProfileEditRow::Bio, "hello".into());
        let out = drawn(&app, 80, 12);
        assert!(out.contains("typing goes to the Input box below"), "{out}");
        assert_eq!(app.input_text(), "hello");
        assert_eq!(app.focus, crate::app::Focus::Input);
    }

    /// x does not clear on its own: the footer asks, and names Enter.
    #[test]
    fn x_asks_in_the_footer() {
        let mut app = app_with_me(UserPrivateResponse {
            id: "me".into(),
            bio: Some("counting".into()),
            ..Default::default()
        });
        app.open_profile_edit();
        app.profile_edit_move(1);
        app.profile_edit_ask_clear();
        let out = drawn(&app, 80, 12);
        assert!(out.contains("Clear About you?"), "{out}");
        assert!(out.contains("Enter clears it"), "{out}");
        assert!(app.profile_edit_keep().is_some());
        assert!(!drawn(&app, 80, 12).contains("Clear About you?"));
    }

    /// The reply preference cycles rather than being typed, so its footer
    /// offers no clearing.
    #[test]
    fn the_reply_row_cycles() {
        let mut app = app_with_me(UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        });
        app.open_profile_edit();
        app.profile_edit_move(5);
        assert_eq!(
            app.profile_edit_selected_row(),
            Some(crate::app::ProfileEditRow::ReplyMentions)
        );
        assert_eq!(app.next_reply_mention_flag(), 1);
        app.me.mention_flags = Some(2);
        assert_eq!(app.next_reply_mention_flag(), 0);
        let out = drawn(&app, 80, 12);
        assert!(out.contains("Enter cycle it"), "{out}");
    }
}
