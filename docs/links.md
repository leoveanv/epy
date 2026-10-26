# Citations and link navigation

Recognized citations display as `[1]`, `[2]`, and similar labels. This applies to linked superscripts, EPUB noteref markup, ARIA noteref roles, and FB2 note links.

Ordinary mathematical superscripts and subscripts retain their existing notation. epy does not guess that every superscript is a citation.

Citation rendering does not rewrite the EPUB or canonical annotation text. Existing highlights keep their exact anchors, quotations, and Obsidian block identifiers. Saved quotations and Obsidian exports retain their original notation.

## Follow links

In normal reading, press `f` to show temporary numbers on visible links. Type a number and press Enter to follow it.

| Input | Action |
| --- | --- |
| `f` | Start follow mode. |
| Digits, then Enter | Follow the numbered link. |
| Backspace | Edit the number. |
| Esc or Ctrl-C | Cancel follow mode. |
| Resize | Cancel hints and rebuild the reading layout. |

Numbers belong to the current follow-mode view, not the book's citation numbering. Two citations both labeled `[1]` receive different hint numbers.

Wrapped links receive one number. Hints account for double-spread reading and inline image space. If a row cannot fit every number, epy shows a numbered link list instead. Up and Down scroll that list.

Internal links jump to the referenced document and fragment. Footnote backlinks work like other internal links. A missing or ambiguous target shows an explanation without moving the reader.

HTTP and HTTPS links open in the default browser. Opening a browser does not move the reader or add a history entry. Other external schemes, including executable and local-file URLs, are rejected.

EPUB links can target readable manifest documents outside the spine. epy makes these documents available after the spine without modifying the source EPUB. MOBI and AZW use their unpacked HTML links. FB2 links can target notes in another body. Web-page relative links resolve against the page URL.

Malformed markup or unsupported links do not receive guessed destinations. Image preview controls remain separate from text-link navigation.

## Back and forward

| Input | Action |
| --- | --- |
| Ctrl-O | Move backward through the jump history. |
| Ctrl-I | Move forward through the jump history. |
| Tab or `t` | Open the table of contents. |

The history retains up to 100 positions per open book. It resets when the book closes.

Internal links, table-of-contents selections, bookmark jumps, quickmark jumps, annotation-list jumps, and search jumps contribute positions. Ordinary scrolling does not create entries.

Back/forward navigation preserves the current reading context, including scrolling after a jump. It uses canonical text positions so returning still works after changing text width. Partial inline image positions also survive reflow.

A new explicit jump after moving backward discards the forward branch. Numeric prefixes repeat history movement, such as `2` followed by Ctrl-O.

## Keep Ctrl-I distinct from Tab

Legacy terminal input sends the same byte for Ctrl-I and Tab. epy preserves that byte as Tab. It never silently assigns Tab to forward navigation.

Outside tmux, epy requests Kitty keyboard disambiguation and restores the previous keyboard mode on exit. Ghostty and Kitty support this mode.

Inside tmux, epy requests extended keys and accepts CSI-u or xterm modifyOtherKeys input. The outer terminal must also support extended keys.

For Ghostty with a recent tmux, add these settings to your tmux configuration:

```tmux
set -s extended-keys on
set -s extended-keys-format csi-u
set -as terminal-features ',xterm-ghostty:extkeys'
```

Reload the configuration and detach/reattach the client so tmux refreshes its terminal capabilities. These settings must apply to the outer Ghostty client, not the inner `tmux-256color` terminal.

On a legacy terminal, Ctrl-I cannot work independently of Tab. Ctrl-O still works, and Tab remains the table-of-contents key. The help screen identifies Ctrl-I's extended-input requirement.

`JumpBackward` and `JumpForward` are configurable under `Keymap`. epy rejects bindings that conflict with another reader command.

## Verification

Run the full suite with:

```sh
.venv/bin/python -m pytest -q
```

Link tests cover citation display, canonical anchor preservation, fragment resolution, browser launch, hint numbering, jump history, reflow, and actual tmux input forwarding. Browser tests use a local stub and do not open a real browser.

Protocol references:

- [Zathura navigation](https://man.archlinux.org/man/zathura.1.en)
- [Kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)
- [tmux modifier keys](https://github.com/tmux/tmux/wiki/Modifier-Keys)
