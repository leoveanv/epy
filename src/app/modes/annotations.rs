use super::{ModalInput, QueuedInput};

use crate::annotations::{
    Annotation, AnnotationCommand, AnnotationDraft, AnnotationJob, AnnotationResult, NoteUpdate,
    TextAnchor,
};
use crate::annotations::{Selection, SelectionMotion, SelectionWindow};
use crate::app::editor::{EditorAction, EditorOptions, EditorState};
use crate::configuration::{Command, InputKey};
use crate::reading::LoadRequest;
use crate::reading::ReadingState;

use super::{ReaderEffect, ReaderMode, ReaderModel};

#[derive(Clone, Debug)]
pub enum AnnotationReturn {
    Reading,
    Selecting(Selection),
    List(usize),
}

#[derive(Clone, Debug)]
pub enum NoteTarget {
    New(TextAnchor),
    Existing(Box<Annotation>),
}

impl NoteTarget {
    pub fn anchor(&self) -> &TextAnchor {
        match self {
            Self::New(anchor) => anchor,
            Self::Existing(annotation) => &annotation.anchor,
        }
    }
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::New(_) => None,
            Self::Existing(annotation) => Some(&annotation.id),
        }
    }
}

#[derive(Clone, Debug)]
pub struct NoteEditor {
    pub state: EditorState,
    pub target: NoteTarget,
    pub return_to: AnnotationReturn,
}

#[derive(Clone, Debug)]
pub enum ConfirmedAction {
    DeleteHighlight(Annotation),
    DeleteNote(Annotation),
}

#[derive(Clone, Debug)]
pub struct AnnotationConfirmation {
    pub message: String,
    pub action: ConfirmedAction,
    pub return_to: AnnotationReturn,
}

#[derive(Clone, Copy, Debug)]
pub enum AnnotationChoiceAction {
    Edit,
    Delete,
}

#[derive(Clone, Debug)]
pub struct AnnotationChooser {
    pub records: Vec<Annotation>,
    pub selected: usize,
    pub action: AnnotationChoiceAction,
    pub return_to: AnnotationReturn,
}

#[derive(Clone, Debug)]
pub struct NoteViewer {
    pub annotation: Annotation,
    pub scroll: usize,
    pub return_to: AnnotationReturn,
}

#[derive(Clone, Debug)]
pub struct AnnotationPendingResume {
    pub return_to: AnnotationReturn,
    pub retry: Box<AnnotationMode>,
}

pub type PendingAnnotation =
    super::pending::PendingOperation<AnnotationJob, AnnotationPendingResume>;

#[derive(Default)]
pub(in crate::app) struct AnnotationState {
    requests: super::RequestSequence,
    focused: Option<String>,
}

impl AnnotationState {
    fn start(
        &mut self,
        command: AnnotationCommand,
        return_to: AnnotationReturn,
        retry: AnnotationMode,
    ) -> PendingAnnotation {
        let job = AnnotationJob {
            request_id: self.requests.next_saturating(),
            command,
        };
        PendingAnnotation::new(
            job,
            AnnotationPendingResume {
                return_to,
                retry: Box::new(retry),
            },
        )
    }

    pub(in crate::app) fn focused(&self) -> Option<&str> {
        self.focused.as_deref()
    }

    pub(in crate::app) fn set_focused(&mut self, id: Option<String>) {
        self.focused = id;
    }
}

#[derive(Clone, Debug)]
pub enum AnnotationMode {
    Selecting(Selection),
    List { selected: usize },
    Editing(Box<NoteEditor>),
    QuitNote(Box<NoteEditor>),
    Confirming(Box<AnnotationConfirmation>),
    Choosing(Box<AnnotationChooser>),
    Viewing(Box<NoteViewer>),
    Pending(Box<PendingAnnotation>),
}

