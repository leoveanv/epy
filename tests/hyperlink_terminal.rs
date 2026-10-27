#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "support/epub.rs"]
mod epub_support;
#[allow(dead_code)]
#[path = "support/pty.rs"]
mod pty_support;

use epub_support::EpubBuilder;
use ers::configuration::{Command, Configuration};
use ers::document::text::cell_width;
use pty_support::PtyReader;
use std::path::PathBuf;

struct LinkFixture {
    _root: tempfile::TempDir,
    book: PathBuf,
    configuration_home: PathBuf,
}

impl LinkFixture {
    fn new(body: &str, next_body: &str, mut configuration: Configuration) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut builder = EpubBuilder::new("3.0");
        builder.documents[1].body = body.into();
        builder.documents[0].body = next_body.into();
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
        Self {
            _root: root,
            book,
            configuration_home,
        }
    }

    fn wrapped() -> Self {
        Self::new(
            "<p>Intro <a href='#figure'>FIRSTWORD words words words words words words words LASTWORD</a> <a epub:type='noteref' href='#note'><sup>3</sup></a></p>",
            "<p>Next</p>",
            Configuration::default(),
        )
    }

    fn continuous() -> Self {
        let mut configuration = Configuration::default();
        configuration.settings.between_chapters = true;
        Self::new(
            "<p><a href='#one'>FIRSTLINK</a></p>",
            "<p><a href='#two'>NEXTLINK</a></p>",
            configuration,
        )
    }

    fn spread() -> Self {
        let mut configuration = Configuration::default();
        configuration.settings.start_with_double_spread = true;
        let body = "<p><a href='#one'>LEFTLINK</a></p>".to_owned()
            + &(0..20)
                .map(|index| format!("<p>Filler {index:03}</p>"))
                .collect::<String>()
            + "<p><a href='#two'>RIGHTLINK</a></p>";
        Self::new(&body, "<p>Next</p>", configuration)
    }

    fn reader(&self, columns: u16, rows: u16) -> PtyReader {
        PtyReader::start_sized(&self.book, &self.configuration_home, false, columns, rows)
    }

    fn screen(&self, columns: u16, rows: u16, marker: &str) -> Screen {
        let mut reader = self.reader(columns, rows);
        reader.wait_text(marker);
        Screen::snapshot(&mut reader, columns.into(), rows.into())
    }
}

#[derive(Clone)]
struct ScreenCell {
    text: String,
    underlined: bool,
}

impl Default for ScreenCell {
    fn default() -> Self {
        Self {
            text: " ".into(),
            underlined: false,
        }
    }
}

struct Screen {
    cells: Vec<ScreenCell>,
    columns: usize,
    rows: usize,
    column: usize,
    row: usize,
    underlined: bool,
}

impl Screen {
    fn snapshot(reader: &mut PtyReader, columns: usize, rows: usize) -> Self {
        let mut screen = Self {
            cells: vec![ScreenCell::default(); columns * rows],
            columns,
            rows,
            column: 0,
            row: 0,
            underlined: false,
        };
        vte::Parser::new().advance(&mut screen, &reader.output_since(0));
        screen
    }

    fn line(&self, row: usize) -> String {
        self.cells[row * self.columns..(row + 1) * self.columns]
            .iter()
            .map(|cell| cell.text.as_str())
            .collect()
    }

    fn text_style(&self, text: &str) -> Option<Vec<bool>> {
        self.cells.chunks(self.columns).find_map(|cells| {
            let line = cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>();
            let start = line.find(text)?;
            let start = line[..start].chars().count();
            Some(
                cells
                    .iter()
                    .flat_map(|cell| cell.text.chars().map(|_| cell.underlined))
                    .skip(start)
                    .take(text.chars().count())
                    .collect(),
            )
        })
    }

    fn clear_line(&mut self, start: usize, end: usize) {
        let base = self.row * self.columns;
        self.cells[base + start..base + end].fill(ScreenCell::default());
    }
}

