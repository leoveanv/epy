pub(super) mod inline;
pub(super) mod preparation;
pub(super) mod preview;
pub(super) mod support;
#[cfg(unix)]
mod tmux_graphics;

use super::render;
use preparation as image_preparation;
use preview as images;

use std::io::{Result, Write};

#[derive(Default)]
pub(super) struct CleanupSequences {
    sequences: Vec<String>,
}

impl CleanupSequences {
    pub(super) fn push(&mut self, sequence: String) {
        self.sequences.push(sequence);
    }

    pub(super) fn flush(&mut self, output: &mut impl Write) -> Result<()> {
        if self.sequences.is_empty() {
            return Ok(());
        }
        for sequence in self.sequences.drain(..) {
            output.write_all(sequence.as_bytes())?;
        }
        output.flush()
    }
}
