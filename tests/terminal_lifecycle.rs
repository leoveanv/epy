#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "support/epub.rs"]
mod epub_support;
#[path = "support/pty.rs"]
mod pty_support;

use epub_support::EpubBuilder;
use ers::configuration::Configuration;
use ers::storage::Store;
use pty_support::PtyReader;
use std::path::PathBuf;

struct TerminalFixture {
    _root: tempfile::TempDir,
    book: PathBuf,
    configuration_home: PathBuf,
    database: PathBuf,
}

impl TerminalFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut builder = EpubBuilder::new("3.0");
        builder.documents[1].body = (0..100)
            .map(|index| {
                format!("<p>Passage {index:04} has enough text for a saved reading anchor.</p>")
            })
            .collect();
        let book = builder.build(root.path()).canonicalize().unwrap();
        let configuration_home = root.path().join("config");
        let directory = configuration_home.join("ers");
        std::fs::create_dir_all(&directory).unwrap();
        let mut configuration = Configuration::default();
        configuration.settings.inline_images = false;
        std::fs::write(
            directory.join("configuration.json"),
            configuration.to_json().unwrap(),
        )
        .unwrap();
        Self {
            _root: root,
            book,
            configuration_home,
            database: directory.join("states.db"),
        }
    }

    fn reader(&self) -> PtyReader {
        let mut reader = PtyReader::start(&self.book, &self.configuration_home);
        reader.wait_text("Passage");
        reader
    }
}

#[test]
fn closed_terminal_exits_without_receiving_a_hangup_signal() {
    let fixture = TerminalFixture::new();
    let mut reader = fixture.reader();
    reader.close_terminal();
    assert!(reader.wait_exit().code().is_some());
}

#[test]
fn closed_terminal_preserves_recent_reading_progress() {
    let fixture = TerminalFixture::new();
    let mut reader = fixture.reader();
    reader.send(b"12jM");
    reader.wait_text("Metadata");
    reader.close_terminal();
    reader.wait_exit();
    let store = Store::open(&fixture.database).unwrap();
    let position = store
        .reading_state(fixture.book.to_str().unwrap())
        .unwrap()
        .expect("A lost terminal must still save recent progress");
    assert!(position.offset > 100);
}

#[test]
fn termination_signal_exits_an_unsaved_note_without_confirmation() {
    let fixture = TerminalFixture::new();
    let mut reader = fixture.reader();
    reader.send(b"v n");
    reader.wait_text("Save");
    reader.terminate();
    assert!(reader.wait_exit().success());
}