impl AnnotationMode {
    fn editing(editor: NoteEditor) -> Self {
        Self::Editing(Box::new(editor))
    }
    fn confirming(confirmation: AnnotationConfirmation) -> Self {
        Self::Confirming(Box::new(confirmation))
    }
    fn choosing(chooser: AnnotationChooser) -> Self {
        Self::Choosing(Box::new(chooser))
    }
    fn viewing(viewer: NoteViewer) -> Self {
        Self::Viewing(Box::new(viewer))
    }
    fn pending(pending: PendingAnnotation) -> Self {
        Self::Pending(Box::new(pending))
    }
    pub fn selection(&self) -> Option<&Selection> {
        match self {
            Self::Selecting(selection) => Some(selection),
            Self::Editing(editor) | Self::QuitNote(editor) => return_selection(&editor.return_to),
            Self::Confirming(confirmation) => return_selection(&confirmation.return_to),
            Self::Choosing(chooser) => return_selection(&chooser.return_to),
            Self::Pending(pending) => pending.resume.retry.selection(),
            _ => None,
        }
    }

    pub fn editor(&self) -> Option<&NoteEditor> {
        match self {
            Self::Editing(editor) | Self::QuitNote(editor) => Some(editor),
            Self::Pending(pending) => pending.resume.retry.editor(),
            _ => None,
        }
    }

    pub fn remap(
        &mut self,
        previous: &dyn crate::annotations::SelectionSurface,
        current: &dyn crate::annotations::SelectionSurface,
    ) -> crate::Result<()> {
        match self {
            Self::Selecting(selection) => selection.remap(previous, current)?,
            Self::Editing(editor) | Self::QuitNote(editor) => {
                remap_return(&mut editor.return_to, previous, current)?
            }
            Self::Confirming(confirmation) => {
                remap_return(&mut confirmation.return_to, previous, current)?
            }
            Self::Choosing(chooser) => remap_return(&mut chooser.return_to, previous, current)?,
            Self::Pending(pending) => {
                pending.resume.retry.remap(previous, current)?;
                remap_return(&mut pending.resume.return_to, previous, current)?;
            }
            _ => {}
        }
        Ok(())
    }
}

fn return_selection(context: &AnnotationReturn) -> Option<&Selection> {
    if let AnnotationReturn::Selecting(selection) = context {
        Some(selection)
    } else {
        None
    }
}

fn remap_return(
    context: &mut AnnotationReturn,
    previous: &dyn crate::annotations::SelectionSurface,
    current: &dyn crate::annotations::SelectionSurface,
) -> crate::Result<()> {
    if let AnnotationReturn::Selecting(selection) = context {
        selection.remap(previous, current)?;
    }
    Ok(())
}

impl ReaderModel {
    pub fn active_selection(&self) -> Option<&Selection> {
        if let ReaderMode::Annotations(mode) = &self.mode {
            mode.selection()
        } else {
            None
        }
    }
    pub fn note_editor(&self) -> Option<&NoteEditor> {
        if let ReaderMode::Annotations(mode) = &self.mode {
            mode.editor()
        } else {
            None
        }
    }

    pub fn reading_height(&self) -> usize {
        self.viewport.rows + usize::from(self.active_selection().is_some())
    }

    pub(in crate::app) fn begin_selection(&mut self) -> Vec<ReaderEffect> {
        if let Some(effects) = self.ensure_annotation_column() {
            return effects;
        }
        match Selection::from_viewport(
            self.selection_surface(),
            self.selection_top(),
            self.viewport.rows + 1,
        ) {
            Ok(selection) => {
                self.mode = ReaderMode::Annotations(AnnotationMode::Selecting(selection));
                self.status.clear();
            }
            Err(error) => self.status = error.to_string(),
        }
        Vec::new()
    }

    pub(in crate::app) fn begin_annotation_list(&mut self) -> Vec<ReaderEffect> {
        if let Some(effects) = self.ensure_annotation_column() {
            return effects;
        }
        let selected = self
            .annotation_state
            .focused()
            .and_then(|id| {
                self.info
                    .annotations
                    .iter()
                    .position(|annotation| annotation.id == id)
            })
            .unwrap_or(0);
        self.mode = ReaderMode::Annotations(AnnotationMode::List { selected });
        Vec::new()
    }

