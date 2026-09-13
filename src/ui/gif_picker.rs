//! The GIF picker (`/gif`): a search line, the provider's answers under
//! it, and the one under the cursor drawn beside the list where the
//! terminal can draw pictures at all.
//!
//! A GIF is sent as its provider page address rather than as a file: the
//! server unfurls that into the moving picture everybody else sees, which
//! is what the web client posts too.

use crate::app::{App, GifPickerState};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(picker) = &app.gif_picker else {
        return;
    };
    let accent = Style::default()
        .fg(crate::ui::theme::accent())
        .add_modifier(Modifier::BOLD);
    let dim = crate::ui::theme::dim_style();
    let muted = crate::ui::theme::muted_style();
    let text = Style::default().fg(crate::ui::theme::text());

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(8),
            Constraint::Percentage(84),
            Constraint::Percentage(8),
        ])
        .split(area);
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(6),
            Constraint::Percentage(88),
            Constraint::Percentage(6),
        ])
        .split(outer[1]);
    let popup = mid[1];
    frame.render_widget(Clear, popup);

    let title = if picker.query.trim().is_empty() {
        " Send a GIF \u{2014} what the provider is pushing ".to_string()
    } else {
        format!(" Send a GIF \u{2014} {}\u{2588} ", picker.query)
    };
    let block = Block::default()
        .title(Line::from(Span::styled(title, accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(body[0]);

    match &picker.state {
        GifPickerState::Loading => {
            frame.render_widget(Paragraph::new(Span::styled(" Looking…", muted)), columns[0]);
        }
        GifPickerState::Failed(message) => {
            frame.render_widget(
                Paragraph::new(Span::styled(format!(" {message}"), muted)),
                columns[0],
            );
        }
        GifPickerState::Ready(gifs) if gifs.is_empty() => {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    " Nothing came back. Type something else and press Enter.",
                    muted,
                )),
                columns[0],
            );
        }
        GifPickerState::Ready(gifs) => {
            let rows: Vec<ListItem> = gifs
                .iter()
                .enumerate()
                .map(|(index, gif)| {
                    let selected = index == picker.selected;
                    let title = if gif.title.trim().is_empty() {
                        gif.slug.clone()
                    } else {
                        gif.title.clone()
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(if selected { " \u{25B8} " } else { "   " }, accent),
                        Span::styled(
                            title,
                            if selected {
                                text.add_modifier(Modifier::BOLD)
                            } else {
                                text
                            },
                        ),
                        Span::styled(format!("   {}x{}", gif.width, gif.height), dim),
                    ]))
                })
                .collect();
            let visible = columns[0].height as usize;
            let scroll = (picker.selected + 1).saturating_sub(visible);
            frame.render_widget(
                List::new(rows.into_iter().skip(scroll).collect::<Vec<_>>()),
                columns[0],
            );

            // the GIF under the cursor, beside the list, in its own block
            // of marker cells for the media overlay to fill in
            if let Some(gif) = gifs.get(picker.selected) {
                let mut lines = vec![
                    Line::from(Span::styled(
                        if gif.title.trim().is_empty() {
                            gif.slug.clone()
                        } else {
                            gif.title.clone()
                        },
                        text,
                    )),
                    Line::from(Span::styled(
                        format!(
                            "{} \u{b7} {}x{}",
                            gif.provider_label(),
                            gif.width,
                            gif.height
                        ),
                        muted,
                    )),
                    Line::from(Span::raw("")),
                ];
                if app.gif_preview_slot(gif).is_none() {
                    lines.push(Line::from(Span::styled(
                        "(no preview here: this terminal draws no pictures)",
                        muted,
                    )));
                }
                frame.render_widget(Paragraph::new(lines), columns[1]);
                if let Some(slot) = app.gif_preview_slot(gif) {
                    let rect = Rect::new(
                        columns[1].x,
                        columns[1].y + 3,
                        slot.cols.min(columns[1].width),
                        slot.rows.min(columns[1].height.saturating_sub(3)),
                    );
                    let key = app.register_media_slot(slot);
                    let buf = frame.buffer_mut();
                    for r in 0..rect.height {
                        let style = crate::app::media_marker_style(key, r);
                        for c in 0..rect.width {
                            if let Some(cell) = buf.cell_mut((rect.x + c, rect.y + r)) {
                                cell.set_symbol("\u{2800}").set_style(style);
                            }
                        }
                    }
                }
            }
        }
    }

    crate::ui::footer::render(
        frame,
        body[1],
        app,
        "type to search  \u{b7}  Enter search, then Enter sends the one under the cursor  \u{b7}  \u{2191}/\u{2193} move  \u{b7}  Esc close",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{GifMediaFormat, GifResponse};
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
            Default::default(),
            None,
            Vec::new(),
            Vec::new(),
            ServerSelection::DirectMessages,
            None,
            Default::default(),
        )
    }

    fn gif(title: &str) -> GifResponse {
        let mut media = std::collections::HashMap::new();
        media.insert(
            "tinygif".to_string(),
            GifMediaFormat {
                src: "https://example.invalid/tiny.gif".into(),
                proxy_src: "https://media.invalid/tiny.gif".into(),
                width: 137,
                height: 90,
            },
        );
        GifResponse {
            id: title.to_string(),
            slug: title.to_string(),
            title: title.to_string(),
            url: format!("https://example.invalid/gifs/{title}"),
            src: "https://example.invalid/x.webm".into(),
            width: 220,
            height: 229,
            media,
            ..Default::default()
        }
    }

    #[test]
    fn the_results_are_listed_with_the_one_under_the_cursor_beside_them() {
        let mut app = app();
        app.open_gif_picker("goat".into());
        app.set_gifs_loaded("goat", vec![gif("goat banjo"), gif("goat yells")]);
        let out = drawn(&app, 90, 24);
        assert!(out.contains("Send a GIF"), "{out}");
        assert!(out.contains("goat banjo"), "{out}");
        assert!(out.contains("goat yells"), "{out}");
        assert_eq!(app.gif_picker_len(), 2);
        app.gif_picker_move(1);
        assert_eq!(
            app.gif_picker_selected().map(|g| g.id),
            Some("goat yells".into())
        );
    }

    /// An answer for another search is ignored: the reader has typed on.
    #[test]
    fn a_late_answer_for_an_old_search_is_dropped() {
        let mut app = app();
        app.open_gif_picker("goat".into());
        app.set_gifs_loaded("cat", vec![gif("cat")]);
        assert_eq!(app.gif_picker_len(), 0);
        app.set_gifs_loaded("goat", vec![gif("goat")]);
        assert_eq!(app.gif_picker_len(), 1);
    }

    #[test]
    fn what_is_sent_is_the_provider_page_and_the_preview_is_a_gif_not_a_video() {
        let g = gif("goat");
        assert_eq!(g.share_url(), "https://example.invalid/gifs/goat");
        assert_eq!(
            g.preview_format().map(|f| f.proxy_src.as_str()),
            Some("https://media.invalid/tiny.gif")
        );
        // with nothing proxied there is nothing to draw, and the picker
        // says so rather than drawing a blank block
        let mut bare = g.clone();
        bare.media.clear();
        assert!(bare.preview_format().is_none());
        let mut app = app();
        app.open_gif_picker(String::new());
        app.set_gifs_loaded("", vec![bare]);
        let out = drawn(&app, 90, 24);
        assert!(out.contains("no preview here"), "{out}");
    }

    #[test]
    fn an_empty_search_says_what_it_is_showing() {
        let mut app = app();
        app.open_gif_picker(String::new());
        app.set_gifs_loaded("", Vec::new());
        let out = drawn(&app, 90, 20);
        assert!(out.contains("what the provider is pushing"), "{out}");
        assert!(out.contains("Nothing came back"), "{out}");
    }
}
