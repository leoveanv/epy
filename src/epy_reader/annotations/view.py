"""Pure selection, editing, and margin layout, independent of curses."""

from bisect import bisect_left, bisect_right
from dataclasses import dataclass, field
from enum import Enum
from typing import List, Optional, Sequence, Tuple

from epy_reader.annotations.models import TextAnchor
from epy_reader.annotations.text import (
    SourceCell,
    cell_width,
    grapheme_ends,
    graphemes,
    wrap_text,
)
from epy_reader.models import CharPos, TextStructure


def starts_word(previous: Optional[SourceCell], current: SourceCell) -> bool:
    if current.text.isspace():
        return False
    if (
        previous is None
        or previous.content_id != current.content_id
        or previous.end != current.start
    ):
        return True
    previous_keyword = previous.text[0].isalnum() or previous.text[0] == "_"
    current_keyword = current.text[0].isalnum() or current.text[0] == "_"
    return previous.text.isspace() or previous_keyword != current_keyword


class Selection:
    structure: TextStructure
    cells: Tuple[SourceCell, ...]
    cell_rows: Tuple[int, ...]
    cursor: int
    mark: Optional[int]
    word_starts: Tuple[int, ...]
    first_cell: int
    last_cell: int

    def __init__(self, structure: TextStructure, row: int) -> None:
        self.structure = structure
        self.cells = structure.source_cells
        self.cell_rows = tuple(cell.row for cell in self.cells)
        if not self.cells:
            raise ValueError("This document has no selectable text.")
        self.cursor = next((i for i, cell in enumerate(self.cells) if cell.row >= row), 0)
        self.mark = None
        content_id = self.cell.content_id
        document_cells = tuple(
            index for index, cell in enumerate(self.cells) if cell.content_id == content_id
        )
        self.first_cell, self.last_cell = document_cells[0], document_cells[-1]
        # NOTE: Vim word motions separate keyword characters from punctuation; source gaps retain whitespace removed by wrapping.
        self.word_starts = tuple(
            index
            for index in document_cells
            if starts_word(self.cells[index - 1] if index else None, self.cells[index])
        )

    @classmethod
    def from_viewport(cls, structure: TextStructure, top_row: int, height: int) -> "Selection":
        visible_cells = [
            cell for cell in structure.source_cells if top_row <= cell.row < top_row + height
        ]
        if not visible_cells:
            raise ValueError("This window has no visible selectable text.")
        midpoint = top_row + height // 2
        nearest = min(visible_cells, key=lambda cell: abs(cell.row - midpoint))
        return cls(structure, nearest.row)

    @property
    def cell(self) -> SourceCell:
        return self.cells[self.cursor]

    def move(self, direction: str) -> None:
        if direction in ("word_forward", "word_backward"):
            candidate = self._word_cursor(direction)
        elif direction in ("left", "right"):
            candidate = max(
                0, min(len(self.cells) - 1, self.cursor + (1 if direction == "right" else -1))
            )
        else:
            rows = sorted(
                {cell.row for cell in self.cells if cell.content_id == self.cell.content_id}
            )
            row_index = rows.index(self.cell.row) + (1 if direction == "down" else -1)
            if not 0 <= row_index < len(rows):
                return
            candidates = [i for i, cell in enumerate(self.cells) if cell.row == rows[row_index]]
            candidate = min(candidates, key=lambda i: abs(self.cells[i].col - self.cell.col))
        if self.cells[candidate].content_id == self.cell.content_id:
            self.cursor = candidate

    def move_rows(self, distance: int) -> None:
        target = self.cell.row + distance
        index = min(
            self.last_cell,
            bisect_left(self.cell_rows, target, self.first_cell, self.last_cell + 1),
        )
        if index > self.first_cell and abs(self.cell_rows[index - 1] - target) < abs(
            self.cell_rows[index] - target
        ):
            index -= 1
        row = self.cell_rows[index]
        start = bisect_left(self.cell_rows, row, self.first_cell, self.last_cell + 1)
        end = bisect_right(self.cell_rows, row, start, self.last_cell + 1)
        self.cursor = min(range(start, end), key=lambda i: abs(self.cells[i].col - self.cell.col))

    # NOTE: Neovim scrolloff keeps context around the cursor, except at the document edges.
    def viewport(self, top_row: int, *, height: int, scrolloff: int) -> int:
        height = max(1, height)
        margin = min(max(0, scrolloff), height // 2)
        top = min(top_row, self.cell.row - margin)
        top = max(top, self.cell.row - height + margin + 1)
        return max(0, min(top, max(0, len(self.structure.text_lines) - height)))

    def _word_cursor(self, direction: str) -> int:
        position = (
            bisect_right(self.word_starts, self.cursor)
            if direction == "word_forward"
            else bisect_left(self.word_starts, self.cursor) - 1
        )
        if position < 0:
            return self.first_cell
        if position >= len(self.word_starts):
            return self.last_cell
        return self.word_starts[position]

    def selected_cells(self) -> Sequence[SourceCell]:
        if self.mark is None:
            return (self.cell,)
        start, end = sorted((self.cursor, self.mark))
        return self.cells[start : end + 1]

    def anchor(self) -> TextAnchor:
        selected = self.selected_cells()
        start, end = selected[0].start, selected[-1].end
        document = next(
            doc for doc in self.structure.documents if doc.content_id == self.cell.content_id
        )
        return TextAnchor(
            content_id=document.content_id,
            start=start,
            end=end,
            quote=document.text[start:end],
            prefix=document.text[max(0, start - 32) : start],
            suffix=document.text[end : end + 32],
        )


@dataclass(frozen=True)
class MarginInput:
    annotation_id: str
    source_row: int
    text: str


@dataclass(frozen=True)
class PlacedNote:
    annotation_id: str
    source_row: int
    row: int
    lines: List[str]


@dataclass(frozen=True)
class ReadingLayout:
    reading_x: int
    notes_x: Optional[int]
    notes_width: int


def reading_layout(columns: int, textwidth: int, notes_width: int) -> ReadingLayout:
    centered = (columns - textwidth) // 2 + 1
    if columns < textwidth + notes_width + 10:
        return ReadingLayout(centered, None, notes_width)
    reading_x = min(centered, columns - textwidth - notes_width - 8)
    return ReadingLayout(reading_x, reading_x + textwidth + 6, notes_width)


def place_notes(notes: Sequence[MarginInput], width: int) -> List[PlacedNote]:
    placed = []
    bottom = 0
    for note in notes:
        lines = []
        for paragraph in note.text.split("\n"):
            lines.extend(wrap_text(paragraph, width) or [""])
        row = max(bottom, note.source_row)
        placed.append(PlacedNote(note.annotation_id, note.source_row, row, lines))
        bottom = row + len(lines) + 1
    return placed


class EditorAction(str, Enum):
    SAVE = "save"
    DELETE = "delete"
    CANCEL = "cancel"


@dataclass(frozen=True)
class EditorLayout:
    lines: List[str]
    cursor: CharPos


@dataclass
class EditorState:
    text: str = ""
    cursor: int = 0
    focus: int = 0
    multiline: bool = True
    can_delete: bool = False
    _boundaries: List[int] = field(default_factory=list, init=False, repr=False)

    def __post_init__(self) -> None:
        self._boundaries = [0]
        self._boundaries.extend(grapheme_ends(self.text))

    @property
    def buttons(self) -> Tuple[EditorAction, ...]:
        if self.can_delete:
            return (EditorAction.SAVE, EditorAction.DELETE, EditorAction.CANCEL)
        return (EditorAction.SAVE, EditorAction.CANCEL)

    def insert(self, value: str) -> None:
        value = value.replace("\t", "    ")
        value = "".join(char for char in value if char == "\n" or char.isprintable())
        if not self.multiline:
            value = value.replace("\n", " ")
        if value:
            self._replace_range(self.cursor, self.cursor, value)

    def handle(self, key: str) -> Optional[EditorAction]:
        if key == "escape":
            return EditorAction.CANCEL
        if key == "tab":
            self.focus = (self.focus + 1) % (len(self.buttons) + 1)
        elif key == "enter":
            if self.focus:
                return self.buttons[self.focus - 1]
            if self.multiline:
                self.insert("\n")
            else:
                self.focus = 1
        elif self.focus:
            return None
        elif key == "left":
            self.cursor = self._previous_boundary()
        elif key == "right":
            self.cursor = self._next_boundary()
        elif key == "home":
            self.cursor = self.text.rfind("\n", 0, self.cursor) + 1
        elif key == "end":
            end = self.text.find("\n", self.cursor)
            self.cursor = len(self.text) if end < 0 else end
        elif key == "backspace" and self.cursor:
            self._replace_range(self._previous_boundary(), self.cursor, "")
        elif key == "delete" and self.cursor < len(self.text):
            self._replace_range(self.cursor, self._next_boundary(), "")
        elif key in ("up", "down"):
            self.move_vertical(key)
        return None

    def _previous_boundary(self) -> int:
        return self._boundaries[max(0, bisect_left(self._boundaries, self.cursor) - 1)]

    def _next_boundary(self) -> int:
        return self._boundaries[
            min(len(self._boundaries) - 1, bisect_right(self._boundaries, self.cursor))
        ]

    def _replace_range(self, start: int, end: int, value: str) -> None:
        anchor_index = max(0, bisect_left(self._boundaries, start) - 1)
        anchor = self._boundaries[anchor_index]
        self.text = self.text[:start] + value + self.text[end:]
        self.cursor = start + len(value)
        del self._boundaries[anchor_index + 1 :]
        self._boundaries.extend(anchor + end for end in grapheme_ends(self.text[anchor:]))

    def layout(self, width: int) -> EditorLayout:
        width = max(1, width)
        lines = [""]
        row, col = 0, 0
        cursor = CharPos(row=0, col=0)
        for index in range(len(self._boundaries) - 1):
            start, end = self._boundaries[index], self._boundaries[index + 1]
            cluster = self.text[start:end]
            size = cell_width(cluster)
            if cluster not in ("\n", "\r\n") and col and col + size > width:
                lines.append("")
                row, col = row + 1, 0
            if start == self.cursor:
                cursor = CharPos(row=row, col=col)
            if cluster in ("\n", "\r\n"):
                lines.append("")
                row, col = row + 1, 0
            else:
                lines[-1] += cluster
                col += size
        if self.cursor == len(self.text):
            if col >= width:
                lines.append("")
                row, col = row + 1, 0
            cursor = CharPos(row=row, col=col)
        return EditorLayout(lines, cursor)

    def move_vertical(self, direction: str) -> None:
        start = self.text.rfind("\n", 0, self.cursor) + 1
        column = cell_width(self.text[start : self.cursor])
        if direction == "up" and start:
            target_start = self.text.rfind("\n", 0, start - 1) + 1
            target_end = start - 1
        elif direction == "down" and self.text.find("\n", self.cursor) >= 0:
            target_start = self.text.find("\n", self.cursor) + 1
            end = self.text.find("\n", target_start)
            target_end = len(self.text) if end < 0 else end
        else:
            return
        self.cursor = target_start
        width = 0
        for cluster in graphemes(self.text[target_start:target_end]):
            width += cell_width(cluster)
            if width > column:
                break
            self.cursor += len(cluster)
