#[path = "support/epub.rs"]
mod epub_support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use epub_support::EpubBuilder;
use ers::book::LibraryEntry;
use ers::reading::ReadingState;
use ers::storage::Store;

struct CliFixture {
    _root: tempfile::TempDir,
    configuration_home: PathBuf,
    first: PathBuf,
    second: PathBuf,
}

impl CliFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let first = Self::book(root.path(), "first", "FIRST-BOOK-CONTENT");
        let second = Self::book(root.path(), "second", "SECOND-BOOK-CONTENT");
        let configuration_home = root.path().join("config");
        Self {
            _root: root,
            configuration_home,
            first,
            second,
        }
    }

    fn book(root: &Path, directory: &str, text: &str) -> PathBuf {
        let path = root.join(directory);
        std::fs::create_dir_all(&path).unwrap();
        let mut builder = EpubBuilder::new("3.0");
        builder.documents[1].body = format!("<p>{text} café</p>");
        builder.build(&path).canonicalize().unwrap()
    }

    fn seed_history(&self) {
        let directory = self.configuration_home.join("ers");
        std::fs::create_dir_all(&directory).unwrap();
        let mut store = Store::open(&directory.join("states.db")).unwrap();
        for (path, title, last_read) in [
            (&self.first, "Northern Archive", "2026-10-02"),
            (&self.second, "Luminous Atlas", "2026-10-01"),
        ] {
            store
                .save_session(
                    &LibraryEntry {
                        last_read: last_read.into(),
                        filepath: path.to_string_lossy().into_owned(),
                        title: Some(title.into()),
                        author: Some("Fixture author".into()),
                        reading_progress: Some(0.5),
                    },
                    &ReadingState {
                        content_id: "OPS/second.xhtml".into(),
                        textwidth: 80,
                        offset: 0,
                        screen_row: 0,
                    },
                )
                .unwrap();
        }
    }

    fn run(&self, arguments: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ers"))
            .args(arguments)
            .env("XDG_CONFIG_HOME", &self.configuration_home)
            .env("PATH", "/nonexistent")
            .output()
            .unwrap()
    }
}

#[test]
fn empty_history_without_a_target_returns_an_error() {
    let fixture = CliFixture::new();
    let result = fixture.run(&[]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("Found no last read ebook file")
    );
}

#[test]
fn history_flag_prints_entries_in_recent_first_order() {
    let fixture = CliFixture::new();
    fixture.seed_history();
    let result = fixture.run(&["--history"]);
    assert!(result.status.success());
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.find("Northern Archive").unwrap() < output.find("Luminous Atlas").unwrap());
}

#[test]
fn missing_target_opens_the_most_recent_history_entry() {
    let fixture = CliFixture::new();
    fixture.seed_history();
    let result = fixture.run(&["--dump"]);
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("FIRST-BOOK-CONTENT café")
    );
}

#[test]
fn multiword_history_search_opens_the_matching_book() {
    let fixture = CliFixture::new();
    fixture.seed_history();
    let result = fixture.run(&["--dump", "Luminous", "Atlas"]);
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("SECOND-BOOK-CONTENT café")
    );
}

#[test]
fn negative_history_numbers_open_the_corresponding_book() {
    let fixture = CliFixture::new();
    fixture.seed_history();
    let result = fixture.run(&["--dump", "-1"]);
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("FIRST-BOOK-CONTENT café")
    );
}

#[test]
fn out_of_range_negative_history_number_returns_lookup_error() {
    let fixture = CliFixture::new();
    fixture.seed_history();
    let result = fixture.run(&["--dump", "-9223372036854775808"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("#-9223372036854775808 file not found")
    );
}

#[test]
fn numbered_history_targets_open_the_selected_book() {
    let fixture = CliFixture::new();
    fixture.seed_history();
    let result = fixture.run(&["--dump", "2"]);
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("SECOND-BOOK-CONTENT café")
    );
}
