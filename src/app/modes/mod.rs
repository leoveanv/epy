pub mod annotations;
pub mod dictionary;
pub mod hyperlinks;
pub mod images;
pub mod library;
pub mod markdown_export;
mod pending;
pub mod reassociation;
pub mod search;

use super::{ModalInput, QueuedInput, ReaderEffect, ReaderMode, ReaderModel, input};
use crate::configuration::{Command, InputKey};

pub(super) struct ModeContext<'a> {
    pub(super) mode: &'a mut ReaderMode,
    pub(super) status: &'a mut String,
}

pub(super) struct PendingModeContext<'a> {
    pub(super) mode: &'a mut ReaderMode,
    pub(super) status: &'a mut String,
    pub(super) overflow: &'a mut bool,
}

impl PendingModeContext<'_> {
    fn report_overflow(&mut self) {
        report_modal_overflow(self.overflow, self.status);
    }
}

pub(super) fn report_modal_overflow(overflow: &mut bool, status: &mut String) {
    *overflow = true;
    *status = "Input queue full. New input was ignored.".into();
}

pub(super) enum ModalKeyOutcome {
    Effects(Vec<ReaderEffect>),
    Quit,
}

#[derive(Clone, Copy)]
pub(super) struct ModeInput {
    pub(super) key: InputKey,
    pub(super) command: Option<Command>,
}

#[derive(Default)]
struct RequestSequence(u64);

impl RequestSequence {
    fn next(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }

    fn next_saturating(&mut self) -> u64 {
        self.0 = self.0.saturating_add(1);
        self.0
    }

    fn next_wrapping(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(1);
        self.0
    }
}
