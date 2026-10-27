# ers

`ers` is a terminal ebook reader written in Rust. It supports EPUB, FB2, MOBI, AZW, AZW3, and HTTP/HTTPS pages, with reading progress, search, bookmarks, links, images, and EPUB annotations.

**Platform:** `ers` has been tested on macOS only. Other platforms are untested.

`ers` is a Rust port of [epy](https://github.com/wustho/epy), which was originally based on [epr](https://github.com/wustho/epr).

## Install

Install Rust 1.91 or newer and the macOS command-line developer tools, then run from this checkout:

```sh
cargo install --locked --path .
```

This installs `ers` in Cargo's binary directory, normally `~/.cargo/bin`. Python is not required to build or run the reader.

To build without installing:

```sh
cargo build --locked --release
./target/release/ers /path/to/book.epub
```

## Open a book

```sh
ers /path/to/book.epub
ers https://example.com/story.html
ers "author or title"
ers --history
ers --dump /path/to/book.epub
```

Run `ers` without arguments to resume the last book, or provide a history number or title/author search. Use `ers --help` for CLI options.

## Reading controls

| Keys | Action |
| --- | --- |
| `j` / `k`, Up / Down | Scroll down / up. |
| `l` / `h`, Space, Right / Left | Next / previous page. |
| Ctrl-D / Ctrl-U | Move down / up half a page. |
| `L` / `H` | Next / previous chapter. |
| `g` / `G` | Beginning / end of chapter. |
| `t` or Tab | Table of contents. |
| `/` | Search a regular expression. Use `n` / `N` for next / previous match. |
| `b` / `B` | Add a bookmark / open bookmarks. |
| `m` then a digit / backtick then a digit | Save / jump to a session quickmark. |
| Ctrl-O / Ctrl-I | Backward / forward through jump history. |
| `R` | Open the library. |
| `D` | Toggle double spread. |
| `-` / `+` | Shrink / enlarge text in single-column mode. |
| `d` | Look up a word. |
| `c` / `s` | Change color profile / toggle progress. |
| `?` / `q` | Help / quit. |

Numeric prefixes repeat supported commands. Terminals must distinguish Ctrl-I from Tab for forward jump history. In tmux, enable `extended-keys`; `ers` accepts CSI-u and xterm extended-key formats. The outer terminal must support extended keys as well.

The reader requires a terminal of at least 22 columns by 12 rows. If the terminal becomes smaller, resize it to continue viewing the book.

During long operations, input buffers have limits. If a buffer fills, `ers` reports ignored input; retry after the operation finishes.

## Highlights and notes

Annotations are available for local EPUB books and do not modify the ebook.

1. Press `v` to enter selection mode, then move with `h/j/k/l` or the arrow keys.
2. Press Space to start a selection and move to its inclusive end. Press Enter to save a highlight, or `n` to add a note.
3. In the note editor, Enter inserts a newline. Press Tab to select **Save**, then Enter to commit. Press `v` or Esc to leave selection mode.

Press `a` to open the annotation list. Select a highlight and press Enter to jump, `e` to edit its note, `f` to read it, or `d` to delete it after confirmation. Press `r` to copy annotations from another identity after validating their exact positions.

Press `O` to link a Markdown file for one-way export. Confirm the destination before writing. Existing handwritten text outside the managed section is preserved. If export fails or encounters a conflict, the annotation remains saved locally; `O` provides retry and recovery options.

## Links and images

Links are underlined, and the status line shows the number of visible links and the configured follow key. Press `f` to show numbers on visible links, then type a number and press Enter. Internal links open in the book; HTTP/HTTPS links open in your browser. Other external schemes are rejected.

Inline images are enabled by default. Press `o` to select an image, Enter to preview it, and `e` in the preview to open it externally. Kitty-compatible terminals can use the Kitty graphics protocol directly. Local tmux sessions with one Ghostty or Kitty client and `allow-passthrough` enabled also use native graphics, with private temporary image files. Other sessions use a text-cell fallback. No `kitten` executable is required.

## Configuration and data

By default, `ers` stores `configuration.json` and `states.db` under `~/.config/ers/`. An absolute `XDG_CONFIG_HOME` changes the parent directory. The configuration file is optional; missing settings use defaults.

For example, create `configuration.json` with:

```json
{
  "settings": {
    "mouse_support": true,
    "between_chapters": false,
    "inline_images": true,
    "dictionary_client": "auto"
  },
  "bindings": {
    "jump_forward": "F"
  }
}
```

Mouse support is disabled by default. When enabled, a left click turns a page, a right click opens the table of contents, the wheel scrolls, and Ctrl-wheel changes text width.

For dictionary lookup, install `wkdict`, `sdcv`, or `dict`, or set `dictionary_client` to a command and its arguments. With `auto`, the reader checks those clients in that order.

The reader uses its own data directory and does not automatically import epy data. Back up `states.db` and linked Markdown files while the reader is closed.

## Credits

Credit to [wustho](https://github.com/wustho) and the contributors to [epy](https://github.com/wustho/epy) for the original Python ebook reader. `ers` ports that reader to Rust and builds on this fork's annotation, image, and link features.

`epy` was originally based on [epr](https://github.com/wustho/epr).

## License

This project is licensed under [GPL-3.0-only](LICENSE). Kindle decoding includes adaptations from [boko](https://github.com/zacharydenton/boko); attribution and license details are in `src/book/kindle/BOKO-NOTICE` and `src/book/kindle/BOKO-LICENSE`.
