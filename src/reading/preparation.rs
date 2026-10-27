use super::LoadRequest;
use super::{DocumentRow, ReadingWindow};
use crate::document::RenderedDocument;
use crate::{Error, Result};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct WindowRequest {
    pub load: LoadRequest,
    pub shift_rows: isize,
    pub viewport_rows: usize,
}

pub struct PreparedWindow {
    pub window: ReadingWindow,
    pub cursor: DocumentRow,
    pub document: Arc<RenderedDocument>,
}

impl PreparedWindow {
    pub fn new(window: ReadingWindow, cursor: DocumentRow) -> Result<Self> {
        window.row_number(cursor)?;
        let document = window
            .document(cursor.document_index)
            .cloned()
            .ok_or_else(|| {
                Error::Compatibility("The viewport document is outside its prepared window.".into())
            })?;
        Ok(Self {
            window,
            cursor,
            document,
        })
    }
}
