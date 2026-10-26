"""Link navigation through the real curses reader."""

from pathlib import Path

import pytest
from link_helpers import LinkChapter, build_link_book
from test_annotation_terminal import TerminalReader
from test_annotation_terminal import terminal_reader as terminal_reader


@pytest.mark.parametrize("terminal_reader", [False, True], indirect=True)
def test_follow_link_history_returns_to_origin(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    build_link_book(reader.book)
    reader.restart()
    reader.wait_for("First passage [1]")
    reader.send("f")
    reader.wait_for("Follow link:")
    reader.send("1", "Enter")
    reader.wait_for("REFERENCE")
    reader.send("C-o")
    reader.wait_for("First passage [1]")
    reader.command("send-keys", "-t", "reader", "-l", "\x1b[105;5u")
    reader.wait_for("REFERENCE")
    reader.send("Tab")
    reader.wait_for("First Chapter")


def test_follow_missing_fragment_leaves_reading_position(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    build_link_book(
        reader.book, [LinkChapter(body='<p>First passage <a href="#missing">broken</a></p>')]
    )
    reader.restart()
    reader.send("f")
    reader.wait_for("Follow link:")
    reader.send("1", "Enter")
    reader.wait_for("missing")
    reader.send("Escape")
    reader.wait_for("First passage")
    assert "broken" in reader.command("capture-pane", "-pt", "reader")


def test_follow_hint_accepts_multi_digit_numbers(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    body = (
        "<p>First passage "
        + " ".join(f'<a href="#n{i}">link{i}</a>' for i in range(1, 13))
        + "</p>"
    )
    body += "<pre>" + "\n".join("padding" for _ in range(60)) + "</pre>"
    body += "".join(f'<p id="n{i}">TARGET {i:02d}</p>' for i in range(1, 13))
    build_link_book(reader.book, [LinkChapter(body=body)])
    reader.restart()
    reader.send("s", "f")
    reader.wait_for("Follow link:")
    reader.send("1", "2", "Enter")
    reader.wait_for("TARGET 12")
    assert reader.command("capture-pane", "-pt", "reader").splitlines()[0].strip() == "TARGET 12"


def test_follow_mode_cancellation_preserves_reading(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    build_link_book(reader.book)
    reader.restart()
    reader.send("f")
    reader.wait_for("Follow link:")
    reader.send("Escape")
    reader.wait_for_absence("Follow link:")
    assert "First passage [1]" in reader.command("capture-pane", "-pt", "reader")


def test_external_link_opens_browser_without_reader_jump(
    terminal_reader: TerminalReader, tmp_path: Path
) -> None:
    reader = terminal_reader
    output = tmp_path / "opened-url"
    browser = tmp_path / "browser"
    browser.write_text("#!/bin/sh\nprintf '%s' \"$1\" > " + str(output) + "\n")
    browser.chmod(0o755)
    reader.command("set-environment", "BROWSER", str(browser) + " %s")
    build_link_book(
        reader.book,
        [
            LinkChapter(
                body='<p>First passage <a href="https://example.org/reference">external</a></p>'
            )
        ],
    )
    reader.restart()
    reader.send("f")
    reader.wait_for("Follow link:")
    reader.send("1", "Enter")
    reader.wait_for_absence("Follow link:")
    assert output.read_text() == "https://example.org/reference"
    assert "First passage" in reader.command("capture-pane", "-pt", "reader")


def test_follow_nonspine_endnote_returns_by_backlink(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    build_link_book(
        reader.book,
        [
            LinkChapter(),
            LinkChapter(
                path="notes.xhtml",
                body='<p id="n">OFFSPINE <a href="chapter.xhtml#start">return</a></p>',
                in_spine=False,
            ),
        ],
    )
    reader.restart()
    reader.send("f")
    reader.wait_for("Follow link:")
    reader.send("1", "Enter")
    reader.wait_for("OFFSPINE")
    reader.send("f")
    reader.wait_for("Follow link:")
    reader.send("1", "Enter")
    reader.wait_for("First passage [1]")
