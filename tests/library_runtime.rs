#[path = "support/epub.rs"]
mod epub_support;
#[path = "support/pending_web.rs"]
mod pending_web;

use epub_support::EpubBuilder;
use ers::app::modes::library::LibraryPhase;
use ers::app::requests::{LibraryCommand, LibraryJob};
use ers::app::runtime::{ReaderLaunch, ReaderRuntime};
use ers::app::workers::{ReaderJobs, WorkerEvent};
use ers::app::{ReaderMode, Viewport};
use ers::book::Book;
use ers::book::LibraryEntry;
use ers::configuration::{Configuration, InputKey};
use ers::reading::ReadingState;
use ers::reading::SaveRequest;
use ers::storage::Store;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct LibraryFixture {
    _directory: tempfile::TempDir,
    database: PathBuf,
    first: PathBuf,
    second: PathBuf,
}

impl LibraryFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let first = build_book(directory.path(), "first");
        let second = build_book(directory.path(), "second");
        let database = directory.path().join("states.db");
        let mut fixture = Self {
            _directory: directory,
            database,
            first,
            second,
        };
        fixture.seed_second();
        fixture
    }

    fn seed_second(&mut self) {
        let book = Book::open(self.second.clone()).unwrap();
        let position = ReadingState {
            content_id: book.contents()[0].clone(),
            textwidth: 80,
            ..ReadingState::default()
        };
        Store::open(&self.database)
            .unwrap()
            .save_session(&history(&self.second), &position)
            .unwrap();
    }

    fn runtime(&self) -> ReaderRuntime {
        let mut runtime = ReaderRuntime::start(ReaderLaunch {
            image_decoder: env!("CARGO_BIN_EXE_ers").into(),
            executables: ers::integrations::ExecutableSearch::capture().unwrap(),
            location: self.first.clone().into(),
            database: self.database.clone(),
            configuration: Configuration::default(),
            viewport: Viewport {
                columns: 100,
                rows: 20,
            },
        });
        wait(&mut runtime, |runtime| runtime.model.is_some());
        runtime
    }

    fn position(&self, path: &Path) -> ReadingState {
        Store::open(&self.database)
            .unwrap()
            .reading_state(path.to_str().unwrap())
            .unwrap()
            .unwrap()
    }
}

fn build_book(directory: &Path, name: &str) -> PathBuf {
    let directory = directory.join(name);
    std::fs::create_dir_all(&directory).unwrap();
    let mut builder = EpubBuilder::new("3.0");
    builder.title = name.into();
    for document in &mut builder.documents {
        document.body = (0..150)
            .map(|index| format!("<p>{name} line {index} has readable words.</p>"))
            .collect();
    }
    builder.build(&directory).canonicalize().unwrap()
}

fn history(path: &Path) -> LibraryEntry {
    LibraryEntry {
        filepath: path.to_str().unwrap().into(),
        last_read: "2000-01-01 00:00:00".into(),
        title: Some("Saved book".into()),
        author: None,
        reading_progress: Some(0.0),
    }
}

