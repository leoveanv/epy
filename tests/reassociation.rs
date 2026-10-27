#[path = "support/annotations.rs"]
mod annotation_support;
#[path = "support/epub.rs"]
mod epub_support;

use epub_support::EpubBuilder;
use ers::annotations::{
    Annotation, AnnotationBook, ExportLink, ReassociationCommand, ReassociationJob,
    ReassociationReply, TextAnchor,
};
use ers::app::workers::{ReaderJobs, WorkerEvent};
use ers::storage::Store;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::{TempDir, tempdir};

struct ReassociationFixture {
    jobs: ReaderJobs,
    directory: TempDir,
    database: PathBuf,
    source: AnnotationBook,
    target: AnnotationBook,
    original: Vec<Annotation>,
    existing: Vec<Annotation>,
    next_request: u64,
}

fn identity(path: &Path, id: &str, title: &str) -> AnnotationBook {
    AnnotationBook {
        id: id.into(),
        title: title.into(),
        author: "Fixture author".into(),
        fingerprint: format!("{:x}", Sha256::digest(std::fs::read(path).unwrap())),
    }
}

fn record(book_id: &str, index: usize) -> Annotation {
    let mut annotation = annotation_support::annotation();
    annotation.book_id = book_id.into();
    annotation.id = format!("00000000-0000-4000-8000-{:012}", index + 101);
    annotation.note = format!("Source note {index}");
    annotation.anchor = match index {
        0 => TextAnchor::new("OPS/second.xhtml", 0, "First token.").unwrap(),
        1 => TextAnchor::new("OPS/second.xhtml", 13, "Second repeat.").unwrap(),
        _ => TextAnchor::new("OPS/retired.xhtml", 0, "Supplemental footnote").unwrap(),
    };
    annotation.content_order = if index == 2 { 2 } else { 0 };
    annotation
}

impl ReassociationFixture {
    fn new(keep_second_position: bool) -> Self {
        let directory = tempdir().unwrap();
        let source_root = directory.path().join("source");
        let target_root = directory.path().join("target");
        std::fs::create_dir(&source_root).unwrap();
        std::fs::create_dir(&target_root).unwrap();
        let mut source_fixture = EpubBuilder::new("3.0");
        source_fixture.title = "Source edition".into();
        source_fixture.documents[1].body = "<p>First token. Second repeat.</p>".into();
        source_fixture.documents[2].href = "retired.xhtml".into();
        let source_path = source_fixture.build(&source_root);
        let mut target_fixture = EpubBuilder::new("3.0");
        target_fixture.title = "Changed edition".into();
        target_fixture.documents[1].body = if keep_second_position {
            "<p>First token. Second repeat.</p>"
        } else {
            "<p>First token. moved Second repeat.</p>"
        }
        .into();
        let target_path = target_fixture.build(&target_root);
        let database = directory.path().join("states.db");
        let mut store = Store::open(&database).unwrap();
        let source = identity(
            &source_path,
            "00000000-0000-4000-8000-000000000011",
            "Source edition",
        );
        let target = identity(
            &target_path,
            "00000000-0000-4000-8000-000000000012",
            "Changed edition",
        );
        store
            .identify_book(
                &source,
                source_path.canonicalize().unwrap().to_str().unwrap(),
            )
            .unwrap();
        store
            .identify_book(
                &target,
                target_path.canonicalize().unwrap().to_str().unwrap(),
            )
            .unwrap();
        for index in 0..3 {
            store.save_annotation(&record(&source.id, index)).unwrap();
        }
        let mut existing = record(&target.id, 0);
        existing.id = "00000000-0000-4000-8000-000000000201".into();
        existing.note = "Existing target note".into();
        store.save_annotation(&existing).unwrap();
        let original = store.annotations(&source.id).unwrap();
        let existing = store.annotations(&target.id).unwrap();
        let source_notes = directory.path().join("Source notes.md");
        std::fs::write(&source_notes, "Source Markdown\n").unwrap();
        store
            .set_link(&ExportLink {
                book_id: source.id.clone(),
                path: source_notes.to_string_lossy().into_owned(),
                digest: None,
                revision: 0,
                exported_revision: -1,
            })
            .unwrap();
        let jobs = ReaderJobs::start(target_path, database.clone(), 40);
        assert!(matches!(
            jobs.events.recv_timeout(Duration::from_secs(5)).unwrap(),
            WorkerEvent::Opened { .. }
        ));
        Self {
            jobs,
            directory,
            database,
            source,
            target,
            original,
            existing,
            next_request: 0,
        }
    }

