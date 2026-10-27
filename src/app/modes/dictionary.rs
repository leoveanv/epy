use super::{ModeContext, ModeInput, ReaderEffect, ReaderMode};
use crate::app::editor::{EditorAction, EditorOptions, EditorState};
use crate::configuration::{Command, InputKey};
use crate::integrations::{
    DICTIONARY_QUERY_BYTES_MAX, DictionaryJob, DictionaryPage, DictionaryResult,
};

#[derive(Default)]
pub(in crate::app) struct DictionaryState {
    requests: super::RequestSequence,
}

pub(in crate::app) enum DictionaryOutcome {
    Effects(Vec<ReaderEffect>),
    Navigate(Command),
}

impl DictionaryState {
    pub(in crate::app) fn begin(context: ModeContext<'_>) -> Vec<ReaderEffect> {
        *context.mode = ReaderMode::Dictionary(Box::new(DictionaryMode::Editing(prompt())));
        *context.status = "Define word | Enter lookup | Escape cancel".into();
        vec![ReaderEffect::CancelDictionary]
    }

    pub(in crate::app) fn key(
        &mut self,
        context: ModeContext<'_>,
        input: ModeInput,
        viewport_rows: usize,
    ) -> DictionaryOutcome {
        let ReaderMode::Dictionary(mode) = std::mem::replace(context.mode, ReaderMode::Reading)
        else {
            return DictionaryOutcome::Effects(Vec::new());
        };
        if matches!(input.key, InputKey::Escape | InputKey::Character('\u{03}'))
            || (!matches!(mode.as_ref(), DictionaryMode::Editing(_))
                && input.command == Some(Command::Quit))
        {
            context.status.clear();
            return DictionaryOutcome::Effects(vec![ReaderEffect::CancelDictionary]);
        }
        match *mode {
            DictionaryMode::Editing(editor) => {
                DictionaryOutcome::Effects(self.edit_key(context, editor, input.key))
            }
            DictionaryMode::Loading { request_id, word } => {
                if input.command == Some(Command::DefineWord) {
                    return DictionaryOutcome::Effects(Self::begin(context));
                }
                *context.mode =
                    ReaderMode::Dictionary(Box::new(DictionaryMode::Loading { request_id, word }));
                DictionaryOutcome::Effects(Vec::new())
            }
            DictionaryMode::Viewing(view) => {
                Self::definition_key(context, view, input.command, viewport_rows)
            }
        }
    }

    fn edit_key(
        &mut self,
        context: ModeContext<'_>,
        mut editor: EditorState,
        key: InputKey,
    ) -> Vec<ReaderEffect> {
        if key == InputKey::Character('\u{15}') {
            editor = prompt();
        } else {
            if let InputKey::Character(character) = key
                && !character.is_control()
                && editor.focus() == 0
                && editor.text().len().saturating_add(character.len_utf8())
                    > DICTIONARY_QUERY_BYTES_MAX
            {
                *context.status = "The dictionary query is too long. Ctrl-U clears it.".into();
                *context.mode = ReaderMode::Dictionary(Box::new(DictionaryMode::Editing(editor)));
                return Vec::new();
            }
            let action = if key == InputKey::Enter && editor.focus() == 0 {
                Some(EditorAction::Save)
            } else {
                editor.handle(key)
            };
            match action {
                Some(EditorAction::Save) if !editor.text().trim().is_empty() => {
                    let job = self.lookup(editor.text());
                    *context.status = format!("Looking up {} | Escape cancels", job.word);
                    *context.mode = ReaderMode::Dictionary(Box::new(DictionaryMode::Loading {
                        request_id: job.request_id,
                        word: job.word.clone(),
                    }));
                    return vec![ReaderEffect::Dictionary(job)];
                }
                Some(EditorAction::Cancel | EditorAction::Save) => {
                    context.status.clear();
                    return vec![ReaderEffect::CancelDictionary];
                }
                _ => {}
            }
        }
        *context.mode = ReaderMode::Dictionary(Box::new(DictionaryMode::Editing(editor)));
        Vec::new()
    }

    fn lookup(&mut self, word: &str) -> DictionaryJob {
        DictionaryJob {
            request_id: self.requests.next_saturating(),
            word: word.into(),
        }
    }

    pub(in crate::app) fn paste(context: ModeContext<'_>, text: &str) {
        if let ReaderMode::Dictionary(mode) = context.mode
            && let DictionaryMode::Editing(editor) = mode.as_mut()
        {
            if text.chars().any(char::is_control)
                || editor.text().len().saturating_add(text.len()) > DICTIONARY_QUERY_BYTES_MAX
            {
                *context.status =
                    "Paste one dictionary query without control characters, at most 4096 bytes."
                        .into();
            } else {
                editor.insert(text);
            }
        }
    }

    pub(in crate::app) fn completed(context: ModeContext<'_>, result: DictionaryResult) {
        if !matches!(context.mode, ReaderMode::Dictionary(mode) if matches!(mode.as_ref(), DictionaryMode::Loading { request_id, .. } if *request_id == result.request_id))
        {
            return;
        }
        *context.status = "Definition | j/k scroll | h/l page | g/G ends | q close".into();
        *context.mode = ReaderMode::Dictionary(Box::new(DictionaryMode::Viewing(DefinitionView {
            page: result.page,
            top: 0,
        })));
    }

    fn definition_key(
        context: ModeContext<'_>,
        mut view: DefinitionView,
        command: Option<Command>,
        viewport_rows: usize,
    ) -> DictionaryOutcome {
        if command == Some(Command::DefineWord) {
            context.status.clear();
            return DictionaryOutcome::Effects(vec![ReaderEffect::CancelDictionary]);
        }
        if let Some(command @ (Command::Metadata | Command::Help | Command::TableOfContents)) =
            command
        {
            return DictionaryOutcome::Navigate(command);
        }
        let height = viewport_rows.saturating_sub(4).max(1);
        let maximum = view.page.lines.len().saturating_sub(height);
        view.top = match command {
            Some(Command::ScrollDown) => view.top.saturating_add(1).min(maximum),
            Some(Command::ScrollUp) => view.top.saturating_sub(1),
            Some(Command::PageDown) => view.top.saturating_add(height).min(maximum),
            Some(Command::PageUp) => view.top.saturating_sub(height),
            Some(Command::BeginningOfCh) => 0,
            Some(Command::EndOfCh) => maximum,
            _ => view.top,
        };
        *context.mode = ReaderMode::Dictionary(Box::new(DictionaryMode::Viewing(view)));
        DictionaryOutcome::Effects(Vec::new())
    }
}

#[derive(Clone, Debug)]
pub struct DefinitionView {
    pub page: DictionaryPage,
    pub top: usize,
}

#[derive(Clone, Debug)]
pub enum DictionaryMode {
    Editing(EditorState),
    Loading { request_id: u64, word: String },
    Viewing(DefinitionView),
}

fn prompt() -> EditorState {
    EditorState::new(
        "",
        EditorOptions {
            multiline: false,
            can_delete: false,
        },
    )
}
