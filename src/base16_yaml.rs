//! Minimal Base16 YAML parsing (no YAML crate needed).
//!
//! Handles both layouts:
//!   old:  `scheme: "Name"` / `base00: "181818"` ...
//!   new:  `name: "Name"` / `variant: "dark"` / `palette:` with `base00: "181818"` ...
//!
//! Only flat `key: value` lines are understood, which covers every published
//! Base16 / tinted-theming scheme file.

use base16::Base16;
use eframe::egui::Color32;

fn yaml_scalar(v: &str) -> String {
    let v = v.trim();
    if let Some(q) = v.chars().next().filter(|&c| c == '"' || c == '\'') {
        return v[1..].split(q).next().unwrap_or("").to_owned();
    }
    v.split(" #").next().unwrap_or("").trim().to_owned()
}

#[expect(clippy::cast_possible_truncation)]
fn parse_hex(s: &str) -> Result<Color32, String> {
    let s = s.trim_start_matches('#');
    let bad = || format!("invalid colour {s:?}");
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let n = u32::from_str_radix(s, 16).map_err(|_| bad())?;
    Ok(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

pub fn parse_base16_yaml(src: &str, fallback_name: &str) -> Result<Base16, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    const OLD_LAYOUT: &str = r#"
scheme: "Old Style"
author: "Someone"
base00: "181818"
base01: "282828"
base02: "383838"
base03: "585858"
base04: "b8b8b8"
base05: "d8d8d8"
base06: "e8e8e8"
base07: "f8f8f8"
base08: "ab4642"
base09: "dc9656"
base0A: "f7ca88"
base0B: "a1b56c"
base0C: "86c1b9"
base0D: "7cafc2"
base0E: "ba8baf"
base0F: "a16946"
"#;

    const NEW_LAYOUT: &str = r##"
system: "base16"
name: "New Style"
variant: "light"
palette:
  base00: "#ffffff"
  base01: "#e0e0e0"
  base02: "#d0d0d0"
  base03: "#b0b0b0"
  base04: "#909090"
  base05: "#505050"
  base06: "#303030"
  base07: "#101010"
  base08: "#ab4642"
  base09: "#dc9656"
  base0A: "#f7ca88"
  base0B: "#a1b56c"
  base0C: "#86c1b9"
  base0D: "#7cafc2"
  base0E: "#ba8baf"
  base0F: "#a16946"
"##;

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
        assert_eq!(parse_hex("000000"), Ok(Color32::from_rgb(0, 0, 0)));
        assert!(parse_hex("").is_err());
        assert!(parse_hex("fff").is_err());
        assert!(parse_hex("ff800000").is_err());
        assert!(parse_hex("gg0000").is_err());
        assert!(parse_hex("+f0000").is_err(), "no sign characters");
    }

    #[test]
    fn old_layout_parses_and_infers_darkness_from_base00() {
        let t = parse_base16_yaml(OLD_LAYOUT, "fallback").unwrap();
        assert_eq!(t.name, "Old Style");
        assert!(t.is_dark);
        assert_eq!(t.c[0], Color32::from_rgb(0x18, 0x18, 0x18));
        assert_eq!(t.c[15], Color32::from_rgb(0xa1, 0x69, 0x46));
    }

    #[test]
    fn new_layout_honours_variant() {
        let t = parse_base16_yaml(NEW_LAYOUT, "fallback").unwrap();
        assert_eq!(t.name, "New Style");
        assert!(!t.is_dark);
        assert_eq!(t.c[0], Color32::WHITE);
    }

    #[test]
    fn variant_overrides_luminance() {
        let src = OLD_LAYOUT.replace("author:", "variant: \"light\"\nauthor:");
        assert!(!parse_base16_yaml(&src, "x").unwrap().is_dark);
        let src = NEW_LAYOUT.replace("\"light\"", "\"DARK\"");
        assert!(
            parse_base16_yaml(&src, "x").unwrap().is_dark,
            "case-insensitive"
        );
    }

    #[test]
    fn missing_name_uses_fallback() {
        let src = OLD_LAYOUT.replace("scheme: \"Old Style\"\n", "");
        assert_eq!(
            parse_base16_yaml(&src, "from-file-name").unwrap().name,
            "from-file-name"
        );
    }

    #[test]
    fn missing_colours_are_listed() {
        let src = OLD_LAYOUT
            .replace("base03: \"585858\"\n", "")
            .replace("base0F: \"a16946\"\n", "");
        let err = parse_base16_yaml(&src, "x").unwrap_err();
        assert!(err.contains("base03") && err.contains("base0F"), "{err}");
    }

    #[test]
    fn bad_colour_names_the_key() {
        let src = OLD_LAYOUT.replace("181818", "zzzzzz");
        let err = parse_base16_yaml(&src, "x").unwrap_err();
        assert!(err.contains("base00"), "{err}");
    }

    #[test]
    fn base24_extra_colours_are_ignored() {
        let src = format!("{OLD_LAYOUT}base10: \"000000\"\nbase17: \"ffffff\"\n");
        assert!(parse_base16_yaml(&src, "x").is_ok());
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let src = format!("# a comment\n\n{OLD_LAYOUT}\n# trailing\n");
        assert!(parse_base16_yaml(&src, "x").is_ok());
    }

    /// Both parsers read the same bundled files, so they must agree on what is in them.
    /// (The luminance fallback differs between them, so only files that state a `variant`
    /// are compared on darkness.)
    #[test]
    fn agrees_with_the_base16_crate_on_every_bundled_theme() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/themes");
        let mut checked = 0;
        let mut disagreements = Vec::new();
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "yaml" && e != "yml") {
                continue;
            }
            let src = fs::read_to_string(&path).unwrap();
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            let ours = parse_base16_yaml(&src, &stem)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let theirs =
                Base16::from_yaml(&src).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert_eq!(ours.c, theirs.c, "{}: colours differ", path.display());
            if ours.is_dark != theirs.is_dark {
                disagreements.push(stem);
            }
            checked += 1;
        }
        assert!(checked > 10, "only {checked} themes found");
        assert!(
            disagreements.is_empty(),
            "dark/light classification differs for: {disagreements:?}"
        );
    }
}
