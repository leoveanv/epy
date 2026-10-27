use super::{ReaderEffect, ReaderMode, ReaderModel};
use crate::app::editor::{EditorAction, EditorOptions, EditorState};
use crate::configuration::{Command, InputKey};
use crate::search::{SearchDirection, SearchJob, SearchLimits, SearchResult, SearchSelection};
use std::collections::VecDeque;

#[derive(Default)]
pub(in crate::app) struct SearchState {
    requests: super::RequestSequence,
    deferred_result: Option<SearchResult>,
}

impl SearchState {
    pub(in crate::app) fn take_deferred_result(&mut self) -> Option<SearchResult> {
        self.deferred_result.take()
    }
}

#[derive(Clone, Debug)]
pub struct SearchSession {
    pub pattern: String,
    pub selection: Option<SearchSelection>,
    pub pending: Option<u64>,
    pub finished: bool,
    request_width: usize,
    prefix: String,
    queued: VecDeque<InputKey>,
    ignored_input: bool,
}

impl SearchSession {
    fn new(pattern: String, width: usize) -> Self {
        Self {
            pattern,
            selection: None,
            pending: None,
            finished: false,
            request_width: width,
            prefix: String::new(),
            queued: VecDeque::new(),
            ignored_input: false,
        }
    }

    fn queue_input(&mut self, key: InputKey) -> bool {
        if self.queued.len() >= super::input::PENDING_INPUTS_MAX {
            self.ignored_input = true;
            return false;
        }
        self.queued.push_back(key);
        true
    }

    fn push_digit(&mut self, digit: char) {
        if self.prefix.len() < 7 {
            self.prefix.push(digit);
        }
    }

    fn take_count(&mut self) -> usize {
        let count = self.prefix.parse::<usize>().unwrap_or(1).max(1);
        self.prefix.clear();
        count
    }

    fn discard_queued_input(&mut self) -> bool {
        let ignored = self.ignored_input || !self.queued.is_empty();
        self.ignored_input = false;
        self.queued.clear();
        if ignored {
            self.prefix.clear();
        }
        ignored
    }
}

#[derive(Clone, Debug)]
pub enum SearchMode {
    Editing(EditorState),
    Results(SearchSession),
}

fn editor(pattern: &str) -> EditorState {
    EditorState::new(
        pattern,
        EditorOptions {
            multiline: false,
            can_delete: false,
        },
    )
}

impl ReaderModel {
    pub(in crate::app) fn begin_search(&mut self) -> Vec<ReaderEffect> {
        self.mode = ReaderMode::Search(Box::new(SearchMode::Editing(editor(""))));
        self.status = "Regex | Enter search | Escape cancel".into();
        Vec::new()
    }

    pub fn search_selection(&self) -> Option<&SearchSelection> {
        match &self.mode {
            ReaderMode::Search(mode) => match mode.as_ref() {
                SearchMode::Results(session) => session.selection.as_ref(),
                _ => None,
            },
            _ => None,
        }
    }

    fn submit_search(
        &mut self,
        mut session: SearchSession,
        direction: SearchDirection,
        count: usize,
        restarting: bool,
    ) -> Vec<ReaderEffect> {
        let request_id = self.search_state.requests.next_saturating();
        let previous = session
            .selection
            .as_ref()
            .and_then(|selection| selection.matches.hits.get(selection.selected));
        let start_row = if restarting {
            previous.map_or(self.row, |hit| {
                self.rendered.layout.row_for_offset(hit.offset)
            })
        } else {
            self.row
        };
        let job = SearchJob {
            request_id,
            pattern: session.pattern.clone(),
            contents: self.info.contents.clone(),
            current: self.rendered.clone(),
            start_row,
            start_offset: if restarting {
                previous.map(|hit| hit.offset)
            } else {
                None
            },
            after_hit: if restarting {
                None
            } else {
                session
                    .selection
                    .as_ref()
                    .map(|selection| selection.selected)
            },
            direction,
            count,
        };
        session.pending = Some(job.request_id);
        session.request_width = self.width;
        session.finished = false;
        self.mode = ReaderMode::Search(Box::new(SearchMode::Results(session)));
        self.status = format!("Searching: {} | Escape cancels", job.pattern);
        vec![ReaderEffect::Search(job)]
    }

