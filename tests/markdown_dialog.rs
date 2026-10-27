#[path = "support/reader.rs"]
mod support;

use ers::annotations::{
    Destination, ExportLink, ExportState, LinkCommand, LinkJob, LinkReply, LinkResult,
    OverwriteApproval,
};
use ers::app::modes::markdown_export::LinkMode;
use ers::app::{ReaderEffect, ReaderMode, ReaderModel};
use ers::configuration::InputKey;

fn job(effects: Vec<ReaderEffect>) -> LinkJob {
    let [ReaderEffect::Link(job)] = effects.as_slice() else {
        panic!("Expected one Markdown operation");
    };
    job.clone()
}

fn link_result(request_id: u64, reply: Result<LinkReply, String>) -> LinkResult {
    LinkResult { request_id, reply }
}

fn open_dialog(state: ExportState) -> ReaderModel {
    let mut reader = support::reader();
    let request = job(reader.handle_key(InputKey::Character('O')));
    assert!(matches!(request.command, LinkCommand::Status));
    reader.link_completed(LinkResult {
        request_id: request.request_id,
        reply: Ok(LinkReply::State(state)),
    });
    reader
}

fn proposed_destination(reader: &mut ReaderModel) {
    reader.handle_paste("/synthetic/My notes.md");
    reader.handle_key(InputKey::Tab);
    let inspect = job(reader.handle_key(InputKey::Enter));
    assert!(
        matches!(inspect.command, LinkCommand::Inspect(path) if path == "/synthetic/My notes.md")
    );
    reader.link_completed(LinkResult {
        request_id: inspect.request_id,
        reply: Ok(LinkReply::Destination(Destination {
            path: "/resolved/My notes.md".into(),
            exists: false,
        })),
    });
}

fn linked_state() -> ExportState {
    ExportState {
        link: Some(ExportLink {
            book_id: "00000000-0000-4000-8000-000000000001".into(),
            path: "/synthetic/Notes.md".into(),
            digest: None,
            revision: 1,
            exported_revision: 1,
        }),
        ..ExportState::default()
    }
}

#[test]
fn interrupting_the_markdown_dialog_requests_a_durable_quit() {
    let mut reader = open_dialog(linked_state());
    assert!(
        reader
            .handle_key(InputKey::Character('\u{03}'))
            .iter()
            .any(|effect| matches!(effect, ReaderEffect::Quit(_)))
    );
}

#[test]
fn stale_markdown_results_leave_the_current_operation_pending() {
    let mut reader = support::reader();
    let request = job(reader.handle_key(InputKey::Character('O')));
    let status = reader.view().status.to_owned();

    reader.link_completed(link_result(
        request.request_id + 1,
        Ok(LinkReply::State(ExportState::default())),
    ));

    assert_eq!(reader.view().status, status);
}

#[test]
fn failed_markdown_inspection_returns_to_the_path_draft() {
    let mut reader = open_dialog(ExportState::default());
    reader.handle_paste("/synthetic/Notes.md");
    reader.handle_key(InputKey::Tab);
    let inspect = job(reader.handle_key(InputKey::Enter));
    reader.handle_key(InputKey::Character('q'));
    reader.link_completed(link_result(
        inspect.request_id,
        Err("Permission denied".into()),
    ));
    assert!(reader.handle_key(InputKey::Escape).is_empty());
    let ReaderMode::Link(mode) = reader.view().mode else {
        panic!("The failed inspection must return to the link dialog");
    };
    let LinkMode::Editing(editor) = mode.as_ref() else {
        panic!("The failed inspection must restore the path editor");
    };
    assert_eq!(editor.text(), "/synthetic/Notes.md");
}

#[test]
fn retried_markdown_operation_replays_buffered_path_input() {
    let mut reader = support::reader();
    let request = job(reader.handle_key(InputKey::Character('O')));
    reader.handle_paste("/synthetic/Notes.md");
    reader.link_completed(link_result(request.request_id, Err("Database busy".into())));
    let retry = job(reader.handle_key(InputKey::Enter));
    reader.link_completed(link_result(
        retry.request_id,
        Ok(LinkReply::State(ExportState::default())),
    ));
    let ReaderMode::Link(mode) = reader.view().mode else {
        panic!("The link dialog must remain open");
    };
    let LinkMode::Editing(editor) = mode.as_ref() else {
        panic!("The successful retry must open the path editor");
    };
    assert_eq!(editor.text(), "/synthetic/Notes.md");
}

#[test]
fn pending_markdown_operation_limits_queued_keys() {
    let mut reader = support::reader();
    let request = job(reader.handle_key(InputKey::Character('O')));
    for _ in 0..4097 {
        reader.handle_key(InputKey::Character('x'));
    }
    reader.link_completed(LinkResult {
        request_id: request.request_id,
        reply: Ok(LinkReply::State(ExportState::default())),
    });
    let ReaderMode::Link(mode) = reader.view().mode else {
        panic!("Expected the path editor");
    };
    let LinkMode::Editing(editor) = mode.as_ref() else {
        panic!("Expected the path editor");
    };
    assert_eq!(editor.text().len(), 4096);
}

#[test]
fn pending_markdown_operation_reports_ignored_input_after_completion() {
    let mut reader = support::reader();
    let request = job(reader.handle_key(InputKey::Character('O')));
    for _ in 0..4097 {
        reader.handle_key(InputKey::Character('x'));
    }
    reader.link_completed(LinkResult {
        request_id: request.request_id,
        reply: Ok(LinkReply::State(ExportState::default())),
    });
    assert!(reader.view().status.contains("input was ignored"));
}

