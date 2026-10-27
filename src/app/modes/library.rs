use std::collections::VecDeque;
use std::sync::Arc;

use super::{ReaderEffect, ReaderMode, ReaderModel};
use crate::app::requests::{LibraryCommand, LibraryJob, LibraryReply, LibraryResult};
use crate::book::LibraryEntry;
use crate::configuration::{Command, InputKey};

#[derive(Default)]
pub(in crate::app) struct LibraryState {
    requests: super::RequestSequence,
}

impl LibraryState {
    fn start(&mut self, command: LibraryCommand) -> LibraryJob {
        LibraryJob {
            request_id: self.requests.next(),
            command,
        }
    }
}

#[derive(Clone, Debug)]
pub enum LibraryPhase {
    Browsing,
    ConfirmRemove,
    Waiting {
        request_id: u64,
        command: LibraryCommand,
    },
    Opening(String),
}

#[derive(Clone, Debug)]
pub struct LibraryMode {
    pub entries: Vec<LibraryEntry>,
    pub selected: usize,
    pub phase: LibraryPhase,
    prefix: String,
    queued: VecDeque<LibraryInput>,
    ignored_input: bool,
}

#[derive(Clone, Debug)]
enum LibraryInput {
    Key(InputKey),
    Motion { key: InputKey, repetitions: usize },
}

impl LibraryInput {
    fn from_key(key: InputKey, command: Option<Command>) -> Self {
        if matches!(
            command,
            Some(Command::ScrollDown | Command::ScrollUp | Command::PageDown | Command::PageUp)
        ) {
            Self::Motion {
                key,
                repetitions: 1,
            }
        } else {
            Self::Key(key)
        }
    }
}

impl LibraryMode {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            selected: 0,
            phase: LibraryPhase::Browsing,
            prefix: String::new(),
            queued: VecDeque::new(),
            ignored_input: false,
        }
    }

    fn queue(&mut self, input: LibraryInput) -> bool {
        if let (
            Some(LibraryInput::Motion {
                key: previous,
                repetitions,
            }),
            LibraryInput::Motion {
                key,
                repetitions: incoming,
            },
        ) = (self.queued.back_mut(), &input)
            && previous == key
        {
            *repetitions = repetitions.saturating_add(*incoming);
            return true;
        }
        if self.queued.len() < 128 {
            self.queued.push_back(input);
            true
        } else {
            self.ignored_input = true;
            false
        }
    }
}

impl ReaderModel {
    pub fn can_autosave(&self) -> bool {
        !self.loading && !self.quitting && !matches!(self.mode, ReaderMode::Library(_))
    }

    pub(in crate::app) fn begin_library(&mut self) -> Vec<ReaderEffect> {
        self.status = "Saving position and loading library…".into();
        self.submit_library(
            LibraryMode::new(),
            LibraryCommand::List(self.save_request(false)),
        )
    }

    fn submit_library(
        &mut self,
        mut mode: LibraryMode,
        command: LibraryCommand,
    ) -> Vec<ReaderEffect> {
        let job = self.library_state.start(command);
        mode.phase = LibraryPhase::Waiting {
            request_id: job.request_id,
            command: job.command.clone(),
        };
        self.mode = ReaderMode::Library(Box::new(mode));
        vec![ReaderEffect::Library(job)]
    }

