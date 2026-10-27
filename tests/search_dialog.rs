#[path = "support/reader.rs"]
mod support;

use ers::app::{ReaderEffect, ReaderMode, ReaderModel};
use ers::configuration::InputKey;
use ers::search::{SearchJob, SearchLimits, SearchPattern, SearchResult, SearchSelection};

fn start(reader: &mut ReaderModel, pattern: &str) -> SearchJob {
    reader.handle_key(InputKey::Character('/'));
    reader.handle_paste(pattern);
    reader
        .handle_key(InputKey::Enter)
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Search(job) => Some(job),
            _ => None,
        })
        .expect("Submitting a regex must request background search")
}

fn completed(job: &SearchJob, selected: usize) -> SearchResult {
    let matches = SearchPattern::compile(&job.pattern, SearchLimits::default())
        .unwrap()
        .find(
            &job.current.layout,
            &ers::cancellation::Cancellation::default(),
        )
        .unwrap();
    SearchResult {
        request_id: job.request_id,
        result: Ok(Some(SearchSelection {
            document: job.current.clone(),
            matches,
            selected,
            exhausted: false,
        })),
    }
}

#[test]
fn cross_document_search_replaces_the_visible_document_rows() {
    let mut reader = support::reader();
    let mut job = start(&mut reader, "Destination");
    let parsed = ers::document::text::parse_document(
        &reader.view().info.contents[1],
        "<p>Destination café</p>",
    )
    .unwrap();
    job.current = std::sync::Arc::new(ers::document::RenderedDocument::new(
        1,
        parsed,
        reader.view().width,
    ));
    reader.search_completed(completed(&job, 0));
    let visible = reader
        .visible_row(0)
        .expect("the destination must be visible");
    assert_eq!(
        visible.document.layout.text_lines[visible.row],
        "Destination café"
    );
}

#[test]
fn submitting_search_does_not_block_reading_on_the_ui_thread() {
    let mut reader = support::reader();
    let job = start(&mut reader, "Line 20");
    assert_eq!(job.pattern, "Line 20");
    assert_eq!(reader.view().row, 0);
    reader.search_completed(completed(&job, 0));
    assert!(reader.view().row > 0);
    assert_eq!(reader.search_selection().unwrap().matches.hits.len(), 1);
}

#[test]
fn canceling_search_rejects_late_results() {
    let mut reader = support::reader();
    let job = start(&mut reader, "Line 20");
    let effects = reader.handle_key(InputKey::Escape);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, ReaderEffect::CancelSearch))
    );
    reader.search_completed(completed(&job, 0));
    assert!(matches!(reader.view().mode, ReaderMode::Reading));
    assert_eq!(reader.view().row, 0);
}

#[test]
fn reopening_search_rejects_the_previous_search_completion() {
    let mut reader = support::reader();
    let first = start(&mut reader, "Line 20");
    reader.handle_key(InputKey::Escape);
    let second = start(&mut reader, "Line 30");
    reader.search_completed(completed(&first, 0));
    assert!(reader.search_selection().is_none());
    assert_eq!(reader.view().row, 0);
    reader.search_completed(completed(&second, 0));
    let selection = reader
        .search_selection()
        .expect("The current search must complete");
    assert!(selection.matches.hits[selection.selected].row > 0);
}

#[test]
fn canceling_search_discards_input_deferred_during_reflow() {
    let mut reader = support::reader();
    let job = start(&mut reader, "Line");
    reader.search_completed(completed(&job, 0));
    let request = reader
        .resize(ers::app::Viewport {
            columns: 30,
            rows: 20,
        })
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Load(request) => Some(request),
            _ => None,
        })
        .expect("Resizing search results must request reflow");
    reader.handle_key(InputKey::Character('b'));
    reader.handle_key(InputKey::Escape);
    let document = std::sync::Arc::new(ers::document::RenderedDocument::new(
        reader.view().rendered.index,
        reader.view().rendered.parsed.clone(),
        request.width,
    ));
    reader.accept_loaded(document, request);
    assert!(matches!(reader.view().mode, ReaderMode::Reading));
}

#[test]
fn invalid_patterns_preserve_the_editable_draft() {
    let mut reader = support::reader();
    let job = start(&mut reader, "[");
    reader.search_completed(SearchResult {
        request_id: job.request_id,
        result: Err("Regex error: missing closing bracket".into()),
    });
    assert!(reader.view().status.contains("Regex error"));
    let retry = reader.handle_key(InputKey::Enter);
    assert!(
        retry
            .iter()
            .any(|effect| matches!(effect, ReaderEffect::Search(job) if job.pattern == "["))
    );
}

#[test]
fn next_match_input_waits_for_the_current_search_result() {
    let mut reader = support::reader();
    let job = start(&mut reader, "Line");
    assert!(reader.handle_key(InputKey::Character('n')).is_empty());
    let effects = reader.search_completed(completed(&job, 0));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, ReaderEffect::Search(job) if job.after_hit == Some(0)))
    );
}

#[test]
fn overflowing_search_input_reports_ignored_keys_immediately() {
    let mut reader = support::reader();
    start(&mut reader, "Line");
    for _ in 0..4097 {
        reader.handle_key(InputKey::Character('j'));
    }
    assert!(reader.view().status.contains("input was ignored"));
}

