mod fast_svg_loader;

use base16::Base16;
use eframe::egui;
use egui::Color32;
use egui::{Key, Modifiers, Popup, PopupCloseBehavior, ScrollArea};
use egui_commonmark::{CommonMarkCache, CommonMarkScrollOptions, CommonMarkViewer, SearchOptions};
use macros::{preload_base16_themes, preload_syntect_themes};
use notify::{RecursiveMode, Watcher};
use pulldown_cmark::{Event, Options, Parser, Tag};
use rfd::FileDialog;
use rust_i18n::t;
use std::{
    collections::HashMap,
    env,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};
use syntect::highlighting::{Color, ThemeSet};
use thag_common::{auto_help, help_system::check_help_and_exit};
use workman::config::{self /*, Config*/};

/// A fast lightweight multi-lingual GUI markdown viewer.
///
/// Relative links are resolved relative to the parent directory of the
/// current markdown file, so navigation between linked documents works correctly.
///
/// Features:
/// - Support for 28 languages, according to your LOCALE or LANG system/environment variable. E.g. LOCALE=fr.
/// - Cross-platform.
/// - Multi-file navigation: files can be selected or dragged and dropped singly or in batches as and when needed.
/// - Large document support.
/// - Light/dark/system theme switching.
/// - Zoom and font scaling with reset.
/// - Optional table of contents sidebar.
/// - Full-document search with options for case (in)sensitive, whole word and regular expression searches.
/// - Search across mixed text and code spans and link anchors.
/// - Automatic live file watching and refresh
///
/// On Unix systems, launching from a terminal automatically detaches the process so the terminal
/// is returned immediately (use --foreground to suppress this).
//# Purpose: A GUI markdown viewer with navigation, zoom, and file-open support.
//# Categories: crates, gui, tools
//# Usage: workman [OPTIONS] [PATH]
//# Option: --foreground (-f): Stay attached to the launching terminal (Unix only). Primarily for debugging.
//# Option: --search-collapsible (-s): Expand collapsible widgets to make them searchable.
//# Option: --version (-V): Print version number and exit.
//# Argument: [PATH]: Optional initial markdown file to open
use workman::html_prep::preprocess_html_with;
use workman::html_render::HtmlRenderer;

const HIST_BACK_ICON: &str = "\u{25c0}";
const HIST_FWD_ICON: &str = "\u{25b6}";
const OPEN_FILES_ICON: &str = "\u{1f4d6}\u{2026}";

#[cfg(target_os = "macos")]
const ALT: &str = "Opt";
#[cfg(target_os = "macos")]
const MOD: &str = "Cmd";

#[cfg(not(target_os = "macos"))]
const ALT: &str = "Alt";
#[cfg(not(target_os = "macos"))]
const MOD: &str = "Ctrl";

/// Documents at or above this byte count get the viewport-cache toggle shown in
/// the toolbar and have caching auto-enabled.  Below this size the simple
/// full-document render path is always used (accurate + fast enough).
const VIEWPORT_CACHE_THRESHOLD: usize = 200_000; // 200 KB

rust_i18n::i18n!("locales", fallback = "en");

macro_rules! syntax_str {
    ($($syntax:literal),+ $(,)?) => {
        &[
            $(
                (
                    $syntax,
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/assets/sublime_syntax/",
                        $syntax,
                        ".sublime-syntax"
                    ))
                ),
            )+
        ]
    };
}

// Preload Base16 themes from the `assets/themes` directory into a static HashMap `THEME_MAP`.
preload_base16_themes! {}

// Preload Syntect themes from the `assets/sublime_themes` directory into a static HashMap `SYNTECT_THEME_MAP`.
preload_syntect_themes! {}

const SYNTAX_STR: &[(&str, &str)] = syntax_str!("PowerShell", "TOML");
const DEFAULT_SYNTECT_THEME_DARK: &str = "Dunkel_Theme";
const DEFAULT_SYNTECT_THEME_LIGHT: &str = "Active4D";

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Base16Filter {
    Light,
    Dark,
    #[default]
    All,
}

