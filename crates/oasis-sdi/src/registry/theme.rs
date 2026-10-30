//! `SdiRegistry::load_theme`: apply a TOML theme to the registry.

use std::collections::HashMap;

use serde::Deserialize;

use oasis_types::backend::Color;
use oasis_types::error::{OasisError, Result};

use super::SdiRegistry;

impl SdiRegistry {
    /// Apply a theme from a TOML string. Each top-level key is an object name;
    /// nested keys set properties (x, y, w, h, visible, alpha, color, text,
    /// font_size). Objects that don't exist yet are created.
    pub fn load_theme(&mut self, toml_str: &str) -> Result<()> {
        /// Apply optional fields from a theme entry to an SDI object.
        macro_rules! apply_theme {
            // Direct copy: entry.field -> obj.field
            ($entry:expr, $obj:expr, copy: [$($f:ident),*]) => {
                $(if let Some(v) = $entry.$f { $obj.$f = v; })*
            };
            // Clone into Option: entry.field -> obj.field = Some(clone)
            ($entry:expr, $obj:expr, clone: [$($f:ident),*]) => {
                $(if let Some(ref v) = $entry.$f { $obj.$f = Some(v.clone()); })*
            };
            // Parse hex color: entry.field -> obj.field = parsed
            ($entry:expr, $obj:expr, color: [$($f:ident),*]) => {
                $(if let Some(ref c) = $entry.$f
                    && let Some(parsed) = parse_color(c)
                { $obj.$f = parsed; })*
            };
            // Parse hex color into Option: entry.field -> obj.field = Some(parsed)
            ($entry:expr, $obj:expr, opt_color: [$($f:ident),*]) => {
                $(if let Some(ref c) = $entry.$f
                    && let Some(parsed) = parse_color(c)
                { $obj.$f = Some(parsed); })*
            };
            // Wrap in Some: entry.field -> obj.field = Some(v)
            ($entry:expr, $obj:expr, wrap: [$($f:ident),*]) => {
                $(if let Some(v) = $entry.$f { $obj.$f = Some(v); })*
            };
        }

        let theme: HashMap<String, ThemeEntry> =
            toml::from_str(toml_str).map_err(|e| OasisError::Config(format!("{e}").into()))?;

        // Apply in name order: newly created objects take their auto-z from the
        // creation counter, and HashMap iteration order is random per process.
        let mut entries: Vec<(String, ThemeEntry)> = theme.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, entry) in entries {
            if !self.contains(&name) {
                self.create(&name);
            }
            let obj = self.get_mut(&name)?;
            apply_theme!(entry, obj, copy: [x, y, w, h, alpha, visible, font_size, overlay]);
            apply_theme!(entry, obj, clone: [text]);
            apply_theme!(entry, obj, color: [color, text_color]);
            apply_theme!(entry, obj, opt_color: [gradient_top, gradient_bottom, stroke_color, shadow_color]);
            apply_theme!(entry, obj, wrap: [border_radius, shadow_level, stroke_width]);
        }
        Ok(())
    }
}

/// Deserialization helper for theme TOML entries.
#[derive(Debug, Deserialize)]
struct ThemeEntry {
    /// X position in virtual screen coordinates.
    x: Option<i32>,
    /// Y position in virtual screen coordinates.
    y: Option<i32>,
    /// Width in pixels.
    w: Option<u32>,
    /// Height in pixels.
    h: Option<u32>,
    /// Alpha (0 = fully transparent, 255 = fully opaque).
    alpha: Option<u8>,
    /// Whether this object is drawn.
    visible: Option<bool>,
    /// Text content.
    text: Option<String>,
    /// Font size in pixels.
    font_size: Option<u16>,
    /// Fill color as hex string ("#RRGGBB" or "#RRGGBBAA").
    color: Option<String>,
    /// Text color as hex string.
    text_color: Option<String>,
    /// Render in overlay pass (on top of base layer).
    overlay: Option<bool>,
    // Extended visual properties.
    /// Corner radius for rounded rectangles (pixels).
    border_radius: Option<u16>,
    /// Gradient top color as hex string.
    gradient_top: Option<String>,
    /// Gradient bottom color as hex string.
    gradient_bottom: Option<String>,
    /// Shadow elevation level (0 = none, 1-3 = increasingly prominent).
    shadow_level: Option<u8>,
    /// Stroke/outline width in pixels.
    stroke_width: Option<u16>,
    /// Stroke/outline color as hex string.
    stroke_color: Option<String>,
    /// Shadow color as hex string (default: black).
    shadow_color: Option<String>,
}

/// Parse a color string like "#RRGGBB" or "#RRGGBBAA".
pub(super) fn parse_color(s: &str) -> Option<Color> {
    oasis_types::color::parse_hex_color(s)
}
