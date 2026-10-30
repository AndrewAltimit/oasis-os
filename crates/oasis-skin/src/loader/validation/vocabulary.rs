//! Vocabulary lints: string fields that only accept a fixed set of names.
//!
//! Every one of these falls back silently at runtime (an unknown layer kind
//! draws nothing, an unknown easing is linear, an unknown anchor is
//! `center`), so a typo would otherwise produce a skin that "works" but not
//! the way its author wrote it.

use crate::theme::{BackgroundLayerConfig, EASING_NAMES, parse_hex_color};

use super::super::Skin;

/// `kind` values understood by `[[background_layers]]` (`image` is drawn by
/// the shell's image-layer path; the rest by `oasis-vector`).
pub(crate) const LAYER_KINDS: &[&str] = &[
    "grid",
    "dot_grid",
    "wireframe_sphere",
    "radar_sweep",
    "concentric_rings",
    "glass_shard",
    "scanlines",
    "eq_bars",
    "crosshair",
    "floating_polygons",
    "pulsing_core",
    "waves",
    "shader",
    "image",
];

/// `[background_layers.position] anchor` values.
pub(crate) const ANCHORS: &[&str] = &[
    "top_left",
    "top_center",
    "top_right",
    "center_left",
    "center",
    "center_right",
    "bottom_left",
    "bottom_center",
    "bottom_right",
];

/// `[wallpaper] style` values (`oasis-core/src/wallpaper.rs`).
pub(crate) const WALLPAPER_STYLES: &[&str] = &[
    "gradient",
    "solid",
    "image",
    "none",
    "grid",
    "scanlines",
    "noise",
    "dots",
    "stripes",
    "checkerboard",
];

/// `[transition] entrance` values (`oasis-app/src/launch.rs`).
pub(crate) const ENTRANCES: &[&str] = &["fade", "assemble", "none"];

/// `[transition] page_style` values.
pub(crate) const PAGE_STYLES: &[&str] = &["slide", "fade"];

/// `[icon_overrides] entrance_style` values.
pub(crate) const ICON_ENTRANCES: &[&str] = &["none", "fade_in", "scale_up", "slide_up"];

/// Top-level `density` values.
pub(crate) const DENSITIES: &[&str] = &["compact", "comfortable", "spacious"];

/// Push `"{ctx}: unknown {what} \"{value}\" (expected a|b|c)"` when `value`
/// is not one of `known`.
fn check_name(warnings: &mut Vec<String>, ctx: &str, what: &str, value: &str, known: &[&str]) {
    if !known.contains(&value) {
        warnings.push(format!(
            "{ctx}: unknown {what} \"{value}\" (expected {})",
            known.join("|")
        ));
    }
}

impl Skin {
    /// Enumerated-name checks over the theme (see the module docs).
    pub(super) fn validate_vocabulary(&self, warnings: &mut Vec<String>) {
        let theme = &self.theme;
        if let Some(ref d) = theme.density {
            check_name(warnings, "theme", "density", d, DENSITIES);
        }
        if let Some(ref wp) = theme.wallpaper
            && let Some(ref style) = wp.style
        {
            check_name(warnings, "wallpaper", "style", style, WALLPAPER_STYLES);
        }
        if let Some(ref t) = theme.transition {
            if let Some(ref e) = t.entrance {
                check_name(warnings, "transition", "entrance", e, ENTRANCES);
            }
            if let Some(ref p) = t.page_style {
                check_name(warnings, "transition", "page_style", p, PAGE_STYLES);
            }
            if let Some(ref e) = t.easing {
                check_name(warnings, "transition", "easing", e, EASING_NAMES);
            }
        }
        if let Some(ref icons) = theme.icon_overrides
            && let Some(ref e) = icons.entrance_style
        {
            check_name(
                warnings,
                "icon_overrides",
                "entrance_style",
                e,
                ICON_ENTRANCES,
            );
        }
        if let Some(ref presets) = theme.animations {
            let mut names: Vec<&String> = presets.keys().collect();
            names.sort();
            for name in names {
                let ctx = format!("animations.{name}");
                check_name(
                    warnings,
                    &ctx,
                    "easing",
                    &presets[name].easing,
                    EASING_NAMES,
                );
            }
        }
        for (table, layers) in [
            ("background_layers", &theme.background_layers),
            ("chrome_layers", &theme.chrome_layers),
        ] {
            for (i, layer) in layers.iter().flatten().enumerate() {
                lint_layer(warnings, &format!("{table}[{i}]"), layer);
            }
        }
    }
}

