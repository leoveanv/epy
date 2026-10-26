"""Book positions stay independent of terminal geometry."""

from pydantic import BaseModel, ConfigDict


class LinkRecord(BaseModel):
    model_config = ConfigDict(frozen=True)


class ContentPosition(LinkRecord):
    content_id: str
    offset: int = 0
    screen_row: int = 0
    image_offset: int = 0
    image_path: str = ""


class TextLink(LinkRecord):
    content_id: str
    start: int
    end: int
    href: str
    label: str


class DocumentAnchor(LinkRecord):
    content_id: str
    fragment: str
    offset: int


class CitationRange(LinkRecord):
    start: int
    end: int
    label: str


class LinkTarget(LinkRecord):
    content_id: str = ""
    fragment: str = ""
    url: str = ""
