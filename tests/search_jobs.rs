#[path = "support/epub.rs"]
mod support;

use ers::app::workers::{ReaderJobs, WorkerEvent};
use ers::document::RenderedDocument;
use ers::search::{SearchDirection, SearchJob, SearchResult};
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;

fn search_job(
    request_id: u64,
    contents: Arc<[String]>,
    current: Arc<RenderedDocument>,
) -> SearchJob {
    SearchJob {
        request_id,
        pattern: "café".into(),
        contents,
        current,
        start_row: 0,
        start_offset: None,
        after_hit: None,
        direction: SearchDirection::Forward,
        count: 1,
    }
}

fn search_result(jobs: &ReaderJobs) -> SearchResult {
    loop {
        match jobs.events.recv_timeout(Duration::from_secs(5)).unwrap() {
            WorkerEvent::Search(result) => return result,
            WorkerEvent::Failed(error) => panic!("The reader worker failed: {error}"),
            _ => {}
        }
    }
}

#[test]
fn reflowed_search_reuses_previously_loaded_chapters() {
    let directory = tempdir().unwrap();
    let path = support::EpubBuilder::new("3.0").build(directory.path());
    let jobs = ReaderJobs::start(path.clone(), directory.path().join("states.db"), 40);
    let WorkerEvent::Opened { info, rendered, .. } =
        jobs.events.recv_timeout(Duration::from_secs(5)).unwrap()
    else {
        panic!("Expected a readable fixture");
    };
    jobs.search(search_job(1, info.contents.clone(), rendered.clone()));
    search_result(&jobs).result.unwrap().unwrap();

    std::fs::remove_file(path).unwrap();
    let reflowed = Arc::new(RenderedDocument::new(0, rendered.parsed.clone(), 30));
    jobs.search(search_job(2, info.contents.clone(), reflowed));
    let selection = search_result(&jobs).result.unwrap().unwrap();

    assert_eq!(selection.document.width, 30);
}

#[test]
fn search_stops_at_the_end_of_its_document_catalog() {
    let directory = tempdir().unwrap();
    let path = support::EpubBuilder::new("3.0").build(directory.path());
    let jobs = ReaderJobs::start(path, directory.path().join("states.db"), 40);
    let WorkerEvent::Opened { info, rendered, .. } =
        jobs.events.recv_timeout(Duration::from_secs(5)).unwrap()
    else {
        panic!("Expected a readable fixture");
    };
    let contents = Arc::from(info.contents[..1].to_vec());
    jobs.search(search_job(1, contents, rendered));
    assert!(search_result(&jobs).result.unwrap().is_none());
}

#[test]
fn background_search_reaches_matches_in_later_documents() {
    let directory = tempdir().unwrap();
    let path = support::EpubBuilder::new("3.0").build(directory.path());
    let jobs = ReaderJobs::start(path, directory.path().join("states.db"), 40);
    let WorkerEvent::Opened { info, rendered, .. } =
        jobs.events.recv_timeout(Duration::from_secs(5)).unwrap()
    else {
        panic!("Expected a readable fixture");
    };
    jobs.search(search_job(42, info.contents.clone(), rendered));
    loop {
        match jobs.events.recv_timeout(Duration::from_secs(5)).unwrap() {
            WorkerEvent::Search(result) => {
                assert_eq!(result.request_id, 42);
                let selection = result
                    .result
                    .unwrap()
                    .expect("The next document contains the match");
                assert_eq!(selection.document.index, 1);
                let hit = &selection.matches.hits[selection.selected];
                assert!(
                    selection
                        .document
                        .parsed
                        .source
                        .text
                        .chars()
                        .skip(hit.offset)
                        .collect::<String>()
                        .starts_with("Café")
                );
                break;
            }
            WorkerEvent::Failed(error) => {
                panic!("Search must not break the reader worker: {error}")
            }
            _ => {}
        }
    }
}
