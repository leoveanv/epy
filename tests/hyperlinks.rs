use std::sync::Arc;

use ers::app::{ReaderEffect, ReaderMode, ReaderModel, ReaderSession};
use ers::configuration::InputKey;
use ers::document::RenderedDocument;
use ers::document::text::parse_document;
use ers::reading::{DocumentRow, ReadingWindow, WindowExtent};
use ers::reading::{external_url, link_hints, place_hints};

#[path = "support/reader.rs"]
mod reader_support;

fn document(index: usize, body: &str) -> Arc<RenderedDocument> {
    Arc::new(RenderedDocument::new(
        index,
        parse_document(&format!("OPS/chapter-{index}.xhtml"), body).unwrap(),
        40,
    ))
}

fn window() -> ReadingWindow {
    ReadingWindow::new(
        vec![
            document(
                0,
                "<p><a href='#one'>first</a> <a href='#two'>second</a></p>",
            ),
            document(1, "<p><a href='#three'>third</a></p>"),
        ],
        WindowExtent {
            starts_book: true,
            ends_book: true,
        },
    )
    .unwrap()
}

fn reader() -> ReaderModel {
    let model = reader_support::reader();
    let mut position = model.position();
    let rendered = Arc::new(RenderedDocument::new(
        0,
        parse_document(
            &position.content_id,
            "<p><a href='#first'>First link</a> <a href='#second'>Second link</a></p>",
        )
        .unwrap(),
        40,
    ));
    position.offset = 0;
    ReaderModel::new(
        ReaderSession {
            info: model.view().info.clone(),
            rendered,
            position,
            configuration: model.view().configuration.clone(),
        },
        model.view().viewport,
    )
}

#[test]
fn visible_hints_include_the_following_document() {
    let window = window();
    let hints = link_hints(
        &window,
        DocumentRow {
            document_index: 0,
            row: 0,
        },
        6,
    );
    assert_eq!(
        hints
            .iter()
            .map(|hint| hint.link.href.as_str())
            .collect::<Vec<_>>(),
        ["#one", "#two", "#three"]
    );
    assert_eq!(
        hints.iter().map(|hint| hint.number).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(hints[2].row, 4);
}

#[test]
fn offscreen_links_do_not_receive_hints() {
    let hints = link_hints(
        &window(),
        DocumentRow {
            document_index: 0,
            row: 1,
        },
        3,
    );
    assert!(hints.is_empty());
}

#[test]
fn wrapped_links_receive_only_one_visible_hint() {
    let window = ReadingWindow::single(
        document(
            0,
            &format!("<p><a href='#long'>{}</a></p>", "long label ".repeat(20)),
        ),
        WindowExtent {
            starts_book: true,
            ends_book: true,
        },
    );
    let hints = link_hints(
        &window,
        DocumentRow {
            document_index: 0,
            row: 1,
        },
        4,
    );
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].row, 0);
}

#[test]
fn hint_placements_do_not_overlap() {
    let hints = link_hints(
        &window(),
        DocumentRow {
            document_index: 0,
            row: 0,
        },
        1,
    );
    let placements = place_hints(&hints, 7).unwrap();
    assert!(
        placements[0].column + 3 <= placements[1].column
            || placements[1].column + 3 <= placements[0].column
    );
}

#[test]
fn crowded_hint_placements_request_the_list_fallback() {
    let hints = link_hints(
        &window(),
        DocumentRow {
            document_index: 0,
            row: 0,
        },
        1,
    );
    assert!(place_hints(&hints, 4).is_none());
}

#[test]
fn browser_targets_resolve_remote_relative_links() {
    assert_eq!(
        external_url("https://example.org/books/chapter", "../other#part")
            .unwrap()
            .unwrap()
            .as_str(),
        "https://example.org/other#part"
    );
}

#[test]
fn local_fragment_links_do_not_launch_a_browser() {
    for from in ["OPS/chapter.xhtml", "https://example.org/chapter"] {
        assert!(external_url(from, "#part").unwrap().is_none());
    }
}

#[test]
fn network_path_links_use_https() {
    assert_eq!(
        external_url("OPS/chapter.xhtml", "//example.org/path")
            .unwrap()
            .unwrap()
            .as_str(),
        "https://example.org/path"
    );
}

#[test]
fn unsafe_browser_targets_are_rejected() {
    for href in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "mailto:a@example.org",
        "https://example.org/\ncommand",
        "https://",
    ] {
        assert!(external_url("OPS/chapter.xhtml", href).is_err(), "{href}");
    }
}

#[test]
fn following_a_link_requires_a_typed_number_and_enter() {
    let mut model = reader();
    assert!(model.handle_key(InputKey::Character('f')).is_empty());
    assert!(model.handle_key(InputKey::Character('2')).is_empty());
    let effects = model.handle_key(InputKey::Enter);
    let request = effects
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Hyperlink(request) => Some(request),
            _ => None,
        })
        .expect("selected hyperlink");
    assert_eq!(request.link.href, "#second");
    assert!(model.view().loading);
}

#[test]
fn pasted_hint_numbers_do_not_follow_links() {
    let mut model = reader();
    model.handle_key(InputKey::Character('f'));
    model.handle_paste("1\n");
    assert!(model.handle_key(InputKey::Enter).is_empty());
    assert!(!model.view().loading);
    assert!(!matches!(model.view().mode, ReaderMode::Reading));
}

#[test]
fn canceling_link_hints_preserves_the_viewport() {
    let mut model = reader();
    let position = model.position();
    model.handle_key(InputKey::Character('f'));
    assert!(model.handle_key(InputKey::Escape).is_empty());
    assert!(matches!(model.view().mode, ReaderMode::Reading));
    assert_eq!(model.position(), position);
}

#[test]
fn resizing_dismisses_stale_link_hints() {
    let mut model = reader();
    model.handle_key(InputKey::Character('f'));
    let viewport = model.view().viewport;
    model.resize(viewport);
    assert!(matches!(model.view().mode, ReaderMode::Reading));
}
