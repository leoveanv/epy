use std::collections::VecDeque;

use super::annotations::AnnotationMode;
use super::{
    ModalInput, ModalKeyOutcome, ModeContext, PendingModeContext, ReaderEffect, ReaderMode,
};
use crate::annotations::{
    ReassociationCommand, ReassociationJob, ReassociationReply, ReassociationReport,
    ReassociationResult, ReassociationSource,
};
use crate::configuration::InputKey;

#[derive(Clone, Debug)]
pub struct ReassociationPicker {
    pub sources: Vec<ReassociationSource>,
    pub selected: usize,
    pub return_index: usize,
}

pub type PendingReassociation =
    super::pending::PendingOperation<ReassociationJob, Box<ReassociationMode>>;

#[derive(Default)]
pub(in crate::app) struct ReassociationState {
    requests: super::RequestSequence,
}

pub(in crate::app) struct ReassociationCompletion {
    pub inputs: VecDeque<ModalInput>,
    pub copied: Option<ReassociationCopy>,
}

pub(in crate::app) struct ReassociationCopy {
    pub report: ReassociationReport,
    pub status: String,
}

impl ReassociationState {
    pub(in crate::app) fn begin(
        &mut self,
        context: PendingModeContext<'_>,
        return_index: usize,
    ) -> Vec<ReaderEffect> {
        self.submit(
            context,
            ReassociationCommand::Sources,
            ReassociationMode::Sources(ReassociationPicker {
                sources: Vec::new(),
                selected: 0,
                return_index,
            }),
        )
    }

    fn submit(
        &mut self,
        context: PendingModeContext<'_>,
        command: ReassociationCommand,
        resume: ReassociationMode,
    ) -> Vec<ReaderEffect> {
        let job = ReassociationJob {
            request_id: self.requests.next_saturating(),
            command,
        };
        let pending = PendingReassociation::new(job.clone(), Box::new(resume));
        *context.overflow = false;
        *context.mode =
            ReaderMode::Reassociation(Box::new(ReassociationMode::Pending(Box::new(pending))));
        *context.status = "Checking annotation identities…".into();
        vec![ReaderEffect::Reassociation(job)]
    }

    pub(in crate::app) fn paste(mut context: PendingModeContext<'_>, text: &str) {
        if let ReaderMode::Reassociation(mode) = &mut *context.mode
            && let ReassociationMode::Pending(pending) = mode.as_mut()
            && !pending.push_paste(text)
        {
            context.report_overflow();
        }
    }

    pub(in crate::app) fn key(
        &mut self,
        context: PendingModeContext<'_>,
        key: InputKey,
        annotation_count: usize,
    ) -> ModalKeyOutcome {
        let ReaderMode::Reassociation(mode) = std::mem::replace(context.mode, ReaderMode::Reading)
        else {
            return ModalKeyOutcome::Effects(Vec::new());
        };
        context.status.clear();
        match *mode {
            ReassociationMode::Pending(pending) => {
                ModalKeyOutcome::Effects(Self::pending_key(context, *pending, key))
            }
            ReassociationMode::Sources(picker) => {
                Self::sources_key(context, picker, key, annotation_count)
            }
            ReassociationMode::Confirm(picker) => self.confirm_key(context, picker, key),
        }
    }

    fn pending_key(
        mut context: PendingModeContext<'_>,
        mut pending: PendingReassociation,
        key: InputKey,
    ) -> Vec<ReaderEffect> {
        match pending.handle_key(key) {
            super::pending::PendingAction::Resume => {
                *context.overflow = false;
                *context.mode = ReaderMode::Reassociation(pending.resume);
                return Vec::new();
            }
            super::pending::PendingAction::Retry => {
                let job = pending.job.clone();
                *context.mode = ReaderMode::Reassociation(Box::new(ReassociationMode::Pending(
                    Box::new(pending),
                )));
                return vec![ReaderEffect::Reassociation(job)];
            }
            super::pending::PendingAction::Overflow => context.report_overflow(),
            super::pending::PendingAction::Buffered => {}
        }
        *context.mode =
            ReaderMode::Reassociation(Box::new(ReassociationMode::Pending(Box::new(pending))));
        Vec::new()
    }

