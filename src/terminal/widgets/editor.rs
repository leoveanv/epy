use super::render::safe_text;
use crate::app::editor::{EditorLayout, EditorState};
use crate::document::text::clip_text;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub(in crate::terminal) enum TextClipping {
    Graphemes,
    Widget,
}

pub(in crate::terminal) struct EditorRows<'a> {
    pub layout: &'a EditorLayout,
    pub area: Rect,
    pub visible_rows: usize,
    pub clipping: TextClipping,
    pub style: Style,
}

pub(in crate::terminal) fn draw_rows(frame: &mut Frame<'_>, rows: EditorRows<'_>) -> usize {
    let first = rows
        .layout
        .cursor_row
        .saturating_sub(rows.visible_rows.saturating_sub(1));
    for (row, line) in rows
        .layout
        .lines
        .iter()
        .skip(first)
        .take(rows.visible_rows)
        .enumerate()
    {
        let y = rows.area.y.saturating_add(row as u16);
        if y >= frame.area().height {
            continue;
        }
        let text = safe_text(line);
        let text = match rows.clipping {
            TextClipping::Graphemes => clip_text(&text, rows.area.width as usize),
            TextClipping::Widget => text,
        };
        frame.render_widget(
            Paragraph::new(text).style(rows.style),
            Rect::new(rows.area.x, y, rows.area.width, 1),
        );
    }
    first
}

pub(in crate::terminal) struct Prompt<'a> {
    pub editor: &'a EditorState,
    pub title: &'a str,
    pub accept_label: &'a str,
    pub style: Style,
}

pub(in crate::terminal) fn draw(frame: &mut Frame<'_>, prompt: Prompt<'_>) {
    let area = frame.area();
    let height = area.height.saturating_sub(1).min(5);
    let rectangle = Rect::new(
        1.min(area.width),
        area.height.saturating_sub(height + 1),
        area.width.saturating_sub(2),
        height,
    );
    let block = Block::default()
        .title(prompt.title)
        .borders(Borders::ALL)
        .style(prompt.style);
    let inside = block.inner(rectangle);
    frame.render_widget(Clear, rectangle);
    frame.render_widget(block, rectangle);
    let layout = prompt.editor.layout(inside.width.max(1) as usize);
    let visible = inside.height.saturating_sub(1) as usize;
    let first = draw_rows(
        frame,
        EditorRows {
            layout: &layout,
            area: inside,
            visible_rows: visible,
            clipping: TextClipping::Graphemes,
            style: prompt.style,
        },
    );
    if inside.height > 0 {
        let accept = prompt.accept_label;
        let labels = match prompt.editor.focus() {
            1 => format!("[{accept}]   Cancel"),
            2 => format!("{accept}   [Cancel]"),
            _ => format!("{accept}   Cancel"),
        };
        frame.render_widget(
            Paragraph::new(clip_text(&labels, inside.width as usize))
                .style(prompt.style.add_modifier(Modifier::BOLD)),
            Rect::new(inside.x, inside.y + inside.height - 1, inside.width, 1),
        );
    }
    if prompt.editor.focus() == 0 && visible > 0 && inside.width > 0 {
        frame.set_cursor_position((
            inside.x
                + layout
                    .cursor_col
                    .min(inside.width.saturating_sub(1) as usize) as u16,
            inside.y + layout.cursor_row.saturating_sub(first).min(visible - 1) as u16,
        ));
    }
}