    fn search_edit_key(&mut self, mut state: EditorState, key: InputKey) -> Vec<ReaderEffect> {
        if matches!(key, InputKey::Escape | InputKey::Character('\u{03}')) {
            self.status.clear();
            return vec![ReaderEffect::CancelSearch];
        }
        if key == InputKey::Character('\u{15}') {
            state = editor("");
        } else {
            if let InputKey::Character(character) = key
                && !character.is_control()
                && state.focus() == 0
                && state.text().len().saturating_add(character.len_utf8())
                    > SearchLimits::default().pattern_bytes_max
            {
                self.status = "The regex is too long. Shorten it or use Ctrl-U to clear.".into();
                self.mode = ReaderMode::Search(Box::new(SearchMode::Editing(state)));
                return Vec::new();
            }
            let action = if key == InputKey::Enter && state.focus() == 0 {
                Some(EditorAction::Save)
            } else {
                state.handle(key)
            };
            match action {
                Some(EditorAction::Save) => {
                    let session = SearchSession::new(state.text().into(), self.width);
                    return self.submit_search(session, SearchDirection::Forward, 1, false);
                }
                Some(EditorAction::Cancel) => {
                    self.status.clear();
                    return vec![ReaderEffect::CancelSearch];
                }
                _ => {}
            }
        }
        self.mode = ReaderMode::Search(Box::new(SearchMode::Editing(state)));
        Vec::new()
    }

    pub(in crate::app) fn handle_search_paste(&mut self, text: &str) {
        if let ReaderMode::Search(mode) = &mut self.mode
            && let SearchMode::Editing(state) = mode.as_mut()
        {
            if text.chars().any(char::is_control)
                || state.text().len().saturating_add(text.len())
                    > SearchLimits::default().pattern_bytes_max
            {
                self.status =
                    "Paste a single regex without control characters, at most 4096 bytes.".into();
            } else {
                state.insert(text);
            }
        }
    }

    pub(in crate::app) fn handle_search_key(&mut self, key: InputKey) -> Vec<ReaderEffect> {
        if self.loading {
            if matches!(key, InputKey::Escape | InputKey::Character('\u{03}')) {
                self.deferred_modal_input.clear();
            } else {
                self.defer_modal_input(super::ModalInput::Key(key));
                return Vec::new();
            }
        }
        let ReaderMode::Search(mode) = std::mem::replace(&mut self.mode, ReaderMode::Reading)
        else {
            return Vec::new();
        };
        let mut session = match *mode {
            SearchMode::Editing(state) => return self.search_edit_key(state, key),
            SearchMode::Results(session) => session,
        };
        if self.configuration.bindings.command_for(key) == Some(Command::Quit) {
            self.search_state.deferred_result = None;
            self.status.clear();
            return vec![ReaderEffect::CancelSearch];
        }
        if key == InputKey::Character('/') {
            self.mode = ReaderMode::Search(Box::new(SearchMode::Editing(editor(&session.pattern))));
            return vec![ReaderEffect::CancelSearch];
        }
        if session.pending.is_some() {
            if !session.queue_input(key) {
                self.status = "Input queue full. New input was ignored.".into();
            }
            self.mode = ReaderMode::Search(Box::new(SearchMode::Results(session)));
            return Vec::new();
        }
        if let InputKey::Character(character) = key
            && character.is_ascii_digit()
        {
            session.push_digit(character);
        } else if matches!(key, InputKey::Character('n' | 'N')) {
            let count = session.take_count();
            let direction = if key == InputKey::Character('n') {
                SearchDirection::Forward
            } else {
                SearchDirection::Backward
            };
            return self.submit_search(session, direction, count, false);
        }
        self.mode = ReaderMode::Search(Box::new(SearchMode::Results(session)));
        Vec::new()
    }

