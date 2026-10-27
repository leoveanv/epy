#[path = "support/annotations.rs"]
mod annotation_support;
#[path = "support/reader.rs"]
mod reader_support;

use ers::annotations::{
    ExportState, ReassociationCommand, ReassociationJob, ReassociationReply, ReassociationReport,
    ReassociationResult, ReassociationSource,
};
use ers::app::modes::annotations::AnnotationMode;
use ers::app::modes::reassociation::ReassociationMode;
use ers::app::{ReaderEffect, ReaderMode, ReaderModel};
use ers::configuration::InputKey;

fn request(effects: Vec<ReaderEffect>) -> ReassociationJob {
    let [ReaderEffect::Reassociation(job)] = effects.as_slice() else {
        panic!("Expected a reassociation request");
    };
    job.clone()
}

fn source() -> ReassociationSource {
    ReassociationSource {
        book: annotation_support::book(),
        annotation_count: 3,
    }
}

fn enter_picker(reader: &mut ReaderModel) {
    let job = request(reader.handle_key(InputKey::Character('r')));
    assert!(matches!(job.command, ReassociationCommand::Sources));
    reader.reassociation_completed(ReassociationResult {
        request_id: job.request_id,
        reply: Ok(ReassociationReply::Sources(vec![source()])),
    });
}

fn open_picker() -> ReaderModel {
    let mut reader = reader_support::reader();
    reader.handle_key(InputKey::Character('a'));
    enter_picker(&mut reader);
    reader
}

fn copy_request(reader: &mut ReaderModel) -> ReassociationJob {
    reader.handle_key(InputKey::Enter);
    request(reader.handle_key(InputKey::Character('y')))
}

fn copied_report() -> ReassociationReport {
    let first = annotation_support::annotation();
    let mut copied = first.clone();
    copied.id = "00000000-0000-4000-8000-000000000003".into();
    ReassociationReport {
        annotations: vec![first, copied.clone()],
        copied_count: 1,
        skipped_count: 2,
        first_copied_id: Some(copied.id),
        export: ExportState {
            running: true,
            ..ExportState::default()
        },
    }
}

#[test]
fn interrupting_the_reassociation_picker_requests_a_durable_quit() {
    let mut reader = open_picker();
    assert!(
        reader
            .handle_key(InputKey::Character('\u{03}'))
            .iter()
            .any(|effect| matches!(effect, ReaderEffect::Quit(_)))
    );
}

#[test]
fn copied_reassociation_focuses_the_copied_annotation() {
    let mut reader = open_picker();
    let job = copy_request(&mut reader);
    let report = copied_report();
    let focused = report.first_copied_id.clone();
    reader.reassociation_completed(ReassociationResult {
        request_id: job.request_id,
        reply: Ok(ReassociationReply::Copied(report)),
    });
    assert_eq!(reader.focused_annotation(), focused.as_deref());
    assert!(matches!(
        reader.view().mode,
        ReaderMode::Annotations(AnnotationMode::List { selected: 1 })
    ));
}

#[test]
fn reassociation_without_a_copied_anchor_restores_the_previous_selection() {
    let mut report = copied_report();
    report.first_copied_id = None;
    let builder = reader_support::ReaderBuilder {
        annotations: report.annotations.clone(),
        ..reader_support::ReaderBuilder::default()
    };
    let mut reader = builder.build();
    reader.handle_key(InputKey::Character('a'));
    reader.handle_key(InputKey::Character('j'));
    enter_picker(&mut reader);
    let job = copy_request(&mut reader);
    reader.reassociation_completed(ReassociationResult {
        request_id: job.request_id,
        reply: Ok(ReassociationReply::Copied(report)),
    });
    assert!(matches!(
        reader.view().mode,
        ReaderMode::Annotations(AnnotationMode::List { selected: 1 })
    ));
}

#[test]
fn copied_reassociation_updates_export_metadata() {
    let mut reader = open_picker();
    let job = copy_request(&mut reader);
    reader.reassociation_completed(ReassociationResult {
        request_id: job.request_id,
        reply: Ok(ReassociationReply::Copied(copied_report())),
    });
    assert!(reader.export_state().running);
}

#[test]
fn copied_reassociation_replays_buffered_navigation() {
    let mut reader = open_picker();
    let job = copy_request(&mut reader);
    reader.handle_key(InputKey::Character('k'));
    reader.reassociation_completed(ReassociationResult {
        request_id: job.request_id,
        reply: Ok(ReassociationReply::Copied(copied_report())),
    });
    assert!(matches!(
        reader.view().mode,
        ReaderMode::Annotations(AnnotationMode::List { selected: 0 })
    ));
}

#[test]
fn pending_reassociation_limits_queued_keys() {
    let mut reader = reader_support::reader();
    reader.handle_key(InputKey::Character('a'));
    let job = request(reader.handle_key(InputKey::Character('r')));
    for _ in 0..4096 {
        reader.handle_key(InputKey::Character('j'));
    }
    reader.handle_key(InputKey::Enter);
    reader.reassociation_completed(ReassociationResult {
        request_id: job.request_id,
        reply: Ok(ReassociationReply::Sources(vec![source()])),
    });
    assert!(
        matches!(reader.view().mode, ReaderMode::Reassociation(mode) if matches!(mode.as_ref(), ReassociationMode::Sources(_)))
    );
}

