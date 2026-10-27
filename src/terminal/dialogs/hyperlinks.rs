use super::render::safe_text;
use crate::app::{ReaderMode, ReaderModel};
use crate::document::text::clip_text;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Paragraph};

pub(in crate::terminal) fn status_hint(model: &ReaderModel) -> Option<String> {
    let ReaderMode::Hyperlinks(selection) = model.view().mode else {
        return None;
    };
    Some(format!(
        "Follow link: {}  (number + Enter, Esc cancels)",
        selection.digits
    ))
}

pub(in crate::terminal) fn draw(
    frame: &mut Frame<'_>,
    model: &ReaderModel,
    style: Style,
    reading_x: usize,
) {
    let ReaderMode::Hyperlinks(selection) = model.view().mode else {
        return;
    };
    let area = frame.area();
    if let Some(placements) = &selection.placements {
        let height = model.view().viewport.rows.max(1);
        for placement in placements {
            let page = placement.row / height;
            let row = placement.row % height;
            let column = reading_x + page * (model.view().width + 7) + placement.column;
            let label = format!("[{}]", placement.number);
            if column < area.width as usize && row < area.height.saturating_sub(1) as usize {
                frame.render_widget(
                    Paragraph::new(label.clone())
                        .style(style.add_modifier(Modifier::REVERSED | Modifier::BOLD)),
                    Rect::new(
                        column as u16,
                        row as u16,
                        (label.len() as u16).min(area.width - column as u16),
                        1,
                    ),
                );
            }
        }
    } else {
        let body = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
        frame.render_widget(Clear, body);
        frame.render_widget(
            Paragraph::new("Links (Up/Down to scroll)").style(style),
            Rect::new(0, 0, area.width, 1.min(body.height)),
        );
        for (row, hint) in selection
            .hints
            .iter()
            .skip(selection.first)
            .take(body.height.saturating_sub(1) as usize)
            .enumerate()
        {
            let label = hint
                .link
                .label
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let text = clip_text(
                &safe_text(&format!("{}: {} -> {}", hint.number, label, hint.link.href)),
                area.width as usize,
            );
            frame.render_widget(
                Paragraph::new(text).style(style),
                Rect::new(0, row as u16 + 1, area.width, 1),
            );
        }
    }
}
