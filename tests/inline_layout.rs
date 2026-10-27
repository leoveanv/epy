use ers::document::layout::{LayoutOptions, TextLayout, layout_document};
use ers::document::text::parse_document;

struct InlineLayoutBuilder {
    width: usize,
    starting_row: usize,
    html: &'static str,
}

impl InlineLayoutBuilder {
    fn new() -> Self {
        Self {
            width: 40,
            starting_row: 0,
            html: "<body><p>Before.</p><img src='left.png'/><p id='after'><b>After.</b></p><img src='right.png'/><p>End.</p></body>",
        }
    }

    fn with_html(mut self, html: &'static str) -> Self {
        self.html = html;
        self
    }

    fn build(&self, image_rows: usize) -> TextLayout {
        let document = parse_document("chapter.xhtml", self.html).unwrap();
        layout_document(
            &document,
            LayoutOptions {
                width: self.width,
                starting_row: self.starting_row,
                image_rows,
            },
        )
    }
}

#[test]
fn platform_specific_copies_of_an_image_are_shown_once() {
    let fixture = InlineLayoutBuilder::new().with_html(
        "<body><div><img class='figure-epub' src='diagram.png'/><img class='figure-amzn' src='diagram.png'/></div><p>After.</p></body>",
    );
    let layout = fixture.build(5);
    assert_eq!(layout.image_maps.len(), 1);
}

#[test]
fn distinct_images_and_intentional_repetitions_are_not_suppressed() {
    for html in [
        "<body><img src='diagram.png'/><img src='diagram.png'/></body>",
        "<body><img class='figure-epub' src='first.png'/><img class='figure-amzn' src='second.png'/></body>",
        "<body><div><img class='figure-epub' src='diagram.png'/></div><div><img class='figure-amzn' src='diagram.png'/></div></body>",
        "<body><img class='figure-epub' src='diagram.png'/><p>Caption</p><img class='figure-amzn' src='diagram.png'/></body>",
    ] {
        let layout = InlineLayoutBuilder::new().with_html(html).build(5);
        assert_eq!(layout.image_maps.len(), 2, "{html}");
    }
}

#[test]
fn suppressed_image_variants_preserve_text_and_annotation_offsets() {
    let variant = parse_document(
        "chapter.xhtml",
        "<body><div><img class='figure-epub' src='diagram.png'/><img class='figure-amzn' src='diagram.png'/></div><p>After.</p></body>",
    )
    .unwrap();
    let original = parse_document(
        "chapter.xhtml",
        "<body><div><img src='diagram.png'/><img src='diagram.png'/></div><p>After.</p></body>",
    )
    .unwrap();
    assert_eq!(variant.source.text, original.source.text);
    assert_eq!(
        variant.paragraphs.last().unwrap().source_start,
        original.paragraphs.last().unwrap().source_start
    );
}

#[test]
fn figure_anchors_resolve_to_the_matching_image_rows() {
    let html = "<body><p>Before.</p><div id='first'><img src='diagram.png'/></div><p>Between.</p><div id='second'><img src='diagram.png'/></div><p>Caption.</p></body>";
    let document = parse_document("chapter.xhtml", html).unwrap();
    for starting_row in [0, 13] {
        for image_rows in [1, 5] {
            let mut fixture = InlineLayoutBuilder::new().with_html(html);
            fixture.starting_row = starting_row;
            let layout = fixture.build(image_rows);
            let rows = layout.image_maps.keys().copied().collect::<Vec<_>>();
            for (index, fragment) in ["first", "second"].iter().enumerate() {
                let anchor = document
                    .semantics
                    .anchors
                    .iter()
                    .find(|anchor| anchor.fragment == *fragment)
                    .unwrap();
                assert_eq!(
                    layout.row_for_offset(anchor.offset),
                    rows[index],
                    "{fragment}, starting_row={starting_row}, image_rows={image_rows}"
                );
            }
        }
    }
}

#[test]
fn image_only_figure_anchors_resolve_to_the_image_row() {
    let html = "<body><div id='figure'><img src='diagram.png'/></div></body>";
    let document = parse_document("chapter.xhtml", html).unwrap();
    let layout = InlineLayoutBuilder::new().with_html(html).build(5);
    let anchor = document
        .semantics
        .anchors
        .iter()
        .find(|anchor| anchor.fragment == "figure")
        .unwrap();
    assert_eq!(
        layout.row_for_offset(anchor.offset),
        *layout.image_maps.keys().next().unwrap()
    );
}

