"""EPUB fixtures with chapter-local image paths."""

import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence

from image_helpers import build_png


@dataclass(frozen=True)
class ImageChapter:
    title: str
    color: bytes = b"\xff\x00\x00"
    paragraphs: int = 2


def build_chapter_images(path: Path, chapters: Sequence[ImageChapter]) -> None:
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("mimetype", "application/epub+zip")
        archive.writestr(
            "META-INF/container.xml",
            '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="book.opf"/></rootfiles></container>',
        )
        manifest = (
            '<item id="nav" href="nav.xhtml" properties="nav" media-type="application/xhtml+xml"/>'
        )
        spine = ""
        links = ""
        for index, chapter in enumerate(chapters):
            filename = f"chapter{index}/text.xhtml"
            manifest += (
                f'<item id="ch{index}" href="{filename}" media-type="application/xhtml+xml"/>'
            )
            spine += f'<itemref idref="ch{index}"/>'
            links += f'<a href="{filename}">{chapter.title}</a>'
            body = f'<p>{chapter.title}</p><img src="figure.png"/>'
            body += "".join(
                f"<p>{chapter.title} paragraph {i}.</p>" for i in range(chapter.paragraphs)
            )
            archive.writestr(filename, "<html><body>" + body + "</body></html>")
            archive.writestr(f"chapter{index}/figure.png", build_png(rgb=chapter.color))
        archive.writestr(
            "book.opf",
            '<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Inline Test</dc:title></metadata><manifest>'
            + manifest
            + "</manifest><spine>"
            + spine
            + "</spine></package>",
        )
        archive.writestr(
            "nav.xhtml",
            '<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc">'
            + links
            + "</nav></body></html>",
        )
