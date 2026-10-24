from dataclasses import dataclass
from typing import List

import pytest

from epy_reader.parser import parse_html


@dataclass(frozen=True)
class ViewportCase:
    height: int
    top_row: int
    expected_row: int


@pytest.mark.parametrize(
    "case",
    [
        ViewportCase(height=5, top_row=0, expected_row=2),
        ViewportCase(height=3, top_row=0, expected_row=0),
        ViewportCase(height=5, top_row=2, expected_row=4),
        ViewportCase(height=20, top_row=6, expected_row=6),
    ],
)
def test_selection_starts_nearest_visible_midpoint(case: ViewportCase) -> None:
    from epy_reader.annotations.view import Selection

    structure = parse_html(
        "<p>first</p><p>second</p><p>third</p><p>fourth</p>", textwidth=80, content_id="chapter"
    )
    selection = Selection.from_viewport(structure, case.top_row, case.height)
    assert selection.cell.row == case.expected_row
    assert selection.cell.col == 0


def test_selection_rejects_viewport_without_text() -> None:
    from epy_reader.annotations.view import Selection

    structure = parse_html("<p>first</p>", textwidth=80, content_id="chapter")
    with pytest.raises(ValueError, match="visible"):
        Selection.from_viewport(structure, 1, 10)


def test_reverse_selection_produces_source_quote() -> None:
    from epy_reader.annotations.view import Selection

    structure = parse_html("<p>first second</p>", textwidth=6, content_id="chapter")
    selection = Selection(structure, 0)
    for _ in range(4):
        selection.move("right")
    selection.mark = selection.cursor
    for _ in range(4):
        selection.move("left")
    assert selection.anchor().quote == "first"


@dataclass(frozen=True)
class ScrolloffCase:
    cursor_row: int
    top_row: int
    scrolloff: int
    expected_top: int
    height: int = 20


@pytest.mark.parametrize(
    "case",
    [
        ScrolloffCase(cursor_row=15, top_row=10, scrolloff=5, expected_top=10),
        ScrolloffCase(cursor_row=14, top_row=10, scrolloff=5, expected_top=9),
        ScrolloffCase(cursor_row=25, top_row=10, scrolloff=5, expected_top=11),
        ScrolloffCase(cursor_row=29, top_row=10, scrolloff=0, expected_top=10),
        ScrolloffCase(cursor_row=1, top_row=10, scrolloff=5, expected_top=0),
        ScrolloffCase(cursor_row=99, top_row=60, scrolloff=5, expected_top=83),
        ScrolloffCase(cursor_row=50, top_row=0, scrolloff=999, expected_top=41),
        ScrolloffCase(cursor_row=50, top_row=0, scrolloff=999, expected_top=50, height=1),
    ],
)
def test_selection_viewport_honors_scrolloff(case: ScrolloffCase) -> None:
    from epy_reader.annotations.view import Selection

    structure = parse_html(
        "<pre>" + "\n".join(str(row) for row in range(100)) + "</pre>", textwidth=80
    )
    selection = Selection(structure, case.cursor_row)
    assert (
        selection.viewport(case.top_row, height=case.height, scrolloff=case.scrolloff)
        == case.expected_top
    )


@pytest.mark.parametrize("distance,expected_row", [(19, 19), (-19, 0), (999, 99)])
def test_selection_row_movement_lands_on_the_nearest_display_row(
    distance: int, expected_row: int
) -> None:
    from epy_reader.annotations.view import Selection

    structure = parse_html(
        "<pre>" + "\n".join(str(row) for row in range(100)) + "</pre>", textwidth=80
    )
    selection = Selection(structure, 0)
    selection.move_rows(distance)
    assert selection.cell.row == expected_row


def test_selection_row_movement_skips_reserved_image_space() -> None:
    from epy_reader.annotations.view import Selection
    from epy_reader.images.layout import ImageLayout

    source = parse_html('<p>before</p><img src="figure.png"/><p>after</p>', textwidth=40)
    layout = ImageLayout.expand(source, image_rows=20)
    selection = Selection(layout.structure, 0)
    selection.move_rows(19)
    expected = next(cell for cell in layout.structure.source_cells if cell.text == "a")
    assert selection.cell == expected