    fn ensure_annotation_column(&mut self) -> Option<Vec<ReaderEffect>> {
        if !self.info.annotations_enabled {
            self.status = "Annotations are available only for local EPUB books.".into();
            return Some(Vec::new());
        }
        if self.spread != 2 {
            return None;
        }
        self.spread = 1;
        let effects = self.reflow(80.min(self.available_width()));
        self.status = "Switched to one column. Press the annotation key again.".into();
        Some(effects)
    }

    pub fn handle_paste(&mut self, text: &str) {
        self.input_warning = false;
        self.cancel_page_turn();
        if matches!(
            self.mode,
            ReaderMode::Library(_) | ReaderMode::Hyperlinks(_) | ReaderMode::Images(_)
        ) {
            return;
        }
        if matches!(self.mode, ReaderMode::Dictionary(_)) {
            self.dictionary_paste(text);
            return;
        }
        if matches!(self.mode, ReaderMode::Search(_)) {
            self.handle_search_paste(text);
            return;
        }
        if self.loading && self.pending.has_mode_sensitive_input() {
            self.queue_input(QueuedInput::Input(ModalInput::Paste(text.into())));
            return;
        }
        if matches!(self.mode, ReaderMode::Reassociation(_)) {
            self.handle_reassociation_paste(text);
            return;
        }
        if matches!(self.mode, ReaderMode::Link(_)) {
            self.handle_link_paste(text);
            return;
        }
        if self.loading && matches!(self.mode, ReaderMode::Annotations(_)) {
            self.defer_modal_input(ModalInput::Paste(text.into()));
            return;
        }
        let accepted = match &mut self.mode {
            ReaderMode::BookmarkPrompt(value) => {
                value.extend(text.chars().filter(|character| !character.is_control()));
                true
            }
            ReaderMode::Annotations(AnnotationMode::Editing(editor))
                if editor.state.focus() == 0 =>
            {
                editor.state.insert(text);
                true
            }
            ReaderMode::Annotations(AnnotationMode::Pending(pending)) => pending.push_paste(text),
            _ => true,
        };
        if !accepted {
            self.report_modal_overflow();
        }
    }

    pub(in crate::app) fn handle_annotation_key(&mut self, key: InputKey) -> Vec<ReaderEffect> {
        if self.loading {
            self.defer_modal_input(ModalInput::Key(key));
            return Vec::new();
        }
        let ReaderMode::Annotations(mode) = std::mem::replace(&mut self.mode, ReaderMode::Reading)
        else {
            return Vec::new();
        };
        match mode {
            AnnotationMode::Selecting(selection) => self.selection_key(selection, key),
            AnnotationMode::List { selected } => self.annotation_list_key(selected, key),
            AnnotationMode::Editing(editor) => self.editor_key(*editor, key),
            AnnotationMode::QuitNote(editor) => self.quit_note_key(*editor, key),
            AnnotationMode::Confirming(confirmation) => self.confirmation_key(*confirmation, key),
            AnnotationMode::Choosing(chooser) => self.chooser_key(*chooser, key),
            AnnotationMode::Viewing(viewer) => self.viewer_key(*viewer, key),
            AnnotationMode::Pending(pending) => self.pending_annotation_key(*pending, key),
        }
    }

