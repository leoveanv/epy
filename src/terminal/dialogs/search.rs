use crate::app::modes::search::SearchMode;
use crate::app::{ReaderMode, ReaderModel};
use ratatui::Frame;
use ratatui::style::Style;

pub(in crate::terminal) fn status_hint(model: &ReaderModel) -> Option<String> {
    let ReaderMode::Search(mode) = model.view().mode else {
        return None;
    };
    match mode.as_ref() {
        SearchMode::Editing(_) => {
            Some("Regex | Enter search | Ctrl-U clear | Escape cancel".into())
        }
        SearchMode::Results(session) => Some(session.selection.as_ref().map_or_else(
            || {
                format!(
                    "Searching: {} | n/N next/previous | Escape close",
                    session.pattern
                )
            },
            |selection| {
                format!(
                    "Searching: {} | Match {}/{} Ch {}/{} | n/N next/previous | Escape close",
                    session.pattern,
                    selection.selected + 1,
                    selection.matches.hits.len(),
                    selection.document.index + 1,
                    model.view().info.contents.len()
                )
            },
        )),
    }
}

pub(in crate::terminal) fn selected(
    model: &ReaderModel,
    row: usize,
    column: usize,
    width: usize,
) -> bool {
    let Some(selection) = model.search_selection().filter(|selection| {
        selection.document.width == model.view().width
            && selection.document.index == model.view().rendered.index
    }) else {
        return false;
    };
    let Some(hit) = selection.matches.hits.get(selection.selected) else {
        return false;
    };
    hit.row == row
        && if hit.column_start == hit.column_end {
            column <= hit.column_start && column + width > hit.column_start
        } else {
            column < hit.column_end && column + width > hit.column_start
        }
}

pub(in crate::terminal) fn zero_marker(
    model: &ReaderModel,
    row: usize,
    line_width: usize,
) -> Option<usize> {
    let selection = model.search_selection()?;
    if selection.document.width != model.view().width
        || selection.document.index != model.view().rendered.index
    {
        return None;
    }
    let hit = selection.matches.hits.get(selection.selected)?;
    (hit.row == row && hit.column_start == hit.column_end && hit.column_start >= line_width)
        .then_some(hit.column_start)
}

pub(in crate::terminal) fn draw(frame: &mut Frame<'_>, model: &ReaderModel, style: Style) {
    let ReaderMode::Search(mode) = model.view().mode else {
        return;
    };
    let SearchMode::Editing(editor) = mode.as_ref() else {
        return;
    };
    super::prompt::draw(
        frame,
        super::prompt::Prompt {
            editor,
            title: "Regex search | Enter search | Ctrl-U clear | Tab controls | Esc cancel",
            accept_label: "Search",
            style,
        },
    );
}