def test_selection_row_movement_stays_inside_its_content_document() -> None:
    from epy_reader.annotations.view import Selection
    from epy_reader.utils import merge_text_structures

    first = parse_html("<pre>one\ntwo\nthree</pre>", textwidth=40, content_id="first")
    second = parse_html(
        "<pre>four\nfive\nsix</pre>",
        textwidth=40,
        content_id="second",
        starting_line=len(first.text_lines),
    )
    selection = Selection(merge_text_structures(first, second), len(first.text_lines))
    selection.move_rows(999)
    assert selection.cell.row == len(first.text_lines) + 2
    selection.move_rows(-999)
    assert selection.cell == second.source_cells[0]


@dataclass(frozen=True)
class WordMotionCase:
    text: str
    start_offset: int
    direction: str
    expected_offset: int
    width: int = 80


@pytest.mark.parametrize(
    "case",
    [
        WordMotionCase("one two three", 0, "word_forward", 4),
        WordMotionCase("one two three", 1, "word_forward", 4),
        WordMotionCase("one two three", 3, "word_forward", 4),
        WordMotionCase("one two three", 4, "word_backward", 0),
        WordMotionCase("one two three", 6, "word_backward", 4),
        WordMotionCase("one two three", 8, "word_forward", 12),
        WordMotionCase("one two three", 12, "word_backward", 8),
        WordMotionCase("one,two... three", 0, "word_forward", 3),
        WordMotionCase("one,two... three", 3, "word_forward", 4),
        WordMotionCase("one,two... three", 4, "word_forward", 7),
        WordMotionCase("one,two... three", 7, "word_forward", 11),
        WordMotionCase("one,two... three", 11, "word_backward", 7),
        WordMotionCase("one,two... three", 4, "word_backward", 3),
        WordMotionCase("one,two... three", 9, "word_backward", 7),
        WordMotionCase("first_word next", 0, "word_forward", 11),
        WordMotionCase("cafe\u0301 next", 0, "word_forward", 6),
        WordMotionCase("cafe\u0301 next", 6, "word_backward", 0),
        WordMotionCase("one two", 0, "word_forward", 4, width=3),
        WordMotionCase("alphabet soup", 3, "word_forward", 9, width=3),
        WordMotionCase("alphabet soup", 6, "word_backward", 0, width=3),
    ],
)
def test_word_motion_lands_at_source_boundary(case: WordMotionCase) -> None:
    from epy_reader.annotations.view import Selection

    structure = parse_html("<p>" + case.text + "</p>", textwidth=case.width, content_id="chapter")
    selection = Selection(structure, 0)
    selection.cursor = next(
        index for index, cell in enumerate(selection.cells) if cell.start == case.start_offset
    )
    selection.move(case.direction)
    assert selection.cell.start == case.expected_offset


def test_word_motion_stops_at_document_end() -> None:
    from epy_reader.annotations.view import Selection
    from epy_reader.utils import merge_text_structures

    structure = merge_text_structures(
        parse_html("<p>one</p>", textwidth=10, content_id="one"),
        parse_html("<p>two</p>", textwidth=10, content_id="two", starting_line=3),
    )
    selection = Selection(structure, 0)
    selection.move("word_forward")
    assert (selection.cell.content_id, selection.cell.start) == ("one", 2)


def test_backward_word_motion_stays_in_current_document() -> None:
    from epy_reader.annotations.view import Selection
    from epy_reader.utils import merge_text_structures

    structure = merge_text_structures(
        parse_html("<p>one</p>", textwidth=10, content_id="one"),
        parse_html("<p>two</p>", textwidth=10, content_id="two", starting_line=3),
    )
    selection = Selection(structure, 3)
    selection.move("right")
    selection.move("word_backward")
    assert (selection.cell.content_id, selection.cell.start) == ("two", 0)


