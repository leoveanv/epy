use super::modes::{
    ModalKeyOutcome, ModeContext, ModeInput, PendingModeContext,
    annotations::{AnnotationMode, AnnotationState},
    dictionary::{DictionaryMode, DictionaryOutcome, DictionaryState},
    hyperlinks::LinkSelection,
    images::{ImageMode, ImageState},
    library::{LibraryMode, LibraryPhase, LibraryState},
    markdown_export::{LinkMode, MarkdownState},
    reassociation::{ReassociationMode, ReassociationState},
    search::{SearchMode, SearchState},
};
use super::{
    BookInfo, ModalInput, ReaderEffect, ReaderSession, input, mouse, navigation, page_turn,
};

use std::collections::VecDeque;
use std::sync::Arc;

use crate::configuration::{Command, Configuration, InputKey};
use crate::document::RenderedDocument;
use crate::reading::{Bookmark, BookmarkRequest, LoadRequest, ReadingState, SaveRequest};
use input::{DeferredInputs, PendingInputs};

#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub columns: usize,
    pub rows: usize,
}

impl Viewport {
    pub(crate) fn available_width(self, spread: usize) -> usize {
        if spread == 2 {
            self.columns.saturating_sub(7) / 2
        } else {
            self.columns.saturating_sub(4)
        }
        .max(1)
    }
}

#[derive(Clone, Debug)]
pub struct JumpTarget {
    pub index: usize,
    pub position: Option<ReadingState>,
    pub fragment: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DialogItem {
    pub label: String,
    pub target: Option<JumpTarget>,
}

#[derive(Clone, Debug)]
pub struct Dialog {
    pub title: String,
    pub items: Vec<DialogItem>,
    pub selected: usize,
}

#[derive(Clone, Debug)]
pub enum ReaderMode {
    Reading,
    Dialog(Dialog),
    BookmarkPrompt(String),
    Hyperlinks(LinkSelection),
    Images(Box<ImageMode>),
    Quickmark { saving: bool },
    Annotations(AnnotationMode),
    Link(Box<LinkMode>),
    Reassociation(Box<ReassociationMode>),
    Search(Box<SearchMode>),
    Library(Box<LibraryMode>),
    Dictionary(Box<DictionaryMode>),
}

pub(super) enum QueuedInput {
    Command {
        command: Command,
        count: usize,
        repetitions: usize,
    },
    Input(ModalInput),
    Mouse {
        action: mouse::MouseAction,
        context: mouse::MouseContext,
        repetitions: usize,
    },
}

pub struct ReaderView<'a> {
    pub info: &'a Arc<BookInfo>,
    pub rendered: &'a Arc<RenderedDocument>,
    pub configuration: &'a Configuration,
    pub row: usize,
    pub width: usize,
    pub spread: usize,
    pub viewport: Viewport,
    pub mode: &'a ReaderMode,
    pub status: &'a str,
    pub loading: bool,
    pub quitting: bool,
    pub color_scheme: usize,
    pub show_progress: bool,
    pub list_top: usize,
    pub input_warning: bool,
}

pub struct ReaderModel {
    pub(super) info: Arc<BookInfo>,
    pub(super) rendered: Arc<RenderedDocument>,
    pub(super) configuration: Configuration,
    pub(super) row: usize,
    pub(super) width: usize,
    pub(super) spread: usize,
    pub(super) viewport: Viewport,
    pub(super) mode: ReaderMode,
    pub(super) status: String,
    pub(super) loading: bool,
    pub(super) loading_request: Option<LoadRequest>,
    pub(super) pending_shift: isize,
    pub(super) pending_hyperlink: Option<crate::document::links::TextLink>,
    pub(super) quitting: bool,
    pub(super) color_scheme: usize,
    pub(super) show_progress: bool,
    pub(super) annotation_state: AnnotationState,
    pub(super) dictionary_state: DictionaryState,
    pub(super) image_state: ImageState,
    pub(super) library_state: LibraryState,
    pub(super) markdown_state: MarkdownState,
    pub(super) reassociation_state: ReassociationState,
    pub(super) search_state: SearchState,
    pub(super) deferred_modal_input: DeferredInputs,
    pub(super) prefix: String,
    pub(super) list_top: usize,
    pub(super) pending: PendingInputs,
    pub(super) pending_input_overflow: bool,
    pub(super) pending_modal_overflow: bool,
    pub(super) input_warning: bool,
    pub(super) navigation: navigation::NavigationState,
    pub(super) quit_after_load: bool,
    pub(super) page_turns: page_turn::PageTurns,
    pub(super) totals: Option<Vec<usize>>,
    pub(super) window: crate::reading::ReadingWindow,
}

