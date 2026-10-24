from pathlib import Path
from typing import Tuple

import pytest

from epy_reader.annotations import AnnotationBook, ExportLink, TextAnchor
from epy_reader.annotations.store import AnnotationStore


@pytest.fixture
def annotated_book(tmp_path: Path) -> Tuple[AnnotationStore, AnnotationBook]:
    source = tmp_path / "book.epub"
    source.write_bytes(b"book")
    store = AnnotationStore(tmp_path / "states.db")
    book = store.identify_book(source, "Book", "Author")
    store.create(
        book.id, TextAnchor(content_id="chapter", start=0, end=5, quote="quote"), note="my note"
    )
    return store, book


def test_export_preserves_handwritten_bytes(
    tmp_path: Path, annotated_book: Tuple[AnnotationStore, AnnotationBook]
) -> None:
    from epy_reader.annotations.export import export_book

    store, book = annotated_book
    destination = tmp_path / "My book.md"
    outside = b"---\r\ntags: [reading]\r\n---\r\n\r\nMy thoughts.\r\n"
    destination.write_bytes(outside)
    store.set_link(ExportLink(book_id=book.id, path=str(destination)))
    export_book(store, book)
    assert destination.read_bytes().startswith(outside)
    assert b"my note" in destination.read_bytes()


def test_repeated_export_is_idempotent(
    tmp_path: Path, annotated_book: Tuple[AnnotationStore, AnnotationBook]
) -> None:
    from epy_reader.annotations.export import export_book

    store, book = annotated_book
    destination = tmp_path / "book.md"
    destination.touch()
    store.set_link(ExportLink(book_id=book.id, path=str(destination)))
    export_book(store, book)
    original = destination.read_bytes()
    export_book(store, book)
    assert destination.read_bytes() == original


def test_note_edit_updates_only_managed_section(
    tmp_path: Path, annotated_book: Tuple[AnnotationStore, AnnotationBook]
) -> None:
    from epy_reader.annotations.export import export_book

    store, book = annotated_book
    destination = tmp_path / "book.md"
    destination.touch()
    store.set_link(ExportLink(book_id=book.id, path=str(destination)))
    export_book(store, book)
    destination.write_bytes(destination.read_bytes() + b"\nHandwritten suffix")
    store.update_note(store.list_annotations(book.id)[0], "changed")
    export_book(store, book)
    assert destination.read_bytes().endswith(b"\nHandwritten suffix")
    assert b"changed" in destination.read_bytes()
    assert b"my note" not in destination.read_bytes()


@pytest.mark.parametrize("mutation", ["edit", "missing", "duplicate", "reversed"])
def test_damaged_section_is_not_overwritten(
    tmp_path: Path, annotated_book: Tuple[AnnotationStore, AnnotationBook], mutation: str
) -> None:
    from epy_reader.annotations.export import ExportConflict, export_book

    store, book = annotated_book
    destination = tmp_path / "book.md"
    destination.touch()
    store.set_link(ExportLink(book_id=book.id, path=str(destination)))
    export_book(store, book)
    content = destination.read_bytes()
    if mutation == "edit":
        content = content.replace(b"my note", b"external edit")
    elif mutation == "missing":
        content = content.replace(b":end -->", b":removed -->")
    elif mutation == "reversed":
        start = ("<!-- epy:annotations:" + book.id + ":start -->").encode()
        end = ("<!-- epy:annotations:" + book.id + ":end -->").encode()
        content = end + start
    else:
        content += content
    destination.write_bytes(content)
    with pytest.raises(ExportConflict):
        export_book(store, book)
    assert destination.read_bytes() == content


def test_overwrite_still_rejects_reversed_markers(
    tmp_path: Path, annotated_book: Tuple[AnnotationStore, AnnotationBook]
) -> None:
    from epy_reader.annotations.export import ExportConflict, export_book

    store, book = annotated_book
    destination = tmp_path / "reversed.md"
    prefix = "<!-- epy:annotations:" + book.id
    destination.write_text(prefix + ":end -->" + prefix + ":start -->")
    store.set_link(ExportLink(book_id=book.id, path=str(destination)))
    with pytest.raises(ExportConflict):
        export_book(store, book, overwrite=True)


def test_deleted_destination_keeps_export_pending(
    tmp_path: Path, annotated_book: Tuple[AnnotationStore, AnnotationBook]
) -> None:
    from epy_reader.annotations.export import export_book

    store, book = annotated_book
    destination = tmp_path / "book.md"
    destination.touch()
    store.set_link(ExportLink(book_id=book.id, path=str(destination)))
    export_book(store, book)
    destination.unlink()
    store.update_note(store.list_annotations(book.id)[0], "saved")
    with pytest.raises(OSError):
        export_book(store, book)
    link = store.get_link(book.id)
    assert link.revision > link.exported_revision
    assert store.list_annotations(book.id)[0].note == "saved"


def test_export_rejects_marker_injection(
    tmp_path: Path, annotated_book: Tuple[AnnotationStore, AnnotationBook]
) -> None:
    from epy_reader.annotations.export import export_book

    store, book = annotated_book
    annotation = store.list_annotations(book.id)[0]
    store.update_note(annotation, "<!-- epy:annotations:" + book.id + ":end -->")
    destination = tmp_path / "book.md"
    destination.touch()
    store.set_link(ExportLink(book_id=book.id, path=str(destination)))
    export_book(store, book)
    assert destination.read_text().count("<!-- epy:annotations:" + book.id + ":end -->") == 1


def test_two_books_preserve_each_others_sections(
    tmp_path: Path, annotated_book: Tuple[AnnotationStore, AnnotationBook]
) -> None:
    from epy_reader.annotations.export import export_book

    store, book = annotated_book
    source = tmp_path / "second.epub"
    source.write_bytes(b"another")
    second = store.identify_book(source, "Other book", "")
    destination = tmp_path / "both.md"
    destination.touch()
    store.set_link(ExportLink(book_id=book.id, path=str(destination)))
    export_book(store, book)
    original = destination.read_bytes()
    store.set_link(ExportLink(book_id=second.id, path=str(destination)))
    export_book(store, second)
    assert destination.read_bytes().startswith(original)


def test_symlink_target_is_updated_without_replacing_link(
    tmp_path: Path, annotated_book: Tuple[AnnotationStore, AnnotationBook]
) -> None:
    from epy_reader.annotations.export import export_book, validate_destination

    store, book = annotated_book
    target = tmp_path / "target.md"
    target.touch()
    link = tmp_path / "link.md"
    link.symlink_to(target)
    store.set_link(ExportLink(book_id=book.id, path=str(validate_destination(str(link)))))
    export_book(store, book)
    assert link.is_symlink()
    assert b"quote" in target.read_bytes()