#[test]
fn pending_markdown_operation_rejects_an_oversized_paste() {
    let mut reader = support::reader();
    let request = job(reader.handle_key(InputKey::Character('O')));
    reader.handle_paste(&"x".repeat(8 * 1024 * 1024 + 1));
    reader.link_completed(LinkResult {
        request_id: request.request_id,
        reply: Ok(LinkReply::State(ExportState::default())),
    });
    let ReaderMode::Link(mode) = reader.view().mode else {
        panic!("Expected the path editor");
    };
    let LinkMode::Editing(editor) = mode.as_ref() else {
        panic!("Expected the path editor");
    };
    assert!(editor.text().is_empty());
}

#[test]
fn missing_markdown_requires_a_separate_creation_confirmation() {
    let mut reader = open_dialog(ExportState::default());
    proposed_destination(&mut reader);
    assert!(reader.handle_key(InputKey::Character('y')).is_empty());
    let create = job(reader.handle_key(InputKey::Character('y')));
    assert!(
        matches!(create.command, LinkCommand::Set { destination, create: true } if destination.path == "/resolved/My notes.md")
    );
}

#[test]
fn canceling_creation_retains_the_original_path_draft() {
    let mut reader = open_dialog(ExportState::default());
    proposed_destination(&mut reader);
    reader.handle_key(InputKey::Character('y'));
    assert!(reader.handle_key(InputKey::Character('n')).is_empty());
    let ReaderMode::Link(mode) = reader.view().mode else {
        panic!("Expected the link dialog");
    };
    let LinkMode::Editing(editor) = mode.as_ref() else {
        panic!("Expected the path draft");
    };
    assert_eq!(editor.text(), "/synthetic/My notes.md");
}

#[test]
fn multiline_path_paste_is_rejected_without_interpreting_commands() {
    let mut reader = open_dialog(ExportState::default());
    reader.handle_paste("/synthetic/Notes.md\nu\ny");
    let ReaderMode::Link(mode) = reader.view().mode else {
        panic!("Expected the link dialog");
    };
    let LinkMode::Editing(editor) = mode.as_ref() else {
        panic!("Expected the path editor");
    };
    assert!(editor.text().is_empty());
    assert!(reader.view().status.contains("single"));
}

#[test]
fn unlinking_requires_an_explicit_confirmation() {
    let mut reader = open_dialog(linked_state());
    assert!(reader.handle_key(InputKey::Character('u')).is_empty());
    reader.handle_paste("y");
    assert!(
        matches!(reader.view().mode, ReaderMode::Link(mode) if matches!(mode.as_ref(), LinkMode::Unlink))
    );
    let request = job(reader.handle_key(InputKey::Character('y')));
    assert!(matches!(request.command, LinkCommand::Unlink));
}

#[test]
fn stale_export_updates_cannot_replace_a_newer_link_revision() {
    let mut reader = support::reader();
    let mut current = linked_state();
    current.link.as_mut().unwrap().revision = 3;
    reader.export_updated(current.clone());
    reader.export_updated(linked_state());
    assert_eq!(reader.export_state().link, current.link);
}

#[test]
fn inspecting_link_metadata_does_not_acknowledge_a_failed_export() {
    let state = linked_state();
    let mut reader = support::reader();
    let mut failed = state.clone();
    failed.error = Some("External edits need confirmation".into());
    reader.export_updated(failed);
    let expected = reader.export_state().clone();
    let request = job(reader.handle_key(InputKey::Character('O')));
    reader.link_completed(LinkResult {
        request_id: request.request_id,
        reply: Ok(LinkReply::State(state)),
    });
    assert_eq!(reader.export_state().error, expected.error);
    assert!(reader.export_state().pending());
}

#[test]
fn clearing_the_path_draft_does_not_unlink_the_book() {
    let state = linked_state();
    let mut reader = open_dialog(state.clone());
    reader.handle_key(InputKey::Enter);
    assert!(reader.handle_key(InputKey::Character('\u{15}')).is_empty());
    let ReaderMode::Link(mode) = reader.view().mode else {
        panic!("Expected the path dialog");
    };
    let LinkMode::Editing(editor) = mode.as_ref() else {
        panic!("Expected the path draft");
    };
    assert!(editor.text().is_empty());
    assert_eq!(reader.export_state().link, state.link);
}

#[test]
fn pasted_yes_does_not_approve_overwriting_external_edits() {
    let state = linked_state();
    let mut reader = open_dialog(state.clone());
    let request = job(reader.handle_key(InputKey::Character('w')));
    assert!(matches!(request.command, LinkCommand::PrepareOverwrite));
    let approval = OverwriteApproval {
        link: state.link.unwrap(),
        file_digest: "a".repeat(64),
    };
    reader.link_completed(LinkResult {
        request_id: request.request_id,
        reply: Ok(LinkReply::Overwrite(approval.clone())),
    });
    reader.handle_paste("y");
    assert!(
        matches!(reader.view().mode, ReaderMode::Link(mode) if matches!(mode.as_ref(), LinkMode::Overwrite(_)))
    );
    let request = job(reader.handle_key(InputKey::Character('y')));
    assert!(matches!(request.command, LinkCommand::Overwrite(actual) if actual == approval));
}
