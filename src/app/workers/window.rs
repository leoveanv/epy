use super::book::BookWorker;
use super::{LoadJob, LoadOperation, Mailbox, WorkerEvent, send_event};
use crate::document::RenderedDocument;
use crate::reading::LoadRequest;
use crate::reading::{
    DocumentRow, PreparedWindow, ReadingWindow, WindowExtent, WindowMovement, WindowRequest,
};
use crate::{Error, Result};
use crossbeam_channel::Sender;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[derive(Clone, Copy)]
enum RowSpace {
    Display,
    Original,
}

impl RowSpace {
    fn len(self, lines: &crate::document::layout::TextLines) -> usize {
        match self {
            Self::Display => lines.len(),
            Self::Original => lines.original_len(),
        }
    }

    fn row(self, lines: &crate::document::layout::TextLines, display_row: usize) -> usize {
        match self {
            Self::Display => display_row,
            Self::Original => lines.image_layout().original_row(display_row),
        }
    }

    fn display_row(self, lines: &crate::document::layout::TextLines, row: usize) -> usize {
        match self {
            Self::Display => row,
            Self::Original => lines.image_layout().display_row(row),
        }
    }
}

struct WindowTask<'a> {
    worker: &'a mut BookWorker,
    width: usize,
    active: &'a dyn Fn() -> bool,
}

impl WindowTask<'_> {
    fn load(&mut self, index: usize) -> Result<Arc<RenderedDocument>> {
        if !(self.active)() {
            return Err(Error::Canceled);
        }
        let document = self.worker.load(&LoadRequest {
            index,
            width: self.width,
            position: None,
            fragment: None,
            at_end: false,
        })?;
        if !(self.active)() {
            return Err(Error::Canceled);
        }
        Ok(document)
    }

    fn origin(&mut self, request: &LoadRequest) -> Result<DocumentRow> {
        if let Some(position) = &request.position {
            return self.saved_origin(position);
        }
        let index = request.index;
        let document = self.load(index)?;
        let row = if request.at_end {
            document.layout.text_lines.len().saturating_sub(1)
        } else if let Some(fragment) = &request.fragment {
            let decoded = percent_encoding::percent_decode_str(fragment).decode_utf8_lossy();
            document
                .parsed
                .semantics
                .anchors
                .iter()
                .find(|anchor| anchor.fragment == decoded)
                .map_or(0, |anchor| document.layout.row_for_offset(anchor.offset))
        } else {
            0
        };
        Ok(DocumentRow {
            document_index: index,
            row,
        })
    }

    fn saved_origin(&mut self, position: &crate::reading::ViewPosition) -> Result<DocumentRow> {
        if let Some(image) = &position.image
            && let Some(index) = self
                .worker
                .info
                .contents
                .iter()
                .position(|id| *id == image.content_id)
        {
            let document = self.load(index)?;
            if let Some(row) = image.row_in(&document.layout) {
                return Ok(DocumentRow {
                    document_index: index,
                    row,
                });
            }
        }
        let text = &position.text;
        let index = self
            .worker
            .info
            .contents
            .iter()
            .position(|id| *id == text.content_id)
            .ok_or_else(|| {
                Error::Compatibility("The saved document is not present in this ebook.".into())
            })?;
        let document = self.load(index)?;
        if text.offset > document.parsed.source.text.chars().count() {
            return Err(Error::Compatibility(
                "The saved position is outside its document.".into(),
            ));
        }
        let origin = DocumentRow {
            document_index: index,
            row: document.layout.row_for_offset(text.offset),
        };
        let screen_row = isize::try_from(text.screen_row).map_err(|_| {
            Error::Compatibility("The saved screen row exceeds the supported range.".into())
        })?;
        self.shift(origin, -screen_row, RowSpace::Original)
    }

    fn shift(
        &mut self,
        mut cursor: DocumentRow,
        rows: isize,
        space: RowSpace,
    ) -> Result<DocumentRow> {
        let mut remaining = rows.unsigned_abs();
        let mut document = self.load(cursor.document_index)?;
        cursor.row = space.row(&document.layout.text_lines, cursor.row);
        if rows < 0 {
            while remaining > cursor.row && cursor.document_index > 0 {
                remaining -= cursor.row;
                cursor.document_index -= 1;
                document = self.load(cursor.document_index)?;
                cursor.row = space.len(&document.layout.text_lines);
            }
            cursor.row = cursor.row.saturating_sub(remaining);
        } else {
            while remaining
                >= space
                    .len(&document.layout.text_lines)
                    .saturating_sub(cursor.row)
                && cursor.document_index + 1 < self.worker.info.contents.len()
            {
                remaining -= space
                    .len(&document.layout.text_lines)
                    .saturating_sub(cursor.row);
                cursor.document_index += 1;
                cursor.row = 0;
                document = self.load(cursor.document_index)?;
            }
            cursor.row = cursor.row.saturating_add(remaining);
        }
        cursor.row = space.display_row(
            &document.layout.text_lines,
            cursor
                .row
                .min(space.len(&document.layout.text_lines).saturating_sub(1)),
        );
        Ok(cursor)
    }

    fn prepare(&mut self, request: &WindowRequest) -> Result<PreparedWindow> {
        let origin = self.origin(&request.load)?;
        let cursor = self.shift(origin, request.shift_rows, RowSpace::Display)?;
        let current = self.load(cursor.document_index)?;
        let mut following = current.layout.text_lines.len().saturating_sub(cursor.row);
        let mut preceding = cursor.row;
        let mut first = cursor.document_index;
        let mut last = first;
        let mut documents = VecDeque::from([current]);
        let budget = request.viewport_rows.max(1).saturating_mul(2);
        while preceding < budget && first > 0 {
            first -= 1;
            let document = self.load(first)?;
            preceding = preceding.saturating_add(document.layout.text_lines.len());
            documents.push_front(document);
        }
        while following < budget && last + 1 < self.worker.info.contents.len() {
            last += 1;
            let document = self.load(last)?;
            following = following.saturating_add(document.layout.text_lines.len());
            documents.push_back(document);
        }
        let window = ReadingWindow::new(
            documents.into_iter().collect(),
            WindowExtent {
                starts_book: first == 0,
                ends_book: last + 1 == self.worker.info.contents.len(),
            },
        )?;
        // #NOTE: Explicit jumps and saved positions keep their requested top row; only motion and end commands fill the last page.
        if request.shift_rows == 0 && !request.load.at_end {
            return PreparedWindow::new(window, cursor);
        }
        match window.fit_viewport(cursor, request.viewport_rows)? {
            WindowMovement::Ready(cursor) => PreparedWindow::new(window, cursor),
            _ => Err(Error::Compatibility(
                "The prepared window does not cover the requested viewport.".into(),
            )),
        }
    }
}

