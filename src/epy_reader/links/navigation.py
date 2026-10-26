"""Canonical link resolution, viewport positions, and bounded session history."""

import posixpath
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional, Set, Union, cast
from urllib.parse import unquote, urljoin, urlsplit

from epy_reader.annotations.text import SourceCell
from epy_reader.ebooks import Ebook, Epub, Mobi
from epy_reader.ebooks.fictionbook import FictionBook
from epy_reader.ebooks.url import URL
from epy_reader.links.models import ContentPosition, LinkTarget, TextLink
from epy_reader.models import TextStructure
from epy_reader.parser import parse_html


@dataclass(frozen=True)
class LinkHint:
    number: int
    link: TextLink
    cell: SourceCell


@dataclass(frozen=True)
class HintPlacement:
    hint: LinkHint
    column: int


def hint_placements(hints: List[LinkHint], *, textwidth: int) -> List[HintPlacement]:
    occupied = cast(Dict[int, Set[int]], {})
    placements = []
    for hint in hints:
        width = len(str(hint.number)) + 2
        used = occupied.setdefault(hint.cell.row, set())
        preferred = min(hint.cell.col, textwidth - width)
        candidates = list(range(preferred, textwidth - width + 1)) + list(range(max(0, preferred)))
        column = next(
            (
                column
                for column in candidates
                if not used.intersection(range(column, column + width))
            ),
            None,
        )
        if column is None:
            return []
        used.update(range(column, column + width))
        placements.append(HintPlacement(hint, column))
    return placements


def visible_hints(
    structure: TextStructure, *, top_row: int, height: int, spread: int
) -> List[LinkHint]:
    cells = [
        cell for cell in structure.source_cells if top_row <= cell.row < top_row + height * spread
    ]
    hints = cast(List[LinkHint], [])
    for link in structure.links:
        cell = next(
            (
                cell
                for cell in cells
                if cell.content_id == link.content_id
                and cell.start < link.end
                and cell.end > link.start
            ),
            None,
        )
        if cell is not None:
            hints.append(LinkHint(len(hints) + 1, link, cell))
    return hints


def capture_position(
    structure: TextStructure, top_row: int, *, image_rows: int = 0
) -> ContentPosition:
    if not structure.source_cells:
        content_id = structure.documents[0].content_id if structure.documents else ""
        return ContentPosition(content_id=content_id)
    cell = next(
        (cell for cell in structure.source_cells if cell.row >= top_row), structure.source_cells[-1]
    )
    position = ContentPosition(
        content_id=cell.content_id, offset=cell.start, screen_row=cell.row - top_row
    )
    for row, path in structure.image_maps.items():
        if row <= top_row < row + image_rows:
            return position.model_copy(update=dict(image_path=path, image_offset=top_row - row))
    return position


def position_row(
    structure: TextStructure, position: ContentPosition, *, image_rows: int = 0
) -> int:
    if position.image_path and image_rows:
        for row, path in structure.image_maps.items():
            candidate = capture_position(structure, row)
            if (
                path == position.image_path
                and candidate.content_id == position.content_id
                and candidate.offset == position.offset
            ):
                return row + min(position.image_offset, image_rows - 1)
    cells = [cell for cell in structure.source_cells if cell.content_id == position.content_id]
    if not cells:
        return 0
    cell = next((cell for cell in cells if cell.end > position.offset), cells[-1])
    return max(0, cell.row - position.screen_row)


class JumpHistory:
    entries: List[ContentPosition]
    cursor: int
    limit: int

    def __init__(self, limit: int = 100) -> None:
        self.entries = []
        self.cursor = -1
        self.limit = max(2, limit)

    def record(self, origin: ContentPosition, destination: ContentPosition) -> None:
        if origin == destination:
            return
        self.entries = self.entries[: self.cursor + 1]
        if not self.entries or self.entries[-1] != origin:
            self.entries.append(origin)
        self.entries.append(destination)
        self.entries = self.entries[-self.limit :]
        self.cursor = len(self.entries) - 1

    def backward(self, current: ContentPosition) -> Optional[ContentPosition]:
        return self.move(current, -1)

    def forward(self, current: ContentPosition) -> Optional[ContentPosition]:
        return self.move(current, 1)

    def move(self, current: ContentPosition, distance: int) -> Optional[ContentPosition]:
        target = self.cursor + distance
        if not 0 <= target < len(self.entries):
            return None
        self.entries[self.cursor] = current
        self.cursor = target
        return self.entries[target]


