"""Curses image selection and a modal, resize-aware terminal preview."""

import curses
import os
import shutil
import sys
from contextlib import nullcontext
from dataclasses import dataclass
from enum import Enum
from typing import Mapping, Optional, Union
from uuid import uuid4

from epy_reader.annotations.input import draw_text
from epy_reader.annotations.text import wrap_text
from epy_reader.board import InfiniBoard
from epy_reader.images.kitten import ImagePreviewError, clear_image, render_image
from epy_reader.images.models import ImageRequest, ImageSource, ImageTarget
from epy_reader.images.terminal import (
    preserve_terminal_state,
    query_graphics_support,
    redraw_safe_passthrough,
)
from epy_reader.models import Key, NoUpdate
from epy_reader.keyboard import read_code
from epy_reader.settings import Keymap
from epy_reader.utils import safe_curs_set


class PreviewAction(Enum):
    CLOSE = "close"
    EXTERNAL = "external"


@dataclass(frozen=True)
class Frame:
    row: int
    col: int
    rows: int
    cols: int


def draw_frame(screen: curses.window, frame: Frame) -> None:
    draw_text(screen, frame.row, frame.col, "┌" + "─" * (frame.cols - 2) + "┐", curses.A_DIM)
    for row in range(frame.row + 1, frame.row + frame.rows - 1):
        draw_text(screen, row, frame.col, "│", curses.A_DIM)
        draw_text(screen, row, frame.col + frame.cols - 1, "│", curses.A_DIM)
    draw_text(
        screen,
        frame.row + frame.rows - 1,
        frame.col,
        "└" + "─" * (frame.cols - 2) + "┘",
        curses.A_DIM,
    )


def select_image(
    board: InfiniBoard, image_maps: Mapping[int, str], row: int, keymap: Keymap
) -> Union[ImageTarget, Key, NoUpdate]:
    targets = [
        ImageTarget(row=image_row, path=path)
        for image_row, path in sorted(image_maps.items())
        if image_row + board.image_rows > row and image_row < row + board.screen_rows * board.spread
    ]
    if not targets:
        return NoUpdate()
    index = 0
    safe_curs_set(0)
    while True:
        board.screen.erase()
        board.write(row)
        _draw_target(board, targets[index], row)
        target_row = (targets[index].row - row) % board.screen_rows
        footer_row = board.screen_rows - 1 if target_row < board.screen_rows - 1 else 0
        draw_text(board.screen, footer_row, 0, " " * (board.screen_cols - 1))
        draw_text(
            board.screen,
            footer_row,
            1,
            f"Image {index + 1}/{len(targets)} | j/k or arrows: select | Enter: preview | Esc: cancel",
            curses.A_DIM,
        )
        board.screen.refresh()
        key = board.getch()
        if key == Key(curses.KEY_RESIZE):
            return key
        if key in keymap.Quit + keymap.OpenImage:
            return NoUpdate()
        if key in keymap.Follow:
            return targets[index]
        if key in keymap.ScrollDown + keymap.PageDown:
            index = (index + 1) % len(targets)
        elif key in keymap.ScrollUp + keymap.PageUp:
            index = (index - 1) % len(targets)


def _draw_target(board: InfiniBoard, target: ImageTarget, row: int) -> None:
    offset = target.row - row
    y = max(0, offset) % board.screen_rows
    base = board.x if offset < board.screen_rows else board.x_alt
    if board.image_rows > 1:
        bottom = min(board.screen_rows - 1, offset + board.image_rows - 1)
        if offset >= board.screen_rows:
            bottom = min(board.screen_rows - 1, y + board.image_rows - 1)
        draw_frame(
            board.screen,
            Frame(row=y, col=base - 1, rows=max(2, bottom - y + 1), cols=board.textwidth + 2),
        )
        return
    line = board.text[target.row]
    marker = line.strip()
    x = base + len(line) - len(line.lstrip())
    draw_frame(board.screen, Frame(row=y - 1, col=x - 2, rows=3, cols=len(marker) + 4))
    draw_text(board.screen, y, x, marker, curses.A_BOLD)


def show_preview(screen: curses.window, source: ImageSource, keymap: Keymap) -> PreviewAction:
    support = query_graphics_support(os.environ, sys.stdout.fileno())
    helper = shutil.which("kitten")
    error = support.reason or (
        "Install standalone kitten for terminal previews. Press e to open externally."
        if helper is None
        else None
    )
    image_id = (uuid4().int & 0xFFFFFFFF) | 1
    available = error is None
    context = (
        redraw_safe_passthrough(os.environ) if available and support.in_tmux else nullcontext()
    )
    with context:
        try:
            while True:
                screen.erase()
                rows, cols = screen.getmaxyx()
                draw_text(screen, 0, 2, "Image preview | " + source.name, curses.A_BOLD)
                draw_text(
                    screen, rows - 1, 1, "Esc/Enter: return | e: open externally", curses.A_DIM
                )
                message = error
                if available and helper is not None:
                    sys.stdout.write(clear_image(image_id, in_tmux=support.in_tmux))
                    sys.stdout.flush()
                    draw_frame(screen, Frame(row=1, col=1, rows=rows - 2, cols=cols - 2))
                    screen.refresh()
                    try:
                        output = render_image(
                            helper,
                            source,
                            ImageRequest(
                                rows=rows,
                                cols=cols,
                                image_id=image_id,
                                in_tmux=support.in_tmux,
                                cell_width_px=support.cell_width_px,
                                cell_height_px=support.cell_height_px,
                            ),
                        )
                        sys.stdout.write(preserve_terminal_state(output))
                        sys.stdout.flush()
                    except ImagePreviewError as exception:
                        message = str(exception)
                if message:
                    for row, line in enumerate(wrap_text(message, max(1, cols - 4)), start=3):
                        draw_text(screen, row, 2, line)
                    screen.refresh()
                action = _preview_action(screen, keymap)
                if action is not None:
                    return action
                support = query_graphics_support(os.environ, sys.stdout.fileno())
        finally:
            if available:
                sys.stdout.write(clear_image(image_id, in_tmux=support.in_tmux))
                sys.stdout.flush()
            safe_curs_set(0)
            screen.erase()
            screen.touchwin()


def _preview_action(screen: curses.window, keymap: Keymap) -> Optional[PreviewAction]:
    while True:
        key = Key(read_code(screen))
        if key == Key(curses.KEY_RESIZE):
            return None
        if key == Key("e"):
            return PreviewAction.EXTERNAL
        if key in keymap.Quit + keymap.Follow + keymap.OpenImage:
            return PreviewAction.CLOSE
