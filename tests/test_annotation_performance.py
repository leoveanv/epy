from time import perf_counter

from epy_reader.annotations.text import clip_text, wrap_text
from epy_reader.annotations.view import EditorState


def test_clipping_long_text_stays_within_interactive_budget() -> None:
    text = "words " * 20000
    started = perf_counter()
    for _ in range(200):
        clipped = clip_text(text, 80)
    elapsed = perf_counter() - started
    assert clipped == text[:80]
    assert elapsed < 0.3


def test_wrapping_long_unicode_note_stays_within_interactive_budget() -> None:
    text = "café words " * 5000
    started = perf_counter()
    lines = wrap_text(text, 70)
    elapsed = perf_counter() - started
    assert " ".join(lines).split() == text.split()
    assert elapsed < 0.5


def test_repeated_backspace_stays_within_interactive_budget() -> None:
    text = "word " * 4000
    state = EditorState(text=text, cursor=len(text))
    started = perf_counter()
    for _ in range(1000):
        state.handle("backspace")
    elapsed = perf_counter() - started
    assert state.text == text[:-1000]
    assert elapsed < 0.3
