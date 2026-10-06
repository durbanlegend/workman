use eframe::egui;
use egui::{Color32, Stroke, Visuals};
use std::{error::Error, fs, path::Path};

#[derive(Debug)]
pub struct Base16 {
    pub name: &'static str,
    pub is_dark: bool,
    /// base00..=base0F, indexed 0..=15
    pub c: [Color32; 16],
}

impl Base16 {
    /// # Errors
    /// Will bubble up any errors encountered reading the file from disk.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        Self::from_yaml(&fs::read_to_string(path)?)
    }

    /// # Errors
    /// Will bubble up any errors encountered parsing the `YAML` file.
    pub fn from_yaml(src: &str) -> Result<Self, Box<dyn Error>> {
        let root: serde_yaml::Value = serde_yaml::from_str(src)?;
        let palette = root.get("palette").unwrap_or(&root);

        let mut c = [Color32::PLACEHOLDER; 16];
        for (i, slot) in c.iter_mut().enumerate() {
            let key = format!("base{i:02X}");
            let hex = palette
                .get(&key)
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("missing colour `{key}`"))?;
            *slot = parse_hex(hex)?;
        }

        let name = root
            .get("name")
            .or_else(|| root.get("scheme"))
            .and_then(|v| v.as_str())
            .unwrap_or("base16")
            .to_owned();

        // Use the `variant` field if present, otherwise guess from base00's luminance.
        let is_dark = match root.get("variant").and_then(|v| v.as_str()) {
            Some("dark") => true,
            Some("light") => false,
            _ => luminance(c[0]) < 0.5_f32,
        };

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

fn parse_hex(s: &str) -> Result<Color32, Box<dyn Error>> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return Err(format!("expected 6 hex digits, got `{s}`").into());
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16);
    Ok(Color32::from_rgb(p(0)?, p(2)?, p(4)?))
}

fn luminance(c: Color32) -> f32 {
    0.0722f32.mul_add(
        f32::from(c.b()),
        0.7152f32.mul_add(f32::from(c.g()), 0.2126 * f32::from(c.r())),
    ) / 255.0
}
