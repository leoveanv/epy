"""Resolve story headings and publisher pages independently of screen layout."""

import xml.etree.ElementTree as ET
import zipfile
from dataclasses import dataclass, replace
from typing import Dict, List, Optional, Sequence, Tuple
from urllib.parse import unquote, urldefrag, urljoin

from pydantic import Field
from typing_extensions import TypeAlias

from epy_reader.annotations.models import (
    Annotation,
    AnnotationReference,
    FrozenRecord,
    TextAnchor,
    resolve_anchor,
    stable_location,
)
from epy_reader.annotations.text import SourceDocument, normalize_paragraph
from epy_reader.ebooks import Epub
from epy_reader.models import CharPos
from epy_reader.parser import HTMLtoLines

AttributeValue: TypeAlias = Optional[str]


class ReferenceHeading(FrozenRecord):
    offset: int
    title: str
    level: int = 1


class ReferencePage(FrozenRecord):
    offset: int
    label: Optional[str] = None


class DocumentReferences(FrozenRecord):
    content_id: str
    text: str
    headings: List[ReferenceHeading]
    pages: List[ReferencePage]
    anchors: Dict[str, int]


class SectionTarget(FrozenRecord):
    content_id: str
    section: Optional[str] = None
    headings: List[str]


class PageTarget(FrozenRecord):
    content_id: str
    section: Optional[str] = None
    label: str


class BookNavigation(FrozenRecord):
    sections: List[SectionTarget] = Field(default_factory=list)
    pages: List[PageTarget] = Field(default_factory=list)


@dataclass(frozen=True)
class HeadingPosition:
    position: CharPos
    level: int
    title: str = ""


@dataclass(frozen=True)
class PagePosition:
    position: CharPos
    label: Optional[str]


@dataclass(frozen=True)
class OpenPage:
    index: int
    tag: str
    depth: int


class ReferenceParser(HTMLtoLines):
    anchors: Dict[str, CharPos]
    headings: List[HeadingPosition]
    pages: List[PagePosition]
    open_pages: List[OpenPage]
    tag_depths: Dict[str, int]
    active_heading: Optional[HeadingPosition]

    def __init__(self) -> None:
        super().__init__()
        self.anchors = {}
        self.headings = []
        self.pages = []
        self.open_pages = []
        self.tag_depths = {}
        self.active_heading = None

    def record_start(
        self, tag: str, attrs: List[Tuple[str, AttributeValue]], self_closing: bool
    ) -> None:
        if self.ishidden or tag in self.hide:
            return
        attributes = dict(attrs)
        position = CharPos(row=len(self.text) - 1, col=len(self.text[-1]))
        section = attributes.get("id") or attributes.get("xml:id") or attributes.get("name")
        if section:
            self.anchors.setdefault(section, position)
        if len(tag) == 2 and tag[0] == "h" and tag[1] in "123456":
            self.active_heading = HeadingPosition(position, int(tag[1]))
        semantics = " ".join(value or "" for key, value in attrs if key.endswith(":type")).split()
        role = (attributes.get("role") or "").split()
        if "pagebreak" in semantics or "doc-pagebreak" in role:
            # NOTE: EPUB pagebreak titles take precedence over text; ARIA labels cover accessible empty markers.
            label = clean_page_label(attributes.get("title")) or clean_page_label(
                attributes.get("aria-label")
            )
            self.pages.append(PagePosition(position, label))
            if not self_closing and not label:
                self.open_pages.append(OpenPage(len(self.pages) - 1, tag, self.tag_depths[tag]))

    def handle_starttag(self, tag: str, attrs: List[Tuple[str, AttributeValue]]) -> None:
        self.tag_depths[tag] = self.tag_depths.get(tag, 0) + 1
        self.record_start(tag, attrs, False)
        super().handle_starttag(tag, attrs)

    def handle_startendtag(self, tag: str, attrs: List[Tuple[str, AttributeValue]]) -> None:
        self.record_start(tag, attrs, True)
        super().handle_startendtag(tag, attrs)

    def text_since(self, position: CharPos) -> str:
        parts = self.text[position.row :]
        parts[0] = parts[0][position.col :]
        return " ".join("\n".join(parts).split())

    def handle_endtag(self, tag: str) -> None:
        if self.active_heading is not None and tag == "h" + str(self.active_heading.level):
            self.headings.append(
                replace(self.active_heading, title=self.text_since(self.active_heading.position))
            )
            self.active_heading = None
        remaining = []
        for page in self.open_pages:
            if page.tag == tag and page.depth == self.tag_depths.get(tag, 0):
                marker = self.pages[page.index]
                self.pages[page.index] = replace(
                    marker, label=self.text_since(marker.position) or None
                )
            else:
                remaining.append(page)
        self.open_pages = remaining
        self.tag_depths[tag] = max(0, self.tag_depths.get(tag, 0) - 1)
        super().handle_endtag(tag)

    def source_offset(self, position: CharPos, offsets: Sequence[int]) -> int:
        return offsets[position.row] + len(
            normalize_paragraph(self.text[position.row][: position.col])
        )

    def document(self, content_id: str) -> DocumentReferences:
        paragraphs = [normalize_paragraph(line) for line in self.text]
        offsets = [0]
        for line in paragraphs:
            offsets.append(offsets[-1] + len(line) + 1)
        return DocumentReferences(
            content_id=content_id,
            text="\n".join(paragraphs),
            headings=[
                ReferenceHeading(
                    offset=self.source_offset(mark.position, offsets),
                    title=mark.title,
                    level=mark.level,
                )
                for mark in self.headings
                if mark.title
            ],
            pages=[
                ReferencePage(
                    offset=self.source_offset(mark.position, offsets),
                    label=clean_page_label(mark.label),
                )
                for mark in self.pages
            ],
            anchors={
                section: self.source_offset(position, offsets)
                for section, position in self.anchors.items()
            },
        )


