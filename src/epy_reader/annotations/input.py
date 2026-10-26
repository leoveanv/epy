"""Curses input with bracketed paste isolated from command dispatch."""

import curses
import sys
from dataclasses import dataclass
from typing import Iterator

from epy_reader.keyboard import PASTE_START, read_value


@dataclass(frozen=True)
class InputEvent:
    key: str = ""
    text: str = ""
    pasted: bool = False


def bracketed_paste(enabled: bool) -> None:
    sys.stdout.write("\x1b[?2004h" if enabled else "\x1b[?2004l")
    sys.stdout.flush()


def read_event(screen: curses.window, *, timeout_ms: int = -1) -> InputEvent:
    value = read_value(screen, timeout_ms=timeout_ms)
    if value == PASTE_START:
        return InputEvent(text=read_paste(screen), pasted=True)
    keys = {
        27: "escape",
        curses.KEY_LEFT: "left",
        curses.KEY_RIGHT: "right",
        curses.KEY_UP: "up",
        curses.KEY_DOWN: "down",
        curses.KEY_HOME: "home",
        curses.KEY_END: "end",
        curses.KEY_BACKSPACE: "backspace",
        curses.KEY_DC: "delete",
        curses.KEY_RESIZE: "resize",
        curses.KEY_NPAGE: "page_down",
        curses.KEY_PPAGE: "page_up",
    }
    if isinstance(value, int):
        return InputEvent(key=keys.get(value, "unknown"))
    controls = {
        "\x1b": "escape",
        "\n": "enter",
        "\r": "enter",
        "\t": "tab",
        "\x7f": "backspace",
        "\b": "backspace",
        "\x15": "half_page_up",
        "\x04": "half_page_down",
    }
    return InputEvent(key=controls.get(value, ""), text="" if value in controls else value)


def read_events(screen: curses.window, limit: int = 32) -> Iterator[InputEvent]:
    # NOTE: Bounded input batches avoid redraw backlogs without consuming keys after a modal Save.
    yield read_event(screen)
    for _ in range(max(0, limit - 1)):
        screen.timeout(0)
        try:
            event = read_event(screen, timeout_ms=0)
        except curses.error:
            break
        finally:
            screen.timeout(-1)
        yield event


def read_paste(screen: curses.window) -> str:
    text = ""
    while not text.endswith("\x1b[201~"):
        value = screen.get_wch()
        if isinstance(value, str):
            text += value
    return text[:-6]


def draw_text(screen: curses.window, row: int, col: int, text: str, attr: int = 0) -> None:
    from epy_reader.annotations.text import clip_text

    rows, cols = screen.getmaxyx()
    if not 0 <= row < rows or not 0 <= col < cols:
        return
    try:
        screen.addstr(row, col, clip_text(text, max(0, cols - col - 1)), attr)
    except curses.error:
        pass


def confirm(screen: curses.window, message: str) -> bool:
    screen.erase()
    draw_text(screen, 1, 2, message)
    draw_text(screen, 3, 2, "y: confirm    any other key: cancel")
    screen.refresh()
    event = read_event(screen)
    return not event.pasted and event.text.lower() == "y"


def notice(screen: curses.window, message: str) -> None:
    screen.erase()
    for row, line in enumerate(message.splitlines()):
        draw_text(screen, row + 1, 2, line)
    draw_text(
        screen,
        min(screen.getmaxyx()[0] - 2, len(message.splitlines()) + 3),
        2,
        "Press any key to continue.",
    )
    screen.refresh()
    read_event(screen)
