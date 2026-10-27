use super::{ReaderEffect, ReaderMode, ReaderModel};
use crate::reading::LoadRequest;
use crate::reading::ViewPosition;
use crate::reading::{DocumentRow, PreparedWindow, WindowMovement, WindowRequest};

impl ReaderModel {
    pub fn selection_surface(&self) -> &dyn crate::annotations::SelectionSurface {
        if self.configuration.settings.between_chapters {
            &self.window
        } else {
            &self.rendered.layout
        }
    }

    pub(crate) fn selection_top(&self) -> usize {
        if self.configuration.settings.between_chapters {
            self.window
                .row_number(DocumentRow {
                    document_index: self.rendered.index,
                    row: self.row,
                })
                .unwrap_or(self.row)
        } else {
            self.row
        }
    }

    pub(crate) fn selection_index(
        &self,
        document_index: usize,
        row: usize,
        col: usize,
    ) -> Option<usize> {
        if self.configuration.settings.between_chapters {
            return self.window.selection_index(document_index, row, col);
        }
        if document_index != self.rendered.index {
            return None;
        }
        let start = self
            .rendered
            .layout
            .source_cells
            .partition_point(|cell| cell.row < row);
        self.rendered
            .layout
            .cells_on_row(row)
            .iter()
            .position(|cell| col >= cell.col && col < cell.col + cell.width)
            .map(|local| start + local)
    }

    pub(super) fn set_selection_top(&mut self, row: usize) {
        if self.configuration.settings.between_chapters {
            let cursor = self.window.cursor_at(row);
            if let Some(document) = self.window.document(cursor.document_index) {
                self.rendered = document.clone();
                self.row = cursor.row;
            }
        } else {
            self.row = row;
        }
    }

    pub fn initial_reading(&mut self, position: ViewPosition) -> Vec<ReaderEffect> {
        let width = self.width.min(self.available_width());
        if !self.configuration.settings.between_chapters
            && width == self.width
            && position.image.is_none()
        {
            return Vec::new();
        }
        let index = self
            .info
            .contents
            .iter()
            .position(|id| id == position.content_id())
            .unwrap_or(self.rendered.index);
        self.request_load(LoadRequest {
            index,
            width,
            position: Some(position),
            fragment: None,
            at_end: false,
        })
    }

    pub(super) fn request_window(&mut self, request: WindowRequest) -> Vec<ReaderEffect> {
        self.loading = true;
        self.loading_request = Some(request.load.clone());
        self.pending_shift = request.shift_rows;
        self.width = request.load.width;
        self.status = "Preparing continuous viewport…".into();
        vec![ReaderEffect::Window(request)]
    }

    pub fn accept_window(&mut self, view: PreparedWindow) -> Vec<ReaderEffect> {
        let remap_error = if let ReaderMode::Annotations(mode) = &mut self.mode {
            mode.remap(&self.window, &view.window)
                .err()
                .map(|error| error.to_string())
        } else {
            None
        };
        self.width = view.document.width;
        self.row = view.cursor.row;
        self.rendered = view.document;
        self.window = view.window;
        self.loading = false;
        self.loading_request = None;
        self.pending_shift = 0;
        self.pending_hyperlink = None;
        self.status = remap_error.unwrap_or_default();
        self.loaded_effects()
    }

    pub(super) fn ensure_continuous_view(&mut self) -> Vec<ReaderEffect> {
        if self.configuration.settings.between_chapters && !self.loading {
            return self.continuous_rows(0);
        }
        Vec::new()
    }

    pub(super) fn continuous_rows(&mut self, delta: isize) -> Vec<ReaderEffect> {
        let origin = DocumentRow {
            document_index: self.rendered.index,
            row: self.row,
        };
        match self
            .window
            .move_rows(origin, delta, self.viewport.rows * self.spread)
        {
            Ok(WindowMovement::Ready(cursor)) => {
                if let Some(document) = self.window.document(cursor.document_index) {
                    self.rendered = document.clone();
                    self.row = cursor.row;
                }
                Vec::new()
            }
            Ok(_) => self.request_window(WindowRequest {
                load: LoadRequest {
                    index: self.rendered.index,
                    width: self.width,
                    position: Some(self.view_position()),
                    at_end: false,
                    fragment: None,
                },
                shift_rows: delta,
                viewport_rows: self.viewport.rows * self.spread,
            }),
            Err(error) => {
                self.status = error.to_string();
                Vec::new()
            }
        }
    }
}
