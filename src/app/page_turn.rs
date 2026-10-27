use super::{ReaderEffect, ReaderMode, ReaderModel};
use crate::reading::PageDirection;
use crate::reading::ReadingState;

#[derive(Default)]
pub(super) struct PageTurns {
    ready: Option<PageDirection>,
    pending: Option<PendingTurn>,
}

struct PendingTurn {
    direction: PageDirection,
    origin: ReadingState,
}

impl ReaderModel {
    pub fn take_page_turn(&mut self) -> Option<PageDirection> {
        self.page_turns.ready.take()
    }

    pub fn cancel_page_turn(&mut self) {
        self.page_turns = PageTurns::default();
    }

    pub(super) fn finish_page_turn(&mut self) {
        if let Some(turn) = self.page_turns.pending.take()
            && turn.origin != self.position()
        {
            self.page_turns.ready = Some(turn.direction);
        }
    }

    pub(super) fn scroll_rows(&mut self, delta: isize) -> Vec<ReaderEffect> {
        if self.spread == 2 {
            let delta =
                delta.saturating_mul(self.viewport.rows.saturating_mul(self.spread) as isize);
            return self.page_rows(delta);
        }
        if self.configuration.settings.between_chapters {
            return self.move_rows(delta);
        }
        let crosses = (delta > 0 && self.row >= self.last_row()) || (delta < 0 && self.row == 0);
        if crosses {
            self.page_rows(delta)
        } else {
            self.move_rows(delta)
        }
    }

    pub(super) fn page_rows(&mut self, delta: isize) -> Vec<ReaderEffect> {
        let origin = self.position();
        let effects = self.move_rows(delta);
        if self.configuration.settings.page_scroll_animation
            && matches!(self.mode, ReaderMode::Reading)
        {
            let direction = if delta < 0 {
                PageDirection::Backward
            } else {
                PageDirection::Forward
            };
            if self.loading {
                self.page_turns.pending = Some(PendingTurn { direction, origin });
            } else if origin != self.position() {
                self.page_turns.ready = Some(direction);
            }
        }
        effects
    }
}