impl Base16Filter {
    const fn allows(self, is_dark: bool) -> bool {
        match self {
            Self::All => true,
            Self::Dark => is_dark,
            Self::Light => !is_dark,
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal Base16 YAML parsing (no YAML crate needed)
//
// Handles both layouts:
//   old:  scheme: "Name"        / base00: "181818" ...
//   new:  name: "Name" / variant: "dark" / palette: { base00: "181818" ... }
// Only flat `key: value` lines are understood, which covers every published
// Base16 / tinted-theming scheme file.
// ---------------------------------------------------------------------------

fn yaml_scalar(v: &str) -> String {
    let v = v.trim();
    if let Some(q) = v.chars().next().filter(|&c| c == '"' || c == '\'') {
        return v[1..].split(q).next().unwrap_or("").to_owned();
    }
    v.split(" #").next().unwrap_or("").trim().to_owned()
}

#[allow(clippy::cast_possible_truncation)]
fn parse_hex(s: &str) -> Result<Color32, String> {
    let s = s.trim_start_matches('#');
    let bad = || format!("invalid colour {s:?}");
    if s.len() != 6 {
        return Err(bad());
    }
    let n = u32::from_str_radix(s, 16).map_err(|_| bad())?;
    Ok(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

fn parse_base16_yaml(src: &str, fallback_name: &str) -> Result<Base16, String> {
    let mut name = None;
    let mut variant = None;
    let mut c: [Option<Color32>; 16] = [None; 16];

    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().trim_matches(|ch| ch == '"' || ch == '\'');
        let value = yaml_scalar(value);

        match key {
            "name" | "scheme" => name = Some(value),
            "variant" => variant = Some(value.to_ascii_lowercase()),
            _ => {
                if let Some(hex) = key.strip_prefix("base").filter(|h| h.len() == 2) && let Ok(i) = usize::from_str_radix(hex, 16)
                        // ignore extra base24 fields
                        && i < c.len()
                {
                    // eprintln!("key={key}, i={i}");
                    c[i] = Some(parse_hex(&value).map_err(|e| format!("{key}: {e}"))?);
                }
            }
        }
    }

    let missing: Vec<String> = (0..16)
        .filter(|&i| c[i].is_none())
        .map(|i| format!("base{i:02X}"))
        .collect();
    if !missing.is_empty() {
        return Err(format!("missing {}", missing.join(", ")));
    }
    let c: [Color32; 16] = c.map(|x| x.unwrap());

    // Prefer an explicit `variant`; otherwise judge by base00 (the background).
    let is_dark = match variant.as_deref() {
        Some("dark") => true,
        Some("light") => false,
        _ => {
            let bg = c[0];
            // 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * (bg.b() as f32) < 128.0
            0.114_f32.mul_add(
                f32::from(bg.b()),
                0.587_f32.mul_add(f32::from(bg.g()), 0.299 * f32::from(bg.r())),
            ) < 128.0
        }
    };

    // Themes are keyed by `&'static str`, so loaded names are leaked. That's a
    // few bytes per file the user chooses to load.
    let name: &'static str = Box::leak(
        name.unwrap_or_else(|| fallback_name.to_owned())
            .into_boxed_str(),
    );

    Ok(Base16 { name, is_dark, c })
}

// ---------------------------------------------------------------------------
// Sample document shown in the theme window
// ---------------------------------------------------------------------------

const SAMPLE_MD: &str = r#"
# Theme preview

Some *emphasis*, **strong text**, `inline code` and a [link](https://example.com).

```rust
fn main() {
    let name = "world";
    println!("Hello, {name}!");
}
```

```toml
[package]
name = "demo"
version = "0.1.0"

[dependencies]
serde = { version = "1", features = ["derive"] }
```

```powershell
Get-ChildItem -Path . -Recurse |
    Where-Object { $_.Length -gt 1MB } |
    Sort-Object Length -Descending
```

```
An unannotated code fence.
No language, so no highlighting.
```
"#;

// ---------------------------------------------------------------------------
// Light/dark classification of the bundled .tmTheme files (computed once)
// ---------------------------------------------------------------------------

/// `(theme name, is_dark)` for every theme in `SYNTECT_THEME_MAP`, sorted by name.
/// "Dark" means the theme's background colour has a luminance below 50%.
/// A theme with no background setting is treated as light; one that fails to
/// parse is skipped.
fn syntect_themes() -> &'static [(&'static str, bool)] {
    static THEMES: OnceLock<Vec<(&'static str, bool)>> = OnceLock::new();
    THEMES.get_or_init(|| {
        let mut v: Vec<_> = SYNTECT_THEME_MAP
            .entries()
            .filter_map(|(&name, &src)| {
                let theme = ThemeSet::load_from_reader(&mut Cursor::new(src)).ok()?;
                let bg = theme.settings.background.unwrap_or(Color::WHITE);
                // let lum = 0.299 * bg.r as f32 + 0.587 * bg.g as f32 + 0.114 * bg.b as f32;
                let lum = 0.114_f32.mul_add(
                    f32::from(bg.b),
                    0.587_f32.mul_add(f32::from(bg.g), 0.299 * f32::from(bg.r)),
                );
                Some((name, lum < 128.0))
            })
            .collect();
        v.sort_unstable_by_key(|(name, _)| *name);
        v
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum ThemeFilter {
    /// Only syntect themes matching the current light/dark mode.
    #[default]
    Matching,
    All,
}

/// Applies contrast colours to both egui themes; font sizes are always left at
/// egui defaults so toggling never causes a scroll-position jump.
///
/// `enhanced = true`  — high-contrast colours (near-white/near-black text, warm backgrounds).
/// `enhanced = false` — stock egui colours.
///
/// Called once at startup and again whenever the toolbar "Contrast+/-" toggle changes.
/// `image_loading_spinners` is kept `false` in both modes.
#[allow(dead_code)]
fn apply_style(ctx: &egui::Context, enhanced: bool) {
    const BRIGHTEN: f32 = 1.3;
    const DARKEN: f32 = 1.0 / BRIGHTEN;
    // ── Dark mode ─────────────────────────────────────────────────────────────────────────
    ctx.global_style_mut(|style| {
        // Show the url of a hyperlink on hover
        style.url_in_tooltip = true;
        // Reduce the tooltip delay down to 0.0 seconds (or something low like 0.05)
        style.interaction.tooltip_delay = 0.0;
        // Don't enforce tooltip delays within this many seconds of viewing the first tooltip
        style.interaction.tooltip_grace_time = 3.0;
        // Optional: Show tooltips even if the mouse is still drifting
        // style.interaction.show_tooltips_only_when_still = false;
    });
    ctx.set_visuals_of(egui::Theme::Dark, {
        let mut v = egui::Visuals::dark();
        if enhanced {
            v.widgets.noninteractive.fg_stroke.color = Color32::from_gray(240);
            v.code_bg_color = Color32::from_gray(100);
            v.hyperlink_color = Color32::from_rgb(100, 185, 255);
        }
        v.image_loading_spinners = false; // always off in a document reader
        v
    });

    // ── Light mode ────────────────────────────────────────────────────────────────────────
    ctx.set_visuals_of(egui::Theme::Light, {
        let mut v = egui::Visuals::light();
        if enhanced {
            v.widgets.noninteractive.fg_stroke.color = Color32::from_gray(5);
            v.panel_fill = Color32::from_rgb(255, 255, 255);
            v.window_fill = Color32::from_rgb(255, 255, 255);
            v.code_bg_color = Color32::from_rgb(225, 225, 230);
            v.hyperlink_color = Color32::from_rgb(0, 100, 210);
        } else {
            v.code_bg_color = Color32::from_rgb(225, 225, 230);
        }
        v.image_loading_spinners = false; // always off in a document reader
        v
    });

    // Balance monospace font size in inline code and fenced code blocks
    // against proportional font size.
    // In egui 0.35+ font styles are stored per-theme, so set both.
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            use egui::{FontFamily, FontId, TextStyle};
            style.text_styles.insert(
                TextStyle::Monospace,
                FontId::new(12.0, FontFamily::Monospace),
            );
        });
    }
}

use egui::ecolor::Hsva;

/// Adjusts a Color32 by a given factor (e.g., 1.2 for +20% brightness).
#[allow(dead_code)]
fn adjust_color(color: Color32, factor: f32) -> Color32 {
    let mut hsva = Hsva::from(color);
    // Scale the Value (brightness) component, keeping it clamped between 0.0 and 1.0
    hsva.v = (hsva.v * factor).clamp(0.0, 1.0);
    let color = Color32::from(hsva);
    // eprintln!("color: {:?}", color);
    color
}

// ─── TOC / heading extraction ──────────────────────────────────────────────────

/// An entry in the table of contents, derived from one ATX heading in the document.
#[derive(Clone)]
struct TocEntry {
    /// Heading depth 1–6.
    level: u8,
    /// Display text (raw heading text; may include inline markup such as `**bold**`).
    text: String,
    /// The `{#slug}` injected into the rendered content, used as the scroll target.
    slug: String,
}

/// Converts heading text to a URL-safe slug: lowercased, non-alphanumeric runs replaced by `-`.
fn slugify(text: &str) -> String {
    let mut slug = String::with_capacity(text.len());
    let mut prev_sep = true; // start true to drop any leading hyphens
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_sep = false;
        } else if !prev_sep {
            slug.push('-');
            prev_sep = true;
        }
    }
    if slug.ends_with('-') {
        slug.pop();
    }
    slug
}

/// Parses an ATX heading line and returns `(level, plain_text)`.
/// `plain_text` is the heading content with any trailing `{…}` attribute block stripped.
/// Returns `None` for non-heading lines, indented lines, or malformed ATX syntax.
#[allow(clippy::cast_possible_truncation)]
fn parse_heading_line(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    // ATX heading must have a space after the `#` run.
    let rest = line[hashes..].strip_prefix(' ')?;
    let text = rest.trim_end();
    if text.is_empty() {
        return None;
    }
    // Strip any trailing `{#id}` / `{.class}` attribute block.
    let plain = text.rfind('{').map_or(text, |brace| {
        let attr = text[brace..].trim_end();
        if attr.ends_with('}') {
            text[..brace].trim_end()
        } else {
            text
        }
    });
    Some((hashes as u8, plain))
}

/// Returns the explicit `{#id}` from a heading line, if present.
fn extract_heading_id(line: &str) -> Option<&str> {
    let brace = line.rfind('{')?;
    let attr = line[brace..].trim_end();
    if attr.starts_with("{#") && attr.ends_with('}') {
        Some(&attr[2..attr.len() - 1])
    } else {
        None
    }
}

/// Scans `raw` markdown, builds a `Vec<TocEntry>` from headings, and returns a version
/// of the content with `{#slug}` attributes injected into every heading that lacks one.
///
/// **Uses pulldown-cmark as the heading oracle** rather than a hand-rolled fence
/// tracker. This guarantees that the TOC and injected IDs are consistent with what
/// the renderer sees. A custom fence tracker diverges from pulldown-cmark in edge
/// cases such as XML-like tags (`<context>`, `<files>`) being treated as type-6 HTML
/// blocks that can swallow a code-fence opener — leading to headings that are
/// unreachable by the scroll mechanism.
fn extract_toc_and_inject_ids(raw: &str) -> (String, Vec<TocEntry>) {
    let pc_opts = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES;

    // Pass 1 — ask pulldown-cmark where headings actually begin.
    let heading_starts: Vec<usize> = Parser::new_ext(raw, pc_opts)
        .into_offset_iter()
        .filter_map(|(event, span)| {
            matches!(event, Event::Start(Tag::Heading { .. })).then_some(span.start)
        })
        .collect();

    // Pass 2 — rebuild content, injecting `{#slug}` at each pulldown-cmark-verified heading.
    let mut out = String::with_capacity(raw.len() + heading_starts.len() * 24);
    let mut toc: Vec<TocEntry> = Vec::new();
    let mut slug_counts: HashMap<String, usize> = HashMap::new();
    let mut pos = 0usize; // read cursor into `raw`

    for line_start in heading_starts {
        // Emit everything between the last position and this heading.
        out.push_str(&raw[pos..line_start]);

        // Extract the heading line (up to but not including the trailing `\n`).
        let newline = raw[line_start..]
            .find('\n')
            .map_or(raw.len(), |p| line_start + p);
        let line = &raw[line_start..newline];

        // Use our ATX parser to get the level and plain text.
        if let Some((level, plain_text)) = parse_heading_line(line) {
            let (slug, injected) = extract_heading_id(line).map_or_else(
                || {
                    let base = slugify(plain_text);
                    let n = slug_counts.entry(base.clone()).or_insert(0);
                    let slug = if *n == 0 { base } else { format!("{base}-{n}") };
                    *n += 1;
                    let with_id = format!("{} {{#{slug}}}", line.trim_end());
                    (slug, with_id)
                },
                |id| (id.to_string(), line.to_string()),
            );
            toc.push(TocEntry {
                level,
                text: plain_text.to_string(),
                slug,
            });
            out.push_str(&injected);
        } else {
            // pulldown-cmark found a heading our ATX parser doesn't recognise
            // (e.g. setext style). Pass it through without injection.
            out.push_str(line);
        }

        // Advance past the newline (or to end-of-file).
        pos = if newline < raw.len() {
            out.push('\n');
            newline + 1
        } else {
            newline
        };
    }

    // Emit the tail of the file after the last heading.
    out.push_str(&raw[pos..]);

    (out, toc)
}

// ─── Image path absolutization ─────────────────────────────────────────────────

/// Rewrites relative image paths in Markdown to absolute `file://` URIs so they
/// load correctly regardless of platform CWD behaviour.
///
/// Paths that already carry a URI scheme (`http://`, `file://`, `data:`, …) are
/// left untouched. If a relative path cannot be resolved (file does not exist)
/// it is also left untouched so existing error behaviour is preserved.
///
/// Note: processes the raw text, so a path inside a fenced code block is also
/// rewritten if it matches the image syntax — an acceptable trade-off for the
/// cross-platform fix.
fn absolutize_image_paths(content: &str, base_dir: &Path) -> String {
    let mut out = String::with_capacity(content.len() + 128);
    let mut rest = content;

    while let Some(bang) = rest.find("![") {
        out.push_str(&rest[..bang]);
        rest = &rest[bang..];

        // Find `](`  — alt text must not contain `]`
        let Some(close_bracket) = rest.find("](") else {
            out.push_str(&rest[..2]);
            rest = &rest[2..];
            continue;
        };

        let prefix = &rest[..close_bracket + 2]; // `![alt](`
        rest = &rest[close_bracket + 2..];

        let Some(close_paren) = rest.find(')') else {
            out.push_str(prefix);
            continue;
        };

        let inner = &rest[..close_paren]; // path, possibly with `"title"`
        rest = &rest[close_paren + 1..];

        // Split optional title: `path "title"` or `path 'title'`
        let (raw_path, title_suffix) = inner
            .find(" \"")
            .or_else(|| inner.find(" '"))
            .map_or_else(|| (inner.trim(), ""), |i| (&inner[..i], &inner[i..]));

        let is_schemed = raw_path.starts_with("http://")
            || raw_path.starts_with("https://")
            || raw_path.starts_with("file://")
            || raw_path.starts_with("data:");

        out.push_str(prefix);
        if is_schemed {
            out.push_str(inner);
        } else if let Ok(abs) = base_dir.join(raw_path).canonicalize() {
            out.push_str(&path_to_file_uri(&abs));
            out.push_str(title_suffix);
        } else {
            // File not found — leave unchanged so the viewer shows a
            // broken-image placeholder rather than silently doing nothing.
            out.push_str(inner);
        }
        out.push(')');
    }
    out.push_str(rest);
    out
}

/// Turns raw file text into what `egui_commonmark` displays: embedded HTML is
/// rewritten (see `html_prep`), then heading `{#slug}` ids are injected (and the
/// TOC built), then relative image paths are absolutized.
fn prepare_markdown(
    raw: &str,
    base_dir: &Path,
    search_collapsible: bool,
) -> (String, Vec<TocEntry>) {
    let prepared = preprocess_html_with(raw, search_collapsible);
    let (id_injected, toc) = extract_toc_and_inject_ids(&prepared);
    (absolutize_image_paths(&id_injected, base_dir), toc)
}

fn path_to_file_uri(path: &Path) -> String {
    let s = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        let s = s.replace('\\', "/");
    }
    // Unix absolute paths start with `/`; Windows paths start with the drive letter.
    if s.starts_with('/') {
        format!("file://{s}") // file:// + /unix/path = file:///unix/path
    } else {
        format!("file:///{s}") // file:/// + C:/... = file:///C:/...
    }
}

// ─── Code fence pre-screening ────────────────────────────────────────────────

/// A code-fence problem detected before rendering.
enum FenceError {
    /// Total fence boundary count is odd — at least one fence is unclosed.
    OddCount { total: usize },
    /// An even-numbered boundary carries a language tag, meaning the preceding
    /// typed opener was never closed.
    UnclosedBeforeTyped {
        at_line: usize,
        at_header: String,
        prev_typed: Option<(usize, String)>,
    },
}

/// Scan `content` for fence problems without a full parser.
///
/// A fence boundary is a line with ≤ 3 leading spaces that starts with a triple backtick
/// or triple tilde (`~~~`) (matching the same rule used by `extract_toc_and_inject_ids`).
///
/// Two invariants are checked:
/// - **(a)** Total boundary count must be even — odd means at least one unclosed.
/// - **(b)** Every *typed* boundary (one with a language tag) must be
///   odd-numbered in sequence; an even-numbered typed boundary means the
///   previous typed opener was never closed.
fn validate_code_fences(content: &str) -> Result<(), FenceError> {
    let mut fence_count = 0usize;
    let mut last_typed: Option<(usize, String)> = None;

    for (idx, line) in content.lines().enumerate() {
        let line_num = idx + 1;
        let trimmed = line.trim_start_matches(' ');
        if line.len() - trimmed.len() > 3 {
            continue;
        }
        let is_backtick = trimmed.starts_with("```");
        let is_tilde = trimmed.starts_with("~~~");
        if !is_backtick && !is_tilde {
            continue;
        }

        fence_count += 1;

        // Strip all leading fence chars to get the language tag (if any).
        let lang = if is_backtick {
            trimmed.trim_start_matches('`')
        } else {
            trimmed.trim_start_matches('~')
        }
        .trim();

        if !lang.is_empty() {
            if fence_count.is_multiple_of(2) {
                // Even-numbered typed boundary → previous opener was never closed.
                return Err(FenceError::UnclosedBeforeTyped {
                    at_line: line_num,
                    at_header: trimmed.trim_end().to_string(),
                    prev_typed: last_typed,
                });
            }
            last_typed = Some((line_num, trimmed.trim_end().to_string()));
        }
    }

    if !fence_count.is_multiple_of(2) {
        return Err(FenceError::OddCount { total: fence_count });
    }
    Ok(())
}

/// Return a one-line description of a fence error suitable for `eprintln!`.
fn fence_err_brief(err: &FenceError, path: &Path) -> String {
    let p = path.display();
    match err {
        FenceError::OddCount { total } => {
            format!("malformed code fence in '{p}': odd boundary count ({total} total)")
        }
        FenceError::UnclosedBeforeTyped {
            at_line,
            at_header,
            prev_typed,
        } => match prev_typed {
            Some((pl, ph)) => format!(
                "malformed code fence in '{p}': unclosed fence between \
                     '{ph}' (line {pl}) and '{at_header}' (line {at_line})"
            ),
            None => format!(
                "malformed code fence in '{p}': unclosed fence before \
                     '{at_header}' (line {at_line})"
            ),
        },
    }
}

