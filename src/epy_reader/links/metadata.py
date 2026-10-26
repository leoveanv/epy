"""Collect HTML link semantics without changing legacy canonical text."""

from dataclasses import dataclass
from typing import Dict, List, Optional, Sequence, Tuple, cast

from typing_extensions import TypeAlias

from epy_reader.annotations.text import normalize_paragraph
from epy_reader.links.models import CitationRange, DocumentAnchor, TextLink
from epy_reader.models import CharPos, TextMark

AttributeValue: TypeAlias = Optional[str]


@dataclass
class LinkMark:
    start: CharPos
    href: str
    citation: bool
    end: Optional[CharPos] = None


@dataclass(frozen=True)
class AnchorMark:
    position: CharPos
    fragment: str


def canonical_offset(position: CharPos, paragraphs: Sequence[str]) -> int:
    return sum(len(normalize_paragraph(line)) + 1 for line in paragraphs[: position.row]) + len(
        normalize_paragraph(paragraphs[position.row][: position.col])
    )


@dataclass(frozen=True)
class CitationContext:
    tag: str
    depth: int


class LinkMetadata:
    links: List[LinkMark]
    active_links: List[LinkMark]
    anchors: List[AnchorMark]
    superscripts: List[TextMark]
    citation_contexts: List[CitationContext]
    tag_depths: Dict[str, int]

    def __init__(self) -> None:
        self.links = []
        self.active_links = []
        self.anchors = []
        self.superscripts = []
        self.citation_contexts = []
        self.tag_depths = {}

    def start(
        self, tag: str, attrs: Sequence[Tuple[str, AttributeValue]], position: CharPos
    ) -> None:
        self.tag_depths[tag] = self.tag_depths.get(tag, 0) + 1
        attributes = dict(attrs)
        semantics = (attributes.get("epub:type") or "").split()
        roles = (attributes.get("role") or "").split()
        if "noteref" in semantics or "doc-noteref" in roles or attributes.get("type") == "note":
            self.citation_contexts.append(CitationContext(tag, self.tag_depths[tag]))
        fragment = attributes.get("id") or attributes.get("xml:id") or attributes.get("name")
        if fragment:
            self.anchors.append(AnchorMark(position, fragment))
        if tag == "sup":
            self.superscripts.append(TextMark(start=position))
        if tag != "a":
            return
        href = attributes.get("href") or attributes.get("xlink:href")
        if href is not None:
            mark = LinkMark(position, href, bool(self.citation_contexts))
            self.links.append(mark)
            self.active_links.append(mark)

    def end(self, tag: str, position: CharPos) -> None:
        depth = self.tag_depths.get(tag, 0)
        self.citation_contexts = [
            context
            for context in self.citation_contexts
            if context.tag != tag or context.depth != depth
        ]
        self.tag_depths[tag] = max(0, depth - 1)
        if tag == "a" and self.active_links:
            self.active_links.pop().end = position
        if tag == "sup" and self.superscripts and self.superscripts[-1].end is None:
            self.superscripts[-1] = TextMark(start=self.superscripts[-1].start, end=position)

    def text_links(self, paragraphs: Sequence[str], content_id: str) -> Tuple[TextLink, ...]:
        text = "\n".join(normalize_paragraph(line) for line in paragraphs)
        result = []
        for mark in self.links:
            if mark.end is None:
                continue
            start, end = (
                canonical_offset(mark.start, paragraphs),
                canonical_offset(mark.end, paragraphs),
            )
            if end > start:
                result.append(
                    TextLink(
                        content_id=content_id,
                        start=start,
                        end=end,
                        href=mark.href,
                        label=text[start:end],
                    )
                )
        return tuple(result)

    def destinations(
        self, paragraphs: Sequence[str], content_id: str
    ) -> Tuple[DocumentAnchor, ...]:
        return tuple(
            DocumentAnchor(
                content_id=content_id,
                fragment=mark.fragment,
                offset=canonical_offset(mark.position, paragraphs),
            )
            for mark in self.anchors
        )

    def citations(self, paragraphs: Sequence[str], content_id: str) -> List[CitationRange]:
        links = self.text_links(paragraphs, content_id)
        candidates = [
            (canonical_offset(mark.start, paragraphs), canonical_offset(mark.end, paragraphs))
            for mark in self.links
            if mark.citation and mark.end is not None
        ]
        for mark in self.superscripts:
            if mark.end is None:
                continue
            start, end = (
                canonical_offset(mark.start, paragraphs),
                canonical_offset(mark.end, paragraphs),
            )
            if any(link.start < end and link.end > start for link in links):
                candidates.append((start, end))
        text = "\n".join(normalize_paragraph(line) for line in paragraphs)
        result = cast(List[CitationRange], [])
        for start, end in sorted(set(candidates)):
            if any(start < item.end and end > item.start for item in result):
                continue
            label = text[start:end].strip()
            if label.startswith("^{") and label.endswith("}"):
                label = label[2:-1]
            label = label.strip("[]")
            if label and "\n" not in label:
                result.append(CitationRange(start=start, end=end, label="[" + label + "]"))
        return result
