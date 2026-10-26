# workman

![The pun only works in English](assets/icon.png)

A fast, multi-lingual GUI markdown viewer, built with [egui](https://github.com/emilk/egui).

## Features

- Support for 28 languages, according to your `LOCALE` or `LANG` environment variable, e.g. `LOCALE=fr`.

- Cross-platform.

- Multi-file navigation: files can be selected or dragged and dropped singly or in batches.

- Fast performance for large documents (after initial scan).

- Personalised theming from the toolbar with built-in, external or default light and dark themes.

- Zoom and font scaling with reset.

- Optional table of contents sidebar.

- Full-document search with options for case (in)sensitive, whole word and regular expression searches.

- Searches can include embedded code spans and link anchors.

- Automatic live file watching and refresh.

Relative links are resolved relative to the parent directory of the current markdown file, so navigation between linked documents works correctly.

## Supported languages

The interface language is chosen from your `LOCALE` or `LANG` environment variable, e.g. `LOCALE=fr`, falling back to English.

The following languages are currently supported:

| Code | Language | | Code | Language |
|------|----------|-|------|----------|
| `af` | Afrikaans | | `it` | Italiano (Italian) |
| `bg` | Български (Bulgarian) | | `nb`, `no` | Norsk (Norwegian) |
| `ca` | Català (Catalan) | | `nl` | Nederlands (Dutch) |
| `cs` | Čeština (Czech) | | `pl` | Polski (Polish) |
| `cy` | Cymraeg (Welsh) | | `pt` | Português (Portuguese) |
| `da` | Dansk (Danish) | | `ro` | Română (Romanian) |
| `de` | Deutsch (German) | | `ru` | Русский (Russian) |
| `el` | Ελληνικά (Greek) | | `sk` | Slovenčina (Slovak) |
| `en` | English | | `sl` | Slovenščina (Slovenian) |
| `es` | Español (Spanish) | | `st` | Sesotho (Southern Sotho) |
| `fi` | Suomi (Finnish) | | `sv` | Svenska (Swedish) |
| `fr` | Français (French) | | `uk` | Українська (Ukrainian) |
| `hr` | Hrvatski (Croatian) | | `xh` | isiXhosa (Xhosa) |
| `hu` | Magyar (Hungarian) | | `zu` | isiZulu (Zulu) |

Unfortunately, `egui` only supports left-to-right languages.

Please report any mistakes in the AI-provided translations as issues on the `workman` GitHub repo.

## Installation

### Prebuilt binaries

macOS and Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/durbanlegend/workman/releases/latest/download/workman_md-installer.sh | sh
```

Windows (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/durbanlegend/workman/releases/latest/download/workman_md-installer.ps1 | iex"
```

Or download an archive for your platform from the [releases page](https://github.com/durbanlegend/workman/releases).

### From crates.io

```sh
cargo install workman_md
```

The installed command is `workman`.

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

| `OPTIONS` |  |
|-----------|--|
| `-f, --foreground` |  Stay attached to the launching terminal (Unix only). Primarily for debugging.
| `-h, --help` |  Print help.
| `-s, --search-collapsible` |  Expand collapsible widgets to make them searchable.
| `-V, --version` |  Print the version and exit.
| | |
| `PATH` |  Optional initial markdown file to open.

On Unix systems, launching from a terminal without the `-f` option automatically detaches the process so the terminal is returned immediately.

## Theming

You can tailor the look of _Workman_ quite extensively to suit your taste.

The palette 🎨 button on the toolbar toggles a _Themes_ window where you can try out and choose from a gallery of built-in themes, or load themes from external `base16` or `base24` `.yaml` files (in the case of `base24` the extra `base24` fields will be ignored). A wide selection is available to browse and download at [tinted-theming](https://github.com/tinted-theming/schemes/blob/spec-0.11/README.md).

Code blocks in markdown have their own separate highlighting system. From the same window you can try out and choose from a gallery of `syntect` code block highlighting themes to go with your markdown theme, or load themes from external `.tmTheme` files. A wide selection is available to browse and download at [TextMate-Themes](https://inkdeep.github.io/TextMate-Themes).

The `Save` button will save the current configuration as the default. The previous configuration file will be renamed with the extension `yyyymmdd-HHMMss` as a backup. The files are small: around ~300 bytes or less, depending on file name and path lengths.

You may of course edit the configuration manually at the following location to specify the exact names (file stems) and locations of your preferred themes:

- MacOS/Linux: '$HOME/.config/workman/config.toml'
- Windows: '$APPDATA/.config/workman/config.toml'

You can copy the [sample configuration file](https://github.com/durbanlegend/workman/blob/theming/assets/default_config.toml) if you need to.

Restart _Workman_ for the changes to take effect.

The `Esc` key is a keyboard shortcut to close the _Themes_ window.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
