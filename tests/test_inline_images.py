"""Image layout preserves source text while kitten supplies clipped image cells."""

import shutil
from dataclasses import replace

import pytest
from image_helpers import build_png

from epy_reader.images.kitten import render_image, split_image_output
from epy_reader.images.layout import ImageLayout
from epy_reader.images.models import ImagePlacement, ImageRequest, ImageSource
from epy_reader.models import TextStructure
from epy_reader.parser import parse_html


def build_structure() -> TextStructure:
    structure = parse_html(
        '<p>Before image.</p><img src="figure.png"/><p id="after"><b>After image.</b></p>',
        textwidth=40,
        section_ids={"after"},
        content_id="chapter.xhtml",
    )
    assert isinstance(structure, TextStructure)
    return structure


def test_image_space_preserves_canonical_text_and_source_mapping() -> None:
    original = build_structure()
    layout = ImageLayout.expand(original, image_rows=10)
    image_row = next(iter(original.image_maps))
    expected_cells = tuple(
        replace(cell, row=cell.row + (9 if cell.row > image_row else 0))
        for cell in original.source_cells
    )
    assert layout.structure.documents == original.documents
    assert layout.structure.source_cells == expected_cells
    assert layout.structure.section_rows["after"] == original.section_rows["after"] + 9
    assert layout.structure.formatting[-1].row == original.formatting[-1].row + 9


def test_expanded_rows_round_trip_to_original_reading_positions() -> None:
    original = build_structure()
    layout = ImageLayout.expand(original, image_rows=10)
    assert [
        layout.original_row(layout.display_row(row)) for row in range(len(original.text_lines))
    ] == list(range(len(original.text_lines)))
    image_row = next(iter(original.image_maps))
    assert layout.original_row(image_row + 8) == image_row


@pytest.mark.parametrize("in_tmux", [False, True])
def test_kitten_cells_keep_image_coordinates_for_partial_scrolling(in_tmux: bool) -> None:
    helper = shutil.which("kitten")
    if helper is None:
        pytest.skip("standalone kitten required")
    output = render_image(
        helper,
        ImageSource(name="figure.png", data=build_png(80, 40)),
        ImageRequest(
            rows=40,
            cols=80,
            image_id=0x01000010,
            in_tmux=in_tmux,
            placement=ImagePlacement(rows=10, cols=40),
        ),
    )
    rendered = split_image_output(output)
    assert len(rendered.lines) == 10
    assert all(line.count("\U0010eeee") == 40 for line in rendered.lines)
    assert rendered.lines[0] != rendered.lines[1]
    assert "\x1b_G" in rendered.transmission or "\x1b\x1b_G" in rendered.transmission
    assert all("\x1b" not in line for line in rendered.lines)
