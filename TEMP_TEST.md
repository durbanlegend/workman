# workman

![The pun only works in English](assets/icon.png)

A fast, lightweight, multi-lingual GUI markdown viewer, built with [egui](https://github.com/emilk/egui).

<img src="https://raw.githubusercontent.com/lampsitter/egui_commonmark/master/assets/example-v4.png" alt="showcase" width=280/>

### Troubleshooting

<details>
    <summary>Click <b>here</b> to view the error logs</summary>

```bash
Error: Connection timeout at server.js:42
```
</details>

* **Underline:** <u>This text will be underlined.</u>
* **Subscript:** H<sub>2</sub>O (Water)
* **Superscript:** The theorem is a<sup>2</sup> + b<sup>2</sup> = c<sup>2</sup>
* **Highlight:** You can use the <mark>mark tag</mark> to highlight text.

To save your current progress, press <kbd>Ctrl</kbd> + <kbd>S</kbd> on your keyboard.

Company Name Registry  
123 Innovation Way  
<br>
Cape Town, South Africa  
7405

<table>
  <tr>
    <th>Item</th>
    <th>Description</th>
    <th>Price</th>
  </tr>
  <tr>
    <td rowspan="2">Bundle A</td>
    <td>Includes the primary software suite and documentation tools.</td>
    <td>$49.00</td>
  </tr>
  <tr>
    <td>Bonus background pack.</td>
    <td>Free</td>
  </tr>
</table>

Check out our product demo video below:

<iframe width="560" height="315" src="https://youtube.com" frameborder="0" allowfullscreen></iframe>


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