def parse_references(markup: str, content_id: str) -> DocumentReferences:
    parser = ReferenceParser()
    parser.feed(markup)
    parser.close()
    return parser.document(content_id)


def children(element: ET.Element, name: str) -> List[ET.Element]:
    return [child for child in element if child.tag.rsplit("}", 1)[-1] == name]


def section_target(base: str, href: str, headings: List[str]) -> SectionTarget:
    target = urldefrag(urljoin(base, href))
    return SectionTarget(
        content_id=unquote(target.url), section=unquote(target.fragment) or None, headings=headings
    )


def page_target(base: str, href: str, label: str) -> PageTarget:
    target = urldefrag(urljoin(base, href))
    return PageTarget(
        content_id=unquote(target.url),
        section=unquote(target.fragment) or None,
        label=clean_page_label(label) or label,
    )


def nav_sections(ordered_list: ET.Element, base: str, parents: List[str]) -> List[SectionTarget]:
    result = []
    for item in children(ordered_list, "li"):
        labels = children(item, "a") or children(item, "span")
        title = " ".join("".join(labels[0].itertext()).split()) if labels else ""
        headings = parents + ([title] if title else [])
        if labels and labels[0].get("href"):
            result.append(section_target(base, labels[0].get("href", ""), headings))
        for nested in children(item, "ol"):
            result.extend(nav_sections(nested, base, headings))
    return result


def ncx_sections(parent: ET.Element, base: str, parents: List[str]) -> List[SectionTarget]:
    result = []
    for point in children(parent, "navPoint"):
        labels, targets = children(point, "navLabel"), children(point, "content")
        title = " ".join("".join(labels[0].itertext()).split()) if labels else ""
        headings = parents + ([title] if title else [])
        if targets and targets[0].get("src"):
            result.append(section_target(base, targets[0].get("src", ""), headings))
        result.extend(ncx_sections(point, base, headings))
    return result


def parse_navigation(markup: str, content_id: str) -> BookNavigation:
    root = ET.fromstring(markup)
    sections = []
    pages = []
    for element in root.iter():
        name = element.tag.rsplit("}", 1)[-1]
        if name == "navMap":
            sections.extend(ncx_sections(element, content_id, []))
        elif name == "pageTarget":
            labels, targets = children(element, "navLabel"), children(element, "content")
            label = (
                " ".join("".join(labels[0].itertext()).split())
                if labels
                else element.get("value", "")
            )
            if targets and label:
                pages.append(page_target(content_id, targets[0].get("src", ""), label))
        elif name == "nav":
            kind = element.get("{http://www.idpf.org/2007/ops}type", "").split()
            if "toc" in kind:
                for ordered_list in children(element, "ol"):
                    sections.extend(nav_sections(ordered_list, content_id, []))
            elif "page-list" in kind:
                for link in element.iter():
                    if link.tag.rsplit("}", 1)[-1] == "a" and link.get("href"):
                        label = link.get("title") or " ".join("".join(link.itertext()).split())
                        if label:
                            pages.append(page_target(content_id, link.get("href", ""), label))
    return BookNavigation(sections=sections, pages=pages)


def clean_page_label(label: Optional[str]) -> Optional[str]:
    if not label:
        return None
    label = " ".join(label.split())
    if label.lower().startswith("page "):
        label = label[5:].strip()
    return label or None


def reference_headings(
    anchor: TextAnchor,
    document: DocumentReferences,
    navigation: BookNavigation,
    fallback: Sequence[str],
) -> List[str]:
    candidates = []
    for target in navigation.sections:
        if target.content_id != document.content_id:
            continue
        offset = document.anchors.get(target.section) if target.section else 0
        if offset is not None and offset <= anchor.start:
            candidates.append((offset, target))
    latest = max(candidates, key=lambda candidate: candidate[0]) if candidates else None
    stack = []
    boundary = 0
    if latest is not None:
        boundary, target = latest
        stack = [
            ReferenceHeading(offset=boundary, title=title, level=index + 1)
            for index, title in enumerate(target.headings)
        ]
    for heading in document.headings:
        if not boundary <= heading.offset <= anchor.start:
            continue
        if stack and heading.title == stack[-1].title:
            stack[-1] = heading
            continue
        while stack and stack[-1].level >= heading.level:
            stack.pop()
        stack.append(heading)
    return [heading.title for heading in stack] or list(fallback)


