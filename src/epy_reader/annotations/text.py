"""Canonical text uses parser paragraphs joined with a newline, before display wrapping."""

import textwrap
from dataclasses import dataclass
from typing import Iterator, List, Sequence

import regex
from wcwidth import wcswidth

# NOTE: Unicode grapheme clusters keep accents and joined emoji together during cursor movement.
_GRAPHEMES = regex.compile(r"\X")


@dataclass(frozen=True)
class SourceDocument:
    content_id: str
    text: str
    normalization_version: int = 1


@dataclass(frozen=True)
class SourceCell:
    content_id: str
    start: int
    end: int
    row: int
    col: int
    width: int
    text: str


def normalize_paragraph(text: str) -> str:
    return text.expandtabs().replace("\r", " ").replace("\v", " ").replace("\f", " ")


def graphemes(text: str) -> List[str]:
    return _GRAPHEMES.findall(text)


def grapheme_ends(text: str) -> Iterator[int]:
    if text.isascii() and "\r" not in text:
        yield from range(1, len(text) + 1)
    else:
        for match in _GRAPHEMES.finditer(text):
            yield match.end()


def cell_width(text: str) -> int:
    if text.isascii() and text.isprintable():
        return len(text)
    return max(0, wcswidth(text))


def clip_text(text: str, width: int) -> str:
    if width <= 0:
        return ""
    if text.isascii() and text.isprintable():
        return text[:width]
    occupied = 0
    end = 0
    for match in _GRAPHEMES.finditer(text):
        occupied += cell_width(match.group())
        if occupied > width:
            break
        end = match.end()
    return text[:end]


def wrap_text(text: str, width: int) -> List[str]:
    if text.isascii():
        return textwrap.wrap(text, max(1, width))
    lines = []
    remaining = text.expandtabs().replace("\n", " ").replace("\r", " ").strip()
    while remaining:
        fragment = clip_text(remaining, max(2, width))
        if not fragment:
            break
        if len(fragment) < len(remaining) and not remaining[len(fragment)].isspace():
            boundary = fragment.rfind(" ")
            if boundary > 0:
                fragment = fragment[:boundary]
        lines.append(fragment.rstrip())
        remaining = remaining[len(fragment) :].lstrip()
    return lines


def map_paragraph(
    document: SourceDocument,
    offset: int,
    paragraph: str,
    rendered: Sequence[str],
    starting_row: int,
    indentation: int = 0,
) -> List[SourceCell]:
    cells = []
    cursor = 0
    for row_index, line in enumerate(rendered):
        fragment = line[indentation:].strip()
        if not fragment:
            continue
        location = paragraph.find(fragment, cursor)
        if location < 0:
            continue
        col = cell_width(line[: line.find(fragment)])
        position = offset + location
        for cluster in graphemes(fragment):
            width = cell_width(cluster)
            cells.append(
                SourceCell(
                    content_id=document.content_id,
                    start=position,
                    end=position + len(cluster),
                    row=starting_row + row_index,
                    col=col,
                    width=width,
                    text=cluster,
                )
            )
            position += len(cluster)
            col += width
        cursor = location + len(fragment)
    return cells
