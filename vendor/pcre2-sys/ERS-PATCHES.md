# Native search engine patch

This directory contains the PCRE2 sources and Rust bindings from `pcre2-sys` 0.2.10.
The upstream bindings are MIT-licensed. `LICENSE-PCRE2.md` retains the upstream engine's license; source-file copyright notices remain under `upstream/`.

Local changes:

- Always build the bundled interpreter statically, without JIT or system-library discovery.
- Compare case-insensitive UTF backreferences through `ers_regex_lowercase`, supplied by the Rust search module.
- Use simple lowercase values for Unicode and ASCII-only lowercase in ASCII-scoped backreferences.
- Check the existing cancellation/deadline callout every 1,024 characters inside long backreference comparisons.

These changes preserve the reader's required Unicode backreference semantics without a Python dependency.
The remaining PCRE2 source and generated bindings are unchanged.
The upstream sys-crate test module is omitted because this private engine requires the reader's folding callback.