/// Checks for one `[[background_layers]]` / `[[chrome_layers]]` entry.
fn lint_layer(warnings: &mut Vec<String>, ctx: &str, layer: &BackgroundLayerConfig) {
    check_name(warnings, ctx, "kind", &layer.kind, LAYER_KINDS);
    if layer.kind == "glass_shard" && layer.points.as_ref().is_none_or(|p| p.len() < 3) {
        warnings.push(format!(
            "{ctx}: glass_shard needs at least 3 `points` (the layer is skipped)"
        ));
    }
    if let Some(anchor) = layer.position.as_ref().and_then(|p| p.anchor.as_deref()) {
        check_name(warnings, ctx, "anchor", anchor, ANCHORS);
    }
    if let Some(ref params) = layer.shader_params {
        for key in ["color_1", "color_2", "color_3"] {
            match params.get(key) {
                None => {},
                Some(toml::Value::String(s)) if parse_hex_color(s).is_some() => {},
                Some(v) => warnings.push(format!(
                    "{ctx}: shader_params.{key} = {v} is not a #RRGGBB[AA] color \
                     (the color is dropped)"
                )),
            }
        }
    }
}

/// Parse-time color lint over a raw `theme.toml` document.
///
/// Every override table stores colors as strings that are parsed lazily
/// with a silent fallback, so a typo such as `clock_color = "#FFF"` or
/// `bar_bg = "black"` would quietly render the derived default instead.
/// This walks the whole document and flags:
///
/// - any string starting with `#` that is not `#RRGGBB` / `#RRGGBBAA`, and
/// - any non-empty, non-`#` string stored under a color-named key (`*color`,
///   `*_bg`, `*_stops`, `color_*`) -- named colors are not supported.
///
/// Top-level base colors, `[palette]`, `[cursor]` and `[boot]` are skipped:
/// [`Skin::validate`] already reports those with field-specific messages.
pub(crate) fn lint_color_strings(theme_toml: &str, warnings: &mut Vec<String>) {
    const EXPLICIT_TOP: &[&str] = &[
        "background",
        "primary",
        "secondary",
        "text",
        "dim_text",
        "status_bar",
        "prompt",
        "output",
        "error",
        "surface",
        "accent",
        "accent_hover",
        "palette",
        "cursor",
        "boot",
    ];
    let Ok(doc) = theme_toml.parse::<toml::Table>() else {
        return; // Parse errors are reported by the typed parse.
    };
    for (key, value) in &doc {
        if !EXPLICIT_TOP.contains(&key.as_str()) {
            walk(key, key, value, warnings);
        }
    }
}

fn is_color_key(key: &str) -> bool {
    key.ends_with("color")
        || key.ends_with("_bg")
        || key.ends_with("_stops")
        || key.starts_with("color_")
}

