import copy
import curses
import dataclasses
import multiprocessing
import os
import re
import shlex
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import uuid
import xml.etree.ElementTree as ET
import webbrowser
from html import unescape
from typing import Dict, List, Mapping, Optional, Sequence, Tuple, Union, cast

import epy_reader.settings as settings
from epy_reader.annotations.input import notice
from epy_reader.annotations.ui import AnnotationController
from epy_reader.board import InfiniBoard
from epy_reader.config import Config
from epy_reader.ebooks import Azw, Ebook, Epub, Mobi
from epy_reader.images.inline import InlineImages
from epy_reader.images.layout import ImageLayout
from epy_reader.images.models import ImageSource, ImageTarget
from epy_reader.images.terminal import query_graphics_support
from epy_reader.images.ui import PreviewAction, select_image, show_preview
from epy_reader.lib import resolve_path
from epy_reader.keyboard import read_code, read_value
from epy_reader.links.models import ContentPosition, TextLink
from epy_reader.links.navigation import BookLinks, JumpHistory, capture_position, position_row
from epy_reader.links.ui import select_link
from epy_reader.models import (
    Direction,
    InlineStyle,
    Key,
    LettersCount,
    LoadedBookContent,
    NoUpdate,
    ReadingState,
    SearchData,
    TextStructure,
    TocEntry,
)
from epy_reader.parser import parse_html
from epy_reader.settings import DoubleSpreadPadding
from epy_reader.speakers import SpeakerBaseModel
from epy_reader.state import State
from epy_reader.utils import (
    choice_win,
    construct_relative_reading_state,
    construct_speaker,
    count_letters,
    count_letters_parallel,
    find_current_content_index,
    get_ebook_obj,
    merge_text_structures,
    pgend,
    safe_curs_set,
    text_win,
)

# TODO: to be deprecated
DEBUG = False


