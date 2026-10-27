use crate::document::layout::{SourceCell, TextLayout};
use std::ops::Deref;

#[derive(Clone, Copy)]
pub struct SelectionCell<'a> {
    pub source: &'a SourceCell,
    pub row: usize,
}

impl Deref for SelectionCell<'_> {
    type Target = SourceCell;
    fn deref(&self) -> &Self::Target {
        self.source
    }
}

pub trait SelectionSurface {
    fn cell(&self, index: usize) -> Option<SelectionCell<'_>>;
    fn cell_count(&self) -> usize;
    fn row_count(&self) -> usize;

    fn cells(&self) -> Box<dyn Iterator<Item = SelectionCell<'_>> + '_> {
        Box::new((0..self.cell_count()).filter_map(move |index| self.cell(index)))
    }
}

impl SelectionSurface for TextLayout {
    fn cell(&self, index: usize) -> Option<SelectionCell<'_>> {
        self.source_cells.get(index).map(|cell| SelectionCell {
            source: cell,
            row: cell.row,
        })
    }
    fn cell_count(&self) -> usize {
        self.source_cells.len()
    }
    fn row_count(&self) -> usize {
        self.text_lines.len()
    }
}
