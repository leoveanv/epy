"""Jump history retains context through existing reader commands and reflow."""

import dataclasses
import fcntl
import json
import os
import pty
import struct
import subprocess
import termios

import pytest
from link_helpers import LinkChapter, build_link_book
from test_annotation_terminal import TerminalReader
from test_annotation_terminal import terminal_reader as terminal_reader

from epy_reader.settings import CfgDefaultKeymaps, Settings


def configure_links(reader: TerminalReader, *, spread: bool = False) -> None:
    (reader.home / ".config/epy/configuration.json").write_text(
        json.dumps(
            dict(
                Setting=dataclasses.asdict(
                    Settings(
                        PageScrollAnimation=False,
                        ShowProgressIndicator=False,
                        StartWithDoubleSpread=spread,
                    )
                ),
                Keymap=dataclasses.asdict(CfgDefaultKeymaps()),
            )
        )
    )


def test_toc_jump_can_return_with_history(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    body = "<p>First passage</p><pre>" + "\n".join(f"LINE {i}" for i in range(100)) + "</pre>"
    build_link_book(reader.book, [LinkChapter(body=body)])
    configure_links(reader)
    reader.restart()
    reader.send("C-d", "Tab", "Enter")
    reader.wait_for("First passage")
    reader.send("C-o")
    reader.wait_for_absence("First passage")
    assert reader.command("capture-pane", "-pt", "reader").splitlines()[0].strip() == "LINE 18"


@pytest.mark.parametrize("spread", [False, True])
def test_link_history_survives_width_change(terminal_reader: TerminalReader, spread: bool) -> None:
    reader = terminal_reader
    body = (
        "<p>First passage</p><pre>"
        + "\n".join(f"LINE {i} text" for i in range(80))
        + '</pre><p><a href="notes.xhtml#n">reference</a></p>'
    )
    build_link_book(
        reader.book,
        [LinkChapter(body=body), LinkChapter(path="notes.xhtml", body='<p id="n">REFERENCE</p>')],
    )
    configure_links(reader, spread=spread)
    reader.restart()
    reader.send("G")
    reader.wait_for("reference")
    before = reader.command("capture-pane", "-pt", "reader").splitlines()[0].strip()
    reader.send("f")
    reader.wait_for("Follow link:")
    reader.send("1", "Enter")
    reader.wait_for("REFERENCE")
    reader.command("resize-window", "-t", "reader", "-x", "120", "-y", "40")
    reader.wait_for("REFERENCE")
    reader.send("C-o")
    reader.wait_for_absence("REFERENCE")
    assert reader.command("capture-pane", "-pt", "reader").splitlines()[0].strip() == before


def test_extended_input_preserves_note_editor_tab(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    reader.send("v", "Space", "n")
    reader.wait_for("Note  |")
    reader.paste("Enhanced input")
    reader.command("send-keys", "-t", "reader", "-l", "\x1b[9u\x1b[13u")
    assert reader.wait_for_note("Enhanced input").note == "Enhanced input"


def test_physical_ctrl_i_reaches_reader_through_tmux(terminal_reader: TerminalReader) -> None:
    reader = terminal_reader
    reader.command("set-option", "-s", "extended-keys", "on")
    reader.command("set-option", "-s", "extended-keys-format", "csi-u")
    reader.command("set-option", "-as", "terminal-features", ",xterm-ghostty:extkeys")
    descriptor, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 180, 1440, 640))
    environment = dict(os.environ, TERM="xterm-ghostty")
    environment.pop("TMUX", None)
    process = subprocess.Popen(
        ["tmux", "-L", reader.socket, "attach-session", "-t", "reader"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=environment,
    )
    os.close(slave)
    try:
        build_link_book(reader.book)
        configure_links(reader)
        reader.restart()
        reader.send("f")
        reader.wait_for("Follow link:")
        reader.send("1", "Enter")
        reader.wait_for("REFERENCE")
        os.write(descriptor, b"\x0f")
        reader.wait_for("First passage [1]")
        os.write(descriptor, b"\x1b[105;5u")
        reader.wait_for("REFERENCE")
        os.write(descriptor, b"\x09")
        reader.wait_for("First Chapter")
    finally:
        process.terminate()
        process.wait(timeout=5)
        os.close(descriptor)
