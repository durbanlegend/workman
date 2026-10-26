# workman_macros

Internal procedural macros for the [workman](https://github.com/durbanlegend/workman) markdown
viewer: they embed the bundled Base16 and TextMate themes into the binary at compile time.

Not intended for use outside workman: the macros read `assets/themes` and
`assets/sublime_themes` relative to the *calling* crate.

Licensed under either of Apache-2.0 or MIT, at your option.
