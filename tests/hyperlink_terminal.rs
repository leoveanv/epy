#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "support/epub.rs"]
mod epub_support;
#[allow(dead_code)]
#[path = "support/pty.rs"]
mod pty_support;
#[path = "support/screen.rs"]
mod screen_support;

use epub_support::EpubBuilder;
use ers::configuration::{Command, Configuration};
use pty_support::PtyReader;
use screen_support::Screen;
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
