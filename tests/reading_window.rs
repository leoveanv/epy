use std::sync::Arc;

use ers::document::RenderedDocument;
use ers::document::text::parse_document;
use ers::reading::{DocumentRow, ReadingWindow, WindowExtent, WindowMovement};

fn document(index: usize, html: &str) -> Arc<RenderedDocument> {
    Arc::new(RenderedDocument::new(
        index,
        parse_document(&format!("part{index}.xhtml"), html).unwrap(),
        20,
    ))
}

// #NOTE: The existing formatter supplies chapter boundary rows, which continuous reading must preserve without adding another separator.
struct WindowFixture {
    documents: Vec<Arc<RenderedDocument>>,
    window: ReadingWindow,
}

impl WindowFixture {
    fn new() -> Self {
        let documents = vec![
            document(0, "<p>alpha</p>"),
            document(1, "<p><b>bravé é</b></p>"),
        ];
        let window = ReadingWindow::new(
            documents.clone(),
            WindowExtent {
                starts_book: true,
                ends_book: true,
            },
        )
        .unwrap();
        Self { documents, window }
    }

    fn before_boundary(&self) -> DocumentRow {
        DocumentRow {
            document_index: 0,
            row: self.documents[0].layout.text_lines.len() - 1,
        }
    }
}

#[test]
fn visible_rows_preserve_the_existing_chapter_boundaries_without_extra_padding() {
    let fixture = WindowFixture::new();
    let expected = fixture
        .documents
        .iter()
        .flat_map(|document| document.layout.text_lines.iter().cloned())
        .collect::<Vec<_>>();
    let origin = DocumentRow {
        document_index: 0,
        row: 0,
    };
    let lines = (0..expected.len())
        .map(|offset| {
            let visible = fixture.window.row(origin, offset).unwrap();
            visible.document.layout.text_lines[visible.row].clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(lines, expected);
}

#[test]
fn crossing_a_boundary_preserves_the_remaining_line_count() {
    let fixture = WindowFixture::new();
    assert_eq!(
        fixture
            .window
            .move_rows(fixture.before_boundary(), 1, 1)
            .unwrap(),
        WindowMovement::Ready(DocumentRow {
            document_index: 1,
            row: 0
        })
    );
}

#[test]
fn moving_backward_restores_the_previous_document_row() {
    let fixture = WindowFixture::new();
    assert_eq!(
        fixture
            .window
            .move_rows(
                DocumentRow {
                    document_index: 1,
                    row: 0
                },
                -1,
                1
            )
            .unwrap(),
        WindowMovement::Ready(fixture.before_boundary())
    );
}

#[test]
fn book_end_keeps_the_last_viewport_filled_across_documents() {
    let fixture = WindowFixture::new();
    let viewport_rows = fixture.documents[1].layout.text_lines.len() + 1;
    assert_eq!(
        fixture
            .window
            .move_rows(
                DocumentRow {
                    document_index: 0,
                    row: 0
                },
                100,
                viewport_rows
            )
            .unwrap(),
        WindowMovement::Ready(fixture.before_boundary())
    );
}

#[test]
fn an_incomplete_tail_requests_more_documents_before_exposing_a_gap() {
    let fixture = WindowFixture::new();
    let window = ReadingWindow::single(
        fixture.documents[0].clone(),
        WindowExtent {
            starts_book: true,
            ends_book: false,
        },
    );
    assert_eq!(
        window.move_rows(fixture.before_boundary(), 1, 2).unwrap(),
        WindowMovement::NeedsLater
    );
}

#[test]
fn an_incomplete_head_requests_the_previous_document() {
    let window = ReadingWindow::single(
        document(1, "<p>bravo</p>"),
        WindowExtent {
            starts_book: false,
            ends_book: true,
        },
    );
    assert_eq!(
        window
            .move_rows(
                DocumentRow {
                    document_index: 1,
                    row: 0
                },
                -1,
                1
            )
            .unwrap(),
        WindowMovement::NeedsEarlier
    );
}

#[test]
fn canonical_positions_preserve_the_boundary_row_before_the_next_document() {
    let fixture = WindowFixture::new();
    let origin = fixture.before_boundary();
    let position = fixture.window.capture_position(origin).unwrap();
    assert_eq!(position.content_id, "part1.xhtml");
    assert_eq!(position.offset, 0);
    assert_eq!(position.screen_row, 1);
    assert_eq!(
        fixture.window.restore_position(&position).unwrap(),
        WindowMovement::Ready(origin)
    );
}

#[test]
fn following_rows_retain_their_document_source_metadata() {
    let fixture = WindowFixture::new();
    let visible = fixture.window.row(fixture.before_boundary(), 1).unwrap();
    assert_eq!(visible.document.index, 1);
    assert_eq!(
        visible.document.layout.cells_on_row(visible.row)[0].content_id,
        "part1.xhtml"
    );
    assert!(!visible.document.styles_on_row(visible.row).is_empty());
}

#[test]
fn empty_documents_preserve_the_existing_boundary_marker() {
    let empty = document(0, "");
    let boundary_rows = empty.layout.text_lines.clone();
    let window = ReadingWindow::new(
        vec![empty, document(1, "<p>bravo</p>")],
        WindowExtent {
            starts_book: true,
            ends_book: true,
        },
    )
    .unwrap();
    let origin = DocumentRow {
        document_index: 0,
        row: 0,
    };
    for (offset, expected) in boundary_rows.iter().enumerate() {
        let visible = window.row(origin, offset).unwrap();
        assert_eq!(&visible.document.layout.text_lines[visible.row], expected);
    }
    assert_eq!(
        window
            .row(origin, boundary_rows.len())
            .unwrap()
            .document
            .index,
        1
    );
    assert_eq!(
        window.capture_position(origin).unwrap().content_id,
        "part1.xhtml"
    );
}

#[test]
fn empty_books_keep_a_stable_canonical_position() {
    let window = ReadingWindow::single(
        document(0, ""),
        WindowExtent {
            starts_book: true,
            ends_book: true,
        },
    );
    let origin = DocumentRow {
        document_index: 0,
        row: 0,
    };
    assert_eq!(
        window.move_rows(origin, 100, 20).unwrap(),
        WindowMovement::Ready(origin)
    );
    assert_eq!(window.capture_position(origin).unwrap().offset, 0);
}

#[test]
fn invalid_window_order_is_rejected_instead_of_joining_unrelated_documents() {
    assert!(
        ReadingWindow::new(
            vec![document(0, "<p>first</p>"), document(2, "<p>third</p>")],
            WindowExtent {
                starts_book: true,
                ends_book: true
            }
        )
        .is_err()
    );
}

#[test]
fn unknown_document_rows_do_not_fall_back_to_the_first_document() {
    let fixture = WindowFixture::new();
    assert!(
        fixture
            .window
            .move_rows(
                DocumentRow {
                    document_index: 7,
                    row: 0
                },
                1,
                1
            )
            .is_err()
    );
}
