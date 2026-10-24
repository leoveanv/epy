import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator, List

import pytest

from epy_reader.annotations import AnnotationReference, TextAnchor
from epy_reader.ebooks import Epub


@dataclass(frozen=True)
class ReferenceCase:
    markup: str
    quote: str
    headings: List[str]
    page_start: str = ""
    page_end: str = ""


@pytest.mark.parametrize(
    "case",
    [
        ReferenceCase(
            "<h1>FIRST STORY</h1><p>before</p><h1>EUGENE</h1><p>selected passage</p>",
            "selected passage",
            ["EUGENE"],
        ),
        ReferenceCase(
            "<h1>EUGENE</h1><h2>Second scene</h2><p>selected passage</p>",
            "selected passage",
            ["EUGENE", "Second scene"],
        ),
        ReferenceCase(
            '<h1>EUGENE</h1><span epub:type="pagebreak" title="12"/><p>selected passage</p>',
            "selected passage",
            ["EUGENE"],
            "12",
        ),
        ReferenceCase(
            '<h1>EUGENE</h1><span role="doc-pagebreak" aria-label="Page xiii"/><p>selected passage</p>',
            "selected passage",
            ["EUGENE"],
            "xiii",
        ),
        ReferenceCase(
            '<h1>EUGENE</h1><span role="doc-pagebreak" title=" " aria-label="12"/><p>selected passage</p>',
            "selected passage",
            ["EUGENE"],
            "12",
        ),
        ReferenceCase(
            '<h1>EUGENE</h1><span epub:type="pagebreak">12</span><p>selected passage</p>',
            "selected passage",
            ["EUGENE"],
            "12",
        ),
        ReferenceCase(
            '<h1>EUGENE</h1><span epub:type="pagebreak" title="12"/><p>first <span role="doc-pagebreak" aria-label="13"/>second</p>',
            "first second",
            ["EUGENE"],
            "12",
            "13",
        ),
    ],
)
def test_reference_matches_passage_context(case: ReferenceCase) -> None:
    from epy_reader.annotations.references import parse_references, resolve_reference

    document = parse_references(case.markup, "stories.xhtml")
    start = document.text.index(case.quote)
    anchor = TextAnchor(
        content_id=document.content_id, start=start, end=start + len(case.quote), quote=case.quote
    )
    reference = resolve_reference(anchor, document, content_order=2)
    assert reference == AnnotationReference(
        headings=case.headings,
        page_start=case.page_start or None,
        page_end=case.page_end or None,
        location="Location 3:" + str(start + 1),
    )


def test_toc_anchor_selects_correct_story_without_heading_tags() -> None:
    from epy_reader.annotations.references import (
        BookNavigation,
        SectionTarget,
        parse_references,
        resolve_reference,
    )

    document = parse_references(
        '<p id="first">FIRST STORY</p><p>before</p><p id="eugene">EUGENE</p><p>selected passage</p>',
        "stories.xhtml",
    )
    navigation = BookNavigation(
        sections=[
            SectionTarget(content_id="stories.xhtml", section="first", headings=["FIRST STORY"]),
            SectionTarget(content_id="stories.xhtml", section="eugene", headings=["EUGENE"]),
        ]
    )
    start = document.text.index("selected passage")
    anchor = TextAnchor(
        content_id=document.content_id, start=start, end=start + 16, quote="selected passage"
    )
    assert resolve_reference(anchor, document, navigation=navigation).headings == ["EUGENE"]


def test_reference_offsets_match_existing_canonical_text() -> None:
    from epy_reader.annotations.references import parse_references
    from epy_reader.parser import parse_html

    markup = (
        '<h1 id="eugene">EU<i>GENE</i></h1><pre>alpha\t beta\nnext</pre><p>selected passage</p>'
    )
    reference_document = parse_references(markup, "stories.xhtml")
    rendered = parse_html(markup, textwidth=80, content_id="stories.xhtml")
    assert reference_document.text == rendered.documents[0].text
    assert reference_document.anchors["eugene"] == reference_document.text.index("EUGENE")


def test_encoded_fragment_matches_literal_source_id() -> None:
    from epy_reader.annotations.references import (
        parse_navigation,
        parse_references,
        resolve_reference,
    )

    document = parse_references(
        '<p id="Eugene%20story">EUGENE</p><p>selected passage</p>', "stories.xhtml"
    )
    navigation = parse_navigation(
        '<html xmlns:epub="http://www.idpf.org/2007/ops"><nav epub:type="toc"><ol><li><a href="stories.xhtml#Eugene%2520story">EUGENE</a></li></ol></nav></html>',
        "nav.xhtml",
    )
    start = document.text.index("selected passage")
    anchor = TextAnchor(
        content_id=document.content_id, start=start, end=start + 16, quote="selected passage"
    )
    assert resolve_reference(anchor, document, navigation=navigation).headings == ["EUGENE"]


