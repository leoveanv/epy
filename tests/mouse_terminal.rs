#![cfg(target_os = "macos")]

#[path = "support/annotations.rs"]
mod annotation_support;
#[path = "support/epub.rs"]
mod epub_support;
#[path = "support/images.rs"]
mod image_support;
#[allow(dead_code)]
#[path = "support/pty.rs"]
mod pty_support;

use epub_support::{AssetInput, EpubBuilder};
use ers::annotations::TextAnchor;
use ers::book::Book;
use ers::configuration::{Command, Configuration};
use ers::document::text::parse_document;
use ers::reading::ReadingState;
use ers::storage::Store;
use image_support::ImageBuilder;
use pty_support::PtyReader;
use std::path::PathBuf;

const RIGHT_PAGE_CLICK: &[u8] = b"\x1b[<0;80;10M\x1b[<0;80;10m";
const LEFT_PAGE_CLICK: &[u8] = b"\x1b[<0;10;10M\x1b[<0;10;10m";
const RIGHT_CLICK: &[u8] = b"\x1b[<2;80;10M\x1b[<2;80;10m";
const FIRST_ITEM_CLICK: &[u8] = b"\x1b[<0;10;3M\x1b[<0;10;3m";
const SECOND_ITEM_CLICK: &[u8] = b"\x1b[<0;10;4M\x1b[<0;10;4m";
const WHEEL_UP: &[u8] = b"\x1b[<64;50;10M";
const WHEEL_DOWN: &[u8] = b"\x1b[<65;50;10M";
const CONTROL_WHEEL_UP: &[u8] = b"\x1b[<80;50;10M";
const CONTROL_WHEEL_DOWN: &[u8] = b"\x1b[<81;50;10M";

struct MouseFixture {
    _root: tempfile::TempDir,
    book: PathBuf,
    configuration_home: PathBuf,
    database: PathBuf,
}

impl MouseFixture {
    fn new(configuration: Configuration) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut builder = EpubBuilder::new("3.0");
        builder.documents[1].body = (0..100)
            .map(|index| format!("<p>Passage {index:04} contains a reading position.</p>"))
            .collect();
        builder.documents[0].body = "<p>FIRST-CHAPTER-TARGET</p>".into();
        Self::build(configuration, root, builder)
    }

    fn with_image(configuration: Configuration) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut builder = EpubBuilder::new("3.0");
        builder.documents[1].body = "<p>Passage</p><img src='figure.png'/>".into();
        builder.assets.push(AssetInput::new(
            "figure.png",
            "image/png",
            ImageBuilder::new().bytes(),
        ));
        Self::build(configuration, root, builder)
    }

    fn with_images(configuration: Configuration) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut builder = EpubBuilder::new("3.0");
        builder.documents[1].body =
            "<p>Passage</p><img src='first.png'/><img src='second.png'/>".into();
        for path in ["first.png", "second.png"] {
            builder.assets.push(AssetInput::new(
                path,
                "image/png",
                ImageBuilder::new().bytes(),
            ));
        }
        Self::build(configuration, root, builder)
    }

    fn build(configuration: Configuration, root: tempfile::TempDir, builder: EpubBuilder) -> Self {
        let book = builder.build(root.path()).canonicalize().unwrap();
        let configuration_home = root.path().join("config");
        let directory = configuration_home.join("ers");
        std::fs::create_dir_all(&directory).unwrap();
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

    fn seed_other_book(&self) -> PathBuf {
        let directory = self._root.path().join("other");
        std::fs::create_dir_all(&directory).unwrap();
        let mut builder = EpubBuilder::new("3.0");
        builder.title = "OTHER-LIBRARY-TITLE".into();
        builder.documents[1].body = "<p>OTHER-BOOK-TARGET</p>".into();
        let book = builder.build(&directory).canonicalize().unwrap();
        let mut reader = PtyReader::start(&book, &self.configuration_home);
        reader.wait_text("OTHER-BOOK-TARGET");
        finish(&mut reader);
        book
    }

    fn seed_annotations(&self) {
        let mut reader = self.reader();
        finish(&mut reader);
        let mut book = Book::open(self.book.clone()).unwrap();
        let content_id = book.contents()[0].clone();
        let source = parse_document(&content_id, &book.raw_text(&content_id).unwrap()).unwrap();
        let mut store = Store::open(&self.database).unwrap();
        let book_id = store.books().unwrap().pop().unwrap().id;
        for (quote, note) in [
            ("Passage 0001", "FIRST-NOTE-UNIQUE"),
            ("Passage 0030", "SECOND-NOTE-UNIQUE"),
        ] {
            let byte_offset = source.source.text.find(quote).unwrap();
            let offset = source.source.text[..byte_offset].chars().count();
            let mut annotation = annotation_support::annotation();
            annotation.id = uuid::Uuid::new_v4().to_string();
            annotation.book_id = book_id.clone();
            annotation.anchor = TextAnchor::new(&content_id, offset, quote).unwrap();
            annotation.note = note.into();
            store.save_annotation(&annotation).unwrap();
        }
    }

    fn seed_many_annotations(&self) -> Vec<usize> {
        let mut reader = self.reader();
        finish(&mut reader);
        let mut book = Book::open(self.book.clone()).unwrap();
        let content_id = book.contents()[0].clone();
        let source = parse_document(&content_id, &book.raw_text(&content_id).unwrap()).unwrap();
        let mut store = Store::open(&self.database).unwrap();
        let book_id = store.books().unwrap().pop().unwrap().id;
        (0..40)
            .map(|index| {
                let quote = format!("Passage {index:04}");
                let byte_offset = source.source.text.find(&quote).unwrap();
                let offset = source.source.text[..byte_offset].chars().count();
                let mut annotation = annotation_support::annotation();
                annotation.id = uuid::Uuid::new_v4().to_string();
                annotation.book_id = book_id.clone();
                annotation.anchor = TextAnchor::new(&content_id, offset, &quote).unwrap();
                annotation.note = format!("NOTE-{index:04}");
                store.save_annotation(&annotation).unwrap();
                offset
            })
            .collect()
    }

    fn saved_position(&self) -> ReadingState {
        Store::open(&self.database)
            .unwrap()
            .reading_state(self.book.to_str().unwrap())
            .unwrap()
            .unwrap()
    }

    fn saved_notes(&self) -> Vec<String> {
        let store = Store::open(&self.database).unwrap();
        let book = store.books().unwrap().pop().unwrap();
        store
            .annotations(&book.id)
            .unwrap()
            .into_iter()
            .map(|annotation| annotation.note)
            .collect()
    }
}

