#[path = "support/annotations.rs"]
mod support;

use std::sync::Arc;

use ers::app::{ReaderEffect, ReaderModel, Viewport};
use ers::configuration::InputKey;

#[path = "support/reader.rs"]
mod reader_support;
use reader_support::reader;

fn annotated_reader(note: &str) -> ReaderModel {
    let mut annotation = support::annotation();
    annotation.anchor = ers::annotations::TextAnchor::new("first.xhtml", 0, "Line 0").unwrap();
    annotation.note = note.into();
    let mut builder = reader_support::ReaderBuilder::default();
    builder.annotations.push(annotation);
    builder.build()
}

#[test]
fn note_viewer_does_not_accumulate_scroll_beyond_the_last_page() {
    use ers::app::ReaderMode;
    use ers::app::modes::annotations::AnnotationMode;
    let mut reader = annotated_reader(&"A note line\n".repeat(100));
    reader.handle_key(InputKey::Character('a'));
    reader.handle_key(InputKey::Character('f'));
    for _ in 0..200 {
        reader.handle_key(InputKey::Character('j'));
    }
    let ReaderMode::Annotations(AnnotationMode::Viewing(viewer)) = reader.view().mode else {
        panic!("Expected the note viewer");
    };
    let bottom = viewer.scroll;
    for _ in 0..100 {
        reader.handle_key(InputKey::Character('j'));
    }
    let ReaderMode::Annotations(AnnotationMode::Viewing(viewer)) = reader.view().mode else {
        panic!("Expected the note viewer");
    };
    assert_eq!(viewer.scroll, bottom);
    reader.handle_key(InputKey::Character('k'));
    let ReaderMode::Annotations(AnnotationMode::Viewing(viewer)) = reader.view().mode else {
        panic!("Expected the note viewer");
    };
    assert_eq!(viewer.scroll, bottom - 1);
}

#[test]
fn interrupting_a_note_requires_confirmation_before_discarding_the_draft() {
    use ers::app::ReaderMode;
    use ers::app::modes::annotations::AnnotationMode;
    let mut reader = reader();
    for key in ['v', ' ', 'w', 'n'] {
        reader.handle_key(InputKey::Character(key));
    }
    reader.handle_paste("Keep this unsaved draft");
    assert!(reader.handle_key(InputKey::Character('\u{03}')).is_empty());
    assert!(matches!(
        reader.view().mode,
        ReaderMode::Annotations(AnnotationMode::QuitNote(_))
    ));
    reader.handle_key(InputKey::Character('n'));
    let ReaderMode::Annotations(AnnotationMode::Editing(editor)) = reader.view().mode else {
        panic!("Canceled quit must retain the editor");
    };
    assert_eq!(editor.state.text(), "Keep this unsaved draft");
    reader.handle_key(InputKey::Character('\u{03}'));
    assert!(
        reader
            .handle_key(InputKey::Character('y'))
            .iter()
            .any(|effect| matches!(effect, ReaderEffect::Quit(_)))
    );
    assert!(reader.view().info.annotations.is_empty());
}

#[test]
fn selection_save_emits_an_inclusive_annotation_draft() {
    use ers::annotations::AnnotationCommand;
    let mut reader = reader();
    for key in ['v', ' ', 'w'] {
        reader.handle_key(InputKey::Character(key));
    }
    let effects = reader.handle_key(InputKey::Enter);
    let draft = effects
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Annotation(job) => match job.command {
                AnnotationCommand::Create(draft) => Some(draft),
                _ => None,
            },
            _ => None,
        })
        .expect("A completed selection must request a durable annotation save");
    assert!(draft.anchor.quote().starts_with("Line "));
    assert!(
        draft
            .anchor
            .resolve(&reader.view().rendered.parsed.source)
            .is_some()
    );
}

