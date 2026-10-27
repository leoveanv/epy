#![cfg(unix)]

#[path = "support/commands.rs"]
mod command_support;
#[path = "support/epub.rs"]
mod epub_support;

use command_support::CommandFixture;
use epub_support::EpubBuilder;
use ers::app::modes::dictionary::DictionaryMode;
use ers::app::runtime::{ReaderLaunch, ReaderRuntime};
use ers::app::{ReaderMode, Viewport};
use ers::configuration::{Configuration, InputKey};
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct DictionaryFixture {
    commands: CommandFixture,
    book: PathBuf,
    database: PathBuf,
}

impl DictionaryFixture {
    fn new() -> Self {
        let commands = CommandFixture::new();
        let book = EpubBuilder::new("3.0").build(commands.root.path());
        let database = commands.root.path().join("states.db");
        Self {
            commands,
            book,
            database,
        }
    }

    fn runtime(&self) -> ReaderRuntime {
        let mut configuration = Configuration::default();
        configuration.settings.dictionary_client = "dict".into();
        let mut runtime = ReaderRuntime::start(ReaderLaunch {
            image_decoder: env!("CARGO_BIN_EXE_ers").into(),
            executables: self.commands.search.clone(),
            location: self.book.clone().into(),
            database: self.database.clone(),
            configuration,
            viewport: Viewport {
                columns: 100,
                rows: 20,
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
        assert!(
            Instant::now() < deadline,
            "The dictionary reader did not settle"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn submit(runtime: &mut ReaderRuntime, word: &str) {
    runtime.handle_key(InputKey::Character('d'));
    runtime.handle_paste(word);
    runtime.handle_key(InputKey::Enter);
}

fn viewing(runtime: &ReaderRuntime) -> bool {
    matches!(runtime.model.as_ref().unwrap().view().mode, ReaderMode::Dictionary(mode) if matches!(mode.as_ref(), DictionaryMode::Viewing(_)))
}

#[test]
fn background_definitions_reach_the_active_reader_without_moving_its_anchor() {
    let fixture = DictionaryFixture::new();
    fixture
        .commands
        .install("dict", "printf 'native naïve definition'");
    let mut runtime = fixture.runtime();
    let before = runtime.model.as_ref().unwrap().view_position();
    submit(&mut runtime, "naïve");
    wait_for(&mut runtime, viewing);
    let model = runtime.model.as_ref().unwrap();
    let ReaderMode::Dictionary(mode) = model.view().mode else {
        unreachable!()
    };
    let DictionaryMode::Viewing(view) = mode.as_ref() else {
        unreachable!()
    };
    assert_eq!(view.page.lines, ["native naïve definition"]);
    assert_eq!(model.view_position(), before);
}

#[test]
fn canceled_dictionary_processes_cannot_outlive_durable_reader_quit() {
    let fixture = DictionaryFixture::new();
    fixture.commands.install(
        "dict",
        &format!(
            "(/bin/sleep 0.3; printf orphan > '{}') & printf ready > '{}'; wait",
            fixture.commands.root.path().join("orphan").display(),
            fixture.commands.root.path().join("ready").display()
        ),
    );
    let mut runtime = fixture.runtime();
    submit(&mut runtime, "slow");
    fixture.commands.wait_file("ready");
    let started = Instant::now();
    runtime.handle_key(InputKey::Escape);
    runtime.handle_key(InputKey::Character('q'));
    wait_for(&mut runtime, ReaderRuntime::is_finished);
    assert!(started.elapsed() < Duration::from_millis(200));
    std::thread::sleep(Duration::from_millis(400));
    assert!(!fixture.commands.root.path().join("orphan").exists());
}

#[test]
fn resized_dictionary_requests_cannot_restore_an_obsolete_window() {
    let fixture = DictionaryFixture::new();
    fixture.commands.install(
        "dict",
        &format!(
            "printf ready > '{}'; /bin/sleep 0.2; printf obsolete",
            fixture.commands.root.path().join("ready").display()
        ),
    );
    let mut runtime = fixture.runtime();
    submit(&mut runtime, "slow");
    fixture.commands.wait_file("ready");
    runtime.resize(Viewport {
        columns: 80,
        rows: 12,
    });
    wait_for(&mut runtime, |runtime| {
        !runtime.model.as_ref().unwrap().view().loading
    });
    std::thread::sleep(Duration::from_millis(300));
    runtime.poll().unwrap();
    assert!(matches!(
        runtime.model.as_ref().unwrap().view().mode,
        ReaderMode::Reading
    ));
}

#[test]
fn dictionary_client_failures_reach_the_definition_viewer() {
    let fixture = DictionaryFixture::new();
    fixture
        .commands
        .install("dict", "printf 'Offline dictionary' >&2; exit 1");
    let mut runtime = fixture.runtime();
    submit(&mut runtime, "word");
    wait_for(&mut runtime, viewing);
    let ReaderMode::Dictionary(mode) = runtime.model.as_ref().unwrap().view().mode else {
        unreachable!()
    };
    let DictionaryMode::Viewing(view) = mode.as_ref() else {
        unreachable!()
    };
    assert_eq!(view.page.title, "Error: Dictionary");
    assert!(view.page.lines.join("\n").contains("Offline dictionary"));
}
