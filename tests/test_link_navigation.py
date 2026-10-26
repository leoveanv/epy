from pathlib import Path

import pytest
from link_helpers import LinkChapter, build_link_book

from epy_reader.ebooks.epub import Epub
from epy_reader.ebooks.fictionbook import FictionBook
from epy_reader.links.models import ContentPosition
from epy_reader.parser import parse_html


def test_internal_link_resolves_percent_encoded_fragment(tmp_path: Path) -> None:
    from epy_reader.links.navigation import BookLinks

    path = tmp_path / "book.epub"
    build_link_book(
        path,
        [
            LinkChapter(body='<p>First passage <a href="notes.xhtml#note%201">note</a></p>'),
            LinkChapter(
                path="notes.xhtml", body='<p>before</p><p id="note 1">target</p>', in_spine=False
            ),
        ],
    )
    ebook = Epub(str(path))
    ebook.initialize()
    links = BookLinks(ebook)
    target = links.resolve("chapter.xhtml", "notes.xhtml#note%201")
    assert links.position(target) == ContentPosition(content_id="notes.xhtml", offset=7)


@pytest.mark.parametrize(
    "href",
    [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "../../outside.xhtml",
        "https://",
        "data:text/html,hi",
        "notes.xhtml#missing",
    ],
)
def test_invalid_link_target_reports_failure(tmp_path: Path, href: str) -> None:
    from epy_reader.links.navigation import BookLinks

    path = tmp_path / "book.epub"
    build_link_book(path)
    ebook = Epub(str(path))
    ebook.initialize()
    links = BookLinks(ebook)
    with pytest.raises(ValueError):
        links.position(links.resolve("chapter.xhtml", href))


def test_fb2_note_link_resolves_across_bodies(tmp_path: Path) -> None:
    from epy_reader.links.navigation import BookLinks

    path = tmp_path / "book.fb2"
    path.write_text(
        '<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0" xmlns:l="http://www.w3.org/1999/xlink"><body><section><p>First passage <a l:href="#n" type="note">1</a></p></section></body><body name="notes"><section id="n"><p>target</p></section></body></FictionBook>'
    )
    ebook = FictionBook(str(path))
    ebook.initialize()
    links = BookLinks(ebook)
    target = links.resolve(str(ebook.contents[0]), "#n")
    assert links.position(target).content_id == str(ebook.contents[1])


def test_external_link_resolves_without_changing_book(tmp_path: Path) -> None:
    from epy_reader.links.navigation import BookLinks

    path = tmp_path / "book.epub"
    build_link_book(path)
    ebook = Epub(str(path))
    ebook.initialize()
    links = BookLinks(ebook)
    assert (
        links.resolve("chapter.xhtml", "https://example.org/a?q=1#b").url
        == "https://example.org/a?q=1#b"
    )


def test_history_restores_latest_scrolled_destination() -> None:
    from epy_reader.links.navigation import JumpHistory

    history = JumpHistory()
    origin = ContentPosition(content_id="chapter", offset=50)
    destination = ContentPosition(content_id="notes", offset=10)
    scrolled = ContentPosition(content_id="notes", offset=200)
    history.record(origin, destination)
    assert history.backward(scrolled) == origin
    assert history.forward(origin) == scrolled


def test_new_jump_discards_forward_branch() -> None:
    from epy_reader.links.navigation import JumpHistory

    history = JumpHistory()
    first = ContentPosition(content_id="a")
    second = ContentPosition(content_id="b")
    third = ContentPosition(content_id="c")
    history.record(first, second)
    history.backward(second)
    history.record(first, third)
    assert history.forward(third) is None
    assert history.backward(third) == first


def test_history_restores_partial_inline_image() -> None:
    from epy_reader.images.layout import ImageLayout
    from epy_reader.links.navigation import capture_position, position_row

    source = parse_html(
        '<p>before</p><img src="a.png"/><p>after</p>', textwidth=80, content_id="chapter"
    )
    first = ImageLayout.expand(source, image_rows=10).structure
    second = ImageLayout.expand(source, image_rows=6).structure
    image_row = next(iter(first.image_maps))
    position = capture_position(first, image_row + 3, image_rows=10)
    assert position_row(second, position, image_rows=6) == next(iter(second.image_maps)) + 3


def test_jump_restore_uses_canonical_offsets_after_rewrap() -> None:
    from epy_reader.links.navigation import capture_position, position_row

    wide = parse_html(
        '<p>First passage <a href="#n"><sup>1</sup></a> and more text.</p>',
        textwidth=80,
        content_id="a",
    )
    narrow = parse_html(
        '<p>First passage <a href="#n"><sup>1</sup></a> and more text.</p>',
        textwidth=8,
        content_id="a",
    )
    target_row = next(cell.row for cell in narrow.source_cells if cell.text == "[")
    position = capture_position(narrow, target_row)
    assert position_row(wide, position) == 0


def test_dense_hints_never_overwrite_another_number() -> None:
    from epy_reader.links.navigation import hint_placements, visible_hints

    structure = parse_html(
        '<p><a href="#a">a</a><a href="#b">b</a><a href="#c">c</a></p>',
        textwidth=12,
        content_id="a",
    )
    hints = visible_hints(structure, top_row=0, height=10, spread=1)
    placements = hint_placements(hints, textwidth=12)
    assert [placement.column for placement in placements] == [0, 3, 6]


def test_visible_hints_number_wrapped_links_once() -> None:
    from epy_reader.links.navigation import visible_hints

    structure = parse_html(
        '<p><a href="#a">long link wraps over lines</a> <a href="#b">second</a></p>',
        textwidth=9,
        content_id="a",
    )
    hints = visible_hints(structure, top_row=0, height=10, spread=1)
    assert [hint.number for hint in hints] == [1, 2]
    assert [hint.link.href for hint in hints] == ["#a", "#b"]