    fn selection_key(&mut self, mut selection: Selection, key: InputKey) -> Vec<ReaderEffect> {
        let layout = self.selection_surface();
        let mut top = self.selection_top();
        let mut selection_error = None;
        if key == InputKey::Escape
            || self
                .configuration
                .bindings
                .effective_keys(Command::AnnotationSelect)
                .is_ok_and(|keys| keys.contains(&key))
        {
            self.status.clear();
            return Vec::new();
        }
        if key == InputKey::Character('\u{03}') {
            return self.execute(Command::Quit, 1);
        }
        let motion = match key {
            InputKey::Left | InputKey::Character('h') => Some(SelectionMotion::Left),
            InputKey::Right | InputKey::Character('l') => Some(SelectionMotion::Right),
            InputKey::Up | InputKey::Character('k') => Some(SelectionMotion::Up),
            InputKey::Down | InputKey::Character('j') => Some(SelectionMotion::Down),
            InputKey::Character('w') => Some(SelectionMotion::WordForward),
            InputKey::Character('b') => Some(SelectionMotion::WordBackward),
            _ => None,
        };
        if let Some(motion) = motion {
            selection.move_cursor(motion, layout);
        } else {
            match key {
                InputKey::Character('\u{04}') | InputKey::Character('\u{15}') => {
                    let distance = self.viewport.rows.div_ceil(2).max(1) as isize;
                    let before = selection.cell(layout).map_or(top, |cell| cell.row);
                    selection.move_rows(
                        if key == InputKey::Character('\u{04}') {
                            distance
                        } else {
                            -distance
                        },
                        layout,
                    );
                    let after = selection.cell(layout).map_or(before, |cell| cell.row);
                    top = top.saturating_add_signed(after as isize - before as isize);
                }
                InputKey::Character(' ') => selection.start_range(),
                InputKey::Enter | InputKey::Character('n') if selection.has_range() => {
                    match selection.anchor(self.selection_document(&selection), layout) {
                        Ok(anchor) if key == InputKey::Character('n') => {
                            self.open_note_editor(
                                NoteTarget::New(anchor),
                                AnnotationReturn::Selecting(selection),
                            );
                            return Vec::new();
                        }
                        Ok(anchor) => {
                            let resume = AnnotationReturn::Selecting(selection.clone());
                            return self.submit_annotation(
                                AnnotationCommand::Create(AnnotationDraft {
                                    anchor,
                                    note: String::new(),
                                }),
                                resume,
                                AnnotationMode::Selecting(selection),
                            );
                        }
                        Err(error) => selection_error = Some(error.to_string()),
                    }
                }
                InputKey::Character('n') => {
                    let records = self.annotations_at_selection(&selection);
                    return self.choose_annotation(
                        records,
                        AnnotationChoiceAction::Edit,
                        AnnotationReturn::Selecting(selection),
                    );
                }
                InputKey::Character('d') | InputKey::Delete | InputKey::Backspace => {
                    let records = self.annotations_at_selection(&selection);
                    return self.choose_annotation(
                        records,
                        AnnotationChoiceAction::Delete,
                        AnnotationReturn::Selecting(selection),
                    );
                }
                _ => {}
            }
        }
        let top = selection.viewport(
            layout,
            SelectionWindow {
                top,
                height: self.viewport.rows + 1,
                scrolloff: self.configuration.settings.annotation_scrolloff as usize,
            },
        );
        self.set_selection_top(top);
        if let Some(error) = selection_error {
            self.status = error;
        }
        self.mode = ReaderMode::Annotations(AnnotationMode::Selecting(selection));
        Vec::new()
    }

    fn selection_document(&self, selection: &Selection) -> &crate::document::text::ParsedDocument {
        selection
            .cell(self.selection_surface())
            .and_then(|cell| self.window.source_document(&cell.content_id))
            .map_or(&self.rendered.parsed, |document| &document.parsed)
    }

    fn annotations_at_selection(&self, selection: &Selection) -> Vec<Annotation> {
        let Some(cell) = selection.cell(self.selection_surface()) else {
            return Vec::new();
        };
        self.info
            .annotations
            .iter()
            .filter(|annotation| {
                annotation.anchor.content_id() == cell.content_id
                    && annotation.anchor.start() < cell.end
                    && annotation.anchor.end() > cell.start
                    && annotation
                        .anchor
                        .resolve(&self.selection_document(selection).source)
                        .is_some()
            })
            .cloned()
            .collect()
    }

    fn choose_annotation(
        &mut self,
        records: Vec<Annotation>,
        action: AnnotationChoiceAction,
        return_to: AnnotationReturn,
    ) -> Vec<ReaderEffect> {
        if records.is_empty() {
            self.restore_annotation_return(return_to, false);
            self.status = "There is no highlight at the cursor.".into();
        } else if records.len() == 1 {
            self.apply_annotation_choice(records[0].clone(), action, return_to);
        } else {
            self.mode = ReaderMode::Annotations(AnnotationMode::choosing(AnnotationChooser {
                records,
                selected: 0,
                action,
                return_to,
            }));
        }
        Vec::new()
    }

