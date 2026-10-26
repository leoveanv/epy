import curses
import re
from typing import Callable, Optional, Tuple, Union, cast

from epy_reader.annotations.text import cell_width
from epy_reader.models import Direction, InlineStyle, Key, NoUpdate
from epy_reader.settings import DoubleSpreadPadding
from epy_reader.keyboard import read_code


class InfiniBoard:
    """
    Wrapper for curses screen to render infinite texts.
    The idea is instead of pre render all the text before reading,
    this will only renders part of text on demand by which available
    page on screen.

    And what this does is only drawing text/string on curses screen
    without .clear() or .refresh() to optimize performance.
    Inline graphics may flush a synchronized redraw through the image renderer.
    """

    screen: curses.window
    screen_rows: int
    screen_cols: int
    textwidth: int
    x: int
    x_alt: int
    text: Tuple[str, ...]
    total_lines: int
    default_style: Tuple[InlineStyle, ...]
    temporary_style: Tuple[InlineStyle, ...]
    spread: int
    annotation_renderer: Optional[Callable[[int], None]]
    image_renderer: Optional[Callable[[int, int], None]]
    image_rows: int

    def __init__(
        self,
        screen: curses.window,
        text: Tuple[str, ...],
        textwidth: int = 80,
        default_style: Tuple[InlineStyle, ...] = tuple(),
        spread: int = 1,
    ) -> None:
        self.screen = screen
        self.screen_rows, self.screen_cols = self.screen.getmaxyx()
        self.textwidth = textwidth
        self.x = ((self.screen_cols - self.textwidth) // 2) + 1
        self.text = text
        self.total_lines = len(text)
        self.default_style = default_style
        self.temporary_style = ()
        self.annotation_renderer = None
        self.image_renderer = None
        self.image_rows = 1
        self.spread = spread

        if self.spread == 2:
            self.x = DoubleSpreadPadding.LEFT.value
            self.x_alt = (
                DoubleSpreadPadding.LEFT.value + self.textwidth + DoubleSpreadPadding.MIDDLE.value
            )

    def feed_temporary_style(self, styles: Optional[Tuple[InlineStyle, ...]] = None) -> None:
        """Reset styling if `styles` is None"""
        self.temporary_style = styles if styles else ()

    def render_styles(
        self, row: int, styles: Tuple[InlineStyle, ...] = (), bottom_padding: int = 0
    ) -> None:
        for i in styles:
            line = self.text[i.row]
            col = cell_width(line[: i.col])
            width = cell_width(line[i.col : i.col + i.n_letters])
            if i.row in range(row, row + self.screen_rows - bottom_padding):
                self.chgat(row, i.row, col, width, self.getbkgd() | i.attr)

            if self.spread == 2 and i.row in range(
                row + self.screen_rows - bottom_padding,
                row + 2 * (self.screen_rows - bottom_padding),
            ):
                self.chgat(
                    row,
                    i.row - (self.screen_rows - bottom_padding),
                    -self.x + self.x_alt + col,
                    width,
                    self.getbkgd() | i.attr,
                )

    def getch(self) -> Union[NoUpdate, Key]:
        input = read_code(self.screen)
        if input == -1:
            return NoUpdate()
        return Key(input)

    def getbkgd(self) -> int:
        # NOTE: Curses returns an attribute integer; some stub releases incorrectly declare a tuple.
        return cast(int, self.screen.getbkgd())

    def chgat(self, row: int, y: int, x: int, n: int, attr: int) -> None:
        for offset in range(n):
            previous = self.screen.inch(y - row, self.x + x + offset) & curses.A_ATTRIBUTES
            combined = (
                ((previous & ~curses.A_COLOR) | attr) if attr & curses.A_COLOR else previous | attr
            )
            self.screen.chgat(y - row, self.x + x + offset, 1, combined)

    def write(self, row: int, bottom_padding: int = 0) -> None:
        for n_row in range(min(self.screen_rows - bottom_padding, self.total_lines - row)):
            text_line = self.text[row + n_row]

            # NOTE: A bug with python itself: https://bugs.python.org/issue8243
            # It's stated in python docs:
            # > Attempting to write to the lower right corner of a window, subwindow,
            # > or pad will cause an exception to be raised after the character is printed.
            # https://github.com/python/cpython/commit/ef5ce884a41c8553a7eff66ebace908c1dcc1f89#diff-cb5622768373b8c93cc8eee30dfb041108783bb419d9eaf205501989cea0049fR691-R692
            #
            # Since the exception is raised "after the character is printed"
            # then it seems to be safe to catch it.
            try:
                self.screen.addstr(n_row, self.x, text_line)
            except curses.error:
                pass

            if (
                self.spread == 2
                and row + self.screen_rows - bottom_padding + n_row < self.total_lines
            ):
                text_line = self.text[row + self.screen_rows - bottom_padding + n_row]
                # TODO: clean this up
                # NOTE: Image markers identify embedded numbered images in double-spread text.
                if re.search("\\[IMG:[0-9]+\\]", text_line):
                    self.screen.addstr(
                        n_row, self.x_alt, text_line.center(self.textwidth), curses.A_BOLD
                    )
                else:
                    self.screen.addstr(n_row, self.x_alt, text_line)

        self.render_styles(row, self.default_style, bottom_padding)
        if self.image_renderer is not None:
            self.image_renderer(row, bottom_padding)
        if self.annotation_renderer is not None:
            self.annotation_renderer(row)
        self.render_styles(row, self.temporary_style, bottom_padding)
        # self.screen.refresh()

    def write_n(
        self,
        row: int,
        n: int = 1,
        direction: Direction = Direction.FORWARD,
        bottom_padding: int = 0,
    ) -> None:
        assert n > 0
        for n_row in range(min(self.screen_rows - bottom_padding, self.total_lines - row)):
            text_line = self.text[row + n_row]
            if direction == Direction.FORWARD:
                # self.screen.addnstr(n_row, self.x + self.textwidth - n, self.text[row+n_row], n)
                # `+ " " * (self.textwidth - len(self.text[row + n_row]))` is workaround to
                # to prevent curses trace because not calling screen.clear()
                self.screen.addnstr(
                    n_row,
                    self.x + self.textwidth - n,
                    text_line + " " * (self.textwidth - len(text_line)),
                    n,
                )

                if (
                    self.spread == 2
                    and row + self.screen_rows - bottom_padding + n_row < self.total_lines
                ):
                    text_line_alt = self.text[row + n_row + self.screen_rows - bottom_padding]
                    self.screen.addnstr(
                        n_row,
                        self.x_alt + self.textwidth - n,
                        text_line_alt + " " * (self.textwidth - len(text_line_alt)),
                        n,
                    )

            else:
                if text_line[self.textwidth - n :]:
                    self.screen.addnstr(n_row, self.x, text_line[self.textwidth - n :], n)

                if (
                    self.spread == 2
                    and row + self.screen_rows - bottom_padding + n_row < self.total_lines
                ):
                    text_line_alt = self.text[row + n_row + self.screen_rows - bottom_padding]
                    self.screen.addnstr(
                        n_row,
                        self.x_alt,
                        text_line_alt[self.textwidth - n :],
                        n,
                    )