    pub(in crate::app) fn handle_library_key(&mut self, key: InputKey) -> Vec<ReaderEffect> {
        let ReaderMode::Library(mut mode) = std::mem::replace(&mut self.mode, ReaderMode::Reading)
        else {
            return Vec::new();
        };
        let command = self.configuration.bindings.command_for(key);
        let opening = matches!(mode.phase, LibraryPhase::Opening(_));
        if key == InputKey::Character('\u{03}') {
            let mut effects = if opening {
                vec![ReaderEffect::CancelBookOpen]
            } else {
                Vec::new()
            };
            effects.extend(self.execute(Command::Quit, 1));
            return effects;
        }
        if key == InputKey::Escape && !mode.prefix.is_empty() {
            mode.prefix.clear();
        } else if matches!(key, InputKey::Escape | InputKey::Character('q'))
            || matches!(command, Some(Command::Library | Command::Quit))
        {
            self.status.clear();
            if opening {
                mode.phase = LibraryPhase::Browsing;
                mode.queued.clear();
                self.mode = ReaderMode::Library(mode);
                let mut effects = vec![ReaderEffect::CancelBookOpen];
                effects.extend(self.resize(self.viewport));
                return effects;
            }
            if matches!(mode.phase, LibraryPhase::ConfirmRemove) {
                mode.phase = LibraryPhase::Browsing;
            } else {
                return Vec::new();
            }
        } else {
            match &mode.phase {
                LibraryPhase::Waiting { .. } => {
                    if !mode.queue(LibraryInput::from_key(key, command)) {
                        self.status = "Input queue full. New input was ignored.".into();
                    }
                }
                LibraryPhase::Opening(_) => {}
                LibraryPhase::ConfirmRemove => {
                    if matches!(key, InputKey::Character('y' | 'Y')) {
                        return self.remove_library_entry(*mode);
                    } else {
                        mode.phase = LibraryPhase::Browsing;
                    }
                }
                LibraryPhase::Browsing => return self.library_browse_key(*mode, key, command),
            }
        }
        self.mode = ReaderMode::Library(mode);
        Vec::new()
    }

    fn library_browse_key(
        &mut self,
        mut mode: LibraryMode,
        key: InputKey,
        command: Option<Command>,
    ) -> Vec<ReaderEffect> {
        if let InputKey::Character(digit) = key
            && digit.is_ascii_digit()
        {
            if mode.prefix.len() < 7 {
                mode.prefix.push(digit);
            }
            self.mode = ReaderMode::Library(Box::new(mode));
            return Vec::new();
        }
        let count = mode.prefix.parse::<usize>().unwrap_or(1).max(1);
        mode.prefix.clear();
        match command {
            Some(Command::ScrollDown | Command::PageDown) => {
                mode.selected = mode
                    .selected
                    .saturating_add(count)
                    .min(mode.entries.len().saturating_sub(1))
            }
            Some(Command::ScrollUp | Command::PageUp) => {
                mode.selected = mode.selected.saturating_sub(count)
            }
            Some(Command::BeginningOfCh) => mode.selected = 0,
            Some(Command::EndOfCh) => mode.selected = mode.entries.len().saturating_sub(1),
            _ if key == InputKey::Enter || command == Some(Command::Follow) => {
                if self.loading {
                    self.queue_input(super::QueuedInput::Input(super::ModalInput::Key(key)));
                    self.mode = ReaderMode::Library(Box::new(mode));
                    return Vec::new();
                }
                if let Some(entry) = mode.entries.get(mode.selected) {
                    if entry.filepath == self.info.path {
                        self.status.clear();
                        return Vec::new();
                    }
                    let path = entry.filepath.clone();
                    self.status = "Saving position before switching books…".into();
                    return self.submit_library(
                        mode,
                        LibraryCommand::Switch {
                            path,
                            departure: self.save_request(false),
                        },
                    );
                }
            }
            _ if key == InputKey::Character('D') => return self.remove_library_entry(mode),
            _ if matches!(key, InputKey::Delete | InputKey::Character('d'))
                && !mode.entries.is_empty() =>
            {
                mode.phase = LibraryPhase::ConfirmRemove;
            }
            _ => {}
        }
        self.mode = ReaderMode::Library(Box::new(mode));
        Vec::new()
    }

    fn remove_library_entry(&mut self, mode: LibraryMode) -> Vec<ReaderEffect> {
        let Some(entry) = mode.entries.get(mode.selected) else {
            self.mode = ReaderMode::Library(Box::new(mode));
            return Vec::new();
        };
        let path = entry.filepath.clone();
        self.status = "Removing saved position and bookmarks…".into();
        self.submit_library(mode, LibraryCommand::Remove(path))
    }

