#[path = "support/epub.rs"]
mod epub_support;
#[allow(dead_code)]
#[path = "support/kindle.rs"]
mod kindle_support;

use boko::export::Exporter;
use ers::book::Book;
use ers::document::text::parse_document;
use kindle_support::{CombinedBuilder, HuffmanBuilder, PalmDatabase};
use std::path::Path;
use tempfile::tempdir;

fn modern_database(root: &Path) -> PalmDatabase {
    let source = boko::Book::open(epub_support::EpubBuilder::new("3.0").build(root)).unwrap();
    let mut output = std::io::Cursor::new(Vec::new());
    boko::Azw3Exporter::new()
        .export(&source, &mut output)
        .unwrap();
    PalmDatabase::from_bytes(output.get_ref())
}

fn text(book: &mut Book) -> ers::Result<String> {
    book.contents()
        .to_vec()
        .iter()
        .map(|id| Ok(parse_document(id, &book.raw_text(id)?)?.source.text))
        .collect::<ers::Result<Vec<_>>>()
        .map(|parts| parts.join("\n"))
}

fn linked_modern_database(root: &Path) -> PalmDatabase {
    let mut fixture = epub_support::EpubBuilder::new("3.0");
    fixture.documents[0].body = "<!-- an unmatched ' --> <h1>First chapter</h1><p>Café 👩‍🔬 <a href='second.xhtml#section'>Target link</a></p>".into();
    fixture.documents[1].body = "<h2 id='section'>Destination</h2><p>Target passage.</p>".into();
    let source = boko::Book::open(fixture.build(root)).unwrap();
    let mut output = std::io::Cursor::new(Vec::new());
    boko::Azw3Exporter::new()
        .export(&source, &mut output)
        .unwrap();
    PalmDatabase::from_bytes(output.get_ref())
}

#[test]
fn kf8_toc_fragments_exist_in_the_reconstructed_documents() {
    let directory = tempdir().unwrap();
    for huffman in [false, true] {
        let database = linked_modern_database(directory.path());
        let database = if huffman {
            HuffmanBuilder::new().encode_modern(database)
        } else {
            database
        };
        let mut book = Book::open(database.write(directory.path(), "mobi")).unwrap();
        for entry in book.toc().to_vec() {
            if let Some(fragment) = entry.section {
                let id = book.contents()[entry.content_index].clone();
                let parsed = parse_document(&id, &book.raw_text(&id).unwrap()).unwrap();
                assert!(
                    parsed
                        .semantics
                        .anchors
                        .iter()
                        .any(|anchor| anchor.fragment == fragment),
                    "TOC fragment {fragment} must survive reconstruction"
                );
            }
        }
    }
}

#[test]
fn kf8_hyperlink_fragments_survive_combined_file_reconstruction() {
    let directory = tempdir().unwrap();
    let database = CombinedBuilder::new(linked_modern_database(directory.path())).database();
    let mut book = Book::open(database.write(directory.path(), "mobi")).unwrap();
    let mut links = 0;
    for id in book.contents().to_vec() {
        let parsed = parse_document(&id, &book.raw_text(&id).unwrap()).unwrap();
        for link in parsed.semantics.links {
            let target = book.resolve_resource(&id, &link.href).unwrap();
            let destination = parse_document(
                &target.content_id,
                &book.raw_text(&target.content_id).unwrap(),
            )
            .unwrap();
            let fragment = target
                .fragment
                .expect("The fixture links to a named section");
            assert!(
                destination
                    .semantics
                    .anchors
                    .iter()
                    .any(|anchor| anchor.fragment == fragment),
                "Link fragment {fragment} must survive reconstruction"
            );
            links += 1;
        }
    }
    assert_eq!(links, 1);
}

#[test]
fn huffman_phrase_lengths_must_fit_their_dictionary_record() {
    let directory = tempdir().unwrap();
    for recursive in [false, true] {
        for truncate_phrase in [false, true] {
            let mut fixture = HuffmanBuilder::new();
            fixture.recursive = recursive;
            fixture.truncate_phrase = truncate_phrase;
            let opened = Book::open(fixture.database().write(directory.path(), "mobi"));
            if truncate_phrase {
                assert!(
                    opened.is_err(),
                    "Truncated CDIC phrases must not silently shorten text"
                );
            } else {
                assert_eq!(
                    text(&mut opened.unwrap()).unwrap().trim_end_matches('\n'),
                    "Café 👩‍🔬 Huffman passage."
                );
            }
        }
    }
}

#[test]
fn huffman_dictionary_partition_bits_must_match_the_offset_table() {
    let directory = tempdir().unwrap();
    let mut fixture = HuffmanBuilder::new();
    fixture.dictionary_bits = u32::MAX;
    assert!(Book::open(fixture.database().write(directory.path(), "mobi")).is_err());
}

