#[path = "support/epub.rs"]
mod epub_support;
#[path = "support/reader.rs"]
mod reader_support;

use ers::app::Viewport;
use ers::app::runtime::{ReaderLaunch, ReaderRuntime};
use ers::book::Book;
use ers::configuration::{Configuration, InputKey};
use ers::document::RenderedDocument;
use ers::document::text::parse_document;
use ers::storage::Store;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct ContinuousFixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
    database: PathBuf,
    documents: Vec<Arc<RenderedDocument>>,
}

impl ContinuousFixture {
    fn new() -> Self {
        Self::with_paragraphs(1)
    }

    fn with_paragraphs(paragraphs: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut builder = epub_support::EpubBuilder::new("3.0");
        for (index, document) in builder.documents.iter_mut().enumerate() {
            document.body = (0..paragraphs)
                .map(|line| format!("<p>Chapter {index} paragraph {line} has café.</p>"))
                .collect();
        }
        let path = builder.build(directory.path()).canonicalize().unwrap();
        let mut book = Book::open(path.clone()).unwrap();
        let contents = book.contents().to_vec();
        let documents = contents
            .iter()
            .enumerate()
            .map(|(index, id)| {
                Arc::new(RenderedDocument::new(
                    index,
                    parse_document(id, &book.raw_text(id).unwrap()).unwrap(),
                    40,
                ))
            })
            .collect();
        let database = directory.path().join("states.db");
        Self {
            _directory: directory,
            path,
            database,
            documents,
        }
    }

    fn runtime(&self, rows: usize) -> ReaderRuntime {
        let mut configuration = Configuration::default();
        configuration.settings.between_chapters = true;
        let mut runtime = ReaderRuntime::start(ReaderLaunch {
            image_decoder: env!("CARGO_BIN_EXE_ers").into(),
            executables: ers::integrations::ExecutableSearch::capture().unwrap(),
            location: self.path.clone().into(),
            database: self.database.clone(),
            configuration,
            viewport: Viewport { columns: 44, rows },
        });
        settle(&mut runtime);
        runtime
    }

