//! Skin loading from TOML configuration files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{OasisError, Result};
use crate::sdi::SdiRegistry;

use super::corrupted::CorruptedModifiers;
use super::strings::SkinStrings;
use super::theme::{SkinTheme, parse_hex_color};

/// Top-level skin manifest (`skin.toml`).
#[derive(Debug, Clone, Deserialize)]
pub struct SkinManifest {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_width")]
    pub screen_width: u32,
    #[serde(default = "default_height")]
    pub screen_height: u32,
}

fn default_version() -> String {
    "1.0".to_string()
}
fn default_width() -> u32 {
    480
}
fn default_height() -> u32 {
    272
}

/// A single SDI object definition in a layout file.
#[derive(Debug, Clone, Deserialize)]
pub struct SkinObjectDef {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub w: Option<u32>,
    pub h: Option<u32>,
    pub color: Option<String>,
    pub text: Option<String>,
    pub text_color: Option<String>,
    pub font_size: Option<u16>,
    pub alpha: Option<u8>,
    pub visible: Option<bool>,
    // Extended visual properties.
    #[serde(default)]
    pub border_radius: Option<u16>,
    #[serde(default)]
    pub gradient_top: Option<String>,
    #[serde(default)]
    pub gradient_bottom: Option<String>,
    #[serde(default)]
    pub shadow_level: Option<u8>,
    #[serde(default)]
    pub stroke_width: Option<u16>,
    #[serde(default)]
    pub stroke_color: Option<String>,
}

/// Layout: a named collection of SDI object definitions (`layout.toml`).
#[derive(Debug, Clone, Deserialize)]
pub struct SkinLayout {
    #[serde(flatten)]
    pub objects: HashMap<String, SkinObjectDef>,
}

/// Feature gates controlling which capabilities a skin exposes.
#[derive(Debug, Clone, Deserialize)]
pub struct SkinFeatures {
    /// Whether the dashboard icon grid is shown.
    #[serde(default = "yes")]
    pub dashboard: bool,
    /// Whether the command terminal is accessible.
    #[serde(default = "yes")]
    pub terminal: bool,
    /// Whether the file browser command (ls/cd/cat) is available.
    #[serde(default = "yes")]
    pub file_browser: bool,
    /// Whether the HTML/CSS browser widget is available.
    #[serde(default = "yes")]
    pub browser: bool,
    /// Whether the window manager is active (for Desktop skin).
    #[serde(default)]
    pub window_manager: bool,
    /// Number of dashboard pages (for icon grid skins).
    #[serde(default = "default_pages")]
    pub dashboard_pages: u32,
    /// Icons per page (grid capacity).
    #[serde(default = "default_icons_per_page")]
    pub icons_per_page: u32,
    /// Grid columns.
    #[serde(default = "default_grid_cols")]
    pub grid_cols: u32,
    /// Grid rows.
    #[serde(default = "default_grid_rows")]
    pub grid_rows: u32,
    /// Available command categories (empty = all).
    #[serde(default)]
    pub command_categories: Vec<String>,
    /// Whether the start menu popup is available.
    #[serde(default = "yes")]
    pub start_menu: bool,
    /// Whether corrupted modifiers are active.
    #[serde(default)]
    pub corrupted: bool,
    /// Whether the battery indicator is shown in the status bar.
    #[serde(default = "yes")]
    pub show_battery: bool,
    /// Whether the clock is shown in the status bar.
    #[serde(default = "yes")]
    pub show_clock: bool,
    /// Whether the version label is shown in the status bar.
    #[serde(default = "yes")]
    pub show_version: bool,
    /// Whether top tabs are shown in the status bar.
    #[serde(default = "yes")]
    pub show_tabs: bool,
    /// Whether media category tabs are shown in the bottom bar.
    #[serde(default = "yes")]
    pub show_media_tabs: bool,
    /// Whether page dots are shown in the bottom bar.
    #[serde(default = "yes")]
    pub show_page_dots: bool,
    /// Custom fade transition duration in frames (default 15).
    #[serde(default)]
    pub transition_fade_frames: Option<u32>,
    /// Custom slide transition duration in frames (default 20).
    #[serde(default)]
    pub transition_slide_frames: Option<u32>,
}

