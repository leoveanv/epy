#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "support/annotations.rs"]
mod annotation_support;
#[path = "support/epub.rs"]
mod epub_support;
#[allow(dead_code)]
#[path = "support/pty.rs"]
mod pty_support;
#[path = "support/screen.rs"]
mod screen_support;

use epub_support::EpubBuilder;
use ers::annotations::{TextAnchor, reading_layout};
use ers::book::Book;
use ers::configuration::Configuration;
use ers::document::RenderedDocument;
use ers::document::text::parse_document;
use ers::storage::Store;
use pty_support::PtyReader;
use screen_support::Screen;
use std::path::PathBuf;

const COLUMNS: usize = 160;
const ROWS: usize = 30;
const TEXT_WIDTH: usize = 80;
const NOTE_WIDTH: usize = 36;
const SOURCE_ROW: usize = 2;

struct NoteFixture {
    _root: tempfile::TempDir,
    book: PathBuf,
    configuration_home: PathBuf,
    database: PathBuf,
}

impl NoteFixture {
    fn new(configuration: Configuration, notes: &[&str]) -> Self {
        Self::with_quote(configuration, notes, "Passage")
    }

    fn with_quote(mut configuration: Configuration, notes: &[&str], quote: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut builder = EpubBuilder::new("3.0");
        let background = (0..12)
            .map(|index| {
                format!(
                    "<p>BACKGROUND-{index:02} {} VISIBLE-END</p>",
                    "words ".repeat(7)
                )
            })
            .collect::<String>();
        builder.documents[1].body =
            "<p>Prologue text.</p><p>Passage to annotate here.</p><p>Concluding text.</p>"
                .to_owned()
                + &background;
        if quote != "Passage" {
            builder.documents[1].body = builder.documents[1]
                .body
                .replace("Passage to annotate here.", quote);
        }
        let book = builder.build(root.path()).canonicalize().unwrap();
        let configuration_home = root.path().join("config");
        let directory = configuration_home.join("ers");
        std::fs::create_dir_all(&directory).unwrap();
        configuration.settings.inline_images = false;
        configuration.settings.page_scroll_animation = false;
        std::fs::write(
            directory.join("configuration.json"),
            configuration.to_json().unwrap(),
        )
        .unwrap();
        let fixture = Self {
            _root: root,
            book,
            configuration_home,
            database: directory.join("states.db"),
        };
        fixture.seed_notes(notes, quote);
        fixture
    }

    fn reader(&self) -> PtyReader {
        self.reader_sized(COLUMNS as u16, ROWS as u16)
    }

    fn reader_sized(&self, columns: u16, rows: u16) -> PtyReader {
        PtyReader::start_sized(&self.book, &self.configuration_home, false, columns, rows)
    }

    fn seed_notes(&self, notes: &[&str], quote: &str) {
        let mut reader = self.reader();
        reader.wait_text("Passage");
        reader.send(b"q");
        assert!(reader.wait_exit().success());

        let mut book = Book::open(self.book.clone()).unwrap();
        let content_id = book.contents()[0].clone();
        let source = parse_document(&content_id, &book.raw_text(&content_id).unwrap()).unwrap();
        let byte_offset = source.source.text.find(quote).unwrap();
        let offset = source.source.text[..byte_offset].chars().count();
        let mut store = Store::open(&self.database).unwrap();
        let book_id = store.books().unwrap().pop().unwrap().id;
        for (index, note) in notes.iter().enumerate() {
            let mut annotation = annotation_support::annotation();
            annotation.id = uuid::Uuid::from_u128(index as u128 + 1).to_string();
            annotation.book_id = book_id.clone();
            annotation.anchor = TextAnchor::new(&content_id, offset, quote).unwrap();
            annotation.note = (*note).into();
            store.save_annotation(&annotation).unwrap();
        }
    }

    fn screen(&self) -> Screen {
        self.screen_sized(COLUMNS as u16, ROWS as u16)
    }

    fn screen_sized(&self, columns: u16, rows: u16) -> Screen {
        let mut reader = self.reader_sized(columns, rows);
        reader.wait_text("NOTE-ONE");
        reader.wait_text("Synthetic");
        Screen::snapshot(&mut reader, columns.into(), rows.into())
    }