/// Build a markdown error page to display instead of a broken file.
/// The error renders nicely in the viewer; the user can fix the file and
/// press Cmd/Ctrl-R to reload.
fn fence_error_content(path: &Path, err: &FenceError) -> String {
    let file = path.display();
    let mod_key = MOD;
    match err {
        FenceError::OddCount { total } => format!(
            "# \u{26a0} Malformed Code Fence\n\n\
             Cannot render `{file}`.\n\n\
             **Odd number of fence boundaries detected ({total} total)** \u{2014} \
             at least one code fence is unclosed.\n\n\
             Please fix the file, then press **{mod_key}-R** to reload."
        ),
        FenceError::UnclosedBeforeTyped {
            at_line,
            at_header,
            prev_typed,
        } => {
            let location = match prev_typed {
                Some((pl, ph)) => {
                    format!("between `{ph}` on line {pl} and `{at_header}` on line {at_line}")
                }
                None => format!("before `{at_header}` on line {at_line}"),
            };
            format!(
                "# \u{26a0} Malformed Code Fence\n\n\
                 Cannot render `{file}`.\n\n\
                 **Unclosed code fence detected** \u{2014} {location}.\n\n\
                 Please fix the file, then press **{mod_key}-R** to reload."
            )
        }
    }
}

/// On Unix systems: if any stdio stream is a real terminal and `--foreground` is not
/// present, spawn a detached child with a new session and exit the parent immediately,
/// freeing the terminal. Errors (e.g. can't find the current exe) fall through silently
/// so that the viewer still runs in the foreground.
#[cfg(unix)]
fn detach_if_tty() {
    use std::io::IsTerminal;
    use std::os::unix::process::CommandExt;

    // Already non-interactive, or user explicitly requested foreground.
    let is_tty = std::io::stdin().is_terminal()
        || std::io::stdout().is_terminal()
        || std::io::stderr().is_terminal();
    if !is_tty {
        return;
    }
    let args_os: Vec<_> = env::args_os().collect();
    let already_detached = args_os
        .iter()
        .any(|a| a == "--foreground" || a == "-f" || a == "-fs" || a == "-sf");
    if already_detached {
        return;
    }

    let Ok(exe) = env::current_exe() else { return };

    // Build child args: skip argv[0] (exe), append marker so the child skips this block.
    let mut child_args: Vec<std::ffi::OsString> = args_os.into_iter().skip(1).collect();
    child_args.push("--foreground".into());

    let result = unsafe {
        std::process::Command::new(&exe)
            .args(&child_args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .pre_exec(|| {
                // Create a new session so the child is fully detached from the
                // controlling terminal.
                unsafe extern "C" {
                    fn setsid() -> std::ffi::c_int;
                }
                if setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            })
            .spawn()
    };
    if result.is_ok() {
        std::process::exit(0);
    }
    // spawn failed — fall through and run in the foreground.
}

/// Detect the preferred UI locale from the operating system.
/// Uses `sys-locale` which reads native OS APIs (`CFPreferences` on macOS,
/// `GetUserDefaultLocaleName` on Windows, POSIX env-vars on Linux).
/// Falls back to `"en"` when no usable locale is detected.
fn detect_locale() -> String {
    let locale = env::var("LOCALE").unwrap_or_else(|_| {
        // eprintln!("No LOCALE env var");
        env::var("LANG").unwrap_or_else(|_| {
            eprintln!("No LANG env var");
            sys_locale::get_locale()
                .filter(|loc| !loc.is_empty() && loc != "C" && loc != "POSIX")
                .unwrap_or_else(|| "en".to_string())
        })
    });
    match locale.as_str() {
        "no" | "no-NO" => "nb",
        other => other,
    }
    .to_string()
}

#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
fn main() -> eframe::Result<()> {
    // Hard size limit — refuse immediately so the GUI doesn't freeze.
    const MAX_BYTES: usize = 50_000_000;

    // Check for help first - automatically extracts from source comments
    let help = auto_help!();
    check_help_and_exit(&help);

    // Set the UI locale from the operating system before any translatable string is used.
    let locale = detect_locale();
    rust_i18n::set_locale(&locale);
    // dbg!(&locale);

    // Strip internal markers before processing positional arguments.
    let args: Vec<String> = env::args().filter(|a| !a.starts_with('-')).collect();

    if args.contains(&"--version".to_string()) || args.contains(&"-V".to_string()) {
        eprintln!(
            "{} version {}",
            PathBuf::from(&args[0])
                .file_name()
                .unwrap()
                .to_string_lossy(),
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }

    let selected_file: Option<PathBuf> = if args.len() > 1 {
        let input_path = Path::new(&args[1]);
        if !input_path.exists() {
            eprintln!("Error: Input file does not exist: {}", input_path.display());
            std::process::exit(1);
        }
        if input_path.is_dir() {
            eprintln!("Error: Input path is a directory: {}", input_path.display());
            let _ = env::set_current_dir(input_path);
            None
        } else if input_path.extension().is_some_and(|e| e == "md") {
            Some(input_path.to_path_buf())
        } else {
            eprintln!(
                "Error: Input file has unsupported extension: {}; must be `md`",
                input_path.display()
            );
            std::process::exit(1);
        }
    } else {
        None
    };

    let search_collapsible =
        env::args().any(|x| x == "-s" || x == "-sf" || x == "-fs" || x == "--search-collapsible");

    #[cfg(unix)]
    detach_if_tty();

    let (canonical_initial_path, raw_content, markdown_content, toc) = selected_file.map_or_else(
        || {
            let canonical_initial_path = env::current_dir()
                .unwrap_or_default()
                .canonicalize()
                .unwrap_or_default();
            let raw_content = t!(
                "welcome.instruction",
                cmd = MOD,
                open_files = OPEN_FILES_ICON,
                hist_back = HIST_BACK_ICON,
                hist_forward = HIST_FWD_ICON
            )
            .to_string();
            let (id_injected, toc) = extract_toc_and_inject_ids(&raw_content);
            let markdown_content = absolutize_image_paths(&id_injected, &canonical_initial_path);
            (canonical_initial_path, raw_content, markdown_content, toc)
        },
        |file| {
            let selected_path = PathBuf::from(&file);
            let canonical_initial_path = selected_path.canonicalize().unwrap_or(selected_path);
            let initial_base_dir = canonical_initial_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf();
            // Keep CWD in sync for canonicalize() calls inside the viewer.
            let _ = env::set_current_dir(&initial_base_dir);

            let raw_content =
                std::fs::read_to_string(&canonical_initial_path).unwrap_or_else(|_| {
                    format!(
                        "# Error\nFailed to read `{}`.",
                        canonical_initial_path.display()
                    )
                });
            let raw_content = if raw_content.len() > MAX_BYTES {
                let size_mb = raw_content.len() as f64 / 1e6;
                eprintln!(
                    "workman: file too large ({size_mb:.1} MB): {}",
                    canonical_initial_path.display()
                );
                format!(
                    "# ⚠ File Too Large\n\n\
                  Cannot render `{}`.\n\n\
                  **File size: {size_mb:.1} MB** — exceeds the {:.0} MB limit.\n\n\
                  Rendering files this large would make the UI unresponsive.\n\
                  Consider splitting the file into smaller sections.",
                    canonical_initial_path.display(),
                    MAX_BYTES as f64 / 1e6,
                )
            } else {
                raw_content
            };
            // Pre-screen for malformed code fences before any processing.
            let raw_content = match validate_code_fences(&raw_content) {
                Ok(()) => raw_content,
                Err(ref fence_err) => {
                    eprintln!(
                        "workman: {}",
                        fence_err_brief(fence_err, &canonical_initial_path)
                    );
                    fence_error_content(&canonical_initial_path, fence_err)
                }
            };
            let (markdown_content, toc) =
                prepare_markdown(&raw_content, &initial_base_dir, search_collapsible);
            (canonical_initial_path, raw_content, markdown_content, toc)
        },
    );

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
                    .expect("assets/icon.png is a valid PNG"),
            )
            .with_title(format!("workman: {}", canonical_initial_path.display())),
        ..Default::default()
    };

    // Load new default fonts
    let mut fonts = egui::FontDefinitions::default();

    let preferred_font = "Inter";

    // Register the font data
    fonts.font_data.insert(
        preferred_font.to_owned(),
        egui::FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-VariableFont_opsz,wght.ttf"
        ))
        .into(),
    );

    // Put it in proportional list
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .insert(0, preferred_font.to_owned());

    // Do the same for monospace
    let monospace_font = "Hack";
    fonts.font_data.insert(
        monospace_font.to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/Hack-Regular.ttf")).into(),
    );

    // Put it in monospace list
    fonts
        .families
        .get_mut(&egui::FontFamily::Monospace)
        .unwrap()
        .insert(0, monospace_font.to_owned());

    // Add it to end of proportional list as a fallback, e.g. displaying widgets in help screen
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .push(monospace_font.to_owned());

    let config = config::maybe_config();
    // eprintln!("config.theming={:#?}", config.as_ref().unwrap().theming);
    let config_default_theme = config
        .as_ref()
        .and_then(|config| config.theming.default_theme.clone());
    let (maybe_theme_name, maybe_base16_built_in): (Option<&'static str>, Option<&Base16>) =
        config_default_theme.map_or((None, None), |default_theme| {
            let maybe_base16_built_in = THEME_MAP.get(&default_theme);
            let built_in_name = Box::leak(default_theme.clone().into_boxed_str());
            if maybe_base16_built_in.is_some() {
                (Some(built_in_name), maybe_base16_built_in)
            } else {
                // eprintln!("Error retrieving theme {default_theme} from preloaded theme map");
                (Some(built_in_name), None)
            }
        });
    let maybe_base_16_loaded: Option<Base16> = if maybe_base16_built_in.is_none()
        && let Some(theme_name) = maybe_theme_name
    {
        retrieve_config_theme(config.as_ref(), theme_name)
    } else {
        None
    };

    let config_default_tm_theme_dark =
        config::maybe_config().and_then(|config| config.theming.default_tm_theme_dark);
    let (tm_theme_dark_name, tm_theme_dark_content): (Option<String>, Option<String>) =
        config_default_tm_theme_dark.map_or((None, None), |tm_dark_name| {
            let maybe_tm_theme_dir = config.and_then(|config| config.theming.tm_theme_dir);
            if let Some(tm_theme_dir) = maybe_tm_theme_dir {
                let path = PathBuf::from(tm_theme_dir).join(tm_dark_name.clone() + ".tmTheme");
                if path.exists() {
                    let maybe_theme = std::fs::read_to_string(&path);
                    match maybe_theme {
                        Ok(ref _theme) => (Some(tm_dark_name), maybe_theme.ok()),
                        Err(e) => {
                            eprintln!("failed to load {}: {e}", path.display());
                            (Some(tm_dark_name), None)
                        }
                    }
                } else {
                    eprintln!("Path {} not found", path.display());
                    (Some(tm_dark_name), None)
                }
            } else {
                (Some(tm_dark_name), None)
            }
        });
    let tm_dark_name = match (tm_theme_dark_name, &tm_theme_dark_content) {
        (None, _) => DEFAULT_SYNTECT_THEME_DARK,
        (Some(tm_dark_name), None) => {
            // No directory configured => prebuilt or default
            if SYNTECT_THEME_MAP.contains_key(&tm_dark_name) {
                Box::leak(tm_dark_name.into_boxed_str())
            } else {
                DEFAULT_SYNTECT_THEME_DARK
            }
        }
        (Some(tm_dark_name), Some(_)) => Box::leak(tm_dark_name.into_boxed_str()),
    };
    let config_default_tm_theme_light =
        config::maybe_config().and_then(|config| config.theming.default_tm_theme_light);
    let tm_theme_light: Option<&str> = config_default_tm_theme_light.map_or(
        Some(DEFAULT_SYNTECT_THEME_LIGHT),
        |default_tm_theme_light| Some(Box::leak(default_tm_theme_light.into_boxed_str())),
    );

    eframe::run_native(
        "Markdown Viewer",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_fonts(fonts);
            // Register our fast SVG loader BEFORE the first frame triggers
            // egui_commonmark's `prepare_show`, which calls `install_image_loaders`.
            // Since `install_image_loaders` skips any loader whose ID is already
            // registered, the default `SvgLoader::default()` (which calls the
            // 20-second `load_system_fonts()`) is never constructed.
            cc.egui_ctx
                .add_image_loader(Arc::new(fast_svg_loader::FastSvgLoader::new()));
            apply_style(&cc.egui_ctx, true);

            if let Some(base16) = maybe_base16_built_in {
                base16.apply(&cc.egui_ctx);
            } else if let Some(ref base16) = maybe_base_16_loaded {
                base16.apply(&cc.egui_ctx);
            } else {
                let () = &cc.egui_ctx.set_theme(egui::ThemePreference::System);
            }

            Ok(Box::new(MarkdownApp::new(
                markdown_content,
                raw_content,
                &canonical_initial_path,
                toc,
                cc.egui_ctx.clone(),
                search_collapsible,
                maybe_theme_name,
                maybe_base_16_loaded,
                Some(tm_dark_name),
                tm_theme_dark_content,
                tm_theme_light,
                // config::maybe_config(),
                // maybe_theme_name,
            )))
        }),
    )
}