    fn lines(&self) -> Vec<String> {
        self.documents
            .iter()
            .flat_map(|document| document.layout.text_lines.iter().cloned())
            .collect()
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
        assert!(
            Instant::now() < deadline,
            "continuous reader did not settle"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn visible_lines(runtime: &ReaderRuntime) -> Vec<String> {
    let model = runtime.model.as_ref().unwrap();
    (0..model.view().viewport.rows * model.view().spread)
        .filter_map(|offset| model.visible_row(offset))
        .map(|visible| visible.document.layout.text_lines[visible.row].clone())
        .collect()
}

fn keys(runtime: &mut ReaderRuntime, text: &str) {
    for character in text.chars() {
        runtime.handle_key(InputKey::from_character(character));
    }
    settle(runtime);
}

#[test]
fn continuous_search_keeps_following_chapters_visible() {
    let fixture = ContinuousFixture::new();
    let mut runtime = fixture.runtime(6);
    runtime.handle_key(InputKey::Character('/'));
    runtime.handle_paste("Chapter 0");
    runtime.handle_key(InputKey::Enter);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        runtime.poll().unwrap();
        let model = runtime.model.as_ref().unwrap();
        if model.search_selection().is_some() && !model.view().loading {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let start = fixture.documents[0].layout.text_lines.len();
    assert_eq!(visible_lines(&runtime), fixture.lines()[start..start + 6]);
}

#[test]
fn continuous_resize_preserves_a_cross_document_boundary_anchor() {
    let fixture = ContinuousFixture::new();
    let mut runtime = fixture.runtime(2);
    keys(
        &mut runtime,
        &(fixture.documents[0].layout.text_lines.len() - 1).to_string(),
    );
    keys(&mut runtime, "j");
    let mut expected = runtime.model.as_ref().unwrap().position();
    runtime.resize(Viewport {
        columns: 34,
        rows: 2,
    });
    settle(&mut runtime);
    expected.textwidth = 30;
    assert_eq!(runtime.model.as_ref().unwrap().position(), expected);
}

#[test]
fn resizing_a_pending_continuous_motion_preserves_its_destination() {
    let fixture = ContinuousFixture::with_paragraphs(150);
    let mut runtime = fixture.runtime(20);
    for key in "99999j".chars() {
        runtime.handle_key(InputKey::from_character(key));
    }
    assert!(runtime.model.as_ref().unwrap().view().loading);
    runtime.resize(Viewport {
        columns: 34,
        rows: 8,
    });
    settle(&mut runtime);
    let model = runtime.model.as_ref().unwrap();
    assert_eq!(model.view().width, 30);
    assert_eq!(model.view().rendered.index, fixture.documents.len() - 1);
    let expected = model
        .view()
        .rendered
        .layout
        .text_lines
        .iter()
        .skip(model.view().rendered.layout.text_lines.len() - 8)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(visible_lines(&runtime), expected);
}

#[test]
fn explicit_continuous_chapter_jumps_preserve_the_requested_top_row() {
    let fixture = ContinuousFixture::new();
    let mut runtime = fixture.runtime(6);
    keys(&mut runtime, "L");
    keys(&mut runtime, "L");
    let expected = fixture
        .documents
        .last()
        .unwrap()
        .layout
        .text_lines
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(visible_lines(&runtime), expected);
    assert_eq!(runtime.model.as_ref().unwrap().position().screen_row, 0);
    keys(&mut runtime, "j");
    assert_eq!(visible_lines(&runtime), expected);
}

#[test]
fn continuous_selection_uses_the_visible_midpoint_document() {
    let fixture = ContinuousFixture::new();
    let mut runtime = fixture.runtime(6);
    keys(&mut runtime, "v ");
    runtime.handle_key(InputKey::Enter);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        runtime.poll().unwrap();
        if !runtime
            .model
            .as_ref()
            .unwrap()
            .view()
            .info
            .annotations
            .is_empty()
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let model = runtime.model.as_ref().unwrap();
    assert_eq!(
        model.view().info.annotations[0].anchor.content_id(),
        fixture.documents[1].parsed.source.content_id
    );
    assert!(
        model.view().info.annotations[0]
            .anchor
            .resolve(&fixture.documents[1].parsed.source)
            .is_some()
    );
}

#[test]
fn continuous_note_drafts_keep_their_document_during_reflow() {
    let fixture = ContinuousFixture::new();
    let mut runtime = fixture.runtime(6);
    keys(&mut runtime, "v n");
    runtime.handle_paste("Draft in the following chapter");
    assert!(
        runtime
            .model
            .as_ref()
            .unwrap()
            .view()
            .info
            .annotations
            .is_empty()
    );
    runtime.resize(Viewport {
        columns: 34,
        rows: 6,
    });
    settle(&mut runtime);
    assert_eq!(
        runtime
            .model
            .as_ref()
            .unwrap()
            .note_editor()
            .unwrap()
            .state
            .text(),
        "Draft in the following chapter"
    );
    runtime.handle_key(InputKey::Tab);
    runtime.handle_key(InputKey::Enter);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        runtime.poll().unwrap();
        if !runtime
            .model
            .as_ref()
            .unwrap()
            .view()
            .info
            .annotations
            .is_empty()
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let annotation = &runtime.model.as_ref().unwrap().view().info.annotations[0];
    assert_eq!(annotation.note, "Draft in the following chapter");
    assert_eq!(
        annotation.anchor.content_id(),
        fixture.documents[1].parsed.source.content_id
    );
    assert!(
        annotation
            .anchor
            .resolve(&fixture.documents[1].parsed.source)
            .is_some()
    );
}

#[test]
fn failed_continuous_motion_is_not_repeated_by_a_later_resize() {
    let mut builder = reader_support::ReaderBuilder::default();
    builder.configuration.settings.between_chapters = true;
    let mut model = builder.build();
    for key in "99999j".chars() {
        model.handle_key(InputKey::from_character(key));
    }
    assert!(model.view().loading);
    model.load_failed("The next chapter is unavailable".into());
    let effects = model.resize(Viewport {
        columns: 100,
        rows: 20,
    });
    let shift = effects
        .into_iter()
        .find_map(|effect| match effect {
            ers::app::ReaderEffect::Window(request) => Some(request.shift_rows),
            _ => None,
        })
        .expect("resize must prepare the current viewport");
    assert_eq!(shift, 0);
}

#[test]
fn continuous_startup_fills_the_viewport_with_the_following_chapter() {
    let fixture = ContinuousFixture::new();
    let runtime = fixture.runtime(6);
    assert_eq!(visible_lines(&runtime), fixture.lines()[..6]);
}

#[test]
fn continuous_line_counts_cross_chapter_boundaries_without_snapping() {
    let fixture = ContinuousFixture::new();
    let mut runtime = fixture.runtime(2);
    let count = fixture.documents[0].layout.text_lines.len() + 1;
    keys(&mut runtime, &format!("{count}j"));
    assert_eq!(visible_lines(&runtime), fixture.lines()[count..count + 2]);
    keys(&mut runtime, "2k");
    assert_eq!(visible_lines(&runtime), fixture.lines()[count - 2..count]);
}

#[test]
fn continuous_book_end_keeps_the_viewport_full() {
    let fixture = ContinuousFixture::new();
    let mut runtime = fixture.runtime(6);
    keys(&mut runtime, "999j");
    let lines = fixture.lines();
    assert_eq!(visible_lines(&runtime), lines[lines.len() - 6..]);
}

#[test]
fn continuous_beginning_returns_to_the_start_of_the_book() {
    let fixture = ContinuousFixture::new();
    let mut runtime = fixture.runtime(2);
    keys(&mut runtime, "L");
    keys(&mut runtime, "g");
    assert_eq!(visible_lines(&runtime), fixture.lines()[..2]);
}

#[test]
fn continuous_boundary_positions_survive_quit_and_reopen() {
    let fixture = ContinuousFixture::new();
    let mut runtime = fixture.runtime(2);
    let count = fixture.documents[0].layout.text_lines.len() - 1;
    keys(&mut runtime, &format!("{count}j"));
    let expected = visible_lines(&runtime);
    let position = runtime.model.as_ref().unwrap().position();
    assert_eq!(
        position.content_id,
        fixture.documents[1].parsed.source.content_id
    );
    assert_eq!(position.screen_row, 1);
    runtime.handle_key(InputKey::Character('q'));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !runtime.is_finished() {
        runtime.poll().unwrap();
        assert!(Instant::now() < deadline);
    }
    drop(runtime);
    let reopened = fixture.runtime(2);
    assert_eq!(visible_lines(&reopened), expected);
    assert_eq!(
        Store::open(&fixture.database)
            .unwrap()
            .reading_state(fixture.path.to_str().unwrap())
            .unwrap()
            .unwrap(),
        position
    );
}