def test_selection_does_not_cross_documents() -> None:
    from epy_reader.annotations.view import Selection
    from epy_reader.utils import merge_text_structures

    structure = merge_text_structures(
        parse_html("<p>one</p>", textwidth=10, content_id="one"),
        parse_html("<p>two</p>", textwidth=10, content_id="two", starting_line=3),
    )
    selection = Selection(structure, 0)
    for _ in range(10):
        selection.move("right")
    assert selection.cell.content_id == "one"


def test_dense_margin_notes_do_not_overlap() -> None:
    from epy_reader.annotations.view import MarginInput, place_notes

    placed = place_notes(
        [
            MarginInput(annotation_id="one", source_row=3, text="a long note here"),
            MarginInput(annotation_id="two", source_row=3, text="next"),
        ],
        width=6,
    )
    assert placed[1].row >= placed[0].row + len(placed[0].lines) + 1


@dataclass(frozen=True)
class EditorLayoutCase:
    text: str
    width: int
    expected_lines: List[str]
    cursor_row: int
    cursor_col: int


@pytest.mark.parametrize(
    "case",
    [
        EditorLayoutCase("alpha ", 20, ["alpha "], 0, 6),
        EditorLayoutCase("alpha  ", 20, ["alpha  "], 0, 7),
        EditorLayoutCase("abc  ", 5, ["abc  ", ""], 1, 0),
        EditorLayoutCase("a\n  ", 20, ["a", "  "], 1, 2),
        EditorLayoutCase("界e\u0301 ", 20, ["界e\u0301 "], 0, 4),
    ],
)
def test_editor_layout_preserves_whitespace_cursor(case: EditorLayoutCase) -> None:
    from epy_reader.annotations.view import EditorState

    state = EditorState(text=case.text, cursor=len(case.text))
    layout = state.layout(case.width)
    assert layout.lines == case.expected_lines
    assert (layout.cursor.row, layout.cursor.col) == (case.cursor_row, case.cursor_col)


@pytest.mark.parametrize("text", ["café e\u0301", "family 👩‍👩‍👧‍👦", "flag 🇷🇴"])
def test_editor_cached_backspace_preserves_graphemes(text: str) -> None:
    from epy_reader.annotations.view import EditorState

    state = EditorState(text=text, cursor=len(text))
    state.layout(70)
    state.insert(" ")
    state.handle("backspace")
    assert state.text == text
    state.handle("backspace")
    assert state.text == text[: text.rfind(" ") + 1]


def test_editor_paste_cannot_activate_save() -> None:
    from epy_reader.annotations.view import EditorState

    state = EditorState()
    state.insert("O\n\tSave\x1b")
    assert state.text == "O\n    Save"
    assert state.focus == 0


@pytest.mark.parametrize("tab_count, expected_action", [(2, "delete"), (3, "cancel")])
def test_existing_note_editor_exposes_separate_delete(tab_count: int, expected_action: str) -> None:
    from epy_reader.annotations.view import EditorState

    state = EditorState(text="existing", cursor=8, can_delete=True)
    for _ in range(tab_count):
        state.handle("tab")
    assert state.handle("enter") == expected_action


def test_editor_cancel_leaves_original_text() -> None:
    from epy_reader.annotations.view import EditorState

    state = EditorState(text="old", cursor=3)
    state.insert(" draft")
    assert state.handle("escape") == "cancel"


def test_editor_vertical_move_keeps_grapheme_boundary() -> None:
    from epy_reader.annotations.view import EditorState

    state = EditorState(text="ab\n界e\u0301", cursor=2)
    state.handle("down")
    assert state.cursor == 4


def test_narrow_pane_uses_overlay() -> None:
    from epy_reader.annotations.view import reading_layout

    assert reading_layout(80, 70, 36).notes_x is None


def test_wide_pane_preserves_reading_center() -> None:
    from epy_reader.annotations.view import reading_layout

    layout = reading_layout(240, 80, 36)
    assert layout.reading_x == 81
    assert layout.notes_x >= layout.reading_x + 80 + 4
