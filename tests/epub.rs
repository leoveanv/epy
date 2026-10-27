#[path = "support/epub.rs"]
mod support;

use ers::book::epub::EpubBook;
use ers::book::{BookMetadata, ResourceLimits, TocEntry};
use support::EpubBuilder;
use tempfile::tempdir;

#[test]
fn background_progress_failure_does_not_disable_durable_quit() {
    use ers::app::workers::{ReaderJobs, WorkerEvent};
    use ers::reading::SaveRequest;
    use ers::storage::Store;
    let directory = tempdir().unwrap();
    let mut fixture = EpubBuilder::new("3.0");
    fixture.missing_resource = Some("notes".into());
    let book = fixture.build(directory.path());
    let database = directory.path().join("states.db");
    let jobs = ReaderJobs::start(book.clone(), database.clone(), 40);
    let WorkerEvent::Opened { position, .. } = jobs
        .events
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
    else {
        panic!("Expected an opened book");
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if matches!(
            jobs.events
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .unwrap(),
            WorkerEvent::Failed(_)
        ) {
            break;
        }
    }
    jobs.save(SaveRequest {
        position: position.clone(),
        progress: None,
        quitting: true,
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let WorkerEvent::Saved { quitting, error } = jobs
            .events
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap()
        {
            assert!(quitting);
            assert!(error.is_none(), "{error:?}");
            break;
        }
    }
    assert_eq!(
        Store::open(&database)
            .unwrap()
            .reading_state(book.canonicalize().unwrap().to_str().unwrap())
            .unwrap(),
        Some(position)
    );
}

#[test]
fn annotation_commit_does_not_wait_for_the_markdown_lock() {
    use ers::annotations::{
        AnnotationCommand, AnnotationDraft, AnnotationJob, ExportLink, TextAnchor,
    };
    use ers::app::workers::{ReaderJobs, WorkerEvent};
    use ers::storage::Store;

    let directory = tempdir().unwrap();
    let book = EpubBuilder::new("3.0").build(directory.path());
    let database = directory.path().join("states.db");
    let notes = directory.path().join("notes.md");
    std::fs::write(&notes, "Handwritten notes\n").unwrap();
    let jobs = ReaderJobs::start(book, database.clone(), 40);
    assert!(matches!(
        jobs.events
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap(),
        WorkerEvent::Opened { .. }
    ));
    let mut store = Store::open(&database).unwrap();
    let identity = store.books().unwrap().remove(0);
    store
        .set_link(&ExportLink {
            book_id: identity.id.clone(),
            path: notes.to_string_lossy().into_owned(),
            digest: None,
            revision: 0,
            exported_revision: -1,
        })
        .unwrap();
    let lock = std::fs::File::create(directory.path().join(".notes.md.ers.lock")).unwrap();
    lock.lock().unwrap();
    jobs.annotate(AnnotationJob {
        request_id: 1,
        command: AnnotationCommand::Create(AnnotationDraft {
            anchor: TextAnchor::new("OPS/second.xhtml", 0, "Second").unwrap(),
            note: "A durable note".into(),
        }),
    })
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut committed = false;
    while std::time::Instant::now() < deadline {
        if let WorkerEvent::Annotation(result) = jobs
            .events
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap()
        {
            assert!(result.error.is_none(), "{:?}", result.error);
            committed = true;
            break;
        }
    }
    assert!(committed);
    assert_eq!(
        store.annotations(&identity.id).unwrap()[0].note,
        "A durable note"
    );
    assert_eq!(
        std::fs::read_to_string(&notes).unwrap(),
        "Handwritten notes\n"
    );
    lock.unlock().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut exported = false;
    while std::time::Instant::now() < deadline {
        if let WorkerEvent::Export(state) = jobs
            .events
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap()
        {
            assert!(state.error.is_none(), "{:?}", state.error);
            if state.pending() {
                continue;
            }
            exported = true;
            break;
        }
    }
    assert!(exported);
    let text = std::fs::read_to_string(&notes).unwrap();
    assert!(text.starts_with("Handwritten notes\n"));
    assert!(text.contains("A durable note"));
    assert!(text.contains("^ers-"));
}

#[test]
fn invalid_saved_positions_do_not_reset_to_the_first_page() {
    use ers::app::workers::{ReaderJobs, WorkerEvent};
    use ers::reading::ReadingState;
    use ers::storage::Store;

    for (content_id, offset) in [("absent.xhtml", 0), ("OPS/second.xhtml", usize::MAX / 2)] {
        let directory = tempdir().unwrap();
        let book = EpubBuilder::new("3.0").build(directory.path());
        let database = directory.path().join("states.db");
        let mut store = Store::open(&database).unwrap();
        store
            .save_reading_state(
                book.canonicalize().unwrap().to_str().unwrap(),
                &ReadingState {
                    content_id: content_id.into(),
                    offset,
                    textwidth: 40,
                    screen_row: 0,
                },
            )
            .unwrap();
        drop(store);
        let jobs = ReaderJobs::start(book, database, 40);
        assert!(matches!(
            jobs.events
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            WorkerEvent::Failed(_)
        ));
    }
}

#[test]
fn cli_dump_reads_spine_without_creating_storage() {
    let directory = tempdir().unwrap();
    let path = EpubBuilder::new("3.0").build(directory.path());
    let config = directory.path().join("configuration");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ers"))
        .args(["--dump"])
        .arg(path)
        .env("XDG_CONFIG_HOME", &config)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Café 👩‍🔬"));
    assert!(!text.contains("Supplemental footnote"));
    assert!(!config.exists());
}

#[test]
fn cli_help_identifies_the_native_command() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ers"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().contains("ers"));
}