fn configuration(mouse: bool) -> Configuration {
    let mut configuration = Configuration::default();
    configuration.settings.mouse_support = mouse;
    configuration.settings.inline_images = false;
    configuration.settings.page_scroll_animation = false;
    configuration
}

fn finish(reader: &mut PtyReader) {
    reader.send(b"q");
    assert!(reader.wait_exit().success());
}

fn open_contents(reader: &mut PtyReader) {
    reader.send(RIGHT_CLICK);
    reader.wait_text("Second chapter");
}

#[test]
fn selecting_an_inline_image_marks_its_margin_without_repainting_its_pixels() {
    let mut configuration = configuration(false);
    configuration.settings.inline_images = true;
    let fixture = MouseFixture::with_image(configuration);
    let mut reader = fixture.reader();
    reader.wait_text("38;2;210;20;40;48;2;210;20;40m");
    std::thread::sleep(std::time::Duration::from_millis(150));
    let checkpoint = reader.output_checkpoint();
    reader.send(b"o");
    reader.wait_text("Image 1/1");
    reader.wait_text("▸");
    let changes = reader.output_since(checkpoint);
    assert!(!changes.windows(20).any(|bytes| bytes == [b' '; 20]));
    reader.send(b"\x1b");
    std::thread::sleep(std::time::Duration::from_millis(30));
    finish(&mut reader);
}

#[test]
fn external_viewer_failures_restore_terminal_input() {
    let mut configuration = configuration(false);
    configuration.settings.default_viewer = "/usr/bin/false".into();
    let fixture = MouseFixture::with_image(configuration);
    let mut reader = fixture.reader();
    reader.send(b"o");
    reader.wait_text("Image 1/1");
    reader.send(b"\r");
    reader.wait_text("figure.png");
    reader.send(b"e");
    // #NOTE: The test PTY does not answer the cursor-position query when the screen resumes.
    reader.wait_text("\x1b[6n");
    reader.send(b"\x1b[1;1R");
    reader.wait_text("viewer");
    reader.wait_text("exited");

    let checkpoint = reader.output_checkpoint();
    reader.send(b"\x1b");
    reader.wait_text_after(checkpoint, "Passage");
    reader.send(b"?");
    reader.wait_text("table_of_contents");
    let checkpoint = reader.output_checkpoint();
    reader.send(b"\x1b");
    reader.wait_output_after(checkpoint);
    finish(&mut reader);
}

