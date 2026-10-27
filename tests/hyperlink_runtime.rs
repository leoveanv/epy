use std::path::PathBuf;
use std::time::{Duration, Instant};

use ers::app::Viewport;
use ers::app::runtime::{ReaderLaunch, ReaderRuntime};
use ers::configuration::{Configuration, InputKey};
use tempfile::TempDir;

#[path = "support/epub.rs"]
mod epub_support;
#[path = "support/fictionbook.rs"]
mod fictionbook_support;
#[allow(dead_code)]
#[path = "support/kindle.rs"]
mod kindle_support;

struct HyperlinkFixture {
    directory: TempDir,
    book: PathBuf,
}

impl HyperlinkFixture {
    fn new(href: &str, target: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut builder = epub_support::EpubBuilder::new("3.0");
        builder.documents[1].body = format!("<p>Start <a href='{href}'>follow this</a></p>");
        builder.documents[0].body = (0..120)
            .map(|index| format!("<p>Lead paragraph {index}</p>"))
            .collect::<String>()
            + target;
        let book = builder.build(directory.path());
        Self::from_book(directory, book)
    }

    fn numbered_notes_and_figure() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut builder = epub_support::EpubBuilder::new("3.0");
        builder.documents[1].body = "<p>Forecast <a href='notes.xhtml#note2'><sup>2</sup></a></p><p>See <a href='#figure'>this page</a>. Correct.<a href='notes.xhtml#note3'><sup>3</sup></a></p><div id='figure'><img class='figure-epub' src='picture.png'/><img class='figure-amzn' src='picture.png'/></div><p>Figure caption.</p>".into();
        builder.documents[2].body =
            "<p id='note2'>Second note destination.</p><p id='note3'>Third note destination.</p>"
                .into();
        let book = builder.build(directory.path());
        Self::from_book(directory, book)
    }

    fn from_book(directory: TempDir, book: PathBuf) -> Self {
        Self { directory, book }
    }

    fn runtime(&self, continuous: bool) -> ReaderRuntime {
        self.runtime_with_images(continuous, true)
    }

    fn runtime_with_images(&self, continuous: bool, inline_images: bool) -> ReaderRuntime {
        let mut configuration = Configuration::default();
        configuration.settings.between_chapters = continuous;
        configuration.settings.inline_images = inline_images;
        let mut runtime = ReaderRuntime::start(ReaderLaunch {
            image_decoder: env!("CARGO_BIN_EXE_ers").into(),
            executables: ers::integrations::ExecutableSearch::capture().unwrap(),
            location: self.book.clone().into(),
            database: self.directory.path().join("states.db"),
            configuration,
            viewport: Viewport {
                columns: 44,
                rows: 6,
            },
        });
        settle(&mut runtime);
        runtime
    }
}

fn settle(runtime: &mut ReaderRuntime) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        runtime.poll().unwrap();
        if runtime
            .model
            .as_ref()
            .is_some_and(|model| !model.view().loading)
        {
            return;
        }
        assert!(Instant::now() < deadline, "reader did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn follow_hint(runtime: &mut ReaderRuntime, number: usize) {
    runtime.handle_key(InputKey::Character('f'));
    for key in number.to_string().chars() {
        runtime.handle_key(InputKey::from_character(key));
    }
    runtime.handle_key(InputKey::Enter);
}

fn follow(runtime: &mut ReaderRuntime) {
    follow_hint(runtime, 1);
}

#[test]
fn external_hyperlinks_emit_one_browser_request_without_a_book_jump() {
    let fixture = HyperlinkFixture::new("https://example.org/path?q=one#part", "<p>Other</p>");
    let mut runtime = fixture.runtime(true);
    let origin = runtime.model.as_ref().unwrap().position();
    follow(&mut runtime);
    settle(&mut runtime);
    assert_eq!(
        runtime.take_browser_request().unwrap().as_str(),
        "https://example.org/path?q=one#part"
    );
    assert!(runtime.take_browser_request().is_none());
    assert_eq!(runtime.model.as_ref().unwrap().position(), origin);
}

#[test]
fn external_hyperlinks_preserve_a_resize_received_during_resolution() {
    let fixture = HyperlinkFixture::new("https://example.org/path", "<p>Other</p>");
    let mut runtime = fixture.runtime(true);
    follow(&mut runtime);
    runtime.resize(Viewport {
        columns: 34,
        rows: 8,
    });
    settle(&mut runtime);
    assert_eq!(runtime.model.as_ref().unwrap().view().rendered.width, 30);
}