def reference_pages(
    document: DocumentReferences, navigation: BookNavigation
) -> List[ReferencePage]:
    pages = {page.offset: page for page in document.pages}
    for target in navigation.pages:
        if target.content_id != document.content_id:
            continue
        offset = document.anchors.get(target.section) if target.section else 0
        if offset is not None and (offset not in pages or pages[offset].label is None):
            pages[offset] = ReferencePage(offset=offset, label=target.label)
    return sorted(pages.values(), key=lambda page: page.offset)


def page_at(pages: Sequence[ReferencePage], offset: int) -> Optional[str]:
    previous = [page for page in pages if page.offset <= offset]
    return previous[-1].label if previous else None


def resolve_reference(
    anchor: TextAnchor,
    document: DocumentReferences,
    content_order: int = 0,
    *,
    navigation: Optional[BookNavigation] = None,
    fallback_headings: Sequence[str] = (),
) -> AnnotationReference:
    source = SourceDocument(content_id=document.content_id, text=document.text)
    if resolve_anchor(anchor, source) is None:
        raise ValueError("The saved quotation does not match this document.")
    navigation = navigation or BookNavigation()
    headings = reference_headings(
        anchor, document, navigation, fallback_headings or ["Chapter {}".format(content_order + 1)]
    )
    pages = reference_pages(document, navigation)
    first = page_at(pages, anchor.start)
    last = page_at(pages, anchor.end - 1)
    return AnnotationReference(
        headings=headings,
        location=stable_location(anchor, content_order),
        page_start=first,
        page_end=last if first and last != first else None,
    )


class BookReferences:
    ebook: Epub
    navigation: BookNavigation
    documents: Dict[str, DocumentReferences]
    content_order: Dict[str, int]

    def __init__(self, ebook: Epub) -> None:
        self.ebook = ebook
        self.documents = {}
        self.content_order = {str(content): index for index, content in enumerate(ebook.contents)}
        self.navigation = self.load_navigation()

    def load_navigation(self) -> BookNavigation:
        try:
            package = ET.fromstring(self.ebook.get_raw_text(self.ebook.root_filepath))
            items = [
                element for element in package.iter() if element.tag.rsplit("}", 1)[-1] == "item"
            ]
            items.sort(key=lambda item: "nav" not in item.get("properties", "").split())
            for element in items:
                if (
                    "nav" in element.get("properties", "").split()
                    or element.get("media-type") == "application/x-dtbncx+xml"
                ):
                    content_id = unquote(urljoin(self.ebook.root_filepath, element.get("href", "")))
                    return parse_navigation(self.ebook.get_raw_text(content_id), content_id)
        except (ET.ParseError, OSError, KeyError, ValueError, zipfile.BadZipFile):
            pass
        return BookNavigation(
            sections=[
                SectionTarget(
                    content_id=str(self.ebook.contents[entry.content_index]),
                    section=entry.section,
                    headings=[entry.label],
                )
                for entry in self.ebook.toc_entries
            ]
        )

    def document(self, content_id: str) -> DocumentReferences:
        if content_id not in self.documents:
            self.documents[content_id] = parse_references(
                self.ebook.get_raw_text(content_id), content_id
            )
        return self.documents[content_id]

    def prime(self, documents: Sequence[SourceDocument]) -> None:
        for document in documents:
            try:
                self.document(document.content_id)
            except (OSError, KeyError, ValueError, zipfile.BadZipFile):
                continue

    def for_anchor(self, anchor: TextAnchor, content_order: int) -> AnnotationReference:
        earlier = [
            target
            for target in self.navigation.sections
            if self.content_order.get(target.content_id, content_order) < content_order
        ]
        fallback = earlier[-1].headings if earlier else []
        try:
            return resolve_reference(
                anchor,
                self.document(anchor.content_id),
                content_order,
                navigation=self.navigation,
                fallback_headings=fallback,
            )
        except (OSError, KeyError, ValueError, zipfile.BadZipFile):
            return AnnotationReference(
                headings=fallback or ["Chapter {}".format(content_order + 1)],
                location=stable_location(anchor, content_order),
            )

    def enrich(self, annotation: Annotation) -> Annotation:
        if (
            annotation.reference is not None
            or annotation.anchor.content_id not in self.content_order
        ):
            return annotation
        order = self.content_order[annotation.anchor.content_id]
        try:
            document = self.document(annotation.anchor.content_id)
            if (
                resolve_anchor(
                    annotation.anchor,
                    SourceDocument(content_id=document.content_id, text=document.text),
                )
                is None
            ):
                return annotation
        except (OSError, KeyError, ValueError, zipfile.BadZipFile):
            return annotation
        reference = self.for_anchor(annotation.anchor, order)
        return annotation.model_copy(
            update=dict(
                reference=reference, chapter=" / ".join(reference.headings), content_order=order
            )
        )
