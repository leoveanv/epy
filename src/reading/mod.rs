mod hyperlinks;
mod models;
mod position;
mod preparation;
mod selection;
pub use hyperlinks::{
    HintPlacement, HyperlinkRequest, LinkHint, external_url, link_hints, place_hints,
};
pub use models::{
    Bookmark, BookmarkRequest, LoadRequest, PageDirection, ReadingState, SaveRequest,
};
pub use position::{ImagePosition, ViewPosition};
pub use preparation::{PreparedWindow, WindowRequest};

use std::sync::Arc;

use crate::document::RenderedDocument;
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentRow {
    pub document_index: usize,
    pub row: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct WindowExtent {
    pub starts_book: bool,
    pub ends_book: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowMovement {
    Ready(DocumentRow),
    NeedsEarlier,
    NeedsLater,
}

pub struct VisibleRow<'a> {
    pub document: &'a RenderedDocument,
    pub row: usize,
}

struct DocumentBand {
    document: Arc<RenderedDocument>,
    first_row: usize,
    first_original_row: usize,
    first_cell: usize,
}

pub struct ReadingWindow {
    bands: Vec<DocumentBand>,
    extent: WindowExtent,
    rows: usize,
}

impl ReadingWindow {
    pub fn new(documents: Vec<Arc<RenderedDocument>>, extent: WindowExtent) -> Result<Self> {
        if documents.is_empty()
            || documents.windows(2).any(|pair| {
                pair[0].index.checked_add(1) != Some(pair[1].index)
                    || pair[0].width != pair[1].width
            })
        {
            return Err(Error::Compatibility(
                "A reading window requires consecutive documents at the same width.".into(),
            ));
        }
        Ok(Self::assemble(documents, extent))
    }

    pub fn single(document: Arc<RenderedDocument>, extent: WindowExtent) -> Self {
        Self::assemble(vec![document], extent)
    }

    fn assemble(documents: Vec<Arc<RenderedDocument>>, extent: WindowExtent) -> Self {
        let mut rows = 0usize;
        let mut original_rows = 0usize;
        let mut cells = 0usize;
        let bands = documents
            .into_iter()
            .map(|document| {
                let first_row = rows;
                let first_original_row = original_rows;
                let first_cell = cells;
                rows = rows.saturating_add(document.layout.text_lines.len());
                original_rows =
                    original_rows.saturating_add(document.layout.text_lines.original_len());
                cells = cells.saturating_add(document.layout.source_cells.len());
                DocumentBand {
                    document,
                    first_row,
                    first_original_row,
                    first_cell,
                }
            })
            .collect();
        Self {
            bands,
            extent,
            rows,
        }
    }

    pub fn document(&self, index: usize) -> Option<&Arc<RenderedDocument>> {
        self.bands
            .iter()
            .find(|band| band.document.index == index)
            .map(|band| &band.document)
    }

    pub fn source_document(&self, content_id: &str) -> Option<&Arc<RenderedDocument>> {
        self.bands
            .iter()
            .find(|band| band.document.parsed.source.content_id == content_id)
            .map(|band| &band.document)
    }

    pub(crate) fn row_number(&self, origin: DocumentRow) -> Result<usize> {
        let band = self
            .bands
            .iter()
            .find(|band| band.document.index == origin.document_index)
            .ok_or_else(|| {
                Error::Compatibility(
                    "The requested document is outside the prepared window.".into(),
                )
            })?;
        if origin.row >= band.document.layout.text_lines.len().max(1) {
            return Err(Error::Compatibility(
                "The requested row is outside its document.".into(),
            ));
        }
        Ok(band.first_row.saturating_add(origin.row))
    }

    pub(crate) fn cursor_at(&self, row: usize) -> DocumentRow {
        self.bands
            .iter()
            .find(|band| {
                row >= band.first_row
                    && row - band.first_row < band.document.layout.text_lines.len()
            })
            .map_or_else(
                || DocumentRow {
                    document_index: self.bands[0].document.index,
                    row: 0,
                },
                |band| DocumentRow {
                    document_index: band.document.index,
                    row: row - band.first_row,
                },
            )
    }

    pub fn row(&self, origin: DocumentRow, offset: usize) -> Option<VisibleRow<'_>> {
        let row = self.row_number(origin).ok()?.checked_add(offset)?;
        self.bands.iter().find_map(|band| {
            let local = row.checked_sub(band.first_row)?;
            (local < band.document.layout.text_lines.len()).then_some(VisibleRow {
                document: &band.document,
                row: local,
            })
        })
    }

    pub fn move_rows(
        &self,
        origin: DocumentRow,
        delta: isize,
        viewport_rows: usize,
    ) -> Result<WindowMovement> {
        let row = self.row_number(origin)?;
        if delta < 0 && delta.unsigned_abs() > row && !self.extent.starts_book {
            return Ok(WindowMovement::NeedsEarlier);
        }
        let target = row.saturating_add_signed(delta);
        if !self.extent.ends_book && target.saturating_add(viewport_rows.max(1)) > self.rows {
            return Ok(WindowMovement::NeedsLater);
        }
        let last = self.rows.saturating_sub(viewport_rows.max(1)).max(row);
        Ok(WindowMovement::Ready(self.cursor_at(target.min(last))))
    }

    pub fn fit_viewport(
        &self,
        origin: DocumentRow,
        viewport_rows: usize,
    ) -> Result<WindowMovement> {
        let row = self.row_number(origin)?;
        if !self.extent.ends_book && row.saturating_add(viewport_rows.max(1)) > self.rows {
            return Ok(WindowMovement::NeedsLater);
        }
        if self.extent.ends_book && !self.extent.starts_book && self.rows < viewport_rows.max(1) {
            return Ok(WindowMovement::NeedsEarlier);
        }
        Ok(WindowMovement::Ready(self.cursor_at(
            row.min(self.rows.saturating_sub(viewport_rows.max(1))),
        )))
    }

    pub fn capture_view(&self, origin: DocumentRow) -> Result<ViewPosition> {
        let text = self.capture_position(origin)?;
        let image = self
            .document(origin.document_index)
            .and_then(|document| ImagePosition::capture(&document.layout, origin.row));
        Ok(ViewPosition { text, image })
    }

    pub fn restore_view(&self, position: &ViewPosition) -> Result<WindowMovement> {
        if let Some(image) = &position.image
            && let Some(document) = self.source_document(&image.content_id)
            && let Some(row) = image.row_in(&document.layout)
        {
            return Ok(WindowMovement::Ready(DocumentRow {
                document_index: document.index,
                row,
            }));
        }
        self.restore_position(&position.text)
    }

    fn original_row_number(&self, origin: DocumentRow) -> Result<usize> {
        self.bands
            .iter()
            .find(|band| band.document.index == origin.document_index)
            .map(|band| {
                band.first_original_row
                    + band
                        .document
                        .layout
                        .text_lines
                        .image_layout()
                        .original_row(origin.row)
            })
            .ok_or_else(|| {
                Error::Compatibility("The position is outside the prepared window.".into())
            })
    }

    fn display_row_for_original(&self, row: usize) -> usize {
        self.bands
            .iter()
            .rev()
            .find(|band| band.first_original_row <= row)
            .map_or(0, |band| {
                band.first_row
                    + band
                        .document
                        .layout
                        .text_lines
                        .image_layout()
                        .display_row(row - band.first_original_row)
            })
    }

    pub fn capture_position(&self, origin: DocumentRow) -> Result<ReadingState> {
        let row = self.row_number(origin)?;
        let original_row = self.original_row_number(origin)?;
        for band in &self.bands {
            if let Some(cell) = band
                .document
                .layout
                .first_cell_from_row(row.saturating_sub(band.first_row))
            {
                return Ok(ReadingState {
                    content_id: cell.content_id.clone(),
                    offset: cell.start,
                    screen_row: band
                        .first_original_row
                        .saturating_add(
                            band.document
                                .layout
                                .text_lines
                                .image_layout()
                                .original_row(cell.row),
                        )
                        .saturating_sub(original_row),
                    textwidth: band.document.width,
                });
            }
        }
        let band = self
            .bands
            .iter()
            .find(|band| band.document.index == origin.document_index)
            .ok_or_else(|| {
                Error::Compatibility("The position is outside the prepared window.".into())
            })?;
        Ok(ReadingState {
            content_id: band.document.parsed.source.content_id.clone(),
            offset: band
                .document
                .layout
                .source_cells
                .last()
                .map_or(0, |cell| cell.start),
            screen_row: 0,
            textwidth: band.document.width,
        })
    }

    pub fn restore_position(&self, position: &ReadingState) -> Result<WindowMovement> {
        let band = self
            .bands
            .iter()
            .find(|band| band.document.parsed.source.content_id == position.content_id)
            .ok_or_else(|| {
                Error::Compatibility("The saved document is outside the prepared window.".into())
            })?;
        if position.offset > band.document.parsed.source.text.chars().count() {
            return Err(Error::Compatibility(
                "The saved offset is outside its document.".into(),
            ));
        }
        let anchor = band.first_original_row
            + band
                .document
                .layout
                .text_lines
                .image_layout()
                .original_row(band.document.layout.row_for_offset(position.offset));
        if position.screen_row > anchor && !self.extent.starts_book {
            return Ok(WindowMovement::NeedsEarlier);
        }
        let row = self.display_row_for_original(anchor.saturating_sub(position.screen_row));
        Ok(WindowMovement::Ready(self.cursor_at(row)))
    }
}