    fn sources_key(
        context: PendingModeContext<'_>,
        mut picker: ReassociationPicker,
        key: InputKey,
        annotation_count: usize,
    ) -> ModalKeyOutcome {
        match key {
            InputKey::Character('\u{03}') => return ModalKeyOutcome::Quit,
            InputKey::Escape | InputKey::Character('q') => {
                Self::return_to_annotations(context.mode, picker.return_index, annotation_count);
                return ModalKeyOutcome::Effects(Vec::new());
            }
            InputKey::Down | InputKey::Character('j') => {
                picker.selected = (picker.selected + 1).min(picker.sources.len().saturating_sub(1))
            }
            InputKey::Up | InputKey::Character('k') => {
                picker.selected = picker.selected.saturating_sub(1)
            }
            InputKey::Enter if picker.sources.get(picker.selected).is_some() => {
                *context.mode =
                    ReaderMode::Reassociation(Box::new(ReassociationMode::Confirm(picker)));
                return ModalKeyOutcome::Effects(Vec::new());
            }
            _ => {}
        }
        *context.mode = ReaderMode::Reassociation(Box::new(ReassociationMode::Sources(picker)));
        ModalKeyOutcome::Effects(Vec::new())
    }

    fn confirm_key(
        &mut self,
        context: PendingModeContext<'_>,
        picker: ReassociationPicker,
        key: InputKey,
    ) -> ModalKeyOutcome {
        if key == InputKey::Character('\u{03}') {
            return ModalKeyOutcome::Quit;
        }
        if matches!(key, InputKey::Character('y' | 'Y'))
            && let Some(source) = picker.sources.get(picker.selected)
        {
            return ModalKeyOutcome::Effects(self.submit(
                context,
                ReassociationCommand::Copy(source.book.id.clone()),
                ReassociationMode::Confirm(picker),
            ));
        }
        *context.mode = ReaderMode::Reassociation(Box::new(ReassociationMode::Sources(picker)));
        ModalKeyOutcome::Effects(Vec::new())
    }

    pub(in crate::app) fn completed(
        context: ModeContext<'_>,
        result: ReassociationResult,
        annotation_count: usize,
    ) -> Option<ReassociationCompletion> {
        let previous = std::mem::replace(context.mode, ReaderMode::Reading);
        let ReaderMode::Reassociation(mode) = previous else {
            *context.mode = previous;
            return None;
        };
        let ReassociationMode::Pending(pending) = *mode else {
            *context.mode = ReaderMode::Reassociation(mode);
            return None;
        };
        let completion = super::pending::PendingCompletion {
            request_id: result.request_id,
            result: result.reply,
        };
        let transition = super::pending::PendingTransition {
            restore: |pending| {
                ReaderMode::Reassociation(Box::new(ReassociationMode::Pending(Box::new(pending))))
            },
            failure_message: |error| {
                format!("Reassociation failed: {error}. Enter retries; Esc returns.")
            },
        };
        let completed = super::pending::complete(
            *pending,
            completion,
            transition,
            ModeContext {
                mode: &mut *context.mode,
                status: &mut *context.status,
            },
        )?;
        let pending = completed.operation;
        let return_index = match pending.resume.as_ref() {
            ReassociationMode::Sources(picker) | ReassociationMode::Confirm(picker) => {
                picker.return_index
            }
            ReassociationMode::Pending(_) => 0,
        };
        context.status.clear();
        match completed.reply {
            ReassociationReply::Sources(sources) => {
                if sources.is_empty() {
                    Self::return_to_annotations(context.mode, return_index, annotation_count);
                    *context.status = "No other annotated book identities are available.".into();
                } else {
                    *context.mode = ReaderMode::Reassociation(Box::new(
                        ReassociationMode::Sources(ReassociationPicker {
                            sources,
                            selected: 0,
                            return_index,
                        }),
                    ));
                }
                Some(ReassociationCompletion {
                    inputs: pending.inputs,
                    copied: None,
                })
            }
            ReassociationReply::Copied(report) => {
                let selected = report
                    .first_copied_id
                    .as_ref()
                    .and_then(|id| report.annotations.iter().position(|item| &item.id == id))
                    .unwrap_or(return_index);
                Self::return_to_annotations(context.mode, selected, report.annotations.len());
                let status = format!(
                    "Copied {} validated annotations; {} skipped. Original records retained.",
                    report.copied_count, report.skipped_count
                );
                Some(ReassociationCompletion {
                    inputs: pending.inputs,
                    copied: Some(ReassociationCopy { report, status }),
                })
            }
        }
    }

    fn return_to_annotations(mode: &mut ReaderMode, selected: usize, annotation_count: usize) {
        *mode = ReaderMode::Annotations(AnnotationMode::List {
            selected: selected.min(annotation_count.saturating_sub(1)),
        });
    }
}

#[derive(Clone, Debug)]
pub enum ReassociationMode {
    Sources(ReassociationPicker),
    Confirm(ReassociationPicker),
    Pending(Box<PendingReassociation>),
}