    pub fn library_completed(&mut self, result: LibraryResult) -> Vec<ReaderEffect> {
        if !matches!(&self.mode, ReaderMode::Library(mode) if matches!(&mode.phase, LibraryPhase::Waiting { request_id, .. } if *request_id == result.request_id))
        {
            return Vec::new();
        }
        let ReaderMode::Library(mut mode) = std::mem::replace(&mut self.mode, ReaderMode::Reading)
        else {
            return Vec::new();
        };
        let previous = std::mem::replace(&mut mode.phase, LibraryPhase::Browsing);
        let mut effects = Vec::new();
        match result.reply {
            Ok(LibraryReply::Entries(entries)) => {
                mode.selected = entries
                    .iter()
                    .position(|entry| entry.filepath == self.info.path)
                    .unwrap_or(0);
                mode.entries = entries;
                self.status.clear();
            }
            Ok(LibraryReply::Removed { path, entries }) => {
                mode.entries = entries;
                mode.selected = mode
                    .selected
                    .saturating_sub(1)
                    .min(mode.entries.len().saturating_sub(1));
                if path == self.info.path {
                    Arc::make_mut(&mut self.info).bookmarks.clear();
                }
                self.status =
                    "Removed saved position and bookmarks. Ebook and annotations retained.".into();
                effects.push(ReaderEffect::ForgetBookSession(path));
            }
            Ok(LibraryReply::SwitchReady(path)) => {
                if matches!(previous, LibraryPhase::Waiting { command: LibraryCommand::Switch { path: expected, .. }, .. } if expected == path)
                {
                    mode.phase = LibraryPhase::Opening(path.clone());
                    self.status = "Opening selected ebook… Escape cancels.".into();
                    effects.push(ReaderEffect::OpenBook(path));
                }
            }
            Err(error) => {
                self.status = format!("Library operation failed: {error}");
                mode.ignored_input |= !mode.queued.is_empty();
                mode.queued.clear();
            }
        }
        let ignored = mode.ignored_input;
        let queued = std::mem::take(&mut mode.queued);
        self.mode = ReaderMode::Library(mode);
        for input in queued {
            effects.extend(self.replay_library_input(input));
        }
        if ignored {
            if let ReaderMode::Library(mode) = &mut self.mode {
                if !matches!(mode.phase, LibraryPhase::Waiting { .. }) {
                    mode.ignored_input = false;
                    mode.prefix.clear();
                }
            } else {
                self.prefix.clear();
            }
            self.pending_input_overflow |= self.loading;
            self.append_input_warning();
        }
        effects
    }

    fn replay_library_input(&mut self, input: LibraryInput) -> Vec<ReaderEffect> {
        match input {
            LibraryInput::Key(key) => self.handle_key(key),
            LibraryInput::Motion { key, repetitions } => {
                self.replay_library_motion(key, repetitions)
            }
        }
    }

    fn replay_library_motion(&mut self, key: InputKey, repetitions: usize) -> Vec<ReaderEffect> {
        match &mut self.mode {
            ReaderMode::Library(mode) => match mode.phase {
                LibraryPhase::Waiting { .. } => {
                    if !mode.queue(LibraryInput::Motion { key, repetitions }) {
                        self.status = "Input queue full. New input was ignored.".into();
                    }
                    return Vec::new();
                }
                LibraryPhase::Browsing => {
                    mode.prefix = mode
                        .prefix
                        .parse::<usize>()
                        .unwrap_or(1)
                        .saturating_add(repetitions.saturating_sub(1))
                        .to_string();
                }
                LibraryPhase::ConfirmRemove if repetitions > 1 => {
                    let mut effects = self.handle_key(key);
                    effects.extend(self.replay_library_input(LibraryInput::Motion {
                        key,
                        repetitions: repetitions - 1,
                    }));
                    return effects;
                }
                _ => {}
            },
            ReaderMode::Reading => {
                self.prefix = self
                    .prefix
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_add(repetitions.saturating_sub(1))
                    .to_string();
            }
            _ => {}
        }
        self.handle_key(key)
    }

    pub fn library_open_failed(&mut self, message: String) {
        if let ReaderMode::Library(mode) = &mut self.mode
            && matches!(mode.phase, LibraryPhase::Opening(_))
        {
            mode.phase = LibraryPhase::Browsing;
            self.status = format!("Cannot open ebook: {message}");
        }
    }
}