fn walk(path: &str, key: &str, value: &toml::Value, warnings: &mut Vec<String>) {
    match value {
        toml::Value::String(s) => {
            let bad = if s.starts_with('#') {
                parse_hex_color(s).is_none()
            } else {
                !s.is_empty() && is_color_key(key)
            };
            if bad {
                warnings.push(format!(
                    "theme.toml: `{path}` = \"{s}\" is not a #RRGGBB or #RRGGBBAA color"
                ));
            }
        },
        toml::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                walk(&format!("{path}[{i}]"), key, item, warnings);
            }
        },
        toml::Value::Table(t) => {
            for (k, v) in t {
                walk(&format!("{path}.{k}"), k, v, warnings);
            }
        },
        _ => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lint(theme: &str) -> Vec<String> {
        let skin = Skin::from_toml_full("name = \"vocab\"", "", "", theme, "").unwrap();
        skin.validate()
    }

    fn has(warnings: &[String], needle: &str) -> bool {
        warnings.iter().any(|w| w.contains(needle))
    }

    #[test]
    fn unknown_layer_kind_warns() {
        let w = lint("[[background_layers]]\nkind = \"glas_shard\"\n");
        assert!(
            has(&w, "background_layers[0]: unknown kind \"glas_shard\""),
            "{w:?}"
        );
    }

    #[test]
    fn every_known_layer_kind_is_accepted() {
        for kind in LAYER_KINDS {
            let w = lint(&format!("[[chrome_layers]]\nkind = \"{kind}\"\n"));
            assert!(!has(&w, "unknown kind"), "{kind}: {w:?}");
        }
    }

    #[test]
    fn glass_shard_without_points_warns() {
        let w = lint("[[background_layers]]\nkind = \"glass_shard\"\npoints = [[0.1, 0.2]]\n");
        assert!(has(&w, "glass_shard needs at least 3"), "{w:?}");
    }

    #[test]
    fn unknown_anchor_warns() {
        let w = lint(
            "[[background_layers]]\nkind = \"crosshair\"\n\
             [background_layers.position]\nanchor = \"middle\"\n",
        );
        assert!(has(&w, "unknown anchor \"middle\""), "{w:?}");
    }

    #[test]
    fn bad_shader_param_color_warns() {
        let w = lint(
            "[[background_layers]]\nkind = \"shader\"\nshader = \"plasma\"\n\
             [background_layers.shader_params]\ncolor_1 = \"#12\"\ncolor_2 = 7\n",
        );
        assert!(has(&w, "shader_params.color_1"), "{w:?}");
        assert!(has(&w, "shader_params.color_2"), "{w:?}");
    }

    #[test]
    fn unknown_easing_names_warn() {
        let w = lint(
            "[transition]\neasing = \"ease_in_cubic\"\n\
             [animations.open]\nduration_ms = 200\neasing = \"bouncy\"\n",
        );
        assert!(
            has(&w, "transition: unknown easing \"ease_in_cubic\""),
            "{w:?}"
        );
        assert!(
            has(&w, "animations.open: unknown easing \"bouncy\""),
            "{w:?}"
        );
    }

    #[test]
    fn every_easing_name_resolves_to_its_curve() {
        // A name in EASING_NAMES must not fall through to `linear` unless it
        // *is* linear: compare the curves at a sample point.
        for name in EASING_NAMES {
            let f = crate::theme::resolve_easing(name);
            if *name != "linear" {
                assert!((f(0.3) - 0.3).abs() > 1e-4, "{name} resolves to linear");
            }
        }
    }

    #[test]
    fn unknown_enumerated_strings_warn() {
        let w = lint(
            "density = \"cozy\"\n[wallpaper]\nstyle = \"gradiant\"\n\
             [transition]\nentrance = \"zoom\"\npage_style = \"flip\"\n\
             [icon_overrides]\nentrance_style = \"pop\"\n",
        );
        for needle in [
            "unknown density \"cozy\"",
            "unknown style \"gradiant\"",
            "unknown entrance \"zoom\"",
            "unknown page_style \"flip\"",
            "unknown entrance_style \"pop\"",
        ] {
            assert!(has(&w, needle), "missing {needle}: {w:?}");
        }
    }

    #[test]
    fn override_color_typos_warn() {
        let w = lint(
            "[bar_overrides]\nclock_color = \"#FFF\"\nbar_bg = \"black\"\n\
             [wallpaper]\ncolor_stops = [\"#000000\", \"#12345\"]\n",
        );
        assert!(has(&w, "`bar_overrides.clock_color` = \"#FFF\""), "{w:?}");
        assert!(has(&w, "`bar_overrides.bar_bg` = \"black\""), "{w:?}");
        assert!(has(&w, "`wallpaper.color_stops[1]` = \"#12345\""), "{w:?}");
        assert!(!has(&w, "color_stops[0]"), "{w:?}");
    }

    #[test]
    fn non_color_strings_are_not_color_linted() {
        let w = lint(
            "[wm_theme]\nglyph_close = \"x\"\nbutton_side = \"left\"\n\
             [bar_overrides]\nurl_text = \"A:\\\\>\"\n",
        );
        assert!(!has(&w, "is not a #RRGGBB"), "{w:?}");
    }

    #[test]
    fn base_colors_are_reported_once() {
        let w = lint("background = \"#XYZ\"\n");
        let n = w.iter().filter(|w| w.contains("#XYZ")).count();
        assert_eq!(n, 1, "{w:?}");
    }
}
