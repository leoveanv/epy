use std::collections::VecDeque;

use super::{ModalInput, ModeContext, ReaderMode, input};
use crate::annotations::{AnnotationJob, LinkJob, ReassociationJob};
use crate::configuration::InputKey;

pub(super) trait PendingJob {
    fn request_id(&self) -> u64;
}

impl PendingJob for AnnotationJob {
    fn request_id(&self) -> u64 {
        self.request_id
    }
}

impl PendingJob for LinkJob {
    fn request_id(&self) -> u64 {
        self.request_id
    }
}

impl PendingJob for ReassociationJob {
    fn request_id(&self) -> u64 {
        self.request_id
    }
}

#[derive(Clone, Debug)]
pub struct PendingOperation<J, S> {
    pub job: J,
    pub resume: S,
    pub inputs: VecDeque<ModalInput>,
    pub error: Option<String>,
}

impl<J, S> PendingOperation<J, S> {
    pub(super) fn new(job: J, resume: S) -> Self {
        Self {
            job,
            resume,
            inputs: VecDeque::new(),
            error: None,
        }
    }

    pub(super) fn handle_key(&mut self, key: InputKey) -> PendingAction {
        if self.error.is_some() {
            match key {
                InputKey::Escape => return PendingAction::Resume,
                InputKey::Enter => {
                    self.error = None;
                    return PendingAction::Retry;
                }
                _ => {}
            }
        }
        if input::push_modal_key(&mut self.inputs, key) {
            PendingAction::Buffered
        } else {
            PendingAction::Overflow
        }
    }

    pub(super) fn push_paste(&mut self, text: &str) -> bool {
        input::push_modal_paste(&mut self.inputs, text)
    }
}

pub(super) enum PendingAction {
    Resume,
    Retry,
    Buffered,
    Overflow,
}

pub(super) struct PendingCompletion<T> {
    pub request_id: u64,
    pub result: Result<T, String>,
}

pub(super) struct PendingTransition<J, S> {
    pub restore: fn(PendingOperation<J, S>) -> ReaderMode,
    pub failure_message: fn(&str) -> String,
}

pub(super) struct PendingSuccess<J, S, T> {
    pub operation: PendingOperation<J, S>,
    pub reply: T,
}

pub(super) fn complete<J: PendingJob, S, T>(
    mut pending: PendingOperation<J, S>,
    completion: PendingCompletion<T>,
    transition: PendingTransition<J, S>,
    display: ModeContext<'_>,
) -> Option<PendingSuccess<J, S, T>> {
    if pending.job.request_id() != completion.request_id {
        *display.mode = (transition.restore)(pending);
        return None;
    }
    match completion.result {
        Ok(reply) => Some(PendingSuccess {
            operation: pending,
            reply,
        }),
        Err(error) => {
            *display.status = (transition.failure_message)(&error);
            pending.error = Some(error);
            *display.mode = (transition.restore)(pending);
            None
        }
    }
}
