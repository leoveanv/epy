use super::{SelectionCell, SelectionSurface};
use crate::annotations::TextAnchor;
use crate::document::links::character_slice;
use crate::document::text::{ParsedDocument, python_whitespace};
use crate::{Error, Result};
use std::ops::RangeInclusive;

#[derive(Clone, Copy, Debug)]
pub enum SelectionMotion {
    Left,
    Right,
    Up,
    Down,
    WordForward,
    WordBackward,
}

#[derive(Clone, Copy, Debug)]
pub struct SelectionWindow {
    pub top: usize,
    pub height: usize,
    pub scrolloff: usize,
}

#[derive(Clone, Debug)]
struct CellRow {
    row: usize,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug)]
pub struct Selection {
    content_id: String,
    cursor: usize,
    mark: Option<usize>,
    first: usize,
    last: usize,
    rows: Vec<CellRow>,
    word_starts: Vec<usize>,
}

impl Selection {
    pub fn from_viewport(layout: &dyn SelectionSurface, top: usize, height: usize) -> Result<Self> {
        let midpoint = top + height / 2;
        let cursor = layout
            .cells()
            .enumerate()
            .filter(|(_, cell)| cell.row >= top && cell.row < top.saturating_add(height))
            .min_by_key(|(_, cell)| cell.row.abs_diff(midpoint))
            .map(|(index, _)| index)
            .ok_or_else(|| {
                Error::Compatibility("This window has no visible selectable text.".into())
            })?;
        Self::at_cursor(layout, cursor)
    }

    fn at_cursor(layout: &dyn SelectionSurface, cursor: usize) -> Result<Self> {
        let cell = layout
            .cell(cursor)
            .ok_or_else(|| Error::Compatibility("This document has no selectable text.".into()))?;
        let content_id = cell.content_id.clone();
        let first = layout
            .cells()
            .position(|cell| cell.content_id == content_id)
            .unwrap_or(cursor);
        let last = (0..layout.cell_count())
            .rev()
            .find(|index| {
                layout
                    .cell(*index)
                    .is_some_and(|cell| cell.content_id == content_id)
            })
            .unwrap_or(cursor);
        let mut rows = Vec::<CellRow>::new();
        let mut word_starts = Vec::new();
        for index in first..=last {
            let cell = layout.cell(index).ok_or_else(|| {
                Error::Compatibility("The selection surface has an invalid cell index.".into())
            })?;
            if let Some(row) = rows.last_mut().filter(|row| row.row == cell.row) {
                row.end = index + 1;
            } else {
                rows.push(CellRow {
                    row: cell.row,
                    start: index,
                    end: index + 1,
                });
            }
            if starts_word(
                index.checked_sub(1).and_then(|index| layout.cell(index)),
                cell,
            ) {
                word_starts.push(index);
            }
        }
        Ok(Self {
            content_id,
            cursor,
            mark: None,
            first,
            last,
            rows,
            word_starts,
        })
    }

