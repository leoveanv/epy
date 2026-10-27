use super::ReadingWindow;
use crate::annotations::{SelectionCell, SelectionSurface};

impl SelectionSurface for ReadingWindow {
    fn cell(&self, index: usize) -> Option<SelectionCell<'_>> {
        self.bands.iter().find_map(|band| {
            let local = index.checked_sub(band.first_cell)?;
            band.document
                .layout
                .source_cells
                .get(local)
                .map(|cell| SelectionCell {
                    source: cell,
                    row: band.first_row + cell.row,
                })
        })
    }
    fn cells(&self) -> Box<dyn Iterator<Item = SelectionCell<'_>> + '_> {
        Box::new(self.bands.iter().flat_map(|band| {
            band.document
                .layout
                .source_cells
                .iter()
                .map(move |cell| SelectionCell {
                    source: cell,
                    row: band.first_row + cell.row,
                })
        }))
    }

    fn cell_count(&self) -> usize {
        self.bands
            .iter()
            .map(|band| band.document.layout.source_cells.len())
            .sum()
    }
    fn row_count(&self) -> usize {
        self.rows
    }
}
