use super::render::safe_text;
use crate::app::modes::dictionary::DictionaryMode;
use crate::app::{ReaderMode, ReaderModel};
use crate::document::text::clip_text;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub(in crate::terminal) fn draw(frame: &mut Frame<'_>, model: &ReaderModel, style: Style) {
    let ReaderMode::Dictionary(mode) = model.view().mode else {
        return;
    };
    match mode.as_ref() {
        DictionaryMode::Editing(editor) => super::prompt::draw(
            frame,
            super::prompt::Prompt {
                editor,
                title: "Define word | Enter lookup | Ctrl-U clear | Tab controls | Esc cancel",
                accept_label: "Lookup",
                style,
            },
        ),
        DictionaryMode::Loading { word, .. } => {
            let area = frame.area();
            let height = area.height.min(5);
            let rectangle = Rect::new(
                2.min(area.width),
                area.height.saturating_sub(height) / 2,
                area.width.saturating_sub(4),
                height,
            );
            let inside = window(
                frame,
                rectangle,
                "Dictionary | Loading | Escape cancels",
                style,
            );
            frame.render_widget(
                Paragraph::new(clip_text(&safe_text(word), inside.width as usize)).style(style),
                inside,
            );
        }
        DictionaryMode::Viewing(view) => {
            let area = frame.area();
            let rectangle = Rect::new(
                2.min(area.width),
                2.min(area.height),
                area.width.saturating_sub(4),
                area.height.saturating_sub(4),
            );
            let inside = window(frame, rectangle, &view.page.title, style);
            for (row, line) in view
                .page
                .lines
                .iter()
                .skip(view.top)
                .take(inside.height as usize)
                .enumerate()
            {
                frame.render_widget(
                    Paragraph::new(clip_text(&safe_text(line), inside.width as usize)).style(style),
                    Rect::new(inside.x, inside.y + row as u16, inside.width, 1),
                );
            }
        }
    }
}

fn window(frame: &mut Frame<'_>, area: Rect, title: &str, style: Style) -> Rect {
    let block = Block::default()
        .title(safe_text(title))
        .borders(Borders::ALL)
        .style(style);
    let inside = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    inside
}
