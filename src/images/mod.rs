mod decoder;
mod svg;
pub use svg::{
    ImageWorkerOptions, SvgDecodeOptions, SvgLimits, SvgRequest, decode_svg, run_svg_helper,
};
mod encoding;
pub use encoding::{
    ImageEncoding, ImageProtocol, PreparedImage, PreparedInlineImage, prepare_image,
};
mod worker;
pub use decoder::{DecodedImage, ImageLimits, ImageSourceFormat, decode_image};
pub use worker::{ImageJob, ImagePayload, ImageResult, ImageWorker};

use crate::reading::{DocumentRow, ReadingWindow};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageTarget {
    pub content_id: String,
    pub path: String,
    pub row_offset: isize,
}

pub fn visible_images(
    window: &ReadingWindow,
    origin: DocumentRow,
    rows: usize,
) -> Vec<ImageTarget> {
    image_targets(window, origin, rows).collect()
}

pub fn image_targets(
    window: &ReadingWindow,
    origin: DocumentRow,
    rows: usize,
) -> impl Iterator<Item = ImageTarget> + '_ {
    (0..rows).filter_map(move |offset| {
        let visible = window.row(origin, offset)?;
        let slot = visible
            .document
            .layout
            .text_lines
            .image_layout()
            .slot_at(visible.row)?;
        if offset != 0 && slot.row_offset != 0 {
            return None;
        }
        let path = visible
            .document
            .layout
            .image_maps
            .get(&(visible.row - slot.row_offset))?;
        Some(ImageTarget {
            content_id: visible.document.parsed.source.content_id.clone(),
            path: path.clone(),
            row_offset: isize::try_from(offset)
                .unwrap_or(isize::MAX)
                .saturating_sub(isize::try_from(slot.row_offset).unwrap_or(isize::MAX)),
        })
    })
}
