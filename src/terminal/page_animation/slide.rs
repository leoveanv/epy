use super::PageDirection;
use crate::document::text::cell_width;
use ratatui::buffer::{Buffer, Cell, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::Style;

pub(super) struct PaneSlide {
    pub area: Rect,
    pub style: Style,
    pub exposed: u16,
    pub direction: PageDirection,
}

struct RowSlide {
    left: u16,
    row: u16,
    source_start: usize,
    source_end: usize,
    destination_start: u16,
}

pub(super) fn paint_pane(buffer: &mut Buffer, slide: PaneSlide) {
    if has_graphics(buffer, slide.area) {
        return;
    }
    for row in slide.area.top()..slide.area.bottom() {
        let source = (slide.area.left()..slide.area.right())
            .filter_map(|column| buffer.cell((column, row)).cloned())
            .collect::<Vec<_>>();
        clear_row(buffer, &slide, row);
        let movement = match slide.direction {
            PageDirection::Forward => RowSlide {
                left: slide.area.left(),
                row,
                source_start: 0,
                source_end: usize::from(slide.exposed),
                destination_start: slide.area.width - slide.exposed,
            },
            PageDirection::Backward => RowSlide {
                left: slide.area.left(),
                row,
                source_start: usize::from(slide.area.width - slide.exposed),
                source_end: usize::from(slide.area.width),
                destination_start: 0,
            },
        };
        copy_graphemes(buffer, &source, movement);
    }
}

fn has_graphics(buffer: &Buffer, area: Rect) -> bool {
    (area.top()..area.bottom()).any(|row| {
        (area.left()..area.right()).any(|column| {
            buffer
                .cell((column, row))
                .is_some_and(|cell| cell.diff_option == CellDiffOption::Skip)
        })
    })
}

fn clear_row(buffer: &mut Buffer, slide: &PaneSlide, row: u16) {
    let mut blank = Cell::default();
    blank.set_style(slide.style);
    for column in slide.area.left()..slide.area.right() {
        if let Some(cell) = buffer.cell_mut((column, row)) {
            *cell = blank.clone();
        }
    }
}

fn copy_graphemes(buffer: &mut Buffer, source: &[Cell], movement: RowSlide) {
    let mut column = 0;
    while column < source.len() {
        let width = cell_width(source[column].symbol()).max(1);
        if column >= movement.source_start && column.saturating_add(width) <= movement.source_end {
            for offset in 0..width {
                let destination = usize::from(movement.left)
                    + usize::from(movement.destination_start)
                    + column
                    + offset
                    - movement.source_start;
                if let (Some(source), Some(target)) = (
                    source.get(column + offset),
                    buffer.cell_mut((destination as u16, movement.row)),
                ) {
                    *target = source.clone();
                }
            }
        }
        column = column.saturating_add(width);
    }
}
