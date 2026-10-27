use std::collections::BTreeMap;

use ers::document::layout::{LayoutOptions, SourceCell, StyleSpan, layout_document};
use ers::document::links::{DocumentAnchor, TextLink};
use ers::document::text::parse_document;
use serde::Deserialize;

#[derive(Deserialize)]
struct LayoutFixture {
    name: String,
    html: String,
    width: usize,
    starting_row: usize,
    text_lines: Vec<String>,
    source_cells: Vec<SourceCell>,
    formatting: Vec<StyleSpan>,
    image_maps: BTreeMap<usize, String>,
    links: Vec<TextLink>,
    anchors: Vec<DocumentAnchor>,
}

#[test]
fn layout_fixtures_preserve_visible_rows() {
    let fixtures =
        serde_json::from_str::<Vec<LayoutFixture>>(include_str!("fixtures/layouts.json")).unwrap();
    for fixture in fixtures {
        let document = parse_document("chapter.xhtml", &fixture.html).unwrap();
        let layout = layout_document(
            &document,
            LayoutOptions {
                width: fixture.width,
                starting_row: fixture.starting_row,
                image_rows: 1,
            },
        );
        assert_eq!(
            layout.text_lines.iter().cloned().collect::<Vec<_>>(),
            fixture.text_lines,
            "{}",
            fixture.name
        );
        assert_eq!(layout.image_maps, fixture.image_maps, "{}", fixture.name);
        assert_eq!(
            layout.source_cells, fixture.source_cells,
            "{}",
            fixture.name
        );
        assert_eq!(layout.formatting, fixture.formatting, "{}", fixture.name);
        assert_eq!(document.semantics.links, fixture.links, "{}", fixture.name);
        assert_eq!(
            document.semantics.anchors, fixture.anchors,
            "{}",
            fixture.name
        );
    }
}