impl ReaderModel {
    pub fn view(&self) -> ReaderView<'_> {
        ReaderView {
            info: &self.info,
            rendered: &self.rendered,
            configuration: &self.configuration,
            row: self.row,
            width: self.width,
            spread: self.spread,
            viewport: self.viewport,
            mode: &self.mode,
            status: &self.status,
            loading: self.loading,
            quitting: self.quitting,
            color_scheme: self.color_scheme,
            show_progress: self.show_progress,
            list_top: self.list_top,
            input_warning: self.input_warning,
        }
    }

    pub fn new(session: ReaderSession, viewport: Viewport) -> Self {
        let spread = if session.configuration.settings.start_with_double_spread {
            2
        } else {
            1
        };
        let show_progress = session.configuration.settings.show_progress_indicator;
        let row = crate::reading::ViewPosition::from(session.position.clone())
            .row_in(&session.rendered.layout);
        let width = session.rendered.width;
        let window = crate::reading::ReadingWindow::single(
            session.rendered.clone(),
            crate::reading::WindowExtent {
                starts_book: session.rendered.index == 0,
                ends_book: session.rendered.index + 1 == session.info.contents.len(),
            },
        );
        let mut model = Self {
            info: session.info,
            rendered: session.rendered,
            configuration: session.configuration,
            row,
            width,
            spread,
            viewport,
            mode: ReaderMode::Reading,
            status: String::new(),
            loading: false,
            loading_request: None,
            pending_shift: 0,
            pending_hyperlink: None,
            quitting: false,
            color_scheme: 0,
            show_progress,
            annotation_state: AnnotationState::default(),
            dictionary_state: DictionaryState::default(),
            image_state: ImageState::default(),
            library_state: LibraryState::default(),
            markdown_state: MarkdownState::default(),
            reassociation_state: ReassociationState::default(),
            search_state: SearchState::default(),
            deferred_modal_input: DeferredInputs::default(),
            prefix: String::new(),
            list_top: 0,
            pending: PendingInputs::default(),
            pending_input_overflow: false,
            pending_modal_overflow: false,
            input_warning: false,
            navigation: navigation::NavigationState::default(),
            quit_after_load: false,
            page_turns: page_turn::PageTurns::default(),
            totals: None,
            window,
        };
        model.row = model
            .row
            .min(model.rendered.layout.text_lines.len().saturating_sub(1));
        model
    }

    pub(super) fn replace_document(&mut self, document: Arc<RenderedDocument>) {
        let retained = self.window.document(document.index).is_some_and(|cached| {
            cached.width == document.width && cached.layout == document.layout
        });
        if !retained {
            self.window = crate::reading::ReadingWindow::single(
                document.clone(),
                crate::reading::WindowExtent {
                    starts_book: document.index == 0,
                    ends_book: document.index + 1 == self.info.contents.len(),
                },
            );
        }
        self.rendered = document;
    }

    pub(super) fn begin_reassociation(&mut self, return_index: usize) -> Vec<ReaderEffect> {
        self.reassociation_state.begin(
            PendingModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
                overflow: &mut self.pending_modal_overflow,
            },
            return_index,
        )
    }

    pub(super) fn handle_reassociation_paste(&mut self, text: &str) {
        ReassociationState::paste(
            PendingModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
                overflow: &mut self.pending_modal_overflow,
            },
            text,
        );
    }

    fn handle_reassociation_key(&mut self, key: InputKey) -> Vec<ReaderEffect> {
        let outcome = self.reassociation_state.key(
            PendingModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
                overflow: &mut self.pending_modal_overflow,
            },
            key,
            self.info.annotations.len(),
        );
        self.apply_modal_key(outcome)
    }

    pub fn reassociation_completed(
        &mut self,
        result: crate::annotations::ReassociationResult,
    ) -> Vec<ReaderEffect> {
        let completion = ReassociationState::completed(
            ModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
            },
            result,
            self.info.annotations.len(),
        );
        let Some(completion) = completion else {
            return Vec::new();
        };
        if let Some(copied) = completion.copied {
            Arc::make_mut(&mut self.info).annotations = copied.report.annotations;
            self.annotation_state
                .set_focused(copied.report.first_copied_id);
            self.export_metadata_loaded(copied.report.export);
            self.status = copied.status;
        }
        self.replay_modal_inputs(completion.inputs)
    }

    fn begin_link(&mut self) -> Vec<ReaderEffect> {
        self.markdown_state.begin(
            PendingModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
                overflow: &mut self.pending_modal_overflow,
            },
            self.info.annotations_enabled,
        )
    }

    pub(super) fn handle_link_paste(&mut self, text: &str) {
        MarkdownState::paste(
            PendingModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
                overflow: &mut self.pending_modal_overflow,
            },
            text,
        );
    }

    fn handle_link_key(&mut self, key: InputKey) -> Vec<ReaderEffect> {
        let outcome = self.markdown_state.key(
            PendingModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
                overflow: &mut self.pending_modal_overflow,
            },
            key,
        );
        self.apply_modal_key(outcome)
    }

    fn apply_modal_key(&mut self, outcome: ModalKeyOutcome) -> Vec<ReaderEffect> {
        match outcome {
            ModalKeyOutcome::Effects(effects) => effects,
            ModalKeyOutcome::Quit => self.execute(Command::Quit, 1),
        }
    }

    pub fn export_state(&self) -> &crate::annotations::ExportState {
        self.markdown_state.export()
    }

    pub(super) fn export_metadata_loaded(&mut self, state: crate::annotations::ExportState) {
        self.markdown_state.metadata_loaded(
            ModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
            },
            state,
        );
    }

    pub fn export_updated(&mut self, state: crate::annotations::ExportState) {
        self.markdown_state.export_updated(
            ModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
            },
            state,
        );
    }

    pub fn link_completed(&mut self, result: crate::annotations::LinkResult) -> Vec<ReaderEffect> {
        let inputs = self.markdown_state.completed(
            ModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
            },
            result,
        );
        inputs.map_or_else(Vec::new, |inputs| self.replay_modal_inputs(inputs))
    }

    fn begin_dictionary(&mut self) -> Vec<ReaderEffect> {
        DictionaryState::begin(ModeContext {
            mode: &mut self.mode,
            status: &mut self.status,
        })
    }

    fn dictionary_key(&mut self, key: InputKey) -> Vec<ReaderEffect> {
        let outcome = self.dictionary_state.key(
            ModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
            },
            ModeInput {
                key,
                command: self.configuration.bindings.command_for(key),
            },
            self.viewport.rows,
        );
        match outcome {
            DictionaryOutcome::Effects(effects) => effects,
            DictionaryOutcome::Navigate(command) => {
                let mut effects = vec![ReaderEffect::CancelDictionary];
                effects.extend(self.execute(command, 1));
                effects
            }
        }
    }

    pub(super) fn dictionary_paste(&mut self, text: &str) {
        DictionaryState::paste(
            ModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
            },
            text,
        );
    }

    pub fn dictionary_completed(&mut self, result: crate::integrations::DictionaryResult) {
        DictionaryState::completed(
            ModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
            },
            result,
        );
    }

    pub fn visible_images(&self) -> impl Iterator<Item = crate::images::ImageTarget> + '_ {
        crate::images::image_targets(
            &self.window,
            crate::reading::DocumentRow {
                document_index: self.rendered.index,
                row: self.row,
            },
            self.viewport.rows * self.spread,
        )
    }

    fn begin_images(&mut self) -> Vec<ReaderEffect> {
        let targets = self.visible_images().collect();
        ImageState::begin(
            ModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
            },
            targets,
        )
    }

    fn image_key(&mut self, key: InputKey) -> Vec<ReaderEffect> {
        self.image_state.key(
            ModeContext {
                mode: &mut self.mode,
                status: &mut self.status,
            },
            ModeInput {
                key,
                command: self.configuration.bindings.command_for(key),
            },
        )
    }

    pub fn image_completed(&mut self, result: crate::images::ImageResult) {
        ImageState::completed(&mut self.mode, result);
    }

    pub fn visible_row(&self, offset: usize) -> Option<crate::reading::VisibleRow<'_>> {
        self.window.row(
            crate::reading::DocumentRow {
                document_index: self.rendered.index,
                row: self.row,
            },
            offset,
        )
    }

    pub fn position(&self) -> ReadingState {
        self.view_position().text
    }

    pub fn view_position(&self) -> crate::reading::ViewPosition {
        if self.configuration.settings.between_chapters
            && let Ok(position) = self.window.capture_view(crate::reading::DocumentRow {
                document_index: self.rendered.index,
                row: self.row,
            })
        {
            return position;
        }
        crate::reading::ViewPosition::capture(&self.rendered.layout, self.row, self.width)
    }

    pub fn set_totals(&mut self, counts: Vec<usize>) {
        let mut sum = 0;
        let mut cumulative = vec![0];
        for count in counts {
            sum += count;
            cumulative.push(sum);
        }
        self.totals = Some(cumulative);
    }

    pub fn progress(&self) -> Option<f64> {
        let totals = self.totals.as_ref()?;
        let total = *totals.last()?;
        if total == 0 {
            return Some(0.0);
        }
        let position = self.position();
        let document = self
            .window
            .source_document(&position.content_id)
            .map_or(self.rendered.as_ref(), |document| document.as_ref());
        Some(
            (totals.get(document.index).copied().unwrap_or(0)
                + document.letters_before(position.offset)) as f64
                / total as f64,
        )
    }

    pub fn save_request(&self, quitting: bool) -> SaveRequest {
        SaveRequest {
            position: self.position(),
            progress: self.progress(),
            quitting,
        }
    }

    pub fn bookmarks_saved(&mut self, bookmarks: Vec<Bookmark>) {
        Arc::make_mut(&mut self.info).bookmarks = bookmarks;
        self.status = "Bookmark saved.".into();
    }

    pub(super) fn last_row(&self) -> usize {
        if self.configuration.settings.between_chapters {
            return self.rendered.layout.text_lines.len().saturating_sub(1);
        }
        self.rendered
            .layout
            .text_lines
            .len()
            .saturating_sub(self.viewport.rows * self.spread)
    }

    pub(super) fn available_width(&self) -> usize {
        self.viewport.available_width(self.spread)
    }

    pub(super) fn request_load(&mut self, request: LoadRequest) -> Vec<ReaderEffect> {
        if self.configuration.settings.between_chapters {
            return self.request_window(crate::reading::WindowRequest {
                load: request,
                shift_rows: 0,
                viewport_rows: self.viewport.rows * self.spread,
            });
        }
        self.loading = true;
        self.loading_request = Some(request.clone());
        self.width = request.width;
        self.status = "Loading…".into();
        vec![ReaderEffect::Load(request)]
    }

    pub fn accept_loaded(
        &mut self,
        document: Arc<RenderedDocument>,
        request: LoadRequest,
    ) -> Vec<ReaderEffect> {
        let remap_error = if let ReaderMode::Annotations(mode) = &mut self.mode {
            mode.remap(&self.rendered.layout, &document.layout)
                .err()
                .map(|error| error.to_string())
        } else {
            None
        };
        self.replace_document(document);
        self.width = request.width;
        self.loading = false;
        self.loading_request = None;
        self.pending_hyperlink = None;
        self.status = remap_error.unwrap_or_default();
        self.row = if request.at_end {
            self.last_row()
        } else if let Some(fragment) = request.fragment {
            let decoded = percent_encoding::percent_decode_str(&fragment).decode_utf8_lossy();
            self.rendered
                .parsed
                .semantics
                .anchors
                .iter()
                .find(|anchor| anchor.fragment == decoded)
                .map_or(0, |anchor| {
                    self.rendered.layout.row_for_offset(anchor.offset)
                })
        } else {
            request
                .position
                .map_or(0, |position| position.row_in(&self.rendered.layout))
        };
        self.row = self
            .row
            .min(self.rendered.layout.text_lines.len().saturating_sub(1));
        self.loaded_effects()
    }

    pub(super) fn loaded_effects(&mut self) -> Vec<ReaderEffect> {
        self.finish_page_turn();
        self.history_arrived();
        let mut effects = self.search_reflow();
        if let Some(result) = self.search_state.take_deferred_result() {
            effects.extend(self.search_completed(result));
        }
        while !self.loading && !self.quitting {
            let Some(input) = self.deferred_modal_input.pop_front() else {
                break;
            };
            match input {
                ModalInput::Key(key) => effects.extend(self.handle_key(key)),
                ModalInput::Paste(text) => self.handle_paste(&text),
            }
        }
        while !self.loading && !self.quitting {
            let Some(input) = self.pending.pop_front() else {
                break;
            };
            match input {
                QueuedInput::Command { command, count, .. } => {
                    effects.extend(self.execute(command, count))
                }
                QueuedInput::Input(ModalInput::Key(key)) => effects.extend(self.handle_key(key)),
                QueuedInput::Input(ModalInput::Paste(text)) => self.handle_paste(&text),
                QueuedInput::Mouse {
                    action, context, ..
                } if context == mouse::MouseContext::from(&self.mode) => {
                    effects.extend(self.handle_mouse(action));
                }
                QueuedInput::Mouse { .. } => {}
            }
        }
        if !self.loading && self.quit_after_load && !self.quitting {
            effects.extend(self.execute(Command::Quit, 1));
        }
        if !self.loading && !self.quitting && self.pending_input_overflow {
            self.pending_input_overflow = false;
            self.append_input_warning();
        }
        effects
    }

    pub fn resize(&mut self, viewport: Viewport) -> Vec<ReaderEffect> {
        let dictionary = matches!(self.mode, ReaderMode::Dictionary(_));
        if dictionary {
            self.mode = ReaderMode::Reading;
            self.status.clear();
        }
        let mut effects = self.resize_viewport(viewport);
        if dictionary {
            effects.push(ReaderEffect::CancelDictionary);
        }
        effects
    }

    fn resize_viewport(&mut self, viewport: Viewport) -> Vec<ReaderEffect> {
        self.cancel_page_turn();
        self.viewport = viewport;
        if matches!(self.mode, ReaderMode::Hyperlinks(_))
            || matches!(&self.mode, ReaderMode::Images(mode) if matches!(mode.as_ref(), ImageMode::Selecting(_)))
        {
            self.mode = ReaderMode::Reading;
        }
        if matches!(&self.mode, ReaderMode::Library(mode) if matches!(&mode.phase, LibraryPhase::Opening(_)))
        {
            return Vec::new();
        }
        let width = self.width.min(self.available_width()).max(1);
        if let Some(link) = self.pending_hyperlink.clone() {
            self.width = width;
            return self.request_hyperlink(link);
        }
        if self.configuration.settings.between_chapters {
            let load = if let Some(mut load) = self.loading_request.clone() {
                load.width = width;
                load
            } else {
                LoadRequest {
                    index: self.rendered.index,
                    width,
                    position: Some(self.view_position()),
                    at_end: false,
                    fragment: None,
                }
            };
            return self.request_window(crate::reading::WindowRequest {
                load,
                shift_rows: self.pending_shift,
                viewport_rows: self.viewport.rows * self.spread,
            });
        }
        if width != self.width {
            if let Some(mut request) = self.loading_request.clone() {
                request.width = width;
                return self.request_load(request);
            }
            self.request_load(LoadRequest {
                index: self.rendered.index,
                width,
                position: Some(self.view_position()),
                at_end: false,
                fragment: None,
            })
        } else {
            self.row = self
                .row
                .min(self.rendered.layout.text_lines.len().saturating_sub(1));
            Vec::new()
        }
    }

    pub fn handle_key(&mut self, key: InputKey) -> Vec<ReaderEffect> {
        self.input_warning = false;
        self.cancel_page_turn();
        if self.quitting {
            return Vec::new();
        }
        if self.loading && self.pending.has_mode_sensitive_input() {
            self.queue_input(QueuedInput::Input(ModalInput::Key(key)));
            return Vec::new();
        }
        if matches!(self.mode, ReaderMode::Dictionary(_)) {
            return self.dictionary_key(key);
        }
        if matches!(self.mode, ReaderMode::Images(_)) {
            return self.image_key(key);
        }
        if matches!(self.mode, ReaderMode::Hyperlinks(_)) {
            return self.hyperlink_key(key);
        }
        if matches!(self.mode, ReaderMode::Library(_)) {
            return self.handle_library_key(key);
        }
        if matches!(self.mode, ReaderMode::Search(_)) {
            return self.handle_search_key(key);
        }
        if matches!(self.mode, ReaderMode::Reassociation(_)) {
            return self.handle_reassociation_key(key);
        }
        if matches!(self.mode, ReaderMode::Link(_)) {
            return self.handle_link_key(key);
        }
        if matches!(self.mode, ReaderMode::Annotations(_)) {
            return self.handle_annotation_key(key);
        }
        if key == InputKey::Character('\u{03}') {
            return self.execute(Command::Quit, 1);
        }
        if !matches!(self.mode, ReaderMode::Reading) {
            return self.handle_mode_key(key);
        }
        if let InputKey::Character(character) = key
            && character.is_ascii_digit()
        {
            if self.prefix.len() < 7 {
                self.prefix.push(character);
            }
            return Vec::new();
        }
        let count = self.take_count();
        let Some(command) = self.configuration.bindings.command_for(key) else {
            return Vec::new();
        };
        if self.loading {
            if command == Command::Quit {
                self.quit_after_load = true;
                self.status = "Waiting for the requested position before saving…".into();
            } else if is_motion(command) {
                self.queue_input(QueuedInput::Command {
                    command,
                    count,
                    repetitions: 1,
                });
            } else {
                self.queue_prefixed_key(key, count);
            }
            return Vec::new();
        }
        self.execute(command, count)
    }

    pub(super) fn queue_input(&mut self, input: QueuedInput) {
        if !self.pending.push(input) {
            self.report_input_overflow();
        }
    }

    pub(super) fn defer_modal_input(&mut self, input: ModalInput) {
        if !self.deferred_modal_input.push(input) {
            self.report_input_overflow();
        }
    }

    fn queue_prefixed_key(&mut self, key: InputKey, count: usize) {
        let prefix = if count > 1 {
            count.to_string()
        } else {
            String::new()
        };
        if !self.pending.can_fit(prefix.len() + 1) {
            self.report_input_overflow();
            return;
        }
        for digit in prefix.chars() {
            self.queue_input(QueuedInput::Input(ModalInput::Key(InputKey::Character(
                digit,
            ))));
        }
        self.queue_input(QueuedInput::Input(ModalInput::Key(key)));
    }

    fn report_input_overflow(&mut self) {
        self.pending_input_overflow = true;
        self.status = "Input queue full. New input was ignored.".into();
    }

    pub(super) fn report_modal_overflow(&mut self) {
        super::modes::report_modal_overflow(&mut self.pending_modal_overflow, &mut self.status);
    }

    pub(super) fn replay_modal_inputs(
        &mut self,
        inputs: VecDeque<ModalInput>,
    ) -> Vec<ReaderEffect> {
        let ignored = std::mem::take(&mut self.pending_modal_overflow);
        let mut effects = Vec::new();
        for input in inputs {
            match input {
                ModalInput::Key(key) => effects.extend(self.handle_key(key)),
                ModalInput::Paste(text) => self.handle_paste(&text),
            }
        }
        if ignored && !self.quitting {
            self.pending_input_overflow |= self.loading;
            self.append_input_warning();
        }
        effects
    }

    pub(super) fn append_input_warning(&mut self) {
        self.input_warning = true;
        let previous = std::mem::take(&mut self.status);
        self.status = "Some input was ignored. Please retry.".into();
        if !previous.is_empty() {
            self.status.push(' ');
            self.status.push_str(&previous);
        }
    }

    pub(super) fn take_count(&mut self) -> usize {
        std::mem::take(&mut self.prefix)
            .parse::<usize>()
            .unwrap_or(1)
            .max(1)
    }

    pub(super) fn execute(&mut self, command: Command, count: usize) -> Vec<ReaderEffect> {
        self.cancel_page_turn();
        self.status.clear();
        match command {
            Command::ScrollDown => self.scroll_rows(count as isize),
            Command::ScrollUp => self.scroll_rows(-(count as isize)),
            Command::PageDown => {
                self.page_rows((self.viewport.rows * self.spread).saturating_mul(count) as isize)
            }
            Command::PageUp => {
                self.page_rows(-((self.viewport.rows * self.spread).saturating_mul(count) as isize))
            }
            Command::HalfScreenDown => {
                self.move_rows((self.viewport.rows / 2).max(1).saturating_mul(count) as isize)
            }
            Command::HalfScreenUp => {
                self.move_rows(-((self.viewport.rows / 2).max(1).saturating_mul(count) as isize))
            }
            Command::NextChapter => self.change_chapter(
                self.rendered
                    .index
                    .saturating_add(count)
                    .min(self.info.contents.len() - 1),
                false,
            ),
            Command::PrevChapter => {
                self.change_chapter(self.rendered.index.saturating_sub(count), false)
            }
            Command::BeginningOfCh if self.configuration.settings.between_chapters => self
                .request_load(LoadRequest {
                    index: 0,
                    width: self.width,
                    position: None,
                    at_end: false,
                    fragment: None,
                }),
            Command::EndOfCh if self.configuration.settings.between_chapters => {
                self.request_load(LoadRequest {
                    index: self.info.contents.len() - 1,
                    width: self.width,
                    position: None,
                    at_end: true,
                    fragment: None,
                })
            }
            Command::BeginningOfCh => {
                self.row = 0;
                Vec::new()
            }
            Command::EndOfCh => {
                self.row = self.last_row();
                Vec::new()
            }
            Command::Enlarge | Command::Shrink | Command::SetWidth if self.spread > 1 => Vec::new(),
            Command::Enlarge => {
                self.reflow(self.width.saturating_add(count).min(self.available_width()))
            }
            Command::Shrink => self.reflow(
                self.width
                    .saturating_sub(count)
                    .max(20.min(self.available_width())),
            ),
            Command::SetWidth => self.reflow(if count > 1 {
                count.min(self.available_width())
            } else if self.width == 80 {
                self.available_width()
            } else {
                80.min(self.available_width())
            }),
            Command::DoubleSpreadToggle => {
                self.spread = if self.spread == 1 { 2 } else { 1 };
                self.reflow(if self.spread == 2 {
                    self.available_width()
                } else {
                    80.min(self.available_width())
                })
            }
            Command::ShowHideProgress => {
                self.show_progress = !self.show_progress;
                Vec::new()
            }
            Command::SwitchColor => {
                self.color_scheme = (self.color_scheme + 1) % 3;
                Vec::new()
            }
            Command::Help => {
                self.open_help();
                Vec::new()
            }
            Command::Metadata => {
                self.open_metadata();
                Vec::new()
            }
            Command::TableOfContents => {
                self.open_toc();
                Vec::new()
            }
            Command::Library => self.begin_library(),
            Command::Follow => self.begin_hyperlinks(),
            Command::OpenImage => self.begin_images(),
            Command::RegexSearch => self.begin_search(),
            Command::JumpBackward => self.history_jump(false, count),
            Command::JumpForward => self.history_jump(true, count),
            Command::MarkPosition | Command::JumpToPosition => {
                let saving = command == Command::MarkPosition;
                self.mode = ReaderMode::Quickmark { saving };
                self.status = if saving {
                    "Quickmark to save (0–9), Escape to cancel"
                } else {
                    "Quickmark to jump to (0–9), Escape to cancel"
                }
                .into();
                Vec::new()
            }
            Command::DefineWord => self.begin_dictionary(),
            Command::AnnotationSelect => self.begin_selection(),
            Command::AnnotationList => self.begin_annotation_list(),
            Command::AnnotationLink => self.begin_link(),
            Command::ShowBookmarks => {
                self.open_bookmarks();
                Vec::new()
            }
            Command::AddBookmark => {
                self.mode = ReaderMode::BookmarkPrompt(String::new());
                self.status = "Bookmark name, Enter to save, Escape to cancel".into();
                Vec::new()
            }
            Command::Quit if self.loading => {
                self.quit_after_load = true;
                self.status = "Waiting for the requested position before saving…".into();
                Vec::new()
            }
            Command::Quit => {
                self.quit_after_load = false;
                self.quitting = true;
                self.status = "Saving reading position…".into();
                vec![ReaderEffect::Quit(self.save_request(true))]
            }
        }
    }

    pub(super) fn reflow(&mut self, width: usize) -> Vec<ReaderEffect> {
        if width == self.width {
            return Vec::new();
        }
        self.request_load(LoadRequest {
            index: self.rendered.index,
            width,
            position: Some(self.view_position()),
            at_end: false,
            fragment: None,
        })
    }

    fn change_chapter(&mut self, index: usize, at_end: bool) -> Vec<ReaderEffect> {
        if index == self.rendered.index {
            return Vec::new();
        }
        self.request_load(LoadRequest {
            index,
            width: self.width,
            position: None,
            at_end,
            fragment: None,
        })
    }

    pub(super) fn move_rows(&mut self, delta: isize) -> Vec<ReaderEffect> {
        if self.configuration.settings.between_chapters {
            return self.continuous_rows(delta);
        }
        if delta > 0
            && self.row >= self.last_row()
            && self.rendered.index + 1 < self.info.contents.len()
        {
            return self.change_chapter(self.rendered.index + 1, false);
        }
        if delta < 0 && self.row == 0 && self.rendered.index > 0 {
            return self.change_chapter(self.rendered.index - 1, true);
        }
        self.row = self
            .row
            .saturating_add_signed(delta)
            .min(self.last_row().max(self.row));
        Vec::new()
    }

    fn handle_mode_key(&mut self, key: InputKey) -> Vec<ReaderEffect> {
        if let ReaderMode::Quickmark { saving } = self.mode {
            return self.quickmark_key(saving, key);
        }
        match &mut self.mode {
            ReaderMode::Dialog(dialog) => match key {
                InputKey::Escape | InputKey::Character('q') => self.mode = ReaderMode::Reading,
                InputKey::Up | InputKey::Character('k') => {
                    dialog.selected = dialog.selected.saturating_sub(1)
                }
                InputKey::Down | InputKey::Character('j') => {
                    dialog.selected =
                        (dialog.selected + 1).min(dialog.items.len().saturating_sub(1))
                }
                InputKey::Enter => {
                    let target = dialog
                        .items
                        .get(dialog.selected)
                        .and_then(|item| item.target.clone());
                    self.mode = ReaderMode::Reading;
                    if let Some(target) = target {
                        return self.request_jump(LoadRequest {
                            index: target.index,
                            width: self.width,
                            position: target.position.map(Into::into),
                            at_end: false,
                            fragment: target.fragment,
                        });
                    }
                }
                _ => {}
            },
            ReaderMode::BookmarkPrompt(value) => match key {
                InputKey::Escape => {
                    self.mode = ReaderMode::Reading;
                    self.status.clear();
                }
                InputKey::Enter => {
                    let name = value.clone();
                    self.mode = ReaderMode::Reading;
                    if !name.is_empty() {
                        self.status = "Saving bookmark…".into();
                        return vec![ReaderEffect::Bookmark(BookmarkRequest {
                            name,
                            position: self.position(),
                        })];
                    }
                }
                InputKey::Backspace | InputKey::Character('\u{7f}' | '\u{08}') => {
                    let count = crate::document::text::graphemes(value)
                        .last()
                        .map_or(0, |cluster| cluster.len());
                    value.truncate(value.len().saturating_sub(count));
                }
                InputKey::Character(character) if !character.is_control() => value.push(character),
                _ => {}
            },
            ReaderMode::Reading
            | ReaderMode::Quickmark { .. }
            | ReaderMode::Annotations(_)
            | ReaderMode::Link(_)
            | ReaderMode::Reassociation(_)
            | ReaderMode::Search(_)
            | ReaderMode::Hyperlinks(_)
            | ReaderMode::Images(_)
            | ReaderMode::Library(_)
            | ReaderMode::Dictionary(_) => {}
        }
        Vec::new()
    }

    fn open_dialog(&mut self, title: &str, items: Vec<DialogItem>) {
        self.mode = ReaderMode::Dialog(Dialog {
            title: title.into(),
            items,
            selected: 0,
        });
    }

    fn open_toc(&mut self) {
        self.open_dialog(
            "Table of Contents",
            self.info
                .toc
                .iter()
                .map(|entry| DialogItem {
                    label: entry.label.clone(),
                    target: Some(JumpTarget {
                        index: entry.content_index,
                        position: None,
                        fragment: entry.section.clone(),
                    }),
                })
                .collect(),
        );
    }

    fn open_bookmarks(&mut self) {
        self.open_dialog(
            "Bookmarks",
            self.info
                .bookmarks
                .iter()
                .filter_map(|bookmark| {
                    let index = self
                        .info
                        .contents
                        .iter()
                        .position(|content| *content == bookmark.position.content_id)?;
                    Some(DialogItem {
                        label: bookmark.name.clone(),
                        target: Some(JumpTarget {
                            index,
                            position: Some(bookmark.position.clone()),
                            fragment: None,
                        }),
                    })
                })
                .collect(),
        );
    }

    fn open_metadata(&mut self) {
        let metadata = &self.info.metadata;
        let lines = [
            metadata.title.clone().unwrap_or_default(),
            metadata.creator.clone().unwrap_or_default(),
            metadata.publisher.clone().unwrap_or_default(),
            metadata.description.clone().unwrap_or_default(),
            self.info.path.clone(),
        ];
        self.open_dialog(
            "Metadata",
            lines
                .into_iter()
                .map(|label| DialogItem {
                    label,
                    target: None,
                })
                .collect(),
        );
    }

    fn open_help(&mut self) {
        self.open_dialog(
            "ers Help",
            self.configuration
                .bindings
                .clone()
                .effective_keys(Command::Quit)
                .ok()
                .map_or_else(Vec::new, |_| {
                    Command::all()
                        .iter()
                        .map(|command| DialogItem {
                            label: format!(
                                "{}  {}",
                                self.configuration
                                    .bindings
                                    .binding(*command)
                                    .unwrap_or_default()
                                    .escape_default(),
                                command.name()
                            ),
                            target: None,
                        })
                        .collect()
                }),
        );
    }
}

fn is_motion(command: Command) -> bool {
    matches!(
        command,
        Command::ScrollDown
            | Command::ScrollUp
            | Command::PageDown
            | Command::PageUp
            | Command::HalfScreenDown
            | Command::HalfScreenUp
            | Command::NextChapter
            | Command::PrevChapter
    )
}