    pub fn cell<'a>(&self, layout: &'a dyn SelectionSurface) -> Option<SelectionCell<'a>> {
        layout
            .cell(self.cursor)
            .filter(|cell| cell.content_id == self.content_id)
    }
    pub fn start_range(&mut self) {
        self.mark = Some(self.cursor);
    }
    pub fn clear_range(&mut self) {
        self.mark = None;
    }
    pub fn has_range(&self) -> bool {
        self.mark.is_some()
    }
    pub fn selected_indices(&self) -> RangeInclusive<usize> {
        let mark = self.mark.unwrap_or(self.cursor);
        self.cursor.min(mark)..=self.cursor.max(mark)
    }

    pub fn move_cursor(&mut self, motion: SelectionMotion, layout: &dyn SelectionSurface) {
        match motion {
            SelectionMotion::Left => self.cursor = self.cursor.saturating_sub(1).max(self.first),
            SelectionMotion::Right => self.cursor = self.cursor.saturating_add(1).min(self.last),
            SelectionMotion::WordForward => {
                self.cursor = self
                    .word_starts
                    .get(
                        self.word_starts
                            .partition_point(|index| *index <= self.cursor),
                    )
                    .copied()
                    .unwrap_or(self.last);
            }
            SelectionMotion::WordBackward => {
                let index = self
                    .word_starts
                    .partition_point(|index| *index < self.cursor);
                self.cursor = index
                    .checked_sub(1)
                    .and_then(|index| self.word_starts.get(index))
                    .copied()
                    .unwrap_or(self.first);
            }
            SelectionMotion::Up | SelectionMotion::Down => {
                let Some(cell) = self.cell(layout) else {
                    return;
                };
                let row_index = self.rows.partition_point(|row| row.row < cell.row);
                let target = match motion {
                    SelectionMotion::Up => row_index.checked_sub(1),
                    _ => row_index.checked_add(1),
                };
                if let Some(row) = target.and_then(|index| self.rows.get(index)) {
                    self.cursor = nearest_column(row, cell.col, layout);
                }
            }
        }
    }

    pub fn move_rows(&mut self, distance: isize, layout: &dyn SelectionSurface) {
        let Some(cell) = self.cell(layout) else {
            return;
        };
        let target = cell.row.saturating_add_signed(distance);
        let next = self
            .rows
            .partition_point(|row| row.row < target)
            .min(self.rows.len() - 1);
        let index = if next > 0
            && self.rows[next - 1].row.abs_diff(target) < self.rows[next].row.abs_diff(target)
        {
            next - 1
        } else {
            next
        };
        self.cursor = nearest_column(&self.rows[index], cell.col, layout);
    }

    pub fn viewport(&self, layout: &dyn SelectionSurface, window: SelectionWindow) -> usize {
        let Some(cell) = self.cell(layout) else {
            return window.top;
        };
        let height = window.height.max(1);
        let margin = window.scrolloff.min(height / 2);
        let top = (window.top as isize)
            .min(cell.row as isize - margin as isize)
            .max(cell.row as isize - height as isize + margin as isize + 1);
        top.max(0)
            .min(layout.row_count().saturating_sub(height) as isize) as usize
    }

    pub fn anchor(
        &self,
        document: &ParsedDocument,
        layout: &dyn SelectionSurface,
    ) -> Result<TextAnchor> {
        let range = self.selected_indices();
        let start = layout
            .cell(*range.start())
            .ok_or_else(|| {
                Error::Compatibility("The selected passage is no longer visible.".into())
            })?
            .start;
        let end = layout
            .cell(*range.end())
            .ok_or_else(|| {
                Error::Compatibility("The selected passage is no longer visible.".into())
            })?
            .end;
        if document.source.content_id != self.content_id {
            return Err(Error::Compatibility(
                "Selections cannot cross document boundaries.".into(),
            ));
        }
        TextAnchor::new(
            &self.content_id,
            start,
            character_slice(&document.source.text, start, end),
        )
        .map(|anchor| {
            anchor.with_context(
                character_slice(&document.source.text, start.saturating_sub(32), start),
                character_slice(&document.source.text, end, end.saturating_add(32)),
            )
        })
    }

    pub fn remap(
        &mut self,
        previous: &dyn SelectionSurface,
        current: &dyn SelectionSurface,
    ) -> Result<()> {
        let cursor = self
            .cell(previous)
            .ok_or_else(|| {
                Error::Compatibility("The selection cursor no longer has a source location.".into())
            })?
            .start;
        let mark = self
            .mark
            .and_then(|index| previous.cell(index))
            .map(|cell| cell.start);
        let index = current
            .cells()
            .position(|cell| cell.content_id == self.content_id && cell.end > cursor)
            .ok_or_else(|| {
                Error::Compatibility("The selection could not be restored after reflow.".into())
            })?;
        let mut remapped = Self::at_cursor(current, index)?;
        remapped.mark = mark.and_then(|offset| {
            current
                .cells()
                .position(|cell| cell.content_id == self.content_id && cell.end > offset)
        });
        *self = remapped;
        Ok(())
    }
}

fn starts_word(previous: Option<SelectionCell<'_>>, current: SelectionCell<'_>) -> bool {
    if current.text.chars().all(python_whitespace) {
        return false;
    }
    let Some(previous) = previous.filter(|previous| {
        previous.content_id == current.content_id && previous.end == current.start
    }) else {
        return true;
    };
    let keyword = |text: &str| {
        text.chars()
            .next()
            .is_some_and(|character| character.is_alphanumeric() || character == '_')
    };
    previous.text.chars().all(python_whitespace)
        || keyword(&previous.text) != keyword(&current.text)
}

fn nearest_column(row: &CellRow, col: usize, layout: &dyn SelectionSurface) -> usize {
    (row.start..row.end)
        .min_by_key(|index| {
            layout
                .cell(*index)
                .map_or(usize::MAX, |cell| cell.col.abs_diff(col))
        })
        .unwrap_or(row.start)
}