    fn apply_annotation_choice(
        &mut self,
        annotation: Annotation,
        action: AnnotationChoiceAction,
        return_to: AnnotationReturn,
    ) {
        match action {
            AnnotationChoiceAction::Edit => {
                self.open_note_editor(NoteTarget::Existing(Box::new(annotation)), return_to)
            }
            AnnotationChoiceAction::Delete => {
                self.mode =
                    ReaderMode::Annotations(AnnotationMode::confirming(AnnotationConfirmation {
                        message: "Delete this highlight and its note?".into(),
                        action: ConfirmedAction::DeleteHighlight(annotation),
                        return_to,
                    }))
            }
        }
    }

    fn open_note_editor(&mut self, target: NoteTarget, return_to: AnnotationReturn) {
        let text = match &target {
            NoteTarget::New(_) => String::new(),
            NoteTarget::Existing(annotation) => annotation.note.clone(),
        };
        let state = EditorState::new(
            text,
            EditorOptions {
                multiline: true,
                can_delete: matches!(target, NoteTarget::Existing(_)),
            },
        );
        self.mode = ReaderMode::Annotations(AnnotationMode::editing(NoteEditor {
            state,
            target,
            return_to,
        }));
        self.status.clear();
    }

    fn quit_note_key(&mut self, editor: NoteEditor, key: InputKey) -> Vec<ReaderEffect> {
        if key == InputKey::Character('y') {
            return self.execute(Command::Quit, 1);
        }
        self.mode = ReaderMode::Annotations(AnnotationMode::editing(editor));
        self.status.clear();
        Vec::new()
    }

    fn editor_key(&mut self, mut editor: NoteEditor, key: InputKey) -> Vec<ReaderEffect> {
        if key == InputKey::Character('\u{03}') {
            self.mode = ReaderMode::Annotations(AnnotationMode::QuitNote(Box::new(editor)));
            return Vec::new();
        }
        match editor.state.handle(key) {
            Some(EditorAction::Cancel) => self.restore_annotation_return(editor.return_to, false),
            Some(EditorAction::Save) => {
                let command = match &editor.target {
                    NoteTarget::New(anchor) => AnnotationCommand::Create(AnnotationDraft {
                        anchor: anchor.clone(),
                        note: editor.state.text().into(),
                    }),
                    NoteTarget::Existing(annotation) => AnnotationCommand::EditNote(NoteUpdate {
                        annotation_id: annotation.id.clone(),
                        note: editor.state.text().into(),
                    }),
                };
                return self.submit_annotation(
                    command,
                    editor.return_to.clone(),
                    AnnotationMode::editing(editor),
                );
            }
            Some(EditorAction::Delete) => {
                if let NoteTarget::Existing(annotation) = editor.target {
                    self.mode = ReaderMode::Annotations(AnnotationMode::confirming(
                        AnnotationConfirmation {
                            message: "Delete this note? The highlight will remain.".into(),
                            action: ConfirmedAction::DeleteNote(*annotation),
                            return_to: editor.return_to,
                        },
                    ));
                }
            }
            None => self.mode = ReaderMode::Annotations(AnnotationMode::editing(editor)),
        }
        Vec::new()
    }