def test_page_list_supplies_page_label_for_empty_pagebreak() -> None:
    from epy_reader.annotations.references import (
        BookNavigation,
        PageTarget,
        parse_references,
        resolve_reference,
    )

    document = parse_references(
        '<h1>EUGENE</h1><span id="p12" epub:type="pagebreak"/><p>selected passage</p>',
        "stories.xhtml",
    )
    navigation = BookNavigation(
        pages=[PageTarget(content_id="stories.xhtml", section="p12", label="12")]
    )
    start = document.text.index("selected passage")
    anchor = TextAnchor(
        content_id=document.content_id, start=start, end=start + 16, quote="selected passage"
    )
    assert resolve_reference(anchor, document, navigation=navigation).page_start == "12"


def test_navigation_resolves_relative_encoded_paths() -> None:
    from epy_reader.annotations.references import parse_navigation

    navigation = parse_navigation(
        '<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="../Text/Story%20One.xhtml#Eugene">EUGENE</a></li></ol></nav><nav epub:type="page-list"><ol><li><a href="../Text/Story%20One.xhtml#p12">12</a></li></ol></nav></body></html>',
        "OPS/Nav/nav.xhtml",
    )
    assert navigation.sections[0].content_id == "OPS/Text/Story One.xhtml"
    assert navigation.sections[0].headings == ["EUGENE"]
    assert navigation.pages[0].label == "12"


@pytest.fixture
def navigation_book(tmp_path: Path) -> Iterator[Epub]:

    path = tmp_path / "navigation.epub"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr(
            "META-INF/container.xml",
            '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>',
        )
        archive.writestr(
            "OPS/book.opf",
            '<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata/><manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/><item id="nav" href="Nav/nav.xhtml" properties="nav" media-type="application/xhtml+xml"/><item id="first" href="Text/stories.xhtml" media-type="application/xhtml+xml"/><item id="next" href="Text/continued.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="first"/><itemref idref="next"/></spine></package>',
        )
        archive.writestr(
            "OPS/toc.ncx",
            '<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint><navLabel><text>LEGACY</text></navLabel><content src="Text/stories.xhtml#Eugene"/></navPoint></navMap></ncx>',
        )
        archive.writestr(
            "OPS/Nav/nav.xhtml",
            '<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><span>STORIES</span><ol><li><a href="../Text/stories.xhtml#Eugene">EUGENE</a></li></ol></li></ol></nav><nav epub:type="page-list"><ol><li><a href="../Text/stories.xhtml#p12">12</a></li></ol></nav></body></html>',
        )
        archive.writestr(
            "OPS/Text/stories.xhtml",
            '<html><body><h1 id="Eugene">EUGENE</h1><span id="p12"/><p>selected passage</p></body></html>',
        )
        archive.writestr(
            "OPS/Text/continued.xhtml", "<html><body><p>continued passage</p></body></html>"
        )
    book = Epub(str(path))
    book.initialize()
    try:
        yield book
    finally:
        book.file.close()


def test_epub3_page_list_wins_over_legacy_ncx(navigation_book: Epub) -> None:
    from epy_reader.annotations.references import BookReferences

    references = BookReferences(navigation_book)
    document = references.document("OPS/Text/stories.xhtml")
    quote = "selected passage"
    start = document.text.index(quote)
    anchor = TextAnchor(
        content_id=document.content_id, start=start, end=start + len(quote), quote=quote
    )
    assert references.for_anchor(anchor, 0).page_start == "12"


def test_story_context_continues_into_next_content_document(navigation_book: Epub) -> None:
    from epy_reader.annotations.references import BookReferences

    references = BookReferences(navigation_book)
    document = references.document("OPS/Text/continued.xhtml")
    quote = "continued passage"
    start = document.text.index(quote)
    anchor = TextAnchor(
        content_id=document.content_id, start=start, end=start + len(quote), quote=quote
    )
    assert references.for_anchor(anchor, 1).headings == ["STORIES", "EUGENE"]


def test_ncx_navigation_resolves_targets() -> None:
    from epy_reader.annotations.references import parse_navigation

    navigation = parse_navigation(
        '<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint><navLabel><text>EUGENE</text></navLabel><content src="Text/stories.xhtml#Eugene"/></navPoint></navMap><pageList><pageTarget value="12"><navLabel><text>12</text></navLabel><content src="Text/stories.xhtml#p12"/></pageTarget></pageList></ncx>',
        "OPS/toc.ncx",
    )
    assert navigation.sections[0].content_id == "OPS/Text/stories.xhtml"
    assert navigation.sections[0].headings == ["EUGENE"]
    assert navigation.pages[0].label == "12"
