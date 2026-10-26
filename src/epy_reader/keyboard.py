"""Decode extended keys without aliasing physical Ctrl-I to Tab."""

import curses
from contextlib import contextmanager
from typing import IO, Iterator, Mapping, Union

CTRL_I = 0x110000
PASTE_START = 0x110001


def modified_key(code: int, modifiers: int) -> int:
    if modifiers & 4:
        if code in (ord("i"), ord("I")):
            return CTRL_I
        if 64 <= code <= 127:
            return code & 31
    if modifiers & ~1:
        return -1
    return 10 if code == 13 else code


def decode_sequence(sequence: str) -> int:
    if sequence == "[200~":
        return PASTE_START
    if not sequence.startswith("[") or sequence.startswith(("[?", "[>")):
        return -1
    body, final = sequence[1:-1], sequence[-1:]
    try:
        parts = body.split(";")
        if final == "u":
            code = int(parts[0].split(":")[0])
            modifier = parts[1].split(":") if len(parts) > 1 else ["1"]
            if len(modifier) > 1 and modifier[1] == "3":
                return -1
            functional = {
                57348: curses.KEY_IC,
                57349: curses.KEY_DC,
                57350: curses.KEY_LEFT,
                57351: curses.KEY_RIGHT,
                57352: curses.KEY_UP,
                57353: curses.KEY_DOWN,
                57354: curses.KEY_PPAGE,
                57355: curses.KEY_NPAGE,
                57356: curses.KEY_HOME,
                57357: curses.KEY_END,
            }
            return functional.get(code, modified_key(code, (int(modifier[0]) - 1) & 63))
        if final == "~" and len(parts) == 3 and parts[0] == "27":
            return modified_key(int(parts[2]), int(parts[1]) - 1)
        arrows = {
            "A": curses.KEY_UP,
            "B": curses.KEY_DOWN,
            "C": curses.KEY_RIGHT,
            "D": curses.KEY_LEFT,
            "H": curses.KEY_HOME,
            "F": curses.KEY_END,
            "Z": curses.KEY_BTAB,
        }
        return arrows.get(final, -1)
    except (ValueError, IndexError):
        return -1


def read_escape(screen: curses.window, *, timeout_ms: int = -1) -> Union[str, int]:
    sequence = ""
    screen.timeout(40)
    try:
        for _ in range(128):
            try:
                value = screen.get_wch()
            except curses.error:
                break
            if not isinstance(value, str):
                curses.unget_wch(value)
                break
            if not sequence and value != "[":
                curses.unget_wch(value)
                break
            sequence += value
            if len(sequence) > 1 and "@" <= value <= "~":
                code = decode_sequence(sequence)
                if sequence.endswith("u") and code >= 0:
                    original = sequence[1:-1].split(";")[0].split(":")[0]
                    if (
                        original.isdigit()
                        and not 57344 <= int(original) <= 63743
                        and code <= 0x10FFFF
                    ):
                        return chr(code)
                return code
    finally:
        screen.timeout(timeout_ms)
    return 27


def read_value(screen: curses.window, *, timeout_ms: int = -1) -> Union[str, int]:
    screen.timeout(timeout_ms)
    value = screen.get_wch()
    if value == "\x1b":
        return read_escape(screen, timeout_ms=timeout_ms)
    return value


def read_code(screen: curses.window, *, timeout_ms: int = -1) -> int:
    try:
        value = read_value(screen, timeout_ms=timeout_ms)
        return ord(value) if isinstance(value, str) else value
    except curses.error:
        return -1


@contextmanager
def keyboard_mode(environment: Mapping[str, str], output: IO[str]) -> Iterator[None]:
    # NOTE: tmux negotiates xterm extended keys with applications, even when it emits CSI-u input.
    enable, disable = (
        ("\x1b[>4;2m", "\x1b[>4;0m") if environment.get("TMUX") else ("\x1b[>1u", "\x1b[<u")
    )
    output.write(enable)
    output.flush()
    try:
        yield
    finally:
        output.write(disable)
        output.flush()
