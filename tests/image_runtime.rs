#[path = "support/epub.rs"]
mod epub_support;

use epub_support::{AssetInput, EpubBuilder};
use ers::app::modes::images::ImageMode;
use ers::app::runtime::{ReaderLaunch, ReaderRuntime};
use ers::app::{ReaderMode, Viewport};
use ers::configuration::{Configuration, InputKey};
#[path = "support/images.rs"]
mod image_support;
use image_support::ImageBuilder;
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct ImageFixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
    database: PathBuf,
}

impl ImageFixture {
    fn new() -> Self {
        Self::with_first_document("<p>First prose</p><img src='left.png'/>")
    }
    fn with_first_document(body: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut builder = EpubBuilder::new("3.0");
        builder.navigation = Some("<html xmlns='http://www.w3.org/1999/xhtml' xmlns:epub='http://www.idpf.org/2007/ops'><body><nav epub:type='toc'><ol><li><a href='second.xhtml'>First</a></li><li><a href='parts/first.xhtml'>Second</a></li></ol></nav></body></html>".into());
        builder.documents[1].body = body.into();
        builder.documents[0].href = "parts/first.xhtml".into();
        builder.documents[0].body = "<img src='right.png'/><p>Following prose</p>".into();
        for (path, width) in [("left.png", 4), ("parts/right.png", 7)] {
            let mut image = ImageBuilder::new();
            image.width = width;
            builder
                .assets
                .push(AssetInput::new(path, "image/png", image.bytes()));
        }
        let path = builder.build(directory.path());
        let database = directory.path().join("states.db");
        Self {
            _directory: directory,
            path,
            database,
        }
    }
    fn runtime(&self) -> ReaderRuntime {
        self.runtime_with_inline_images(true)
    }

    fn runtime_with_inline_images(&self, inline_images: bool) -> ReaderRuntime {
        let mut configuration = Configuration::default();
        configuration.settings.between_chapters = true;
        configuration.settings.inline_images = inline_images;
        let mut runtime = ReaderRuntime::start(ReaderLaunch {
            image_decoder: env!("CARGO_BIN_EXE_ers").into(),
            executables: ers::integrations::ExecutableSearch::capture().unwrap(),
            location: self.path.clone().into(),
            database: self.database.clone(),
            configuration,
            viewport: Viewport {
                columns: 44,
                rows: 24,
            },
        });
        wait_for(&mut runtime, |runtime| {
            runtime
                .model
                .as_ref()
                .is_some_and(|model| !model.view().loading)
        });
        runtime
    }
}