#[test]
fn canceling_a_note_does_not_emit_a_database_mutation() {
    use ers::app::ReaderMode;
    use ers::app::modes::annotations::AnnotationMode;
    let mut reader = reader();
    for key in ['v', ' ', 'w', 'n'] {
        reader.handle_key(InputKey::Character(key));
    }
    reader.handle_paste("Unsaved draft\nSecond line");
    assert!(reader.handle_key(InputKey::Escape).is_empty());
    assert!(matches!(
        reader.view().mode,
        ReaderMode::Annotations(AnnotationMode::Selecting(_))
    ));
    assert!(reader.view().info.annotations.is_empty());
}

#[test]
fn pasted_commands_do_not_escape_the_note_editor() {
    use ers::app::ReaderMode;
    use ers::app::modes::annotations::AnnotationMode;
    let mut reader = reader();
    for key in ['v', ' ', 'w', 'n'] {
        reader.handle_key(InputKey::Character(key));
    }
    reader.handle_paste("q\nv\nO\n👩‍🔬");
    let ReaderMode::Annotations(AnnotationMode::Editing(editor)) = reader.view().mode else {
        panic!("Paste must remain in the note editor");
    };
    assert_eq!(editor.state.text(), "q\nv\nO\n👩‍🔬");
    assert!(!reader.view().quitting);
}

#[test]
fn note_save_defers_following_commands_until_database_acknowledgement() {
    use ers::annotations::{AnnotationCommand, AnnotationResult};
    use ers::app::ReaderMode;
    let mut reader = reader();
    for key in ['v', ' ', 'w', 'n'] {
        reader.handle_key(InputKey::Character(key));
    }
    reader.handle_paste("Committed note");
    reader.handle_key(InputKey::Tab);
    let effects = reader.handle_key(InputKey::Enter);
    let job = effects
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Annotation(job) => Some(job),
            _ => None,
        })
        .unwrap();
    assert!(matches!(job.command, AnnotationCommand::Create(_)));
    reader.handle_key(InputKey::Character('v'));
    assert!(!matches!(reader.view().mode, ReaderMode::Reading));
    reader.annotation_completed(AnnotationResult {
        request_id: job.request_id,
        annotations: Vec::new(),
        saved_id: None,
        error: None,
        export: ers::annotations::ExportState::default(),
    });
    assert!(matches!(reader.view().mode, ReaderMode::Reading));
}

fn pending_annotation_save() -> (ReaderModel, ers::annotations::AnnotationJob) {
    let mut reader = reader();
    for key in ['v', ' ', 'w'] {
        reader.handle_key(InputKey::Character(key));
    }
    let job = reader
        .handle_key(InputKey::Enter)
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Annotation(job) => Some(job),
            _ => None,
        })
        .expect("A selection must request an annotation save");
    (reader, job)
}

fn annotation_save_result(
    request_id: u64,
    error: Option<&str>,
) -> ers::annotations::AnnotationResult {
    ers::annotations::AnnotationResult {
        request_id,
        annotations: Vec::new(),
        saved_id: None,
        error: error.map(str::to_owned),
        export: ers::annotations::ExportState::default(),
    }
}

fn finish_annotation_save(reader: &mut ReaderModel, job: ers::annotations::AnnotationJob) {
    reader.annotation_completed(annotation_save_result(job.request_id, None));
}

#[test]
fn stale_annotation_completion_preserves_the_failed_save_and_buffered_input() {
    let (mut reader, job) = pending_annotation_save();
    reader.handle_key(InputKey::Character('v'));
    reader.annotation_completed(annotation_save_result(
        job.request_id,
        Some("Database busy"),
    ));
    reader.annotation_completed(annotation_save_result(job.request_id + 1, None));
    assert!(reader.view().status.contains("Save failed: Database busy"));
    let retry = reader.handle_key(InputKey::Enter);
    assert!(retry.iter().any(|effect| matches!(effect, ReaderEffect::Annotation(retry) if retry.request_id == job.request_id)));
    finish_annotation_save(&mut reader, job);
    assert!(matches!(reader.view().mode, ers::app::ReaderMode::Reading));
}

