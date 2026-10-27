use ers::annotations::references::{ReferenceRequest, resolve_reference};
use ers::annotations::{AnnotationReference, TextAnchor};
use ers::book::{BookNavigation, parse_navigation};
use ers::document::text::{ParsedDocument, parse_document};

struct ReferenceCase {
    markup: &'static str,
    quote: &'static str,
    headings: Vec<String>,
    page_start: Option<String>,
    page_end: Option<String>,
}

impl ReferenceCase {
    fn new(markup: &'static str, quote: &'static str, headings: &[&str]) -> Self {
        Self {
            markup,
            quote,
            headings: headings.iter().map(|title| (*title).into()).collect(),
            page_start: None,
            page_end: None,
        }
    }

    fn pages(mut self, first: &str, last: Option<&str>) -> Self {
        self.page_start = Some(first.into());
        self.page_end = last.map(str::to_owned);
        self
    }
}

fn anchor(document: &ParsedDocument, quote: &str) -> TextAnchor {
    let byte = document.source.text.find(quote).unwrap();
    TextAnchor::new(
        &document.source.content_id,
        document.source.text[..byte].chars().count(),
        quote,
    )
    .unwrap()
}

fn reference(
    document: &ParsedDocument,
    quote: &str,
    navigation: &BookNavigation,
) -> AnnotationReference {
    resolve_reference(ReferenceRequest {
        anchor: &anchor(document, quote),
        document,
        navigation,
        content_order: 2,
        fallback_headings: &[],
    })
    .unwrap()
}

#[test]
fn references_use_the_selected_passages_heading_context() {
    let cases = [
        ReferenceCase::new("<h1>FIRST STORY</h1><p>before</p><h1>SECOND STORY</h1><p>selected passage</p>", "selected passage", &["SECOND STORY"]),
        ReferenceCase::new("<h1>STORY ALPHA</h1><h2>Second scene</h2><p>selected passage</p>", "selected passage", &["STORY ALPHA", "Second scene"]),
        ReferenceCase::new("<h1>STORY ALPHA</h1><span epub:type='pagebreak' title='12'/><p>selected passage</p>", "selected passage", &["STORY ALPHA"]).pages("12", None),
        ReferenceCase::new("<h1>STORY ALPHA</h1><span role='doc-pagebreak' aria-label='Page xiii'/><p>selected passage</p>", "selected passage", &["STORY ALPHA"]).pages("xiii", None),
        ReferenceCase::new("<h1>STORY ALPHA</h1><span role='doc-pagebreak' title=' ' aria-label='12'/><p>selected passage</p>", "selected passage", &["STORY ALPHA"]).pages("12", None),
        ReferenceCase::new("<h1>STORY ALPHA</h1><span epub:type='pagebreak'>12</span><p>selected passage</p>", "selected passage", &["STORY ALPHA"]).pages("12", None),
        ReferenceCase::new("<h1>STORY ALPHA</h1><span epub:type='pagebreak'><span>2</span>5</span><p>selected passage</p>", "selected passage", &["STORY ALPHA"]).pages("25", None),
        ReferenceCase::new("<h1>STORY ALPHA</h1><span epub:type='pagebreak' title='12'/><p>first <span role='doc-pagebreak' aria-label='13'/>second</p>", "first second", &["STORY ALPHA"]).pages("12", Some("13")),
        ReferenceCase::new("<head><h1>Hidden</h1><span role='doc-pagebreak' title='99'/></head><h1>Café <i>scene</i></h1><pre>alpha\t beta\nnext</pre><p>selected 👩‍🔬 passage</p>", "selected 👩‍🔬 passage", &["Café scene"]),
    ];
    for case in cases {
        let parsed = parse_document("stories.xhtml", case.markup).unwrap();
        let selected = anchor(&parsed, case.quote);
        assert_eq!(
            reference(&parsed, case.quote, &BookNavigation::default()),
            AnnotationReference {
                headings: case.headings,
                location: format!("Location 3:{}", selected.start() + 1),
                page_start: case.page_start,
                page_end: case.page_end,
                version: 1
            }
        );
    }
}

