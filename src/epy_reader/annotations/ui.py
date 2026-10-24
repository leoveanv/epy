"""Terminal orchestration for annotations; persistence and layout live outside curses."""

import curses
from dataclasses import dataclass, replace
from pathlib import Path
from typing import List, Optional, Sequence, Tuple

from epy_reader.annotations.input import (
    bracketed_paste,
    confirm,
    draw_text,
    notice,
    read_event,
    read_events,
)
from epy_reader.annotations.models import (
    Annotation,
    AnnotationBook,
    ExportLink,
    TextAnchor,
    resolve_anchor,
)
from epy_reader.annotations.references import BookReferences
from epy_reader.annotations.store import AnnotationStore
from epy_reader.annotations.text import SourceCell, clip_text, wrap_text
from epy_reader.annotations.view import (
    EditorAction,
    EditorState,
    MarginInput,
    ReadingLayout,
    Selection,
    place_notes,
    reading_layout,
)
from epy_reader.board import InfiniBoard
from epy_reader.config import Config
from epy_reader.ebooks import Ebook, Epub
from epy_reader.models import Key, ReadingState, TextStructure


@dataclass(frozen=True)
class VisibleAnnotation:
    annotation: Annotation
    cells: Sequence[SourceCell]


@dataclass(frozen=True)
class EditorResult:
    action: EditorAction
    text: str