    fn annotation_list_key(&mut self, mut selected: usize, key: InputKey) -> Vec<ReaderEffect> {
        selected = selected.min(self.info.annotations.len().saturating_sub(1));
        match key {
            InputKey::Escape | InputKey::Character('q') => return Vec::new(),
            InputKey::Character('\u{03}') => return self.execute(Command::Quit, 1),
            InputKey::Up | InputKey::Character('k') => selected = selected.saturating_sub(1),
            InputKey::Down | InputKey::Character('j') => {
                selected = (selected + 1).min(self.info.annotations.len().saturating_sub(1))
            }
            InputKey::Enter => {
                if let Some(annotation) = self.info.annotations.get(selected).cloned()
                    && let Some(index) = self
                        .info
                        .contents
                        .iter()
                        .position(|content| *content == annotation.anchor.content_id())
                {
                    self.annotation_state.set_focused(Some(annotation.id));
                    return self.request_jump(LoadRequest {
                        index,
                        width: self.width,
                        position: Some(
                            ReadingState {
                                content_id: annotation.anchor.content_id().into(),
                                textwidth: self.width,
                                offset: annotation.anchor.start(),
                                screen_row: 0,
                            }
                            .into(),
                        ),
                        at_end: false,
                        fragment: None,
                    });
                }
            }
            InputKey::Character('r') => return self.begin_reassociation(selected),
            InputKey::Character('e' | 'd' | 'f') => {
                if let Some(annotation) = self.info.annotations.get(selected).cloned() {
                    let context = AnnotationReturn::List(selected);
                    match key {
                        InputKey::Character('e') => self
                            .open_note_editor(NoteTarget::Existing(Box::new(annotation)), context),
                        InputKey::Character('d') => self.apply_annotation_choice(
                            annotation,
                            AnnotationChoiceAction::Delete,
                            context,
                        ),
                        _ => {
                            self.mode =
                                ReaderMode::Annotations(AnnotationMode::viewing(NoteViewer {
                                    annotation,
                                    scroll: 0,
                                    return_to: context,
                                }))
                        }
                    }
                    return Vec::new();
                }
            }
            _ => {}
        }
        self.mode = ReaderMode::Annotations(AnnotationMode::List { selected });
        Vec::new()
    }

    fn chooser_key(&mut self, mut chooser: AnnotationChooser, key: InputKey) -> Vec<ReaderEffect> {
        if key == InputKey::Character('\u{03}') {
            return self.execute(Command::Quit, 1);
        }
        match key {
            InputKey::Escape => {
                self.restore_annotation_return(chooser.return_to, false);
                return Vec::new();
            }
            InputKey::Up | InputKey::Character('k') => {
                chooser.selected = chooser.selected.saturating_sub(1)
            }
            InputKey::Down | InputKey::Character('j') => {
                chooser.selected =
                    (chooser.selected + 1).min(chooser.records.len().saturating_sub(1))
            }
            InputKey::Enter => {
                if let Some(annotation) = chooser.records.get(chooser.selected).cloned() {
                    self.apply_annotation_choice(annotation, chooser.action, chooser.return_to);
                    return Vec::new();
                }
            }
            _ => {}
        }
        self.mode = ReaderMode::Annotations(AnnotationMode::choosing(chooser));
        Vec::new()
    }

    fn confirmation_key(
        &mut self,
        confirmation: AnnotationConfirmation,
        key: InputKey,
    ) -> Vec<ReaderEffect> {
        if key != InputKey::Character('y') && key != InputKey::Character('Y') {
            self.restore_annotation_return(confirmation.return_to, false);
            return Vec::new();
        }
        let command = match &confirmation.action {
            ConfirmedAction::DeleteHighlight(annotation) => {
                AnnotationCommand::Delete(annotation.id.clone())
            }
            ConfirmedAction::DeleteNote(annotation) => AnnotationCommand::EditNote(NoteUpdate {
                annotation_id: annotation.id.clone(),
                note: String::new(),
            }),
        };
        self.submit_annotation(
            command,
            confirmation.return_to.clone(),
            AnnotationMode::confirming(confirmation),
        )
    }

    fn viewer_key(&mut self, mut viewer: NoteViewer, key: InputKey) -> Vec<ReaderEffect> {
        if key == InputKey::Character('\u{03}') {
            return self.execute(Command::Quit, 1);
        }
        let maximum_scroll = crate::annotations::note_view_lines(
            viewer.annotation.anchor.quote(),
            &viewer.annotation.note,
            self.viewport.columns.saturating_sub(4),
        )
        .len()
        .saturating_sub(self.viewport.rows.saturating_sub(2));
        viewer.scroll = viewer.scroll.min(maximum_scroll);
        match key {
            InputKey::Escape | InputKey::Character('q') => {
                self.restore_annotation_return(viewer.return_to, false);
                return Vec::new();
            }
            InputKey::Up | InputKey::Character('k') => {
                viewer.scroll = viewer.scroll.saturating_sub(1)
            }
            InputKey::Down | InputKey::Character('j') => {
                viewer.scroll = viewer.scroll.saturating_add(1)
            }
            InputKey::PageDown | InputKey::Character('\u{04}') => {
                viewer.scroll = viewer.scroll.saturating_add(self.viewport.rows / 2)
            }
            InputKey::PageUp | InputKey::Character('\u{15}') => {
                viewer.scroll = viewer.scroll.saturating_sub(self.viewport.rows / 2)
            }
            _ => {}
        }
        viewer.scroll = viewer.scroll.min(maximum_scroll);
        self.mode = ReaderMode::Annotations(AnnotationMode::viewing(viewer));
        Vec::new()
    }

