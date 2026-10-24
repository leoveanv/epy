"""SQLite annotation ownership is independent of disposable reading history."""

import hashlib
import sqlite3
from contextlib import closing
from datetime import datetime, timezone
from pathlib import Path
from typing import List, Optional
from uuid import uuid4

from epy_reader.annotations.models import (
    Annotation,
    AnnotationBook,
    AnnotationReference,
    ExportLink,
    TextAnchor,
)


class AnnotationStore:
    path: Path

    def __init__(self, path: Path) -> None:
        self.path = Path(path)
        self.migrate()

    def connect(self) -> sqlite3.Connection:
        connection = sqlite3.connect(str(self.path), timeout=10)
        connection.row_factory = sqlite3.Row
        connection.execute("PRAGMA foreign_keys = ON")
        return connection

    def migrate(self) -> None:
        existed = self.path.exists()
        with closing(self.connect()) as connection:
            version = connection.execute("PRAGMA user_version").fetchone()[0]
            if version > 1:
                raise ValueError("This annotation database requires a newer epy.")
            if version == 1:
                return
            backup = Path(str(self.path) + ".pre-annotations.bak")
            if existed and not backup.exists():
                with closing(sqlite3.connect(str(backup))) as destination:
                    connection.backup(destination)
            connection.executescript("""
                BEGIN IMMEDIATE;
                CREATE TABLE IF NOT EXISTS annotation_books (
                    id TEXT PRIMARY KEY, fingerprint TEXT UNIQUE NOT NULL,
                    title TEXT NOT NULL, author TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS annotation_locations (
                    path TEXT PRIMARY KEY, book_id TEXT NOT NULL REFERENCES annotation_books(id)
                );
                CREATE TABLE IF NOT EXISTS annotations (
                    id TEXT PRIMARY KEY, book_id TEXT NOT NULL REFERENCES annotation_books(id),
                    content_order INTEGER NOT NULL, start INTEGER NOT NULL, payload TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS annotations_book_order
                    ON annotations(book_id, content_order, start, id);
                CREATE TABLE IF NOT EXISTS annotation_exports (
                    book_id TEXT PRIMARY KEY REFERENCES annotation_books(id),
                    path TEXT NOT NULL, digest TEXT, revision INTEGER NOT NULL DEFAULT 0,
                    exported_revision INTEGER NOT NULL DEFAULT -1
                );
                PRAGMA user_version = 1;
                COMMIT;
            """)

    def identify_book(self, path: Path, title: str, author: str) -> AnnotationBook:
        fingerprint = hashlib.sha256()
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                fingerprint.update(chunk)
        with closing(self.connect()) as connection, connection:
            row = connection.execute(
                "SELECT * FROM annotation_books WHERE fingerprint=?", (fingerprint.hexdigest(),)
            ).fetchone()
            book = (
                AnnotationBook(**dict(row))
                if row
                else AnnotationBook(
                    id=str(uuid4()), fingerprint=fingerprint.hexdigest(), title=title, author=author
                )
            )
            connection.execute(
                "INSERT OR IGNORE INTO annotation_books VALUES (?, ?, ?, ?)",
                (book.id, book.fingerprint, book.title, book.author),
            )
            connection.execute(
                "INSERT OR REPLACE INTO annotation_locations VALUES (?, ?)",
                (str(path.resolve()), book.id),
            )
            return book

    def list_books(self) -> List[AnnotationBook]:
        with closing(self.connect()) as connection:
            return [
                AnnotationBook(**dict(row))
                for row in connection.execute("SELECT * FROM annotation_books ORDER BY title, id")
            ]

    def list_annotations(self, book_id: str) -> List[Annotation]:
        with closing(self.connect()) as connection:
            return [
                Annotation.model_validate_json(row[0])
                for row in connection.execute(
                    "SELECT payload FROM annotations WHERE book_id=? ORDER BY content_order, start, id",
                    (book_id,),
                )
            ]

    def create(
        self,
        book_id: str,
        anchor: TextAnchor,
        note: str = "",
        chapter: str = "",
        content_order: int = 0,
        reference: Optional[AnnotationReference] = None,
    ) -> Annotation:
        now = datetime.now(timezone.utc).isoformat()
        annotation = Annotation(
            id=str(uuid4()),
            book_id=book_id,
            anchor=anchor,
            note=note,
            chapter=chapter,
            reference=reference,
            content_order=content_order,
            created_at=now,
            updated_at=now,
        )
        self.save(annotation)
        return annotation

    def save(self, annotation: Annotation) -> None:
        with closing(self.connect()) as connection, connection:
            connection.execute(
                "INSERT OR REPLACE INTO annotations VALUES (?, ?, ?, ?, ?)",
                (
                    annotation.id,
                    annotation.book_id,
                    annotation.content_order,
                    annotation.anchor.start,
                    annotation.model_dump_json(),
                ),
            )
            connection.execute(
                "UPDATE annotation_exports SET revision=revision+1 WHERE book_id=?",
                (annotation.book_id,),
            )

    def update_note(self, annotation: Annotation, note: str) -> Annotation:
        updated = annotation.model_copy(
            update=dict(note=note, updated_at=datetime.now(timezone.utc).isoformat())
        )
        self.save(updated)
        return updated

    def delete(self, annotation: Annotation) -> None:
        with closing(self.connect()) as connection, connection:
            connection.execute("DELETE FROM annotations WHERE id=?", (annotation.id,))
            connection.execute(
                "UPDATE annotation_exports SET revision=revision+1 WHERE book_id=?",
                (annotation.book_id,),
            )

    def get_link(self, book_id: str) -> Optional[ExportLink]:
        with closing(self.connect()) as connection:
            row = connection.execute(
                "SELECT * FROM annotation_exports WHERE book_id=?", (book_id,)
            ).fetchone()
            return ExportLink(**dict(row)) if row else None

    def set_link(self, link: ExportLink) -> None:
        with closing(self.connect()) as connection, connection:
            connection.execute(
                "INSERT OR REPLACE INTO annotation_exports VALUES (?, ?, ?, ?, ?)",
                (link.book_id, link.path, link.digest, link.revision, link.exported_revision),
            )

    def unlink(self, book_id: str) -> None:
        with closing(self.connect()) as connection, connection:
            connection.execute("DELETE FROM annotation_exports WHERE book_id=?", (book_id,))

    def mark_exported(self, link: ExportLink, digest: str) -> None:
        with closing(self.connect()) as connection, connection:
            connection.execute(
                "UPDATE annotation_exports SET digest=?, exported_revision=? WHERE book_id=? AND path=?",
                (digest, link.revision, link.book_id, link.path),
            )
