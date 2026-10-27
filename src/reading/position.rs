use super::ReadingState;
use crate::document::layout::TextLayout;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImagePosition {
    pub content_id: String,
    pub path: String,
    pub ordinal: usize,
    pub row_offset: usize,
}

impl ImagePosition {
    pub fn capture(layout: &TextLayout, row: usize) -> Option<Self> {
        let images = layout.text_lines.image_layout();
        let slot = images.slot_at(row)?;
        let start = row - slot.row_offset;
        Some(Self {
            content_id: layout.content_id.clone(),
            path: layout.image_maps.get(&start)?.clone(),
            ordinal: slot.ordinal,
            row_offset: slot.row_offset,
        })
    }

    pub fn row_in(&self, layout: &TextLayout) -> Option<usize> {
        if self.content_id != layout.content_id {
            return None;
        }
        let images = layout.text_lines.image_layout();
        let start = images.slot_row(self.ordinal, 0)?;
        if layout.image_maps.get(&start)? != &self.path {
            return None;
        }
        images.slot_row(self.ordinal, self.row_offset)
    }
}

// #NOTE: Image identity governs image jumps; fallback text can change when a prepared window gains another chapter.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewPosition {
    pub text: ReadingState,
    pub image: Option<ImagePosition>,
}

impl From<ReadingState> for ViewPosition {
    fn from(text: ReadingState) -> Self {
        Self { text, image: None }
    }
}

impl ViewPosition {
    pub fn capture(layout: &TextLayout, row: usize, width: usize) -> Self {
        let images = layout.text_lines.image_layout();
        let cell = layout
            .first_cell_from_row(row)
            .or_else(|| layout.source_cells.last());
        Self {
            text: ReadingState {
                content_id: layout.content_id.clone(),
                offset: cell.map_or(0, |cell| cell.start),
                screen_row: cell.map_or(0, |cell| {
                    images
                        .original_row(cell.row)
                        .saturating_sub(images.original_row(row))
                }),
                textwidth: width,
            },
            image: ImagePosition::capture(layout, row),
        }
    }

    pub fn row_in(&self, layout: &TextLayout) -> usize {
        if let Some(row) = self.image.as_ref().and_then(|image| image.row_in(layout)) {
            return row;
        }
        let images = layout.text_lines.image_layout();
        images.display_row(
            images
                .original_row(layout.row_for_offset(self.text.offset))
                .saturating_sub(self.text.screen_row),
        )
    }

    pub fn content_id(&self) -> &str {
        self.image
            .as_ref()
            .map_or(&self.text.content_id, |image| &image.content_id)
    }
}
