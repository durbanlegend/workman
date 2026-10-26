//! Base16 colour schemes for egui.
//!
//! Parses Base16 / tinted-theming scheme files (without a YAML library) and turns them into
//! egui [`Visuals`].
//!
//! Two layouts are understood:
//!
//! * old: `scheme: "Name"` / `base00: "181818"` ...
//! * new: `name: "Name"` / `variant: "dark"` / `palette:` with `base00: "181818"` ...
//!
//! Only flat `key: value` lines are read, which covers every published scheme file. Extra keys
//! (such as the `base10`..`base17` of Base24 schemes) are ignored.

use eframe::egui;
use egui::{Color32, Stroke, Visuals};
use std::{error::Error, fmt, fs, io, path::Path};

/// Why a scheme couldn't be loaded.
#[derive(Debug)]
pub enum Base16Error {
    /// The file couldn't be read.
    Io(io::Error),
    /// The text isn't a usable scheme.
    Parse(String),
}

impl fmt::Display for Base16Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Parse(msg) => write!(f, "{msg}"),
        }
    }
}

impl Error for Base16Error {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Parse(_) => None,
        }
    }
}

impl From<io::Error> for Base16Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Clone, Debug)]
pub struct Base16 {
    pub name: &'static str,
    pub is_dark: bool,
    /// base00..=base0F, indexed 0..=15
    pub c: [Color32; 16],
}

impl Base16 {
    /// Loads a scheme from a file. A scheme without a name is called after the file.
    ///
    /// # Errors
    /// Fails if the file can't be read or isn't a usable scheme.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Base16Error> {
        let path = path.as_ref();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("base16");
        Self::from_yaml_named(&fs::read_to_string(path)?, stem)
    }

    /// Parses a scheme; one without a name is called `base16`.
    ///
    /// # Errors
    /// Fails if a colour is missing or malformed.
    pub fn from_yaml(src: &str) -> Result<Self, Base16Error> {
        Self::from_yaml_named(src, "base16")
    }

    /// Parses a scheme; one without a name is called `fallback_name`.
    ///
    /// The name is leaked, because themes are keyed by `&'static str`; that's a few bytes per
    /// scheme the user chooses to load.
    ///
    /// # Errors
    /// Fails if a colour is missing or malformed.
    pub fn from_yaml_named(src: &str, fallback_name: &str) -> Result<Self, Base16Error> {
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
            let key = key.trim().trim_matches(['"', '\'']);
            let value = yaml_scalar(value);

            match key {
                "name" | "scheme" => name = Some(value),
                "variant" => variant = Some(value.to_ascii_lowercase()),
                _ => {
                    // Ignore keys that aren't `base00`..`base0F` (e.g. Base24's `base10`+).
                    if let Some(hex) = key.strip_prefix("base").filter(|h| h.len() == 2)
                        && let Ok(i) = usize::from_str_radix(hex, 16)
                        && i < c.len()
                    {
                        c[i] = Some(
                            parse_hex(&value)
                                .map_err(|e| Base16Error::Parse(format!("{key}: {e}")))?,
                        );
                    }
                }
            }
        }

        let missing: Vec<String> = (0..16)
            .filter(|&i| c[i].is_none())
            .map(|i| format!("base{i:02X}"))
            .collect();
        if !missing.is_empty() {
            return Err(Base16Error::Parse(format!(
                "missing {}",
                missing.join(", ")
            )));
        }
        let c: [Color32; 16] = c.map(|x| x.unwrap_or(Color32::PLACEHOLDER));

        // Prefer an explicit `variant`; otherwise judge by base00 (the background).
        let is_dark = match variant.as_deref() {
            Some("dark") => true,
            Some("light") => false,
            _ => luminance(c[0]) < 0.5_f32,
        };

        let name = name.unwrap_or_else(|| fallback_name.to_owned());
        Ok(Self {
            name: Box::leak(name.into_boxed_str()),
            is_dark,
            c,
        })
    }

    /// Apply this scheme to an egui context.
    ///
    /// egui keeps separate styles for Light and Dark and follows the OS theme by
    /// default, so plain `set_visuals` only affects whichever theme is active at
    /// that instant. Instead, write to the matching theme slot and pin the
    /// theme preference so egui doesn't switch away from it.
    pub fn apply(&self, ctx: &egui::Context) {
        let theme = if self.is_dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        };
        ctx.set_visuals_of(theme, self.visuals());
        ctx.set_theme(theme); // pins ThemePreference to Dark/Light (ignores OS setting)
    }

    #[must_use]
    pub fn visuals(&self) -> Visuals {
        let c = &self.c;
        let (bg, bg_alt, bg_sel, border, muted) = (c[0x0], c[0x1], c[0x2], c[0x3], c[0x4]);
        let (fg, fg_hi, fg_max) = (c[0x5], c[0x6], c[0x7]);
        let (red, orange, blue) = (c[0x8], c[0x9], c[0xD]);

        // Start from egui's own defaults so every field we don't touch stays sane.
        let mut v = if self.is_dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };

        // Backgrounds
        v.panel_fill = bg;
        v.window_fill = bg;
        v.window_stroke = Stroke::new(1.0_f32, border);
        v.extreme_bg_color = bg_alt; // text edits, scroll areas, plot bg
        v.faint_bg_color = bg_alt; // striped table rows
        v.code_bg_color = bg_alt; // inline code / code blocks (markdown!)

        // Semantic colours
        v.hyperlink_color = blue;
        v.warn_fg_color = orange;
        v.error_fg_color = red;
        v.selection.bg_fill = blue;
        v.selection.stroke = Stroke::new(1.0_f32, bg);

        // Widget states. Text colour is taken from `fg_stroke`.
        let w = &mut v.widgets;

        w.noninteractive.bg_fill = bg;
        w.noninteractive.weak_bg_fill = bg;
        w.noninteractive.bg_stroke = Stroke::new(1.0_f32, bg_sel); // separators, frames
        w.noninteractive.fg_stroke = Stroke::new(1.0_f32, fg);

        w.inactive.bg_fill = bg_alt;
        w.inactive.weak_bg_fill = bg_alt; // idle buttons
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0_f32, fg);

        w.hovered.bg_fill = bg_sel;
        w.hovered.weak_bg_fill = bg_sel;
        w.hovered.bg_stroke = Stroke::new(1.0_f32, border);
        w.hovered.fg_stroke = Stroke::new(1.5_f32, fg_hi);

        w.active.bg_fill = border;
        w.active.weak_bg_fill = border;
        w.active.bg_stroke = Stroke::new(1.0_f32, muted);
        w.active.fg_stroke = Stroke::new(2.0_f32, fg_max);

        w.open.bg_fill = bg_alt;
        w.open.weak_bg_fill = bg_alt;
        w.open.bg_stroke = Stroke::new(1.0_f32, border);
        w.open.fg_stroke = Stroke::new(1.0_f32, fg_hi);

        v
    }
}

