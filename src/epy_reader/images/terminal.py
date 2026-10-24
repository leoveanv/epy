"""Terminal discovery and the small output boundary used by curses previews."""

import struct
import subprocess
from contextlib import contextmanager
from typing import Iterator, Mapping, Optional, Sequence

from epy_reader.images.kitten import ImagePreviewError
from epy_reader.images.models import GraphicsSupport

SUPPORTED_TERMINALS = {"ghostty", "xterm-ghostty", "kitty", "xterm-kitty"}


# NOTE: Kitten saves image-ID colors, so epy must own DEC cursor and attribute restoration.
def preserve_terminal_state(output: str) -> str:
    return "\x1b7" + output.replace("\x1b7", "").replace("\x1b8", "") + "\x1b8"


def query_graphics_support(
    environment: Mapping[str, str], terminal_fd: Optional[int] = None
) -> GraphicsSupport:
    if environment.get("TMUX"):
        return _query_tmux(environment)
    terminal = environment.get("TERM", "")
    program = environment.get("TERM_PROGRAM", "").lower()
    if terminal not in SUPPORTED_TERMINALS and program not in SUPPORTED_TERMINALS:
        return GraphicsSupport(
            reason="Terminal preview requires Ghostty or Kitty. Press e to open externally."
        )
    return _query_cell_size(terminal_fd)


def _query_tmux(environment: Mapping[str, str]) -> GraphicsSupport:
    command = ["display-message", "-p"]
    if environment.get("TMUX_PANE"):
        command.extend(["-t", environment["TMUX_PANE"]])
    command.append(
        "#{client_termname}\t#{allow-passthrough}\t#{client_cell_width}\t#{client_cell_height}"
    )
    try:
        result = _run_tmux(environment, command)
        terminal, passthrough, width, height = result.split("\t")
        reason = None
        if terminal not in SUPPORTED_TERMINALS:
            reason = "Terminal preview requires Ghostty or Kitty. Press e to open externally."
        elif passthrough not in {"on", "all"}:
            reason = "tmux needs: set -g allow-passthrough on. Press e to open externally."
        return GraphicsSupport(
            in_tmux=True,
            reason=reason,
            cell_width_px=max(0, int(width or 0)) or 8,
            cell_height_px=max(0, int(height or 0)) or 16,
        )
    except (OSError, subprocess.SubprocessError, ValueError):
        return GraphicsSupport(
            in_tmux=True, reason="Cannot query tmux graphics support. Press e to open externally."
        )


def _query_cell_size(terminal_fd: Optional[int]) -> GraphicsSupport:
    if terminal_fd is not None:
        try:
            import fcntl
            import termios

            rows, cols, width, height = struct.unpack(
                "HHHH", fcntl.ioctl(terminal_fd, termios.TIOCGWINSZ, b"\0" * 8)
            )
            if rows and cols and width and height:
                return GraphicsSupport(
                    cell_width_px=max(1, width // cols), cell_height_px=max(1, height // rows)
                )
        except (ImportError, OSError):
            pass
    return GraphicsSupport()


def _run_tmux(environment: Mapping[str, str], arguments: Sequence[str]) -> str:
    return subprocess.run(
        ["tmux", *arguments],
        env=environment,
        capture_output=True,
        text=True,
        check=True,
        timeout=1,
    ).stdout.rstrip("\r\n")


# NOTE: tmux drops ordinary passthrough during full redraws. Only epy's pane temporarily needs all.
# NOTE: Restore the exact local override, including inheritance, after deleting the preview image.
@contextmanager
def redraw_safe_passthrough(environment: Mapping[str, str]) -> Iterator[None]:
    pane = environment.get("TMUX_PANE")
    if not environment.get("TMUX") or not pane:
        raise ImagePreviewError("Cannot identify this tmux pane. Press e to open externally.")
    previous = _run_tmux(environment, ["show-options", "-p", "-v", "-t", pane, "allow-passthrough"])
    _run_tmux(environment, ["set-option", "-p", "-t", pane, "allow-passthrough", "all"])
    try:
        yield
    finally:
        if previous:
            _run_tmux(environment, ["set-option", "-p", "-t", pane, "allow-passthrough", previous])
        else:
            _run_tmux(environment, ["set-option", "-p", "-u", "-t", pane, "allow-passthrough"])