#[test]
fn ignored_pending_input_is_visible_after_a_bookmark_prompt_opens() {
    let fixture = MouseFixture::new(configuration(false));
    let mut reader = fixture.reader();
    let lock = rusqlite::Connection::open(&fixture.database).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    reader.send(b"v w\r");
    reader.wait_text("aving annota");
    reader.send(b"vb");
    reader.send(&vec![b'x'; 4095]);
    reader.wait_text("Input queue full");
    lock.execute_batch("ROLLBACK").unwrap();
    reader.wait_text("Some input was ignored");
    reader.send(b"\x1b");
    std::thread::sleep(std::time::Duration::from_millis(30));
    finish(&mut reader);
}

#[test]
fn stored_highlights_color_only_their_quoted_text() {
    let fixture = MouseFixture::new(configuration(true));
    fixture.seed_annotations();
    let mut reader = fixture.reader();
    reader.wait_text("\u{1b}[38;5;223;48;5;52mPassage 0001");
    reader.wait_text("\u{1b}[3;24H\u{1b}[39;49mcontains");
    finish(&mut reader);
}

#[test]
fn overlapping_highlights_color_their_complete_union() {
    let fixture = MouseFixture::new(configuration(true));
    fixture.seed_annotations();
    let mut store = Store::open(&fixture.database).unwrap();
    let book = store.books().unwrap().pop().unwrap();
    let mut annotation = store.annotations(&book.id).unwrap().remove(0);
    annotation.id = uuid::Uuid::new_v4().to_string();
    annotation.anchor = TextAnchor::new(
        annotation.anchor.content_id(),
        annotation.anchor.start() + 8,
        "0001 contains a",
    )
    .unwrap();
    store.save_annotation(&annotation).unwrap();
    let mut reader = fixture.reader();
    reader.wait_text("\u{1b}[38;5;223;48;5;52mPassage 0001 contains a");
    reader.wait_text("\u{1b}[3;35H\u{1b}[39;49mreading");
    finish(&mut reader);
}

#[test]
fn separated_highlights_leave_the_text_between_them_uncolored() {
    let fixture = MouseFixture::new(configuration(true));
    fixture.seed_annotations();
    let mut store = Store::open(&fixture.database).unwrap();
    let book = store.books().unwrap().pop().unwrap();
    let mut annotation = store.annotations(&book.id).unwrap().remove(0);
    annotation.id = uuid::Uuid::new_v4().to_string();
    annotation.anchor = TextAnchor::new(
        annotation.anchor.content_id(),
        annotation.anchor.start() + 24,
        "reading",
    )
    .unwrap();
    store.save_annotation(&annotation).unwrap();
    let mut reader = fixture.reader();
    reader.wait_text("\u{1b}[38;5;223;48;5;52mPassage 0001");
    reader.wait_text("\u{1b}[3;24H\u{1b}[39;49mcontains");
    reader.wait_text("\u{1b}[3;35H\u{1b}[38;5;223;48;5;52mreading");
    finish(&mut reader);
}

#[test]
fn crowded_note_margin_displays_an_overflow_indicator() {
    let fixture = MouseFixture::new(configuration(true));
    fixture.seed_annotations();
    let mut store = Store::open(&fixture.database).unwrap();
    let book = store.books().unwrap().pop().unwrap();
    let original = store.annotations(&book.id).unwrap().remove(0);
    for index in 0..24 {
        let mut annotation = original.clone();
        annotation.id = uuid::Uuid::new_v4().to_string();
        annotation.note = format!("Crowded note {index}");
        store.save_annotation(&annotation).unwrap();
    }
    let mut reader =
        PtyReader::start_sized(&fixture.book, &fixture.configuration_home, false, 160, 30);
    reader.wait_text("More notes below; a: list");
    finish(&mut reader);
}