class AnnotationController:
    screen: curses.window
    ebook: Ebook
    config: Config
    enabled: bool
    store: AnnotationStore
    book: AnnotationBook
    references: BookReferences
    records: List[Annotation]
    visible: List[VisibleAnnotation]
    selection: Optional[Selection]
    draft_anchor: Optional[TextAnchor]
    draft_note: Optional[str]
    draft_id: Optional[str]
    focused_id: Optional[str]
    pending_jump: Optional[str]
    status: str
    retry_done: bool
    resized_during_edit: bool
    editor_size: Optional[Tuple[int, int]]
    highlight_attr: int
    selection_attr: int
    board: InfiniBoard
    structure: TextStructure
    layout: ReadingLayout

    def __init__(self, screen: curses.window, ebook: Ebook, config: Config, database: str) -> None:
        self.screen = screen
        self.ebook = ebook
        self.config = config
        self.enabled = isinstance(ebook, Epub) and Path(ebook.path).suffix.lower() in (
            ".epub",
            ".epub3",
        )
        self.records = []
        self.visible = []
        self.selection = None
        self.draft_anchor = None
        self.draft_note = None
        self.draft_id = None
        self.focused_id = None
        self.pending_jump = None
        self.status = ""
        self.retry_done = False
        self.resized_during_edit = False
        self.editor_size = None
        if self.enabled:
            assert isinstance(ebook, Epub)
            self.references = BookReferences(ebook)
            self.store = AnnotationStore(Path(database))
            metadata = ebook.get_meta()
            self.book = self.store.identify_book(
                Path(ebook.path), metadata.title or Path(ebook.path).stem, metadata.creator or ""
            )
            self.refresh()
        self.highlight_attr = curses.A_REVERSE
        self.selection_attr = curses.A_REVERSE | curses.A_BOLD
        try:
            curses.init_pair(4, config.setting.AnnotationColorFG, config.setting.AnnotationColorBG)
            self.highlight_attr = curses.color_pair(4)
        except (curses.error, ValueError):
            pass

    def refresh(self) -> None:
        self.records = []
        for annotation in self.store.list_annotations(self.book.id):
            corrected = self.references.enrich(annotation)
            if corrected != annotation:
                self.store.save(corrected)
            self.records.append(corrected)

    def attach(self, board: InfiniBoard, structure: TextStructure) -> None:
        self.board = board
        self.structure = structure
        self.visible = []
        if not self.enabled or board.spread != 1:
            return
        self.references.prime(structure.documents)
        self.refresh()
        for annotation in self.records:
            document = next(
                (
                    doc
                    for doc in structure.documents
                    if doc.content_id == annotation.anchor.content_id
                ),
                None,
            )
            if document is not None and resolve_anchor(annotation.anchor, document):
                cells = [
                    cell
                    for cell in structure.source_cells
                    if cell.content_id == annotation.anchor.content_id
                    and cell.start < annotation.anchor.end
                    and cell.end > annotation.anchor.start
                ]
                self.visible.append(VisibleAnnotation(annotation, cells))
        self.layout = reading_layout(
            board.screen_cols, board.textwidth, self.config.setting.AnnotationNotesWidth
        )
        board.x = self.layout.reading_x
        board.annotation_renderer = self.render
        if not self.retry_done:
            self.retry_done = True
            self.export_pending()

    def paint_cells(self, cells: Sequence[SourceCell], row: int, attr: int) -> None:
        rows, cols = self.screen.getmaxyx()
        height = rows - (1 if self.selection is not None else 0)
        for cell in cells:
            y, x = cell.row - row, self.board.x + cell.col
            if not 0 <= y < height or not 0 <= x < cols - cell.width:
                continue
            try:
                previous = self.screen.inch(y, x) & curses.A_ATTRIBUTES
                combined = (
                    ((previous & ~curses.A_COLOR) | attr)
                    if attr & curses.A_COLOR
                    else previous | attr
                )
                self.screen.chgat(y, x, cell.width, combined)
            except curses.error:
                pass

    def render(self, row: int) -> None:
        for visible in self.visible:
            self.paint_cells(visible.cells, row, self.highlight_attr)
        if self.selection is not None:
            self.paint_cells(self.selection.selected_cells(), row, self.selection_attr)
        self.render_notes(row)
        if self.status and self.board.x > 2:
            width = self.board.x - 2
            message = self.status if width >= 20 else ("! O" if "pending" in self.status else "+")
            draw_text(self.screen, 0, 1, clip_text(message, width), curses.A_DIM)

    def render_notes(self, row: int) -> None:
        if self.layout.notes_x is None:
            return
        rows, cols = self.screen.getmaxyx()
        if self.layout.notes_x >= cols:
            return
        notes = []
        for visible in self.visible:
            annotation = visible.annotation
            text = self.draft_note if annotation.id == self.draft_id else annotation.note
            if (
                text
                and visible.cells
                and visible.cells[-1].row >= row
                and visible.cells[0].row < row + rows - 1
            ):
                notes.append(MarginInput(annotation.id, max(0, visible.cells[0].row - row), text))
        if self.draft_anchor is not None and self.draft_id is None:
            cells = self.selection.selected_cells() if self.selection is not None else ()
            if cells:
                notes.append(
                    MarginInput("draft", max(0, cells[0].row - row), self.draft_note or "")
                )
                notes.sort(key=lambda note: note.source_row)
        for note in place_notes(notes, self.layout.notes_width):
            self.render_connector(note.source_row, note.row)
            for index, line in enumerate(note.lines):
                if note.row + index >= rows - 2:
                    draw_text(self.screen, rows - 2, self.layout.notes_x, "↓ More notes: a → f")
                    break
                attr = (
                    curses.A_UNDERLINE
                    if note.annotation_id == self.focused_id and index == 0
                    else 0
                )
                draw_text(self.screen, note.row + index, self.layout.notes_x, line, attr)

    def render_connector(self, source_row: int, note_row: int) -> None:
        start = self.board.x + self.board.textwidth + 1
        notes_x = self.layout.notes_x
        if notes_x is None:
            return
        end = notes_x - 1
        bottom = self.screen.getmaxyx()[0] - 3
        if source_row > bottom:
            return
        ascii_lines = self.config.setting.AnnotationAsciiConnectors
        draw_text(self.screen, source_row, start, "." * max(0, end - start), curses.A_DIM)
        if source_row != note_row:
            draw_text(self.screen, source_row, end - 1, "+" if ascii_lines else "╮", curses.A_DIM)
            for row in range(source_row + 1, min(note_row, bottom)):
                draw_text(self.screen, row, end - 1, ":" if ascii_lines else "┊", curses.A_DIM)
            draw_text(
                self.screen,
                min(note_row, bottom),
                end - 1,
                "+" if ascii_lines else "╰",
                curses.A_DIM,
            )

    def handles(self, key: Key) -> bool:
        keymap = self.config.keymap
        return key in keymap.AnnotationSelect + keymap.AnnotationList + keymap.AnnotationLink

    def handle(self, key: Key, state: ReadingState) -> ReadingState:
        if not self.enabled:
            notice(self.screen, "Annotations currently support local EPUB files only.")
            return state
        try:
            if key in self.config.keymap.AnnotationSelect:
                return self.select(state)
            if key in self.config.keymap.AnnotationList:
                return self.manage(state)
            self.link_dialog()
        except (OSError, ValueError) as error:
            notice(self.screen, str(error))
        return state

    def select(self, state: ReadingState) -> ReadingState:
        height = self.screen.getmaxyx()[0] - 1
        self.selection = Selection.from_viewport(self.structure, state.row, height)
        viewport = state.row
        bracketed_paste(True)
        try:
            curses.curs_set(1)
        except curses.error:
            pass
        try:
            while True:
                rows = self.screen.getmaxyx()[0]
                viewport = self.selection.viewport(
                    viewport, height=rows - 1, scrolloff=self.config.setting.AnnotationScrolloff
                )
                self.screen.erase()
                self.board.write(viewport, bottom_padding=1)
                draw_text(
                    self.screen,
                    rows - 1,
                    0,
                    "Select: hjkl/arrows  Ctrl-U/D: half page  w/b: words  Space: start  Enter: save  n: note  d/Del: delete  v/Esc: exit",
                )
                self.screen.move(
                    self.selection.cell.row - viewport, self.board.x + self.selection.cell.col
                )
                self.screen.refresh()
                event = read_event(self.screen)
                if event.pasted:
                    continue
                toggle = bool(event.text) and Key(event.text) in self.config.keymap.AnnotationSelect
                if event.key in ("escape", "resize") or toggle:
                    return replace(state, row=viewport)
                under_cursor = self.annotations_under_cursor()
                direction = event.key or dict(
                    h="left", j="down", k="up", l="right", w="word_forward", b="word_backward"
                ).get(event.text)
                if direction in ("left", "right", "up", "down", "word_forward", "word_backward"):
                    self.selection.move(direction)
                elif direction in ("half_page_up", "half_page_down"):
                    previous_row = self.selection.cell.row
                    distance = max(1, (rows - 1) // 2)
                    self.selection.move_rows(
                        distance if direction == "half_page_down" else -distance
                    )
                    viewport += self.selection.cell.row - previous_row
                elif event.text == " ":
                    self.selection.mark = self.selection.cursor
                elif event.key in ("delete", "backspace") or event.text == "d":
                    self.resized_during_edit = False
                    self.delete_at_cursor(under_cursor)
                    if self.resized_during_edit:
                        return replace(state, row=viewport)
                elif event.text == "n" and self.selection.mark is None and under_cursor:
                    annotation = self.choose_annotation(under_cursor)
                    if annotation is None:
                        if self.resized_during_edit:
                            return replace(state, row=viewport)
                        continue
                    self.resized_during_edit = False
                    result = self.edit_note(annotation.note, viewport, annotation_id=annotation.id)
                    if self.apply_note_edit(annotation, result):
                        self.refresh_selection()
                    if self.resized_during_edit:
                        return replace(state, row=viewport)
                elif event.key == "enter" or event.text == "n":
                    anchor = self.selection.anchor()
                    self.resized_during_edit = False
                    result = (
                        self.edit_note("", viewport, anchor=anchor)
                        if event.text == "n"
                        else EditorResult(EditorAction.SAVE, "")
                    )
                    if result.action == EditorAction.SAVE:
                        order = list(self.ebook.contents).index(anchor.content_id)
                        reference = self.references.for_anchor(anchor, order)
                        chapter = " / ".join(reference.headings)
                        self.store.create(
                            self.book.id, anchor, result.text, chapter, order, reference=reference
                        )
                        self.export_pending()
                        self.refresh_selection()
                    if self.resized_during_edit:
                        return replace(state, row=viewport)
        finally:
            self.selection = None
            bracketed_paste(False)
            try:
                curses.curs_set(0)
            except curses.error:
                pass

    def refresh_selection(self) -> None:
        self.attach(self.board, self.structure)
        if self.selection is not None:
            self.selection.mark = None

    def annotations_under_cursor(self) -> List[Annotation]:
        if self.selection is None:
            return []
        cell = self.selection.cell
        return [
            item.annotation
            for item in self.visible
            if item.annotation.anchor.content_id == cell.content_id
            and item.annotation.anchor.start <= cell.start < item.annotation.anchor.end
        ]

    def choose_annotation(self, annotations: Sequence[Annotation]) -> Optional[Annotation]:
        if not annotations:
            return None
        if len(annotations) == 1:
            return annotations[0]
        index = 0
        while True:
            self.screen.erase()
            rows = self.screen.getmaxyx()[0]
            draw_text(
                self.screen,
                0,
                1,
                "Overlapping highlights | arrows/jk: choose  Enter: select  Esc: cancel",
            )
            start = max(0, index - rows + 4)
            for offset, annotation in enumerate(annotations[start : start + rows - 3]):
                label = (
                    annotation.anchor.quote.replace("\n", " ")
                    + " | "
                    + (annotation.note or "(No note)")
                )
                draw_text(
                    self.screen,
                    offset + 2,
                    2,
                    label,
                    curses.A_REVERSE if start + offset == index else 0,
                )
            self.screen.refresh()
            event = read_event(self.screen)
            if event.pasted:
                continue
            if event.key in ("escape", "resize"):
                self.resized_during_edit = event.key == "resize"
                return None
            if event.key == "enter":
                return annotations[index]
            if event.key == "down" or event.text == "j":
                index = min(len(annotations) - 1, index + 1)
            elif event.key == "up" or event.text == "k":
                index = max(0, index - 1)

    def delete_at_cursor(self, annotations: Sequence[Annotation]) -> None:
        annotation = self.choose_annotation(annotations)
        if annotation is None:
            return
        if confirm(self.screen, "Delete this highlight and its note?"):
            self.store.delete(annotation)
            self.export_pending()
            self.refresh_selection()

    def apply_note_edit(self, annotation: Annotation, result: EditorResult) -> bool:
        if result.action == EditorAction.CANCEL:
            return False
        if result.action == EditorAction.DELETE:
            if not confirm(self.screen, "Delete this note? The highlight will remain."):
                return False
            note = ""
        else:
            note = result.text
        self.store.update_note(annotation, note)
        self.export_pending()
        return True

    def edit_note(
        self,
        text: str,
        viewport: int,
        anchor: Optional[TextAnchor] = None,
        annotation_id: Optional[str] = None,
    ) -> EditorResult:
        self.draft_anchor = anchor
        self.draft_id = annotation_id
        try:
            return self.editor(
                "Note", text, viewport, multiline=True, can_delete=annotation_id is not None
            )
        finally:
            self.draft_anchor = None
            self.draft_id = None
            self.draft_note = None

    def editor(
        self, title: str, text: str, viewport: int, multiline: bool, can_delete: bool = False
    ) -> EditorResult:
        state = EditorState(text=text, cursor=len(text), multiline=multiline, can_delete=can_delete)
        self.editor_size = None
        bracketed_paste(True)
        try:
            while True:
                self.draw_editor(title, state, viewport)
                for event in read_events(self.screen):
                    if event.key == "resize":
                        self.resized_during_edit = True
                        break
                    if event.pasted or event.text:
                        if state.focus == 0:
                            state.insert(event.text)
                        continue
                    action = state.handle(event.key)
                    if action is not None:
                        return EditorResult(action, state.text)
        finally:
            bracketed_paste(self.selection is not None)
            try:
                curses.curs_set(1 if self.selection is not None else 0)
            except curses.error:
                pass

    def draw_editor(self, title: str, state: EditorState, viewport: int) -> None:
        rows, cols = self.screen.getmaxyx()
        if state.multiline:
            self.draft_note = state.text
        if self.editor_size != (rows, cols):
            self.screen.erase()
            if state.multiline and self.board.x + self.board.textwidth < cols:
                try:
                    self.board.write(viewport)
                except curses.error:
                    pass
            self.editor_size = (rows, cols)
        elif state.multiline:
            self.clear_note_margin()
            self.render_notes(viewport)
        width = min(70, cols - 4)
        height = min(10, rows - 3)
        top = rows - height - 1
        for row in range(top, rows):
            draw_text(self.screen, row, 1, " " * (cols - 2))
        hint = "Enter: newline" if state.multiline else "Enter: controls"
        draw_text(self.screen, top, 2, title + "  | Tab: controls  " + hint + "  Esc: cancel")
        layout = state.layout(width)
        cursor_row = layout.cursor.row
        scroll = max(0, cursor_row - height + 4)
        for index, line in enumerate(layout.lines[scroll : scroll + height - 3]):
            draw_text(self.screen, top + 1 + index, 2, line)
        self.draw_editor_buttons(state, rows - 2, cols)
        if state.focus == 0:
            self.screen.move(
                min(rows - 3, top + 1 + cursor_row - scroll),
                min(cols - 2, 2 + layout.cursor.col),
            )
        try:
            curses.curs_set(1 if state.focus == 0 else 0)
        except curses.error:
            pass
        self.screen.refresh()

    def clear_note_margin(self) -> None:
        if self.layout.notes_x is None:
            return
        rows, cols = self.screen.getmaxyx()
        start = self.board.x + self.board.textwidth + 1
        if start >= cols:
            return
        blank = " " * (cols - start - 1)
        for row in range(rows):
            draw_text(self.screen, row, start, blank)

    def draw_editor_buttons(self, state: EditorState, row: int, columns: int) -> None:
        labels = {
            EditorAction.SAVE: "[ Save ]",
            EditorAction.DELETE: "[ Delete note ]",
            EditorAction.CANCEL: "[ Cancel ]",
        }
        buttons = list(enumerate(state.buttons, 1))
        if sum(len(labels[action]) + 4 for _, action in buttons) > columns - 4:
            buttons = [buttons[max(0, state.focus - 1)]]
        col = 2
        for index, action in buttons:
            label = labels[action]
            draw_text(self.screen, row, col, label, curses.A_REVERSE if state.focus == index else 0)
            col += len(label) + 4

    def manage(self, state: ReadingState) -> ReadingState:
        index = 0
        while True:
            self.refresh()
            if not self.records:
                self.screen.erase()
                draw_text(
                    self.screen,
                    1,
                    2,
                    "No annotations yet. Press v while reading to select a passage.",
                )
                draw_text(
                    self.screen,
                    3,
                    2,
                    "r: reassociate annotations from another book    any other key: close",
                )
                self.screen.refresh()
                event = read_event(self.screen)
                if event.text == "r" and not event.pasted:
                    self.reassociate()
                    continue
                return state
            index = min(index, len(self.records) - 1)
            self.screen.erase()
            rows = self.screen.getmaxyx()[0]
            draw_text(
                self.screen,
                0,
                1,
                "Annotations | Enter: jump  e: edit  d: delete  f: read note  r: reassociate  Esc: close",
            )
            start = max(0, index - rows + 4)
            for offset, annotation in enumerate(self.records[start : start + rows - 3]):
                label = annotation.chapter + ": " + annotation.anchor.quote.replace("\n", " ")
                document = next(
                    (
                        doc
                        for doc in self.structure.documents
                        if doc.content_id == annotation.anchor.content_id
                    ),
                    None,
                )
                if document is not None and resolve_anchor(annotation.anchor, document) is None:
                    label = "[unresolved] " + label
                draw_text(
                    self.screen,
                    offset + 2,
                    2,
                    label,
                    curses.A_REVERSE if start + offset == index else 0,
                )
            self.screen.refresh()
            event = read_event(self.screen)
            if event.pasted:
                continue
            annotation = self.records[index]
            if event.key == "escape":
                return state
            if event.key == "down" or event.text == "j":
                index = min(len(self.records) - 1, index + 1)
            elif event.key == "up" or event.text == "k":
                index = max(0, index - 1)
            elif event.text == "d" and confirm(self.screen, "Delete this highlight and its note?"):
                self.store.delete(annotation)
                self.export_pending()
                self.refresh_selection()
            elif event.text == "e":
                result = self.edit_note(annotation.note, state.row, annotation_id=annotation.id)
                if self.apply_note_edit(annotation, result):
                    self.refresh_selection()
            elif event.text == "f":
                self.view_note(annotation)
            elif event.text == "r":
                self.reassociate()
            elif event.key == "enter":
                self.focused_id = annotation.id
                cells = [
                    cell
                    for cell in self.structure.source_cells
                    if cell.content_id == annotation.anchor.content_id
                    and cell.start >= annotation.anchor.start
                ]
                if cells:
                    return replace(state, row=max(0, cells[0].row - 2))
                self.pending_jump = annotation.id
                return replace(
                    state,
                    content_index=annotation.content_order,
                    row=0,
                    section=None,
                    rel_pctg=None,
                )

    def restore_jump(self, state: ReadingState) -> ReadingState:
        if self.pending_jump is None:
            return state
        annotation = next((item for item in self.records if item.id == self.pending_jump), None)
        self.pending_jump = None
        if annotation is not None:
            resolved = next(
                (item for item in self.visible if item.annotation.id == annotation.id), None
            )
            if resolved is not None and resolved.cells:
                return replace(state, row=max(0, resolved.cells[0].row - 2))
            self.status = (
                "This annotation is unresolved; its saved quotation remains available in a."
            )
        return state

    def view_note(self, annotation: Annotation) -> None:
        scroll = 0
        while True:
            self.screen.erase()
            rows, cols = self.screen.getmaxyx()
            lines = []
            for paragraph in (
                annotation.anchor.quote + "\n\n" + (annotation.note or "(No note)")
            ).split("\n"):
                lines.extend(wrap_text(paragraph, cols - 4) or [""])
            draw_text(self.screen, 0, 2, "Note | arrows/jk: scroll  Esc: close")
            for index, line in enumerate(lines[scroll : scroll + rows - 3]):
                draw_text(self.screen, index + 2, 2, line)
            self.screen.refresh()
            event = read_event(self.screen)
            if event.key == "escape":
                return
            if event.key == "down" or event.text == "j":
                scroll = min(max(0, len(lines) - rows + 3), scroll + 1)
            elif event.key == "up" or event.text == "k":
                scroll = max(0, scroll - 1)

    def reassociate(self) -> None:
        from epy_reader.parser import parse_html

        candidates = [book for book in self.store.list_books() if book.id != self.book.id]
        if not candidates:
            notice(self.screen, "No other annotated books to reassociate.")
            return
        for book in candidates:
            if not confirm(self.screen, "Copy validated annotations from '{}'?".format(book.title)):
                continue
            structures = [
                parse_html(self.ebook.get_raw_text(content), textwidth=80, content_id=str(content))
                for content in self.ebook.contents
            ]
            documents = [
                item.documents[0] for item in structures if isinstance(item, TextStructure)
            ]
            copied = 0
            for annotation in self.store.list_annotations(book.id):
                document = next(
                    (doc for doc in documents if doc.content_id == annotation.anchor.content_id),
                    None,
                )
                if document is not None and resolve_anchor(annotation.anchor, document):
                    self.store.create(
                        self.book.id,
                        annotation.anchor,
                        annotation.note,
                        annotation.chapter,
                        list(self.ebook.contents).index(annotation.anchor.content_id),
                    )
                    copied += 1
            self.export_pending()
            self.refresh_selection()
            notice(
                self.screen,
                "Copied {} validated annotations; original records retained.".format(copied),
            )
            return

    def link_dialog(self) -> None:
        from epy_reader.annotations.export import (
            ExportConflict,
            export_book,
            validate_destination,
        )

        link = self.store.get_link(self.book.id)
        if link is not None:
            self.screen.erase()
            draw_text(self.screen, 1, 2, "Linked to: " + link.path)
            draw_text(
                self.screen,
                3,
                2,
                "Enter: change path  r: retry  w: confirm section overwrite  u: unlink  Esc: close",
            )
            self.screen.refresh()
            event = read_event(self.screen)
            if event.pasted:
                return
            if event.text == "u":
                if confirm(
                    self.screen, "Remove this export link? Annotations and Markdown remain."
                ):
                    self.store.unlink(self.book.id)
                return
            if event.text in ("r", "w"):
                overwrite = event.text == "w" and confirm(
                    self.screen,
                    "Overwrite only this book's managed section? External edits inside it will be lost.",
                )
                if event.text == "w" and not overwrite:
                    return
                export_book(self.store, self.book, overwrite=overwrite)
                self.status = "Exported to Obsidian."
                return
            if event.key != "enter":
                return
        result = self.editor(
            "Absolute Markdown path", link.path if link else "", 0, multiline=False
        )
        if result.action != EditorAction.SAVE:
            return
        destination = validate_destination(result.text)
        if not confirm(self.screen, "Link and export to " + str(destination) + "?"):
            return
        created = False
        if not destination.exists():
            if not confirm(self.screen, "Create " + str(destination) + "?"):
                return
            destination.touch(exist_ok=False)
            created = True
        previous = self.store.get_link(self.book.id)
        proposed = (
            previous
            if previous and previous.path == str(destination) and not created
            else ExportLink(book_id=self.book.id, path=str(destination))
        )
        try:
            from epy_reader.annotations.export import merge_section, render_section

            content = destination.read_bytes()
            section = render_section(
                self.book,
                self.store.list_annotations(self.book.id),
                "\r\n" if b"\r\n" in content else "\n",
            )
            merge_section(content, self.book.id, section, proposed.digest)
        except (OSError, ExportConflict):
            if created and destination.exists() and destination.stat().st_size == 0:
                destination.unlink()
            raise
        self.store.set_link(proposed)
        self.export_pending()

    def export_pending(self) -> None:
        from epy_reader.annotations.export import export_book

        if not self.enabled:
            return
        link = self.store.get_link(self.book.id)
        if link is None or link.revision <= link.exported_revision:
            return
        try:
            export_book(self.store, self.book)
            self.status = "Saved and exported to Obsidian."
        except (OSError, ValueError) as error:
            self.status = "Saved locally; export pending. O to retry: " + str(error)
