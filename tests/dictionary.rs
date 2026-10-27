#![cfg(unix)]

#[path = "support/commands.rs"]
mod command_support;

use command_support::CommandFixture;
use ers::integrations::{
    DictionaryCommand, DictionaryDefinition, DictionaryJob, DictionaryOptions, DictionaryResult,
    DictionaryWorker,
};
use std::time::{Duration, Instant};

fn worker(fixture: &CommandFixture, client: &str) -> DictionaryWorker {
    DictionaryWorker::start(DictionaryOptions::new(client, fixture.search.clone())).unwrap()
}

fn result(worker: &DictionaryWorker) -> DictionaryResult {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(result) = worker.take_result() {
            return result;
        }
        assert!(Instant::now() < deadline, "The dictionary did not complete");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn dictionary_auto_detection_preserves_the_reference_priority() {
    let fixture = CommandFixture::new();
    let dict = fixture.install("dict", "exit 0");
    let sdcv = fixture.install("sdcv", "exit 0");
    let wkdict = fixture.install("wkdict", "exit 0");
    for (program, arguments) in [(wkdict, vec![]), (sdcv, vec!["-n".into()]), (dict, vec![])] {
        assert_eq!(
            DictionaryCommand::resolve("auto", &fixture.search).unwrap(),
            DictionaryCommand {
                program: program.clone(),
                arguments
            }
        );
        std::fs::remove_file(program).unwrap();
    }
}

#[test]
fn configured_dictionary_arguments_preserve_quoted_words() {
    let fixture = CommandFixture::new();
    let program = fixture.install("dict", "exit 0");
    fixture.install("wkdict", "exit 0");
    let command =
        DictionaryCommand::resolve("dict --profile 'reading profile'", &fixture.search).unwrap();
    assert_eq!(
        command.for_word("naïve reader; $(touch never)"),
        DictionaryCommand {
            program,
            arguments: vec![
                "--profile".into(),
                "reading profile".into(),
                "naïve reader; $(touch never)".into()
            ]
        }
    );
}

#[test]
fn missing_preferred_dictionaries_fall_back_to_available_clients() {
    let fixture = CommandFixture::new();
    let program = fixture.install("sdcv", "exit 0");
    assert_eq!(
        DictionaryCommand::resolve("missing --flag", &fixture.search).unwrap(),
        DictionaryCommand {
            program,
            arguments: vec!["-n".into()]
        }
    );
}

#[test]
fn dictionary_queries_never_interpret_shell_metacharacters() {
    let fixture = CommandFixture::new();
    fixture.install("dict", "printf '<%s>' \"$@\"");
    let worker = worker(&fixture, "dict --profile 'reading profile'");
    let word = format!(
        "naïve; /usr/bin/touch {}",
        fixture.root.path().join("injected").display()
    );
    worker.request(
        DictionaryJob {
            request_id: 7,
            word: word.clone(),
        },
        80,
    );
    let reply = result(&worker);
    assert_eq!(reply.request_id, 7);
    assert_eq!(
        reply.result.unwrap(),
        DictionaryDefinition {
            word: word.clone(),
            text: format!("<--profile><reading profile><{word}>")
        }
    );
    assert!(!fixture.root.path().join("injected").exists());
}

#[test]
fn dictionary_output_removes_terminal_commands_without_losing_unicode() {
    let fixture = CommandFixture::new();
    fixture.install(
        "dict",
        "printf '\\033]52;c;clipboard\\007\\033[31mnaïve\\033[0m\\nline two\\tvalue\\001'",
    );
    let worker = worker(&fixture, "dict");
    worker.request(
        DictionaryJob {
            request_id: 1,
            word: "naïve".into(),
        },
        80,
    );
    assert_eq!(
        result(&worker).result.unwrap().text,
        "naïve\nline two\tvalue"
    );
}

#[test]
fn definition_pages_wrap_to_the_requested_viewport_width() {
    let fixture = CommandFixture::new();
    fixture.install("dict", "printf abcdefghijklmnopqr");
    let worker = worker(&fixture, "dict");
    worker.request(
        DictionaryJob {
            request_id: 1,
            word: "example".into(),
        },
        6,
    );
    let page = result(&worker).page;
    assert_eq!(page.title, "Definition: EXAMPLE");
    assert_eq!(page.lines, ["abcdef", "ghijkl", "mnopqr"]);
}

#[test]
fn dictionary_stderr_remains_a_visible_error() {
    let fixture = CommandFixture::new();
    fixture.install("dict", "printf 'Network unavailable' >&2");
    let worker = worker(&fixture, "dict");
    worker.request(
        DictionaryJob {
            request_id: 1,
            word: "test".into(),
        },
        80,
    );
    assert!(
        result(&worker)
            .result
            .unwrap_err()
            .contains("Network unavailable")
    );
}

#[test]
fn unavailable_dictionary_clients_report_an_actionable_error() {
    let fixture = CommandFixture::new();
    let worker = worker(&fixture, "auto");
    worker.request(
        DictionaryJob {
            request_id: 1,
            word: "test".into(),
        },
        80,
    );
    assert!(
        result(&worker)
            .result
            .unwrap_err()
            .contains("dictionary_client")
    );
}

#[test]
fn oversized_dictionary_output_is_rejected_on_either_stream() {
    for redirection in ["", " >&2"] {
        let fixture = CommandFixture::new();
        fixture.install(
            "dict",
            &format!("exec /usr/bin/yes definition{redirection}"),
        );
        let mut options = DictionaryOptions::new("dict", fixture.search.clone());
        options.limits.output_bytes_max = 64;
        let worker = DictionaryWorker::start(options).unwrap();
        worker.request(
            DictionaryJob {
                request_id: 1,
                word: "test".into(),
            },
            80,
        );
        assert!(result(&worker).result.unwrap_err().contains("output limit"));
    }
}

#[test]
fn stalled_dictionary_clients_obey_the_deadline() {
    let fixture = CommandFixture::new();
    fixture.install("dict", "exec /bin/sleep 30");
    let mut options = DictionaryOptions::new("dict", fixture.search.clone());
    options.limits.timeout = Duration::from_millis(50);
    let worker = DictionaryWorker::start(options).unwrap();
    let started = Instant::now();
    worker.request(
        DictionaryJob {
            request_id: 1,
            word: "test".into(),
        },
        80,
    );
    assert!(result(&worker).result.unwrap_err().contains("deadline"));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn superseding_dictionary_queries_retire_the_previous_process_tree() {
    let fixture = CommandFixture::new();
    fixture.install("dict", &format!("case \"$1\" in slow) (/bin/sleep 0.3; printf orphan > '{}') & printf ready > '{}'; wait;; *) printf newest;; esac", fixture.root.path().join("orphan").display(), fixture.root.path().join("ready").display()));
    let worker = worker(&fixture, "dict");
    worker.request(
        DictionaryJob {
            request_id: 1,
            word: "slow".into(),
        },
        80,
    );
    fixture.wait_file("ready");
    worker.request(
        DictionaryJob {
            request_id: 2,
            word: "fast".into(),
        },
        80,
    );
    let reply = result(&worker);
    assert_eq!(reply.request_id, 2);
    assert_eq!(reply.result.unwrap().text, "newest");
    std::thread::sleep(Duration::from_millis(400));
    assert!(!fixture.root.path().join("orphan").exists());
    assert!(worker.take_result().is_none());
}

#[test]
fn dropping_dictionary_workers_cleans_their_process_tree_without_waiting_for_the_client() {
    let fixture = CommandFixture::new();
    fixture.install(
        "dict",
        &format!(
            "(/bin/sleep 0.3; printf orphan > '{}') & printf ready > '{}'; wait",
            fixture.root.path().join("orphan").display(),
            fixture.root.path().join("ready").display()
        ),
    );
    let worker = worker(&fixture, "dict");
    worker.request(
        DictionaryJob {
            request_id: 1,
            word: "slow".into(),
        },
        80,
    );
    fixture.wait_file("ready");
    let started = Instant::now();
    drop(worker);
    assert!(started.elapsed() < Duration::from_millis(200));
    std::thread::sleep(Duration::from_millis(400));
    assert!(!fixture.root.path().join("orphan").exists());
}
