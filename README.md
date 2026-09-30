# workman

A fast, lightweight, multi-lingual GUI markdown viewer, built with [egui](https://github.com/emilk/egui).

## Features

- Support for 28 languages, according to your `LOCALE` or `LANG` environment variable, e.g. `LOCALE=fr`.

- Multi-file navigation: files can be selected or dragged and dropped singly or in batches.

- Large document support.

- Light/dark/system theme switching.

- Zoom and font scaling with reset.

- Optional table of contents sidebar.

- Full-document search with options for case (in)sensitive, whole word and regular expression searches.

- Search across mixed text and code spans and link anchors.

- Automatic live file watching and refresh.

Relative links are resolved relative to the parent directory of the current markdown file, so navigation between linked documents works correctly.

## Installation

```sh
cargo install --git https://github.com/durbanlegend/workman
```

Or from a local clone:

```sh
git clone https://github.com/durbanlegend/workman
cd workman
cargo install --path .
```

## Usage

```sh
workman [OPTIONS] [PATH]
```

- `PATH`: optional initial markdown file to open.

- `--foreground`: stay attached to the launching terminal (Unix only). Primarily for debugging.

- `-V`, `--version`: print the version and exit.

- `-h`, `--help`: print help.

On Unix systems, launching from a terminal automatically detaches the process so the terminal is returned immediately.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
