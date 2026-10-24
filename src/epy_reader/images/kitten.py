"""Delegate image decoding and terminal graphics to the standalone kitten helper."""

import re
import subprocess
import tempfile
from pathlib import Path

from epy_reader.images.models import ImageCells, ImageRequest, ImageSource

# NOTE: Kitten 0.49.2 rejects dimensions of 297 or more cells, so its safe maximum is 296.
MAX_PLACEHOLDER_CELLS = 296
# NOTE: This expression removes ANSI CSI sequences, including colors, from helper error messages.
ANSI_ESCAPE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


class ImagePreviewError(ValueError):
    pass


def render_image(helper: str, source: ImageSource, request: ImageRequest) -> str:
    if len(source.data) > 32 * 1024 * 1024:
        raise ImagePreviewError(
            "Image exceeds the 32 MiB preview limit. Press e to open externally."
        )
    if request.rows < 8 or request.cols < 20:
        raise ImagePreviewError("Terminal is too small for an image preview.")
    cols = min(request.cols - 4, MAX_PLACEHOLDER_CELLS)
    rows = min(request.rows - 5, MAX_PLACEHOLDER_CELLS)
    left = (request.cols - cols) // 2
    top = 2 + (request.rows - 5 - rows) // 2
    if request.placement is not None:
        cols = min(request.placement.cols, MAX_PLACEHOLDER_CELLS)
        rows = min(request.placement.rows, MAX_PLACEHOLDER_CELLS)
        left, top = request.placement.left, request.placement.top
    arguments = [
        helper,
        "icat",
        "--stdin=no",
        "--transfer-mode=stream",
        "--unicode-placeholder",
        "--loop=0",
        "--scale-up",
        f"--use-window-size={request.cols},{request.rows},"
        f"{request.cols * request.cell_width_px},{request.rows * request.cell_height_px}",
        f"--place={cols}x{rows}@{left}x{top}",
        f"--image-id={request.image_id}",
        "--passthrough=" + ("tmux" if request.in_tmux else "none"),
    ]
    try:
        with tempfile.NamedTemporaryFile(suffix=Path(source.name).suffix) as image:
            image.write(source.data)
            image.flush()
            result = subprocess.run(
                [*arguments, image.name],
                stdin=subprocess.DEVNULL,
                capture_output=True,
                start_new_session=True,
                timeout=10,
            )
        if result.returncode or not result.stdout:
            detail = _helper_detail(result.stderr).replace(image.name, source.name)
            raise ImagePreviewError(
                f"Cannot render this image with kitten: {detail} Press e to open externally."
            )
        return result.stdout.decode("utf-8")
    except (OSError, subprocess.SubprocessError, UnicodeError) as error:
        raise ImagePreviewError(
            "Cannot render this image with kitten. Press e to open externally."
        ) from error


# NOTE: These expressions separate complete graphics packets from kitten-generated placeholder rows.
_GRAPHICS_PACKET = re.compile(r"\x1bPtmux;(?:[^\x1b]|\x1b\x1b)*\x1b\\|\x1b_G[^\x1b]*\x1b\\")
_PLACEHOLDER_ROW = re.compile(r"\U0010eeee[^\x1b\r\n]*")


def split_image_output(output: str) -> ImageCells:
    packets = _GRAPHICS_PACKET.findall(output)
    lines = _PLACEHOLDER_ROW.findall(_GRAPHICS_PACKET.sub("", output))
    if not packets or not lines:
        raise ImagePreviewError("Kitten returned no inline image cells.")
    return ImageCells(transmission="".join(packets), lines=tuple(lines))


def _helper_detail(stderr: bytes) -> str:
    message = ANSI_ESCAPE.sub("", stderr.decode("utf-8", errors="replace"))
    return " ".join(message.split())[:400] or "kitten returned no image data."


# NOTE: kitten's clear options also remove other applications' images, so delete only epy's image ID.
def clear_image(image_id: int, *, in_tmux: bool = False) -> str:
    command = f"\x1b_Ga=d,d=I,i={image_id},q=2\x1b\\"
    return "\x1bPtmux;" + command.replace("\x1b", "\x1b\x1b") + "\x1b\\" if in_tmux else command