    pub fn search_completed(&mut self, result: SearchResult) -> Vec<ReaderEffect> {
        if !matches!(self.mode, ReaderMode::Search(_)) {
            return Vec::new();
        }
        if self.loading {
            self.search_state.deferred_result = Some(result);
            return Vec::new();
        }
        let ReaderMode::Search(mode) = std::mem::replace(&mut self.mode, ReaderMode::Reading)
        else {
            return Vec::new();
        };
        let SearchMode::Results(mut session) = *mode else {
            self.mode = ReaderMode::Search(mode);
            return Vec::new();
        };
        if session.pending != Some(result.request_id) {
            self.mode = ReaderMode::Search(Box::new(SearchMode::Results(session)));
            return Vec::new();
        }
        session.pending = None;
        match result.result {
            Ok(Some(selection)) if selection.document.width != self.width => {
                return self.submit_search(session, SearchDirection::Forward, 1, true);
            }
            Ok(Some(selection)) => {
                self.show_search_selection(&selection);
                session.finished = selection.exhausted;
                session.selection = Some(selection);
            }
            Ok(None) => session.finished = true,
            Err(error) => {
                let ignored = session.discard_queued_input();
                self.status = error;
                self.mode = ReaderMode::Search(Box::new(if session.selection.is_none() {
                    SearchMode::Editing(editor(&session.pattern))
                } else {
                    SearchMode::Results(session)
                }));
                if ignored {
                    self.append_input_warning();
                }
                return Vec::new();
            }
        }
        self.status = if session.finished {
            format!(
                "Finished searching: {} | n/N reverse direction | Escape close",
                session.pattern
            )
        } else {
            String::new()
        };
        let mut queued = std::mem::take(&mut session.queued);
        self.mode = ReaderMode::Search(Box::new(SearchMode::Results(session)));
        let mut effects = Vec::new();
        while let Some(key) = queued.pop_front() {
            effects.extend(self.handle_search_key(key));
            if let ReaderMode::Search(mode) = &mut self.mode
                && let SearchMode::Results(session) = mode.as_mut()
                && session.pending.is_some()
            {
                session.queued.extend(queued);
                break;
            }
        }
        let ignored = if let ReaderMode::Search(mode) = &mut self.mode
            && let SearchMode::Results(session) = mode.as_mut()
            && session.ignored_input
        {
            if session.pending.is_none() {
                session.ignored_input = false;
                session.prefix.clear();
            }
            true
        } else {
            false
        };
        if effects.is_empty() {
            effects.extend(self.ensure_continuous_view());
        }
        if ignored {
            self.pending_input_overflow |= self.loading;
            self.append_input_warning();
        }
        effects
    }

    fn show_search_selection(&mut self, selection: &SearchSelection) {
        let Some(hit) = selection.matches.hits.get(selection.selected) else {
            return;
        };
        let origin = self.view_position();
        let same_document = self.rendered.index == selection.document.index;
        self.replace_document(selection.document.clone());
        let base = if same_document { self.row } else { 0 };
        let page = (self.reading_height() * self.spread).max(1);
        self.row = if hit.row < base {
            base.saturating_sub((base - hit.row).div_ceil(page) * page)
        } else {
            base + (hit.row - base) / page * page
        }
        .min(self.last_row());
        self.record_jump(origin);
    }

    pub(in crate::app) fn search_reflow(&mut self) -> Vec<ReaderEffect> {
        let restart = matches!(&self.mode, ReaderMode::Search(mode) if matches!(mode.as_ref(), SearchMode::Results(session) if session.request_width != self.width));
        if restart {
            let ReaderMode::Search(mode) = std::mem::replace(&mut self.mode, ReaderMode::Reading)
            else {
                return Vec::new();
            };
            if let SearchMode::Results(session) = *mode {
                return self.submit_search(session, SearchDirection::Forward, 1, true);
            }
        }
        Vec::new()
    }
}