#[test]
fn failed_annotation_save_returns_to_the_unsaved_note_and_discards_buffered_keys() {
    let mut reader = reader();
    for key in ['v', ' ', 'w', 'n'] {
        reader.handle_key(InputKey::Character(key));
    }
    reader.handle_paste("Uncommitted draft");
    reader.handle_key(InputKey::Tab);
    let job = reader
        .handle_key(InputKey::Enter)
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Annotation(job) => Some(job),
            _ => None,
        })
        .expect("Saving the note must emit an annotation job");
    reader.handle_key(InputKey::Character('v'));
    reader.annotation_completed(annotation_save_result(
        job.request_id,
        Some("Database busy"),
    ));
    assert!(reader.handle_key(InputKey::Escape).is_empty());
    let ers::app::ReaderMode::Annotations(ers::app::modes::annotations::AnnotationMode::Editing(
        editor,
    )) = reader.view().mode
    else {
        panic!("The failed save must return to the note editor");
    };
    assert_eq!(editor.state.text(), "Uncommitted draft");
}

#[test]
fn pending_annotation_save_limits_queued_keys() {
    let (mut reader, job) = pending_annotation_save();
    let initial_progress = reader.view().show_progress;
    reader.handle_key(InputKey::Character('v'));
    for _ in 0..4096 {
        reader.handle_key(InputKey::Character('s'));
    }
    finish_annotation_save(&mut reader, job);
    assert_ne!(reader.view().show_progress, initial_progress);
}

#[test]
fn pending_annotation_save_reports_ignored_input_after_completion() {
    let (mut reader, job) = pending_annotation_save();
    for _ in 0..4097 {
        reader.handle_key(InputKey::Character('s'));
    }
    finish_annotation_save(&mut reader, job);
    assert!(reader.view().status.contains("input was ignored"));
}

#[test]
fn ignored_annotation_input_is_reported_after_a_successful_retry() {
    let (mut reader, job) = pending_annotation_save();
    for _ in 0..4097 {
        reader.handle_key(InputKey::Character('s'));
    }
    reader.annotation_completed(ers::annotations::AnnotationResult {
        request_id: job.request_id,
        annotations: Vec::new(),
        saved_id: None,
        error: Some("Database busy".into()),
        export: ers::annotations::ExportState::default(),
    });
    assert!(
        reader
            .handle_key(InputKey::Enter)
            .iter()
            .any(|effect| matches!(effect, ReaderEffect::Annotation(_)))
    );
    finish_annotation_save(&mut reader, job);
    assert!(reader.view().status.contains("input was ignored"));
}

#[test]
fn ignored_pending_input_is_reported_after_a_concurrent_reflow() {
    let (mut reader, job) = pending_annotation_save();
    reader.handle_key(InputKey::Character('v'));
    reader.handle_key(InputKey::Character('b'));
    for _ in 0..4095 {
        reader.handle_key(InputKey::Character('x'));
    }
    let request = reader
        .resize(Viewport {
            columns: 30,
            rows: 20,
        })
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Load(request) => Some(request),
            _ => None,
        })
        .expect("Resizing during a save must request reflow");
    finish_annotation_save(&mut reader, job);
    finish_reflow(&mut reader, request);
    assert!(reader.view().status.contains("input was ignored"));
}

#[test]
fn ignored_input_notice_is_not_duplicated_after_concurrent_reflow() {
    let (mut reader, job) = pending_annotation_save();
    for _ in 0..4097 {
        reader.handle_key(InputKey::Character('s'));
    }
    let request = reader
        .resize(Viewport {
            columns: 30,
            rows: 20,
        })
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Load(request) => Some(request),
            _ => None,
        })
        .expect("Resizing during a save must request reflow");
    finish_annotation_save(&mut reader, job);
    finish_reflow(&mut reader, request);
    assert_eq!(
        reader
            .view()
            .status
            .matches("Some input was ignored")
            .count(),
        1
    );
}

#[test]
fn ignored_pending_input_is_reported_after_a_failed_reflow() {
    let (mut reader, job) = pending_annotation_save();
    for _ in 0..4097 {
        reader.handle_key(InputKey::Character('s'));
    }
    reader.resize(Viewport {
        columns: 30,
        rows: 20,
    });
    finish_annotation_save(&mut reader, job);
    reader.load_failed("Reflow failed".into());
    assert!(reader.view().status.contains("Reflow failed"));
    assert!(reader.view().status.contains("input was ignored"));
}

