use ers::book::{Book, BookLocation};
use ers::document::text::parse_document;
use tempfile::tempdir;

#[path = "support/fictionbook.rs"]
mod fictionbook_support;
use fictionbook_support::FictionBookBuilder;

#[test]
fn fictionbook_preserves_body_and_section_reading_order() {
    let directory = tempdir().unwrap();
    let mut book = Book::open(BookLocation::local(
        FictionBookBuilder::new().build(directory.path()),
    ))
    .unwrap();
    let contents = book.contents().to_vec();
    assert_eq!(contents.len(), 3);
    let text = contents
        .iter()
        .map(|content| {
            parse_document(content, &book.raw_text(content).unwrap())
                .unwrap()
                .source
                .text
        })
        .collect::<Vec<_>>();
    assert!(text[0].contains("Opening passage"));
    assert!(text[0].contains("Nested passage."));
    assert!(text[1].contains("Last passage."));
    assert!(text[2].contains("Supplemental note."));
}

#[test]
fn fictionbook_metadata_uses_complete_author_names() {
    let directory = tempdir().unwrap();
    let book = Book::open(BookLocation::local(
        FictionBookBuilder::new().build(directory.path()),
    ))
    .unwrap();
    assert_eq!(book.metadata().title.as_deref(), Some("A café story"));
    assert_eq!(
        book.metadata().creator.as_deref(),
        Some("Fixture Q. Author")
    );
    assert_eq!(
        book.metadata().identifier.as_deref(),
        Some("synthetic-fb2-id")
    );
}

#[test]
fn fictionbook_note_links_resolve_to_the_notes_document() {
    let directory = tempdir().unwrap();
    let mut book = Book::open(BookLocation::local(
        FictionBookBuilder::new().build(directory.path()),
    ))
    .unwrap();
    let first = book.contents()[0].clone();
    let parsed = parse_document(&first, &book.raw_text(&first).unwrap()).unwrap();
    let link = &parsed.semantics.links[0];
    let destination = book.resolve_resource(&first, &link.href).unwrap();
    assert_eq!(destination.content_id, book.contents()[2]);
    assert_eq!(destination.fragment.as_deref(), Some("note-one"));
}

#[test]
fn fictionbook_encoded_note_ids_keep_cross_body_destinations() {
    let directory = tempdir().unwrap();
    let mut fixture = FictionBookBuilder::new();
    fixture.first_body = fixture.first_body.replace("#note-one", "#note%2520one");
    fixture.notes_body = fixture
        .notes_body
        .replace("id='note-one'", "id='note%20one'");
    let mut book = Book::open(BookLocation::local(fixture.build(directory.path()))).unwrap();
    let first = book.contents()[0].clone();
    let parsed = parse_document(&first, &book.raw_text(&first).unwrap()).unwrap();
    let destination = book
        .resolve_resource(&first, &parsed.semantics.links[0].href)
        .unwrap();
    assert_eq!(destination.content_id, book.contents()[2]);
    assert_eq!(destination.fragment.as_deref(), Some("note%20one"));
}

#[test]
fn fictionbook_embedded_images_retain_original_bytes() {
    let directory = tempdir().unwrap();
    let mut book = Book::open(BookLocation::local(
        FictionBookBuilder::new().build(directory.path()),
    ))
    .unwrap();
    let first = book.contents()[0].clone();
    let parsed = parse_document(&first, &book.raw_text(&first).unwrap()).unwrap();
    let image = parsed
        .paragraphs
        .iter()
        .find_map(|paragraph| paragraph.image_path.as_ref())
        .unwrap();
    let resource = book.resolve_resource(&first, image).unwrap();
    assert_eq!(book.asset(&resource.content_id).unwrap().bytes, b"hello");
}

#[test]
fn fictionbook_nested_sections_have_navigable_toc_targets() {
    let directory = tempdir().unwrap();
    let book = Book::open(BookLocation::local(
        FictionBookBuilder::new().build(directory.path()),
    ))
    .unwrap();
    assert!(
        book.toc()
            .iter()
            .any(|entry| entry.label == "A nested scene"
                && entry.content_index == 0
                && entry.section.as_deref() == Some("nested"))
    );
}