#[test]
fn navigation_fragments_select_the_latest_story_boundary() {
    let parsed = parse_document("OPS/Text/Story One.xhtml", "<p id='first'>FIRST STORY</p><p>before</p><p id='second%20story'>SECOND STORY</p><p>selected passage</p>").unwrap();
    let navigation = parse_navigation("<html xmlns:epub='http://www.idpf.org/2007/ops'><nav epub:type='toc'><ol><li><span>STORIES</span><ol><li><a href='../Text/Story%20One.xhtml#first'>FIRST STORY</a></li><li><a href='../Text/Story%20One.xhtml#second%2520story'>SECOND STORY</a></li></ol></li></ol></nav></html>", "OPS/Nav/nav.xhtml").unwrap();
    assert_eq!(
        reference(&parsed, "selected passage", &navigation).headings,
        ["STORIES", "SECOND STORY"]
    );
}

#[test]
fn page_list_labels_fill_empty_markers_without_replacing_publisher_labels() {
    let navigation = parse_navigation("<html xmlns:epub='http://www.idpf.org/2007/ops'><nav epub:type='page-list'><a href='stories.xhtml#p12'>Page 12</a><a href='stories.xhtml#p13'>13</a></nav></html>", "nav.xhtml").unwrap();
    let parsed = parse_document("stories.xhtml", "<span id='p12' epub:type='pagebreak'/><p>first <span id='p13' role='doc-pagebreak' title='Publisher 13'/>second</p>").unwrap();
    let result = reference(&parsed, "first second", &navigation);
    assert_eq!(
        (result.page_start.as_deref(), result.page_end.as_deref()),
        (Some("12"), Some("Publisher 13"))
    );
}

#[test]
fn ncx_navigation_preserves_parent_headings_and_page_targets() {
    let navigation = parse_navigation("<ncx xmlns='http://www.daisy.org/z3986/2005/ncx/'><navMap><navPoint><navLabel><text>STORIES</text></navLabel><content src='Text/stories.xhtml'/><navPoint><navLabel><text>STORY ALPHA</text></navLabel><content src='Text/stories.xhtml#alpha'/></navPoint></navPoint></navMap><pageList><pageTarget value='12'><content src='Text/stories.xhtml#p12'/></pageTarget></pageList></ncx>", "OPS/toc.ncx").unwrap();
    let parsed = parse_document(
        "OPS/Text/stories.xhtml",
        "<p id='alpha'>STORY ALPHA</p><span id='p12'/><p>selected passage</p>",
    )
    .unwrap();
    let result = reference(&parsed, "selected passage", &navigation);
    assert_eq!(result.headings, ["STORIES", "STORY ALPHA"]);
    assert_eq!(result.page_start.as_deref(), Some("12"));
}

#[test]
fn repeated_quotes_do_not_relocate_reference_anchors() {
    let parsed = parse_document(
        "stories.xhtml",
        "<h1>Story</h1><p>wrong selected passage</p>",
    )
    .unwrap();
    let selected = TextAnchor::new("stories.xhtml", 0, "selected passage").unwrap();
    assert!(
        resolve_reference(ReferenceRequest {
            anchor: &selected,
            document: &parsed,
            navigation: &BookNavigation::default(),
            content_order: 0,
            fallback_headings: &[]
        })
        .is_err()
    );
}

#[test]
fn continued_documents_use_the_previous_navigation_context() {
    let parsed = parse_document("continued.xhtml", "<p>continued passage</p>").unwrap();
    let fallback = vec!["STORIES".into(), "STORY ALPHA".into()];
    let result = resolve_reference(ReferenceRequest {
        anchor: &anchor(&parsed, "continued passage"),
        document: &parsed,
        navigation: &BookNavigation::default(),
        content_order: 3,
        fallback_headings: &fallback,
    })
    .unwrap();
    assert_eq!(result.headings, fallback);
}

#[test]
fn deeply_nested_navigation_is_rejected_without_recursing_unboundedly() {
    let markup = format!(
        "<html xmlns:epub='http://www.idpf.org/2007/ops'><nav epub:type='toc'>{}<a href='stories.xhtml'>Story</a>{}</nav></html>",
        "<ol><li>".repeat(200),
        "</li></ol>".repeat(200)
    );
    assert!(parse_navigation(&markup, "nav.xhtml").is_err());
}