#[test]
fn pending_annotation_save_rejects_an_oversized_paste() {
    let (mut reader, job) = pending_annotation_save();
    reader.handle_key(InputKey::Character('v'));
    reader.handle_key(InputKey::Character('b'));
    reader.handle_paste(&"x".repeat(8 * 1024 * 1024 + 1));
    finish_annotation_save(&mut reader, job);
    let ers::app::ReaderMode::BookmarkPrompt(value) = reader.view().mode else {
        panic!("Expected a bookmark prompt");
    };
    assert!(value.is_empty());
}

#[test]
fn pending_annotation_save_limits_combined_paste_bytes() {
    let (mut reader, job) = pending_annotation_save();
    reader.handle_key(InputKey::Character('v'));
    reader.handle_key(InputKey::Character('b'));
    reader.handle_paste(&"a".repeat(4 * 1024 * 1024));
    reader.handle_paste(&"b".repeat(4 * 1024 * 1024));
    reader.handle_paste("x");
    finish_annotation_save(&mut reader, job);
    let ers::app::ReaderMode::BookmarkPrompt(value) = reader.view().mode else {
        panic!("Expected a bookmark prompt");
    };
    assert_eq!(value.len(), 8 * 1024 * 1024);
}

#[test]
fn repeated_movement_updates_the_current_viewport_without_draw_steps() {
    let mut reader = reader();
    for _ in 0..60 {
        reader.handle_key(InputKey::Character('j'));
    }
    assert_eq!(reader.view().row, 60);
}

#[test]
fn numeric_prefix_applies_to_the_next_movement() {
    let mut reader = reader();
    for key in ['1', '2', 'j'] {
        reader.handle_key(InputKey::Character(key));
    }
    assert_eq!(reader.view().row, 12);
}

fn request_reflow(reader: &mut ReaderModel) -> ers::reading::LoadRequest {
    reader
        .handle_key(InputKey::Character('+'))
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Load(request) => Some(request),
            _ => None,
        })
        .expect("A width change must request reflow")
}

fn finish_reflow(reader: &mut ReaderModel, request: ers::reading::LoadRequest) {
    let document = Arc::new(ers::document::RenderedDocument::new(
        reader.view().rendered.index,
        reader.view().rendered.parsed.clone(),
        request.width,
    ));
    reader.accept_loaded(document, request);
}

fn overloaded_reflow() -> ReaderModel {
    let mut reader = reader();
    let request = request_reflow(&mut reader);
    for _ in 0..2048 {
        reader.handle_key(InputKey::Character('j'));
        reader.handle_key(InputKey::Character('k'));
    }
    reader.handle_key(InputKey::Character('j'));
    finish_reflow(&mut reader, request);
    reader
}

#[test]
fn excess_alternating_input_during_reflow_is_not_replayed() {
    assert_eq!(overloaded_reflow().view().row, 0);
}

#[test]
fn ignored_input_is_reported_after_reflow() {
    assert!(
        overloaded_reflow()
            .view()
            .status
            .contains("input was ignored")
    );
}

#[test]
fn repeated_input_during_reflow_respects_the_queue_limit() {
    let mut reader = reader();
    let request = request_reflow(&mut reader);
    for _ in 0..4097 {
        reader.handle_key(InputKey::Character('j'));
    }
    let html = "<p>Long synthetic paragraph for a scrollable chapter.</p>".repeat(4200);
    let parsed = ers::document::text::parse_document("first.xhtml", &html).unwrap();
    let document = Arc::new(ers::document::RenderedDocument::new(
        0,
        parsed,
        request.width,
    ));
    reader.accept_loaded(document, request);
    assert_eq!(reader.view().row, 4096);
}

#[test]
fn rejected_prefixed_input_does_not_affect_the_next_command() {
    let mut reader = reader();
    let request = request_reflow(&mut reader);
    for _ in 0..2047 {
        reader.handle_key(InputKey::Character('j'));
        reader.handle_key(InputKey::Character('k'));
    }
    for key in ['1', '2', 't'] {
        reader.handle_key(InputKey::Character(key));
    }
    finish_reflow(&mut reader, request);
    reader.handle_key(InputKey::Character('j'));
    assert_eq!(reader.view().row, 1);
}

