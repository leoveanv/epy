"""Exercise selection, graphics passthrough, and cleanup in the actual curses reader."""

import base64
import dataclasses
import fcntl
import json
import os
import pty
import select
import shlex
import shutil
import sqlite3
import struct
import subprocess
import sys
import termios
import time
import zipfile
import zlib
from dataclasses import dataclass, field
from pathlib import Path
from threading import Event, Lock, Thread
from typing import Iterator, List, Optional

import pytest
from image_helpers import build_png
from inline_helpers import ImageChapter, build_chapter_images
from test_annotation_terminal import TerminalReader, build_epub
from test_annotation_terminal import terminal_reader as terminal_reader

from epy_reader.settings import CfgDefaultKeymaps, Settings


@dataclass
class ImageTransmission:
    image_id: int
    pixels: bytes
    width_px: int
    height_px: int


@dataclass
class GraphicsClient:
    descriptor: int
    traffic: bytes = b""
    lock: Lock = field(default_factory=Lock)

    def read(self) -> None:
        with self.lock:
            for _ in range(64):
                if not select.select([self.descriptor], [], [], 0.01)[0]:
                    break
                self.traffic += os.read(self.descriptor, 65536)

    def collect(self, stopped: Event) -> None:
        while not stopped.wait(0.001):
            self.read()

    def wait_for(self, token: bytes) -> bytes:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            self.read()
            if token in self.traffic:
                return self.traffic
        raise AssertionError("Missing graphics traffic: " + repr(token))

    # NOTE: Kitten omits optional final-chunk markers and base64 padding in its graphics output.
    def images(self) -> List[ImageTransmission]:
        transmissions = []
        payload = ""
        image_id = 0
        metadata = dict()
        for packet in self.traffic.decode("utf-8", errors="ignore").split("\x1b_G")[1:]:
            if "\x1b\\" not in packet:
                continue
            command = packet.split("\x1b\\", 1)[0]
            if ";" not in command:
                continue
            header, chunk = command.split(";", 1)
            fields = dict(field.split("=", 1) for field in header.split(","))
            if fields.get("a") == "T":
                payload = ""
                image_id = int(fields["i"])
                metadata = fields
            payload += chunk
            if fields.get("m", "0") == "0":
                pixels = base64.b64decode(payload + "=" * (-len(payload) % 4), validate=True)
                if metadata.get("o") == "z":
                    pixels = zlib.decompress(pixels)
                transmissions.append(
                    ImageTransmission(image_id, pixels, int(metadata["s"]), int(metadata["v"]))
                )
        return transmissions

    def wait_for_images(self, count: int = 1) -> List[ImageTransmission]:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            self.read()
            images = self.images()
            if len(images) >= count:
                return images
        raise AssertionError("Expected {} transmitted images, got {}".format(count, len(images)))


def configure_image_reader(
    reader: TerminalReader, viewer: str = "false", *, inline: bool = False, seamless: bool = False
) -> None:
    settings = Settings(
        PageScrollAnimation=False,
        ShowProgressIndicator=False,
        DefaultViewer=viewer,
        InlineImages=inline,
        SeamlessBetweenChapters=seamless,
    )
    (reader.home / ".config/epy/configuration.json").write_text(
        json.dumps(
            dict(
                Setting=dataclasses.asdict(settings), Keymap=dataclasses.asdict(CfgDefaultKeymaps())
            )
        )
    )


def build_image_book(reader: TerminalReader, *, second_image: bytes = b"") -> None:
    build_epub(
        reader.book,
        '<p>First passage before images.</p><img src="first.png"/>'
        '<p>Between images.</p><img src="second.png"/><p>After images.</p>',
    )
    with zipfile.ZipFile(reader.book, "a") as archive:
        archive.writestr("first.png", build_png())
        archive.writestr("second.png", second_image or build_png(rgb=b"\x00\x00\xff"))


