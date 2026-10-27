use ers::book::{Book, BookLocation};
use ers::document::text::parse_document;
#[path = "support/epub.rs"]
mod epub_support;

#[allow(dead_code)]
#[path = "support/kindle.rs"]
mod kindle_support;
use kindle_support::MobiBuilder;
use tempfile::tempdir;

#[test]
fn native_mobi_decoding_preserves_unicode_in_every_chapter() {
    let directory = tempdir().unwrap();
    for compression in [1, 2] {
        let mut fixture = MobiBuilder::new();
        fixture.compression = compression;
        let mut book =
            Book::open(BookLocation::local(fixture.build(directory.path(), "mobi"))).unwrap();
        let text = book
            .contents()
            .to_vec()
            .iter()
            .map(|content| {
                parse_document(content, &book.raw_text(content).unwrap())
                    .unwrap()
                    .source
                    .text
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Café 👩‍🔬 first passage."));
        assert!(text.contains("Last passage."));
        assert_eq!(
            book.metadata().title.as_deref(),
            Some(fixture.title.as_str())
        );
    }
}

#[test]
fn cp1252_kindle_metadata_does_not_replace_unicode_punctuation() {
    let directory = tempdir().unwrap();
    let mut fixture = MobiBuilder::new();
    fixture.encoding = 1252;
    fixture.title = "Curly “title” café".into();
    fixture.html = encoding_rs::WINDOWS_1252
        .encode("<p>Curly “quoted” café text.</p>")
        .0
        .into_owned();
    let book = Book::open(BookLocation::local(fixture.build(directory.path(), "mobi"))).unwrap();
    assert_eq!(
        book.metadata().title.as_deref(),
        Some(fixture.title.as_str())
    );
    assert_eq!(book.toc()[0].label, fixture.title);
}

#[test]
fn kf8_reconstruction_uses_content_instead_of_the_filename_extension() {
    use boko::export::Exporter;
    let directory = tempdir().unwrap();
    let epub = epub_support::EpubBuilder::new("3.0").build(directory.path());
    let source = boko::Book::open(&epub).unwrap();
    let mut generated = std::io::Cursor::new(Vec::new());
    boko::Azw3Exporter::new()
        .export(&source, &mut generated)
        .unwrap();
    for extension in ["azw3", "mobi", "azw"] {
        let path = directory.path().join(format!("native-kf8.{extension}"));
        std::fs::write(&path, generated.get_ref()).unwrap();
        let mut book = Book::open(BookLocation::local(path)).unwrap();
        let text = book
            .contents()
            .to_vec()
            .iter()
            .map(|content| {
                parse_document(content, &book.raw_text(content).unwrap())
                    .unwrap()
                    .source
                    .text
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Café 👩‍🔬"));
        assert!(text.contains("Second"));
        assert_eq!(book.contents().len(), source.spine().len());
    }
}

#[test]
fn azw_extension_uses_the_native_mobi_decoder() {
    let directory = tempdir().unwrap();
    let book = Book::open(BookLocation::local(
        MobiBuilder::new().build(directory.path(), "azw"),
    ))
    .unwrap();
    assert!(!book.contents().is_empty());
}

#[test]
fn kindle_header_lengths_must_fit_their_record() {
    let directory = tempdir().unwrap();
    let mut fixture = MobiBuilder::new();
    fixture.header_length = u32::MAX;
    assert!(Book::open(BookLocation::local(fixture.build(directory.path(), "mobi"))).is_err());
}

#[test]
fn encrypted_kindle_files_report_an_error_without_external_tools() {
    let directory = tempdir().unwrap();
    let mut fixture = MobiBuilder::new();
    fixture.encrypted = true;
    assert!(Book::open(BookLocation::local(fixture.build(directory.path(), "mobi"))).is_err());
}

#[test]
fn unsupported_kindle_compression_is_not_treated_as_plain_text() {
    let directory = tempdir().unwrap();
    let mut fixture = MobiBuilder::new();
    fixture.compression = 99;
    assert!(Book::open(BookLocation::local(fixture.build(directory.path(), "mobi"))).is_err());
}
