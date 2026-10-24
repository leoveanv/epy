import shutil
import sqlite3
from pathlib import Path

import pytest

from epy_reader.parser import parse_html


def test_source_mapping_survives_rewrap() -> None:
    wide = parse_html("<p>same phrase then same phrase</p>", textwidth=80, content_id="chapter")
    narrow = parse_html("<p>same phrase then same phrase</p>", textwidth=9, content_id="chapter")
    assert wide.documents == narrow.documents
    assert "".join(cell.text for cell in narrow.source_cells if cell.start >= 17) == "samephrase"


def test_source_mapping_uses_terminal_cells() -> None:
    structure = parse_html("<p>界e\u0301 next</p>", textwidth=5, content_id="chapter")
    assert [(cell.text, cell.col, cell.width) for cell in structure.source_cells[:2]] == [
        ("界", 0, 2),
        ("e\u0301", 2, 1),
    ]


def test_repeated_passage_keeps_exact_anchor() -> None:
    from epy_reader.annotations import TextAnchor, resolve_anchor

    structure = parse_html("<p>same same</p>", textwidth=80, content_id="chapter")
    anchor = TextAnchor(content_id="chapter", start=5, end=9, quote="same")
    assert resolve_anchor(anchor, structure.documents[0]) == anchor


def test_anchor_quote_length_must_match_range() -> None:
    from epy_reader.annotations import TextAnchor

    with pytest.raises(ValueError):
        TextAnchor(content_id="chapter", start=0, end=2, quote="different")


def test_mismatched_anchor_stays_unresolved() -> None:
    from epy_reader.annotations import TextAnchor, resolve_anchor

    structure = parse_html("<p>same same</p>", textwidth=80, content_id="chapter")
    anchor = TextAnchor(content_id="chapter", start=0, end=4, quote="lost")
    assert resolve_anchor(anchor, structure.documents[0]) is None


def test_storage_follows_moved_book(tmp_path: Path) -> None:
    from epy_reader.annotations.store import AnnotationStore

    source = tmp_path / "book.epub"
    source.write_bytes(b"book content")
    store = AnnotationStore(tmp_path / "state.db")
    original = store.identify_book(source, "Title", "Author")
    moved = tmp_path / "moved.epub"
    shutil.move(source, moved)
    assert store.identify_book(moved, "Title", "Author").id == original.id


def test_changed_book_does_not_inherit_annotations(tmp_path: Path) -> None:
    from epy_reader.annotations.store import AnnotationStore

    source = tmp_path / "book.epub"
    source.write_bytes(b"old")
    store = AnnotationStore(tmp_path / "state.db")
    original = store.identify_book(source, "Title", "")
    source.write_bytes(b"new")
    assert store.identify_book(source, "Title", "").id != original.id


def test_annotation_reopens_after_history_removal(tmp_path: Path) -> None:
    from epy_reader.annotations import TextAnchor
    from epy_reader.annotations.store import AnnotationStore

    source = tmp_path / "book.epub"
    source.write_bytes(b"book")
    database = tmp_path / "state.db"
    store = AnnotationStore(database)
    book = store.identify_book(source, "Title", "")
    saved = store.create(book.id, TextAnchor(content_id="chapter", start=0, end=4, quote="text"))
    with sqlite3.connect(database) as connection:
        connection.execute("CREATE TABLE reading_states (filepath TEXT PRIMARY KEY)")
        connection.execute("INSERT INTO reading_states VALUES (?)", (str(source),))
        connection.execute("DELETE FROM reading_states")
    assert AnnotationStore(database).list_annotations(book.id) == [saved]


def test_migration_preserves_existing_data(tmp_path: Path) -> None:
    from epy_reader.annotations.store import AnnotationStore

    database = tmp_path / "states.db"
    with sqlite3.connect(database) as connection:
        connection.execute("CREATE TABLE bookmarks (name TEXT)")
        connection.execute("INSERT INTO bookmarks VALUES (?)", ("kept",))
    AnnotationStore(database)
    with sqlite3.connect(database) as connection:
        assert connection.execute("SELECT name FROM bookmarks").fetchall() == [("kept",)]
    assert Path(str(database) + ".pre-annotations.bak").exists()


def test_seamless_mapping_retains_document_identity() -> None:
    from epy_reader.utils import merge_text_structures

    first = parse_html("<p>first</p>", textwidth=20, content_id="one")
    second = parse_html("<p>second</p>", textwidth=20, content_id="two", starting_line=3)
    merged = merge_text_structures(first, second)
    assert [
        (cell.content_id, cell.row) for cell in merged.source_cells if cell.text in ("f", "s")
    ] == [("one", 0), ("one", 0), ("two", 3)]
