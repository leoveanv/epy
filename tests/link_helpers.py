"""Small ebook fixtures with explicit hyperlink destinations."""

import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence


@dataclass(frozen=True)
class LinkChapter:
    path: str = "chapter.xhtml"
    body: str = '<p id="start">First passage <a href="notes.xhtml#n"><sup>1</sup></a></p>'
    in_spine: bool = True


def build_link_book(path: Path, chapters: Sequence[LinkChapter] = ()) -> None:
    chapters = chapters or (
        LinkChapter(),
        LinkChapter(
            path="notes.xhtml",
            body='<p id="n">REFERENCE <a href="chapter.xhtml#start">return</a></p>',
        ),
    )
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
        for index, chapter in enumerate(chapters):
            manifest += (
                f'<item id="ch{index}" href="{chapter.path}" media-type="application/xhtml+xml"/>'
            )
            if chapter.in_spine:
                spine += f'<itemref idref="ch{index}"/>'
            archive.writestr(chapter.path, "<html><body>" + chapter.body + "</body></html>")
        archive.writestr(
            "book.opf",
            '<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Links</dc:title></metadata><manifest>'
            + manifest
            + "</manifest><spine>"
            + spine
            + "</spine></package>",
        )
        archive.writestr(
            "nav.xhtml",
            '<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><a href="'
            + chapters[0].path
            + '">First Chapter</a></nav></body></html>',
        )
