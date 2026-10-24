# Inline images and previews

Images appear within the reading text when kitten and terminal graphics support are available.
Scroll and change pages normally. Images clip at the viewport edges, including in two-column reading.

Each image gets a stable area based on the text width: 20 rows at 80 columns, capped at 24 rows.
Kitten fits the image inside that area without distorting it. Smaller images can leave blank space around them.
Only visible images load, and the current layout caches up to 32 images. Resizing rebuilds the cache.

Set `InlineImages` to `false` under `Setting` in epy's configuration to keep placeholders instead.
Unsupported terminals and missing helpers also retain placeholders. Failed inline images remain selectable for preview diagnostics or external viewing.

Bookmarks and saved reading positions use the original text rows. Canonical annotation offsets and exported locations do not include image space.
Reopening at a partial image returns to its start. Resizing within the session retains the image-row offset.

## Preview controls

Press `o` while an image or placeholder is visible. A dim border marks the selected image, and the footer shows its number.

| Key | Action |
| --- | --- |
| `j` / `k`, arrows | Select the next / previous visible image |
| Enter or `f` | Preview the selected image inside the terminal |
| Esc or `o` | Cancel selection |
| Esc, Enter, `q`, or `o` in the preview | Return to reading |
| `e` in the preview | Open the original in the external viewer |

Selection requires confirmation even when only one image is visible. The preview resizes with the terminal and preserves your reading position.

## Renderer

Terminal previews use the standalone `kitten` executable through `kitten icat`. Ghostty remains your terminal. The Kitty app is not required.

Download the executable for your platform from [Kitty releases](https://github.com/kovidgoyal/kitty/releases), verify its checksum, and place it on your `PATH`.
Check the installation with `kitten --version`. This integration was tested with kitten 0.49.2.

Kitten handles image decoding, resizing, and graphics transmission. PNG, JPEG, GIF, BMP, TIFF, and WebP work without ImageMagick. Additional formats require ImageMagick.
Animated images show their first frame. epy limits input to 32 MiB and allows up to 10 seconds for rendering.
The image area stays within kitten's safe limit of 296 cells per axis, even in wider or taller panes.
Wide images scale down and stay centered. Rendering failures include the helper's diagnostic message.

The helper receives explicit dimensions and does not read terminal input, so it cannot consume your navigation keys.
Missing helpers, unsupported terminals, and rendering errors show an explanation and offer `e` for external viewing.

## Terminal setup

The preview supports Ghostty and Kitty. Inside tmux, enable passthrough in your configuration and running server:

```tmux
set -g allow-passthrough on
```

```sh
tmux set-option -g allow-passthrough on
```

To prevent dropped images during redraws, epy temporarily sets `allow-passthrough all` only for its own pane.
Inline reading retains that pane setting until the layout closes. Preview-only mode retains it until the preview closes.
Both restore the exact previous override or inheritance and delete only their own images.
Curses owns inline image cells, and synchronized redraws prevent tmux from dropping uploads during heavy screen updates.
Preview output preserves terminal cursor attributes, so image-ID colors cannot leak into the reading text.

Ghostty 1.3.1 has a [reported tmux image-sizing problem](https://github.com/ghostty-org/ghostty/issues/13056).
If a preview appears too narrow, use `e` to view the original externally or a Ghostty version with the fix.

## External viewing

The external viewer receives the original image bytes. Configure it with `DefaultViewer` in epy's configuration, or keep `auto` for platform defaults.
Return to epy and dismiss the notice when finished so its temporary file can be removed.

The source EPUB is never modified.