#[test]
fn overflowing_search_input_is_reported_after_results_arrive() {
    let mut reader = support::reader();
    let job = start(&mut reader, "Line");
    for _ in 0..4097 {
        reader.handle_key(InputKey::Character('j'));
    }
    reader.search_completed(completed(&job, 0));
    assert!(reader.view().status.contains("input was ignored"));
}

#[test]
fn overflow_notice_survives_chained_searches() {
    let mut reader = support::reader();
    let first = start(&mut reader, "Line");
    reader.handle_key(InputKey::Character('n'));
    for _ in 0..4096 {
        reader.handle_key(InputKey::Character('j'));
    }
    let pending = reader
        .search_completed(completed(&first, 0))
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Search(job) => Some(job),
            _ => None,
        })
        .expect("A buffered next-match key must start another search");
    assert!(reader.view().status.contains("input was ignored"));
    reader.search_completed(completed(&pending, 1));
    assert!(reader.view().status.contains("input was ignored"));
}

#[test]
fn rejected_search_command_does_not_leave_a_numeric_prefix() {
    let mut reader = support::reader();
    let job = start(&mut reader, "Line");
    for _ in 0..4095 {
        reader.handle_key(InputKey::Character('j'));
    }
    reader.handle_key(InputKey::Character('5'));
    reader.handle_key(InputKey::Character('n'));
    assert!(reader.search_completed(completed(&job, 0)).is_empty());
    let effects = reader.handle_key(InputKey::Character('n'));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, ReaderEffect::Search(job) if job.count == 1))
    );
}

#[test]
fn failed_search_reports_discarded_queued_input() {
    let mut reader = support::reader();
    let job = start(&mut reader, "Line");
    reader.handle_key(InputKey::Character('n'));
    reader.search_completed(SearchResult {
        request_id: job.request_id,
        result: Err("Search failed".into()),
    });
    assert!(reader.view().status.contains("Search failed"));
    assert!(reader.view().status.contains("input was ignored"));
}

#[test]
fn failed_search_does_not_replay_stale_input_after_a_retry() {
    let mut reader = support::reader();
    let first = start(&mut reader, "Line");
    reader.search_completed(completed(&first, 0));
    let pending = reader
        .handle_key(InputKey::Character('n'))
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Search(job) => Some(job),
            _ => None,
        })
        .expect("Moving to the next match must start a search");
    reader.handle_key(InputKey::Character('n'));
    reader.search_completed(SearchResult {
        request_id: pending.request_id,
        result: Err("Search failed".into()),
    });
    let retry = reader
        .handle_key(InputKey::Character('n'))
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Search(job) => Some(job),
            _ => None,
        })
        .expect("The user must be able to retry the next match");
    let effects = reader.search_completed(completed(&retry, 1));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, ReaderEffect::Search(_)))
    );
}

#[test]
fn late_search_results_do_not_close_an_unrelated_dialog() {
    let mut reader = support::reader();
    let job = start(&mut reader, "Line");
    reader.handle_key(InputKey::Escape);
    reader.handle_key(InputKey::Character('M'));
    reader.search_completed(completed(&job, 0));
    assert!(matches!(reader.view().mode, ReaderMode::Dialog(_)));
}

#[test]
fn search_cancel_button_closes_the_editor() {
    let mut reader = support::reader();
    reader.handle_key(InputKey::Character('/'));
    reader.handle_key(InputKey::Tab);
    reader.handle_key(InputKey::Tab);
    reader.handle_key(InputKey::Enter);
    assert!(matches!(reader.view().mode, ReaderMode::Reading));
}

#[test]
fn typed_search_patterns_remain_within_the_input_limit() {
    let mut reader = support::reader();
    reader.handle_key(InputKey::Character('/'));
    for _ in 0..4200 {
        reader.handle_key(InputKey::Character('x'));
    }
    let ReaderMode::Search(mode) = reader.view().mode else {
        panic!("Expected the pattern editor");
    };
    let ers::app::modes::search::SearchMode::Editing(editor) = mode.as_ref() else {
        panic!("Expected an editable pattern");
    };
    assert!(editor.text().len() <= SearchLimits::default().pattern_bytes_max);
}

#[test]
fn failed_reflow_releases_a_completed_search_result() {
    let mut reader = support::reader();
    let job = start(&mut reader, "Line 20");
    reader.resize(ers::app::Viewport {
        columns: 30,
        rows: 20,
    });
    reader.search_completed(completed(&job, 0));
    assert!(reader.search_selection().is_none());
    reader.load_failed("Resize failed".into());
    assert!(
        reader.search_selection().is_some(),
        "A completed search must not remain pending after reflow fails"
    );
    assert!(reader.view().status.contains("Resize failed"));
}

#[test]
fn completed_search_jumps_are_available_in_navigation_history() {
    let mut reader = support::reader();
    let origin = reader.view_position();
    let job = start(&mut reader, "Line 40");
    reader.search_completed(completed(&job, 0));
    reader.handle_key(InputKey::Escape);
    let effects = reader.handle_key(InputKey::Character('\u{0f}'));
    assert!(effects.iter().any(|effect| matches!(effect, ReaderEffect::Load(request) if request.position.as_ref() == Some(&origin))));
}
