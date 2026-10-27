mod slide;

use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::reading::PageDirection;

pub struct PagePane {
    pub area: Rect,
    pub style: Style,
}

pub struct PageAnimation {
    direction: PageDirection,
    duration: Duration,
}

impl PageAnimation {
    pub fn new(direction: PageDirection, width: u16) -> Self {
        // #NOTE: The old slide used one millisecond per column; the cap leaves room for frame scheduling within the 100 ms settling target.
        Self {
            direction,
            duration: Duration::from_millis(u64::from(width.min(64))),
        }
    }

    pub fn is_finished(&self, elapsed: Duration) -> bool {
        elapsed >= self.duration
    }

    pub fn paint(&self, buffer: &mut Buffer, panes: &[PagePane], elapsed: Duration) {
        if self.is_finished(elapsed) {
            return;
        }
        for pane in panes {
            let area = pane.area.intersection(buffer.area);
            let exposed =
                (u128::from(area.width) * elapsed.as_nanos() / self.duration.as_nanos()) as u16;
            slide::paint_pane(
                buffer,
                slide::PaneSlide {
                    area,
                    style: pane.style,
                    exposed,
                    direction: self.direction,
                },
            );
        }
    }
}
