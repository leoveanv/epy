pub mod editor;

use std::collections::VecDeque;

use super::QueuedInput;
use crate::configuration::InputKey;

pub(super) const PENDING_INPUTS_MAX: usize = 4096;
const PENDING_PASTE_BYTES_MAX: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub enum ModalInput {
    Key(InputKey),
    Paste(String),
}

fn paste_bytes(input: &ModalInput) -> usize {
    match input {
        ModalInput::Paste(text) => text.len(),
        ModalInput::Key(_) => 0,
    }
}

fn fits_paste(current: usize, additional: usize) -> bool {
    additional <= PENDING_PASTE_BYTES_MAX.saturating_sub(current)
}

fn modal_input_fits(inputs: &VecDeque<ModalInput>, paste_length: usize) -> bool {
    if inputs.len() >= PENDING_INPUTS_MAX {
        return false;
    }
    if paste_length == 0 {
        return true;
    }
    let queued_bytes = inputs
        .iter()
        .map(paste_bytes)
        .fold(0, usize::saturating_add);
    fits_paste(queued_bytes, paste_length)
}

pub(super) fn push_modal_key(inputs: &mut VecDeque<ModalInput>, key: InputKey) -> bool {
    if !modal_input_fits(inputs, 0) {
        return false;
    }
    inputs.push_back(ModalInput::Key(key));
    true
}

pub(super) fn push_modal_paste(inputs: &mut VecDeque<ModalInput>, text: &str) -> bool {
    if !modal_input_fits(inputs, text.len()) {
        return false;
    }
    inputs.push_back(ModalInput::Paste(text.into()));
    true
}

#[derive(Default)]
pub(super) struct DeferredInputs {
    entries: VecDeque<ModalInput>,
    paste_bytes: usize,
}

impl DeferredInputs {
    pub(super) fn push(&mut self, input: ModalInput) -> bool {
        let bytes = paste_bytes(&input);
        if self.entries.len() >= PENDING_INPUTS_MAX || !fits_paste(self.paste_bytes, bytes) {
            return false;
        }
        self.paste_bytes += bytes;
        self.entries.push_back(input);
        true
    }

    pub(super) fn pop_front(&mut self) -> Option<ModalInput> {
        let input = self.entries.pop_front()?;
        self.paste_bytes -= paste_bytes(&input);
        Some(input)
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Default)]
pub(super) struct PendingInputs {
    entries: VecDeque<QueuedInput>,
    count: usize,
    mode_sensitive: usize,
    paste_bytes: usize,
}

impl PendingInputs {
    pub(super) fn has_mode_sensitive_input(&self) -> bool {
        self.mode_sensitive > 0
    }

    pub(super) fn can_fit(&self, count: usize) -> bool {
        count <= PENDING_INPUTS_MAX.saturating_sub(self.count)
    }

    pub(super) fn push(&mut self, input: QueuedInput) -> bool {
        let paste_bytes = match &input {
            QueuedInput::Input(input) => paste_bytes(input),
            _ => 0,
        };
        if !self.can_fit(1) || !fits_paste(self.paste_bytes, paste_bytes) {
            return false;
        }
        self.count += 1;
        self.mode_sensitive += usize::from(matches!(
            input,
            QueuedInput::Input(_) | QueuedInput::Mouse { .. }
        ));
        self.paste_bytes += paste_bytes;
        match (self.entries.back_mut(), &input) {
            (
                Some(QueuedInput::Command {
                    command: previous,
                    count: total,
                    repetitions,
                }),
                QueuedInput::Command { command, count, .. },
            ) if previous == command => {
                *total = total.saturating_add(*count);
                *repetitions += 1;
            }
            (
                Some(QueuedInput::Mouse {
                    action: previous,
                    context: previous_context,
                    repetitions,
                }),
                QueuedInput::Mouse {
                    action, context, ..
                },
            ) if previous == action && previous_context == context => *repetitions += 1,
            _ => self.entries.push_back(input),
        }
        true
    }

    pub(super) fn pop_front(&mut self) -> Option<QueuedInput> {
        let input = self.entries.pop_front()?;
        match input {
            QueuedInput::Command {
                command,
                count,
                repetitions,
            } => {
                self.count -= repetitions;
                Some(QueuedInput::Command {
                    command,
                    count,
                    repetitions,
                })
            }
            QueuedInput::Mouse {
                action,
                context,
                repetitions,
            } => {
                self.count -= 1;
                self.mode_sensitive -= 1;
                if repetitions > 1 {
                    self.entries.push_front(QueuedInput::Mouse {
                        action,
                        context,
                        repetitions: repetitions - 1,
                    });
                }
                Some(QueuedInput::Mouse {
                    action,
                    context,
                    repetitions: 1,
                })
            }
            QueuedInput::Input(input) => {
                self.count -= 1;
                self.mode_sensitive -= 1;
                self.paste_bytes -= paste_bytes(&input);
                Some(QueuedInput::Input(input))
            }
        }
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}
