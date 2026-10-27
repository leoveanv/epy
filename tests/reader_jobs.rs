#[path = "support/epub.rs"]
mod support;

use ers::app::workers::{ReaderJobs, WorkerEvent};
use ers::reading::LoadRequest;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn failed_loads_identify_the_current_request_generation() {
    let directory = tempdir().unwrap();
    let book = support::EpubBuilder::new("3.0").build(directory.path());
    let jobs = ReaderJobs::start(book, directory.path().join("states.db"), 40);
    let WorkerEvent::Opened { info, .. } =
        jobs.events.recv_timeout(Duration::from_secs(5)).unwrap()
    else {
        panic!("The fixture must open");
    };
    jobs.load(LoadRequest {
        index: info.contents.len(),
        width: 40,
        position: None,
        at_end: false,
        fragment: None,
    });
    loop {
        match jobs.events.recv_timeout(Duration::from_secs(5)).unwrap() {
            WorkerEvent::LoadFailed {
                generation,
                message,
            } => {
                assert!(jobs.is_current(generation));
                assert!(message.contains("outside this book"));
                break;
            }
            WorkerEvent::Failed(message) => {
                panic!("A load failure needs its own generation: {message}")
            }
            _ => {}
        }
    }
}
