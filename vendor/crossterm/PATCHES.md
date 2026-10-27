# Local crossterm 0.29.0 patch

This directory contains the crates.io release of crossterm 0.29.0 under its original MIT license. The release archive SHA-256 is `d8b9f2e4c67f833b660cdb0a3523065869fb35570177239812ed4c905aeff87b`.

## Changes

- `src/event/source/unix/mio.rs` returns `UnexpectedEof` when terminal input closes and propagates fatal read errors. Previously, the inner loop retried both indefinitely, bypassing the polling deadline and preventing application shutdown.
- `src/event/sys/unix/parse.rs` accepts xterm `modifyOtherKeys` input (`CSI 27;modifier;code~`) by passing its validated modifier and code point through the existing CSI-u decoder. This supports tmux's xterm extended-key format without conflating Ctrl-I and Tab.
- `src/terminal/sys/unix.rs` removes redundant parentheses that trigger a compiler warning with the current toolchain. This change does not alter behavior.

The event-loop fix follows the approach in upstream [PR 1067](https://github.com/crossterm-rs/crossterm/pull/1067), addressing [issue 793](https://github.com/crossterm-rs/crossterm/issues/793) and [issue 1110](https://github.com/crossterm-rs/crossterm/issues/1110). Only the mio input loop is patched here. The optional `use-dev-tty` and event-stream backends are not part of this reader's configuration.

The reader's terminal lifecycle tests exercise real pseudoterminal closure without a hangup signal, recent-position persistence, and termination during an unsaved note. Native PTY keyboard tests exercise CSI-u and xterm history navigation, key release/repeat, and keyboard-mode restoration. They require no Python runtime.
