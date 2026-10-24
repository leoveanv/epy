"""Exercise the real curses application in an isolated tmux server."""

import dataclasses
import json
import shlex
import shutil
import sqlite3
import subprocess
import sys
import time
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator, List
from uuid import uuid4

import pytest

from epy_reader.annotations import Annotation
from epy_reader.settings import CfgDefaultKeymaps, Settings


@dataclass
class TerminalReader:
    socket: str
    home: Path
    book: Path

    def command(self, *arguments: str) -> str:
        result = subprocess.run(
            ["tmux", "-L", self.socket, *arguments], check=True, capture_output=True, text=True
        )
        return result.stdout

    def send(self, *keys: str) -> None:
        self.command("send-keys", "-t", "reader", *keys)

    def paste(self, text: str) -> None:
        self.command("send-keys", "-t", "reader", "-l", "\x1b[200~" + text + "\x1b[201~")

    def wait_for(self, text: str) -> None:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            captured = self.command("capture-pane", "-pt", "reader")
            if text in captured:
                return
            time.sleep(0.05)
        raise AssertionError(
            "Missing {!r} in terminal:\n{}\n{}".format(
                text, captured, (self.home / "crash.log").read_text()
            )
        )

    def wait_for_annotation_count(self, count: int) -> List[Annotation]:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            annotations = self.annotations()
            if len(annotations) == count:
                return annotations
            time.sleep(0.02)
        raise AssertionError("Expected {} annotations, found {}.".format(count, len(annotations)))

    def wait_for_note(self, text: str) -> Annotation:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            annotation = next((item for item in self.annotations() if item.note == text), None)
            if annotation is not None:
                return annotation
            time.sleep(0.02)
        raise AssertionError("Expected saved note: " + text)

    def wait_for_absence(self, text: str) -> None:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if text not in self.command("capture-pane", "-pt", "reader"):
                return
            time.sleep(0.01)
        raise AssertionError("Unexpected terminal text: " + text)

    def restart(self) -> None:
        command = (
            shlex.join(
                [
                    "env",
                    "HOME=" + str(self.home),
                    "TERM=tmux-256color",
                    sys.executable,
                    "-m",
                    "epy_reader",
                    str(self.book),
                ]
            )
            + " 2>"
            + shlex.quote(str(self.home / "crash.log"))
        )
        self.command("respawn-pane", "-k", "-t", "reader", command)
        self.wait_for("First passage")

    def annotations(self) -> List[Annotation]:
        with sqlite3.connect(self.home / ".config/epy/states.db") as connection:
            return [
                Annotation.model_validate_json(row[0])
                for row in connection.execute("SELECT payload FROM annotations ORDER BY start")
            ]


def build_epub(
    path: Path,
    body: str = "<p>First passage for highlighting and notes.</p><p>Second passage follows.</p>",
) -> None:
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("mimetype", "application/epub+zip")
        archive.writestr(
            "META-INF/container.xml",
            '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="book.opf"/></rootfiles></container>',
        )
        archive.writestr(
            "book.opf",
            '<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Annotation Test</dc:title></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/><item id="nav" href="nav.xhtml" properties="nav" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="chapter"/></spine></package>',
        )
        archive.writestr(
            "nav.xhtml",
            '<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><a href="chapter.xhtml">First Chapter</a></nav></body></html>',
        )
        archive.writestr(
            "chapter.xhtml",
            "<html><body>" + body + "</body></html>",
        )


@pytest.fixture
def terminal_reader(tmp_path: Path, request: pytest.FixtureRequest) -> Iterator[TerminalReader]:
    if shutil.which("tmux") is None:
        pytest.skip("tmux is required for the terminal integration test")
    home = tmp_path / "home"
    configuration = home / ".config/epy"
    configuration.mkdir(parents=True)
    settings = Settings(
        PageScrollAnimation=False, SeamlessBetweenChapters=getattr(request, "param", False)
    )
    (configuration / "configuration.json").write_text(
        json.dumps(
            dict(
                Setting=dataclasses.asdict(settings), Keymap=dataclasses.asdict(CfgDefaultKeymaps())
            )
        )
    )
    book = tmp_path / "book.epub"
    build_epub(book)
    reader = TerminalReader("epy-test-" + uuid4().hex, home, book)
    command = (
        shlex.join(
            [
                "env",
                "HOME=" + str(home),
                "TERM=tmux-256color",
                sys.executable,
                "-m",
                "epy_reader",
                str(book),
            ]
        )
        + " 2>"
        + shlex.quote(str(home / "crash.log"))
    )
    reader.command(
        "-f", "/dev/null", "new-session", "-d", "-x", "180", "-y", "40", "-s", "reader", command
    )
    reader.command("set-option", "-w", "-t", "reader", "remain-on-exit", "on")
    try:
        reader.wait_for("First passage")
        yield reader
    finally:
        subprocess.run(["tmux", "-L", reader.socket, "kill-server"], capture_output=True)