#[test]
fn notes_that_exactly_fit_the_margin_do_not_report_overflow() {
    let fixture = MouseFixture::new(configuration(true));
    fixture.seed_annotations();
    let mut store = Store::open(&fixture.database).unwrap();
    let book = store.books().unwrap().pop().unwrap();
    let annotations = store.annotations(&book.id).unwrap();
    let original = annotations.first().unwrap();
    for annotation in &annotations {
        store.delete_annotation(annotation).unwrap();
    }
    for index in 0..14 {
        let mut annotation = original.clone();
        annotation.id = uuid::Uuid::from_u128(index + 1).to_string();
        annotation.anchor =
            TextAnchor::new(annotation.anchor.content_id(), 0, "Passage 0000").unwrap();
        annotation.note = format!("NOTE-{index:02}");
        store.save_annotation(&annotation).unwrap();
    }
    let mut reader =
        PtyReader::start_sized(&fixture.book, &fixture.configuration_home, false, 160, 30);
    reader.wait_text("NOTE-13");
    reader.wait_text("row");
    assert!(!reader.saw_output("More notes below; a: list"));
    finish(&mut reader);
}

#[test]
fn right_half_click_turns_a_page() {
    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    reader.send(RIGHT_PAGE_CLICK);
    finish(&mut reader);
    assert!(fixture.saved_position().offset > 100);
}

#[test]
fn left_half_click_returns_to_the_previous_page() {
    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    reader.send(b"l");
    reader.send(LEFT_PAGE_CLICK);
    finish(&mut reader);
    assert_eq!(fixture.saved_position().offset, 0);
}

#[test]
fn dragging_back_to_the_start_does_not_turn_a_page() {
    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    reader.send(b"\x1b[<0;80;10M\x1b[<32;81;10M\x1b[<32;80;10M\x1b[<0;80;10m");
    finish(&mut reader);
    assert_eq!(fixture.saved_position().offset, 0);
}

#[test]
fn clicks_on_a_too_small_terminal_cannot_activate_hidden_dialogs() {
    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    reader.resize(20, 10);
    reader.wait_text("small");
    let checkpoint = reader.output_checkpoint();
    reader.send(&[RIGHT_CLICK, SECOND_ITEM_CLICK, SECOND_ITEM_CLICK, b"j"].concat());
    reader.wait_output_after(checkpoint);
    reader.resize(100, 30);
    finish(&mut reader);
    assert!(fixture.saved_position().content_id.contains("second"));
}

#[test]
fn focus_loss_cancels_a_partially_pressed_mouse_click() {
    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    reader.send(b"\x1b[<0;80;10M\x1b[O\x1b[<0;80;10m");
    finish(&mut reader);
    assert_eq!(fixture.saved_position().offset, 0);
}

#[test]
fn right_click_opens_the_table_of_contents() {
    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    open_contents(&mut reader);
    reader.send(b"q");
    finish(&mut reader);
    assert_eq!(fixture.saved_position().offset, 0);
}

#[test]
fn clicks_queued_during_reflow_cannot_activate_a_new_dialog() {
    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    reader.send(&[CONTROL_WHEEL_UP, b"t", SECOND_ITEM_CLICK, SECOND_ITEM_CLICK].concat());
    reader.wait_text("Second chapter");
    reader.send(b"q");
    finish(&mut reader);
    assert!(fixture.saved_position().content_id.contains("second"));
}

#[test]
fn clicking_a_different_contents_item_once_only_selects_it() {
    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    reader.send(b"t");
    reader.wait_text("Second chapter");
    reader.send(SECOND_ITEM_CLICK);
    reader.send(b"q");
    finish(&mut reader);
    assert!(fixture.saved_position().content_id.contains("second"));
}

#[test]
fn clicking_a_contents_item_twice_opens_its_chapter() {
    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    reader.send(b"t");
    reader.wait_text("Second chapter");
    reader.send(SECOND_ITEM_CLICK);
    reader.send(SECOND_ITEM_CLICK);
    reader.wait_text("FIRST-CHAPTER-TARGET");
    finish(&mut reader);
    assert!(fixture.saved_position().content_id.contains("first"));
}

#[test]
fn clicking_an_annotation_twice_jumps_to_its_passage() {
    let fixture = MouseFixture::new(configuration(true));
    fixture.seed_annotations();
    let mut reader = fixture.reader();
    reader.send(b"a");
    reader.wait_text("SECOND-NOTE-UNIQUE");
    reader.send(SECOND_ITEM_CLICK);
    reader.send(SECOND_ITEM_CLICK);
    finish(&mut reader);
    assert!(fixture.saved_position().offset > 1000);
}

