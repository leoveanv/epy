use super::render::safe_text;
use crate::document::text::clip_text;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub(in crate::terminal) fn dialog_window(frame: &mut Frame<'_>, title: &str, style: Style) -> Rect {
    let area = frame.area();
    let rectangle = Rect::new(
        2.min(area.width),
        1.min(area.height),
        area.width.saturating_sub(4),
        area.height.saturating_sub(3),
    );
    frame.render_widget(Clear, rectangle);
    let block = Block::default()
        .title(safe_text(title))
        .borders(Borders::ALL)
        .style(style);
    let inside = block.inner(rectangle);
    frame.render_widget(block, rectangle);
    inside
}

pub(in crate::terminal) struct ListRows {
    pub area: Rect,
    pub selected: usize,
    pub first: usize,
    pub style: Style,
}

pub(in crate::terminal) fn draw_rows<I, S>(frame: &mut Frame<'_>, rows: ListRows, labels: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let first =
        crate::app::mouse::first_visible_item(rows.first, rows.selected, rows.area.height as usize);
    for (row, label) in labels
        .into_iter()
        .skip(first)
        .take(rows.area.height as usize)
        .enumerate()
    {
        let style = if first + row == rows.selected {
            rows.style.add_modifier(Modifier::REVERSED)
        } else {
            rows.style
        };
        frame.render_widget(
            Paragraph::new(clip_text(
                &safe_text(label.as_ref()),
                rows.area.width as usize,
            ))
            .style(style),
            Rect::new(rows.area.x, rows.area.y + row as u16, rows.area.width, 1),
        );
    }
}

pub(in crate::terminal) fn popup(area: Rect) -> Rect {
    Rect::new(
        1.min(area.width),
        1.min(area.height),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}

pub(in crate::terminal) struct ListPaint<'a> {
    pub title: &'a str,
    pub labels: &'a [String],
    pub selected: usize,
    pub first: usize,
    pub empty_message: Option<&'a str>,
    pub style: Style,
}

pub(in crate::terminal) fn draw_list(
    frame: &mut Frame<'_>,
    title: &str,
    labels: &[String],
    selected: usize,
    style: Style,
) {
    draw_scrolled_list(
        frame,
        ListPaint {
            title,
            labels,
            selected,
            first: 0,
            empty_message: None,
            style,
        },
    );
}

pub(in crate::terminal) fn draw_scrolled_list(frame: &mut Frame<'_>, list: ListPaint<'_>) {
    let ListPaint {
        title,
        labels,
        selected,
        first,
        empty_message,
        style,
    } = list;
    let area = popup(frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(style);
    let inside = block.inner(area);
    frame.render_widget(block, area);
    if labels.is_empty() {
        if let Some(message) = empty_message {
            frame.render_widget(Paragraph::new(message).style(style), inside);
        }
        return;
    }
    draw_rows(
        frame,
        ListRows {
            area: inside,
            selected,
            first,
            style,
        },
        labels,
    );
}