#[test]
fn text_anchors_between_images_resolve_to_the_text() {
    let html =
        "<body><img src='first.png'/><p id='caption'>Caption.</p><img src='second.png'/></body>";
    let document = parse_document("chapter.xhtml", html).unwrap();
    let layout = InlineLayoutBuilder::new().with_html(html).build(5);
    let anchor = document
        .semantics
        .anchors
        .iter()
        .find(|anchor| anchor.fragment == "caption")
        .unwrap();
    let row = layout.row_for_offset(anchor.offset);
    assert_eq!(layout.text_lines[row], "Caption.");
}

#[test]
fn inline_images_reserve_blank_rows_after_each_marker() {
    let fixture = InlineLayoutBuilder::new();
    let original = fixture.build(1);
    let expanded = fixture.build(5);
    assert_eq!(expanded.text_lines.len(), original.text_lines.len() + 8);
    for row in expanded.image_maps.keys() {
        assert!(expanded.text_lines[*row].contains("[IMAGE]"));
        assert!((1..5).all(|offset| expanded.text_lines[*row + offset].is_empty()));
    }
}

#[test]
fn inline_images_preserve_source_cells_at_their_projected_rows() {
    let fixture = InlineLayoutBuilder::new();
    let original = fixture.build(1);
    let expanded = fixture.build(5);
    let expected = original
        .source_cells
        .iter()
        .cloned()
        .map(|mut cell| {
            cell.row += original
                .image_maps
                .keys()
                .filter(|row| **row < cell.row)
                .count()
                * 4;
            cell
        })
        .collect::<Vec<_>>();
    assert_eq!(expanded.source_cells, expected);
}

#[test]
fn inline_images_preserve_styles_at_their_projected_rows() {
    let fixture = InlineLayoutBuilder::new();
    let original = fixture.build(1);
    let expanded = fixture.build(5);
    let expected = original
        .formatting
        .iter()
        .cloned()
        .map(|mut span| {
            span.row += original
                .image_maps
                .keys()
                .filter(|row| **row < span.row)
                .count()
                * 4;
            span
        })
        .collect::<Vec<_>>();
    assert_eq!(expanded.formatting, expected);
}

#[test]
fn inline_row_mapping_round_trips_rows_with_a_starting_offset() {
    let mut fixture = InlineLayoutBuilder::new();
    fixture.starting_row = 13;
    let original = fixture.build(1);
    let expanded = fixture.build(5);
    let mapping = expanded.text_lines.image_layout();
    for row in fixture.starting_row..fixture.starting_row + original.text_lines.len() {
        let displayed = mapping.display_row(row);
        assert_eq!(mapping.original_row(displayed), row);
        assert_eq!(
            expanded.text_lines[displayed - fixture.starting_row],
            original.text_lines[row - fixture.starting_row]
        );
    }
}

#[test]
fn partial_image_rows_retain_their_image_slot_identity() {
    let layout = InlineLayoutBuilder::new().build(5);
    let mapping = layout.text_lines.image_layout();
    for (ordinal, row) in layout.image_maps.keys().enumerate() {
        for offset in 0..5 {
            let position = mapping.slot_at(row + offset).unwrap();
            assert_eq!(position.ordinal, ordinal);
            assert_eq!(position.row_offset, offset);
            assert_eq!(
                mapping.original_row(row + offset),
                mapping.original_row(*row)
            );
        }
        assert!(mapping.slot_at(row + 5).is_none());
    }
}

#[test]
fn taller_inline_slots_do_not_allocate_more_text_rows() {
    let fixture = InlineLayoutBuilder::new();
    let original = fixture.build(1);
    let expanded = fixture.build(24);
    assert_eq!(expanded.text_lines.len(), original.text_lines.len() + 46);
    assert_eq!(
        expanded.text_lines.storage_bytes(),
        original.text_lines.storage_bytes()
    );
}

#[test]
fn inline_slot_heights_stay_within_the_supported_range() {
    let fixture = InlineLayoutBuilder::new();
    let original = fixture.build(1);
    assert_eq!(fixture.build(0).text_lines, original.text_lines);
    assert_eq!(
        fixture.build(usize::MAX).text_lines,
        fixture.build(24).text_lines
    );
}
