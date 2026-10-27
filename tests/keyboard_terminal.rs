#![cfg(target_os = "macos")]

#[path = "support/epub.rs"]
mod epub_support;
#[allow(dead_code)]
#[path = "support/pty.rs"]
mod pty_support;

use epub_support::EpubBuilder;
use ers::configuration::Configuration;
use ers::storage::Store;
use pty_support::PtyReader;
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct KeyboardFixture {
    _root: tempfile::TempDir,
    book: PathBuf,
    configuration_home: PathBuf,
    database: PathBuf,
}

impl KeyboardFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut builder = EpubBuilder::new("3.0");
        builder.documents[1].body = (0..100)
            .map(|index| format!("<p>Passage {index:04} contains a reading position.</p>"))
            .collect();
        builder.documents[0].body = "<p>FIRST-CHAPTER-TARGET</p>".into();
        let book = builder.build(root.path()).canonicalize().unwrap();
        let configuration_home = root.path().join("config");
        let directory = configuration_home.join("ers");
        std::fs::create_dir_all(&directory).unwrap();
        let mut configuration = Configuration::default();
        configuration.settings.inline_images = false;
        configuration.settings.page_scroll_animation = false;
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

    fn reader(&self, tmux: bool) -> PtyReader {
        let mut reader = PtyReader::start_with_tmux(&self.book, &self.configuration_home, tmux);
        reader.wait_text("Passage");
        reader
    }

    fn wait_for_content(&self, reader: &mut PtyReader, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            reader.output_checkpoint();
            let position = Store::open(&self.database)
                .unwrap()
                .reading_state(self.book.to_str().unwrap())
                .unwrap();
            if position
                .as_ref()
                .is_some_and(|position| position.content_id.contains(expected))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Reading position did not reach {expected}: {position:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn notes(&self) -> Vec<String> {
        let store = Store::open(&self.database).unwrap();
        let book = store.books().unwrap().pop().unwrap();
        store
            .annotations(&book.id)
            .unwrap()
            .into_iter()
            .map(|annotation| annotation.note)
            .collect()
    }

    fn position(&self) -> ers::reading::ReadingState {
        Store::open(&self.database)
            .unwrap()
            .reading_state(self.book.to_str().unwrap())
            .unwrap()
            .unwrap()
    }

    fn offset(&self) -> usize {
        self.position().offset
    }
}

fn finish(reader: &mut PtyReader) {
    reader.send(b"q");
    assert!(reader.wait_exit().success());
}

fn verify_forward_key(sequence: &[u8]) {
    let fixture = KeyboardFixture::new();
    let mut reader = fixture.reader(false);
    reader.send(b"\t");
    reader.wait_text("Second chapter");
    reader.send(b"j\r");
    reader.wait_text("FIRST-CHAPTER-TARGET");
    fixture.wait_for_content(&mut reader, "first");
    reader.send(b"\x0f");
    fixture.wait_for_content(&mut reader, "second");
    reader.send(sequence);
    fixture.wait_for_content(&mut reader, "first");
    finish(&mut reader);
}

#[test]
fn csi_u_control_i_moves_forward_without_treating_tab_as_control_i() {
    verify_forward_key(b"\x1b[105;5u");
}

#[test]
fn xterm_modified_control_i_moves_forward_without_treating_tab_as_control_i() {
    verify_forward_key(b"\x1b[27;5;105~");
}

#[test]
fn released_keys_do_not_move_the_reader() {
    let fixture = KeyboardFixture::new();
    let mut reader = fixture.reader(false);
    reader.send(b"\x1b[106;1:3u");
    finish(&mut reader);
    assert_eq!(fixture.offset(), 0);
}

#[test]
fn repeated_keys_move_the_reader() {
    let fixture = KeyboardFixture::new();
    let mut reader = fixture.reader(false);
    reader.send(b"\x1b[106;1:2u");
    finish(&mut reader);
    assert!(fixture.offset() > 0);
}

#[test]
fn tmux_requests_and_restores_extended_keyboard_mode() {
    let fixture = KeyboardFixture::new();
    let mut reader = fixture.reader(true);
    reader.wait_text("\x1b[>4;2m");
    finish(&mut reader);
    assert!(reader.saw_output("\x1b[>4;0m"));
}

#[test]
fn bracketed_paste_preserves_multiline_notes_with_tmux_keyboard_mode() {
    let fixture = KeyboardFixture::new();
    let mut reader = fixture.reader(true);
    reader.send(b"v n");
    reader.wait_text("Save");
    reader.send(b"\x1b[200~first\nsecond\tindented\x1b[201~");
    reader.send(b"\t\r");
    reader.wait_text("Annotation sav");
    reader.send(b"v");
    finish(&mut reader);
    assert_eq!(fixture.notes(), ["first\nsecond    indented"]);
}

#[test]
fn focus_reporting_is_enabled_and_restored_on_exit() {
    let fixture = KeyboardFixture::new();
    let mut reader = fixture.reader(false);
    reader.wait_text("\x1b[?1004h");
    finish(&mut reader);
    assert!(reader.saw_output("\x1b[?1004l"));
}

#[test]
fn an_unfinished_note_survives_a_one_cell_terminal_resize() {
    let fixture = KeyboardFixture::new();
    let mut reader = fixture.reader(false);
    reader.send(b"v n");
    reader.wait_text("Save");
    reader.send(b"first");
    reader.wait_text("first");
    reader.resize(20, 10);
    reader.wait_text("small");
    let checkpoint = reader.output_checkpoint();
    reader.resize(1, 1);
    reader.wait_output_after(checkpoint);
    let checkpoint = reader.output_checkpoint();
    reader.resize(100, 30);
    reader.wait_output_after(checkpoint);
    reader.send(b"-tail\t\r");
    reader.wait_text("Annotation sav");
    reader.send(b"v");
    finish(&mut reader);
    assert_eq!(fixture.notes(), ["first-tail"]);
    assert_eq!(fixture.position().textwidth, 80);
}

#[test]
fn starting_in_a_small_terminal_waits_for_usable_geometry() {
    let fixture = KeyboardFixture::new();
    let mut reader =
        PtyReader::start_sized(&fixture.book, &fixture.configuration_home, false, 20, 10);
    reader.wait_text("small");
    reader.resize(100, 30);
    reader.wait_text("Passage");
    finish(&mut reader);
    assert_eq!(fixture.position().textwidth, 80);
}

#[test]
fn a_small_terminal_can_quit_without_opening_the_book() {
    let fixture = KeyboardFixture::new();
    let mut reader =
        PtyReader::start_sized(&fixture.book, &fixture.configuration_home, false, 20, 10);
    reader.wait_text("small");
    finish(&mut reader);
    assert!(
        Store::open(&fixture.database)
            .unwrap()
            .reading_state(fixture.book.to_str().unwrap())
            .unwrap()
            .is_none()
    );
    assert!(reader.saw_output("\x1b[?1004l"));
    assert!(reader.saw_output("\x1b[<1u"));
}

#[test]
fn direct_terminal_restores_its_keyboard_stack() {
    let fixture = KeyboardFixture::new();
    let mut reader = fixture.reader(false);
    reader.wait_text("\x1b[>3u");
    finish(&mut reader);
    assert!(reader.saw_output("\x1b[<1u"));
}
