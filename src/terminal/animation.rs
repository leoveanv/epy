use std::time::Instant;

use crate::reading::PageDirection;
use crate::terminal::page_animation::{PageAnimation, PagePane};
use ratatui::buffer::Buffer;

pub(super) struct RunningAnimation {
    page: PageAnimation,
    started: Instant,
}

impl RunningAnimation {
    pub fn new(direction: PageDirection, width: u16, now: Instant) -> Self {
        Self {
            page: PageAnimation::new(direction, width),
            started: now,
        }
    }

    pub fn is_finished(&self, now: Instant) -> bool {
        self.page
            .is_finished(now.saturating_duration_since(self.started))
    }

    pub fn paint(&self, buffer: &mut Buffer, panes: &[PagePane], now: Instant) {
        self.page
            .paint(buffer, panes, now.saturating_duration_since(self.started));
    }
}