@pytest.fixture
def noted_reader(terminal_reader: TerminalReader) -> TerminalReader:
    reader = terminal_reader
    reader.send("v", "Space", "l", "l", "l", "l", "n")
    reader.wait_for("Note  |")
    reader.paste("Existing note")
    reader.send("Tab", "Enter")
    reader.wait_for_note("Existing note")
    reader.send("Escape")
    return reader


def test_restart_upgrades_existing_obsidian_story_references(
    terminal_reader: TerminalReader, tmp_path: Path
) -> None:
    import hashlib

    from epy_reader.annotations import ExportLink, TextAnchor
    from epy_reader.annotations.store import AnnotationStore
    from epy_reader.parser import parse_html

    reader = terminal_reader
    body = '<h1>OTHER STORY</h1><p>First passage</p><h1>EUGENE</h1><span epub:type="pagebreak" title="12"/><p>Eugene passage</p>'
    build_epub(reader.book, body)
    source = parse_html(
        "<html><body>" + body + "</body></html>", textwidth=80, content_id="chapter.xhtml"
    ).documents[0]
    start = source.text.index("Eugene passage")
    store = AnnotationStore(reader.home / ".config/epy/states.db")
    book = store.identify_book(reader.book, "Annotation Test", "")
    saved = store.create(
        book.id,
        TextAnchor(
            content_id=source.content_id, start=start, end=start + 14, quote="Eugene passage"
        ),
        note="kept note",
        chapter="WRONG STORY",
    )
    section = (
        "<!-- epy:annotations:"
        + book.id
        + ":start -->\n### WRONG STORY\nold export\n<!-- epy:annotations:"
        + book.id
        + ":end -->"
    ).encode()
    destination = tmp_path / "Obsidian.md"
    outside = b"---\ntags: [books]\n---\nMy handwritten notes.\n\n"
    destination.write_bytes(outside + section + b"\n")
    store.set_link(
        ExportLink(
            book_id=book.id,
            path=str(destination),
            digest=hashlib.sha256(section).hexdigest(),
            revision=0,
            exported_revision=0,
        )
    )
    reader.restart()
    corrected = next(item for item in reader.annotations() if item.id == saved.id)
    assert corrected.chapter == "EUGENE"
    assert corrected.note == "kept note"
    assert corrected.reference.page_start == "12"
    markdown = destination.read_bytes()
    assert markdown.startswith(outside)
    assert b"### EUGENE" in markdown
    assert b"WRONG STORY" not in markdown