class Reader:
    is_color_supported: bool
    page_animation: Optional[Direction]
    show_reading_progress: bool
    reading_progress: Optional[float]
    search_data: Optional[SearchData]
    jump_list: Dict[str, ReadingState]
    _tts_speaker: Optional[SpeakerBaseModel]
    tts_support: bool
    is_speaking: bool
    _multiprocess_support: bool
    _process_counting_letter: Optional[multiprocessing.Process]
    letters_count: Optional[LettersCount]
    _ext_dict_app: Optional[str]
    _image_viewer: Optional[str]
    totlines_per_content: Tuple[int, ...]
    original_content_lines: Tuple[int, ...]
    image_layout: Optional[ImageLayout]
    inline_images: Optional[InlineImages]
    _inline_content_index: int
    links: BookLinks
    jump_history: JumpHistory
    pending_position: Optional[ContentPosition]
    jump_origin: Optional[ContentPosition]

    def __init__(self, screen: curses.window, ebook: Ebook, config: Config, state: State) -> None:

        self.image_layout = None
        self.inline_images = None
        self.original_content_lines = ()
        self.setting = config.setting
        self.keymap = config.keymap
        # to build help menu text
        self.keymap_user_dict = config.keymap_user_dict

        self.seamless = self.setting.SeamlessBetweenChapters

        # keys that will make
        # windows exit and return the said key
        self._win_keys = (
            # curses.KEY_RESIZE is a must
            (Key(curses.KEY_RESIZE),)
            + self.keymap.TableOfContents
            + self.keymap.Metadata
            + self.keymap.Help
        )

        # screen initialization
        self.screen = screen
        self.screen.keypad(True)
        safe_curs_set(0)
        if self.setting.MouseSupport:
            curses.mousemask(-1)
        # curses.mouseinterval(0)
        self.screen.clear()

        # screen color
        self.is_color_supported = False
        try:
            curses.use_default_colors()
            curses.init_pair(1, self.setting.DefaultColorFG, self.setting.DefaultColorBG)
            curses.init_pair(2, self.setting.DarkColorFG, self.setting.DarkColorBG)
            curses.init_pair(3, self.setting.LightColorFG, self.setting.LightColorBG)
            self.screen.bkgd(curses.color_pair(1))
            self.is_color_supported = True
        except (curses.error, ValueError):
            self.is_color_supported = False

        # show loader and start heavy resources processes
        self.show_loader(subtext="initalizing ebook")

        # main ebook object
        self.ebook = ebook
        try:
            self.ebook.initialize()
        except (KeyboardInterrupt, Exception) as e:
            self.ebook.cleanup()
            if DEBUG:
                raise e
            else:
                sys.exit("ERROR: Badly-structured ebook.\n" + str(e))

        self.links = BookLinks(ebook)
        self.links.include_supplemental()
        self.jump_history = JumpHistory()
        self.pending_position = None
        self.jump_origin = None

        # state
        self.state = state
        self.annotations = AnnotationController(screen, ebook, config, state.filepath)

        # page scroll animation
        self.page_animation = None

        # show reading progress
        self.show_reading_progress = self.setting.ShowProgressIndicator
        self.reading_progress = None  # calculate after count_letters()

        # search storage
        self.search_data = None

        # double spread
        self.spread = 2 if self.setting.StartWithDoubleSpread else 1

        # jumps marker container
        self.jump_list = dict()

        # TTS speaker utils
        self._tts_speaker = construct_speaker(
            self.setting.PreferredTTSEngine, self.setting.TTSEngineArgs
        )
        self.tts_support = bool(self._tts_speaker)
        self.is_speaking = False

        # multi process & progress percentage
        self._multiprocess_support = False if multiprocessing.cpu_count() == 1 else True
        self._process_counting_letter = None
        self.letters_count = None

    def run_counting_letters(self):
        if self._multiprocess_support:
            try:
                self._proc_parent, self._proc_child = multiprocessing.Pipe()
                self._process_counting_letter = multiprocessing.Process(
                    name="epy-subprocess-counting-letters",
                    target=count_letters_parallel,
                    args=(copy.deepcopy(self.ebook), self._proc_child),
                )
                # forking will raise
                # zlib.error: Error -3 while decompressing data: invalid distance too far back
                self._process_counting_letter.start()
            except Exception as e:
                if DEBUG:
                    raise e
                self._multiprocess_support = False
        if not self._multiprocess_support:
            self.letters_count = count_letters(self.ebook)

    def try_assign_letters_count(self, *, force_wait=False) -> None:
        if isinstance(self._process_counting_letter, multiprocessing.Process):
            if force_wait and self._process_counting_letter.is_alive():
                self._process_counting_letter.join()

            if self._process_counting_letter.exitcode == 0:
                self.letters_count = self._proc_parent.recv()
                self._proc_parent.close()
                self._process_counting_letter.terminate()
                self._process_counting_letter.close()
                self._process_counting_letter = None

    def calculate_reading_progress(
        self, letters_per_content: List[int], reading_state: ReadingState
    ) -> None:
        if self.letters_count:
            self.reading_progress = (
                self.letters_count.cumulative[reading_state.content_index]
                + sum(
                    letters_per_content[: reading_state.row + (self.screen_rows * self.spread) - 1]
                )
            ) / self.letters_count.all

    @property
    def screen_rows(self) -> int:
        return self.screen.getmaxyx()[0]

    @property
    def screen_cols(self) -> int:
        return self.screen.getmaxyx()[1]

    @property
    def ext_dict_app(self) -> Optional[str]:
        self._ext_dict_app = None

        if shutil.which(self.setting.DictionaryClient.split()[0]):
            self._ext_dict_app = self.setting.DictionaryClient
        else:
            for i in settings.DICT_PRESET_LIST:
                if shutil.which(i) is not None:
                    self._ext_dict_app = i
                    break
            if self._ext_dict_app in {"sdcv"}:
                self._ext_dict_app += " -n"

        return self._ext_dict_app

    @property
    def image_viewer(self) -> Optional[str]:
        self._image_viewer = None

        if shutil.which(self.setting.DefaultViewer.split()[0]) is not None:
            self._image_viewer = self.setting.DefaultViewer
        elif sys.platform == "win32":
            self._image_viewer = "start"
        elif sys.platform == "darwin":
            self._image_viewer = "open"
        else:
            for i in settings.VIEWER_PRESET_LIST:
                if shutil.which(i) is not None:
                    self._image_viewer = i
                    break

        if self._image_viewer in {"gio"}:
            self._image_viewer += " open"

        return self._image_viewer

    def open_image(self, board: InfiniBoard, name: str, data: bytes) -> Union[NoUpdate, Key]:
        action = show_preview(self.screen, ImageSource(name=name, data=data), self.keymap)
        if action == PreviewAction.EXTERNAL:
            self.open_external_image(name, data)
        if self.screen.getmaxyx() != (board.screen_rows, board.screen_cols):
            return Key(curses.KEY_RESIZE)
        return NoUpdate()

    def open_external_image(self, name: str, data: bytes) -> None:
        viewer = self.image_viewer
        if viewer is None:
            notice(self.screen, "No external image viewer was found.")
            return
        descriptor, path = tempfile.mkstemp(suffix=os.path.splitext(name)[1])
        try:
            with os.fdopen(descriptor, "wb") as temporary:
                temporary.write(data)
            if viewer == "start" and sys.platform == "win32":
                getattr(os, "startfile")(path)
            else:
                result = subprocess.run(
                    shlex.split(viewer) + [path],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
                if result.returncode:
                    raise OSError("External viewer exited with status " + str(result.returncode))
            notice(self.screen, "Opened externally: " + name)
        finally:
            os.remove(path)

    def show_images(
        self, board: InfiniBoard, image_maps: Mapping[int, str], reading_state: ReadingState
    ) -> Union[NoUpdate, Key]:
        target = select_image(board, image_maps, reading_state.row, self.keymap)
        if not isinstance(target, ImageTarget):
            return target
        image_path = target.path
        if isinstance(self.ebook, (Epub, Mobi, Azw)):
            target_state = dataclasses.replace(reading_state, row=target.row)
            content_index = (
                self.convert_absolute_reading_state_to_relative(target_state).content_index
                if self.seamless
                else reading_state.content_index
            )
            content_path = self.ebook.contents[content_index]
            assert isinstance(content_path, str)
            image_path = resolve_path(content_path, image_path)
        name, data = self.ebook.get_img_bytestr(image_path)
        return self.open_image(board, name, data)

    def show_loader(self, *, loader_str: str = "\u231b", subtext: Optional[str] = None):
        self.screen.clear()
        rows, cols = self.screen.getmaxyx()
        middle_row = (rows - 1) // 2
        self.screen.addstr(middle_row, 0, loader_str.center(cols))
        if subtext:
            self.screen.addstr(middle_row + 1, 0, subtext.center(cols))
        # self.screen.addstr(((rows-2)//2)+1, (cols-len(msg))//2, msg)
        self.screen.refresh()

    @choice_win(True)
    def show_win_options(self, title, options, active_index, key_set):
        return title, options, active_index, key_set

    @text_win
    def show_win_error(self, title, msg, key):
        return title, msg, key

    @choice_win()
    def toc(self, toc_entries: Tuple[TocEntry, ...], index: int):
        return (
            "Table of Contents",
            [i.label for i in toc_entries],
            index,
            self.keymap.TableOfContents,
        )

    @text_win
    def show_win_metadata(self):
        if os.path.isfile(self.ebook.path):
            mdata = "[File Info]\nPATH: {}\nSIZE: {} MB\n \n[Book Info]\n".format(
                self.ebook.path, round(os.path.getsize(self.ebook.path) / 1024**2, 2)
            )
        else:
            mdata = "[File Info]\nPATH: {}\n \n[Book Info]\n".format(self.ebook.path)

        book_metadata = self.ebook.get_meta()
        for field in dataclasses.fields(book_metadata):
            value = getattr(book_metadata, field.name)
            if value:
                value = unescape(re.sub("<[^>]*>", "", value))
                mdata += f"{field.name.title()}: {value}\n"

        return "Metadata", mdata, self.keymap.Metadata

    @text_win
    def show_win_help(self):
        src = "Key Bindings:\n"
        labels = {
            name: value.replace("\x15", "Ctrl-U")
            .replace("\x04", "Ctrl-D")
            .replace("\x0f", "Ctrl-O")
            or ("Ctrl-I (extended input)" if name == "JumpForward" else "")
            for name, value in self.keymap_user_dict.items()
        }
        dig = max(len(label) for label in labels.values()) + 2
        for name, label in labels.items():
            # NOTE: This expression splits PascalCase action names into readable help labels.
            src += "{}  {}\n".format(label.rjust(dig), " ".join(re.findall("[A-Z][^A-Z]*", name)))
        return "Help", src, self.keymap.Help

    @text_win
    def define_word(self, word):
        rows, cols = self.screen.getmaxyx()
        hi, wi = 5, 16
        Y, X = (rows - hi) // 2, (cols - wi) // 2

        p = subprocess.Popen(
            "{} {}".format(self.ext_dict_app, word),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            shell=True,
        )

        dictwin = curses.newwin(hi, wi, Y, X)
        dictwin.box()
        dictwin.addstr((hi - 1) // 2, (wi - 10) // 2, "Loading...")
        dictwin.refresh()

        out, err = p.communicate()

        dictwin.clear()
        dictwin.refresh()

        if err == b"":
            return "Definition: " + word.upper(), out.decode(), self.keymap.DefineWord
        else:
            return "Error: " + self.ext_dict_app, err.decode(), self.keymap.DefineWord

    def show_win_choices_bookmarks(self):
        idx = 0
        while True:
            bookmarks = [i[0] for i in self.state.get_bookmarks(self.ebook)]
            if not bookmarks:
                return self.keymap.ShowBookmarks[0], None

            retk, idx, todel = self.show_win_options(
                "Bookmarks", bookmarks, idx, self.keymap.ShowBookmarks
            )
            if todel is not None:
                self.state.delete_bookmark(self.ebook, bookmarks[todel])
            else:
                return retk, idx

    def show_win_library(self):
        while True:
            library_items = self.state.get_from_history()
            if not library_items:
                return self.keymap.Library[0], None

            retk, choice_index, todel_index = self.show_win_options(
                "Library", [str(item) for item in library_items], 0, self.keymap.Library
            )
            if todel_index is not None:
                self.state.delete_from_library(library_items[todel_index].filepath)
            else:
                return retk, choice_index

    def input_prompt(self, prompt: str) -> Union[NoUpdate, Key, str]:
        """
        :param prompt: prompt text
        :return: NoUpdate if cancelled or interrupted
                 Key if curses.KEY_RESIZE triggered
                 str for successful input
        """
        # prevent pad hole when prompting for input while
        # other window is active
        # pad.refresh(y, 0, 0, x, rows-2, x+width)
        rows, cols = self.screen.getmaxyx()
        stat = curses.newwin(1, cols, rows - 1, 0)
        if self.is_color_supported:
            stat.bkgd(cast(int, self.screen.getbkgd()))
        stat.keypad(True)
        curses.echo(True)
        safe_curs_set(2)

        init_text = ""

        stat.addstr(0, 0, prompt, curses.A_REVERSE)
        stat.addstr(0, len(prompt), init_text)
        stat.refresh()

        try:
            while True:
                # NOTE: getch() only handles ascii
                # to handle wide char like: é, use get_wch()
                ipt = Key(read_value(stat))
                # get_wch() return ambiguous type
                # str for string input but int for function or special keys
                # if type(ipt) == str:
                #     ipt = ord(ipt)

                if ipt == Key(27):
                    stat.clear()
                    stat.refresh()
                    curses.echo(False)
                    safe_curs_set(0)
                    return NoUpdate()
                elif ipt == Key(10):
                    stat.clear()
                    stat.refresh()
                    curses.echo(False)
                    safe_curs_set(0)
                    return init_text if init_text else NoUpdate()
                elif ipt in (Key(8), Key(127), Key(curses.KEY_BACKSPACE)):
                    init_text = init_text[:-1]
                elif ipt == Key(curses.KEY_RESIZE):
                    stat.clear()
                    stat.refresh()
                    curses.echo(False)
                    safe_curs_set(0)
                    return Key(curses.KEY_RESIZE)
                # elif len(init_text) <= maxlen:
                else:
                    init_text += ipt.char

                stat.clear()
                stat.addstr(0, 0, prompt, curses.A_REVERSE)
                stat.addstr(
                    0,
                    len(prompt),
                    init_text
                    if len(prompt + init_text) < cols
                    else "..." + init_text[len(prompt) - cols + 4 :],
                )
                stat.refresh()
        except KeyboardInterrupt:
            stat.clear()
            stat.refresh()
            curses.echo(False)
            safe_curs_set(0)
            return NoUpdate()

    def searching(
        self, board: InfiniBoard, src: Sequence[str], reading_state: ReadingState, tot
    ) -> Union[NoUpdate, ReadingState, Key]:
        # reusable loop indices

        rows, cols = self.screen.getmaxyx()
        # unnecessary
        # if self.spread == 2:
        #     reading_state = dataclasses.replace(reading_state, textwidth=(cols - 7) // 2)

        if not self.search_data:
            candidate_text = self.input_prompt(" Regex:")
            if isinstance(candidate_text, str):
                self.search_data = SearchData(value=candidate_text)
            else:
                assert isinstance(candidate_text, NoUpdate) or isinstance(candidate_text, Key)
                return candidate_text

        found = []
        try:
            pattern = re.compile(self.search_data.value, re.IGNORECASE)
        except re.error as reerrmsg:
            self.search_data = None
            tmpk = self.show_win_error("!Regex Error", str(reerrmsg), tuple())
            return tmpk

        for n, text_line in enumerate(src):
            for j in pattern.finditer(text_line):
                found.append([n, j.span()[0], j.span()[1] - j.span()[0]])

        if not found:
            if (
                self.search_data.direction == Direction.FORWARD
                and reading_state.content_index + 1 < tot
            ):
                return ReadingState(
                    content_index=reading_state.content_index + 1,
                    textwidth=reading_state.textwidth,
                    row=0,
                )
            elif (
                self.search_data.direction == Direction.BACKWARD and reading_state.content_index > 0
            ):
                return ReadingState(
                    content_index=reading_state.content_index - 1,
                    textwidth=reading_state.textwidth,
                    row=0,
                )
            else:
                s = cast(Union[NoUpdate, Key], NoUpdate())
                while True:
                    if s in self.keymap.Quit:
                        self.search_data = None
                        self.screen.clear()
                        self.screen.refresh()
                        return reading_state
                    # TODO: maybe >= 0?
                    elif s == Key("n") and reading_state.content_index == 0:
                        self.search_data = dataclasses.replace(
                            self.search_data, direction=Direction.FORWARD
                        )
                        return ReadingState(
                            content_index=reading_state.content_index + 1,
                            textwidth=reading_state.textwidth,
                            row=0,
                        )
                    elif s == Key("N") and reading_state.content_index + 1 == tot:
                        self.search_data = dataclasses.replace(
                            self.search_data, direction=Direction.BACKWARD
                        )
                        return ReadingState(
                            content_index=reading_state.content_index - 1,
                            textwidth=reading_state.textwidth,
                            row=0,
                        )

                    self.screen.clear()
                    self.screen.addstr(
                        rows - 1,
                        0,
                        " Finished searching: " + self.search_data.value[: cols - 22] + " ",
                        curses.A_REVERSE,
                    )
                    board.write(reading_state.row, 1)
                    self.screen.refresh()
                    s = board.getch()

        sidx = len(found) - 1
        if self.search_data.direction == Direction.FORWARD:
            if reading_state.row > found[-1][0]:
                return ReadingState(
                    content_index=reading_state.content_index + 1,
                    textwidth=reading_state.textwidth,
                    row=0,
                )
            for n, i in enumerate(found):
                if i[0] >= reading_state.row:
                    sidx = n
                    break

        s = cast(Union[NoUpdate, Key], NoUpdate())
        msg = (
            " Searching: "
            + self.search_data.value
            + " --- Res {}/{} Ch {}/{} ".format(
                sidx + 1, len(found), reading_state.content_index + 1, tot
            )
        )
        while True:
            if s in self.keymap.Quit:
                self.search_data = None
                # for i in found:
                #     pad.chgat(i[0], i[1], i[2], pad.getbkgd())
                board.feed_temporary_style()
                # pad.format()
                # self.screen.clear()
                # self.screen.refresh()
                return reading_state
            elif s == Key("n"):
                self.search_data = dataclasses.replace(
                    self.search_data, direction=Direction.FORWARD
                )
                if sidx == len(found) - 1:
                    if reading_state.content_index + 1 < tot:
                        return ReadingState(
                            content_index=reading_state.content_index + 1,
                            textwidth=reading_state.textwidth,
                            row=0,
                        )
                    else:
                        s = NoUpdate()
                        msg = " Finished searching: " + self.search_data.value + " "
                        continue
                else:
                    sidx += 1
                    msg = (
                        " Searching: "
                        + self.search_data.value
                        + " --- Res {}/{} Ch {}/{} ".format(
                            sidx + 1, len(found), reading_state.content_index + 1, tot
                        )
                    )
            elif s == Key("N"):
                self.search_data = dataclasses.replace(
                    self.search_data, direction=Direction.BACKWARD
                )
                if sidx == 0:
                    if reading_state.content_index > 0:
                        return ReadingState(
                            content_index=reading_state.content_index - 1,
                            textwidth=reading_state.textwidth,
                            row=0,
                        )
                    else:
                        s = NoUpdate()
                        msg = " Finished searching: " + self.search_data.value + " "
                        continue
                else:
                    sidx -= 1
                    msg = (
                        " Searching: "
                        + self.search_data.value
                        + " --- Res {}/{} Ch {}/{} ".format(
                            sidx + 1, len(found), reading_state.content_index + 1, tot
                        )
                    )
            elif s == Key(curses.KEY_RESIZE):
                return Key(curses.KEY_RESIZE)

            # if reading_state.row + rows - 1 > pad.chunks[pad.find_chunkidx(reading_state.row)]:
            #     reading_state = dataclasses.replace(
            #         reading_state, row=pad.chunks[pad.find_chunkidx(reading_state.row)] + 1
            #     )

            while found[sidx][0] not in list(
                range(reading_state.row, reading_state.row + (rows - 1) * self.spread)
            ):
                if found[sidx][0] > reading_state.row:
                    reading_state = dataclasses.replace(
                        reading_state, row=reading_state.row + ((rows - 1) * self.spread)
                    )
                else:
                    reading_state = dataclasses.replace(
                        reading_state, row=reading_state.row - ((rows - 1) * self.spread)
                    )
                    if reading_state.row < 0:
                        reading_state = dataclasses.replace(reading_state, row=0)

            # formats = [InlineStyle(row=i[0], col=i[1], n_letters=i[2], attr=curses.A_REVERSE) for i in found]
            # pad.feed_style(formats)
            styles = []
            for n, i in enumerate(found):
                attr = curses.A_REVERSE if n == sidx else curses.A_NORMAL
                # pad.chgat(i[0], i[1], i[2], pad.getbkgd() | attr)
                styles.append(
                    InlineStyle(row=i[0], col=i[1], n_letters=i[2], attr=board.getbkgd() | attr)
                )
            board.feed_temporary_style(tuple(styles))

            self.screen.clear()
            self.screen.addstr(rows - 1, 0, msg, curses.A_REVERSE)
            self.screen.refresh()
            # pad.refresh(reading_state.row, 0, 0, x, rows - 2, x + reading_state.textwidth)
            board.write(reading_state.row, 1)
            s = board.getch()

    def speaking(self, text):
        self.is_speaking = True
        self.screen.addstr(self.screen_rows - 1, 0, " Speaking! ", curses.A_REVERSE)
        self.screen.refresh()
        self.screen.timeout(1)
        try:
            self._tts_speaker.speak(text)

            while True:
                if self._tts_speaker.is_done():
                    k = self.keymap.PageDown[0]
                    break
                tmp = read_code(self.screen, timeout_ms=1)
                k = NoUpdate() if tmp == -1 else Key(tmp)
                if k == Key(curses.KEY_MOUSE):
                    mouse_event = curses.getmouse()
                    if mouse_event[4] == curses.BUTTON2_CLICKED:
                        k = self.keymap.Quit[0]
                    elif mouse_event[4] == curses.BUTTON1_CLICKED:
                        if mouse_event[1] < self.screen_cols // 2:
                            k = self.keymap.PageUp[0]
                        else:
                            k = self.keymap.PageDown[0]
                    elif mouse_event[4] == curses.BUTTON4_PRESSED:
                        k = self.keymap.ScrollUp[0]
                    elif mouse_event[4] == 2097152:
                        k = self.keymap.ScrollDown[0]
                if (
                    k
                    in self.keymap.Quit
                    + self.keymap.PageUp
                    + self.keymap.PageDown
                    + self.keymap.ScrollUp
                    + self.keymap.ScrollDown
                    + self.keymap.HalfScreenUp
                    + self.keymap.HalfScreenDown
                    + (curses.KEY_RESIZE,)
                ):
                    self._tts_speaker.stop()
                    break
        finally:
            self.screen.timeout(-1)
            self._tts_speaker.cleanup()

        if k in self.keymap.Quit:
            self.is_speaking = False
            k = NoUpdate()
        return k

    def savestate(self, reading_state: ReadingState) -> None:
        reading_state = self.original_state(reading_state)
        if self.seamless:
            reading_state = construct_relative_reading_state(
                reading_state, self.original_content_lines or self.totlines_per_content
            )
        self.state.set_last_reading_state(self.ebook, reading_state)
        self.state.update_library(self.ebook, self.reading_progress)

    def cleanup(self) -> None:
        self.ebook.cleanup()

        if isinstance(self._process_counting_letter, multiprocessing.Process):
            if self._process_counting_letter.is_alive():
                self._process_counting_letter.terminate()
                # weird python multiprocessing issue, need to call .join() before .close()
                # ValueError: Cannot close a process while it is still running.
                # You should first call join() or terminate().
                self._process_counting_letter.join()
                self._process_counting_letter.close()

    def convert_absolute_reading_state_to_relative(self, reading_state) -> ReadingState:
        if not self.seamless:
            raise RuntimeError(
                "Reader.convert_absolute_reading_state_to_relative() only implemented when Seamless=True"
            )
        return construct_relative_reading_state(reading_state, self.totlines_per_content)

    def convert_relative_reading_state_to_absolute(
        self, reading_state: ReadingState
    ) -> ReadingState:
        if not self.seamless:
            raise RuntimeError(
                "Reader.convert_relative_reading_state_to_absolute() only implemented when Seamless=True"
            )

        absolute_row = reading_state.row + sum(
            self.totlines_per_content[: reading_state.content_index]
        )
        absolute_pctg = (
            absolute_row / sum(self.totlines_per_content) if reading_state.rel_pctg else None
        )

        return dataclasses.replace(
            reading_state, content_index=0, row=absolute_row, rel_pctg=absolute_pctg
        )

    def get_all_book_contents(self, reading_state: ReadingState) -> LoadedBookContent:
        if not self.seamless:
            raise RuntimeError("Reader.get_all_book_contents() only implemented when Seamless=True")

        contents = self.ebook.contents
        toc_entries = self.ebook.toc_entries

        text_structure = TextStructure(
            text_lines=tuple(), image_maps=dict(), section_rows=dict(), formatting=tuple()
        )
        toc_entries_tmp = []
        section_rows_tmp = dict()

        # self.totlines_per_content only defined when Seamless=True
        self.totlines_per_content = tuple()

        for n, content in enumerate(contents):
            self.show_loader(subtext=f"loading contents ({n + 1}/{len(contents)})")
            starting_line = sum(self.totlines_per_content)
            assert isinstance(content, str) or isinstance(content, ET.Element)
            text_structure_tmp = parse_html(
                self.ebook.get_raw_text(content),
                textwidth=reading_state.textwidth,
                section_ids=set(toc_entry.section for toc_entry in toc_entries),  # type: ignore
                starting_line=starting_line,
                content_id=str(content),
            )
            assert isinstance(text_structure_tmp, TextStructure)
            # self.totlines_per_content.append(len(text_structure_tmp.text_lines))
            self.totlines_per_content += (len(text_structure_tmp.text_lines),)

            for toc_entry in toc_entries:
                if toc_entry.content_index == n:
                    if toc_entry.section:
                        toc_entries_tmp.append(dataclasses.replace(toc_entry, content_index=0))
                    else:
                        section_id_tmp = str(uuid.uuid4())
                        toc_entries_tmp.append(
                            TocEntry(label=toc_entry.label, content_index=0, section=section_id_tmp)
                        )
                        section_rows_tmp[section_id_tmp] = starting_line

            text_structure = merge_text_structures(text_structure, text_structure_tmp)

        text_structure = dataclasses.replace(
            text_structure, section_rows={**text_structure.section_rows, **section_rows_tmp}
        )

        return LoadedBookContent(text_structure, tuple(toc_entries_tmp), (self.ebook.contents[0],))

    def get_current_book_content(self, reading_state: ReadingState) -> LoadedBookContent:
        contents = self.ebook.contents
        toc_entries = self.ebook.toc_entries
        content_path = contents[reading_state.content_index]
        content = self.ebook.get_raw_text(content_path)
        text_structure = parse_html(  # type: ignore
            content,
            textwidth=reading_state.textwidth,
            section_ids=set(toc_entry.section for toc_entry in toc_entries),  # type: ignore
            content_id=str(content_path),
        )
        assert isinstance(text_structure, TextStructure)
        return LoadedBookContent(text_structure, toc_entries, contents)

    def original_state(self, state: ReadingState) -> ReadingState:
        if self.image_layout is None:
            return state
        row = self.image_layout.original_row(state.row)
        total = len(self.image_layout.row_starts) - 1
        return dataclasses.replace(
            state,
            row=row,
            image_offset=state.row - self.image_layout.display_row(row),
            rel_pctg=row / total if state.rel_pctg is not None else None,
        )

    def display_state(self, state: ReadingState) -> ReadingState:
        if self.image_layout is None:
            return state
        original_row = state.row
        if state.rel_pctg is not None:
            original_row = round(state.rel_pctg * (len(self.image_layout.row_starts) - 1))
        row = self.image_layout.display_row(original_row) + min(
            state.image_offset, self.image_layout.image_rows - 1
        )
        return dataclasses.replace(state, row=row, image_offset=0, rel_pctg=None)

    def load_inline_source(self, target: ImageTarget) -> ImageSource:
        index = self._inline_content_index
        if self.seamless:
            index = construct_relative_reading_state(
                ReadingState(content_index=0, textwidth=80, row=target.row),
                self.totlines_per_content,
            ).content_index
        path = target.path
        if isinstance(self.ebook, (Epub, Mobi, Azw)):
            content = self.ebook.contents[index]
            assert isinstance(content, str)
            path = resolve_path(content, path)
        name, data = self.ebook.get_img_bytestr(path)
        return ImageSource(name=name, data=data)

    def viewport_position(self, structure: TextStructure, row: int) -> ContentPosition:
        return capture_position(
            structure, row, image_rows=self.image_layout.image_rows if self.image_layout else 0
        )

    def record_arrival(self, structure: TextStructure, reading_state: ReadingState) -> None:
        if self.jump_origin is not None:
            self.jump_history.record(
                self.jump_origin, self.viewport_position(structure, reading_state.row)
            )
            self.jump_origin = None

    def jump_to(
        self, position: ContentPosition, reading_state: ReadingState, structure: TextStructure
    ) -> ReadingState:
        if any(document.content_id == position.content_id for document in structure.documents):
            return dataclasses.replace(
                reading_state,
                row=position_row(
                    structure,
                    position,
                    image_rows=self.image_layout.image_rows if self.image_layout else 0,
                ),
                rel_pctg=None,
                section=None,
            )
        self.pending_position = position
        index = next(
            index
            for index, content in enumerate(self.ebook.contents)
            if str(content) == position.content_id
        )
        return ReadingState(content_index=index, textwidth=reading_state.textwidth, row=0)

    def follow_link(self, link: TextLink) -> Optional[ContentPosition]:
        target = self.links.resolve(link.content_id, link.href)
        if target.url:
            if not webbrowser.open(target.url):
                raise ValueError("The default browser could not open the link.")
            return None
        return self.links.position(target)

    def read(self, reading_state: ReadingState) -> Union[ReadingState, Ebook]:
        self.image_layout = None
        self.inline_images = None
        self.original_content_lines = ()
        self._inline_content_index = reading_state.content_index
        helper = shutil.which("kitten") if self.setting.InlineImages else None
        if helper and curses.COLORS >= 256 and curses.COLOR_PAIRS >= 48:
            support = query_graphics_support(os.environ, sys.stdout.fileno())
            if support.reason is None:
                self.inline_images = InlineImages(
                    helper, support, self.load_inline_source, os.environ
                )
        try:
            result = self.read_content(reading_state)
            if isinstance(result, ReadingState) and (
                self.seamless or result.content_index == reading_state.content_index
            ):
                result = self.original_state(result)
            return result
        finally:
            if self.inline_images is not None:
                self.inline_images.close()
            if self.seamless and self.original_content_lines:
                self.totlines_per_content = self.original_content_lines
            self.image_layout = None

    def read_content(self, reading_state: ReadingState) -> Union[ReadingState, Ebook]:
        # reusable loop indices

        k = self.keymap.RegexSearch[0] if self.search_data else NoUpdate()
        rows, cols = self.screen.getmaxyx()
        if self.spread == 1 and reading_state.textwidth > cols - 4:
            reading_state = dataclasses.replace(reading_state, textwidth=max(20, cols - 4))

        mincols_doublespr = (
            DoubleSpreadPadding.LEFT.value
            + 22
            + DoubleSpreadPadding.MIDDLE.value
            + 22
            + DoubleSpreadPadding.RIGHT.value
        )
        if cols < mincols_doublespr:
            self.spread = 1
        if self.spread == 2:
            reading_state = dataclasses.replace(
                reading_state,
                textwidth=(
                    cols
                    - sum(
                        [
                            DoubleSpreadPadding.LEFT.value,
                            DoubleSpreadPadding.MIDDLE.value,
                            DoubleSpreadPadding.RIGHT.value,
                        ]
                    )
                )
                // 2,
            )
        self.show_loader(subtext="loading contents")
        # get text structure, toc entries and contents of the book
        if self.seamless:
            loaded_content = self.get_all_book_contents(reading_state)
            # adjustment
            reading_state = self.convert_relative_reading_state_to_absolute(reading_state)
        else:
            loaded_content = self.get_current_book_content(reading_state)

        text_structure = loaded_content.text_structure
        toc_entries = loaded_content.toc_entries
        contents = loaded_content.contents
        totlines = len(text_structure.text_lines)

        backward_row = reading_state.row if reading_state.row < 0 else None
        if reading_state.row < 0 and totlines <= rows * self.spread:
            reading_state = dataclasses.replace(reading_state, row=0)
        elif reading_state.rel_pctg is not None:
            reading_state = dataclasses.replace(
                reading_state, row=round(reading_state.rel_pctg * totlines)
            )
        else:
            reading_state = dataclasses.replace(reading_state, row=reading_state.row % totlines)

        if self.inline_images is not None and text_structure.image_maps:
            self.image_layout = ImageLayout.expand(
                text_structure, image_rows=min(24, max(3, reading_state.textwidth // 4))
            )
            reading_state = self.display_state(reading_state)
            text_structure = self.image_layout.structure
            totlines = len(text_structure.text_lines)
            if backward_row is not None:
                reading_state = dataclasses.replace(
                    reading_state, row=max(0, totlines + backward_row)
                )
            if self.seamless:
                self.original_content_lines = self.totlines_per_content
                boundaries = [0]
                for length in self.original_content_lines:
                    boundaries.append(boundaries[-1] + length)
                self.totlines_per_content = tuple(
                    self.image_layout.display_row(end) - self.image_layout.display_row(start)
                    for start, end in zip(boundaries, boundaries[1:])
                )

        board = InfiniBoard(
            screen=self.screen,
            text=text_structure.text_lines,
            textwidth=reading_state.textwidth,
            default_style=text_structure.formatting,
            spread=self.spread,
        )

        letters_per_content = []
        for text_line in text_structure.text_lines:
            # NOTE: Reading progress counts non-whitespace characters in each rendered line.
            letters_per_content.append(len(re.sub(r"\s", "", text_line)))

        self.annotations.attach(board, text_structure)
        if self.inline_images is not None and self.image_layout is not None:
            self.inline_images.attach(
                board, text_structure.image_maps, self.image_layout.image_rows
            )
        reading_state = self.annotations.restore_jump(reading_state)

        self.screen.clear()
        self.screen.refresh()
        # try-except clause if there is issue
        # with curses resize event
        board.write(reading_state.row)

        # if reading_state.section is not None
        # then override reading_state.row to follow the section
        if reading_state.section:
            reading_state = dataclasses.replace(
                reading_state, row=text_structure.section_rows.get(reading_state.section, 0)
            )

        if self.pending_position is not None:
            reading_state = dataclasses.replace(
                reading_state,
                row=position_row(
                    text_structure,
                    self.pending_position,
                    image_rows=self.image_layout.image_rows if self.image_layout else 0,
                ),
                rel_pctg=None,
                section=None,
            )
            self.pending_position = None
        self.record_arrival(text_structure, reading_state)

        checkpoint_row = None
        countstring = ""

        try:
            while True:
                self.record_arrival(text_structure, reading_state)
                if k in (
                    self.keymap.Follow
                    + self.keymap.TableOfContents
                    + self.keymap.ShowBookmarks
                    + self.keymap.JumpToPosition
                    + self.keymap.AnnotationList
                    + self.keymap.RegexSearch
                ):
                    self.jump_origin = self.viewport_position(text_structure, reading_state.row)
                if countstring == "":
                    count = 1
                else:
                    count = int(countstring)
                if isinstance(k, Key) and k in tuple(Key(i) for i in range(48, 58)):
                    countstring = countstring + k.char
                else:
                    if isinstance(k, Key) and self.annotations.handles(k):
                        if self.spread == 2:
                            self.spread = 1
                            self.annotations.status = "Annotation mode uses single-column reading."
                            return dataclasses.replace(
                                reading_state, rel_pctg=reading_state.row / totlines
                            )
                        updated_state = self.annotations.handle(k, reading_state)
                        if (
                            updated_state.content_index != reading_state.content_index
                            or updated_state.textwidth != reading_state.textwidth
                            or self.annotations.pending_jump is not None
                            or self.screen.getmaxyx() != (rows, cols)
                        ):
                            return updated_state
                        reading_state = updated_state
                        k = NoUpdate()
                    if k in self.keymap.Quit:
                        if k == Key(27) and countstring != "":
                            countstring = ""
                        else:
                            self.try_assign_letters_count(force_wait=True)
                            self.calculate_reading_progress(letters_per_content, reading_state)

                            self.savestate(
                                dataclasses.replace(
                                    reading_state, rel_pctg=reading_state.row / totlines
                                )
                            )
                            sys.exit()

                    elif k in self.keymap.Follow:
                        selection = select_link(board, text_structure, reading_state.row)
                        if isinstance(selection, Key):
                            k = selection
                            continue
                        if isinstance(selection, TextLink):
                            try:
                                position = self.follow_link(selection)
                                if position is not None:
                                    return self.jump_to(position, reading_state, text_structure)
                            except (ValueError, OSError, KeyError, webbrowser.Error) as error:
                                notice(self.screen, str(error))

                    elif k in self.keymap.JumpBackward or k in self.keymap.JumpForward:
                        current = self.viewport_position(text_structure, reading_state.row)
                        move = (
                            self.jump_history.backward
                            if k in self.keymap.JumpBackward
                            else self.jump_history.forward
                        )
                        destination = None
                        for _ in range(count):
                            position = move(destination or current)
                            if position is None:
                                break
                            destination = position
                        if destination is not None:
                            return self.jump_to(destination, reading_state, text_structure)

                    elif k in self.keymap.TTSToggle and self.tts_support:
                        tospeak = ""
                        for spoken_line in text_structure.text_lines[
                            reading_state.row : reading_state.row + (rows * self.spread)
                        ]:
                            # NOTE: Blank lines insert spoken pauses between paragraphs.
                            if re.match(r"^\s*$", spoken_line) is not None:
                                tospeak += "\n. \n"
                            else:
                                tospeak += spoken_line + " "
                        k = self.speaking(tospeak)
                        if (
                            totlines - reading_state.row <= rows
                            and reading_state.content_index == len(contents) - 1
                        ):
                            self.is_speaking = False
                        continue

                    elif k in self.keymap.DoubleSpreadToggle:
                        if cols < mincols_doublespr:
                            k = self.show_win_error(
                                "Screen is too small",
                                "Min: {} cols x {} rows".format(mincols_doublespr, 12),
                                (Key("D"),),
                            )
                        self.spread = (self.spread % 2) + 1
                        return ReadingState(
                            content_index=reading_state.content_index,
                            textwidth=reading_state.textwidth,
                            row=reading_state.row,
                            rel_pctg=reading_state.row / totlines,
                        )

                    elif k in self.keymap.ScrollUp:
                        if self.spread == 2:
                            k = self.keymap.PageUp[0]
                            continue
                        if count > 1:
                            checkpoint_row = reading_state.row - 1
                        if reading_state.row >= count:
                            reading_state = dataclasses.replace(
                                reading_state, row=reading_state.row - count
                            )
                        elif reading_state.row == 0 and reading_state.content_index != 0:
                            self.page_animation = Direction.BACKWARD
                            # return -1, width, -rows, None, ""
                            return ReadingState(
                                content_index=reading_state.content_index - 1,
                                textwidth=reading_state.textwidth,
                                row=-rows,
                            )
                        else:
                            reading_state = dataclasses.replace(reading_state, row=0)

                    elif k in self.keymap.PageUp:
                        if reading_state.row == 0 and reading_state.content_index != 0:
                            self.page_animation = Direction.BACKWARD
                            text_structure_content_before = parse_html(
                                self.ebook.get_raw_text(contents[reading_state.content_index - 1]),
                                textwidth=reading_state.textwidth,
                            )
                            assert isinstance(text_structure_content_before, TextStructure)
                            return ReadingState(
                                content_index=reading_state.content_index - 1,
                                textwidth=reading_state.textwidth,
                                row=-rows * self.spread
                                if self.inline_images is not None
                                else rows
                                * self.spread
                                * (
                                    len(text_structure_content_before.text_lines)
                                    // (rows * self.spread)
                                ),
                            )
                        else:
                            if reading_state.row >= rows * self.spread * count:
                                self.page_animation = Direction.BACKWARD
                                reading_state = dataclasses.replace(
                                    reading_state,
                                    row=reading_state.row - (rows * self.spread * count),
                                )
                            else:
                                reading_state = dataclasses.replace(reading_state, row=0)

                    elif k in self.keymap.ScrollDown:
                        if self.spread == 2:
                            k = self.keymap.PageDown[0]
                            continue
                        if count > 1:
                            checkpoint_row = reading_state.row + rows - 1
                        if reading_state.row + count <= totlines - rows:
                            reading_state = dataclasses.replace(
                                reading_state, row=reading_state.row + count
                            )
                        elif (
                            reading_state.row >= totlines - rows
                            and reading_state.content_index != len(contents) - 1
                        ):
                            self.page_animation = Direction.FORWARD
                            return ReadingState(
                                content_index=reading_state.content_index + 1,
                                textwidth=reading_state.textwidth,
                                row=0,
                            )

                    elif k in self.keymap.PageDown:
                        if totlines - reading_state.row > rows * self.spread:
                            self.page_animation = Direction.FORWARD
                            reading_state = dataclasses.replace(
                                reading_state, row=reading_state.row + (rows * self.spread)
                            )
                        elif reading_state.content_index != len(contents) - 1:
                            self.page_animation = Direction.FORWARD
                            return ReadingState(
                                content_index=reading_state.content_index + 1,
                                textwidth=reading_state.textwidth,
                                row=0,
                            )

                    elif k in self.keymap.HalfScreenUp + self.keymap.HalfScreenDown:
                        down = k in self.keymap.HalfScreenDown
                        end = pgend(totlines, rows * self.spread)
                        if (
                            down
                            and reading_state.row >= end
                            and reading_state.content_index < len(contents) - 1
                        ):
                            return ReadingState(
                                content_index=reading_state.content_index + 1,
                                textwidth=reading_state.textwidth,
                                row=0,
                            )
                        if not down and reading_state.row == 0 and reading_state.content_index > 0:
                            return ReadingState(
                                content_index=reading_state.content_index - 1,
                                textwidth=reading_state.textwidth,
                                row=-rows * self.spread,
                            )
                        distance = max(1, rows * self.spread // 2) * count
                        reading_state = dataclasses.replace(
                            reading_state,
                            row=max(
                                0, min(end, reading_state.row + (distance if down else -distance))
                            ),
                        )

                    elif k in self.keymap.NextChapter:
                        ntoc = find_current_content_index(
                            toc_entries,
                            text_structure.section_rows,
                            reading_state.content_index,
                            reading_state.row,
                        )
                        if ntoc < len(toc_entries) - 1:
                            if reading_state.content_index == toc_entries[ntoc + 1].content_index:
                                try:
                                    reading_state = dataclasses.replace(
                                        reading_state,
                                        row=text_structure.section_rows[
                                            toc_entries[ntoc + 1].section  # type: ignore
                                        ],
                                    )
                                except KeyError:
                                    pass
                            else:
                                return ReadingState(
                                    content_index=toc_entries[ntoc + 1].content_index,
                                    textwidth=reading_state.textwidth,
                                    row=0,
                                    section=toc_entries[ntoc + 1].section,
                                )

                    elif k in self.keymap.PrevChapter:
                        ntoc = find_current_content_index(
                            toc_entries,
                            text_structure.section_rows,
                            reading_state.content_index,
                            reading_state.row,
                        )
                        if ntoc > 0:
                            if reading_state.content_index == toc_entries[ntoc - 1].content_index:
                                reading_state = dataclasses.replace(
                                    reading_state,
                                    row=text_structure.section_rows.get(
                                        toc_entries[ntoc - 1].section or "",
                                        0,
                                    ),
                                )
                            else:
                                return ReadingState(
                                    content_index=toc_entries[ntoc - 1].content_index,
                                    textwidth=reading_state.textwidth,
                                    row=0,
                                    section=toc_entries[ntoc - 1].section,
                                )

                    elif k in self.keymap.BeginningOfCh:
                        ntoc = find_current_content_index(
                            toc_entries,
                            text_structure.section_rows,
                            reading_state.content_index,
                            reading_state.row,
                        )
                        try:
                            reading_state = dataclasses.replace(
                                reading_state,
                                row=text_structure.section_rows[toc_entries[ntoc].section],  # type: ignore
                            )
                        except (KeyError, IndexError):
                            reading_state = dataclasses.replace(reading_state, row=0)

                    elif k in self.keymap.EndOfCh:
                        ntoc = find_current_content_index(
                            toc_entries,
                            text_structure.section_rows,
                            reading_state.content_index,
                            reading_state.row,
                        )
                        try:
                            if (
                                text_structure.section_rows[toc_entries[ntoc + 1].section] - rows  # type: ignore
                                >= 0
                            ):
                                reading_state = dataclasses.replace(
                                    reading_state,
                                    row=text_structure.section_rows[toc_entries[ntoc + 1].section]  # type: ignore
                                    - rows,
                                )
                            else:
                                reading_state = dataclasses.replace(
                                    reading_state,
                                    row=text_structure.section_rows[toc_entries[ntoc].section],  # type: ignore
                                )
                        except (KeyError, IndexError):
                            reading_state = dataclasses.replace(
                                reading_state, row=pgend(totlines, rows)
                            )

                    elif k in self.keymap.TableOfContents:
                        if not toc_entries:
                            k = self.show_win_error(
                                "Table of Contents",
                                "N/A: TableOfContents is unavailable for this book.",
                                self.keymap.TableOfContents,
                            )
                            continue
                        ntoc = find_current_content_index(
                            toc_entries,
                            text_structure.section_rows,
                            reading_state.content_index,
                            reading_state.row,
                        )
                        rettock, fllwd, _ = self.toc(toc_entries, ntoc)
                        if rettock is not None:  # and rettock in WINKEYS:
                            k = rettock
                            continue
                        elif fllwd is not None:
                            if reading_state.content_index == toc_entries[fllwd].content_index:
                                try:
                                    reading_state = dataclasses.replace(
                                        reading_state,
                                        row=text_structure.section_rows[toc_entries[fllwd].section],
                                    )
                                except KeyError:
                                    reading_state = dataclasses.replace(reading_state, row=0)
                            else:
                                return ReadingState(
                                    content_index=toc_entries[fllwd].content_index,
                                    textwidth=reading_state.textwidth,
                                    row=0,
                                    section=toc_entries[fllwd].section,
                                )

                    elif k in self.keymap.Metadata:
                        k = self.show_win_metadata()
                        if k in self._win_keys:
                            continue

                    elif k in self.keymap.Help:
                        k = self.show_win_help()
                        if k in self._win_keys:
                            continue

                    elif (
                        k in self.keymap.Enlarge
                        and (reading_state.textwidth + count) < cols - 4
                        and self.spread == 1
                    ):
                        return dataclasses.replace(
                            reading_state,
                            textwidth=reading_state.textwidth + count,
                            rel_pctg=reading_state.row / totlines,
                        )

                    elif (
                        k in self.keymap.Shrink
                        and reading_state.textwidth >= 22
                        and self.spread == 1
                    ):
                        return dataclasses.replace(
                            reading_state,
                            textwidth=reading_state.textwidth - count,
                            rel_pctg=reading_state.row / totlines,
                        )

                    elif k in self.keymap.SetWidth and self.spread == 1:
                        if countstring == "":
                            # if called without a count, toggle between 80 cols and full width
                            if reading_state.textwidth != 80 and cols - 4 >= 80:
                                return ReadingState(
                                    content_index=reading_state.content_index,
                                    textwidth=80,
                                    row=reading_state.row,
                                    rel_pctg=reading_state.row / totlines,
                                )
                            else:
                                return ReadingState(
                                    content_index=reading_state.content_index,
                                    textwidth=cols - 4,
                                    row=reading_state.row,
                                    rel_pctg=reading_state.row / totlines,
                                )
                        else:
                            reading_state = dataclasses.replace(reading_state, textwidth=count)
                        if reading_state.textwidth < 20:
                            reading_state = dataclasses.replace(reading_state, textwidth=20)
                        elif reading_state.textwidth >= cols - 4:
                            reading_state = dataclasses.replace(reading_state, textwidth=cols - 4)

                        return ReadingState(
                            content_index=reading_state.content_index,
                            textwidth=reading_state.textwidth,
                            row=reading_state.row,
                            rel_pctg=reading_state.row / totlines,
                        )

                    elif k in self.keymap.RegexSearch:
                        ret_object = self.searching(
                            board,
                            text_structure.text_lines,
                            reading_state,
                            len(contents),
                        )
                        if isinstance(ret_object, Key) or isinstance(ret_object, NoUpdate):
                            k = ret_object
                            # k = ret_object.value
                            continue
                        elif isinstance(ret_object, ReadingState) and self.search_data:
                            return ret_object
                        # else:
                        elif isinstance(ret_object, ReadingState):
                            # y = ret_object
                            reading_state = ret_object

                    elif k in self.keymap.OpenImage:
                        try:
                            k = self.show_images(board, text_structure.image_maps, reading_state)
                        except Exception as error:
                            k = self.show_win_error("Error Opening Image", str(error), tuple())
                            if DEBUG:
                                raise
                        continue

                    elif (
                        k in self.keymap.SwitchColor
                        and self.is_color_supported
                        and countstring in {"", "0", "1", "2"}
                    ):
                        if countstring == "":
                            count_color = curses.pair_number(cast(int, self.screen.getbkgd()))
                            if count_color not in {2, 3}:
                                count_color = 1
                            count_color = count_color % 3
                        else:
                            count_color = count
                        self.screen.bkgd(curses.color_pair(count_color + 1))
                        # pad.format()
                        return ReadingState(
                            content_index=reading_state.content_index,
                            textwidth=reading_state.textwidth,
                            row=reading_state.row,
                        )

                    elif k in self.keymap.AddBookmark:
                        bmname = self.input_prompt(" Add bookmark:")
                        if isinstance(bmname, str) and bmname:
                            try:
                                self.state.insert_bookmark(
                                    self.ebook,
                                    bmname,
                                    self.original_state(
                                        dataclasses.replace(
                                            reading_state, rel_pctg=reading_state.row / totlines
                                        )
                                    ),
                                )
                            except sqlite3.IntegrityError:
                                k = self.show_win_error(
                                    "Error: Add Bookmarks",
                                    f"Bookmark with name '{bmname}' already exists.",
                                    (Key("B"),),
                                )
                                continue
                        else:
                            k = bmname
                            continue

                    elif k in self.keymap.ShowBookmarks:
                        bookmarks = self.state.get_bookmarks(self.ebook)
                        if not bookmarks:
                            k = self.show_win_error(
                                "Bookmarks",
                                "N/A: Bookmarks are not found in this book.",
                                self.keymap.ShowBookmarks,
                            )
                            continue
                        else:
                            retk, idxchoice = self.show_win_choices_bookmarks()
                            if retk is not None:
                                k = retk
                                continue
                            elif idxchoice is not None:
                                bookmark_to_jump = self.state.get_bookmarks(self.ebook)[idxchoice][
                                    1
                                ]
                                if (
                                    bookmark_to_jump.content_index == reading_state.content_index
                                    and (
                                        self.image_layout is not None
                                        or bookmark_to_jump.textwidth == reading_state.textwidth
                                    )
                                ):
                                    reading_state = self.display_state(
                                        dataclasses.replace(
                                            bookmark_to_jump, textwidth=reading_state.textwidth
                                        )
                                    )
                                else:
                                    return ReadingState(
                                        content_index=bookmark_to_jump.content_index,
                                        textwidth=reading_state.textwidth,
                                        row=bookmark_to_jump.row,
                                        rel_pctg=bookmark_to_jump.rel_pctg,
                                    )

                    elif k in self.keymap.DefineWord and self.ext_dict_app:
                        word = self.input_prompt(" Define:")
                        if isinstance(word, str) and word:
                            defin = self.define_word(word)
                            if defin in self._win_keys:
                                k = defin
                                continue
                        else:
                            k = word
                            continue

                    elif k in self.keymap.MarkPosition:
                        jumnum = board.getch()
                        if isinstance(jumnum, Key) and jumnum in tuple(
                            Key(i) for i in range(48, 58)
                        ):
                            self.jump_list[jumnum.char] = self.original_state(reading_state)
                        else:
                            k = NoUpdate()
                            continue

                    elif k in self.keymap.JumpToPosition:
                        jumnum = board.getch()
                        if (
                            isinstance(jumnum, Key)
                            and jumnum in tuple(Key(i) for i in range(48, 58))
                            and jumnum.char in self.jump_list
                        ):
                            marked_reading_state = self.jump_list[jumnum.char]
                            if marked_reading_state.content_index == reading_state.content_index:
                                marked_reading_state = self.display_state(marked_reading_state)
                            return dataclasses.replace(
                                marked_reading_state,
                                textwidth=reading_state.textwidth,
                                rel_pctg=None
                                if marked_reading_state.textwidth == reading_state.textwidth
                                else marked_reading_state.rel_pctg,
                                section="",
                            )
                        else:
                            k = NoUpdate()
                            continue

                    elif k in self.keymap.ShowHideProgress:
                        self.show_reading_progress = not self.show_reading_progress

                    elif k in self.keymap.Library:
                        self.try_assign_letters_count(force_wait=True)
                        self.calculate_reading_progress(letters_per_content, reading_state)

                        self.savestate(
                            dataclasses.replace(
                                reading_state, rel_pctg=reading_state.row / totlines
                            )
                        )
                        library_items = self.state.get_from_history()
                        if not library_items:
                            k = self.show_win_error(
                                "Library",
                                "N/A: No reading history.",
                                self.keymap.Library,
                            )
                            continue
                        else:
                            retk, choice_index = self.show_win_library()
                            if retk is not None:
                                k = retk
                                continue
                            elif choice_index is not None:
                                return get_ebook_obj(library_items[choice_index].filepath)

                    elif k == Key(curses.KEY_RESIZE):
                        self.pending_position = self.viewport_position(
                            text_structure, reading_state.row
                        )
                        self.savestate(
                            dataclasses.replace(
                                reading_state, rel_pctg=reading_state.row / totlines
                            )
                        )
                        # stated in pypi windows-curses page:
                        # to call resize_term right after KEY_RESIZE
                        if sys.platform == "win32":
                            curses.resize_term(rows, cols)
                            rows, cols = self.screen.getmaxyx()
                        else:
                            rows, cols = self.screen.getmaxyx()
                            curses.resize_term(rows, cols)
                        if cols < 22 or rows < 12:
                            sys.exit("ERROR: Screen was too small (min 22cols x 12rows).")
                        if cols <= reading_state.textwidth + 4:
                            return ReadingState(
                                content_index=reading_state.content_index,
                                textwidth=cols - 4,
                                row=reading_state.row,
                                rel_pctg=reading_state.row / totlines,
                            )
                        else:
                            return ReadingState(
                                content_index=reading_state.content_index,
                                textwidth=reading_state.textwidth,
                                row=reading_state.row,
                            )

                    countstring = ""

                if checkpoint_row:
                    board.feed_temporary_style(
                        (
                            InlineStyle(
                                row=checkpoint_row,
                                col=0,
                                n_letters=reading_state.textwidth,
                                attr=curses.A_UNDERLINE,
                            ),
                        )
                    )

                try:
                    if (
                        self.setting.PageScrollAnimation
                        and self.page_animation
                        and self.image_layout is None
                    ):
                        self.screen.clear()
                        for i in range(1, reading_state.textwidth + 1):
                            curses.napms(1)
                            # self.screen.clear()
                            board.write_n(reading_state.row, i, self.page_animation)
                            self.screen.refresh()
                        self.page_animation = None

                    self.screen.clear()
                    self.screen.addstr(0, 0, countstring)
                    board.write(reading_state.row)

                    # check if letters counting process is done
                    self.try_assign_letters_count()

                    # reading progress
                    self.calculate_reading_progress(letters_per_content, reading_state)

                    # display reading progress
                    if (
                        self.reading_progress
                        and self.show_reading_progress
                        and (cols - reading_state.textwidth - 2) // 2 > 3
                    ):
                        reading_progress_str = "{}%".format(int(self.reading_progress * 100))
                        self.screen.addstr(
                            0, cols - len(reading_progress_str), reading_progress_str
                        )

                    self.screen.refresh()
                except curses.error:
                    pass

                if self.is_speaking:
                    k = self.keymap.TTSToggle[0]
                    continue

                k = board.getch()
                if k == Key(curses.KEY_MOUSE):
                    mouse_event = curses.getmouse()
                    if mouse_event[4] == curses.BUTTON1_CLICKED:
                        if mouse_event[1] < cols // 2:
                            k = self.keymap.PageUp[0]
                        else:
                            k = self.keymap.PageDown[0]
                    elif mouse_event[4] == curses.BUTTON3_CLICKED:
                        k = self.keymap.TableOfContents[0]
                    elif mouse_event[4] == curses.BUTTON4_PRESSED:
                        k = self.keymap.ScrollUp[0]
                    elif mouse_event[4] == 2097152:
                        k = self.keymap.ScrollDown[0]
                    elif mouse_event[4] == curses.BUTTON4_PRESSED + curses.BUTTON_CTRL:
                        k = self.keymap.Enlarge[0]
                    elif mouse_event[4] == 2097152 + curses.BUTTON_CTRL:
                        k = self.keymap.Shrink[0]
                    elif mouse_event[4] == curses.BUTTON2_CLICKED:
                        k = self.keymap.TTSToggle[0]

                if checkpoint_row:
                    board.feed_temporary_style()
                    checkpoint_row = None

        except KeyboardInterrupt:
            self.savestate(
                dataclasses.replace(reading_state, rel_pctg=reading_state.row / totlines)
            )
            sys.exit()


def start_reading(stdscr, filepath: str):

    ebook = get_ebook_obj(filepath)
    state = State()
    config = Config()

    reader = Reader(screen=stdscr, ebook=ebook, config=config, state=state)

    def handle_signal(signum, _):
        """
        Method to raise SystemExit based on signal received
        to trigger `try-finally` clause
        """
        msg = f"[{os.getpid()}] killed"
        if signal.Signals(signum) == signal.SIGTERM:
            msg = f"[{os.getpid()}] terminated"
        sys.exit(msg)

    signal.signal(signal.SIGTERM, handle_signal)

    try:
        reader.run_counting_letters()

        reading_state = state.get_last_reading_state(reader.ebook)
        if reader.screen_cols <= reading_state.textwidth + 4:
            reading_state = dataclasses.replace(reading_state, textwidth=reader.screen_cols - 4)
        else:
            reading_state = dataclasses.replace(reading_state, rel_pctg=None)

        while True:
            reading_state_or_ebook = reader.read(reading_state)

            if isinstance(reading_state_or_ebook, Ebook):
                return reading_state_or_ebook.path
            else:
                reading_state = reading_state_or_ebook
                if reader.seamless:
                    reading_state = reader.convert_absolute_reading_state_to_relative(reading_state)

    finally:
        reader.cleanup()
