use std::collections::VecDeque;

use super::{
    ModalInput, ModalKeyOutcome, ModeContext, PendingModeContext, ReaderEffect, ReaderMode,
};
use crate::annotations::{
    Destination, ExportState, LinkCommand, LinkJob, LinkReply, LinkResult, OverwriteApproval,
};
use crate::app::editor::{EditorAction, EditorOptions, EditorState};
use crate::configuration::InputKey;

pub type PendingLink = super::pending::PendingOperation<LinkJob, Box<LinkMode>>;

#[derive(Default)]
pub(in crate::app) struct MarkdownState {
    requests: super::RequestSequence,
    export: ExportState,
}

impl MarkdownState {
    pub(in crate::app) fn export(&self) -> &ExportState {
        &self.export
    }

    pub(in crate::app) fn metadata_loaded(
        &mut self,
        context: ModeContext<'_>,
        mut state: ExportState,
    ) {
        if !state.running
            && state.error.is_none()
            && state.link.is_some()
            && state.link == self.export.link
        {
            state.error = self.export.error.clone();
        }
        self.export_updated(context, state);
    }

    pub(in crate::app) fn export_updated(&mut self, context: ModeContext<'_>, state: ExportState) {
        if !self.accept_export(state) {
            return;
        }
        if let Some(error) = &self.export.error {
            *context.status = format!("Annotations saved; export pending: {error}");
        } else if !self.export.running
            && matches!(context.mode, ReaderMode::Link(mode) if matches!(mode.as_ref(), LinkMode::Summary))
        {
            *context.status = "Markdown export up to date.".into();
        }
    }

    fn accept_export(&mut self, state: ExportState) -> bool {
        if let Some(current) = &self.export.link
            && let Some(incoming) = &state.link
            && current.book_id == incoming.book_id
            && current.path == incoming.path
            && (incoming.revision < current.revision
                || (state.error.is_none()
                    && incoming.revision == current.revision
                    && incoming.exported_revision < current.exported_revision))
        {
            return false;
        }
        self.export = state;
        true
    }

    pub(in crate::app) fn begin(
        &mut self,
        context: PendingModeContext<'_>,
        annotations_enabled: bool,
    ) -> Vec<ReaderEffect> {
        if !annotations_enabled {
            *context.status = "Annotations are available only for local EPUB books.".into();
            return Vec::new();
        }
        self.submit(context, LinkCommand::Status, LinkMode::Summary)
    }

    fn submit(
        &mut self,
        context: PendingModeContext<'_>,
        command: LinkCommand,
        resume: LinkMode,
    ) -> Vec<ReaderEffect> {
        let job = LinkJob {
            request_id: self.requests.next_saturating(),
            command,
        };
        let pending = PendingLink::new(job.clone(), Box::new(resume));
        *context.overflow = false;
        *context.mode = ReaderMode::Link(Box::new(LinkMode::Pending(Box::new(pending))));
        *context.status = "Checking Markdown link…".into();
        vec![ReaderEffect::Link(job)]
    }

    pub(in crate::app) fn paste(mut context: PendingModeContext<'_>, text: &str) {
        let accepted = if let ReaderMode::Link(mode) = &mut *context.mode {
            match mode.as_mut() {
                LinkMode::Editing(editor) => {
                    if text.chars().any(char::is_control) {
                        *context.status =
                            "Paste a single literal path without control characters.".into();
                    } else {
                        editor.insert(text);
                        context.status.clear();
                    }
                    true
                }
                LinkMode::Pending(pending) => pending.push_paste(text),
                _ => true,
            }
        } else {
            true
        };
        if !accepted {
            context.report_overflow();
        }
    }

    pub(in crate::app) fn key(
        &mut self,
        context: PendingModeContext<'_>,
        key: InputKey,
    ) -> ModalKeyOutcome {
        let ReaderMode::Link(mode) = std::mem::replace(context.mode, ReaderMode::Reading) else {
            return ModalKeyOutcome::Effects(Vec::new());
        };
        let mode = *mode;
        if let LinkMode::Pending(pending) = mode {
            return ModalKeyOutcome::Effects(Self::pending_key(context, *pending, key));
        }
        context.status.clear();
        if key == InputKey::Character('\u{03}') {
            return ModalKeyOutcome::Quit;
        }
        let effects = match mode {
            LinkMode::Summary => self.summary_key(context, key),
            LinkMode::Editing(_) if key == InputKey::Character('\u{15}') => {
                *context.mode = ReaderMode::Link(Box::new(path_editor("")));
                Vec::new()
            }
            LinkMode::Editing(mut editor) => match editor.handle(key) {
                Some(EditorAction::Save) => self.submit(
                    context,
                    LinkCommand::Inspect(editor.text().into()),
                    LinkMode::Editing(editor),
                ),
                Some(EditorAction::Cancel) => Vec::new(),
                _ => {
                    *context.mode = ReaderMode::Link(Box::new(LinkMode::Editing(editor)));
                    Vec::new()
                }
            },
            LinkMode::Destination {
                editor,
                destination,
            } => {
                if key == InputKey::Character('y') {
                    if destination.exists {
                        return ModalKeyOutcome::Effects(self.submit(
                            context,
                            LinkCommand::Set {
                                destination,
                                create: false,
                            },
                            LinkMode::Editing(editor),
                        ));
                    }
                    *context.mode = ReaderMode::Link(Box::new(LinkMode::Create {
                        editor,
                        destination,
                    }));
                } else {
                    *context.mode = ReaderMode::Link(Box::new(LinkMode::Editing(editor)));
                }
                Vec::new()
            }
            LinkMode::Create {
                editor,
                destination,
            } => {
                if key == InputKey::Character('y') {
                    self.submit(
                        context,
                        LinkCommand::Set {
                            destination,
                            create: true,
                        },
                        LinkMode::Editing(editor),
                    )
                } else {
                    *context.mode = ReaderMode::Link(Box::new(LinkMode::Editing(editor)));
                    Vec::new()
                }
            }
            LinkMode::Unlink => {
                if matches!(key, InputKey::Character('y' | 'Y')) {
                    self.submit(context, LinkCommand::Unlink, LinkMode::Summary)
                } else {
                    *context.mode = ReaderMode::Link(Box::new(LinkMode::Summary));
                    Vec::new()
                }
            }
            LinkMode::Overwrite(approval) => {
                if key == InputKey::Character('y') {
                    self.submit(context, LinkCommand::Overwrite(approval), LinkMode::Summary)
                } else {
                    *context.mode = ReaderMode::Link(Box::new(LinkMode::Summary));
                    Vec::new()
                }
            }
            LinkMode::Pending(_) => unreachable!(),
        };
        ModalKeyOutcome::Effects(effects)
    }

