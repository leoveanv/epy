"""Reserve stable image slots without changing canonical book text or stored row positions."""

from bisect import bisect_right
from dataclasses import dataclass, replace
from typing import List, Tuple, cast

from epy_reader.models import TextStructure


@dataclass(frozen=True)
class ImageLayout:
    structure: TextStructure
    row_starts: Tuple[int, ...]
    image_rows: int

    @classmethod
    def expand(cls, source: TextStructure, *, image_rows: int) -> "ImageLayout":
        lines = cast(List[str], [])
        starts = []
        for row, line in enumerate(source.text_lines):
            starts.append(len(lines))
            lines.append(line)
            if row in source.image_maps:
                lines.extend([""] * (image_rows - 1))
        starts.append(len(lines))
        return cls(
            structure=replace(
                source,
                text_lines=tuple(lines),
                image_maps={starts[row]: path for row, path in source.image_maps.items()},
                section_rows={name: starts[row] for name, row in source.section_rows.items()},
                formatting=tuple(
                    replace(style, row=starts[style.row]) for style in source.formatting
                ),
                source_cells=tuple(
                    replace(cell, row=starts[cell.row]) for cell in source.source_cells
                ),
            ),
            row_starts=tuple(starts),
            image_rows=image_rows,
        )

    def display_row(self, row: int) -> int:
        if row < 0:
            return row
        return self.row_starts[min(row, len(self.row_starts) - 1)]

    def original_row(self, row: int) -> int:
        if row < 0:
            return row
        return max(0, bisect_right(self.row_starts, row) - 1)