// Retrieve from config directory if any
fn retrieve_config_theme(config: Option<&config::Config>, theme_name: &str) -> Option<Base16> {
    let maybe_base16_dir = config.and_then(|config| config.theming.base16_dir.as_ref());
    maybe_base16_dir.and_then(|base16_dir| {
        let path = PathBuf::from(base16_dir.clone()).join(theme_name.to_owned() + ".yaml");
        let path = if path.exists() {
            path
        } else {
            PathBuf::from(base16_dir).join(theme_name.to_owned() + ".yml")
        };
        if path.exists() {
            let maybe_theme = Base16::from_file(&path);
            match maybe_theme {
                Ok(ref _theme) => maybe_theme.ok(),
                Err(e) => {
                    eprintln!("failed to load {}: {e}", path.display());
                    None
                }
            }
        } else {
            eprintln!("Path {} (and .yaml) not found", path.display());
            None
        }
    })
}

/// Pending navigation action triggered by the toolbar buttons.
enum NavAction {
    None,
    Back,
    Close(egui::Context),
    Forward,
    History(usize),
}

/// The state holder for our egui app.
#[allow(clippy::struct_excessive_bools)]
struct MarkdownApp {
    /// Processed markdown text currently loaded (image paths absolutized, heading IDs injected).
    content: String,
    /// Raw file content as read from disk, used for text search.
    raw_content: String,
    /// The canonicalized path of the file we are viewing (so we know its parent folder).
    current_file_path: PathBuf,
    /// Required by `egui_commonmark` for rendering images/styles.
    cache: CommonMarkCache,
    /// Ordered list of visited file paths.
    history: Vec<PathBuf>,
    /// Current position within `history`.
    history_index: usize,
    /// Multiplicative scale applied to content text only (toolbar stays at 1×).
    font_scale: f32,
    /// Table of contents entries extracted from the current document.
    toc: Vec<TocEntry>,
    /// Whether the TOC side panel is visible.
    show_toc: bool,
    /// Whether the search bar is visible.
    search_open: bool,
    /// When `true`, the search text field will grab keyboard focus on the next frame.
    search_focus: bool,
    /// Whether the F1 help window is visible.
    show_help: bool,
    /// Separate cache for the help window's `CommonMarkViewer`.
    help_cache: CommonMarkCache,
    /// Heading positions extracted from `self.content` (content-relative bytes, not
    /// raw-content bytes). Used by `scroll_to_active_match` for accurate section
    /// navigation — avoids the offset errors that arise when comparing content
    /// positions against `toc.byte_start` values (which are raw-content relative).
    content_heading_positions: Vec<(usize, String)>, // (content byte start, slug)
    // ── File watcher ──────────────────────────────────────────────────────────
    /// Stored `egui::Context` so background threads can call `request_repaint()`.
    egui_ctx: egui::Context,
    /// Active `notify` watcher; keeping it alive via ownership.
    watcher: Option<notify::RecommendedWatcher>,
    /// Bridge-thread channel: a `()` arrives whenever the watched file changes.
    watcher_rx: Option<Receiver<()>>,
    /// When set, a brief "reloaded" notice is shown in the toolbar until this instant.
    last_reload: Option<Instant>,
    /// `true` until the end of the very first frame; used to request key-window focus
    /// on startup so that keyboard shortcuts work without requiring a prior mouse click.
    first_frame: bool,
    /// Whether the viewport-cache performance mode is active.
    /// Auto-enabled for documents ≥ [`VIEWPORT_CACHE_THRESHOLD`] bytes.
    /// Hidden from the UI for smaller documents.
    use_viewport_cache: bool,
    html: HtmlRenderer,
    /// Whether to expand disclosure widgets to expose them to the `egui_commonmark` search facility.
    search_collapsible: bool,
    /// The collection of available themes
    themes: HashMap<&'static str, Base16>,
    /// The current `Base16` markdown theme, if overriding `egui` defaults.
    current_theme: Option<&'static str>,
    /// The current `syntect` theme for code block highlighting in dark mode, if overriding the app default.
    syntect_theme_dark: Option<&'static str>,
    /// The current `syntect` theme for code block highlighting in light mode, if overriding the app default.
    syntect_theme_light: Option<&'static str>,
    theme_window_open: bool,
    syntect_filter: ThemeFilter,
    sample_cache: CommonMarkCache, // must know the syntect themes, as for your main cache
    base16_filter: Base16Filter,   // Default => All
    base16_load_error: Option<String>,
    // config: Option<Config>,
    // config_theme: Option<&'static str>,
}

impl MarkdownApp {
    #[expect(clippy::too_many_arguments)]
    fn new(
        content: String,
        raw_content: String,
        path: &Path,
        toc: Vec<TocEntry>,
        ctx: egui::Context,
        search_collapsible: bool,
        current_theme: Option<&'static str>,
        maybe_base16_loaded: Option<Base16>,
        syntect_theme_dark: Option<&'static str>,
        maybe_theme_dark_content: Option<String>,
        syntect_theme_light: Option<&'static str>,
        // config_theme: Option<&'static str>,
    ) -> Self {
        let content_len = &content.len();

        let mut cache = CommonMarkCache::default();

        let syntect_theme_dark: Option<&'static str> = maybe_theme_dark_content.map_or(
            Some(DEFAULT_SYNTECT_THEME_DARK),
            |dark_theme_content| {
                let dark_theme_name = syntect_theme_dark.unwrap();
                let add_syntax_theme_from_bytes = cache
                    .add_syntax_theme_from_bytes(dark_theme_name, dark_theme_content.as_bytes());
                match add_syntax_theme_from_bytes {
                    Ok(()) => syntect_theme_dark,
                    Err(e) => {
                        eprintln!("failed to add syntax theme `{dark_theme_name}` from bytes: {e}");
                        Some(DEFAULT_SYNTECT_THEME_DARK)
                    }
                }
            },
        );
        let mut app = Self {
            content,
            raw_content,
            current_file_path: path.to_path_buf(),
            cache,
            history: if path.is_file() {
                vec![path.to_path_buf()]
            } else {
                vec![]
            },
            history_index: 0,
            font_scale: 1.0,
            toc,
            show_toc: true,
            search_open: false,
            search_focus: false,
            show_help: false,
            help_cache: CommonMarkCache::default(),
            egui_ctx: ctx,
            content_heading_positions: Vec::new(),
            watcher: None,
            watcher_rx: None,
            last_reload: None,
            first_frame: true,
            use_viewport_cache: content_len >= &VIEWPORT_CACHE_THRESHOLD,
            html: HtmlRenderer::new(path_to_file_uri(path)),
            search_collapsible,
            themes: HashMap::default(),
            current_theme,
            syntect_theme_dark,
            syntect_theme_light,
            theme_window_open: false,
            syntect_filter: ThemeFilter::All,
            sample_cache: CommonMarkCache::default(),
            base16_filter: Base16Filter::All,
            base16_load_error: None,
            // config,
            // config_theme,
        };
        // eprintln!("CWD={}", std::env::current_dir().unwrap().display());
        if let Some(base_16_loaded) = maybe_base16_loaded {
            app.add_to_loaded_themes(base_16_loaded);
        }