#[test]
fn oversized_queued_paste_does_not_change_the_note_draft() {
    use ers::app::ReaderMode;
    use ers::app::modes::annotations::AnnotationMode;

    let mut reader = reader();
    let request = request_reflow(&mut reader);
    for key in ['v', ' ', 'w', 'n'] {
        reader.handle_key(InputKey::Character(key));
    }
    reader.handle_paste(&"x".repeat(8 * 1024 * 1024 + 1));
    finish_reflow(&mut reader, request);
    let ReaderMode::Annotations(AnnotationMode::Editing(editor)) = reader.view().mode else {
        panic!("Expected the note editor");
    };
    assert!(editor.state.text().is_empty());
}

fn note_editor_reflow() -> (ReaderModel, ers::reading::LoadRequest) {
    let mut reader = reader();
    for key in ['v', ' ', 'w', 'n'] {
        reader.handle_key(InputKey::Character(key));
    }
    let request = reader
        .resize(Viewport {
            columns: 30,
            rows: 20,
        })
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Load(request) => Some(request),
            _ => None,
        })
        .expect("Resizing the note editor must request reflow");
    (reader, request)
}

#[test]
fn deferred_note_input_during_reflow_respects_the_queue_limit() {
    use ers::app::ReaderMode;
    use ers::app::modes::annotations::AnnotationMode;

    let (mut reader, request) = note_editor_reflow();
    for _ in 0..4097 {
        reader.handle_key(InputKey::Character('x'));
    }
    finish_reflow(&mut reader, request);
    let ReaderMode::Annotations(AnnotationMode::Editing(editor)) = reader.view().mode else {
        panic!("Expected the note editor");
    };
    assert_eq!(editor.state.text().len(), 4096);
}

#[test]
fn oversized_deferred_paste_does_not_change_the_note_draft() {
    use ers::app::ReaderMode;
    use ers::app::modes::annotations::AnnotationMode;

    let (mut reader, request) = note_editor_reflow();
    reader.handle_paste(&"x".repeat(8 * 1024 * 1024 + 1));
    finish_reflow(&mut reader, request);
    let ReaderMode::Annotations(AnnotationMode::Editing(editor)) = reader.view().mode else {
        panic!("Expected the note editor");
    };
    assert!(editor.state.text().is_empty());
}

#[test]
fn width_change_requests_a_canonical_viewport_anchor() {
    let mut reader = reader();
    for _ in 0..15 {
        reader.handle_key(InputKey::Character('j'));
    }
    let before = reader.position();
    let effects = reader.handle_key(InputKey::Character('+'));
    let request = effects
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Load(request) => Some(request),
            _ => None,
        })
        .unwrap();
    assert_eq!(request.position.unwrap().text.offset, before.offset);
    assert_eq!(request.width, 41);
}

#[test]
fn quit_returns_a_durable_save_request() {
    let mut reader = reader();
    reader.handle_key(InputKey::Character('j'));
    let effects = reader.handle_key(InputKey::Character('q'));
    assert!(
        effects
            .into_iter()
            .any(|effect| matches!(effect, ReaderEffect::Quit(_)))
    );
}

#[test]
fn chapter_navigation_requests_the_next_document() {
    let mut reader = reader();
    let effects = reader.handle_key(InputKey::Character('L'));
    assert!(
        effects
            .into_iter()
            .any(|effect| matches!(effect, ReaderEffect::Load(request) if request.index == 1))
    );
}

#[test]
fn resize_preserves_the_current_source_position() {
    let mut reader = reader();
    for _ in 0..20 {
        reader.handle_key(InputKey::Character('j'));
    }
    let before = reader.position();
    let effects = reader.resize(Viewport {
        columns: 36,
        rows: 15,
    });
    assert!(effects.into_iter().any(|effect| matches!(effect, ReaderEffect::Load(request) if request.position.as_ref().is_some_and(|position| position.text.offset == before.offset))));
}