fn wait_for(runtime: &mut ReaderRuntime, ready: impl Fn(&ReaderRuntime) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        runtime.poll().unwrap();
        if ready(runtime) {
            return;
        }
        assert!(Instant::now() < deadline, "The image reader did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn inline_images_follow_the_configured_slot_height() {
    let fixture = ImageFixture::new();
    for (enabled, height) in [(true, 10), (false, 1)] {
        let runtime = fixture.runtime_with_inline_images(enabled);
        let model = runtime.model.as_ref().unwrap();
        let row = *model
            .view()
            .rendered
            .layout
            .image_maps
            .keys()
            .next()
            .unwrap();
        let mapping = model.view().rendered.layout.text_lines.image_layout();
        assert!(mapping.slot_at(row + height - 1).is_some());
        assert!(mapping.slot_at(row + height).is_none());
    }
}

#[test]
fn continuous_jumps_restore_partial_images_in_their_own_documents() {
    let fixture = ImageFixture::new();
    let mut runtime = fixture.runtime();
    runtime.resize(Viewport {
        columns: 44,
        rows: 4,
    });
    wait_for(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    let row = *runtime
        .model
        .as_ref()
        .unwrap()
        .view()
        .rendered
        .layout
        .image_maps
        .keys()
        .next()
        .unwrap()
        + 7;
    for key in format!("{row}jm1").chars() {
        runtime.handle_key(InputKey::from_character(key));
    }
    wait_for(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    let marked = runtime.model.as_ref().unwrap().view_position().image;
    runtime.handle_key(InputKey::Character('L'));
    wait_for(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    for key in "3j".chars() {
        runtime.handle_key(InputKey::from_character(key));
    }
    let departure = runtime.model.as_ref().unwrap().view_position().image;
    for key in "`1".chars() {
        runtime.handle_key(InputKey::from_character(key));
    }
    wait_for(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    assert_eq!(
        runtime.model.as_ref().unwrap().view_position().image,
        marked
    );
    runtime.handle_key(InputKey::Character('\u{0f}'));
    wait_for(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    assert_eq!(
        runtime.model.as_ref().unwrap().view_position().image,
        departure
    );
}

#[test]
fn following_chapter_search_hits_stay_below_expanded_image_slots() {
    let fixture = ImageFixture::new();
    let mut runtime = fixture.runtime();
    for key in "/Following prose\n".chars() {
        runtime.handle_key(InputKey::from_character(key));
    }
    wait_for(&mut runtime, |runtime| {
        runtime.model.as_ref().unwrap().search_selection().is_some()
    });
    let selection = runtime.model.as_ref().unwrap().search_selection().unwrap();
    let image_row = *selection.document.layout.image_maps.keys().next().unwrap();
    assert_eq!(selection.matches.hits.len(), 1);
    assert!(selection.matches.hits[0].row >= image_row + 10);
}

#[test]
fn inline_image_reflow_keeps_selection_on_visible_text() {
    let fixture = ImageFixture::new();
    let mut runtime = fixture.runtime();
    runtime.handle_key(InputKey::Character('L'));
    wait_for(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    runtime.handle_key(InputKey::Character('v'));
    for columns in [44, 28] {
        runtime.resize(Viewport { columns, rows: 24 });
        wait_for(&mut runtime, |runtime| {
            !runtime.model.as_ref().unwrap().view().loading
        });
        let model = runtime.model.as_ref().unwrap();
        let selected = model
            .active_selection()
            .unwrap()
            .cell(model.selection_surface())
            .unwrap();
        let displayed = model
            .view()
            .rendered
            .layout
            .text_lines
            .iter()
            .position(|line| line.contains("Following prose"))
            .unwrap();
        assert_eq!(selected.source.row, displayed);
        assert_eq!(selected.source.text, "F");
    }
}

#[test]
fn new_books_allow_selecting_leading_images() {
    let fixture = ImageFixture::with_first_document("<img src='left.png'/><p>First prose</p>");
    let mut runtime = fixture.runtime();
    runtime.handle_key(InputKey::Character('o'));
    assert!(
        matches!(runtime.model.as_ref().unwrap().view().mode, ReaderMode::Images(mode) if matches!(mode.as_ref(), ImageMode::Selecting(selection) if selection.targets.iter().any(|target| target.path == "left.png")))
    );
}

#[test]
fn resizing_dismisses_stale_image_selection() {
    let fixture = ImageFixture::new();
    let mut runtime = fixture.runtime();
    runtime.handle_key(InputKey::Character('o'));
    runtime.resize(Viewport {
        columns: 34,
        rows: 8,
    });
    wait_for(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    assert!(matches!(
        runtime.model.as_ref().unwrap().view().mode,
        ReaderMode::Reading
    ));
}

#[test]
fn image_selection_preserves_the_reading_position() {
    let fixture = ImageFixture::new();
    let mut runtime = fixture.runtime();
    let before = runtime.model.as_ref().unwrap().position();
    runtime.handle_key(InputKey::Character('o'));
    assert!(matches!(
        runtime.model.as_ref().unwrap().view().mode,
        ReaderMode::Images(_)
    ));
    runtime.handle_key(InputKey::Escape);
    assert_eq!(runtime.model.as_ref().unwrap().position(), before);
    assert!(matches!(
        runtime.model.as_ref().unwrap().view().mode,
        ReaderMode::Reading
    ));
}

#[test]
fn selecting_following_chapter_images_uses_their_own_resource_context() {
    let fixture = ImageFixture::new();
    let mut runtime = fixture.runtime();
    runtime.handle_key(InputKey::Character('o'));
    runtime.handle_key(InputKey::Character('j'));
    runtime.handle_key(InputKey::Enter);
    wait_for(
        &mut runtime,
        |runtime| matches!(runtime.model.as_ref().unwrap().view().mode, ReaderMode::Images(mode) if matches!(mode.as_ref(), ImageMode::Preview(_))),
    );
    let ReaderMode::Images(mode) = runtime.model.as_ref().unwrap().view().mode else {
        panic!("Image mode");
    };
    let ImageMode::Preview(preview) = mode.as_ref() else {
        panic!("Image preview");
    };
    assert_eq!(preview.payload.name, "right.png");
    assert_eq!(preview.payload.decoded.as_ref().unwrap().pixels.width(), 7);
}

#[test]
fn pasted_image_choices_do_not_open_previews() {
    let fixture = ImageFixture::new();
    let mut runtime = fixture.runtime();
    runtime.handle_key(InputKey::Character('o'));
    runtime.handle_paste("j\n");
    assert!(
        matches!(runtime.model.as_ref().unwrap().view().mode, ReaderMode::Images(mode) if matches!(mode.as_ref(), ImageMode::Selecting(selection) if selection.selected == 0))
    );
}

#[test]
fn canceled_image_loads_cannot_reactivate_a_preview() {
    let fixture = ImageFixture::new();
    let mut runtime = fixture.runtime();
    runtime.handle_key(InputKey::Character('o'));
    runtime.handle_key(InputKey::Enter);
    runtime.handle_key(InputKey::Escape);
    for _ in 0..20 {
        runtime.poll().unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(matches!(
        runtime.model.as_ref().unwrap().view().mode,
        ReaderMode::Reading
    ));
}

#[test]
fn resizing_an_image_preview_preserves_the_book_anchor() {
    let fixture = ImageFixture::new();
    let mut runtime = fixture.runtime();
    let before = runtime.model.as_ref().unwrap().position();
    runtime.handle_key(InputKey::Character('o'));
    runtime.handle_key(InputKey::Enter);
    wait_for(
        &mut runtime,
        |runtime| matches!(runtime.model.as_ref().unwrap().view().mode, ReaderMode::Images(mode) if matches!(mode.as_ref(), ImageMode::Preview(_))),
    );
    runtime.resize(Viewport {
        columns: 34,
        rows: 8,
    });
    wait_for(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    let model = runtime.model.as_ref().unwrap();
    assert_eq!(model.position().content_id, before.content_id);
    assert_eq!(model.position().offset, before.offset);
    assert!(
        matches!(model.view().mode, ReaderMode::Images(mode) if matches!(mode.as_ref(), ImageMode::Preview(_)))
    );
}
