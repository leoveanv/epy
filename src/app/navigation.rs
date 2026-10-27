use super::{ReaderEffect, ReaderMode, ReaderModel};
use crate::configuration::InputKey;
use crate::reading::LoadRequest;
use crate::reading::ViewPosition;
use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub(super) struct JumpHistory {
    entries: Vec<ViewPosition>,
    cursor: Option<usize>,
}

fn same_position(left: &ViewPosition, right: &ViewPosition) -> bool {
    if left.image.is_some() || right.image.is_some() {
        return left.image == right.image;
    }
    left.text.content_id == right.text.content_id
        && left.text.offset == right.text.offset
        && left.text.screen_row == right.text.screen_row
}

impl JumpHistory {
    fn record(&mut self, origin: ViewPosition, destination: ViewPosition) {
        if same_position(&origin, &destination) {
            return;
        }
        self.entries
            .truncate(self.cursor.map_or(0, |cursor| cursor + 1));
        if self
            .entries
            .last()
            .is_none_or(|last| !same_position(last, &origin))
        {
            self.entries.push(origin);
        }
        self.entries.push(destination);
        if self.entries.len() > 100 {
            self.entries.drain(..self.entries.len() - 100);
        }
        self.cursor = Some(self.entries.len() - 1);
    }

    fn traverse(
        &mut self,
        current: ViewPosition,
        forward: bool,
        count: usize,
    ) -> Option<ViewPosition> {
        let cursor = self.cursor?;
        let target = if forward {
            cursor.saturating_add(count).min(self.entries.len() - 1)
        } else {
            cursor.saturating_sub(count)
        };
        if target == cursor {
            return None;
        }
        self.entries[cursor] = current;
        self.cursor = Some(target);
        Some(self.entries[target].clone())
    }
}

#[derive(Clone)]
pub(super) enum HistoryChange {
    Jump(ViewPosition),
    Traverse(JumpHistory),
}

#[derive(Clone, Default)]
pub(super) struct NavigationState {
    history: JumpHistory,
    quickmarks: BTreeMap<char, ViewPosition>,
    pending: Option<HistoryChange>,
}

impl ReaderModel {
    pub(super) fn request_jump(&mut self, request: LoadRequest) -> Vec<ReaderEffect> {
        self.begin_jump();
        self.request_load(request)
    }

    pub(super) fn begin_jump(&mut self) {
        self.navigation.pending = Some(HistoryChange::Jump(self.view_position()));
    }

    pub(super) fn clear_pending_jump(&mut self) {
        self.navigation.pending = None;
    }

    fn jump_position(&mut self, mut position: ViewPosition) -> Vec<ReaderEffect> {
        let Some(index) = self
            .info
            .contents
            .iter()
            .position(|id| id == position.content_id())
        else {
            self.status = "The marked document is not in this book.".into();
            return Vec::new();
        };
        position.text.textwidth = self.width;
        self.request_load(LoadRequest {
            index,
            width: self.width,
            position: Some(position),
            at_end: false,
            fragment: None,
        })
    }

    pub(super) fn history_jump(&mut self, forward: bool, count: usize) -> Vec<ReaderEffect> {
        let mut history = self.navigation.history.clone();
        let Some(position) = history.traverse(self.view_position(), forward, count) else {
            self.status = if forward {
                "No later jump."
            } else {
                "No earlier jump."
            }
            .into();
            return Vec::new();
        };
        self.navigation.pending = Some(HistoryChange::Traverse(history));
        self.jump_position(position)
    }

    pub(super) fn record_jump(&mut self, origin: ViewPosition) {
        let destination = self.view_position();
        self.navigation.history.record(origin, destination);
    }

    pub(super) fn history_arrived(&mut self) {
        let position = self.view_position();
        match self.navigation.pending.take() {
            Some(HistoryChange::Jump(origin)) => self.navigation.history.record(origin, position),
            Some(HistoryChange::Traverse(history)) => self.navigation.history = history,
            None => {}
        }
    }

    pub(super) fn quickmark_key(&mut self, saving: bool, key: InputKey) -> Vec<ReaderEffect> {
        self.mode = ReaderMode::Reading;
        self.status.clear();
        let InputKey::Character(mark) = key else {
            return Vec::new();
        };
        if !mark.is_ascii_digit() {
            return Vec::new();
        }
        if saving {
            self.navigation
                .quickmarks
                .insert(mark, self.view_position());
            self.status = format!("Quickmark {mark} saved for this session.");
            return Vec::new();
        }
        let Some(position) = self.navigation.quickmarks.get(&mark).cloned() else {
            self.status = format!("Quickmark {mark} is not set.");
            return Vec::new();
        };
        self.navigation.pending = Some(HistoryChange::Jump(self.view_position()));
        self.jump_position(position)
    }

    pub fn load_failed(&mut self, message: String) -> Vec<ReaderEffect> {
        self.cancel_page_turn();
        self.navigation.pending = None;
        self.loading = false;
        self.loading_request = None;
        self.pending_shift = 0;
        self.pending_hyperlink = None;
        self.prefix.clear();
        self.width = self.rendered.width;
        self.pending.clear();
        let ignored = std::mem::take(&mut self.pending_input_overflow);
        self.deferred_modal_input.clear();
        self.status = message;
        if ignored {
            self.append_input_warning();
        }
        if self.quit_after_load {
            self.quit_after_load = false;
            return self.execute(crate::configuration::Command::Quit, 1);
        }
        if let Some(result) = self.search_state.take_deferred_result() {
            let status = std::mem::take(&mut self.status);
            let effects = self.search_completed(result);
            self.status = status;
            return effects;
        }
        Vec::new()
    }
}
