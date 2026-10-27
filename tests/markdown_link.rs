#[path = "support/epub.rs"]
mod epub_support;

use epub_support::EpubBuilder;
use ers::annotations::{ExportState, LinkCommand, LinkJob, LinkReply};
use ers::app::workers::{ReaderJobs, WorkerEvent};
use ers::storage::Store;
use std::path::Path;
use std::time::{Duration, Instant};
use tempfile::{TempDir, tempdir};

struct LinkFixture {
    jobs: ReaderJobs,
    directory: TempDir,
    next_request: u64,
    book_id: String,
}

impl LinkFixture {
    fn new() -> Self {
        let directory = tempdir().unwrap();
        let path = EpubBuilder::new("3.0").build(directory.path());
        let jobs = ReaderJobs::start(path, directory.path().join("states.db"), 40);
        assert!(matches!(
            jobs.events.recv_timeout(Duration::from_secs(5)).unwrap(),
            WorkerEvent::Opened { .. }
        ));
        let book_id = Store::open(&directory.path().join("states.db"))
            .unwrap()
            .books()
            .unwrap()
            .remove(0)
            .id;
        Self {
            jobs,
            directory,
            next_request: 0,
            book_id,
        }
    }

    fn command(&mut self, command: LinkCommand) -> LinkReply {
        self.next_request += 1;
        self.jobs
            .link(LinkJob {
                request_id: self.next_request,
                command,
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let WorkerEvent::Link(result) = self
                .jobs
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap()
            {
                assert_eq!(result.request_id, self.next_request);
                return result.reply.unwrap();
            }
        }
    }

    fn wait_export(&self) -> ExportState {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let WorkerEvent::Export(state) = self
                .jobs
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap()
                && !state.running
            {
                return state;
            }
        }
    }

    fn link_existing(&mut self, path: &Path) -> ers::annotations::Destination {
        let LinkReply::Destination(destination) =
            self.command(LinkCommand::Inspect(path.to_string_lossy().into_owned()))
        else {
            panic!("Expected a validated destination");
        };
        self.command(LinkCommand::Set {
            destination: destination.clone(),
            create: false,
        });
        destination
    }
}

#[test]
fn linking_exports_without_replacing_handwritten_content() {
    let mut fixture = LinkFixture::new();
    let notes = fixture.directory.path().join("Notes with spaces.md");
    std::fs::write(&notes, "---\ntitle: Notes\n---\nPersonal text\n").unwrap();
    fixture.link_existing(&notes);
    let result = fixture.wait_export();
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(
        std::fs::read_to_string(&notes)
            .unwrap()
            .starts_with("---\ntitle: Notes\n---\nPersonal text\n")
    );
    let link = Store::open(&fixture.directory.path().join("states.db"))
        .unwrap()
        .export_link(&fixture.book_id)
        .unwrap()
        .unwrap();
    assert_eq!(link.revision, link.exported_revision);
    assert!(link.digest.is_some());
}

#[test]
fn inspecting_a_missing_destination_does_not_create_a_file() {
    let mut fixture = LinkFixture::new();
    let path = fixture.directory.path().join("New notes.md");
    let LinkReply::Destination(destination) =
        fixture.command(LinkCommand::Inspect(path.to_string_lossy().into_owned()))
    else {
        panic!("Expected destination");
    };
    assert!(!destination.exists);
    assert!(!path.exists());
    assert!(
        Store::open(&fixture.directory.path().join("states.db"))
            .unwrap()
            .export_link(&fixture.book_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn relinking_cancels_an_export_waiting_for_the_previous_file_lock() {
    let mut fixture = LinkFixture::new();
    let previous = fixture.directory.path().join("Previous.md");
    let next = fixture.directory.path().join("Next.md");
    std::fs::write(&previous, "Previous content\n").unwrap();
    std::fs::write(&next, "Next content\n").unwrap();
    let lock =
        std::fs::File::create(fixture.directory.path().join(".Previous.md.ers.lock")).unwrap();
    lock.lock().unwrap();
    fixture.link_existing(&previous);
    let approved = fixture.link_existing(&next);
    let state = fixture.wait_export();
    assert!(state.error.is_none(), "{:?}", state.error);
    assert_eq!(state.link.unwrap().path, approved.path);
    assert_eq!(
        std::fs::read_to_string(&previous).unwrap(),
        "Previous content\n"
    );
    let exported = std::fs::read_to_string(next).unwrap();
    assert!(exported.starts_with("Next content\n"));
    assert!(exported.contains(&format!(
        "<!-- ers:annotations:{}:start -->",
        fixture.book_id
    )));
    lock.unlock().unwrap();
}

#[test]
fn unlinking_retains_the_exported_markdown() {
    let mut fixture = LinkFixture::new();
    let notes = fixture.directory.path().join("Notes.md");
    std::fs::write(&notes, "Personal text\n").unwrap();
    fixture.link_existing(&notes);
    assert!(fixture.wait_export().error.is_none());
    let before = std::fs::read(&notes).unwrap();
    fixture.command(LinkCommand::Unlink);
    assert_eq!(std::fs::read(&notes).unwrap(), before);
    assert!(
        Store::open(&fixture.directory.path().join("states.db"))
            .unwrap()
            .export_link(&fixture.book_id)
            .unwrap()
            .is_none()
    );
}