    fn submit_annotation(
        &mut self,
        command: AnnotationCommand,
        resume: AnnotationReturn,
        retry: AnnotationMode,
    ) -> Vec<ReaderEffect> {
        let pending = self.annotation_state.start(command, resume, retry);
        let job = pending.job.clone();
        self.pending_modal_overflow = false;
        self.mode = ReaderMode::Annotations(AnnotationMode::pending(pending));
        self.status = "Saving annotation…".into();
        vec![ReaderEffect::Annotation(job)]
    }

    fn pending_annotation_key(
        &mut self,
        mut pending: PendingAnnotation,
        key: InputKey,
    ) -> Vec<ReaderEffect> {
        match pending.handle_key(key) {
            super::pending::PendingAction::Resume => {
                self.pending_modal_overflow = false;
                self.mode = ReaderMode::Annotations(*pending.resume.retry);
                self.status = "Returned to the unsaved draft; buffered commands canceled.".into();
                return Vec::new();
            }
            super::pending::PendingAction::Retry => {
                let job = pending.job.clone();
                self.mode = ReaderMode::Annotations(AnnotationMode::pending(pending));
                self.status = "Retrying annotation save…".into();
                return vec![ReaderEffect::Annotation(job)];
            }
            super::pending::PendingAction::Overflow => self.report_modal_overflow(),
            super::pending::PendingAction::Buffered => {}
        }
        self.mode = ReaderMode::Annotations(AnnotationMode::pending(pending));
        Vec::new()
    }

    pub fn focused_annotation(&self) -> Option<&str> {
        self.annotation_state.focused()
    }

    pub fn annotation_completed(&mut self, result: AnnotationResult) -> Vec<ReaderEffect> {
        let previous = std::mem::replace(&mut self.mode, ReaderMode::Reading);
        let ReaderMode::Annotations(AnnotationMode::Pending(pending)) = previous else {
            self.mode = previous;
            return Vec::new();
        };
        let AnnotationResult {
            request_id,
            annotations,
            saved_id,
            error,
            export,
        } = result;
        let completion = super::pending::PendingCompletion {
            request_id,
            result: error.map_or(Ok(()), Err),
        };
        let transition = super::pending::PendingTransition {
            restore: |pending| ReaderMode::Annotations(AnnotationMode::pending(pending)),
            failure_message: |error| {
                format!("Save failed: {error}. Enter retries; Escape returns to the draft.")
            },
        };
        let display = super::ModeContext {
            mode: &mut self.mode,
            status: &mut self.status,
        };
        let Some(completed) = super::pending::complete(*pending, completion, transition, display)
        else {
            return Vec::new();
        };
        let pending = completed.operation;
        std::sync::Arc::make_mut(&mut self.info).annotations = annotations;
        self.annotation_state.set_focused(saved_id);
        self.export_updated(export);
        self.restore_annotation_return(pending.resume.return_to, true);
        self.status = if matches!(pending.job.command, AnnotationCommand::Delete(_)) {
            "Annotation deleted."
        } else {
            "Annotation saved."
        }
        .into();
        self.replay_modal_inputs(pending.inputs)
    }

    fn restore_annotation_return(&mut self, context: AnnotationReturn, clear_range: bool) {
        self.status.clear();
        self.mode = match context {
            AnnotationReturn::Reading => ReaderMode::Reading,
            AnnotationReturn::Selecting(mut selection) => {
                if clear_range {
                    selection.clear_range();
                }
                ReaderMode::Annotations(AnnotationMode::Selecting(selection))
            }
            AnnotationReturn::List(selected) => ReaderMode::Annotations(AnnotationMode::List {
                selected: selected.min(self.info.annotations.len().saturating_sub(1)),
            }),
        };
    }
}