    fn highlight_last_row(&self, quote: &str, width: usize) -> usize {
        let mut book = Book::open(self.book.clone()).unwrap();
        let content_id = book.contents()[0].clone();
        let parsed = parse_document(&content_id, &book.raw_text(&content_id).unwrap()).unwrap();
        let offset = parsed.source.text.find(quote).unwrap();
        let end = offset + quote.chars().count();
        let rendered = RenderedDocument::new(0, parsed, width);
        rendered
            .layout
            .source_cells
            .iter()
            .filter(|cell| cell.start < end && cell.end > offset)
            .map(|cell| cell.row)
            .max()
            .unwrap()
    }

    fn editor_screen(&self, columns: u16, rows: u16) -> Screen {
        let mut reader = self.reader_sized(columns, rows);
        reader.wait_text("Passage");
        let checkpoint = reader.output_checkpoint();
        reader.send(b"ae");
        reader.wait_text_after(checkpoint, "\u{1b}[?25h");
        Screen::snapshot(&mut reader, columns.into(), rows.into())
    }
}

fn gutter(screen: &Screen, row: usize) -> String {
    gutter_at_width(screen, row, COLUMNS, TEXT_WIDTH)
}

fn gutter_at_width(screen: &Screen, row: usize, columns: usize, width: usize) -> String {
    let layout = reading_layout(columns, width, NOTE_WIDTH);
    screen
        .line(row)
        .chars()
        .skip(layout.reading_x + width + 1)
        .take(5)
        .collect()
}

fn displaced_notes(configuration: Configuration) -> NoteFixture {
    NoteFixture::new(
        configuration,
        &["NOTE-ONE\nNOTE-ONE-CONTINUATION", "NOTE-TWO"],
    )
}

#[test]
fn aligned_note_has_a_dim_dotted_connector() {
    let fixture = NoteFixture::new(Configuration::default(), &["NOTE-ONE"]);
    let screen = fixture.screen();
    let layout = reading_layout(COLUMNS, TEXT_WIDTH, NOTE_WIDTH);
    assert_eq!(gutter(&screen, SOURCE_ROW), ".... ");
    assert!(screen.dimmed(SOURCE_ROW, layout.reading_x + TEXT_WIDTH + 1));
    assert!(
        screen
            .line(SOURCE_ROW)
            .contains("Passage to annotate here.")
    );
    assert!(screen.line(SOURCE_ROW).contains("NOTE-ONE"));
}

#[test]
fn displaced_note_uses_dotted_elbows_without_covering_notes() {
    let screen = displaced_notes(Configuration::default()).screen();
    assert_eq!(gutter(&screen, SOURCE_ROW), "...╮ ");
    assert_eq!(gutter(&screen, 3), "   ┊ ");
    assert_eq!(gutter(&screen, 4), "   ┊ ");
    assert_eq!(gutter(&screen, 5), "   ╰ ");
    assert!(screen.line(3).contains("NOTE-ONE-CONTINUATION"));
    assert!(screen.line(5).contains("NOTE-TWO"));
}

#[test]
fn ascii_connector_uses_plain_text_for_displaced_notes() {
    let mut configuration = Configuration::default();
    configuration.settings.annotation_ascii_connectors = true;
    let screen = displaced_notes(configuration).screen();
    assert_eq!(gutter(&screen, SOURCE_ROW), "...+ ");
    assert_eq!(gutter(&screen, 3), "   : ");
    assert_eq!(gutter(&screen, 5), "   + ");
}

#[test]
fn wrapped_highlight_connects_from_its_last_visible_row() {
    let quote = "Passage to annotate here. ".repeat(5);
    let quote = quote.trim_end();
    let fixture = NoteFixture::with_quote(Configuration::default(), &["NOTE-ONE"], quote);
    let last = fixture.highlight_last_row(quote, TEXT_WIDTH);
    let screen = fixture.screen();
    assert!(last > SOURCE_ROW);
    assert_eq!(gutter(&screen, last), ".... ");
    assert!(screen.line(last).contains("NOTE-ONE"));
}

