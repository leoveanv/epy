import curses

import pytest


@pytest.mark.parametrize(
    "sequence,expected",
    [
        ("[105;5u", "ctrl_i"),
        ("[111;5u", 15),
        ("[117;5u", 21),
        ("[100;5u", 4),
        ("[27;5;105~", "ctrl_i"),
        ("[9u", 9),
        ("[57352u", curses.KEY_UP),
        ("[13u", 10),
        ("[27u", 27),
        ("[65;2u", ord("A")),
        ("[1;2A", curses.KEY_UP),
        ("[105;5:3u", -1),
        ("[?1u", -1),
    ],
)
def test_extended_keys_decode_without_tab_alias(sequence: str, expected: object) -> None:
    from epy_reader.keyboard import CTRL_I, decode_sequence

    assert decode_sequence(sequence) == (CTRL_I if expected == "ctrl_i" else expected)