class BookLinks:
    ebook: Ebook
    documents: Dict[str, Union[str, ET.Element]]
    cache: Dict[str, TextStructure]

    def __init__(self, ebook: Ebook) -> None:
        self.ebook = ebook
        self.documents = {str(content): content for content in ebook.contents}
        self.cache = {}
        if isinstance(ebook, Epub):
            self.load_manifest(ebook)

    def load_manifest(self, ebook: Epub) -> None:
        package_path = (
            str(Path(ebook.root_dirpath) / "content.opf")
            if isinstance(ebook, Mobi)
            else ebook.root_filepath
        )
        package = ET.fromstring(ebook.get_raw_text(package_path))
        for item in package.iter():
            if (
                item.tag.rsplit("}", 1)[-1] == "item"
                and item.get("media-type") in {"application/xhtml+xml", "text/html"}
                and "nav" not in item.get("properties", "").split()
            ):
                content_id = unquote(urljoin(package_path, item.get("href", "")))
                self.documents.setdefault(content_id, content_id)

    def include_supplemental(self) -> None:
        if not isinstance(self.ebook, Epub):
            return
        existing = {str(content) for content in self.ebook.contents}
        self.ebook.contents = tuple(str(content) for content in self.ebook.contents) + tuple(
            key for key in self.documents if key not in existing
        )

    def document(self, content_id: str) -> TextStructure:
        if content_id not in self.documents:
            raise ValueError("The linked document is not in this ebook.")
        if content_id not in self.cache:
            source = self.ebook.get_raw_text(self.documents[content_id])
            structure = parse_html(source, textwidth=80, content_id=content_id)
            assert isinstance(structure, TextStructure)
            self.cache[content_id] = structure
        return self.cache[content_id]

    def resolve(self, content_id: str, href: str) -> LinkTarget:
        if any(ord(char) < 32 or ord(char) == 127 for char in href):
            raise ValueError("The link contains control characters.")
        parsed = urlsplit(href)
        if isinstance(self.ebook, URL) and not href.startswith("#"):
            href = urljoin(self.ebook.path, href)
            parsed = urlsplit(href)
        if parsed.netloc and not parsed.scheme:
            href = "https:" + href
            parsed = urlsplit(href)
        if parsed.scheme or parsed.netloc:
            if parsed.scheme not in {"http", "https"} or not parsed.hostname:
                raise ValueError("Only HTTP and HTTPS links can open in a browser.")
            return LinkTarget(url=href)
        fragment = unquote(parsed.fragment)
        path = unquote(parsed.path)
        destination = (
            posixpath.normpath(posixpath.join(posixpath.dirname(content_id), path))
            if path
            else content_id
        )
        if isinstance(self.ebook, FictionBook) and fragment:
            candidates = [
                key
                for key in self.documents
                if any(anchor.fragment == fragment for anchor in self.document(key).anchors)
            ]
            if len(candidates) != 1:
                raise ValueError("The note destination is missing or ambiguous.")
            destination = candidates[0]
        if destination not in self.documents:
            raise ValueError("The linked document is not in this ebook.")
        return LinkTarget(content_id=destination, fragment=fragment)

    def position(self, target: LinkTarget) -> ContentPosition:
        if target.url:
            raise ValueError("External links do not have a reading position.")
        document = self.document(target.content_id)
        if not target.fragment:
            return ContentPosition(content_id=target.content_id)
        anchors = [anchor for anchor in document.anchors if anchor.fragment == target.fragment]
        if len(anchors) != 1:
            raise ValueError("The link destination is missing or ambiguous: " + target.fragment)
        return ContentPosition(content_id=target.content_id, offset=anchors[0].offset)
