"""Movement controls through the real curses reader, without graphics dependencies."""

import dataclasses
import json

import pytest
from test_annotation_terminal import TerminalReader, build_epub
from test_annotation_terminal import terminal_reader as terminal_reader

from epy_reader.settings import CfgDefaultKeymaps, Settings


def prepare_navigation_book(
    reader: TerminalReader, *, scrolloff: int = 0, spread: bool = False
) -> None:
    lines = ["First passage"] + [f"Line {index:03d}" for index in range(1, 120)]
    build_epub(reader.book, "<pre>" + "\n".join(lines) + "</pre>")
    settings = Settings(
        PageScrollAnimation=False, ShowProgressIndicator=False, StartWithDoubleSpread=spread
    )
    if scrolloff:
        settings = dataclasses.replace(settings, AnnotationScrolloff=scrolloff)
    (reader.home / ".config/epy/configuration.json").write_text(
        json.dumps(
            dict(
                Setting=dataclasses.asdict(settings), Keymap=dataclasses.asdict(CfgDefaultKeymaps())
            )
        )
    )
    reader.restart()


@pytest.mark.parametrize("spread,half_page_lines", [(False, 20), (True, 40)])
def test_reading_control_d_scrolls_half_the_visible_page(
    terminal_reader: TerminalReader, spread: bool, half_page_lines: int
) -> None:
    reader = terminal_reader
    prepare_navigation_book(reader, spread=spread)
    reader.send("C-d")
    reader.wait_for_absence("First passage")
    assert (
        reader.command("capture-pane", "-pt", "reader")
        .splitlines()[0]
        .strip()
        .startswith(f"Line {half_page_lines:03d}")
    )
    reader.send("C-u")
    reader.wait_for("First passage")
    assert (
        reader.command("capture-pane", "-pt", "reader")
        .splitlines()[0]
        .strip()
        .startswith("First passage")
    )


def test_half_page_distance_updates_after_resize(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    prepare_navigation_book(reader)
    reader.command("resize-window", "-t", "reader", "-x", "180", "-y", "31")
    reader.send("C-d")
    reader.wait_for_absence("First passage")
    assert reader.command("capture-pane", "-pt", "reader").splitlines()[0].strip() == "Line 015"


def test_half_page_controls_have_readable_help_labels(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    reader.send("?")
    reader.wait_for("Ctrl-U")
    reader.wait_for("Ctrl-D")
    assert "Half Screen" in reader.command("capture-pane", "-pt", "reader")


def test_reading_half_page_count_repeats_the_movement(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    prepare_navigation_book(reader)
    reader.send("2", "C-d")
    reader.wait_for_absence("First passage")
    assert reader.command("capture-pane", "-pt", "reader").splitlines()[0].strip() == "Line 040"


def test_reading_control_d_reaches_the_last_full_viewport(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    prepare_navigation_book(reader)
    reader.send("G")
    reader.wait_for("Line 119")
    ending = reader.command("capture-pane", "-pt", "reader")
    reader.send("g", "5", "C-d")
    reader.wait_for_absence("First passage")
    assert reader.command("capture-pane", "-pt", "reader") == ending


def test_selection_half_page_moves_the_cursor_with_the_viewport(
    terminal_reader: TerminalReader,
) -> None:
    reader = terminal_reader
    prepare_navigation_book(reader)
    reader.send("v", "C-d")
    reader.wait_for("Line 057")
    captured = reader.command("capture-pane", "-pt", "reader").splitlines()
    cursor_row = int(reader.command("display-message", "-pt", "reader", "#{cursor_y}").strip())
    assert captured[0].strip() == "Line 019"
    assert captured[cursor_row].strip() == "Line 038"
    assert cursor_row == 19
    reader.send("C-u")
    reader.wait_for("First passage")
    assert int(reader.command("display-message", "-pt", "reader", "#{cursor_y}").strip()) == 19


def test_half_page_movement_extends_the_marked_selection(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    prepare_navigation_book(reader)
    reader.send("v", "Space", "C-d", "Enter")
    annotation = reader.wait_for_annotation_count(1)[0]
    expected = "\n".join(f"Line {row:03d}" for row in range(19, 38)) + "\nL"
    assert annotation.anchor.quote == expected


def test_selection_scrolloff_keeps_context_above_the_cursor(
    terminal_reader: TerminalReader,
) -> None:
    reader = terminal_reader
    prepare_navigation_book(reader, scrolloff=5)
    reader.send("v", "C-d")
    reader.wait_for("Line 057")
    reader.send(*(["k"] * 15))
    reader.wait_for("Line 018")
    assert reader.command("capture-pane", "-pt", "reader").splitlines()[0].strip() == "Line 018"
    assert int(reader.command("display-message", "-pt", "reader", "#{cursor_y}").strip()) == 5


@pytest.mark.parametrize("scrolloff,top,cursor_row", [(0, 1, 38), (5, 6, 33), (999, 20, 19)])
def test_selection_scrolloff_keeps_context_below_the_cursor(
    terminal_reader: TerminalReader, scrolloff: int, top: int, cursor_row: int
) -> None:
    reader = terminal_reader
    prepare_navigation_book(reader, scrolloff=scrolloff)
    reader.send("v", *(["j"] * 20))
    reader.wait_for(f"Line {top + 38:03d}")
    assert (
        reader.command("capture-pane", "-pt", "reader").splitlines()[0].strip() == f"Line {top:03d}"
    )
    assert (
        int(reader.command("display-message", "-pt", "reader", "#{cursor_y}").strip()) == cursor_row
    )
