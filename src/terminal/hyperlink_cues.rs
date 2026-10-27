use std::collections::BTreeMap;
use std::ops::Range;

use crate::app::{ReaderMode, ReaderModel};
use crate::configuration::{Command, InputKey, KeyBindings};
use crate::document::RenderedDocument;
use crate::document::links::TextLink;
use crate::reading::VisibleRow;

pub(super) struct VisibleLinks<'a> {
    documents: BTreeMap<usize, Vec<&'a TextLink>>,
}

impl<'a> VisibleLinks<'a> {
    pub(super) fn new(rows: &[VisibleRow<'a>]) -> Self {
        let mut documents = BTreeMap::new();
        for row in rows {
            documents
                .entry(row.document.index)
                .or_insert_with(|| visible_document_links(row.document, rows));
        }
        Self { documents }
    }

    pub(super) fn overlaps(&self, document: usize, source: Range<usize>) -> bool {
        self.documents.get(&document).is_some_and(|links| {
            links
                .iter()
                .any(|link| link.start < source.end && link.end > source.start)
        })
    }

    fn count(&self) -> usize {
        self.documents.values().map(Vec::len).sum()
    }

    pub(super) fn status_hint(&self, model: &ReaderModel) -> Option<String> {
        let count = self.count();
        if count == 0 || !matches!(model.view().mode, ReaderMode::Reading) {
            return None;
        }
        let key = follow_key(&model.view().configuration.bindings)?;
        let noun = if count == 1 { "link" } else { "links" };
        Some(format!("{count} {noun} · {key}: follow"))
    }
}

fn visible_document_links<'a>(
    document: &'a RenderedDocument,
    rows: &[VisibleRow<'a>],
) -> Vec<&'a TextLink> {
    let links = &document.parsed.semantics.links;
    if links.is_empty() {
        return Vec::new();
    }
    let cells = rows
        .iter()
        .filter(|row| row.document.index == document.index)
        .flat_map(|row| document.layout.cells_on_row(row.row))
        .collect::<Vec<_>>();
    let (Some(first), Some(last)) = (cells.first(), cells.last()) else {
        return Vec::new();
    };
    links
        .iter()
        .filter(|link| link.start < last.end && link.end > first.start)
        .filter(|link| {
            cells
                .iter()
                .any(|cell| cell.start < link.end && cell.end > link.start)
        })
        .collect()
}

fn follow_key(bindings: &KeyBindings) -> Option<String> {
    let configured = bindings
        .binding(Command::Follow)
        .unwrap_or_default()
        .chars()
        .map(InputKey::from_character);
    configured
        .chain(Command::Follow.builtin_keys().iter().copied())
        .filter(|key| bindings.command_for(*key) == Some(Command::Follow))
        .find_map(key_name)
}

fn key_name(key: InputKey) -> Option<String> {
    match key {
        InputKey::Character(' ') => Some("Space".into()),
        InputKey::Character(character @ '\u{1}'..='\u{1a}') => {
            Some(format!("Ctrl-{}", char::from(b'@' + character as u8)))
        }
        InputKey::Character(character) if !character.is_control() => Some(character.to_string()),
        InputKey::Tab => Some("Tab".into()),
        InputKey::Enter => Some("Enter".into()),
        InputKey::Escape => Some("Esc".into()),
        InputKey::ControlI => Some("Ctrl-I".into()),
        InputKey::Backspace => Some("Backspace".into()),
        _ => None,
    }
}