#[test]
fn highlight_extending_below_the_viewport_connects_from_the_last_visible_row() {
    let quote = "Passage to annotate here. ".repeat(180);
    let quote = quote.trim_end();
    let fixture = NoteFixture::with_quote(Configuration::default(), &["NOTE-ONE"], quote);
    let screen = fixture.screen();
    let last_visible_row = ROWS - 3;
    assert_eq!(gutter(&screen, last_visible_row), ".... ");
    assert!(screen.line(last_visible_row).contains("NOTE-ONE"));
}

#[test]
fn crowded_connector_lanes_keep_an_explicit_branch_for_each_note() {
    for ascii in [false, true] {
        let mut configuration = Configuration::default();
        configuration.settings.annotation_ascii_connectors = ascii;
        let fixture = NoteFixture::new(
            configuration,
            &[
                "NOTE-ONE",
                "NOTE-TWO",
                "NOTE-THREE",
                "NOTE-FOUR",
                "NOTE-FIVE",
                "NOTE-SIX",
            ],
        );
        let screen = fixture.screen();
        for row in [4, 6, 8, 10, 12] {
            let branch = gutter(&screen, row);
            assert!(
                branch.contains(if ascii { '+' } else { '╰' }) || branch.contains('├'),
                "Missing branch on row {row}: {branch:?}"
            );
        }
    }
}

#[test]
fn notes_sharing_an_anchor_have_separate_connector_lanes() {
    let fixture = NoteFixture::new(
        Configuration::default(),
        &["NOTE-ONE\nCONTINUATION", "NOTE-TWO", "NOTE-THREE"],
    );
    let screen = fixture.screen();
    assert_eq!(gutter(&screen, SOURCE_ROW), ".╮.╮ ");
    assert_eq!(gutter(&screen, 3), " ┊ ┊ ");
    assert_eq!(gutter(&screen, 5), " ┊ ╰ ");
    assert_eq!(gutter(&screen, 7), " ╰.. ");
    assert!(screen.line(5).contains("NOTE-TWO"));
    assert!(screen.line(7).contains("NOTE-THREE"));
}

#[test]
fn ascii_notes_sharing_an_anchor_have_separate_connector_lanes() {
    let mut configuration = Configuration::default();
    configuration.settings.annotation_ascii_connectors = true;
    let fixture = NoteFixture::new(configuration, &["NOTE-ONE", "NOTE-TWO", "NOTE-THREE"]);
    let screen = fixture.screen();
    assert_eq!(gutter(&screen, SOURCE_ROW), ".+.+ ");
    assert_eq!(gutter(&screen, 3), " : : ");
    assert_eq!(gutter(&screen, 4), " : + ");
    assert_eq!(gutter(&screen, 6), " +.. ");
}

#[test]
fn note_editor_border_fits_the_text_region_at_different_terminal_widths() {
    let fixture = NoteFixture::new(Configuration::default(), &["NOTE-ONE"]);
    for (columns, rows) in [(160, 30), (40, 14)] {
        let screen = fixture.editor_screen(columns, rows);
        let height = (rows - 2).min(11);
        let top = rows - height - 1;
        let right = (columns - 2).min(72);
        assert_eq!(
            screen.line(top as usize).chars().nth(right as usize),
            Some('┐')
        );
        assert_eq!(
            screen
                .line((top + height - 1) as usize)
                .chars()
                .nth(right as usize),
            Some('┘')
        );
    }
}

#[test]
fn note_editor_preserves_page_content_outside_its_border() {
    let fixture = NoteFixture::new(Configuration::default(), &["NOTE-ONE"]);
    let screen = fixture.editor_screen(160, 30);
    assert!(
        screen
            .line(18)
            .chars()
            .skip(73)
            .collect::<String>()
            .contains("VISIBLE-END")
    );
}

#[test]
fn note_editor_moves_a_fitting_word_to_the_next_screen_row() {
    let text = format!("NOTE-ONE {} know", "x".repeat(58));
    let fixture = NoteFixture::new(Configuration::default(), &[&text]);
    let screen = fixture.editor_screen(160, 30);
    let first = screen.line(19).chars().skip(2).take(70).collect::<String>();
    let second = screen.line(20).chars().skip(2).take(70).collect::<String>();
    assert_eq!(first.trim_end(), format!("NOTE-ONE {}", "x".repeat(58)));
    assert_eq!(second.trim_end(), "know");
}