fn yes() -> bool {
    true
}
fn default_pages() -> u32 {
    3
}
fn default_icons_per_page() -> u32 {
    6
}
fn default_grid_cols() -> u32 {
    3
}
fn default_grid_rows() -> u32 {
    2
}

impl Default for SkinFeatures {
    fn default() -> Self {
        Self {
            dashboard: true,
            terminal: true,
            file_browser: true,
            browser: true,
            window_manager: false,
            dashboard_pages: 3,
            icons_per_page: 6,
            grid_cols: 3,
            grid_rows: 2,
            command_categories: Vec::new(),
            start_menu: true,
            corrupted: false,
            show_battery: true,
            show_clock: true,
            show_version: true,
            show_tabs: true,
            show_media_tabs: true,
            show_page_dots: true,
            transition_fade_frames: None,
            transition_slide_frames: None,
        }
    }
}

/// A fully loaded skin ready for use.
#[derive(Debug, Clone)]
pub struct Skin {
    pub manifest: SkinManifest,
    pub layout: SkinLayout,
    pub features: SkinFeatures,
    pub theme: SkinTheme,
    pub strings: SkinStrings,
    pub corrupted_modifiers: Option<CorruptedModifiers>,
}

impl Skin {
    /// Load a skin from TOML strings (basic 3-file format for backwards compat).
    pub fn from_toml(manifest_toml: &str, layout_toml: &str, features_toml: &str) -> Result<Self> {
        Self::from_toml_full(manifest_toml, layout_toml, features_toml, "", "")
    }

    /// Load a skin from all TOML configuration strings.
    pub fn from_toml_full(
        manifest_toml: &str,
        layout_toml: &str,
        features_toml: &str,
        theme_toml: &str,
        strings_toml: &str,
    ) -> Result<Self> {
        let manifest: SkinManifest = toml::from_str(manifest_toml)
            .map_err(|e| OasisError::Config(format!("skin.toml: {e}")))?;
        let layout: SkinLayout = toml::from_str(layout_toml)
            .map_err(|e| OasisError::Config(format!("layout.toml: {e}")))?;
        let features: SkinFeatures = toml::from_str(features_toml)
            .map_err(|e| OasisError::Config(format!("features.toml: {e}")))?;

        let theme: SkinTheme = if theme_toml.is_empty() {
            SkinTheme::default()
        } else {
            toml::from_str(theme_toml)
                .map_err(|e| OasisError::Config(format!("theme.toml: {e}")))?
        };

        let strings: SkinStrings = if strings_toml.is_empty() {
            SkinStrings::default()
        } else {
            toml::from_str(strings_toml)
                .map_err(|e| OasisError::Config(format!("strings.toml: {e}")))?
        };

        let corrupted_modifiers = if features.corrupted {
            Some(CorruptedModifiers::default())
        } else {
            None
        };

        Ok(Self {
            manifest,
            layout,
            features,
            theme,
            strings,
            corrupted_modifiers,
        })
    }

    /// Load a skin with explicit corrupted modifier configuration.
    pub fn from_toml_corrupted(
        manifest_toml: &str,
        layout_toml: &str,
        features_toml: &str,
        theme_toml: &str,
        strings_toml: &str,
        corrupted_toml: &str,
    ) -> Result<Self> {
        let mut skin = Self::from_toml_full(
            manifest_toml,
            layout_toml,
            features_toml,
            theme_toml,
            strings_toml,
        )?;

        if !corrupted_toml.is_empty() {
            let modifiers: CorruptedModifiers = toml::from_str(corrupted_toml)
                .map_err(|e| OasisError::Config(format!("corrupted.toml: {e}")))?;
            skin.corrupted_modifiers = Some(modifiers);
        }

        Ok(skin)
    }

