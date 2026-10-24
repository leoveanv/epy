"""Persistent annotation records and pure anchor resolution."""

from typing import Optional

from pydantic import BaseModel, ConfigDict, model_validator

from epy_reader.annotations.text import SourceDocument


class FrozenRecord(BaseModel):
    model_config = ConfigDict(frozen=True)


class AnnotationBook(FrozenRecord):
    id: str
    fingerprint: str
    title: str
    author: str


class TextAnchor(FrozenRecord):
    content_id: str
    start: int
    end: int
    quote: str
    prefix: str = ""
    suffix: str = ""
    normalization_version: int = 1

    @model_validator(mode="after")
    def validate_range(self) -> "TextAnchor":
        if self.start < 0 or self.end <= self.start or self.end - self.start != len(self.quote):
            raise ValueError("An annotation must contain a nonempty text range.")
        return self


class Annotation(FrozenRecord):
    id: str
    book_id: str
    anchor: TextAnchor
    note: str = ""
    chapter: str = ""
    content_order: int = 0
    created_at: str
    updated_at: str


class ExportLink(FrozenRecord):
    book_id: str
    path: str
    digest: Optional[str] = None
    revision: int = 0
    exported_revision: int = -1


def resolve_anchor(anchor: TextAnchor, document: SourceDocument) -> Optional[TextAnchor]:
    if anchor.content_id != document.content_id:
        return None
    if anchor.normalization_version != document.normalization_version:
        return None
    if document.text[anchor.start : anchor.end] != anchor.quote:
        return None
    return anchor
