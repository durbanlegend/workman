# workman

![The pun only works in English](assets/icon.png)

A fast, lightweight, multi-lingual GUI markdown viewer, built with [egui](https://github.com/emilk/egui).

## Features

- Support for 28 languages, according to your `LOCALE` or `LANG` environment variable, e.g. `LOCALE=fr`.

- Cross-platform.

- Multi-file navigation: files can be selected or dragged and dropped singly or in batches.

- Large document support.

- Light/dark/system theme switching.

- Zoom and font scaling with reset.

- Optional table of contents sidebar.

- Full-document search with options for case (in)sensitive, whole word and regular expression searches.

- Search across mixed text and code spans and link anchors.

- Automatic live file watching and refresh.

Relative links are resolved relative to the parent directory of the current markdown file, so navigation between linked documents works correctly.

## Supported languages

The interface language is chosen from your `LOCALE` or `LANG` environment variable, e.g. `LOCALE=fr`, falling back to English.

The following languages are currently supported. Please report any mistakes in the AI-provided translations as issues on the `workman` GitHub repo. 

| Code | Language | Code | Language |
|------|----------|------|----------|
| `af` | Afrikaans | `it` | Italiano
| `bg` | Български | `nb`, `no`  | Norsk |
| `ca` | Català | `nl` | Nederlands |
| `cs` | Čeština | `pl` | Polski |
| `cy` | Cymraeg | `pt` | Português |
| `da` | Dansk | `ro` | Română |
| `de` | Deutsch | `ru` | Русский |
| `el` | Ελληνικά | `sk` | Slovenčina |
| `en` | English | `sl` | Slovenščina |
| `es` | Español | `st` | Sesotho |
| `fi` | Suomi | `sv` | Svenska |
| `fr` | Français | `uk` | Українська |
| `hr` | Hrvatski | `xh` | isiXhosa |
| `hu` | Magyar | `zu` | isiZulu |

## Installation

### Prebuilt binaries

macOS and Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/durbanlegend/workman/releases/latest/download/workman-installer.sh | sh
```

Windows (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/durbanlegend/workman/releases/latest/download/workman-installer.ps1 | iex"
```

Or download an archive for your platform from the [releases page](https://github.com/durbanlegend/workman/releases).

### From source

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