fn wait(runtime: &mut ReaderRuntime, predicate: impl Fn(&ReaderRuntime) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        runtime.poll().unwrap();
        if predicate(runtime) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Reader did not settle: {}",
            runtime
                .model
                .as_ref()
                .map_or("opening", |model| model.view().status)
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn keys(runtime: &mut ReaderRuntime, text: &str) {
    for character in text.chars() {
        runtime.handle_key(InputKey::from_character(character));
    }
}

fn menu(runtime: &mut ReaderRuntime) {
    keys(runtime, "R");
    wait(
        runtime,
        |runtime| matches!(runtime.model.as_ref().unwrap().view().mode, ReaderMode::Library(mode) if matches!(mode.phase, LibraryPhase::Browsing)),
    );
}

fn choose(runtime: &mut ReaderRuntime, path: &Path) {
    let model = runtime.model.as_ref().unwrap();
    let ReaderMode::Library(mode) = model.view().mode else {
        panic!("library must be open");
    };
    let destination = mode
        .entries
        .iter()
        .position(|entry| entry.filepath == path.to_str().unwrap())
        .unwrap();
    let selected = mode.selected;
    for _ in selected..destination {
        runtime.handle_key(InputKey::Down);
    }
    for _ in destination..selected {
        runtime.handle_key(InputKey::Up);
    }
    runtime.handle_key(InputKey::Enter);
}

fn switch(runtime: &mut ReaderRuntime, path: &Path) {
    menu(runtime);
    choose(runtime, path);
    wait(runtime, |runtime| {
        runtime.model.as_ref().is_some_and(|model| {
            model.view().info.path == path.to_str().unwrap() && !model.view().loading
        })
    });
}

#[test]
fn successful_switch_durably_saves_the_departing_book() {
    let fixture = LibraryFixture::new();
    let mut runtime = fixture.runtime();
    keys(&mut runtime, "17j");
    let expected = runtime.model.as_ref().unwrap().position();
    switch(&mut runtime, &fixture.second);
    assert_eq!(fixture.position(&fixture.first), expected);
}

#[test]
fn unreadable_destination_keeps_the_current_book_usable() {
    let fixture = LibraryFixture::new();
    let mut runtime = fixture.runtime();
    keys(&mut runtime, "17j");
    let expected = runtime.model.as_ref().unwrap().position();
    std::fs::remove_file(&fixture.second).unwrap();
    menu(&mut runtime);
    choose(&mut runtime, &fixture.second);
    wait(&mut runtime, |runtime| {
        runtime
            .model
            .as_ref()
            .unwrap()
            .view()
            .status
            .contains("Cannot open ebook")
    });
    let model = runtime.model.as_ref().unwrap();
    assert_eq!(model.view().info.path, fixture.first.to_str().unwrap());
    assert_eq!(model.position(), expected);
    runtime.handle_key(InputKey::Escape);
    runtime.handle_key(InputKey::Down);
    assert_ne!(runtime.model.as_ref().unwrap().position(), expected);
}

#[test]
fn failed_departure_save_does_not_activate_the_destination() {
    let fixture = LibraryFixture::new();
    let mut runtime = fixture.runtime();
    menu(&mut runtime);
    let connection = rusqlite::Connection::open(&fixture.database).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_departure BEFORE UPDATE ON reading_states BEGIN SELECT RAISE(FAIL, 'save denied'); END;").unwrap();
    choose(&mut runtime, &fixture.second);
    wait(&mut runtime, |runtime| {
        runtime
            .model
            .as_ref()
            .unwrap()
            .view()
            .status
            .contains("save denied")
    });
    assert_eq!(
        runtime.model.as_ref().unwrap().view().info.path,
        fixture.first.to_str().unwrap()
    );
    assert_eq!(
        Store::open(&fixture.database)
            .unwrap()
            .books()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn returning_to_a_book_restores_its_own_quickmarks() {
    let fixture = LibraryFixture::new();
    let mut runtime = fixture.runtime();
    keys(&mut runtime, "5jm1");
    let first_mark = runtime.model.as_ref().unwrap().position();
    keys(&mut runtime, "10j");
    switch(&mut runtime, &fixture.second);
    keys(&mut runtime, "8jm1");
    switch(&mut runtime, &fixture.first);
    keys(&mut runtime, "`1");
    wait(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    assert_eq!(runtime.model.as_ref().unwrap().position(), first_mark);
}

#[test]
fn returning_to_a_book_retains_its_jump_history() {
    let fixture = LibraryFixture::new();
    let mut runtime = fixture.runtime();
    keys(&mut runtime, "5jm1");
    keys(&mut runtime, "20j");
    let origin = runtime.model.as_ref().unwrap().position();
    keys(&mut runtime, "`1");
    wait(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    switch(&mut runtime, &fixture.second);
    switch(&mut runtime, &fixture.first);
    runtime.handle_key(InputKey::Character('\u{0f}'));
    wait(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    assert_eq!(runtime.model.as_ref().unwrap().position(), origin);
}

#[test]
fn idle_saves_are_scoped_to_the_active_book() {
    let fixture = LibraryFixture::new();
    let mut runtime = fixture.runtime();
    runtime.autosave();
    wait(&mut runtime, |_| {
        Store::open(&fixture.database)
            .unwrap()
            .reading_state(fixture.first.to_str().unwrap())
            .unwrap()
            .is_some()
    });
    switch(&mut runtime, &fixture.second);
    runtime.autosave();
    wait(&mut runtime, |_| {
        Store::open(&fixture.database)
            .unwrap()
            .history()
            .unwrap()
            .iter()
            .any(|entry| {
                entry.filepath == fixture.second.to_str().unwrap()
                    && entry.last_read != "2000-01-01 00:00:00"
            })
    });
    assert_eq!(
        Store::open(&fixture.database).unwrap().history().unwrap()[0].filepath,
        fixture.second.to_str().unwrap()
    );
}

#[test]
fn library_removal_leaves_the_ebook_file_intact() {
    let fixture = LibraryFixture::new();
    let mut runtime = fixture.runtime();
    menu(&mut runtime);
    let model = runtime.model.as_ref().unwrap();
    let ReaderMode::Library(mode) = model.view().mode else {
        panic!("library must be open");
    };
    let destination = mode
        .entries
        .iter()
        .position(|entry| entry.filepath == fixture.second.to_str().unwrap())
        .unwrap();
    for _ in 0..destination {
        runtime.handle_key(InputKey::Down);
    }
    keys(&mut runtime, "dy");
    wait(
        &mut runtime,
        |runtime| matches!(runtime.model.as_ref().unwrap().view().mode, ReaderMode::Library(mode) if matches!(mode.phase, LibraryPhase::Browsing)),
    );
    assert!(fixture.second.is_file());
    assert!(
        Store::open(&fixture.database)
            .unwrap()
            .reading_state(fixture.second.to_str().unwrap())
            .unwrap()
            .is_none()
    );
}

#[test]
fn durable_library_saves_supersede_older_queued_idle_snapshots() {
    let fixture = LibraryFixture::new();
    let book = Book::open(fixture.first.clone()).unwrap();
    let position = ReadingState {
        content_id: book.contents()[0].clone(),
        ..ReadingState::default()
    };
    let connection = rusqlite::Connection::open(&fixture.database).unwrap();
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let jobs = ReaderJobs::start(fixture.first.clone(), fixture.database.clone(), 80);
    jobs.save(SaveRequest {
        position: position.clone(),
        progress: None,
        quitting: false,
    });
    let mut destination = position;
    destination.offset = 72;
    jobs.library(LibraryJob {
        request_id: 1,
        command: LibraryCommand::List(SaveRequest {
            position: destination.clone(),
            progress: None,
            quitting: false,
        }),
    })
    .unwrap();
    connection.execute_batch("COMMIT").unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let event = jobs
            .events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if matches!(event, WorkerEvent::Totals(_)) {
            break;
        }
        if let WorkerEvent::Failed(message) = event {
            panic!("{message}");
        }
    }
    assert_eq!(fixture.position(&fixture.first), destination);
}

#[test]
fn returning_to_a_book_preserves_its_saved_wide_layout() {
    let fixture = LibraryFixture::new();
    let mut runtime = fixture.runtime();
    keys(&mut runtime, "=");
    wait(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    let width = runtime.model.as_ref().unwrap().view().width;
    assert!(width > 80);
    switch(&mut runtime, &fixture.second);
    switch(&mut runtime, &fixture.first);
    assert_eq!(runtime.model.as_ref().unwrap().view().width, width);
}

#[test]
fn reopening_the_reader_preserves_its_saved_wide_layout() {
    let fixture = LibraryFixture::new();
    let mut runtime = fixture.runtime();
    keys(&mut runtime, "=");
    wait(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    let width = runtime.model.as_ref().unwrap().view().width;
    keys(&mut runtime, "q");
    wait(&mut runtime, ReaderRuntime::is_finished);
    drop(runtime);
    let runtime = fixture.runtime();
    assert_eq!(runtime.model.as_ref().unwrap().view().width, width);
}

#[test]
fn book_switching_retains_book_specific_display_preferences() {
    let fixture = LibraryFixture::new();
    let mut runtime = fixture.runtime();
    keys(&mut runtime, "Dcs");
    wait(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    let first = runtime.model.as_ref().unwrap();
    let expected_spread = first.view().spread;
    let expected_color = first.view().color_scheme;
    let expected_progress = first.view().show_progress;
    switch(&mut runtime, &fixture.second);
    let second = runtime.model.as_ref().unwrap();
    assert_ne!(second.view().spread, expected_spread);
    assert_ne!(second.view().color_scheme, expected_color);
    assert_ne!(second.view().show_progress, expected_progress);
    switch(&mut runtime, &fixture.first);
    let first = runtime.model.as_ref().unwrap();
    assert_eq!(first.view().spread, expected_spread);
    assert_eq!(first.view().color_scheme, expected_color);
    assert_eq!(first.view().show_progress, expected_progress);
}

#[test]
fn canceled_web_switch_cannot_activate_a_late_destination() {
    let fixture = LibraryFixture::new();
    let server = pending_web::PendingWeb::start();
    let entry = history(Path::new(&server.location));
    Store::open(&fixture.database)
        .unwrap()
        .save_session(&entry, &ReadingState::default())
        .unwrap();
    let mut runtime = fixture.runtime();
    let position = runtime.model.as_ref().unwrap().position();
    menu(&mut runtime);
    choose(&mut runtime, Path::new(&server.location));
    wait(
        &mut runtime,
        |runtime| matches!(runtime.model.as_ref().unwrap().view().mode, ReaderMode::Library(mode) if matches!(mode.phase, LibraryPhase::Opening(_))),
    );
    server
        .requests
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    runtime.resize(Viewport {
        columns: 60,
        rows: 18,
    });
    runtime.handle_key(InputKey::Escape);
    runtime.handle_key(InputKey::Escape);
    server.respond();
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        runtime.poll().unwrap();
        assert_eq!(
            runtime.model.as_ref().unwrap().view().info.path,
            fixture.first.to_str().unwrap()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let model = runtime.model.as_ref().unwrap();
    assert!(!model.view().loading);
    assert_eq!(model.view().width, 56);
    assert_eq!(model.position().offset, position.offset);
    let before_motion = model.position();
    runtime.handle_key(InputKey::Down);
    assert_ne!(runtime.model.as_ref().unwrap().position(), before_motion);
}