#[test]
fn pending_reassociation_reports_ignored_input_after_completion() {
    let mut reader = reader_support::reader();
    reader.handle_key(InputKey::Character('a'));
    let job = request(reader.handle_key(InputKey::Character('r')));
    for _ in 0..4097 {
        reader.handle_key(InputKey::Character('j'));
    }
    reader.reassociation_completed(ReassociationResult {
        request_id: job.request_id,
        reply: Ok(ReassociationReply::Sources(vec![source()])),
    });
    assert!(reader.view().status.contains("input was ignored"));
}

#[test]
fn pending_reassociation_rejects_an_oversized_paste() {
    let mut reader = reader_support::reader();
    reader.handle_key(InputKey::Character('a'));
    let job = request(reader.handle_key(InputKey::Character('r')));
    reader.handle_key(InputKey::Escape);
    reader.handle_key(InputKey::Escape);
    reader.handle_key(InputKey::Character('b'));
    reader.handle_paste(&"x".repeat(8 * 1024 * 1024 + 1));
    reader.reassociation_completed(ReassociationResult {
        request_id: job.request_id,
        reply: Ok(ReassociationReply::Sources(vec![source()])),
    });
    let ReaderMode::BookmarkPrompt(value) = reader.view().mode else {
        panic!("Expected a bookmark prompt");
    };
    assert!(value.is_empty());
}

#[test]
fn stale_reassociation_completion_preserves_the_failed_request_and_buffered_input() {
    let mut reader = reader_support::reader();
    reader.handle_key(InputKey::Character('a'));
    let job = request(reader.handle_key(InputKey::Character('r')));
    reader.handle_key(InputKey::Character('j'));
    reader.reassociation_completed(ReassociationResult {
        request_id: job.request_id,
        reply: Err("Database busy".into()),
    });
    reader.reassociation_completed(ReassociationResult {
        request_id: job.request_id + 1,
        reply: Ok(ReassociationReply::Sources(vec![source(), source()])),
    });
    assert!(
        reader
            .view()
            .status
            .contains("Reassociation failed: Database busy")
    );
    let retry = request(reader.handle_key(InputKey::Enter));
    assert_eq!(retry.request_id, job.request_id);
    reader.reassociation_completed(ReassociationResult {
        request_id: job.request_id,
        reply: Ok(ReassociationReply::Sources(vec![source(), source()])),
    });
    assert!(
        matches!(reader.view().mode, ReaderMode::Reassociation(mode) if matches!(mode.as_ref(), ReassociationMode::Sources(picker) if picker.selected == 1))
    );
}

#[test]
fn reassociation_requires_confirmation_after_source_selection() {
    let mut reader = open_picker();
    assert!(reader.handle_key(InputKey::Enter).is_empty());
    assert!(
        matches!(reader.view().mode, ReaderMode::Reassociation(mode) if matches!(mode.as_ref(), ReassociationMode::Confirm(_)))
    );
    let job = request(reader.handle_key(InputKey::Character('y')));
    assert!(
        matches!(job.command, ReassociationCommand::Copy(book_id) if book_id == source().book.id)
    );
}

#[test]
fn failed_reassociation_returns_to_its_confirmation() {
    let mut reader = open_picker();
    reader.handle_key(InputKey::Enter);
    let copy = request(reader.handle_key(InputKey::Character('y')));
    reader.handle_key(InputKey::Character('j'));
    reader.reassociation_completed(ReassociationResult {
        request_id: copy.request_id,
        reply: Err("Database busy".into()),
    });
    assert!(reader.handle_key(InputKey::Escape).is_empty());
    assert!(
        matches!(reader.view().mode, ReaderMode::Reassociation(mode) if matches!(mode.as_ref(), ReassociationMode::Confirm(picker) if picker.selected == 0))
    );
}

#[test]
fn canceled_reassociation_returns_to_the_source_picker() {
    let mut reader = open_picker();
    reader.handle_key(InputKey::Enter);
    assert!(reader.handle_key(InputKey::Character('n')).is_empty());
    assert!(
        matches!(reader.view().mode, ReaderMode::Reassociation(mode) if matches!(mode.as_ref(), ReassociationMode::Sources(_)))
    );
    assert!(reader.view().info.annotations.is_empty());
}

#[test]
fn pasted_yes_does_not_confirm_a_reassociation() {
    let mut reader = open_picker();
    reader.handle_key(InputKey::Enter);
    reader.handle_paste("y");
    assert!(
        matches!(reader.view().mode, ReaderMode::Reassociation(mode) if matches!(mode.as_ref(), ReassociationMode::Confirm(_)))
    );
}

#[test]
fn absent_source_identities_leave_the_annotation_list_available() {
    let mut reader = reader_support::reader();
    reader.handle_key(InputKey::Character('a'));
    let job = request(reader.handle_key(InputKey::Character('r')));
    reader.reassociation_completed(ReassociationResult {
        request_id: job.request_id,
        reply: Ok(ReassociationReply::Sources(Vec::new())),
    });
    assert!(matches!(
        reader.view().mode,
        ReaderMode::Annotations(AnnotationMode::List { .. })
    ));
    assert!(reader.view().status.contains("No other annotated"));
}