/// The value of a `key: value` line: unquoted, without any trailing comment.
fn yaml_scalar(v: &str) -> String {
    let v = v.trim();
    if let Some(q) = v.chars().next().filter(|&c| c == '"' || c == '\'') {
        return v[1..].split(q).next().unwrap_or("").to_owned();
    }
    v.split(" #").next().unwrap_or("").trim().to_owned()
}

/// Parses `rrggbb` or `#rrggbb`.
fn parse_hex(s: &str) -> Result<Color32, String> {
    let h = s.trim().trim_start_matches('#');
    // Checking for ASCII hex digits first also makes the byte-index slicing below safe.
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("expected 6 hex digits, got `{s}`"));
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map_err(|e| e.to_string());
    Ok(Color32::from_rgb(p(0)?, p(2)?, p(4)?))
}

fn luminance(c: Color32) -> f32 {
    0.0722_f32.mul_add(
        f32::from(c.b()),
        0.7152_f32.mul_add(f32::from(c.g()), 0.2126 * f32::from(c.r())),
    ) / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(indent: &str, bg: &str) -> String {
        let mut s = String::new();
        for i in 0..16 {
            let colour = if i == 0 {
                bg.to_owned()
            } else {
                format!("{:02x}{:02x}{:02x}", i * 10, i * 10, i * 10)
            };
            s.push_str(&format!("{indent}base{i:02X}: \"{colour}\"\n"));
        }
        s
    }

    fn old_layout(bg: &str) -> String {
        format!("scheme: \"Old\"\n{}", palette("", bg))
    }

    fn new_layout(extra: &str, bg: &str) -> String {
        format!("name: \"New\"\n{extra}palette:\n{}", palette("  ", bg))
    }

    #[test]
    fn old_and_new_layouts_parse() {
        let old = Base16::from_yaml(&old_layout("101010")).unwrap();
        assert_eq!(old.name, "Old");
        assert_eq!(old.c[0], Color32::from_rgb(0x10, 0x10, 0x10));
        assert_eq!(old.c[1], Color32::from_rgb(10, 10, 10));
        assert_eq!(old.c[15], Color32::from_rgb(150, 150, 150));

        let new = Base16::from_yaml(&new_layout("", "101010")).unwrap();
        assert_eq!(new.name, "New");
        assert_eq!(new.c, old.c);
    }

    #[test]
    fn hash_prefixed_colours_are_accepted() {
        let src = new_layout("", "#ffffff");
        assert_eq!(Base16::from_yaml(&src).unwrap().c[0], Color32::WHITE);
    }

    #[test]
    fn name_defaults_to_base16() {
        let src = format!("palette:\n{}", palette("  ", "000000"));
        assert_eq!(Base16::from_yaml(&src).unwrap().name, "base16");
    }

    #[test]
    fn variant_wins_over_luminance() {
        let light_on_black = new_layout("variant: \"light\"\n", "000000");
        assert!(!Base16::from_yaml(&light_on_black).unwrap().is_dark);
        let dark_on_white = new_layout("variant: \"dark\"\n", "ffffff");
        assert!(Base16::from_yaml(&dark_on_white).unwrap().is_dark);
    }

    #[test]
    fn darkness_is_inferred_from_the_background() {
        assert!(Base16::from_yaml(&old_layout("101010")).unwrap().is_dark);
        assert!(!Base16::from_yaml(&old_layout("f0f0f0")).unwrap().is_dark);
        // Pure green is bright (luminance ~0.72) even though the red and blue are 0.
        assert!(!Base16::from_yaml(&old_layout("00ff00")).unwrap().is_dark);
    }

    #[test]
    fn missing_colour_is_an_error_naming_it() {
        let src = old_layout("000000").replace("base0C", "baseXX");
        let err = Base16::from_yaml(&src).unwrap_err().to_string();
        assert!(err.contains("base0C"), "{err}");
    }

    #[test]
    fn malformed_colours_are_errors_not_panics() {
        for bad in [
            "",
            "fff",
            "ggggg0",
            "1234567",
            "+f0000",
            "\u{e9}\u{e9}\u{e9}",
            "a\u{e9}\u{20ac}a",
        ] {
            assert!(
                Base16::from_yaml(&old_layout(bad)).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn invalid_yaml_is_an_error() {
        assert!(Base16::from_yaml("{ not: [valid").is_err());
        assert!(Base16::from_yaml("").is_err());
    }

    #[test]
    fn from_file_reports_missing_files() {
        assert!(Base16::from_file("/definitely/not/here.yaml").is_err());
    }

    #[test]
    fn visuals_follow_the_scheme() {
        let dark = Base16::from_yaml(&old_layout("101010")).unwrap();
        let v = dark.visuals();
        assert!(v.dark_mode);
        assert_eq!(v.panel_fill, dark.c[0]);
        assert_eq!(v.hyperlink_color, dark.c[0xD]);
        assert_eq!(v.error_fg_color, dark.c[0x8]);

        let light = Base16::from_yaml(&old_layout("f0f0f0")).unwrap();
        assert!(!light.visuals().dark_mode);
    }

    #[test]
    fn luminance_is_between_zero_and_one() {
        assert!(luminance(Color32::BLACK).abs() < f32::EPSILON);
        assert!((luminance(Color32::WHITE) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn yaml_scalar_forms() {
        assert_eq!(yaml_scalar("  plain  "), "plain");
        assert_eq!(yaml_scalar("\"double quoted\""), "double quoted");
        assert_eq!(yaml_scalar("'single quoted'"), "single quoted");
        assert_eq!(yaml_scalar("value # trailing comment"), "value");
        assert_eq!(yaml_scalar("\"with # hash\" # comment"), "with # hash");
        assert_eq!(yaml_scalar(""), "");
        assert_eq!(yaml_scalar("\"unterminated"), "unterminated");
    }

    #[test]
    fn hex_colours() {
        assert_eq!(parse_hex("ff8000"), Ok(Color32::from_rgb(255, 128, 0)));
        assert_eq!(parse_hex("#FF8000"), Ok(Color32::from_rgb(255, 128, 0)));
        assert_eq!(parse_hex(" 000000 "), Ok(Color32::from_rgb(0, 0, 0)));
        for bad in ["", "fff", "ff800000", "gg0000", "+f0000", "-f0000"] {
            assert!(parse_hex(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn missing_name_uses_the_fallback() {
        let src = old_layout("000000").replace("scheme: \"Old\"\n", "");
        assert_eq!(Base16::from_yaml(&src).unwrap().name, "base16");
        assert_eq!(Base16::from_yaml_named(&src, "mine").unwrap().name, "mine");
    }

    #[test]
    fn all_missing_colours_are_listed() {
        let src = old_layout("000000")
            .replace("base03:", "x03:")
            .replace("base0F:", "x0F:");
        let err = Base16::from_yaml(&src).unwrap_err().to_string();
        assert!(err.contains("base03") && err.contains("base0F"), "{err}");
    }

    #[test]
    fn a_bad_colour_names_its_key() {
        let err = Base16::from_yaml(&old_layout("zzzzzz"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("base00"), "{err}");
    }

    #[test]
    fn base24_extras_comments_and_blank_lines_are_ignored() {
        let src = format!(
            "# comment\n\n{}base10: \"000000\"\nbase17: \"ffffff\"\n# trailing\n",
            old_layout("000000")
        );
        assert!(Base16::from_yaml(&src).is_ok());
    }

    #[test]
    fn variant_is_case_insensitive() {
        let src = new_layout("variant: \"DARK\"\n", "ffffff");
        assert!(Base16::from_yaml(&src).unwrap().is_dark);
    }

    #[test]
    fn from_file_names_a_nameless_scheme_after_the_file() {
        let dir = std::env::temp_dir().join(format!("base16-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("my-scheme.yaml");
        fs::write(&path, old_layout("000000").replace("scheme: \"Old\"\n", "")).unwrap();
        assert_eq!(Base16::from_file(&path).unwrap().name, "my-scheme");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_have_display_and_source() {
        let io = Base16::from_file("/definitely/not/here.yaml").unwrap_err();
        assert!(matches!(io, Base16Error::Io(_)));
        assert!(io.source().is_some());
        let parse = Base16::from_yaml("").unwrap_err();
        assert!(parse.source().is_none());
        assert!(!parse.to_string().is_empty());
    }
}
