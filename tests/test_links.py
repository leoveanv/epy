"""Link metadata and display projections preserve canonical annotation positions."""

import pytest

from epy_reader.annotations.models import TextAnchor, resolve_anchor
from epy_reader.annotations.view import Selection
from epy_reader.images.layout import ImageLayout
from epy_reader.models import TextStructure
from epy_reader.parser import parse_html
from epy_reader.utils import merge_text_structures


def linked_text(body: str, *, width: int = 80, content_id: str = "chapter.xhtml") -> TextStructure:
    structure = parse_html(
        "<html><body>" + body + "</body></html>", textwidth=width, content_id=content_id
    )
    assert isinstance(structure, TextStructure)
    return structure


@pytest.mark.parametrize(
    "marker",
    [
        '<a href="notes.xhtml#n1"><sup>1</sup></a>',
        '<sup><a href="notes.xhtml#n1">1</a></sup>',
        '<a href="notes.xhtml#n1" epub:type="noteref">1</a>',
        '<a href="notes.xhtml#n1" role="doc-noteref">[1]</a>',
        '<a xlink:href="#n1" type="note">1</a>',
        '<span role="doc-noteref"><a href="notes.xhtml#n1">1</a></span>',
    ],
)
def test_citation_markup_displays_bracketed_label(marker: str) -> None:
    structure = linked_text("<p>At heart." + marker + " After.</p>")
    assert structure.text_lines[0] == "At heart.[1] After."


def test_citation_display_preserves_existing_anchor() -> None:
    structure = linked_text('<p>Before<a href="#note"><sup>14</sup></a> after</p>')
    anchor = TextAnchor(content_id="chapter.xhtml", start=0, end=17, quote="Before^{14} after")
    assert resolve_anchor(anchor, structure.documents[0]) == anchor
    assert structure.text_lines[0] == "Before[14] after"
    after = next(cell for cell in structure.source_cells if cell.text == "a")
    assert after.start == structure.documents[0].text.index("after")


def test_selecting_bracketed_citation_keeps_full_source_range() -> None:
    structure = linked_text('<p><a href="#n"><sup>1</sup></a></p>')
    selection = Selection(structure, 0)
    selection.mark = 0
    selection.cursor = 2
    assert selection.anchor().quote == "^{1}"


def test_mathematical_superscripts_keep_their_notation() -> None:
    structure = linked_text('<p>x<sup>2</sup> + H<sub>2</sub>O<a href="#n"><sup>1</sup></a></p>')
    assert structure.text_lines[0] == "x^{2} + H_{2}O[1]"


def test_wrapped_links_retain_canonical_text_ranges() -> None:
    structure = linked_text(
        '<p>界 <a href="https://example.org">a long linked phrase</a></p>', width=10
    )
    link = structure.links[0]
    assert structure.documents[0].text[link.start : link.end] == "a long linked phrase"
    assert link.href == "https://example.org"
    assert (
        len(
            {
                cell.row
                for cell in structure.source_cells
                if cell.start < link.end and cell.end > link.start
            }
        )
        > 1
    )


def test_all_destination_ids_are_indexed_at_exact_offsets() -> None:
    structure = linked_text('<p>first <a name="legacy"/>middle <span id="target"/>last</p>')
    offsets = {anchor.fragment: anchor.offset for anchor in structure.anchors}
    assert offsets["target"] == structure.documents[0].text.index("last")
    assert offsets["legacy"] == structure.documents[0].text.index("middle")


def test_hidden_links_are_not_followable() -> None:
    structure = linked_text('<head><a href="https://hidden">hidden</a></head><p>visible</p>')
    assert structure.links == ()


def test_seamless_documents_keep_duplicate_fragment_names() -> None:
    first = linked_text('<p id="n">one <a href="#n">back</a></p>', content_id="one.xhtml")
    second = linked_text('<p id="n">two</p>', content_id="two.xhtml")
    merged = merge_text_structures(first, second)
    assert {(anchor.content_id, anchor.fragment) for anchor in merged.anchors} == {
        ("one.xhtml", "n"),
        ("two.xhtml", "n"),
    }
    assert merged.links == first.links


def test_image_reflow_preserves_link_source_positions() -> None:
    source = linked_text('<img src="a.png"/><p><a href="#n"><sup>1</sup></a></p>')
    expanded = ImageLayout.expand(source, image_rows=6).structure
    assert expanded.links == source.links
    assert expanded.anchors == source.anchors
    assert expanded.source_cells[0].row == source.source_cells[0].row + 5