    /// Apply this skin's layout to an SDI registry. Existing objects are
    /// updated, missing objects are created.
    pub fn apply_layout(&self, sdi: &mut SdiRegistry) {
        for (name, def) in &self.layout.objects {
            if !sdi.contains(name) {
                sdi.create(name);
            }
            if let Ok(obj) = sdi.get_mut(name) {
                if let Some(x) = def.x {
                    obj.x = x;
                }
                if let Some(y) = def.y {
                    obj.y = y;
                }
                if let Some(w) = def.w {
                    obj.w = w;
                }
                if let Some(h) = def.h {
                    obj.h = h;
                }
                if let Some(a) = def.alpha {
                    obj.alpha = a;
                }
                if let Some(v) = def.visible {
                    obj.visible = v;
                }
                if let Some(ref t) = def.text {
                    obj.text = Some(t.clone());
                }
                if let Some(fs) = def.font_size {
                    obj.font_size = fs;
                }
                if let Some(ref c) = def.color
                    && let Some(parsed) = parse_hex_color(c)
                {
                    obj.color = parsed;
                }
                if let Some(ref c) = def.text_color
                    && let Some(parsed) = parse_hex_color(c)
                {
                    obj.text_color = parsed;
                }
                // Extended visual properties.
                if let Some(r) = def.border_radius {
                    obj.border_radius = Some(r);
                }
                if let Some(ref c) = def.gradient_top
                    && let Some(parsed) = parse_hex_color(c)
                {
                    obj.gradient_top = Some(parsed);
                }
                if let Some(ref c) = def.gradient_bottom
                    && let Some(parsed) = parse_hex_color(c)
                {
                    obj.gradient_bottom = Some(parsed);
                }
                if let Some(s) = def.shadow_level {
                    obj.shadow_level = Some(s);
                }
                if let Some(sw) = def.stroke_width {
                    obj.stroke_width = Some(sw);
                }
                if let Some(ref c) = def.stroke_color
                    && let Some(parsed) = parse_hex_color(c)
                {
                    obj.stroke_color = Some(parsed);
                }
            }
        }
    }

    /// Load a skin from a directory containing TOML files.
    ///
    /// Requires `skin.toml`, `layout.toml`, and `features.toml`.
    /// Optional files: `theme.toml`, `strings.toml`, `corrupted.toml`.
    pub fn from_directory(dir: &Path) -> Result<Self> {
        let read = |name: &str| -> Result<String> {
            let p = dir.join(name);
            std::fs::read_to_string(&p)
                .map_err(|e| OasisError::Config(format!("{}: {e}", p.display())))
        };
        let read_opt =
            |name: &str| -> String { std::fs::read_to_string(dir.join(name)).unwrap_or_default() };

        let manifest = read("skin.toml")?;
        let layout = read("layout.toml")?;
        let features = read("features.toml")?;
        let theme = read_opt("theme.toml");
        let strings = read_opt("strings.toml");
        let corrupted = read_opt("corrupted.toml");

        if corrupted.is_empty() {
            Self::from_toml_full(&manifest, &layout, &features, &theme, &strings)
        } else {
            Self::from_toml_corrupted(&manifest, &layout, &features, &theme, &strings, &corrupted)
        }
    }