impl vte::Perform for Screen {
    fn print(&mut self, character: char) {
        let text = character.to_string();
        let width = cell_width(&text);
        if width == 0 {
            if self.column > 0 && self.row < self.rows {
                self.cells[self.row * self.columns + self.column.min(self.columns) - 1]
                    .text
                    .push(character);
            }
        } else if self.row < self.rows && self.column < self.columns {
            self.cells[self.row * self.columns + self.column] = ScreenCell {
                text,
                underlined: self.underlined,
            };
            self.column += width;
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\r' => self.column = 0,
            b'\n' => self.row = (self.row + 1).min(self.rows - 1),
            b'\x08' => self.column = self.column.saturating_sub(1),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], _: bool, action: char) {
        if !intermediates.is_empty() {
            return;
        }
        let values = params.iter().map(|part| part[0]).collect::<Vec<_>>();
        let first = usize::from(*values.first().unwrap_or(&0));
        let amount = first.max(1);
        match action {
            'H' | 'f' => {
                self.row = amount.saturating_sub(1).min(self.rows - 1);
                self.column = usize::from(*values.get(1).unwrap_or(&1))
                    .max(1)
                    .saturating_sub(1)
                    .min(self.columns - 1);
            }
            'G' => self.column = amount.saturating_sub(1).min(self.columns - 1),
            'd' => self.row = amount.saturating_sub(1).min(self.rows - 1),
            'A' => self.row = self.row.saturating_sub(amount),
            'B' => self.row = (self.row + amount).min(self.rows - 1),
            'C' => self.column = (self.column + amount).min(self.columns - 1),
            'D' => self.column = self.column.saturating_sub(amount),
            'J' if first == 2 => self.cells.fill(ScreenCell::default()),
            'K' => match first {
                0 => self.clear_line(self.column.min(self.columns), self.columns),
                1 => self.clear_line(0, (self.column + 1).min(self.columns)),
                2 => self.clear_line(0, self.columns),
                _ => {}
            },
            'm' => {
                if values.is_empty() {
                    self.underlined = false;
                }
                for value in values {
                    match value {
                        0 | 24 => self.underlined = false,
                        4 => self.underlined = true,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn underlined(screen: &Screen, text: &str) -> bool {
    screen
        .text_style(text)
        .is_some_and(|style| style.into_iter().all(|value| value))
}

#[test]
fn link_text_is_underlined_without_changing_neighboring_text() {
    let fixture = LinkFixture::new(
        "<p>BEFORE <a href='#figure'>LINKED</a> AFTER</p>",
        "<p>Next</p>",
        Configuration::default(),
    );
    let mut reader = fixture.reader(100, 30);
    reader.wait_text("Synthetic");
    let screen = Screen::snapshot(&mut reader, 100, 30);
    assert!(underlined(&screen, "LINKED"));
    assert_eq!(screen.text_style("BEFORE"), Some(vec![false; 6]));
    assert_eq!(screen.text_style("AFTER"), Some(vec![false; 5]));
    assert!(screen.line(0).contains("BEFORE LINKED AFTER"));
}

#[test]
fn wrapped_link_text_remains_underlined_on_each_line() {
    let screen = LinkFixture::wrapped().screen(44, 16, "Synthetic");
    assert!(underlined(&screen, "FIRSTWORD"));
    assert!(underlined(&screen, "LASTWORD"));
}

#[test]
fn projected_citation_text_is_underlined() {
    let screen = LinkFixture::wrapped().screen(44, 16, "Synthetic");
    assert!(underlined(&screen, "[3]"));
}

#[test]
fn wrapped_link_counts_once_in_the_follow_hint() {
    let screen = LinkFixture::wrapped().screen(44, 16, "Synthetic");
    assert!(screen.line(15).contains("2 links · f: follow"));
}

#[test]
fn visible_links_show_the_configured_follow_key_with_progress() {
    let mut configuration = Configuration::default();
    configuration.bindings.set(Command::Follow, "lz");
    let fixture = LinkFixture::new(
        "<p><a href='#one'>FIRST</a> and <a href='#two'>SECOND</a></p>",
        "<p>Next</p>",
        configuration,
    );
    let mut reader = fixture.reader(100, 30);
    reader.wait_text("Synthetic");
    let screen = Screen::snapshot(&mut reader, 100, 30);
    let status = screen.line(29);
    assert!(status.contains("2 links · z: follow"), "{status:?}");
    assert!(status.contains("Synthetic"), "{status:?}");
    reader.send(b"z");
    reader.wait_text("Esc cancels");
    assert!(
        Screen::snapshot(&mut reader, 100, 30)
            .line(29)
            .contains("Follow link:")
    );
}

#[test]
fn empty_follow_binding_displays_the_builtin_enter_key() {
    let mut configuration = Configuration::default();
    configuration.bindings.set(Command::Follow, "");
    let fixture = LinkFixture::new(
        "<p><a href='#one'>LINKED</a></p>",
        "<p>Next</p>",
        configuration,
    );
    let mut reader = fixture.reader(100, 30);
    reader.wait_text("Synthetic");
    let screen = Screen::snapshot(&mut reader, 100, 30);
    assert!(screen.line(29).contains("1 link · Enter: follow"));
}

#[test]
fn link_hint_remains_visible_when_progress_is_disabled() {
    let mut configuration = Configuration::default();
    configuration.settings.show_progress_indicator = false;
    let fixture = LinkFixture::new(
        "<p><a href='#one'>LINKED</a></p>",
        "<p>Next</p>",
        configuration,
    );
    let mut reader = fixture.reader(100, 30);
    reader.wait_text("LINKED");
    let screen = Screen::snapshot(&mut reader, 100, 30);
    assert!(screen.line(29).contains("1 link · f: follow"));
}

#[test]
fn scrolling_past_links_removes_the_follow_hint() {
    let body = "<p><a href='#one'>TOPLINK</a></p>".to_owned()
        + &(0..100)
            .map(|index| format!("<p>Filler {index:03}</p>"))
            .collect::<String>()
        + "<p>END_MARKER</p>";
    let fixture = LinkFixture::new(&body, "<p>Next</p>", Configuration::default());
    let mut reader = fixture.reader(100, 30);
    reader.wait_text("Synthetic");
    assert!(
        Screen::snapshot(&mut reader, 100, 30)
            .line(29)
            .contains("1 link · f: follow")
    );
    reader.send(b"G");
    reader.wait_text("END_MARKER");
    let status = Screen::snapshot(&mut reader, 100, 30).line(29);
    assert!(!status.contains("link"), "{status:?}");
}

#[test]
fn links_in_the_following_continuous_chapter_are_underlined() {
    let screen = LinkFixture::continuous().screen(100, 30, "NEXTLINK");
    assert!(underlined(&screen, "NEXTLINK"));
}

#[test]
fn continuous_chapter_links_are_counted_in_the_follow_hint() {
    let screen = LinkFixture::continuous().screen(100, 30, "NEXTLINK");
    assert!(screen.line(29).contains("2 links · f: follow"));
}

#[test]
fn links_on_the_second_side_of_a_spread_are_underlined() {
    let screen = LinkFixture::spread().screen(100, 30, "Synthetic");
    assert!(underlined(&screen, "RIGHTLINK"));
}

#[test]
fn links_on_both_sides_of_a_spread_are_counted_in_the_follow_hint() {
    let screen = LinkFixture::spread().screen(100, 30, "Synthetic");
    assert!(screen.line(29).contains("2 links · f: follow"));
}