#[test]
fn epub_versions_preserve_spine_then_supplemental_order() {
    for version in ["1.0", "2.0", "3.0"] {
        let directory = tempdir().unwrap();
        let fixture = EpubBuilder::new(version);
        let book = EpubBook::open(&fixture.build(directory.path())).unwrap();
        assert_eq!(
            book.contents(),
            [
                "OPS/second.xhtml",
                "OPS/first chapter.xhtml",
                "OPS/notes.xhtml"
            ]
        );
    }
}

#[test]
fn epub_navigation_preserves_labels_and_fragments() {
    for version in ["2.0", "3.0"] {
        let directory = tempdir().unwrap();
        let fixture = EpubBuilder::new(version);
        let book = EpubBook::open(&fixture.build(directory.path())).unwrap();
        assert_eq!(
            book.toc(),
            [
                TocEntry {
                    label: "Second chapter".into(),
                    content_index: 0,
                    section: Some("section".into())
                },
                TocEntry {
                    label: "First".into(),
                    content_index: 1,
                    section: None
                }
            ]
        );
    }
}

#[test]
fn epub_resources_retain_original_utf8_text() {
    let directory = tempdir().unwrap();
    let fixture = EpubBuilder::new("3.0");
    let mut book = EpubBook::open(&fixture.build(directory.path())).unwrap();
    assert_eq!(
        book.raw_text("OPS/first chapter.xhtml").unwrap(),
        fixture.documents[0].body
    );
    assert_eq!(
        book.asset("OPS/picture.png").unwrap(),
        b"synthetic image bytes"
    );
}

#[test]
fn epub_metadata_decodes_xml_entities() {
    let directory = tempdir().unwrap();
    let fixture = EpubBuilder::new("3.0");
    let book = EpubBook::open(&fixture.build(directory.path())).unwrap();
    assert_eq!(
        book.metadata(),
        &BookMetadata {
            title: Some(fixture.title),
            creator: Some("Fixture author".into()),
            ..BookMetadata::default()
        }
    );
}

#[test]
fn archive_limits_reject_oversized_resources() {
    let directory = tempdir().unwrap();
    let fixture = EpubBuilder::new("3.0");
    let limits = ResourceLimits {
        resource_bytes_max: 20,
        ..ResourceLimits::default()
    };
    assert!(EpubBook::open_with_limits(&fixture.build(directory.path()), limits).is_err());
}

#[test]
fn resource_lookup_rejects_parent_traversal() {
    let directory = tempdir().unwrap();
    let fixture = EpubBuilder::new("3.0");
    let mut book = EpubBook::open(&fixture.build(directory.path())).unwrap();
    assert!(book.asset("../../secret").is_err());
}
