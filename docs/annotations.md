# Highlights and Obsidian notes

This fork adds keyboard annotations for local EPUB files. Zotero can continue storing the books; epy never modifies the source EPUB.

## Run this checkout

From the repository root:

```sh
python3.12 -m venv .venv
.venv/bin/python -m pip install -e .
.venv/bin/epy "/absolute/path/to/book.epub"
```

Use Python 3.12. The bundled MOBI tools depend on `imghdr`, which Python 3.13 removed.

## Highlight a passage

1. Press `v` to start on the text line nearest the window's vertical midpoint, then move with arrow keys or `h/j/k/l`.
2. Press `Space` to mark the start, then move to the last character of the passage.
3. Press `Enter` to save a highlight, or `n` to attach a note. Saving keeps you in `v` mode.

The selection includes both endpoint characters. It can cross wrapped lines and paragraphs within one EPUB content document.

Use `w` to move to the next word start and `b` to move back to a word start, including across wrapped lines.

Letters, digits, and underscores form words; punctuation forms separate runs. Word movement never crosses an EPUB content document boundary.

After saving, the completed selection clears and the cursor stays in place. Press `n` to edit that highlight's note, or `Space` to start another selection.

Press `v` or `Esc` to leave selection mode. Saved highlights remain; only the unsaved selection is discarded.

Keyboard selection is separate from tmux copy mode, which must be inactive.

Annotation commands use ordinary keys and need no changes to tmux configuration. Ghostty and tmux can pass bracketed paste into the editor.

Entering annotations from double-spread reading switches to single-column reading. Press the annotation key again after the switch.

## Write and manage notes

The note editor accepts multiline text. Arrow keys move the cursor; Home, End, Backspace, and Delete edit the draft.

`Enter` inserts a newline. Press `Tab` to focus **Save**, then `Enter` to commit.

Saving or deleting a note returns to `v` mode when you opened the editor there.

When editing an existing highlight's note, the controls are **Save**, **Delete note**, and **Cancel**. Cycle through them with `Tab`.

**Delete note** requires confirmation and preserves the highlight. New annotation drafts offer **Save** and **Cancel**.

Notes appear in the right margin when space permits. Growing drafts push later notes downward, preserving passage order.

When notes extend below the screen, an indicator points to the annotation list. Narrow panes omit the margin; the note viewer remains available.

Press `a` to open the annotation list:

| Key | Action |
| --- | --- |
| Arrow keys or `j/k` | Choose an annotation. |
| `Enter` | Jump to its passage and mark its margin note as focused. |
| `e` | Edit its note. |
| `d` | Delete its highlight and note after confirmation. |
| `f` | Read the quotation and note in a scrollable viewer. |
| `r` | Copy validated annotations from another book identity. |
| `Esc` | Return to reading. |

In the note viewer, use arrow keys or `j/k` to scroll, then `Esc` to return without moving the reading position.

In `v` mode, place the cursor on a highlight and press `n` to edit its existing note without creating another highlight.

Press `d`, Delete, or Backspace on a highlight to remove it after confirmation. Removing a highlight also removes its note.

Press `Space` first when you intend to select a new range, even if it overlaps an existing highlight.

Overlapping highlights remain independent annotations. A chooser appears when editing or deleting could affect more than one highlight.

Notes without highlights are not supported.

## Link an Obsidian file

Press uppercase `O`, paste an absolute `.md` file path, then use `Tab` and `Enter` to save the dialog.

Paste the literal path, including spaces. Do not add shell quotes, backslash-escape spaces, or use `~`.

Confirm the resolved destination. Creating a missing file requires another confirmation; its parent directory must already exist.

The destination can contain your frontmatter and handwritten notes. epy appends a book-specific managed section and replaces only that section afterward.

Each annotation exports its quotation, optional note, and a stable Obsidian block identifier. Export follows book order rather than creation order.

Highlights group under the story or chapter containing the selected passage, even when several stories share one EPUB document.

Nested headings become nested Markdown headings, which Obsidian can fold. A `---` separator distinguishes each complete highlight/note pair.

When available, references use the book's page labels, including page ranges. Otherwise, `Location D:C` identifies the EPUB document and character offset, both starting at 1.

Locations do not depend on terminal size or text width. They are not estimated print-page numbers.

For example:

```markdown
### EUGENE

**Location 2:66587**

> First highlighted passage.

^epy-ANNOTATION-UUID

**Note**

My note about this passage.

---

**Location 2:67030**

> Another highlighted passage.
```

Reopening a book upgrades older annotations when their saved quotations still match. Linked Markdown refreshes automatically, subject to the existing conflict checks.

You can also press `O`, then `r`, to refresh the linked export.

epy exports after linking, saving a highlight, editing a note, or deleting an annotation. Draft keystrokes do not trigger file writes.

Write your own Obsidian content outside the managed section. epy never imports Obsidian edits.

## Export errors and recovery

Annotations save to SQLite before export. A failed export leaves them saved locally, with a pending-export notice in the left margin when space permits.

For an existing link, press `O` to access:

| Key | Action |
| --- | --- |
| `r` | Retry export. |
| `w` | Confirm overwriting external edits inside this book's managed section. |
| `Enter` | Change or recreate the linked destination after confirmation. |
| `u` | Remove the link without deleting annotations or Markdown. |
| `Esc` | Close the dialog. |

Missing or malformed delimiters block export. Overwrite confirmation cannot repair ambiguous boundaries.

To append a fresh section after intentionally removing its delimiters, unlink the file and link it again. Inspect the old content first.

Relinking leaves the old destination untouched. A previously linked file that disappears is never silently recreated.

Exports use atomic replacement, preserve file permissions, and check for concurrent changes. A hidden `.FILENAME.md.epy.bak` retains the previous file contents.

A hidden `.FILENAME.md.epy.lock` coordinates cooperating epy writers. An unrelated editor can still race the final replacement; the backup aids recovery.

## Persistence and moved books

Annotations and export links live in epy's existing `states.db`, under `~/.config/epy/` or `~/.epy/`.

Before the first schema upgrade, epy creates `states.db.pre-annotations.bak`. Back up the active database regularly, preferably while epy is closed.

Renaming or moving an unchanged EPUB preserves its annotations and export link. Identical copies share the same annotations.

Changing EPUB bytes creates a separate identity, even if Zotero only changed its metadata. Old annotations remain stored.

Open `a`, then `r`, to copy annotations from the old identity after confirmation. Only quotations that still match their exact document positions are copied.

Unmatched records remain with the old identity. epy does not guess which occurrence of a repeated quotation you intended.

Removing a book from reading history does not remove its annotations.

## Settings

The existing JSON configuration accepts these additions under `Setting`:

```json
{
  "AnnotationNotesWidth": 36,
  "AnnotationColorFG": 223,
  "AnnotationColorBG": 52,
  "AnnotationAsciiConnectors": false
}
```

The note width accepts 16–100 terminal cells. Colors use the same curses palette as the existing color settings.

Under `Keymap`, configure `AnnotationSelect`, `AnnotationList`, and `AnnotationLink`. Defaults are `v`, `a`, and `O`.

Conflicting annotation bindings produce a configuration error.

## Tests

Run `python -m pytest -q`. Terminal tests use an isolated tmux server and skip when tmux is unavailable.
