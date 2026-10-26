"""Display citations without rewriting the canonical annotation document."""

from dataclasses import dataclass, replace
from typing import List, Sequence, cast

from epy_reader.annotations.text import SourceCell
from epy_reader.links.models import CitationRange


@dataclass(frozen=True)
class SourceRange:
    start: int
    end: int


@dataclass(frozen=True)
class DisplayParagraph:
    text: str
    ranges: List[SourceRange]

    def source_cells(self, cells: Sequence[SourceCell], offset: int) -> List[SourceCell]:
        return [
            replace(
                cell,
                start=self.ranges[cell.start - offset].start,
                end=self.ranges[cell.end - offset - 1].end,
            )
            for cell in cells
        ]

    def display_offset(self, source_offset: int) -> int:
        return next(
            (index for index, span in enumerate(self.ranges) if span.end > source_offset),
            len(self.text),
        )


def project_paragraph(
    text: str, offset: int, citations: Sequence[CitationRange]
) -> DisplayParagraph:
    ranges = cast(List[SourceRange], [])
    displayed = []
    cursor = 0
    for citation in citations:
        start, end = citation.start - offset, citation.end - offset
        if start < cursor or end > len(text) or start < 0:
            continue
        displayed.append(text[cursor:start])
        ranges.extend(SourceRange(offset + i, offset + i + 1) for i in range(cursor, start))
        displayed.append(citation.label)
        ranges.extend(SourceRange(citation.start, citation.end) for _ in citation.label)
        cursor = end
    displayed.append(text[cursor:])
    ranges.extend(SourceRange(offset + i, offset + i + 1) for i in range(cursor, len(text)))
    return DisplayParagraph("".join(displayed), ranges)
