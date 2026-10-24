"""Lazy kitten rendering with curses-owned placeholder cells and a bounded image cache."""

from __future__ import annotations

import curses
import sys
from collections import OrderedDict
from contextlib import ExitStack
from dataclasses import dataclass
from typing import Callable, Mapping, Optional
from uuid import uuid4

from epy_reader.board import InfiniBoard
from epy_reader.images.kitten import (
    ImagePreviewError,
    clear_image,
    render_image,
    split_image_output,
)
from epy_reader.images.models import (
    GraphicsSupport,
    ImageCells,
    ImagePlacement,
    ImageRequest,
    ImageSource,
    ImageTarget,
)
from epy_reader.images.terminal import redraw_safe_passthrough


@dataclass
class CachedImage:
    image_id: int
    color_pair: int
    cells: Optional[ImageCells] = None


class InlineImages:
    helper: str
    support: GraphicsSupport
    load_source: Callable[[ImageTarget], ImageSource]
    environment: Mapping[str, str]
    passthrough: ExitStack
    cache: OrderedDict[int, CachedImage]
    board: InfiniBoard
    targets: Mapping[int, str]
    image_rows: int

    def __init__(
        self,
        helper: str,
        support: GraphicsSupport,
        load_source: Callable[[ImageTarget], ImageSource],
        environment: Mapping[str, str],
    ) -> None:
        self.helper = helper
        self.support = support
        self.load_source = load_source
        self.cache = OrderedDict()
        self.passthrough = ExitStack()
        self.environment = environment

    def attach(self, board: InfiniBoard, targets: Mapping[int, str], image_rows: int) -> None:
        self.board = board
        self.targets = targets
        self.image_rows = image_rows
        board.image_rows = image_rows
        board.image_renderer = self.paint
        if self.support.in_tmux:
            self.passthrough.enter_context(redraw_safe_passthrough(self.environment))

    def paint(self, row: int, bottom_padding: int) -> None:
        height = self.board.screen_rows - bottom_padding
        for page in range(self.board.spread):
            start = row + page * height
            left = self.board.x if page == 0 else self.board.x_alt
            for image_row, path in self.targets.items():
                if image_row < start + height and image_row + self.image_rows > start:
                    image = self.get_image(ImageTarget(row=image_row, path=path))
                    self.paint_image(image, top=image_row - start, left=left, height=height)
        self.refresh()

    # NOTE: Batch combining-cell updates so tmux does not throttle partial placeholder redraws.
    def refresh(self) -> None:
        self.transmit("\x1b[?2026h")
        try:
            self.board.screen.refresh()
        finally:
            self.transmit("\x1b[?2026l")

    def paint_image(self, image: CachedImage, *, top: int, left: int, height: int) -> None:
        if image.cells is None:
            return
        lines = image.cells.lines
        width = lines[0].count("\U0010eeee")
        offset = (self.image_rows - len(lines)) // 2
        for line_row in range(max(0, -top), min(self.image_rows, height - top)):
            y = top + line_row
            try:
                self.board.screen.addstr(y, left, " " * self.board.textwidth, self.board.getbkgd())
                index = line_row - offset
                if 0 <= index < len(lines):
                    self.board.screen.addstr(
                        y,
                        left + (self.board.textwidth - width) // 2,
                        lines[index],
                        curses.color_pair(image.color_pair),
                    )
            except curses.error:
                pass

    def get_image(self, target: ImageTarget) -> CachedImage:
        if target.row in self.cache:
            self.cache.move_to_end(target.row)
            return self.cache[target.row]
        slot = 16 + len(self.cache)
        if len(self.cache) == 32:
            _, previous = self.cache.popitem(last=False)
            slot = previous.color_pair
            self.delete(previous)
        image = CachedImage(image_id=(uuid4().int & 0xFF000000) | slot, color_pair=slot)
        self.cache[target.row] = image
        try:
            source = self.load_source(target)
            output = render_image(
                self.helper,
                source,
                ImageRequest(
                    rows=self.board.screen_rows,
                    cols=self.board.screen_cols,
                    image_id=image.image_id,
                    in_tmux=self.support.in_tmux,
                    cell_width_px=self.support.cell_width_px,
                    cell_height_px=self.support.cell_height_px,
                    placement=ImagePlacement(rows=self.image_rows, cols=self.board.textwidth),
                ),
            )
            cells = split_image_output(output)
            _, background = curses.pair_content(curses.pair_number(self.board.getbkgd()))
            curses.init_pair(slot, slot, background)
            self.transmit(cells.transmission)
            image.cells = ImageCells(transmission="", lines=cells.lines)
        except (ImagePreviewError, OSError, KeyError, curses.error):
            pass
        return image

    def transmit(self, output: str) -> None:
        sys.stdout.write(output)
        sys.stdout.flush()

    def delete(self, image: CachedImage) -> None:
        if image.cells is not None:
            self.transmit(clear_image(image.image_id, in_tmux=self.support.in_tmux))

    def close(self) -> None:
        try:
            for image in self.cache.values():
                self.delete(image)
            self.cache.clear()
        finally:
            self.passthrough.close()
