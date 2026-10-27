#[path = "support/epub.rs"]
mod support;

use ers::annotations::{Annotation, AnnotationCommand, AnnotationDraft, AnnotationJob, TextAnchor};
use ers::app::workers::{ReaderJobs, WorkerEvent};
use ers::book::epub::EpubBook;
use ers::document::text::parse_document;
use std::time::{Duration, Instant};
use support::EpubBuilder;
use tempfile::tempdir;

fn navigation_fixture() -> EpubBuilder {
    let mut fixture = EpubBuilder::new("3.0");
    fixture.navigation_path = "Nav/nav.xhtml".into();
    fixture.navigation = Some("<html xmlns='http://www.w3.org/1999/xhtml' xmlns:epub='http://www.idpf.org/2007/ops'><nav epub:type='toc'><ol><li><span>COLLECTION</span><ol><li><a href='../second.xhtml#story'>STORY ALPHA</a></li></ol></li></ol></nav><nav epub:type='page-list'><a href='../second.xhtml#p12'>12</a></nav></html>".into());
    fixture.documents[1].body =
        "<h1 id='story'>STORY ALPHA</h1><span id='p12'/><p>selected passage</p>".into();
    fixture.documents[0].body = "<p>continued passage</p>".into();
    fixture
}

fn create(fixture: &EpubBuilder, content: &str, quote: &str) -> Annotation {
    let directory = tempdir().unwrap();
    let path = fixture.build(directory.path());
    let mut book = EpubBook::open(&path).unwrap();
    let document = parse_document(content, &book.raw_text(content).unwrap()).unwrap();
    let byte = document.source.text.find(quote).unwrap();
    let anchor =
        TextAnchor::new(content, document.source.text[..byte].chars().count(), quote).unwrap();
    let jobs = ReaderJobs::start(path, directory.path().join("states.db"), 40);
    jobs.annotate(AnnotationJob {
        request_id: 1,
        command: AnnotationCommand::Create(AnnotationDraft {
            anchor,
            note: "A reference note".into(),
        }),
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let WorkerEvent::Annotation(result) = jobs
            .events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            assert!(result.error.is_none(), "{:?}", result.error);
            return result.annotations.into_iter().next().unwrap();
        }
    }
}

#[test]
fn saved_annotations_include_nested_navigation_headings() {
    let annotation = create(
        &navigation_fixture(),
        "OPS/second.xhtml",
        "selected passage",
    );
    assert_eq!(
        annotation.reference.unwrap().headings,
        ["COLLECTION", "STORY ALPHA"]
    );
}

#[test]
fn saved_annotations_use_epub_page_list_labels() {
    let annotation = create(
        &navigation_fixture(),
        "OPS/second.xhtml",
        "selected passage",
    );
    assert_eq!(
        annotation.reference.unwrap().page_start.as_deref(),
        Some("12")
    );
}

#[test]
fn saved_annotations_keep_story_context_in_continued_documents() {
    let annotation = create(
        &navigation_fixture(),
        "OPS/first chapter.xhtml",
        "continued passage",
    );
    assert_eq!(
        annotation.reference.unwrap().headings,
        ["COLLECTION", "STORY ALPHA"]
    );
}

#[test]
fn epub_navigation_resolves_links_from_its_own_directory() {
    let directory = tempdir().unwrap();
    let book = EpubBook::open(&navigation_fixture().build(directory.path())).unwrap();
    assert_eq!(book.toc()[0].content_index, 0);
    assert_eq!(book.toc()[0].section.as_deref(), Some("story"));
}

#[test]
fn epub3_accepts_navigation_with_multiple_property_tokens() {
    let directory = tempdir().unwrap();
    let mut fixture = navigation_fixture();
    fixture.navigation_properties = "scripted nav".into();
    assert!(EpubBook::open(&fixture.build(directory.path())).is_ok());
}

#[test]
fn epub3_uses_ncx_when_html_navigation_is_absent() {
    let directory = tempdir().unwrap();
    let mut fixture = EpubBuilder::new("3.0");
    fixture.use_ncx = true;
    let book = EpubBook::open(&fixture.build(directory.path())).unwrap();
    assert_eq!(book.toc()[0].label, "Second chapter");
}