#[test]
fn browser_failures_remain_visible_after_pending_reflow() {
    let fixture = HyperlinkFixture::new("https://example.org/path", "<p>Other</p>");
    let mut runtime = fixture.runtime(true);
    follow(&mut runtime);
    settle(&mut runtime);
    assert!(runtime.take_browser_request().is_some());
    runtime.resize(Viewport {
        columns: 34,
        rows: 8,
    });
    runtime.integration_completed(Err("Synthetic OS browser failure".into()));
    settle(&mut runtime);
    assert!(
        runtime
            .model
            .as_ref()
            .unwrap()
            .view()
            .status
            .contains("Synthetic OS browser failure")
    );
}

#[test]
fn reopening_preserves_hyperlink_arrivals_near_a_chapter_end() {
    let fixture = HyperlinkFixture::new(
        "first%20chapter.xhtml#target",
        "<p id='target'>Target café</p>",
    );
    let mut runtime = fixture.runtime(false);
    follow(&mut runtime);
    settle(&mut runtime);
    let position = runtime.model.as_ref().unwrap().position();
    runtime.handle_key(InputKey::Character('q'));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !runtime.is_finished() {
        runtime.poll().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(runtime);
    let reopened = fixture.runtime(false);
    assert_eq!(reopened.model.as_ref().unwrap().position(), position);
}

#[test]
fn height_only_resize_preserves_a_hyperlink_arrival() {
    let fixture = HyperlinkFixture::new(
        "first%20chapter.xhtml#target",
        "<p id='target'>Target café</p>",
    );
    let mut runtime = fixture.runtime(false);
    follow(&mut runtime);
    settle(&mut runtime);
    let position = runtime.model.as_ref().unwrap().position();
    runtime.resize(Viewport {
        columns: 44,
        rows: 12,
    });
    settle(&mut runtime);
    assert_eq!(runtime.model.as_ref().unwrap().position(), position);
}

#[test]
fn fictionbook_hyperlinks_reach_the_notes_body() {
    let directory = tempfile::tempdir().unwrap();
    let path = fictionbook_support::FictionBookBuilder::new().build(directory.path());
    let fixture = HyperlinkFixture::from_book(directory, path);
    let mut runtime = fixture.runtime(false);
    let notes = runtime.model.as_ref().unwrap().view().info.contents[2].clone();
    follow(&mut runtime);
    settle(&mut runtime);
    assert_eq!(runtime.model.as_ref().unwrap().position().content_id, notes);
}

#[test]
fn ambiguous_fictionbook_notes_do_not_choose_the_first_body() {
    let directory = tempfile::tempdir().unwrap();
    let mut builder = fictionbook_support::FictionBookBuilder::new();
    builder
        .first_body
        .push_str("<section id='note-one'><p>Duplicate note</p></section>");
    let path = builder.build(directory.path());
    let fixture = HyperlinkFixture::from_book(directory, path);
    let mut runtime = fixture.runtime(false);
    let origin = runtime.model.as_ref().unwrap().position();
    follow(&mut runtime);
    settle(&mut runtime);
    assert_eq!(runtime.model.as_ref().unwrap().position(), origin);
    assert!(
        runtime
            .model
            .as_ref()
            .unwrap()
            .view()
            .status
            .contains("missing or ambiguous")
    );
}

#[test]
fn kindle_filepos_hyperlinks_reach_their_byte_addressed_destination() {
    let directory = tempfile::tempdir().unwrap();
    let mut builder = kindle_support::MobiBuilder::new();
    let markup = format!(
        "<html><body><p><a filepos='0000000000'>Jump</a></p>{}<p id='target'>Kindle destination</p></body></html>",
        "<p>Prélude café 👩‍🔬</p>".repeat(120)
    );
    let offset = markup.find("<p id='target'>").unwrap();
    builder.html = markup
        .replacen("0000000000", &format!("{offset:010}"), 1)
        .into_bytes();
    let path = builder.build(directory.path(), "mobi");
    let fixture = HyperlinkFixture::from_book(directory, path);
    let mut runtime = fixture.runtime(false);
    follow(&mut runtime);
    settle(&mut runtime);
    let model = runtime.model.as_ref().unwrap();
    let visible = model.visible_row(0).unwrap();
    assert_eq!(
        visible.document.layout.text_lines[visible.row],
        "Kindle destination"
    );
}

#[test]
fn interrupting_pending_external_links_does_not_launch_a_browser() {
    let fixture = HyperlinkFixture::new("https://example.org/path", "<p>Other</p>");
    let mut runtime = fixture.runtime(true);
    follow(&mut runtime);
    runtime.handle_key(InputKey::Character('\u{03}'));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !runtime.is_finished() {
        runtime.poll().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(runtime.take_browser_request().is_none());
}

#[test]
fn first_reference_hint_opens_the_second_note() {
    let fixture = HyperlinkFixture::numbered_notes_and_figure();
    let mut runtime = fixture.runtime(false);
    follow_hint(&mut runtime, 1);
    settle(&mut runtime);
    let model = runtime.model.as_ref().unwrap();
    let visible = model.visible_row(0).unwrap();
    assert_eq!(
        visible.document.layout.text_lines[visible.row],
        "Second note destination."
    );
}

#[test]
fn third_reference_hint_opens_the_third_note() {
    let fixture = HyperlinkFixture::numbered_notes_and_figure();
    let mut runtime = fixture.runtime(false);
    follow_hint(&mut runtime, 3);
    settle(&mut runtime);
    let model = runtime.model.as_ref().unwrap();
    let visible = model.visible_row(0).unwrap();
    assert_eq!(
        visible.document.layout.text_lines[visible.row],
        "Third note destination."
    );
}

#[test]
fn hyperlink_to_a_figure_lands_on_the_image() {
    for continuous in [false, true] {
        for inline_images in [false, true] {
            let fixture = HyperlinkFixture::numbered_notes_and_figure();
            let mut runtime = fixture.runtime_with_images(continuous, inline_images);
            follow_hint(&mut runtime, 2);
            settle(&mut runtime);
            let model = runtime.model.as_ref().unwrap();
            let visible = model.visible_row(0).unwrap();
            assert_eq!(
                visible
                    .document
                    .layout
                    .image_maps
                    .get(&visible.row)
                    .map(String::as_str),
                Some("picture.png"),
                "continuous={continuous}, inline_images={inline_images}"
            );
        }
    }
}

#[test]
fn internal_hyperlinks_reach_their_exact_fragment() {
    for continuous in [false, true] {
        let fixture = HyperlinkFixture::new(
            "first%20chapter.xhtml#target",
            "<p id='target'>Target café</p>",
        );
        let mut runtime = fixture.runtime(continuous);
        follow(&mut runtime);
        settle(&mut runtime);
        let model = runtime.model.as_ref().unwrap();
        assert_eq!(
            model.visible_row(0).unwrap().document.layout.text_lines
                [model.visible_row(0).unwrap().row],
            "Target café"
        );
        assert_eq!(model.position().content_id, "OPS/first chapter.xhtml");
    }
}

#[test]
fn hyperlink_arrivals_are_available_in_jump_history() {
    let fixture = HyperlinkFixture::new(
        "first%20chapter.xhtml#target",
        "<p id='target'>Target café</p>",
    );
    let mut runtime = fixture.runtime(true);
    let origin = runtime.model.as_ref().unwrap().position();
    follow(&mut runtime);
    settle(&mut runtime);
    runtime.handle_key(InputKey::Character('\u{0f}'));
    settle(&mut runtime);
    assert_eq!(runtime.model.as_ref().unwrap().position(), origin);
}

#[test]
fn missing_hyperlink_fragments_leave_the_departure_position_unchanged() {
    let fixture = HyperlinkFixture::new(
        "first%20chapter.xhtml#absent",
        "<p id='target'>Target café</p>",
    );
    let mut runtime = fixture.runtime(false);
    let origin = runtime.model.as_ref().unwrap().position();
    follow(&mut runtime);
    settle(&mut runtime);
    assert_eq!(runtime.model.as_ref().unwrap().position(), origin);
    assert!(
        runtime
            .model
            .as_ref()
            .unwrap()
            .view()
            .status
            .contains("missing or ambiguous")
    );
}

#[test]
fn ambiguous_hyperlink_fragments_do_not_choose_the_first_anchor() {
    let fixture = HyperlinkFixture::new(
        "first%20chapter.xhtml#target",
        "<p id='target'>One</p><p id='target'>Two</p>",
    );
    let mut runtime = fixture.runtime(true);
    let origin = runtime.model.as_ref().unwrap().position();
    follow(&mut runtime);
    settle(&mut runtime);
    assert_eq!(runtime.model.as_ref().unwrap().position(), origin);
    assert!(
        runtime
            .model
            .as_ref()
            .unwrap()
            .view()
            .status
            .contains("missing or ambiguous")
    );
}

#[test]
fn resizing_a_pending_hyperlink_keeps_its_destination() {
    let fixture = HyperlinkFixture::new(
        "first%20chapter.xhtml#target",
        "<p id='target'>Target café</p>",
    );
    let mut runtime = fixture.runtime(true);
    follow(&mut runtime);
    runtime.resize(Viewport {
        columns: 34,
        rows: 8,
    });
    settle(&mut runtime);
    let model = runtime.model.as_ref().unwrap();
    assert_eq!(model.position().content_id, "OPS/first chapter.xhtml");
    assert_eq!(model.view().width, 30);
    assert_eq!(
        model.visible_row(0).unwrap().document.layout.text_lines[model.visible_row(0).unwrap().row],
        "Target café"
    );
}