@pytest.fixture
def image_reader(terminal_reader: TerminalReader) -> TerminalReader:
    configure_image_reader(terminal_reader)
    build_image_book(terminal_reader)
    terminal_reader.restart()
    return terminal_reader


@pytest.fixture
def graphics_client(image_reader: TerminalReader) -> Iterator[GraphicsClient]:
    if shutil.which("kitten") is None:
        pytest.skip("standalone kitten is required for the graphics integration tests")
    reader = image_reader
    descriptor, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 180, 1440, 640))
    environment = dict(os.environ, TERM="xterm-ghostty", TERM_PROGRAM="ghostty")
    environment.pop("TMUX", None)
    reader.command("set-option", "-g", "allow-passthrough", "on")
    process = subprocess.Popen(
        ["tmux", "-L", reader.socket, "attach-session", "-t", "reader"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=environment,
    )
    os.close(slave)
    client = GraphicsClient(descriptor)
    stopped = Event()
    collector = Thread(target=client.collect, args=(stopped,), daemon=True)
    collector.start()
    try:
        deadline = time.monotonic() + 5
        while not reader.command("display-message", "-p", "#{client_termname}").strip():
            client.read()
            if time.monotonic() >= deadline:
                raise AssertionError("Graphics client did not attach")
        yield client
    finally:
        stopped.set()
        collector.join(timeout=5)
        process.kill()
        process.wait(timeout=5)
        os.close(descriptor)


def test_inline_images_appear_without_opening_preview(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    configure_image_reader(reader, inline=True)
    reader.restart()
    images = graphics_client.wait_for_images(2)
    reader.wait_for("Between images")
    captured = reader.command("capture-pane", "-pt", "reader")
    assert "\U0010eeee" in captured
    assert "[IMAGE]" not in captured
    assert images[0].pixels[:3] != images[1].pixels[:3]


def test_inline_scrolling_reuses_transmitted_image_rows(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    configure_image_reader(reader, inline=True)
    reader.restart()
    graphics_client.wait_for_images(2)
    before = reader.command("capture-pane", "-pt", "reader").splitlines()
    reader.send("5", "j")
    reader.wait_for_absence("First passage")
    after = reader.command("capture-pane", "-pt", "reader").splitlines()
    assert after[0].strip() == before[5].strip()
    graphics_client.read()
    assert len(graphics_client.images()) == 2


def test_partially_visible_inline_image_can_open_in_preview(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    configure_image_reader(reader, inline=True)
    reader.restart()
    graphics_client.wait_for_images(2)
    reader.send("5", "j")
    reader.wait_for_absence("First passage")
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("Enter")
    reader.wait_for("Image preview | first.png")
    preview = graphics_client.wait_for_images(3)[-1]
    assert preview.pixels[:3] == b"\xff\x00\x00"


@pytest.mark.parametrize("width", [80, 60])
def test_inline_bookmark_uses_original_text_rows(
    image_reader: TerminalReader, graphics_client: GraphicsClient, width: int
) -> None:
    reader = image_reader
    original = reader.command("capture-pane", "-pt", "reader").splitlines()
    original_row = next(i for i, line in enumerate(original) if "Between images." in line)
    reader.command("resize-window", "-t", "reader", "-x", "180", "-y", "20")
    configure_image_reader(reader, inline=True)
    reader.restart()
    graphics_client.wait_for_images()
    reader.send("2", "4", "j")
    reader.wait_for_absence("First passage")
    reader.send("b")
    reader.wait_for("Add bookmark")
    reader.send("example", "Enter")
    reader.wait_for_absence("Add bookmark")
    reader.send("B")
    reader.wait_for("example")
    with sqlite3.connect(reader.home / ".config/epy/states.db") as connection:
        row = connection.execute("SELECT row FROM bookmarks WHERE name=?", ("example",)).fetchone()[
            0
        ]
    assert row == original_row
    reader.send("Escape", "g", str(width), "=")
    reader.wait_for("First passage")
    reader.send("B")
    reader.wait_for("example")
    reader.send("Enter")
    reader.wait_for_absence("Bookmarks")
    reader.wait_for_absence("First passage")
    assert (
        reader.command("capture-pane", "-pt", "reader").splitlines()[0].strip() == "Between images."
    )


def test_inline_highlights_keep_the_original_canonical_quote(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    configure_image_reader(reader, inline=True)
    reader.restart()
    graphics_client.wait_for_images(2)
    reader.send("v", "Space", "l", "l", "l", "l", "Enter")
    annotations = reader.wait_for_annotation_count(1)
    assert annotations[0].anchor.quote in "Between images."
    assert "[IMAGE]" not in annotations[0].anchor.quote


def test_inline_resize_keeps_the_visible_image_fragment(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    configure_image_reader(reader, inline=True)
    reader.restart()
    graphics_client.wait_for_images(2)
    reader.send("5", "j")
    reader.wait_for_absence("First passage")
    before = reader.command("capture-pane", "-pt", "reader").splitlines()[0]
    reader.command("resize-window", "-t", "reader", "-x", "78", "-y", "28")
    images = graphics_client.wait_for_images(3)
    reader.wait_for("Between images")
    after = reader.command("capture-pane", "-pt", "reader").splitlines()[0]
    assert after.split("\U0010eeee")[1][0] == before.split("\U0010eeee")[1][0]
    assert images[-1].width_px <= 74 * 8


def test_inline_preview_preserves_surrounding_text_colors(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    configure_image_reader(reader, inline=True)
    reader.restart()
    graphics_client.wait_for_images(2)
    before = reader.command("capture-pane", "-ept", "reader")
    reader.send("o", "Enter")
    graphics_client.wait_for_images(3)
    reader.send("Escape")
    reader.wait_for_absence("Image preview")
    assert reader.command("capture-pane", "-ept", "reader") == before


@pytest.mark.parametrize("seamless", [False, True])
def test_inline_chapter_navigation_resolves_local_image_paths(
    image_reader: TerminalReader, graphics_client: GraphicsClient, seamless: bool
) -> None:
    reader = image_reader
    build_chapter_images(
        reader.book,
        [
            ImageChapter("First passage", paragraphs=20),
            ImageChapter("Second chapter", color=b"\x00\x00\xff"),
        ],
    )
    configure_image_reader(reader, inline=True, seamless=seamless)
    reader.restart()
    graphics_client.wait_for_images()
    reader.send("L")
    reader.wait_for("Second chapter paragraph")
    images = graphics_client.wait_for_images(2)
    assert images[-1].pixels[:3] == b"\x00\x00\xff"
    reader.send("H")
    reader.wait_for("First passage")
    assert reader.command("display-message", "-pt", "reader", "#{pane_dead}").strip() == "0"


def test_inline_two_column_reading_clips_images_into_each_column(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    configure_image_reader(reader, inline=True)
    reader.restart()
    graphics_client.wait_for_images(2)
    reader.send("D")
    graphics_client.wait_for_images(4)
    reader.wait_for("After images")
    assert any(
        line.count("\U0010eeee") > 80
        for line in reader.command("capture-pane", "-pt", "reader").splitlines()
    )


def test_inline_restart_preserves_text_position_without_image_support(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    configure_image_reader(reader, inline=True)
    reader.command("resize-window", "-t", "reader", "-x", "180", "-y", "20")
    reader.restart()
    graphics_client.wait_for_images()
    reader.send("2", "4", "j")
    reader.wait_for_absence("First passage")
    reader.send("q")
    reader.wait_for("Pane is dead")
    configure_image_reader(reader, inline=False)
    reader.send("Enter")
    command = shlex.join(
        [
            "env",
            "HOME=" + str(reader.home),
            "TERM=tmux-256color",
            sys.executable,
            "-m",
            "epy_reader",
            str(reader.book),
        ]
    )
    reader.command("respawn-pane", "-k", "-t", "reader", command)
    reader.wait_for("Between images")
    assert (
        reader.command("capture-pane", "-pt", "reader").splitlines()[0].strip() == "Between images."
    )


def test_inline_cache_evicts_old_images_without_deleting_visible_ones(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    chapters = [ImageChapter("First passage", paragraphs=20)] + [
        ImageChapter(f"Chapter {index}", paragraphs=20) for index in range(1, 34)
    ]
    build_chapter_images(reader.book, chapters)
    configure_image_reader(reader, inline=True, seamless=True)
    reader.restart()
    first = graphics_client.wait_for_images()[0]
    for index in range(1, 34):
        reader.send("L")
        reader.wait_for(f"Chapter {index} paragraph")
        graphics_client.wait_for_images(index + 1)
    traffic = graphics_client.wait_for(f"a=d,d=I,i={first.image_id},q=2".encode())
    latest = graphics_client.images()[-1]
    assert f"a=d,d=I,i={latest.image_id},q=2".encode() not in traffic


def test_inline_exit_deletes_owned_images(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    configure_image_reader(reader, inline=True)
    reader.restart()
    images = graphics_client.wait_for_images(2)
    reader.send("q")
    for transmission in images:
        graphics_client.wait_for("a=d,d=I,i={},q=2".format(transmission.image_id).encode())
    assert (
        reader.command("show-options", "-p", "-v", "-t", "reader", "allow-passthrough").strip()
        == ""
    )


def test_image_selection_shows_the_active_image(image_reader: TerminalReader) -> None:
    reader = image_reader
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("Down")
    reader.wait_for("Image 2/2")
    reader.send("Up")
    reader.wait_for("Image 1/2")
    assert "┌" in reader.command("capture-pane", "-pt", "reader")


def test_cancelling_image_selection_restores_reading(image_reader: TerminalReader) -> None:
    reader = image_reader
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("Escape")
    reader.wait_for_absence("Image 1/2")
    assert "First passage" in reader.command("capture-pane", "-pt", "reader")


def test_unsupported_terminal_explains_external_viewing(image_reader: TerminalReader) -> None:
    reader = image_reader
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("Enter")
    reader.wait_for("Ghostty or Kitty")
    reader.send("Escape")
    reader.wait_for_absence("Image preview")
    assert reader.command("display-message", "-pt", "reader", "#{pane_dead}").strip() == "0"


def test_selected_image_is_transmitted_through_tmux(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("j", "Enter")
    reader.wait_for("Image preview | second.png")
    transmission = graphics_client.wait_for_images()[0]
    assert transmission.pixels[:3] == b"\x00\x00\xff"


def test_wide_images_open_in_large_tmux_panes(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    build_image_book(reader, second_image=build_png(160, 40, rgb=b"\x00\x00\xff"))
    reader.restart()
    reader.command("resize-window", "-t", "reader", "-x", "308", "-y", "64")
    reader.wait_for("First passage")
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("j", "Enter")
    transmission = graphics_client.wait_for_images()[0]
    assert transmission.pixels[:3] == b"\x00\x00\xff"
    assert transmission.width_px <= 296 * 8
    reader.send("Escape")
    reader.wait_for_absence("Image preview")


def test_leaving_preview_deletes_its_image(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("Enter")
    transmission = graphics_client.wait_for_images()[0]
    reader.send("Escape")
    graphics_client.wait_for("a=d,d=I,i={},q=2".format(transmission.image_id).encode())
    reader.wait_for_absence("Image preview")
    assert "First passage" in reader.command("capture-pane", "-pt", "reader")


@pytest.mark.parametrize("theme", [0, 1, 2])
def test_closing_preview_preserves_reading_text_colors(
    image_reader: TerminalReader, graphics_client: GraphicsClient, theme: int
) -> None:
    reader = image_reader
    reader.send(str(theme), "c")
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("Escape")
    reader.wait_for_absence("Image 1/2")
    before = reader.command("capture-pane", "-ept", "reader")
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("Enter")
    graphics_client.wait_for_images()
    reader.send("Escape")
    reader.wait_for_absence("Image preview")
    after = reader.command("capture-pane", "-ept", "reader")
    assert after == before


def test_resizing_preview_retransmits_the_image(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("Enter")
    first = graphics_client.wait_for_images()[0]
    reader.command("resize-window", "-t", "reader", "-x", "120", "-y", "28")
    second = graphics_client.wait_for_images(2)[1]
    assert second.width_px < first.width_px
    assert second.pixels[:3] == first.pixels[:3]
    reader.send("Escape")
    reader.wait_for_absence("Image preview")
    assert reader.command("display-message", "-pt", "reader", "#{pane_dead}").strip() == "0"


def test_invalid_image_offers_external_viewing(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    build_image_book(reader, second_image=b"not an image")
    reader.restart()
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("j", "Enter")
    reader.wait_for("Cannot render")
    assert not graphics_client.images()


def test_external_viewer_receives_the_selected_original(
    image_reader: TerminalReader, tmp_path: Path
) -> None:
    reader = image_reader
    helper = tmp_path / "external viewer.py"
    helper.write_text("import shutil, sys\nshutil.copyfile(sys.argv[2], sys.argv[1])\n")
    result = tmp_path / "selected image.png"
    viewer = shlex.join([sys.executable, str(helper), str(result)])
    configure_image_reader(reader, viewer=viewer)
    reader.restart()
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("j", "Enter")
    reader.wait_for("Image preview")
    reader.send("e")
    reader.wait_for("Opened externally")
    with zipfile.ZipFile(reader.book) as archive:
        assert result.read_bytes() == archive.read("second.png")
    reader.send("Escape")
    reader.wait_for_absence("Opened externally")


@pytest.mark.parametrize("local_mode", [None, "on", "all"])
def test_preview_restores_the_pane_passthrough_setting(
    image_reader: TerminalReader, graphics_client: GraphicsClient, local_mode: Optional[str]
) -> None:
    reader = image_reader
    if local_mode is not None:
        reader.command("set-option", "-p", "-t", "reader", "allow-passthrough", local_mode)
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("Enter")
    graphics_client.wait_for_images()
    assert (
        reader.command("display-message", "-pt", "reader", "#{allow-passthrough}").strip() == "all"
    )
    reader.send("Escape")
    reader.wait_for_absence("Image preview")
    assert reader.command(
        "show-options", "-p", "-v", "-t", "reader", "allow-passthrough"
    ).strip() == (local_mode or "")


def test_missing_image_helper_offers_external_viewing(
    image_reader: TerminalReader, graphics_client: GraphicsClient
) -> None:
    reader = image_reader
    reader.restart(search_path="/opt/homebrew/bin:/usr/bin:/bin")
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.send("Enter")
    reader.wait_for("Install standalone kitten")
    reader.send("Escape")
    reader.wait_for_absence("Image preview")
    assert reader.command("display-message", "-pt", "reader", "#{pane_dead}").strip() == "0"


def test_single_image_still_requires_confirmation(image_reader: TerminalReader) -> None:
    reader = image_reader
    with zipfile.ZipFile(reader.book) as archive:
        original = archive.read("first.png")
    build_epub(reader.book, '<p>First passage.</p><img src="first.png"/>')
    with zipfile.ZipFile(reader.book, "a") as archive:
        archive.writestr("first.png", original)
    reader.restart()
    reader.send("o")
    reader.wait_for("Image 1/1")
    assert "Image preview" not in reader.command("capture-pane", "-pt", "reader")


def test_resizing_image_selection_returns_to_reading(image_reader: TerminalReader) -> None:
    reader = image_reader
    reader.send("o")
    reader.wait_for("Image 1/2")
    reader.command("resize-window", "-t", "reader", "-x", "120", "-y", "28")
    reader.wait_for_absence("Image 1/2")
    reader.send("o")
    reader.wait_for("Image 1/2")
    assert reader.command("display-message", "-pt", "reader", "#{pane_dead}").strip() == "0"
