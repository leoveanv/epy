"""Numbered link hints are a temporary overlay, never part of book text."""

import curses
from typing import Optional, Union

from epy_reader.annotations.input import draw_text, notice
from epy_reader.board import InfiniBoard
from epy_reader.links.models import TextLink
from epy_reader.links.navigation import hint_placements, visible_hints
from epy_reader.models import Key, NoUpdate, TextStructure


def select_link(
    board: InfiniBoard, structure: TextStructure, top_row: int
) -> Optional[Union[TextLink, Key]]:
    height = board.screen_rows - 1
    hints = visible_hints(structure, top_row=top_row, height=height, spread=board.spread)
    if not hints:
        notice(board.screen, "No visible links.")
        return None
    placements = hint_placements(hints, textwidth=board.textwidth)
    first_hint = 0
    digits = ""
    while True:
        board.screen.erase()
        if placements:
            board.write(top_row, bottom_padding=1)
            for placement in placements:
                hint = placement.hint
                page, row = divmod(hint.cell.row - top_row, height)
                left = board.x if page == 0 else board.x_alt
                label = "[" + str(hint.number) + "]"
                draw_text(
                    board.screen,
                    row,
                    left + placement.column,
                    label,
                    curses.A_REVERSE | curses.A_BOLD,
                )
        else:
            draw_text(board.screen, 0, 0, "Links (Up/Down to scroll)")
            for row, hint in enumerate(hints[first_hint : first_hint + height - 1], 1):
                label = " ".join(hint.link.label.split())
                draw_text(
                    board.screen, row, 0, str(hint.number) + ": " + label + " -> " + hint.link.href
                )
        draw_text(
            board.screen, height, 0, "Follow link: " + digits + "  (number + Enter, Esc cancels)"
        )
        board.screen.refresh()
        key = board.getch()
        if isinstance(key, NoUpdate):
            continue
        if key == Key(curses.KEY_RESIZE):
            return key
        if key in (Key(27), Key(3)):
            return None
        if not placements and key in (Key(curses.KEY_UP), Key(curses.KEY_DOWN)):
            first_hint = max(
                0, min(len(hints) - 1, first_hint + (-1 if key == Key(curses.KEY_UP) else 1))
            )
        elif key in (Key(curses.KEY_BACKSPACE), Key(127), Key(8)):
            digits = digits[:-1]
        elif key in (Key(10), Key(13)):
            number = int(digits) if digits else 0
            if 1 <= number <= len(hints):
                return hints[number - 1].link
            digits = ""
        elif key.char.isascii() and key.char.isdigit() and len(digits) < 8:
            digits += key.char