#[test]
fn clicking_a_scrolled_list_item_twice_uses_the_visible_row() {
    let fixture = MouseFixture::new(configuration(true));
    let offsets = fixture.seed_many_annotations();
    let mut reader = fixture.reader();
    reader.send(b"a");
    reader.wait_text("NOTE-0000");
    let checkpoint = reader.output_checkpoint();
    reader.send(&[b'j'; 30]);
    reader.wait_output_after(checkpoint);
    let checkpoint = reader.output_checkpoint();
    reader.send(FIRST_ITEM_CLICK);
    reader.wait_output_after(checkpoint);
    reader.send(FIRST_ITEM_CLICK);
    finish(&mut reader);
    assert_eq!(fixture.saved_position().offset, offsets[5]);
}

#[test]
fn clicking_the_selected_reassociation_source_opens_confirmation() {
    let fixture = MouseFixture::new(configuration(true));
    fixture.seed_annotations();
    let other = fixture.seed_other_book();
    let mut reader = PtyReader::start(&other, &fixture.configuration_home);
    reader.wait_text("OTHER-BOOK-TARGET");
    reader.send(b"ar");
    reader.wait_text("2 annotations");
    reader.send(FIRST_ITEM_CLICK);
    reader.wait_text("Validate");
    reader.send(b"\x03");
    assert!(reader.wait_exit().success());
}

#[test]
fn clicking_a_library_item_twice_opens_that_book() {
    let fixture = MouseFixture::new(configuration(true));
    fixture.seed_other_book();
    let mut reader = fixture.reader();
    reader.send(b"R");
    reader.wait_text("OTHER-LIBRARY-TITLE");
    reader.send(SECOND_ITEM_CLICK);
    reader.send(SECOND_ITEM_CLICK);
    reader.wait_text("OTHER-BOOK-TARGET");
    finish(&mut reader);
}

#[test]
fn wheel_motion_during_a_pending_library_load_selects_the_next_book() {
    let fixture = MouseFixture::new(configuration(true));
    fixture.seed_other_book();
    let mut reader = fixture.reader();
    let lock = rusqlite::Connection::open(&fixture.database).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    reader.send(b"R");
    reader.wait_text("Escape");
    let checkpoint = reader.output_checkpoint();
    reader.send(WHEEL_DOWN);
    reader.wait_output_after(checkpoint);
    lock.execute_batch("COMMIT").unwrap();
    reader.wait_text("OTHER-LIBRARY-TITLE");
    reader.send(b"\r");
    reader.wait_text("OTHER-BOOK-TARGET");
    finish(&mut reader);
}

#[test]
fn right_click_closes_the_contents_without_moving_the_book() {
    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    reader.send(b"t");
    reader.wait_text("Second chapter");
    reader.send(RIGHT_CLICK);
    finish(&mut reader);
    assert_eq!(fixture.saved_position().offset, 0);
}

#[test]
fn control_wheel_enlarges_text_even_without_an_enlarge_key() {
    let mut settings = configuration(true);
    settings.bindings.set(Command::Enlarge, "");
    let fixture = MouseFixture::new(settings);
    let mut reader = fixture.reader();
    reader.send(CONTROL_WHEEL_UP);
    finish(&mut reader);
    assert_eq!(fixture.saved_position().textwidth, 81);
}

#[test]
fn control_wheel_shrinks_text_even_without_a_shrink_key() {
    let mut settings = configuration(true);
    settings.bindings.set(Command::Shrink, "");
    let fixture = MouseFixture::new(settings);
    let mut reader = fixture.reader();
    reader.send(CONTROL_WHEEL_DOWN);
    finish(&mut reader);
    assert_eq!(fixture.saved_position().textwidth, 79);
}

#[test]
fn control_wheel_cannot_change_double_spread_width() {
    let mut settings = configuration(true);
    settings.settings.start_with_double_spread = true;
    let baseline = MouseFixture::new(settings.clone());
    let mut reader = baseline.reader();
    finish(&mut reader);

    let fixture = MouseFixture::new(settings);
    let mut reader = fixture.reader();
    reader.send(CONTROL_WHEEL_DOWN);
    finish(&mut reader);
    assert_eq!(
        fixture.saved_position().textwidth,
        baseline.saved_position().textwidth
    );
}