    fn command(&mut self, command: ReassociationCommand) -> Result<ReassociationReply, String> {
        self.next_request += 1;
        self.jobs
            .reassociate(ReassociationJob {
                request_id: self.next_request,
                command,
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let WorkerEvent::Reassociation(result) = self
                .jobs
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap()
            {
                assert_eq!(result.request_id, self.next_request);
                return result.reply;
            }
        }
    }
}

#[test]
fn reassociation_sources_exclude_the_current_identity() {
    let mut fixture = ReassociationFixture::new(false);
    let ReassociationReply::Sources(sources) =
        fixture.command(ReassociationCommand::Sources).unwrap()
    else {
        panic!("Expected source identities");
    };
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].book, fixture.source);
    assert_eq!(sources[0].annotation_count, 3);
}

#[test]
fn reassociation_copies_only_exact_document_positions() {
    let mut fixture = ReassociationFixture::new(false);
    let ReassociationReply::Copied(report) = fixture
        .command(ReassociationCommand::Copy(fixture.source.id.clone()))
        .unwrap()
    else {
        panic!("Expected a copy report");
    };
    assert_eq!((report.copied_count, report.skipped_count), (1, 2));
    let store = Store::open(&fixture.database).unwrap();
    assert_eq!(
        store.annotations(&fixture.source.id).unwrap(),
        fixture.original
    );
    let target = store.annotations(&fixture.target.id).unwrap();
    assert_eq!(target.len(), fixture.existing.len() + 1);
    assert!(target.contains(&fixture.existing[0]));
    let copied = target
        .iter()
        .find(|annotation| annotation.id != fixture.existing[0].id)
        .unwrap();
    assert_eq!(copied.anchor, fixture.original[0].anchor);
    assert_eq!(copied.note, fixture.original[0].note);
    assert_ne!(copied.id, fixture.original[0].id);
    assert_eq!(copied.reference.as_ref().unwrap().location, "Location 1:1");
    assert!(store.export_link(&fixture.target.id).unwrap().is_none());
    assert_eq!(
        std::fs::read_to_string(fixture.directory.path().join("Source notes.md")).unwrap(),
        "Source Markdown\n"
    );
}

#[test]
fn failed_reassociation_rolls_back_every_copied_record() {
    let mut fixture = ReassociationFixture::new(true);
    let mut store = Store::open(&fixture.database).unwrap();
    let notes = fixture.directory.path().join("Target notes.md");
    std::fs::write(&notes, "Target Markdown\n").unwrap();
    store
        .set_link(&ExportLink {
            book_id: fixture.target.id.clone(),
            path: notes.to_string_lossy().into_owned(),
            digest: None,
            revision: 0,
            exported_revision: -1,
        })
        .unwrap();
    let connection = rusqlite::Connection::open(&fixture.database).unwrap();
    connection.execute_batch(&format!("CREATE TRIGGER reject_second_copy BEFORE INSERT ON annotations WHEN NEW.book_id='{}' AND NEW.start=13 BEGIN SELECT RAISE(ABORT, 'reject second copy'); END;", fixture.target.id)).unwrap();
    assert!(
        fixture
            .command(ReassociationCommand::Copy(fixture.source.id.clone()))
            .is_err()
    );
    assert_eq!(
        store.annotations(&fixture.target.id).unwrap(),
        fixture.existing
    );
    assert_eq!(
        store.annotations(&fixture.source.id).unwrap(),
        fixture.original
    );
    assert_eq!(
        store
            .export_link(&fixture.target.id)
            .unwrap()
            .unwrap()
            .revision,
        0
    );
}