#[test]
fn combined_container_requires_its_declared_boundary_marker() {
    let directory = tempdir().unwrap();
    let mut fixture = CombinedBuilder::new(modern_database(directory.path()));
    fixture.boundary_marker = b"NOTBOUND".to_vec();
    assert!(Book::open(fixture.database().write(directory.path(), "mobi")).is_err());
}

#[test]
fn combined_boundary_cannot_refer_to_the_original_header() {
    let directory = tempdir().unwrap();
    let mut fixture = CombinedBuilder::new(modern_database(directory.path()));
    fixture.boundary_index = Some(0);
    assert!(Book::open(fixture.database().write(directory.path(), "mobi")).is_err());
}

#[test]
fn kf8_flow_table_entries_must_fit_their_record() {
    let directory = tempdir().unwrap();
    let mut database = modern_database(directory.path());
    let index = u32::from_be_bytes(database.records[0][0xc0..0xc4].try_into().unwrap()) as usize;
    database.records[index][8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(Book::open(database.write(directory.path(), "azw3")).is_err());
}

#[test]
fn kf8_flow_intervals_must_not_reverse_their_endpoints() {
    let directory = tempdir().unwrap();
    let mut database = modern_database(directory.path());
    let index = u32::from_be_bytes(database.records[0][0xc0..0xc4].try_into().unwrap()) as usize;
    database.records[index][12..16].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(Book::open(database.write(directory.path(), "azw3")).is_err());
}

#[test]
fn extended_metadata_records_must_fit_their_declared_region() {
    let directory = tempdir().unwrap();
    let mut fixture = CombinedBuilder::new(modern_database(directory.path())).database();
    fixture.records[0][252..256].copy_from_slice(&12u32.to_be_bytes());
    assert!(Book::open(fixture.write(directory.path(), "mobi")).is_err());
}

#[test]
fn combined_metadata_does_not_inherit_the_original_rendition_language() {
    let directory = tempdir().unwrap();
    let mut fixture = CombinedBuilder::new(modern_database(directory.path()));
    fixture.original_language = Some("fr".into());
    let book = Book::open(fixture.database().write(directory.path(), "mobi")).unwrap();
    assert_eq!(book.metadata().language, None);
}

#[test]
fn combined_resource_indices_preserve_shared_image_bytes() {
    use base64::Engine;
    let directory = tempdir().unwrap();
    let image = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZ0AAAAASUVORK5CYII=").unwrap();
    let mut fixture = CombinedBuilder::new(modern_database(directory.path()));
    fixture.shared_assets.push(image.clone());
    let mut book = Book::open(fixture.database().write(directory.path(), "mobi")).unwrap();
    assert_eq!(book.asset("images/image_0000.png").unwrap().bytes, image);
}

#[test]
fn oversized_optional_assets_do_not_prevent_kf8_reading() {
    let directory = tempdir().unwrap();
    let mut fixture = epub_support::EpubBuilder::new("3.0");
    let mut image = vec![0; 32 * 1024];
    image[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    fixture.assets.push(epub_support::AssetInput::new(
        "large.png",
        "image/png",
        image,
    ));
    let source = boko::Book::open(fixture.build(directory.path())).unwrap();
    let mut output = std::io::Cursor::new(Vec::new());
    boko::Azw3Exporter::new()
        .export(&source, &mut output)
        .unwrap();
    let mut options = ers::book::BookOpenOptions::default();
    options.limits.resource_bytes_max = 16 * 1024;
    let path = PalmDatabase::from_bytes(output.get_ref()).write(directory.path(), "azw3");
    let mut book = Book::open_with_options(ers::book::BookLocation::local(path), options)
        .expect("An optional oversized image must not prevent reading the text");
    assert!(text(&mut book).unwrap().contains("Café 👩‍🔬"));
}

#[test]
fn kf8_stylesheet_flows_remain_readable_as_resources() {
    let directory = tempdir().unwrap();
    let mut fixture = epub_support::EpubBuilder::new("3.0");
    fixture.assets.push(epub_support::AssetInput::new(
        "style.css",
        "text/css",
        b"p { color: #123456; }".to_vec(),
    ));
    fixture.documents[0].body = "<link rel='stylesheet' href='style.css'/><p>Café 👩‍🔬</p>".into();
    let source = boko::Book::open(fixture.build(directory.path())).unwrap();
    let mut output = std::io::Cursor::new(Vec::new());
    boko::Azw3Exporter::new()
        .export(&source, &mut output)
        .unwrap();
    for huffman in [false, true] {
        let database = PalmDatabase::from_bytes(output.get_ref());
        let database = if huffman {
            HuffmanBuilder::new().encode_modern(database)
        } else {
            database
        };
        let mut book = Book::open(database.write(directory.path(), "azw3")).unwrap();
        let asset = book.asset("styles/style0000.css").unwrap();
        assert!(String::from_utf8(asset.bytes).unwrap().contains("#123456"));
    }
}
