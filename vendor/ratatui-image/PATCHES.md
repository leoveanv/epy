# Local ratatui-image 11.1.0 patch

This directory contains the source of `ratatui-image` 11.1.0 under its original MIT license. The crates.io archive SHA-256 is `9593d10c61ec2007f429d1fafc4f4a19fecdbae4d06b531a2d505227fb82f5ea`.

The local patch adds `Kitty::from_transmission`, which uses an application-supplied Kitty graphics transmission with the library's existing placeholder renderer. `ers` uses it to transfer images through local tmux sessions using private temporary files instead of sending large pixel streams through the terminal.
