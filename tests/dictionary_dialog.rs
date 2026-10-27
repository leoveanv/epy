#[path = "support/reader.rs"]
mod support;

use ers::app::modes::dictionary::DictionaryMode;
use ers::app::{ReaderEffect, ReaderMode, ReaderModel, Viewport};
use ers::configuration::InputKey;
use ers::integrations::{DictionaryDefinition, DictionaryJob, DictionaryPage, DictionaryResult};

fn start(reader: &mut ReaderModel, word: &str) -> DictionaryJob {
    reader.handle_key(InputKey::Character('d'));
    reader.handle_paste(word);
    reader
        .handle_key(InputKey::Enter)
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Dictionary(job) => Some(job),
            _ => None,
        })
        .expect("Submitting a word must request a background definition")
}

fn completed(job: &DictionaryJob, lines: usize) -> DictionaryResult {
    let text = (0..lines)
        .map(|row| format!("Definition row {row}"))
        .collect::<Vec<_>>()
        .join("\n");
    let page = DictionaryPage::new(
        &format!("Definition: {}", job.word.to_uppercase()),
        &text,
        94,
    );
    DictionaryResult {
        request_id: job.request_id,
        result: Ok(DictionaryDefinition {
            word: job.word.clone(),
            text,
        }),
        page,
    }
}

#[test]
fn dictionary_prompts_submit_unicode_without_moving_the_book() {
    let mut reader = support::reader();
    let before = reader.view_position();
    let job = start(&mut reader, "naïve reader");
    assert_eq!(job.word, "naïve reader");
    assert_eq!(reader.view_position(), before);
    assert!(
        matches!(reader.view().mode, ReaderMode::Dictionary(mode) if matches!(mode.as_ref(), DictionaryMode::Loading { .. }))
    );
}

#[test]
fn canceling_dictionary_queries_ignores_late_results() {
    let mut reader = support::reader();
    let before = reader.view_position();
    let job = start(&mut reader, "word");
    assert!(
        reader
            .handle_key(InputKey::Escape)
            .iter()
            .any(|effect| matches!(effect, ReaderEffect::CancelDictionary))
    );
    reader.dictionary_completed(completed(&job, 5));
    assert!(matches!(reader.view().mode, ReaderMode::Reading));
    assert_eq!(reader.view_position(), before);
}

#[test]
fn definition_scrolling_clamps_at_the_last_visible_page() {
    let mut reader = support::reader();
    let job = start(&mut reader, "word");
    reader.dictionary_completed(completed(&job, 60));
    for _ in 0..100 {
        reader.handle_key(InputKey::Character('j'));
    }
    let ReaderMode::Dictionary(mode) = reader.view().mode else {
        panic!("The definition must remain open")
    };
    let DictionaryMode::Viewing(view) = mode.as_ref() else {
        panic!("The definition must be visible")
    };
    assert_eq!(
        view.top,
        60 - reader.view().viewport.rows.saturating_sub(4).max(1)
    );
    reader.handle_key(InputKey::Character('k'));
    let ReaderMode::Dictionary(mode) = reader.view().mode else {
        panic!("The definition must remain open")
    };
    let DictionaryMode::Viewing(view) = mode.as_ref() else {
        panic!("The definition must be visible")
    };
    assert_eq!(
        view.top,
        60 - reader.view().viewport.rows.saturating_sub(4).max(1) - 1
    );
}

#[test]
fn pasted_definition_controls_cannot_close_the_viewer() {
    let mut reader = support::reader();
    let before = reader.view_position();
    let job = start(&mut reader, "word");
    reader.dictionary_completed(completed(&job, 5));
    reader.handle_paste("q\n");
    assert!(matches!(reader.view().mode, ReaderMode::Dictionary(_)));
    reader.handle_key(InputKey::Character('q'));
    assert!(matches!(reader.view().mode, ReaderMode::Reading));
    assert_eq!(reader.view_position(), before);
    assert!(!reader.view().quitting);
}

#[test]
fn resizing_dictionary_windows_dismisses_stale_viewport_results() {
    let mut reader = support::reader();
    let job = start(&mut reader, "word");
    let effects = reader.resize(Viewport {
        columns: 100,
        rows: 8,
    });
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, ReaderEffect::CancelDictionary))
    );
    reader.dictionary_completed(completed(&job, 5));
    assert!(matches!(reader.view().mode, ReaderMode::Reading));
}

#[test]
fn superseded_dictionary_results_cannot_replace_the_current_query() {
    let mut reader = support::reader();
    let first = start(&mut reader, "first");
    reader.handle_key(InputKey::Escape);
    let second = start(&mut reader, "second");
    reader.dictionary_completed(completed(&first, 5));
    assert!(
        matches!(reader.view().mode, ReaderMode::Dictionary(mode) if matches!(mode.as_ref(), DictionaryMode::Loading { request_id, .. } if *request_id == second.request_id))
    );
    reader.dictionary_completed(completed(&second, 5));
    let ReaderMode::Dictionary(mode) = reader.view().mode else {
        panic!("The definition must remain open")
    };
    let DictionaryMode::Viewing(view) = mode.as_ref() else {
        panic!("The current definition must appear")
    };
    assert_eq!(view.page.title, "Definition: SECOND");
}

#[test]
fn dictionary_prompts_keep_reader_shortcuts_as_query_text() {
    let mut reader = support::reader();
    reader.handle_key(InputKey::Character('d'));
    for character in "dictionary quest".chars() {
        reader.handle_key(InputKey::Character(character));
    }
    let ReaderMode::Dictionary(mode) = reader.view().mode else {
        panic!("Typing q must not close the prompt")
    };
    let DictionaryMode::Editing(editor) = mode.as_ref() else {
        panic!("Typing must not submit the prompt")
    };
    assert_eq!(editor.text(), "dictionary quest");
}

#[test]
fn dictionary_prompts_reject_pasted_control_characters() {
    let mut reader = support::reader();
    reader.handle_key(InputKey::Character('d'));
    reader.handle_paste("word\nq");
    let ReaderMode::Dictionary(mode) = reader.view().mode else {
        panic!("The prompt must remain open")
    };
    let DictionaryMode::Editing(editor) = mode.as_ref() else {
        panic!("The prompt must remain editable")
    };
    assert!(editor.text().is_empty());
    assert!(!reader.view().quitting);
}

#[test]
fn metadata_shortcuts_leave_the_definition_without_changing_position() {
    let mut reader = support::reader();
    let before = reader.view_position();
    let job = start(&mut reader, "word");
    reader.dictionary_completed(completed(&job, 5));
    reader.handle_key(InputKey::Character('M'));
    assert!(matches!(reader.view().mode, ReaderMode::Dialog(dialog) if dialog.title == "Metadata"));
    assert_eq!(reader.view_position(), before);
}
