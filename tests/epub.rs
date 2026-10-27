#[path = "support/epub.rs"]
mod support;

use ers::book::epub::EpubBook;
use ers::book::{BookMetadata, ResourceLimits, TocEntry};
use support::EpubBuilder;
use tempfile::tempdir;

#[test]
fn epub_versions_preserve_spine_then_supplemental_order() {
    for version in ["1.0", "2.0", "3.0"] {
        let directory = tempdir().unwrap();
        let fixture = EpubBuilder::new(version);
        let book = EpubBook::open(&fixture.build(directory.path())).unwrap();
        assert_eq!(
            book.contents(),
            [
                "OPS/second.xhtml",
                "OPS/first chapter.xhtml",
                "OPS/notes.xhtml"
            ]
        );
    }
}

#[test]
fn epub_navigation_preserves_labels_and_fragments() {
    for version in ["2.0", "3.0"] {
        let directory = tempdir().unwrap();
        let fixture = EpubBuilder::new(version);
        let book = EpubBook::open(&fixture.build(directory.path())).unwrap();
        assert_eq!(
            book.toc(),
            [
                TocEntry {
                    label: "Second chapter".into(),
                    content_index: 0,
                    section: Some("section".into())
                },
                TocEntry {
                    label: "First".into(),
                    content_index: 1,
                    section: None
                }
            ]
        );
    }
}

#[test]
fn epub_resources_retain_original_utf8_text() {
    let directory = tempdir().unwrap();
    let fixture = EpubBuilder::new("3.0");
    let mut book = EpubBook::open(&fixture.build(directory.path())).unwrap();
    assert_eq!(
        book.raw_text("OPS/first chapter.xhtml").unwrap(),
        fixture.documents[0].body
    );
    assert_eq!(
        book.asset("OPS/picture.png").unwrap(),
        b"synthetic image bytes"
    );
}

#[test]
fn epub_metadata_decodes_xml_entities() {
    let directory = tempdir().unwrap();
    let fixture = EpubBuilder::new("3.0");
    let book = EpubBook::open(&fixture.build(directory.path())).unwrap();
    assert_eq!(
        book.metadata(),
        &BookMetadata {
            title: Some(fixture.title),
            creator: Some("Fixture author".into()),
            ..BookMetadata::default()
        }
    );
}

#[test]
fn archive_limits_reject_oversized_resources() {
    let directory = tempdir().unwrap();
    let fixture = EpubBuilder::new("3.0");
    let limits = ResourceLimits {
        resource_bytes_max: 20,
        ..ResourceLimits::default()
    };
    assert!(EpubBook::open_with_limits(&fixture.build(directory.path()), limits).is_err());
}

#[test]
fn resource_lookup_rejects_parent_traversal() {
    let directory = tempdir().unwrap();
    let fixture = EpubBuilder::new("3.0");
    let mut book = EpubBook::open(&fixture.build(directory.path())).unwrap();
    assert!(book.asset("../../secret").is_err());
}