        add_code_block_themes(&mut app.cache);
        add_code_block_themes(&mut app.sample_cache);
        app.start_watching();
        app.build_content_headings();
        app
    }

    /// (Re-)arm the file watcher for `self.current_file_path`.
    ///
    /// Drops any previous watcher first. Uses a `notify::RecommendedWatcher` — on macOS
    /// that is `FSEvents`, on Linux `inotify`, on Windows `ReadDirectoryChanges`. A bridge
    /// thread applies a short quiet-period debounce before forwarding a wake signal so
    /// that editors performing multi-step atomic writes don't trigger duplicate reloads.
    fn start_watching(&mut self) {
        // Drop the old watcher; this closes the raw channel sender in the bridge thread,
        // causing the bridge thread to exit its recv() loop cleanly.
        self.watcher = None;
        self.watcher_rx = None;

        let path = self.current_file_path.clone();

        // Raw notify channel: carries every individual FS event.
        let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<notify::Event>>();

        match notify::recommended_watcher(raw_tx) {
            Ok(mut w) => {
                if let Err(e) = w.watch(&path, RecursiveMode::NonRecursive) {
                    eprintln!("workman: cannot watch {}: {e}", path.display());
                    return;
                }

                // Bridge channel: only carries the "something changed" signal.
                let (bridge_tx, bridge_rx) = mpsc::channel::<()>();
                let ctx = self.egui_ctx.clone();
                let watch_path = path.clone();

                std::thread::Builder::new()
                    .name("workman-watcher".into())
                    .spawn(move || {
                        while let Ok(result) = raw_rx.recv() {
                            match result {
                                Ok(event) => {
                                    // Skip pure read events; react to anything that
                                    // changes file content or inode (atomic-write editors
                                    // like vim perform a rename/create rather than a
                                    // modify, so we must catch Create events too).
                                    if matches!(event.kind, notify::EventKind::Access(_)) {
                                        continue;
                                    }
                                    // Only act if our file is in the affected paths.
                                    // (Watching non-recursively means other files in the
                                    // same dir shouldn't arrive, but be defensive.)
                                    if !event.paths.iter().any(|p| p == &watch_path) {
                                        continue;
                                    }
                                    // Debounce: collect rapid follow-on events
                                    // (e.g. a write followed immediately by metadata
                                    // updates) into one reload.
                                    std::thread::sleep(Duration::from_millis(120));
                                    while raw_rx.try_recv().is_ok() {}
                                    if bridge_tx.send(()).is_err() {
                                        break; // receiver dropped; exit
                                    }
                                    ctx.request_repaint();
                                }
                                Err(e) => {
                                    eprintln!("workman: watcher error: {e}");
                                }
                            }
                        }
                    })
                    .expect("failed to spawn watcher bridge thread");

                self.watcher = Some(w);
                self.watcher_rx = Some(bridge_rx);
            }
            Err(e) => {
                eprintln!("workman: failed to create file watcher: {e}");
            }
        }
    }

    const fn can_go_back(&self) -> bool {
        self.history_index > 0
    }

    const fn can_go_forward(&self) -> bool {
        self.history_index + 1 < self.history.len()
    }

    /// Load `path` from disk and update content, TOC, raw content, and CWD.
    /// Returns `true` on success. Re-arms the file watcher for the new path.
    #[allow(clippy::cast_precision_loss)]
    fn load_file(&mut self, path: PathBuf) -> bool {
        match std::fs::read_to_string(&path) {
            Ok(raw) => {
                // Hard size limit: refuse files that would make the UI unusable.
                const MAX_BYTES: usize = 50_000_000; // 50 MB
                if raw.len() > MAX_BYTES {
                    let size_mb = raw.len() as f64 / 1e6;
                    let err = format!(
                        "# ⚠ File Too Large\n\n\
                         Cannot render `{}`.\n\n\
                         **File size: {size_mb:.1} MB** — exceeds the {:.0} MB limit.\n\n\
                         Rendering files this large would make the UI unresponsive.\n\
                         Consider splitting the file into smaller sections.",
                        path.display(),
                        MAX_BYTES as f64 / 1e6,
                    );
                    eprintln!(
                        "workman: file too large ({size_mb:.1} MB): {}",
                        path.display()
                    );
                    // Still "load" the error page so the watcher can detect a fix.
                    let base_dir = path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .to_path_buf();
                    let _ = env::set_current_dir(&base_dir);
                    let (id_injected, toc) = extract_toc_and_inject_ids(&err);
                    self.content = absolutize_image_paths(&id_injected, &base_dir);
                    self.raw_content = err;
                    self.html = HtmlRenderer::new(path_to_file_uri(&path));
                    self.current_file_path = path;
                    self.toc = toc;
                    self.cache = CommonMarkCache::default();
                    add_code_block_themes(&mut self.cache);
                    self.use_viewport_cache = self.content.len() >= VIEWPORT_CACHE_THRESHOLD;
                    self.start_watching();
                    self.build_content_headings();
                    return true;
                }
                // Pre-screen for malformed code fences.
                let raw = match validate_code_fences(&raw) {
                    Ok(()) => raw,
                    Err(ref fence_err) => {
                        eprintln!("workman: {}", fence_err_brief(fence_err, &path));
                        fence_error_content(&path, fence_err)
                    }
                };
                let base_dir = path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf();
                // Keep CWD in sync for future canonicalize() calls.
                let _ = env::set_current_dir(&base_dir);
                let (content, toc) = prepare_markdown(&raw, &base_dir, self.search_collapsible);
                self.content = content;
                self.raw_content = raw;
                self.html = HtmlRenderer::new(path_to_file_uri(&path));
                self.current_file_path = path;
                self.toc = toc;
                // Clear the cache so egui_commonmark doesn't carry over stale state.
                self.cache = CommonMarkCache::default();
                add_code_block_themes(&mut self.cache);
                self.use_viewport_cache = self.content.len() >= VIEWPORT_CACHE_THRESHOLD;
                // Re-arm the watcher for the (possibly different) new file.
                self.start_watching();
                // Build content-relative heading positions for accurate scroll navigation.
                self.build_content_headings();
                true
            }
            Err(e) => {
                eprintln!("Failed to read {}: {e}", path.display());
                false
            }
        }
    }

    /// Reload the current file from disk without changing history.
    fn reload_file(&mut self) -> bool {
        let path = self.current_file_path.clone();
        if path.is_file() {
            if self.load_file(path) {
                self.last_reload = Some(Instant::now());
            }
            true
        } else {
            false
        }
    }

    /// Navigate one step back in history. Returns `true` on success.
    fn go_back(&mut self) -> bool {
        if self.can_go_back() {
            self.history_index -= 1;
            let path = self.history[self.history_index].clone();
            // eprintln!(
            //     "Succeeded in self.go_back(), self.history_index={}",
            //     self.history_index
            // );
            self.load_file(path)
        } else {
            eprintln!(
                "Failed self.go_back(), self.history_index={0}",
                self.history_index
            );
            false
        }
    }

    /// Navigate one step forward in history. Returns `true` on success.
    fn go_forward(&mut self) -> bool {
        if self.can_go_forward() {
            self.history_index += 1;
            let path = self.history[self.history_index].clone();
            // eprintln!(
            //     "Succeeded in self.go_forward(), self.history_index={}",
            //     self.history_index
            // );
            self.load_file(path)
        } else {
            eprintln!(
                "Failed self.go_forward(), self.history_index={0}",
                self.history_index
            );
            false
        }
    }

    /// Close current file and try to go one step back in history, or failing that,
    /// one step forward.
    /// Returns `true` on success.
    fn close(&mut self, ctx: &egui::Context) -> bool {
        let remove_hist_index = self.history_index;
        let success = self.go_back() || self.go_forward();
        eprintln!(
            "success={success}, remove_hist_index={remove_hist_index}, self.history={:?}",
            self.history
        );
        if success {
            match self.history.len() {
                0 => false,
                1 => {
                    let _ = self.history.remove(0);
                    true
                }
                _ => {
                    let _ = self.history.remove(remove_hist_index);
                    if remove_hist_index < self.history.len() {
                        // Shift index back
                        self.history_index = self.history_index.saturating_sub(1);
                    }
                    true
                }
            }
        } else {
            match self.history.len() {
                0 => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    true
                }
                1 => {
                    let _ = self.history.remove(0);
                    self.welcome(ctx);
                    true
                }
                _ => {
                    unreachable!()
                }
            }
        }
    }

    fn welcome(&mut self, ctx: &egui::Context) {
        self.raw_content = t!(
            "welcome.instruction",
            cmd = MOD,
            open_files = OPEN_FILES_ICON,
            hist_back = HIST_BACK_ICON,
            hist_forward = HIST_FWD_ICON
        )
        .to_string();
        let canonical_initial_path = env::current_dir()
            .unwrap_or_default()
            .canonicalize()
            .unwrap_or_default();
        let (id_injected, toc) = extract_toc_and_inject_ids(&self.raw_content);
        self.content = absolutize_image_paths(&id_injected, &canonical_initial_path);
        self.toc = toc;
        self.show_toc = true;
        self.html = HtmlRenderer::new(path_to_file_uri(&canonical_initial_path));
        self.current_file_path = canonical_initial_path;
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "workman: {}",
            self.current_file_path.display()
        )));
    }

    /// Load a specific entry from the history stack. Returns `true` on success.
    fn load_history(&mut self, n: usize) -> bool {
        self.history_index = n;
        let path = self.history[self.history_index].clone();
        self.load_file(path)
    }

    /// Resolve a clicked relative link, load it, and push it onto history (discarding any
    /// forward entries). Returns `true` on success so the caller can update the window title.
    fn handle_link_click(&mut self, clicked_url: &str) -> bool {
        // Strip any fragment identifier (#anchor) — it's not part of the file path.
        let url_path = match clicked_url.split_once('#') {
            Some((path, _fragment)) => path,
            None => clicked_url,
        };
        if url_path.is_empty() {
            return false; // Pure anchor link with no file component.
        }

        // Resolve relative to the current file's directory.
        // `current_file_path` is always canonicalized (absolute), so `parent()` is reliable.
        let current_dir = self
            .current_file_path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let mut target_path = current_dir.join(url_path);
        // Canonicalize to resolve '..' / '.' and confirm the file exists.
        // Setting CWD first (in load_file) ensures canonicalize works for relative fallbacks.
        if let Ok(canonical) = target_path.canonicalize() {
            target_path = canonical;
        }

        if self.load_file(target_path.clone()) {
            // Discard forward history and record the new entry.
            self.history.truncate(self.history_index + 1);
            self.history.push(target_path);
            self.history_index = self.history.len() - 1;
            true
        } else {
            false
        }
    }

    /// Populate `content_heading_positions` by parsing `self.content` with
    /// pulldown-cmark. These content-relative positions are used instead of
    /// `toc.byte_start` (which is raw-content-relative) when scrolling to the
    /// section containing a search match.
    ///
    /// Pairs headings by document order with `self.toc` entries rather than
    /// relying on `id: Some(...)` — robust even if a heading's injected
    /// `{#slug}` is absent or unparsed for any reason.
    fn build_content_headings(&mut self) {
        self.content_heading_positions.clear();
        let opts = Options::ENABLE_TABLES
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_HEADING_ATTRIBUTES;
        let mut toc_idx = 0usize;
        for (event, span) in Parser::new_ext(&self.content, opts).into_offset_iter() {
            if let Event::Start(Tag::Heading { .. }) = event {
                if let Some(entry) = self.toc.get(toc_idx) {
                    self.content_heading_positions
                        .push((span.start, entry.slug.clone()));
                }
                toc_idx += 1;
            }
        }
    }

    fn load_and_register_history(&mut self, path: PathBuf) -> bool {
        let canonical = path.canonicalize().unwrap_or(path);
        if self.load_file(canonical.clone()) {
            self.history.truncate(self.history_index + 1);
            self.history.push(canonical);
            self.history_index = self.history.len() - 1;
            true
        } else {
            false
        }
    }

    fn remove_search(&mut self, id: egui::Id) {
        let search_query_mut = self.cache.search_query_mut(&id);
        let last_search_query = search_query_mut.clone();
        // Remove search matches
        *search_query_mut = String::new();
        self.cache.update_search_matches(&id, &self.content);
        // Restore search box contents for when box is reopened
        *self.cache.search_query_mut(&id) = last_search_query;
    }

    /// A compact button that opens a popup listing theme names.
    ///
    /// The popup stays open while the user clicks through entries and only closes
    /// on a click outside it or on `Esc`. `selected` is updated on each click and
    /// the returned `Response` reports `changed()` for that frame, so you can look
    /// up `themes[*selected]` and apply it on the next render.
    /// Button shows the current theme; popup has light/dark/all radios + list.
    fn theme_picker(&mut self, ui: &mut egui::Ui) {
        let button = ui
            .button(format!("🎨 {}", self.current_theme.unwrap_or("Default")))
            .on_hover_text("Choose a UI theme");

        Popup::menu(&button)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(180.0);

                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.base16_filter, Base16Filter::Light, "☀ Light");
                    ui.radio_value(&mut self.base16_filter, Base16Filter::Dark, "🌙 Dark");
                    ui.radio_value(&mut self.base16_filter, Base16Filter::All, "All");
                });
                ui.separator();

                // Built after the radios so a filter change applies this frame.
                let mut names: Vec<&'static str> = THEME_MAP
                    .into_iter()
                    .chain(self.themes.iter())
                    .filter(|(_, t)| self.base16_filter.allows(t.is_dark))
                    .map(|(&k, _)| k)
                    .collect();
                names.sort_unstable();

                ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    if ui
                        .selectable_label(self.current_theme.is_none(), "(egui default)")
                        .clicked()
                    {
                        self.current_theme = None;
                    }

                    for name in names {
                        if ui
                            .selectable_label(self.current_theme == Some(name), name)
                            .clicked()
                        {
                            self.current_theme = Some(name);
                            self.apply_theme(ui, name);
                        }
                    }
                });
            });
    }

    fn apply_theme(&self, ui: &egui::Ui, name: &str) {
        THEME_MAP
            .get(name)
            .or_else(|| {
                self.themes.get(name).or_else(|| {
                    eprintln!("Could not retrieve theme for key {name}");
                    None
                })
            })
            .unwrap()
            .apply(ui.ctx());
    }

    /// Toolbar button: just toggles the window.
    fn theme_button(&mut self, ui: &mut egui::Ui) {
        if ui.button("🎨").on_hover_text("Themes…").clicked() {
            self.theme_window_open = !self.theme_window_open;
        }
    }

    /// Call once per frame (e.g. at the end of `update`). Draws nothing if closed.
    fn theme_window(&mut self, ctx: &egui::Context) {
        if !self.theme_window_open {
            return;
        }

        // Esc closes the window, but not when it should only close an open
        // popup/combo box (those handle Esc themselves).
        if !egui::Popup::is_any_open(ctx)
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.theme_window_open = false;
            return;
        }

        // `.open()` needs `&mut bool`; use a local to avoid borrowing `self`.
        let mut open = true;

        egui::Window::new("Themes")
            .open(&mut open) // gives the title-bar close button
            .collapsible(false)
            .resizable(true)
            .default_size([420.0, 500.0])
            // Start near the top-right so the main document stays visible; still draggable.
            .default_pos(ctx.content_rect().right_top() + egui::vec2(-440.0, 8.0))
            .show(ctx, |ui| {
                let dark = ui.visuals().dark_mode;

                // --- Controls ---------------------------------------------
                ui.horizontal(|ui| {
                    ui.horizontal(|ui| {
                        ui.label("Main theme:");
                        self.theme_picker(ui);
                        if ui
                            .button("📂 Load…")
                            .on_hover_text("Load a Base16 YAML theme")
                            .clicked()
                            && let Some(name) = self.load_base16_file()
                        {
                            self.apply_theme(ui, name);
                        }
                    });
                    if let Some(err) = &self.base16_load_error {
                        ui.colored_label(ui.visuals().error_fg_color, err);
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Code themes:");
                    let which = if dark { "dark" } else { "light" };
                    ui.radio_value(
                        &mut self.syntect_filter,
                        ThemeFilter::Matching,
                        format!("Only {which}"),
                    );
                    ui.radio_value(&mut self.syntect_filter, ThemeFilter::All, "All");
                });
                let all = self.syntect_filter == ThemeFilter::All;

                // The slot being edited depends on the current light/dark mode.
                let (slot, default) = if dark {
                    (&mut self.syntect_theme_dark, DEFAULT_SYNTECT_THEME_DARK)
                } else {
                    (&mut self.syntect_theme_light, DEFAULT_SYNTECT_THEME_LIGHT)
                };

                ui.horizontal(|ui| {
                    ui.label(if dark {
                        "Dark code theme:"
                    } else {
                        "Light code theme:"
                    });
                    egui::ComboBox::from_id_salt("syntect_theme")
                        .selected_text(slot.unwrap_or("(default)"))
                        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                        .height(300.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(slot, None, format!("(default: {default})"));
                            for &(name, is_dark) in
                                syntect_themes().iter().filter(|(_, d)| all || *d == dark)
                            {
                                let label = if all {
                                    format!("{} {name}", if is_dark { "🌙" } else { "☀" })
                                } else {
                                    name.to_owned()
                                };
                                ui.selectable_value(slot, Some(name), label);
                            }
                        });
                });

                ui.separator();

                // --- Sample markdown --------------------------------------
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        CommonMarkViewer::new()
                            .syntax_theme_dark(
                                self.syntect_theme_dark
                                    .unwrap_or(DEFAULT_SYNTECT_THEME_DARK),
                            )
                            .syntax_theme_light(
                                self.syntect_theme_light
                                    .unwrap_or(DEFAULT_SYNTECT_THEME_LIGHT),
                            )
                            .show(ui, &mut self.sample_cache, SAMPLE_MD);
                    });
            });

        self.theme_window_open = open;
    }

    /// Blocking native file dialog (rfd), then parse, insert and select.
    fn load_base16_file(&mut self) -> Option<&'static str> {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Load Base16 theme")
            .add_filter("Base16 YAML", &["yaml", "yml"])
            .pick_file()
        else {
            return None; // cancelled
        };

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("custom");
        let result = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|src| parse_base16_yaml(&src, stem));

        match result {
            Ok(theme) => Some(self.add_to_loaded_themes(theme)),
            Err(e) => {
                self.base16_load_error = Some(format!("{}: {e}", path.display()));
                None
            }
        }
    }

    fn add_to_loaded_themes(&mut self, mut theme: Base16) -> &'static str {
        // Don't silently replace an existing theme of the same name.
        if self.themes.contains_key(theme.name) {
            theme.name = Box::leak(format!("{} (file)", theme.name).into_boxed_str());
        }
        let name = theme.name;

        // Make sure the new theme is actually visible in the list.
        if !self.base16_filter.allows(theme.is_dark) {
            self.base16_filter = Base16Filter::All;
        }

        self.themes.insert(name, theme);
        eprintln!("Inserted {name}");
        self.current_theme = Some(name);
        self.base16_load_error = None;
        name
    }
}