    /// Scan a directory for skin subdirectories (those containing `skin.toml`).
    ///
    /// Returns `(name, path)` pairs sorted by name.
    pub fn discover_skins(dir: &Path) -> Vec<(String, PathBuf)> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut skins: Vec<(String, PathBuf)> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("skin.toml").is_file())
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                (name, e.path())
            })
            .collect();
        skins.sort_by(|a, b| a.0.cmp(&b.0));
        skins
    }

    /// Tear down the current SDI tree and rebuild from a new skin.
    ///
    /// The VFS overlay is NOT affected -- file state persists across swaps.
    /// Returns the old skin for potential rollback.
    pub fn swap(current: &Skin, new_skin: Skin, sdi: &mut SdiRegistry) -> Skin {
        // Destroy all SDI objects defined in the current layout.
        for name in current.layout.objects.keys() {
            let _ = sdi.destroy(name);
        }

        // Apply the new skin's layout.
        new_skin.apply_layout(sdi);

        new_skin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
name = "classic"
version = "1.0"
author = "AndrewAltimit"
description = "PSP-style icon grid dashboard"
screen_width = 480
screen_height = 272
"#;

    const LAYOUT: &str = r##"
[status_bar]
x = 0
y = 0
w = 480
h = 24
color = "#283C5A"

[content_bg]
x = 0
y = 24
w = 480
h = 248
color = "#1A1A2D"
"##;

    const FEATURES: &str = r#"
dashboard = true
terminal = true
file_browser = true
browser = true
window_manager = false
dashboard_pages = 3
icons_per_page = 6
grid_cols = 3
grid_rows = 2
"#;

    #[test]
    fn load_skin_from_toml() {
        let skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
        assert_eq!(skin.manifest.name, "classic");
        assert_eq!(skin.manifest.screen_width, 480);
        assert_eq!(skin.layout.objects.len(), 2);
        assert!(skin.features.dashboard);
        assert!(skin.features.browser);
        assert!(!skin.features.window_manager);
        assert_eq!(skin.features.grid_cols, 3);
    }

    #[test]
    fn apply_layout_creates_objects() {
        let skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
        let mut sdi = SdiRegistry::new();
        skin.apply_layout(&mut sdi);
        assert!(sdi.contains("status_bar"));
        assert!(sdi.contains("content_bg"));
        let bar = sdi.get("status_bar").unwrap();
        assert_eq!(bar.w, 480);
        assert_eq!(bar.h, 24);
    }

    #[test]
    fn apply_layout_updates_existing() {
        let skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
        let mut sdi = SdiRegistry::new();
        {
            let obj = sdi.create("status_bar");
            obj.x = 999;
        }
        skin.apply_layout(&mut sdi);
        let bar = sdi.get("status_bar").unwrap();
        assert_eq!(bar.x, 0); // Updated by layout.
    }

    #[test]
    fn default_features() {
        let f = SkinFeatures::default();
        assert!(f.dashboard);
        assert!(f.terminal);
        assert_eq!(f.dashboard_pages, 3);
        assert_eq!(f.icons_per_page, 6);
    }

    #[test]
    fn manifest_defaults() {
        let toml = r#"name = "minimal""#;
        let m: SkinManifest = toml::from_str(toml).unwrap();
        assert_eq!(m.screen_width, 480);
        assert_eq!(m.screen_height, 272);
        assert_eq!(m.version, "1.0");
    }

    #[test]
    fn from_toml_full_with_theme_and_strings() {
        let theme_toml = r##"
background = "#000000"
prompt = "#00FF00"
"##;
        let strings_toml = r#"
prompt_format = "hack> "
title = "HACKER TERM"
boot_text = ["Initializing..."]
"#;
        let skin =
            Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme_toml, strings_toml).unwrap();
        assert_eq!(skin.strings.prompt_format, "hack> ");
        assert_eq!(skin.strings.title, "HACKER TERM");
        assert_eq!(
            skin.theme.background_color(),
            crate::backend::Color::rgb(0, 0, 0)
        );
        assert!(skin.corrupted_modifiers.is_none());
    }

    #[test]
    fn corrupted_feature_creates_modifiers() {
        let features = r#"
terminal = true
corrupted = true
"#;
        let skin = Skin::from_toml(MANIFEST, LAYOUT, features).unwrap();
        assert!(skin.corrupted_modifiers.is_some());
    }

    #[test]
    fn swap_skin_replaces_sdi_objects() {
        let skin_a = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
        let mut sdi = SdiRegistry::new();
        skin_a.apply_layout(&mut sdi);
        assert!(sdi.contains("status_bar"));
        assert!(sdi.contains("content_bg"));

        let layout_b = r##"
[terminal_bg]
x = 0
y = 0
w = 480
h = 272
color = "#000000"
"##;
        let skin_b = Skin::from_toml(MANIFEST, layout_b, FEATURES).unwrap();
        let _new = Skin::swap(&skin_a, skin_b, &mut sdi);

        // Old objects removed.
        assert!(!sdi.contains("status_bar"));
        assert!(!sdi.contains("content_bg"));
        // New objects created.
        assert!(sdi.contains("terminal_bg"));
    }

    #[test]
    fn swap_preserves_non_layout_objects() {
        let skin_a = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
        let mut sdi = SdiRegistry::new();
        skin_a.apply_layout(&mut sdi);
        // Create an SDI object NOT defined in the layout (e.g., from WM).
        sdi.create("wm_object");

        let layout_b = r##"
[terminal_bg]
x = 0
y = 0
w = 480
h = 272
color = "#000000"
"##;
        let skin_b = Skin::from_toml(MANIFEST, layout_b, FEATURES).unwrap();
        let _new = Skin::swap(&skin_a, skin_b, &mut sdi);

        // WM object survives.
        assert!(sdi.contains("wm_object"));
    }

    #[test]
    fn from_toml_corrupted_custom_modifiers() {
        let corrupted_toml = r#"
position_jitter = 5
text_garble_chance = 0.3
intensity = 0.5
"#;
        let skin =
            Skin::from_toml_corrupted(MANIFEST, LAYOUT, FEATURES, "", "", corrupted_toml).unwrap();
        let mods = skin.corrupted_modifiers.unwrap();
        assert_eq!(mods.position_jitter, 5);
        assert!((mods.intensity - 0.5).abs() < f32::EPSILON);
    }

    // -- Malformed TOML tests --

    #[test]
    fn malformed_manifest_toml() {
        let bad = "this is [[[not valid";
        let result = Skin::from_toml(bad, LAYOUT, FEATURES);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("skin.toml"));
    }

    #[test]
    fn malformed_layout_toml() {
        let bad = "[unclosed";
        let result = Skin::from_toml(MANIFEST, bad, FEATURES);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("layout.toml"));
    }

    #[test]
    fn malformed_features_toml() {
        let bad = "dashboard = not_a_bool";
        let result = Skin::from_toml(MANIFEST, LAYOUT, bad);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("features.toml"));
    }

    #[test]
    fn malformed_theme_toml() {
        let bad = "color = [invalid";
        let result = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, bad, "");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("theme.toml"));
    }

    #[test]
    fn malformed_strings_toml() {
        let bad = "prompt_format = [oops";
        let result = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, "", bad);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("strings.toml"));
    }

    #[test]
    fn malformed_corrupted_toml() {
        let bad = "position_jitter = \"not a number\"";
        let result = Skin::from_toml_corrupted(MANIFEST, LAYOUT, FEATURES, "", "", bad);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("corrupted.toml"));
    }

    // -- Partial / minimal TOML tests --

    #[test]
    fn minimal_manifest_only_name() {
        let toml = r#"name = "bare""#;
        let m: SkinManifest = toml::from_str(toml).unwrap();
        assert_eq!(m.name, "bare");
        assert_eq!(m.version, "1.0");
        assert_eq!(m.author, "");
        assert_eq!(m.description, "");
        assert_eq!(m.screen_width, 480);
        assert_eq!(m.screen_height, 272);
    }

    #[test]
    fn empty_layout_produces_no_objects() {
        let skin = Skin::from_toml(MANIFEST, "", FEATURES).unwrap();
        assert!(skin.layout.objects.is_empty());
    }

    #[test]
    fn empty_features_uses_defaults() {
        let skin = Skin::from_toml(MANIFEST, LAYOUT, "").unwrap();
        assert!(skin.features.dashboard);
        assert!(skin.features.terminal);
        assert_eq!(skin.features.grid_cols, 3);
        assert_eq!(skin.features.grid_rows, 2);
    }

    #[test]
    fn partial_features_fills_defaults() {
        let features = r#"
dashboard = false
window_manager = true
"#;
        let skin = Skin::from_toml(MANIFEST, LAYOUT, features).unwrap();
        assert!(!skin.features.dashboard);
        assert!(skin.features.window_manager);
        // Defaults for unspecified fields:
        assert!(skin.features.terminal);
        assert!(skin.features.browser);
        assert_eq!(skin.features.dashboard_pages, 3);
    }

    // -- Layout object partial fields --

    #[test]
    fn layout_object_partial_fields() {
        let layout = r##"
[partial_obj]
x = 10
color = "#FF0000"
"##;
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        let obj = &skin.layout.objects["partial_obj"];
        assert_eq!(obj.x, Some(10));
        assert!(obj.y.is_none());
        assert!(obj.w.is_none());
        assert!(obj.h.is_none());
        assert_eq!(obj.color, Some("#FF0000".to_string()));
    }

    #[test]
    fn layout_object_extended_visual_properties() {
        let layout = r##"
[fancy]
x = 0
y = 0
w = 100
h = 50
border_radius = 8
gradient_top = "#FF0000"
gradient_bottom = "#0000FF"
shadow_level = 3
stroke_width = 2
stroke_color = "#00FF00"
"##;
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        let obj = &skin.layout.objects["fancy"];
        assert_eq!(obj.border_radius, Some(8));
        assert_eq!(obj.gradient_top, Some("#FF0000".to_string()));
        assert_eq!(obj.gradient_bottom, Some("#0000FF".to_string()));
        assert_eq!(obj.shadow_level, Some(3));
        assert_eq!(obj.stroke_width, Some(2));
        assert_eq!(obj.stroke_color, Some("#00FF00".to_string()));
    }

    #[test]
    fn apply_layout_extended_properties() {
        let layout = r##"
[styled]
x = 5
y = 10
w = 100
h = 50
border_radius = 4
shadow_level = 2
stroke_width = 1
stroke_color = "#FFFFFF"
gradient_top = "#FF0000"
gradient_bottom = "#0000FF"
"##;
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        let mut sdi = SdiRegistry::new();
        skin.apply_layout(&mut sdi);
        let obj = sdi.get("styled").unwrap();
        assert_eq!(obj.border_radius, Some(4));
        assert_eq!(obj.shadow_level, Some(2));
        assert_eq!(obj.stroke_width, Some(1));
    }

    // -- Invalid color strings --

    #[test]
    fn apply_layout_invalid_color_ignored() {
        let layout = r##"
[bad_color]
x = 0
y = 0
color = "not-a-color"
text_color = "also-bad"
"##;
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        let mut sdi = SdiRegistry::new();
        skin.apply_layout(&mut sdi);
        // Object created but colors remain default (parse_hex_color returns None)
        assert!(sdi.contains("bad_color"));
    }

    // -- Skin swap --

    #[test]
    fn swap_returns_new_skin() {
        let skin_a = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
        let mut sdi = SdiRegistry::new();
        skin_a.apply_layout(&mut sdi);

        let manifest_b = r#"name = "new_skin""#;
        let layout_b = r##"
[new_obj]
x = 0
y = 0
w = 480
h = 272
"##;
        let skin_b = Skin::from_toml(manifest_b, layout_b, FEATURES).unwrap();
        let result = Skin::swap(&skin_a, skin_b, &mut sdi);
        assert_eq!(result.manifest.name, "new_skin");
    }

    // -- Empty corrupted TOML skips modifiers --

    #[test]
    fn from_toml_corrupted_empty_string() {
        let skin = Skin::from_toml_corrupted(MANIFEST, LAYOUT, FEATURES, "", "", "").unwrap();
        // Empty corrupted_toml means no override, but features.corrupted is false
        // so corrupted_modifiers is None.
        assert!(skin.corrupted_modifiers.is_none());
    }

    // -- Discover skins with nonexistent directory --

    #[test]
    fn discover_skins_nonexistent_dir() {
        let skins = Skin::discover_skins(Path::new("/nonexistent/path/to/skins"));
        assert!(skins.is_empty());
    }

    // -- from_directory with nonexistent dir --

    #[test]
    fn from_directory_missing_files() {
        let result = Skin::from_directory(Path::new("/nonexistent/skin/dir"));
        assert!(result.is_err());
    }
}