#[test]
fn a_numeric_prefix_applies_to_wheel_motion() {
    let wheel_fixture = MouseFixture::new(configuration(true));
    let mut wheel_reader = wheel_fixture.reader();
    wheel_reader.send(b"3");
    wheel_reader.send(WHEEL_DOWN);
    finish(&mut wheel_reader);

    let key_fixture = MouseFixture::new(configuration(true));
    let mut key_reader = key_fixture.reader();
    key_reader.send(b"3j");
    finish(&mut key_reader);
    assert_eq!(
        wheel_fixture.saved_position().offset,
        key_fixture.saved_position().offset
    );
}

#[test]
fn wheel_motion_during_reflow_is_saved_before_quitting() {
    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    let mut input = CONTROL_WHEEL_UP.to_vec();
    input.extend_from_slice(WHEEL_DOWN);
    input.extend_from_slice(b"q");
    reader.send(&input);
    assert!(reader.wait_exit().success());
    let position = fixture.saved_position();
    assert_eq!(position.textwidth, 81);
    assert!(position.offset > 0);
}

#[test]
fn disabled_mouse_input_cannot_move_the_reader() {
    let fixture = MouseFixture::new(configuration(false));
    let mut reader = fixture.reader();
    reader.send(WHEEL_DOWN);
    finish(&mut reader);
    assert_eq!(fixture.saved_position().offset, 0);
}

#[test]
fn wheel_selects_the_next_visible_image() {
    let fixture = MouseFixture::with_images(configuration(true));
    let mut reader = fixture.reader();
    reader.send(b"o");
    reader.wait_text("j/k");
    reader.send(WHEEL_DOWN);
    reader.send(b"\r");
    reader.wait_text("second.png");
    reader.send(b"q");
    finish(&mut reader);
}

#[test]
fn a_press_in_loading_cannot_close_the_resulting_definition() {
    let helper = tempfile::tempdir().unwrap();
    let gate = helper.path().join("allow-result");
    let script = helper.path().join("dictionary.sh");
    std::fs::write(
        &script,
        format!(
            "while [ ! -f {} ]; do /bin/sleep 0.01; done\nprintf 'GATED-DEFINITION\\n'\n",
            shell_words::quote(gate.to_str().unwrap())
        ),
    )
    .unwrap();
    let mut settings = configuration(true);
    settings.settings.dictionary_client =
        format!("/bin/sh {}", shell_words::quote(script.to_str().unwrap()));
    let fixture = MouseFixture::new(settings);
    let mut reader = fixture.reader();
    reader.send(b"dword\r");
    reader.wait_text("Loading");
    let checkpoint = reader.output_checkpoint();
    reader.send(b"\x1b[<2;80;10M");
    reader.send(WHEEL_DOWN);
    reader.wait_output_after(checkpoint);
    std::fs::write(gate, "ready").unwrap();
    reader.wait_text("GATED-DEFINITION");
    reader.send(b"\x1b[<2;80;10m");
    reader.send(b"d");
    finish(&mut reader);
}

#[test]
fn a_numeric_prefix_only_applies_to_the_first_queued_wheel_event() {
    let reference = MouseFixture::new(configuration(true));
    let mut reader = reference.reader();
    reader.send(b"+7j");
    finish(&mut reader);
    let expected = reference.saved_position();

    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    reader.send(&[CONTROL_WHEEL_UP, b"5", &WHEEL_DOWN.repeat(3)].concat());
    finish(&mut reader);
    let actual = fixture.saved_position();
    assert_eq!(actual.offset, expected.offset);
    assert_eq!(actual.textwidth, expected.textwidth);
}

#[test]
fn a_wheel_burst_during_reflow_continues_across_a_chapter_boundary() {
    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    reader.send(&[CONTROL_WHEEL_UP, &WHEEL_DOWN.repeat(200)].concat());
    finish(&mut reader);
    let position = fixture.saved_position();
    assert!(position.content_id.contains("notes"), "{position:?}");
}

#[test]
fn wheel_input_does_not_move_an_unfinished_note_cursor() {
    let fixture = MouseFixture::new(configuration(true));
    let mut reader = fixture.reader();
    reader.send(b"v n");
    reader.wait_text("Save");
    reader.send(b"first\rsecond");
    reader.send(WHEEL_UP);
    reader.send(b"-tail\t\r");
    reader.wait_text("Annotation sav");
    reader.send(b"v");
    finish(&mut reader);
    assert_eq!(fixture.saved_notes(), ["first\nsecond-tail"]);
}