    fn pending_key(
        mut context: PendingModeContext<'_>,
        mut pending: PendingLink,
        key: InputKey,
    ) -> Vec<ReaderEffect> {
        match pending.handle_key(key) {
            super::pending::PendingAction::Resume => {
                *context.overflow = false;
                *context.mode = ReaderMode::Link(pending.resume);
                context.status.clear();
                return Vec::new();
            }
            super::pending::PendingAction::Retry => {
                let job = pending.job.clone();
                *context.mode = ReaderMode::Link(Box::new(LinkMode::Pending(Box::new(pending))));
                return vec![ReaderEffect::Link(job)];
            }
            super::pending::PendingAction::Overflow => context.report_overflow(),
            super::pending::PendingAction::Buffered => {}
        }
        *context.mode = ReaderMode::Link(Box::new(LinkMode::Pending(Box::new(pending))));
        Vec::new()
    }

    fn summary_key(&mut self, context: PendingModeContext<'_>, key: InputKey) -> Vec<ReaderEffect> {
        match key {
            InputKey::Escape | InputKey::Character('q') => Vec::new(),
            InputKey::Character('r') => {
                self.submit(context, LinkCommand::Refresh, LinkMode::Summary)
            }
            InputKey::Character('w') => {
                self.submit(context, LinkCommand::PrepareOverwrite, LinkMode::Summary)
            }
            InputKey::Character('u') => {
                *context.mode = ReaderMode::Link(Box::new(LinkMode::Unlink));
                Vec::new()
            }
            InputKey::Enter => {
                let path = self.export.link.as_ref().map_or("", |link| &link.path);
                *context.mode = ReaderMode::Link(Box::new(path_editor(path)));
                Vec::new()
            }
            _ => {
                *context.mode = ReaderMode::Link(Box::new(LinkMode::Summary));
                Vec::new()
            }
        }
    }

    pub(in crate::app) fn completed(
        &mut self,
        context: ModeContext<'_>,
        result: LinkResult,
    ) -> Option<VecDeque<ModalInput>> {
        let previous = std::mem::replace(context.mode, ReaderMode::Reading);
        let ReaderMode::Link(mode) = previous else {
            *context.mode = previous;
            return None;
        };
        let LinkMode::Pending(pending) = *mode else {
            *context.mode = ReaderMode::Link(mode);
            return None;
        };
        let completion = super::pending::PendingCompletion {
            request_id: result.request_id,
            result: result.reply,
        };
        let transition = super::pending::PendingTransition {
            restore: |pending| ReaderMode::Link(Box::new(LinkMode::Pending(Box::new(pending)))),
            failure_message: |error| {
                format!("Markdown operation failed: {error}. Enter retries; Esc returns.")
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
        context.status.clear();
        match completed.reply {
            LinkReply::Destination(destination) => {
                let LinkMode::Editing(editor) = *pending.resume else {
                    *context.mode = ReaderMode::Link(pending.resume);
                    return None;
                };
                *context.mode = ReaderMode::Link(Box::new(LinkMode::Destination {
                    editor,
                    destination,
                }));
            }
            LinkReply::Overwrite(approval) => {
                *context.mode = ReaderMode::Link(Box::new(LinkMode::Overwrite(approval)))
            }
            LinkReply::State(state) => {
                self.metadata_loaded(
                    ModeContext {
                        mode: &mut *context.mode,
                        status: &mut *context.status,
                    },
                    state,
                );
                if matches!(pending.job.command, LinkCommand::Unlink) {
                    *context.status =
                        "Markdown link removed; annotations and files retained.".into();
                } else {
                    *context.mode = ReaderMode::Link(Box::new(if self.export.link.is_some() {
                        LinkMode::Summary
                    } else {
                        path_editor("")
                    }));
                    if self.export.running {
                        *context.status = "Markdown export queued.".into();
                    }
                }
            }
        }
        Some(pending.inputs)
    }
}

#[derive(Clone, Debug)]
pub enum LinkMode {
    Summary,
    Editing(EditorState),
    Destination {
        editor: EditorState,
        destination: Destination,
    },
    Create {
        editor: EditorState,
        destination: Destination,
    },
    Overwrite(OverwriteApproval),
    Unlink,
    Pending(Box<PendingLink>),
}

fn path_editor(path: &str) -> LinkMode {
    LinkMode::Editing(EditorState::new(
        path,
        EditorOptions {
            multiline: false,
            can_delete: false,
        },
    ))
}