def test_note_editor_cursor_tracks_trailing_space(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    reader.send("v", "Space", "n")
    reader.wait_for("Note  |")
    reader.paste("alpha ")
    reader.wait_for("alpha")
    assert int(reader.command("display-message", "-pt", "reader", "#{cursor_x}").strip()) == 8
    reader.send("BSpace", "Tab", "Enter")
    assert reader.wait_for_note("alpha").note == "alpha"


def test_escape_leaves_selection_without_terminal_delay(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    reader.send("v")
    reader.wait_for("Select:")
    started = time.monotonic()
    reader.send("Escape")
    reader.wait_for_absence("Select:")
    reader.wait_for("First passage")
    assert time.monotonic() - started < 0.35


@pytest.mark.parametrize("terminal_reader", [False, True], indirect=True)
def test_selection_exit_reuses_loaded_book_after_source_becomes_unavailable(
    terminal_reader: TerminalReader,
) -> None:
    reader = terminal_reader
    build_epub(reader.book, "<p>First passage</p><p>" + "cached text " * 4000 + "</p>")
    reader.restart()
    reader.send("v")
    reader.wait_for("Select:")
    reader.book.write_bytes(b"source temporarily unavailable")
    reader.send("v", "a")
    reader.wait_for("No annotations yet")
    assert "Traceback" not in (reader.home / "crash.log").read_text()


def test_saved_highlight_accepts_note_without_reentering_selection(
    terminal_reader: TerminalReader,
) -> None:
    reader = terminal_reader
    reader.send("v", "Space", "l", "l", "l", "l", "Enter")
    original = reader.wait_for_annotation_count(1)[0]
    reader.wait_for("Select:")
    reader.send("n")
    reader.wait_for("[ Delete note ]")
    reader.paste("Added after saving")
    reader.send("Tab", "Enter")
    updated = reader.wait_for_note("Added after saving")
    reader.wait_for("Select:")
    assert (updated.id, updated.anchor) == (original.id, original.anchor)
    assert len(reader.annotations()) == 1


def test_selection_saves_multiple_independent_highlights(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    reader.send("v", "Space", "l", "l", "l", "l", "Enter")
    reader.wait_for_annotation_count(1)
    reader.send("w", "Space", "l", "l", "l", "l", "Enter")
    saved = reader.wait_for_annotation_count(2)
    assert [annotation.anchor.quote for annotation in saved] == ["Secon", "passa"]


def test_v_toggles_selection_back_to_reading(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    reader.send("v", "v", "a")
    reader.wait_for("No annotations yet")
    assert reader.annotations() == []


def test_selection_starts_in_middle_without_scrolling(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    lines = ["First passage"] + ["Line " + str(index) for index in range(1, 60)]
    with zipfile.ZipFile(reader.book, "a") as archive:
        archive.writestr(
            "chapter.xhtml", "<html><body><pre>" + "\n".join(lines) + "</pre></body></html>"
        )
    reader.restart()
    reader.send("j", "j", "j", "v")
    reader.wait_for("Select:")
    cursor_row = int(reader.command("display-message", "-pt", "reader", "#{cursor_y}").strip())
    screen = reader.command("capture-pane", "-pt", "reader")
    assert cursor_row == 19
    assert "Line 3" in screen.splitlines()[0]
    reader.send("Space", "Enter")
    saved = reader.wait_for_annotation_count(1)[0]
    assert saved.anchor.start == len("\n".join(lines[:22])) + 1


def test_n_edits_existing_note_without_duplicate(noted_reader: TerminalReader) -> None:
    reader = noted_reader
    original = reader.annotations()[0]
    reader.send("v", "n")
    reader.wait_for("[ Delete note ]")
    reader.send("End")
    reader.paste(" edited")
    reader.send("Tab", "Enter")
    updated = reader.wait_for_note("Existing note edited")
    reader.wait_for("Select:")
    assert updated.id == original.id
    assert len(reader.annotations()) == 1


def test_delete_note_preserves_highlight(noted_reader: TerminalReader) -> None:
    reader = noted_reader
    original = reader.annotations()[0]
    reader.send("v", "n")
    reader.wait_for("[ Delete note ]")
    reader.send("Tab", "Tab", "Enter")
    reader.wait_for("Delete this note?")
    reader.send("y")
    remaining = reader.wait_for_note("")
    reader.wait_for("Select:")
    assert (remaining.id, remaining.anchor) == (original.id, original.anchor)
    assert len(reader.annotations()) == 1


@pytest.mark.parametrize("delete_key", ["DC", "BSpace", "d"])
def test_selection_delete_removes_highlight_with_note(
    noted_reader: TerminalReader, delete_key: str
) -> None:
    reader = noted_reader
    reader.send("v", delete_key)
    reader.wait_for("Delete this highlight and its note?")
    reader.send("y")
    assert reader.wait_for_annotation_count(0) == []
    reader.wait_for("Select:")


def test_selection_delete_cancellation_keeps_annotation(noted_reader: TerminalReader) -> None:
    reader = noted_reader
    original = reader.annotations()
    reader.send("v", "d")
    reader.wait_for("Delete this highlight and its note?")
    reader.send("n")
    reader.wait_for("Select:")
    assert reader.annotations() == original


def test_word_motions_extend_highlight_in_tmux(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    reader.send("v", "w", "Space", "w", "w", "b", "h", "h", "Enter")
    reader.wait_for("First passage")
    assert reader.annotations()[0].anchor.quote == "passage"


def test_resize_during_note_cancel_returns_to_reading(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    reader.send("v", "Space", "n")
    reader.wait_for("Note  |")
    reader.command("resize-window", "-t", "reader", "-x", "60", "-y", "24")
    reader.send("Escape")
    reader.wait_for("First passage")
    reader.send("a")
    reader.wait_for("No annotations yet")
    assert reader.annotations() == []


def test_reopen_keeps_highlight_after_resize(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    reader.send("v", "Space", "l", "l", "l", "l", "Enter")
    reader.wait_for("First passage")
    original = reader.annotations()[0]
    reader.command("resize-window", "-t", "reader", "-x", "75", "-y", "24")
    reader.wait_for("First passage")
    reader.restart()
    assert reader.annotations() == [original]
    assert "48;5;52m" in reader.command("capture-pane", "-ept", "reader")


def test_unicode_inline_style_covers_complete_word(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    with zipfile.ZipFile(reader.book, "a") as archive:
        archive.writestr(
            "chapter.xhtml",
            "<html><body><p>界<i>First</i> passage for highlighting and notes.</p></body></html>",
        )
    reader.restart()
    styled = reader.command("capture-pane", "-ept", "reader")
    assert "First" in styled
    assert "\x1b[3m" in styled


def test_export_status_does_not_hide_reading_text(terminal_reader: TerminalReader) -> None:
    from epy_reader.annotations import ExportLink
    from epy_reader.annotations.store import AnnotationStore

    reader = terminal_reader
    text = "First passage\n" + "\n".join("Line " + str(index) for index in range(1, 50))
    with zipfile.ZipFile(reader.book, "a") as archive:
        archive.writestr("chapter.xhtml", "<html><body><pre>" + text + "</pre></body></html>")
    reader.restart()
    reader.send("v", "Space", "Enter")
    reader.wait_for("First passage")
    book_id = reader.annotations()[0].book_id
    store = AnnotationStore(reader.home / ".config/epy/states.db")
    store.set_link(ExportLink(book_id=book_id, path=str(reader.home / "missing.md")))
    reader.restart()
    reader.wait_for("Line 39")
    assert "Line 39" in reader.command("capture-pane", "-pt", "reader")


def test_changed_book_offers_validated_reassociation(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    reader.send("v", "Space", "l", "l", "l", "l", "Enter")
    reader.wait_for("First passage")
    with zipfile.ZipFile(reader.book, "a") as archive:
        archive.writestr("extra.txt", "file fingerprint changed")
    reader.restart()
    reader.send("a")
    reader.wait_for("No annotations yet")
    reader.send("r")
    reader.wait_for("Copy validated annotations")
    reader.send("y")
    reader.wait_for("Copied 1 validated annotations")
    assert len(reader.annotations()) == 2


def test_keyboard_annotation_exports_in_tmux(
    terminal_reader: TerminalReader, tmp_path: Path
) -> None:
    reader = terminal_reader
    reader.send("v")
    reader.wait_for("Select:")
    reader.send("k", "Space", "l", "l", "l", "l", "n")
    reader.wait_for("Note  |")
    reader.paste("My first note\nsecond line")
    reader.send("Tab", "Enter")
    reader.wait_for("My first note")
    reader.send("Escape", "O")
    reader.wait_for("Absolute Markdown path")
    destination = tmp_path / "Obsidian note.md"
    reader.paste(str(destination))
    reader.send("Tab", "Enter")
    reader.wait_for("Link and export")
    reader.send("y")
    reader.wait_for("Create ")
    reader.send("y")
    reader.wait_for("exported to Obsidian")
    assert reader.annotations()[0].anchor.quote == "First"
    assert "My first note" in destination.read_text()
    reader.send("a")
    reader.wait_for("Annotations |")
    reader.send("e")
    reader.wait_for("Note  |")
    reader.send("End")
    reader.paste(" edited")
    reader.send("Tab", "Enter")
    reader.wait_for("Annotations |")
    assert "edited" in destination.read_text()
    reader.send("d")
    reader.wait_for("Delete this highlight")
    reader.send("y")
    reader.wait_for("No annotations yet")
    assert reader.annotations() == []
    assert "My first note" not in destination.read_text()