fn add_code_block_themes(cache: &mut CommonMarkCache) {
    for (name, syntax_yaml) in SYNTAX_STR {
        // eprintln!("name={}", name.to_lowercase());
        if let Err(e) = cache.add_syntax_from_str(syntax_yaml, Some(&name.to_lowercase())) {
            eprintln!("failed to load {name}: {e}");
        }
    }
    for (theme, plist_content) in &SYNTECT_THEME_MAP {
        cache
            .add_syntax_theme_from_bytes(*theme, plist_content.as_bytes())
            .unwrap();
    }
}

impl eframe::App for MarkdownApp {
    #[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        const RED_ALERT: usize = 10_000_000;
        const AMBER_ALERT: usize = 2_000_000;

        // ── Poll file watcher ─────────────────────────────────────────────────────────────
        // Drain all pending "file changed" signals from the bridge thread.
        // We set a flag rather than reloading immediately so the reload happens
        // after all panels are drawn this frame (avoids a mid-frame content swap).
        let mut watcher_triggered = false;
        if let Some(rx) = &self.watcher_rx {
            while rx.try_recv().is_ok() {
                watcher_triggered = true;
            }
        }

        // ── Pre-compute read-only snapshots for use in closures ───────────────────────────
        let can_go_back = self.can_go_back();
        let can_go_forward = self.can_go_forward();
        let back_tip = self
            .history_index
            .checked_sub(1)
            .and_then(|i| self.history.get(i))
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let forward_tip = self
            .history
            .get(self.history_index + 1)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let current_path_label = self.current_file_path.display().to_string();
        let font_scale = self.font_scale;
        let show_toc = self.show_toc;
        let search_open = self.search_open;
        let show_help = self.show_help;
        // Is the auto-reload notice still within its 2-second display window?
        let show_reload_notice = self
            .last_reload
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2));
        let doc_is_large = self.content.len() >= VIEWPORT_CACHE_THRESHOLD;
        let use_viewport_cache = self.use_viewport_cache & doc_is_large;

        // ── Mutable locals updated by keyboard / buttons, applied at end of frame ─────────
        let mut nav_action = NavAction::None;
        let mut open_files_requested = false;
        let mut refresh_requested = false;
        let mut new_font_scale = self.font_scale;
        let mut new_show_toc = show_toc;
        let mut new_search_open = search_open;
        let mut new_show_help = show_help;
        let mut new_use_viewport_cache = use_viewport_cache;
        let font_scale_label = format!("Aa {:.0}%", self.font_scale * 100.0);

        // ── Global keyboard shortcuts ─────────────────────────────────────────────────────
        // Collect all key states in one input() call to avoid re-locking the context.
        let (
            exit_key,
            close_file_key,
            open_key,
            zoom_in_key,
            zoom_out_key,
            zoom_reset_key,
            font_enlarge_key,
            font_reduce_key,
            font_reset_key,
            cmd_t,
            cmd_f,
            cmd_r,
            f1_key,
            search_escape,
            escape_key,
        ) = ui.ctx().input(|i| {
            use egui::Key;
            (
                i.modifiers.command && i.modifiers.alt && i.key_pressed(Key::Q),
                i.modifiers.command && i.key_pressed(Key::W),
                i.modifiers.command && i.key_pressed(Key::O),
                i.modifiers.command && i.key_pressed(Key::Equals),
                i.modifiers.command && i.key_pressed(Key::Minus),
                i.modifiers.command && i.key_pressed(Key::Z),
                i.modifiers.command && i.modifiers.shift && i.key_pressed(Key::A),
                i.modifiers.command && i.key_pressed(Key::A),
                i.modifiers.command && i.key_pressed(Key::Num0),
                i.modifiers.command && i.key_pressed(Key::T),
                i.modifiers.command && i.key_pressed(Key::F),
                i.modifiers.command && i.key_pressed(Key::R),
                i.key_pressed(Key::F1),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Escape),
            )
        });

        let dropped_files: Vec<PathBuf> = ui.ctx().input(|i| {
            if i.raw.dropped_files.is_empty() {
                vec![]
            } else {
                // Select the first '.md' file; ignore other extensions.
                let collect = i
                    .raw
                    .dropped_files
                    .iter()
                    .filter(|f| f.path().extension().is_some_and(|e| e == "md"))
                    .map(|f| f.path().to_owned())
                    .collect();
                if i.raw.dropped_files.is_empty() {
                    vec![]
                } else {
                    collect
                }
            }
        });

        // Act on shortcuts (zoom/font) only when text field does not have focus.
        let wants_text = ui.ctx().egui_wants_keyboard_input();

        if exit_key {
            eprintln!("Exiting app");
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        } else if close_file_key {
            nav_action = NavAction::Close(ui.ctx().clone());
        } else if open_key {
            open_files_requested = true;
        } else if !wants_text {
            if zoom_in_key {
                let z = ui.ctx().zoom_factor();
                ui.ctx().set_zoom_factor((z * 1.1).min(3.0));
            } else if zoom_out_key {
                let z = ui.ctx().zoom_factor();
                ui.ctx().set_zoom_factor((z / 1.1).max(0.4));
            } else if zoom_reset_key {
                ui.ctx().set_zoom_factor(1.0);
            } else if font_enlarge_key {
                new_font_scale = (new_font_scale * 1.1).min(3.0);
            } else if font_reduce_key {
                new_font_scale = (new_font_scale / 1.1).max(0.4);
            } else if font_reset_key {
                new_font_scale = 1.0;
            }
        }

        // Derive the viewer Id from the Ui context each frame. This scopes it
        // to the widget hierarchy (emilk's preferred pattern) and avoids
        // global hash collisions. Stable as long as the widget tree is stable.
        let id = ui.make_persistent_id(current_path_label);

        // Feature toggles (independent of text focus).
        if cmd_t {
            new_show_toc = !show_toc;
        }
        if cmd_f {
            new_search_open = !search_open;
            if new_search_open {
                // Run the previous search if any
                if !self.cache.search_query_mut(&id).is_empty() {
                    self.cache.update_search_matches(&id, &self.content);
                }
                // Opening the bar — request focus for the text field.
                self.search_focus = true;
            } else {
                self.remove_search(id);
            }
        }
        if cmd_r && self.current_file_path.is_file() {
            refresh_requested = true;
        }
        if f1_key || (escape_key && show_help) {
            new_show_help = !show_help;
        }
        // Esc should close the search box and remove the search.
        if new_search_open && search_escape {
            new_search_open = false;
            self.remove_search(id);
        }

        // ── Top panel: toolbar ────────────────────────────────────────────────────────────
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if ui
                        .button("⚙")
                        .on_hover_text(t!("toolbar.theme_system").to_string())
                        .clicked()
                    {
                        // if let Some(system_theme) = ui.ctx().system_theme() {
                        //     ui.ctx().set_theme(system_theme);
                        // } else {
                        //     eprintln!("Could not access system theme");
                        // }
                        self.current_theme = None;
                        ui.ctx().set_theme(egui::ThemePreference::System);
                        apply_style(ui.ctx(), true);
                    }
                    if ui
                        .button("🌙")
                        .on_hover_text(t!("toolbar.theme_dark").to_string())
                        .clicked()
                    {
                        self.current_theme = None;
                        ui.ctx().set_theme(egui::ThemePreference::Dark);
                        apply_style(ui.ctx(), true);
                    }
                    if ui
                        .button("☀")
                        .on_hover_text(t!("toolbar.theme_light").to_string())
                        .clicked()
                    {
                        self.current_theme = None;
                        ui.ctx().set_theme(egui::ThemePreference::Light);
                        apply_style(ui.ctx(), true);
                    }
                });

                self.theme_button(ui);
                ui.separator();

                if ui
                    .selectable_label(new_show_toc, "§")
                    .on_hover_text(t!("toolbar.toc_toggle", cmd = MOD).to_string())
                    .clicked()
                {
                    new_show_toc = !new_show_toc;
                }
                if ui
                    .selectable_label(new_search_open, "🔍")
                    .on_hover_text(t!("toolbar.search_toggle", cmd = MOD).to_string())
                    .clicked()
                {
                    new_search_open = !new_search_open;
                    if new_search_open {
                        self.search_focus = true;
                    }
                }

                ui.separator();

                if ui
                    .add_enabled(can_go_back, egui::Button::new(HIST_BACK_ICON))
                    .on_hover_text(&back_tip)
                    .clicked()
                {
                    nav_action = NavAction::Back;
                }

                if ui
                    .add_enabled(can_go_forward, egui::Button::new(HIST_FWD_ICON))
                    .on_hover_text(&forward_tip)
                    .clicked()
                {
                    nav_action = NavAction::Forward;
                }
                if ui
                    .button("X")
                    .on_hover_text(t!("toolbar.close_file_tip", cmd = MOD).to_string())
                    .clicked()
                {
                    nav_action = NavAction::Close(ui.ctx().clone());
                }
                if self.history.len() > 1 {
                    egui::ComboBox::from_id_salt("history_selector")
                        .selected_text(
                            self.history
                                .get(self.history_index)
                                .map(|p| p.display().to_string())
                                .unwrap_or_default(),
                        )
                        .show_ui(ui, |ui| {
                            for (index, path) in self.history.iter().enumerate() {
                                if ui
                                    .selectable_label(
                                        index == self.history_index,
                                        path.display().to_string(),
                                    )
                                    .clicked()
                                {
                                    nav_action = NavAction::History(index);
                                    ui.close();
                                }
                            }
                        });
                }

                ui.separator();
                if ui
                    .button("📖…")
                    .on_hover_text(t!("toolbar.open_files", cmd = MOD).to_string())
                    .clicked()
                {
                    open_files_requested = true;
                }
                if ui
                    .add_enabled(self.current_file_path.is_file(), egui::Button::new("🔄"))
                    .on_hover_text(t!("toolbar.reload", cmd = MOD).to_string())
                    .clicked()
                {
                    refresh_requested = true;
                }
                // Transient auto-reload notice — fades after 2 s.
                if show_reload_notice {
                    ui.separator();
                    ui.label(
                        egui::RichText::new(t!("status.reloaded").to_string())
                            .color(ui.visuals().weak_text_color()), // .small(),
                    )
                    .on_hover_text(t!("status.reloaded_tip").to_string());
                    // Keep asking for repaints until the notice expires.
                    ui.ctx().request_repaint_after(Duration::from_millis(250));
                }

                // Viewport-cache toggle — only shown for large documents.
                if doc_is_large {
                    ui.separator();
                    let (status_view_mode, status_view_mode_tip) = if use_viewport_cache {
                        ("status.cached", "status.cached_tip")
                    } else {
                        ("status.plain", "status.plain_tip")
                    };
                    if ui
                        .selectable_label(use_viewport_cache, t!(status_view_mode).to_string())
                        .on_hover_text(t!(status_view_mode_tip).to_string())
                        .clicked()
                    {
                        new_use_viewport_cache = !use_viewport_cache;
                        // Clear stale split-point cache so toggling ON triggers a fresh
                        // full render, and toggling OFF starts clean.
                        // self.cache = CommonMarkCache::default();
                        self.cache.clear_viewers();
                        add_code_block_themes(&mut self.cache);
                    }
                }

                // Persistent file-size badge for large documents.
                // Uses a coloured Frame (white text on solid background) for
                // legibility in both light and dark themes.
                {
                    let sz = self.raw_content.len();
                    if sz > AMBER_ALERT {
                        let size_str = if sz >= 1_000_000_000 {
                            format!("{:.1} GB", sz as f64 / 1e9)
                        } else {
                            format!("{:.1} MB", sz as f64 / 1e6)
                        };
                        let fill = if sz > RED_ALERT {
                            Color32::from_rgb(255, 73, 73) // red: very large
                        } else {
                            Color32::from_rgb(255, 200, 100) // amber: large
                        };
                        ui.separator();
                        egui::Frame::new()
                            .fill(fill)
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(5, 2))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(format!("\u{26a0} {size_str}"))
                                        .color(if sz > RED_ALERT {
                                            Color32::WHITE
                                        } else {
                                            Color32::BLACK
                                        })
                                        .strong(), // .small(),
                                )
                                .on_hover_text(t!("status.large_file_tip").to_string());
                            });
                    }
                }

                ui.separator();

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let cmd_opt: String = format!("{MOD}-{ALT}");

                    if ui
                        .button("🇽")
                        .on_hover_text(t!("toolbar.close", cmd = cmd_opt).to_string())
                        .clicked()
                    {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui
                        .selectable_label(new_show_help, "❓")
                        .on_hover_text(t!("toolbar.help").to_string())
                        .clicked()
                    {
                        new_show_help = !new_show_help;
                    }
                    ui.separator();
                    // Font scale controls (content text only; toolbar stays at 1×).
                    // RTL layout: first-added = rightmost, so render `+, label, −`
                    // to produce the visual sequence `− | 100% | +` left-to-right.
                    if ui
                        .small_button("+")
                        .on_hover_text(t!("toolbar.font_enlarge", cmd = MOD).to_string())
                        .clicked()
                    {
                        new_font_scale = (new_font_scale * 1.1).min(3.0);
                    }
                    if ui
                        .button(&font_scale_label)
                        .on_hover_text(t!("toolbar.font_reset", cmd = MOD).to_string())
                        .clicked()
                    {
                        new_font_scale = 1.0;
                    }
                    if ui
                        .small_button("−")
                        .on_hover_text(t!("toolbar.font_reduce", cmd = MOD).to_string())
                        .clicked()
                    {
                        new_font_scale = (new_font_scale / 1.1).max(0.4);
                    }
                    ui.separator();
                    // Zoom controls.
                    let zoom = ui.ctx().zoom_factor();
                    if ui
                        .small_button("+")
                        .on_hover_text(t!("toolbar.zoom_in", cmd = MOD).to_string())
                        .clicked()
                    {
                        ui.ctx().set_zoom_factor((zoom * 1.1).min(3.0));
                    }
                    if ui
                        .button(format!("↕{:.0}%", zoom * 100.0))
                        .on_hover_text(t!("toolbar.zoom_reset", cmd = MOD).to_string())
                        .clicked()
                    {
                        ui.ctx().set_zoom_factor(1.0);
                    }
                    if ui
                        .small_button("−")
                        .on_hover_text(t!("toolbar.zoom_out", cmd = MOD).to_string())
                        .clicked()
                    {
                        ui.ctx().set_zoom_factor((zoom / 1.1).max(0.4));
                    }
                    ui.separator();
                });
            });
        });

        // ── Top panel: search bar (shown when search is open) ─────────────────────────────
        if new_search_open {
            egui::Panel::top("search_bar").show(ui, |ui| {
                ui.horizontal(|ui| {
                    let text_color = if self.cache.search_regex_error(&id).is_some() {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().text_color()
                    };

                    ui.label("🔍");
                    let response = ui.add(
                        egui::TextEdit::singleline(self.cache.search_query_mut(&id))
                            .text_color(text_color)
                            .hint_text(t!("search.placeholder").to_string())
                            .desired_width(280.0),
                    );
                    if let Some(error) = &self.cache.search_regex_error(&id) {
                        response.clone().on_hover_text(error);
                    }

                    // Grab focus when the bar first opens.
                    if self.search_focus {
                        response.request_focus();
                        self.search_focus = false;
                    }

                    // Checked unconditionally (not gated on the text edit still
                    // having focus): a single-line TextEdit surrenders focus the
                    // moment Enter is pressed, so `response.has_focus()` would
                    // already be false here. We re-request focus below so that
                    // repeated Enter presses keep working without having to
                    // click back into the box each time.
                    let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if enter_pressed {
                        response.request_focus();
                    }

                    let mut search_options_changed = false;

                    let mut search_toggle =
                        |ui: &mut egui::Ui,
                         flag: SearchOptions,
                         label: egui::WidgetText,
                         tooltip: String| {
                            let selected = self.cache.search_options_mut(&id).contains(flag);

                            if ui
                                .selectable_label(selected, label)
                                .on_hover_text(tooltip)
                                .clicked()
                            {
                                self.cache.search_options_mut(&id).toggle(flag);
                                search_options_changed = true;
                            }
                        };

                    search_toggle(
                        ui,
                        SearchOptions::CASE_SENSITIVE,
                        "Aa".into(),
                        t!("search.case_sensitive_tip").to_string(),
                    );

                    search_toggle(
                        ui,
                        SearchOptions::WHOLE_WORD,
                        egui::RichText::new("wd").underline().into(),
                        t!("search.whole_word_tip").to_string(),
                    );

                    search_toggle(
                        ui,
                        SearchOptions::REGEX,
                        ".*".into(),
                        t!("search.regex_tip").to_string(),
                    );

                    if search_options_changed {
                        self.cache.update_search_matches(&id, &self.content);
                    }

                    if response.changed() {
                        self.cache.update_search_matches(&id, &self.content);
                    }
                    let match_count = self.cache.search_ranges(&id).len();
                    match self.cache.active_match(&id) {
                        Some(i) if match_count > 0 => ui.weak(format!("{} / {match_count}", i + 1)),
                        _ => ui.weak(t!("search.no_matches").to_string()),
                    };

                    // Prev / next buttons.
                    let has_matches = match_count > 0;
                    if ui
                        .add_enabled(
                            has_matches,
                            egui::Button::new(egui::RichText::new("\u{276e}").monospace()),
                        )
                        .on_hover_text(t!("search.prev_tip").to_string())
                        .clicked()
                        || (enter_pressed && ui.input(|i| i.modifiers.shift))
                    {
                        self.cache.go_to_match(&id, -1);
                    }
                    if ui
                        .add_enabled(
                            has_matches,
                            egui::Button::new(egui::RichText::new("\u{276f}").monospace()),
                        )
                        .on_hover_text(t!("search.next_tip").to_string())
                        .clicked()
                        || (enter_pressed && !ui.input(|i| i.modifiers.shift))
                    {
                        self.cache.go_to_match(&id, 1);
                    }

                    if ui
                        .button("X")
                        .on_hover_text(t!("search.close_tip").to_string())
                        .clicked()
                    {
                        new_search_open = false;
                    }

                    ui.separator();
                });
            });
        }

        // ── Left side panel: table of contents ───────────────────────────────────────────
        if new_show_toc {
            egui::Panel::left("toc")
                .resizable(true)
                .default_size(220.0)
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.strong(t!("toc.title").to_string());
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("toc_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for entry in &self.toc {
                                let indent = f32::from(entry.level.saturating_sub(1)) * 10.0;
                                // Reserve a background slot BEFORE the row so it sits
                                // behind the text in the draw-list (correct z-order).
                                let bg_idx = ui.painter().add(egui::Shape::Noop);
                                // Full-width row: click/hover sense covers the whole
                                // row, not just the label text.
                                let row_h = ui.spacing().interact_size.y;
                                let (row_rect, row_resp) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), row_h),
                                    egui::Sense::click(),
                                );
                                if row_resp.hovered() {
                                    ui.painter().set(
                                        bg_idx,
                                        egui::Shape::rect_filled(
                                            row_rect,
                                            egui::CornerRadius::ZERO,
                                            ui.visuals().widgets.hovered.weak_bg_fill,
                                        ),
                                    );
                                }
                                // Paint the truncated label in the indented portion.
                                // allocate_new_ui does not advance the parent cursor
                                // (row_rect was already accounted for above), and the
                                // explicit Layout prevents put()'s centred-and-justified
                                // default from pushing text to the middle of the panel.
                                if ui.is_rect_visible(row_rect) {
                                    // Paint the text directly so we control both position
                                    // and truncation without fighting the layout system.
                                    let color = ui.visuals().text_color();
                                    let font_id = egui::TextStyle::Body.resolve(ui.style());
                                    let max_w = (row_rect.width() - indent).max(0.0);
                                    let mut job = egui::text::LayoutJob::single_section(
                                        entry.text.clone(),
                                        egui::TextFormat {
                                            font_id,
                                            color,
                                            ..Default::default()
                                        },
                                    );
                                    job.wrap.max_rows = 1;
                                    job.wrap.overflow_character = Some('\u{2026}'); // …
                                    job.wrap.max_width = max_w;
                                    // Allow breaking mid-word so truncation is progressive
                                    // (letter by letter) rather than dropping a whole word.
                                    job.wrap.break_anywhere = true;
                                    let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
                                    let y = galley.size().y.mul_add(-0.5, row_rect.center().y);
                                    ui.painter().galley(
                                        egui::pos2(row_rect.min.x + indent, y),
                                        galley,
                                        color,
                                    );
                                }
                                if row_resp
                                    .on_hover_text(&entry.text)
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    // Sanity-check: confirm the injected ID is actually
                                    // present in the rendered content. If it is not, the
                                    // scroll will silently do nothing and the target will
                                    // be lost (egui_commonmark clears it at end of show).
                                    let pat = format!("{{#{}}}", entry.slug);
                                    if !self.content.contains(&pat) {
                                        eprintln!(
                                            "workman: TOC scroll MISS: \
                                             {pat} not found in rendered content. \
                                             slug={:?} heading={:?}",
                                            entry.slug, entry.text
                                        );
                                    }
                                    self.cache.scroll_to_heading(&id, Some(entry.slug.clone()));
                                }
                            }
                        });
                });
        } // end TOC panel

        // ── Central panel: the markdown document ──────────────────────────────────────────────
        egui::CentralPanel::default().show(ui, |ui| {
            // ── Style the scroll bar that show_scrollable will create internally ─
            // These settings propagate into the inner ScrollArea because
            // show_scrollable inherits ui.style() from this outer ui.
            customise_scrollbar(ui);

            // ── Keyboard scrolling ───────────────────────────────────────────
            let user_scrolled = self.cache.handle_keyboard_scrolling(&id, ui);

            // ── Font scale ──────────────────────────────────────────────────────────
            // Applying to the outer ui propagates into show_scrollable's
            // inner ScrollArea since it inherits style from us.
            if (font_scale - 1.0).abs() > 0.005 {
                use egui::{FontFamily, FontId, TextStyle};
                let s = ui.style_mut();
                let base_body = s.text_styles.get(&TextStyle::Body).map_or(14.0, |f| f.size);
                let base_mono = s
                    .text_styles
                    .get(&TextStyle::Monospace)
                    .map_or(12.0, |f| f.size);
                let base_heading = s
                    .text_styles
                    .get(&TextStyle::Heading)
                    .map_or(21.0, |f| f.size);
                let base_small = s
                    .text_styles
                    .get(&TextStyle::Small)
                    .map_or(10.0, |f| f.size);
                s.text_styles.insert(
                    TextStyle::Body,
                    FontId::new(base_body * font_scale, FontFamily::Proportional),
                );
                s.text_styles.insert(
                    TextStyle::Monospace,
                    FontId::new(base_mono * font_scale, FontFamily::Monospace),
                );
                s.text_styles.insert(
                    TextStyle::Heading,
                    FontId::new(base_heading * font_scale, FontFamily::Proportional),
                );
                s.text_styles.insert(
                    TextStyle::Small,
                    FontId::new(base_small * font_scale, FontFamily::Proportional),
                );
            }

            let (match_bg, active_bg) = if ui.visuals().dark_mode {
                (
                    egui::Color32::from_rgb(30, 115, 105),
                    egui::Color32::from_rgb(95, 75, 165),
                )
            } else {
                (
                    egui::Color32::from_rgb(140, 220, 210),
                    egui::Color32::from_rgb(185, 165, 240),
                )
            };

            // ── Render with or without viewport culling ─────────────────────────────────────────
            // show_scrollable does a full render on first open to populate
            // split-point and heading-position caches, then culls to the
            // visible viewport on all subsequent frames.  The source_id is
            // keyed to the file path so navigating to a new file resets state.
            // RenderHtmlFn is `dyn Fn(..) + 'static`, so the closure must own a
            // (cheap, Rc-backed) clone rather than borrow `self.html`.
            let html = self.html.clone();
            CommonMarkViewer::new()
                .syntax_theme_dark(
                    self.syntect_theme_dark
                        .unwrap_or(DEFAULT_SYNTECT_THEME_DARK),
                )
                .syntax_theme_light(
                    self.syntect_theme_light
                        .unwrap_or(DEFAULT_SYNTECT_THEME_LIGHT),
                )
                .search_match_color(match_bg)
                .search_active_match_color(active_bg)
                .enable_scroll_to_heading(true)
                .render_html_fn(Some(&move |ui, chunk| html.render(ui, chunk)))
                .show_scrollable(
                    id,
                    ui,
                    &mut self.cache,
                    &CommonMarkScrollOptions::default().viewport_cache(new_use_viewport_cache),
                    &self.content,
                );

            self.cache
                .sync_scrollable_active_match(&id, self.use_viewport_cache, user_scrolled);
        });

        // ── Intercept link clicks from egui_commonmark ────────────────────────────────────
        // egui_commonmark dispatches link clicks by pushing OutputCommand::OpenUrl onto the
        // context output. Intercept here: handle relative links ourselves, re-queue external ones.
        let clicked_url = ui.ctx().output_mut(|o| {
            let pos = o
                .commands
                .iter()
                .position(|cmd| matches!(cmd, egui::OutputCommand::OpenUrl(_)));
            pos.map(|idx| {
                if let egui::OutputCommand::OpenUrl(open_url) = o.commands.remove(idx) {
                    open_url.url
                } else {
                    unreachable!()
                }
            })
        });

        // ── Execute navigation ────────────────────────────────────────────────────────────
        let navigated = if let Some(url) = clicked_url {
            if url.starts_with("http://") || url.starts_with("https://") {
                // Re-queue external links for the platform to open in the browser.
                ui.ctx().open_url(egui::output::OpenUrl::new_tab(url));
                false
            } else {
                self.handle_link_click(&url)
            }
        } else if open_files_requested {
            let start_dir = if self.current_file_path.is_dir() {
                self.current_file_path.clone()
            } else {
                self.current_file_path
                    .parent()
                    .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
            };
            FileDialog::new()
                .add_filter("Markdown", &["md", "markdown"])
                .set_directory(&start_dir)
                .pick_files()
                .is_some_and(|paths| {
                    let mut success = false;
                    for path in paths {
                        if self.load_and_register_history(path) && !success {
                            success = true;
                        }
                    }
                    success
                })
        } else if !dropped_files.is_empty() {
            let mut success = false;
            for path in dropped_files {
                if self.load_and_register_history(path) && !success {
                    success = true;
                }
            }
            success
        } else if refresh_requested {
            // No history change on refresh.
            let _ = self.reload_file();
            false
        } else if watcher_triggered {
            // Auto-reload: the file watcher signalled a change on disk.
            // Reload without history change, then stamp the notice timer.
            if self.reload_file() {
                self.last_reload = Some(Instant::now());
            }
            false // not a navigation; title stays the same
        } else {
            match nav_action {
                NavAction::Back => self.go_back(),
                NavAction::Forward => self.go_forward(),
                NavAction::History(n) => self.load_history(n),
                NavAction::None => false,
                NavAction::Close(ctx) => self.close(&ctx),
            }
        };

        // If load_file() ran this frame (navigation, manual reload, or watcher-triggered
        // auto-reload), it already set `self.use_viewport_cache` to the correct value for
        // the newly loaded document.  Propagate that back into `new_use_viewport_cache` so
        // the frame-end commit below does not overwrite it with the stale pre-load snapshot
        // (which would be `false` whenever the *previous* file was small or had caching
        // disabled, causing large incoming files to open without viewport caching).
        if navigated || watcher_triggered {
            new_use_viewport_cache = self.use_viewport_cache;
        }

        // Reclaim key-window focus after the native file dialog releases it;
        // without this the next keyboard shortcut typically needs two presses.
        // if open_files_requested {
        //     ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        // }

        // ── Commit state changes ──────────────────────────────────────────────────────────
        self.font_scale = new_font_scale;
        self.show_toc = new_show_toc;
        self.search_open = new_search_open;
        self.show_help = new_show_help;
        // Final safety guard: never enable caching for docs below the threshold.
        // When load_file() ran this frame, new_use_viewport_cache was already synced to
        // self.use_viewport_cache (see above), so both the large→small case (would
        // wrongly re-enable cache) and the small/disabled→large case (would wrongly
        // leave cache off) are handled before we reach this point.  The `&& threshold`
        // check here is a belt-and-braces fallback for any path we may have missed.
        self.use_viewport_cache =
            new_use_viewport_cache && (self.content.len() >= VIEWPORT_CACHE_THRESHOLD);

        if navigated {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(format!(
                    "workman: {}",
                    self.current_file_path.display()
                )));
        }

        // ── Help window (floats above everything else) ────────────────────────────────────
        if self.show_help {
            let mut open = true;
            egui::Window::new(t!("help.window_title").to_string())
                .auto_sized()
                .collapsible(false)
                .open(&mut open)
                .show(ui, |ui| {
                    customise_scrollbar(ui);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let help_text =
                            t!("help.text", prev = "\u{276e}", next = "\u{276f}").to_string();
                        CommonMarkViewer::new().show(ui, &mut self.help_cache, &help_text);
                    });
                });
            if !open {
                self.show_help = false;
            }
        }
        // ui.ctx().request_repaint_after(Duration::from_millis(250));
        // ── On the very first frame, claim key-window focus so shortcuts work immediately
        // without requiring the user to click first (macOS key-window / winit issue).
        if self.first_frame {
            // dbg!(&self.first_frame);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
            self.first_frame = false;
        }

        self.theme_window(ui.ctx());
    }
}

fn customise_scrollbar(ui: &mut egui::Ui) {
    let scroll = &mut ui.style_mut().spacing.scroll;
    scroll.floating = true;
    scroll.content_margin = egui::Margin::same(10);
    scroll.bar_inner_margin = 30.0;
    scroll.bar_width = 10.0;
    scroll.dormant_handle_opacity = 0.15;
    scroll.interact_handle_opacity = 0.55;
    scroll.active_handle_opacity = 0.80;
}
