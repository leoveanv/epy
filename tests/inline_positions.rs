use std::sync::Arc;

use ers::document::RenderedDocument;
use ers::document::layout::LayoutOptions;
use ers::document::text::parse_document;
use ers::reading::{DocumentRow, ReadingWindow, WindowExtent, WindowMovement};

struct InlineWindowBuilder {
    width: usize,
    image_rows: usize,
    first: String,
}

struct InlineWindowFixture {
    documents: Vec<Arc<RenderedDocument>>,
    window: ReadingWindow,
}

impl InlineWindowBuilder {
    fn new() -> Self {
        Self {
            width: 40,
            image_rows: 5,
            first: "<p>Before.</p><img src='same.png'/><p>Between.</p><img src='same.png'/><p>After.</p>".into(),
        }
    }

    fn build(&self) -> InlineWindowFixture {
        let documents = [&self.first, "<p>Following chapter.</p>"]
            .into_iter()
            .enumerate()
            .map(|(index, html)| {
                Arc::new(RenderedDocument::with_layout(
                    index,
                    parse_document(&format!("part{index}.xhtml"), html).unwrap(),
                    LayoutOptions {
                        width: self.width,
                        starting_row: 0,
                        image_rows: self.image_rows,
                    },
                ))
            })
            .collect::<Vec<_>>();
        let window = ReadingWindow::new(
            documents.clone(),
            WindowExtent {
                starts_book: true,
                ends_book: true,
            },
        )
        .unwrap();
        InlineWindowFixture { documents, window }
    }
}

#[test]
fn partially_visible_inline_images_keep_their_negative_origin() {
    let fixture = InlineWindowBuilder::new().build();
    let row = *fixture.documents[0]
        .layout
        .image_maps
        .keys()
        .next()
        .unwrap();
    let targets = ers::images::visible_images(
        &fixture.window,
        DocumentRow {
            document_index: 0,
            row: row + 2,
        },
        2,
    );
    assert_eq!(
        targets,
        vec![ers::images::ImageTarget {
            content_id: fixture.documents[0].parsed.source.content_id.clone(),
            path: "same.png".into(),
            row_offset: -2,
        }]
    );
}

#[test]
fn inline_slots_do_not_change_saved_text_positions() {
    let mut builder = InlineWindowBuilder::new();
    let expanded = builder.build();
    builder.image_rows = 1;
    let original = builder.build();
    let mapping = expanded.documents[0].layout.text_lines.image_layout();
    for row in 0..original.documents[0].layout.text_lines.len() {
        assert_eq!(
            expanded
                .window
                .capture_position(DocumentRow {
                    document_index: 0,
                    row: mapping.display_row(row)
                })
                .unwrap(),
            original
                .window
                .capture_position(DocumentRow {
                    document_index: 0,
                    row
                })
                .unwrap(),
        );
    }
}

#[test]
fn saved_text_positions_restore_to_expanded_image_markers() {
    let mut builder = InlineWindowBuilder::new();
    let expanded = builder.build();
    builder.image_rows = 1;
    let original = builder.build();
    let row = *original.documents[0]
        .layout
        .image_maps
        .keys()
        .next()
        .unwrap();
    let saved = original
        .window
        .capture_position(DocumentRow {
            document_index: 0,
            row,
        })
        .unwrap();
    assert_eq!(
        expanded.window.restore_position(&saved).unwrap(),
        WindowMovement::Ready(DocumentRow {
            document_index: 0,
            row: *expanded.documents[0]
                .layout
                .image_maps
                .keys()
                .next()
                .unwrap(),
        })
    );
}

#[test]
fn partial_inline_positions_round_trip_repeated_image_occurrences() {
    let fixture = InlineWindowBuilder::new().build();
    for row in fixture.documents[0].layout.image_maps.keys() {
        for offset in 0..5 {
            let origin = DocumentRow {
                document_index: 0,
                row: row + offset,
            };
            let position = fixture.window.capture_view(origin).unwrap();
            assert_eq!(
                fixture.window.restore_view(&position).unwrap(),
                WindowMovement::Ready(origin)
            );
        }
    }
}

#[test]
fn partial_inline_positions_clamp_after_reflow() {
    let mut builder = InlineWindowBuilder::new();
    builder.image_rows = 12;
    let wide = builder.build();
    let row = *wide.documents[0].layout.image_maps.keys().nth(1).unwrap();
    let saved = wide
        .window
        .capture_view(DocumentRow {
            document_index: 0,
            row: row + 11,
        })
        .unwrap();
    builder.image_rows = 3;
    builder.width = 16;
    let narrow = builder.build();
    let expected = *narrow.documents[0].layout.image_maps.keys().nth(1).unwrap() + 2;
    assert_eq!(
        narrow.window.restore_view(&saved).unwrap(),
        WindowMovement::Ready(DocumentRow {
            document_index: 0,
            row: expected
        })
    );
}

#[test]
fn partial_inline_positions_survive_image_only_documents() {
    let mut builder = InlineWindowBuilder::new();
    builder.first = "<img src='only.png'/>".into();
    let fixture = builder.build();
    let row = *fixture.documents[0]
        .layout
        .image_maps
        .keys()
        .next()
        .unwrap()
        + 3;
    let origin = DocumentRow {
        document_index: 0,
        row,
    };
    let position = fixture.window.capture_view(origin).unwrap();
    assert_eq!(
        fixture.window.restore_view(&position).unwrap(),
        WindowMovement::Ready(origin)
    );
}

#[test]
fn cross_document_text_positions_ignore_inline_expansion() {
    let mut builder = InlineWindowBuilder::new();
    builder.first = "<p>Before.</p><img src='last.png'/>".into();
    let expanded = builder.build();
    builder.image_rows = 1;
    let original = builder.build();
    let original_row = *original.documents[0]
        .layout
        .image_maps
        .keys()
        .next()
        .unwrap();
    let expanded_row = *expanded.documents[0]
        .layout
        .image_maps
        .keys()
        .next()
        .unwrap();
    assert_eq!(
        expanded
            .window
            .capture_position(DocumentRow {
                document_index: 0,
                row: expanded_row + 3
            })
            .unwrap(),
        original
            .window
            .capture_position(DocumentRow {
                document_index: 0,
                row: original_row
            })
            .unwrap()
    );
}

#[test]
fn cross_document_partial_images_restore_the_image_document() {
    let mut builder = InlineWindowBuilder::new();
    builder.first = "<p>Before.</p><img src='last.png'/>".into();
    let fixture = builder.build();
    let row = *fixture.documents[0]
        .layout
        .image_maps
        .keys()
        .next()
        .unwrap()
        + 3;
    let origin = DocumentRow {
        document_index: 0,
        row,
    };
    let position = fixture.window.capture_view(origin).unwrap();
    assert_eq!(
        position.text.content_id,
        fixture.documents[1].parsed.source.content_id
    );
    assert_eq!(
        fixture.window.restore_view(&position).unwrap(),
        WindowMovement::Ready(origin)
    );
}