fn follow_hyperlink(
    worker: &mut BookWorker,
    request: &crate::reading::HyperlinkRequest,
    generation: u64,
    active: &dyn Fn() -> bool,
) -> Result<WorkerEvent> {
    match worker.resolve_hyperlink(request, active)? {
        super::hyperlinks::HyperlinkDestination::External(url) => {
            Ok(WorkerEvent::ExternalLink { generation, url })
        }
        super::hyperlinks::HyperlinkDestination::Internal(load) => {
            if let Some(viewport_rows) = request.continuous_rows {
                let view = WindowTask {
                    worker,
                    width: request.width,
                    active,
                }
                .prepare(&WindowRequest {
                    load,
                    shift_rows: 0,
                    viewport_rows,
                })?;
                Ok(WorkerEvent::WindowLoaded { generation, view })
            } else {
                let rendered = worker.load(&load)?;
                Ok(WorkerEvent::Loaded {
                    generation,
                    rendered,
                    request: load,
                })
            }
        }
    }
}

pub(super) fn run_load(
    worker: &mut BookWorker,
    job: LoadJob,
    mailbox: &Mailbox,
    sender: &Sender<WorkerEvent>,
) {
    let active = || {
        mailbox.generation.load(Ordering::SeqCst) == job.generation
            && !mailbox.stopped.load(Ordering::SeqCst)
    };
    let event = match job.operation {
        LoadOperation::Hyperlink(request) => {
            follow_hyperlink(worker, &request, job.generation, &active)
        }
        LoadOperation::Document(request) => {
            worker.load(&request).map(|rendered| WorkerEvent::Loaded {
                generation: job.generation,
                rendered,
                request,
            })
        }
        LoadOperation::Window(request) => WindowTask {
            worker,
            width: request.load.width.max(1),
            active: &active,
        }
        .prepare(&request)
        .map(|view| WorkerEvent::WindowLoaded {
            generation: job.generation,
            view,
        }),
    };
    if active() {
        send_event(
            sender,
            mailbox,
            event.unwrap_or_else(|error| WorkerEvent::LoadFailed {
                generation: job.generation,
                message: error.to_string(),
            }),
        );
    }
}
