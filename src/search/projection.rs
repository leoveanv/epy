use super::SearchHit;
use super::native::ByteRange;
use crate::document::layout::TextLayout;
use crate::document::text::cell_width;
use std::iter::Peekable;
use unicode_segmentation::{GraphemeIndices, UnicodeSegmentation};

struct DisplayCluster {
    byte_end: usize,
    column: usize,
    width: usize,
}

struct LineCursor<'a> {
    remaining: Peekable<GraphemeIndices<'a>>,
    current: Option<DisplayCluster>,
}

impl<'a> LineCursor<'a> {
    fn new(line: &'a str) -> Self {
        Self {
            remaining: line.grapheme_indices(true).peekable(),
            current: None,
        }
    }

    fn seek(&mut self, byte: usize) -> usize {
        while self
            .current
            .as_ref()
            .is_none_or(|cluster| cluster.byte_end <= byte)
        {
            let Some((start, text)) = self.remaining.next() else {
                break;
            };
            let column = self
                .current
                .as_ref()
                .map_or(0, |cluster| cluster.column + cluster.width);
            self.current = Some(DisplayCluster {
                byte_end: start + text.len(),
                column,
                width: cell_width(text),
            });
        }
        self.current.as_ref().map_or(0, |cluster| {
            if byte >= cluster.byte_end {
                cluster.column + cluster.width
            } else {
                cluster.column
            }
        })
    }

    fn end_column(&self) -> usize {
        self.current
            .as_ref()
            .map_or(0, |cluster| cluster.column + cluster.width)
    }
}

pub(super) fn project_hits(
    layout: &TextLayout,
    row: usize,
    line: &str,
    ranges: &[ByteRange],
) -> Vec<SearchHit> {
    let cells = layout.cells_on_row(row);
    let mut cursor = LineCursor::new(line);
    ranges
        .iter()
        .map(|range| {
            let column_start = cursor.seek(range.start);
            let column_end = if range.end > range.start {
                cursor.seek(range.end - 1);
                cursor.end_column()
            } else {
                column_start
            };
            let index = cells.partition_point(|cell| cell.col + cell.width <= column_start);
            let (offset, screen_row) = if let Some(cell) = cells.get(index) {
                (cell.start, 0)
            } else if let Some(cell) = cells.last() {
                (cell.end, 0)
            } else {
                layout
                    .first_cell_from_row(row)
                    .or_else(|| layout.source_cells.last())
                    .map_or((0, 0), |cell| (cell.start, cell.row.saturating_sub(row)))
            };
            SearchHit {
                row,
                column_start,
                column_end,
                offset,
                screen_row,
            }
        })
        .collect()
}
