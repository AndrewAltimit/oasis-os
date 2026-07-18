//! Skin loading from TOML configuration files.

mod parsing;
mod validation;

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use serde::Deserialize;

use oasis_sdi::SdiRegistry;
use oasis_types::backend::{SdiBackend, TextureId};
use oasis_types::error::{OasisError, Result};

use super::assets::SkinAsset;
use super::corrupted::CorruptedModifiers;
use super::strings::SkinStrings;
use super::theme::SkinTheme;

pub use parsing::{SkinLayout, SkinObjectDef};

/// Top-level skin manifest (`skin.toml`).
#[derive(Debug, Clone, Deserialize)]
pub struct SkinManifest {
    /// Human-readable skin name.
    pub name: String,
    /// Skin version string (default "1.0").
    #[serde(default = "default_version")]
    pub version: String,
    /// Skin author name.
    #[serde(default)]
    pub author: String,
    /// Short description of the skin.
    #[serde(default)]
    pub description: String,
    /// Virtual screen width in pixels (default 480).
    #[serde(default = "default_width")]
    pub screen_width: u32,
    /// Virtual screen height in pixels (default 272).
    #[serde(default = "default_height")]
    pub screen_height: u32,
    /// Desktop render width override (default: PSP-native 480x272 skins
    /// are upscaled to 1280x720 on desktop; other sizes render as-is).
    #[serde(default)]
    pub desktop_width: Option<u32>,
    /// Desktop render height override (see `desktop_width`).
    #[serde(default)]
    pub desktop_height: Option<u32>,
    /// Parent skin to inherit from (built-in name).
    ///
    /// Child skin only needs to specify overrides; non-overridden fields
    /// come from the parent. Max depth 3 to prevent infinite recursion.
    #[serde(default)]
    pub inherits: Option<String>,
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
    /// Dashboard icon layout: `"grid"` (uniform cells, default) or `"free"`
    /// (desktop-style per-icon positions with column auto-flow and drag &
    /// drop on pointer targets).
    #[serde(default = "default_icon_layout")]
    pub icon_layout: String,
    /// In free layout, snap dragged icons to a virtual grid on drop
    /// (default true — matches real desktop behaviour).
    #[serde(default = "yes")]
    pub snap_to_grid: bool,
    /// Launch apps on a single pointer click (default true). When false,
    /// the first click only selects the icon and a second click launches.
    #[serde(default = "yes")]
    pub launch_on_single_click: bool,
    /// Draw the skin's own software mouse cursor instead of relying on the
    /// host OS pointer (default false). Themeable via `[cursor]` in
    /// theme.toml.
    #[serde(default)]
    pub software_cursor: bool,
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
    #[serde(default)]
    pub show_tabs: bool,
    /// Whether media category tabs (AUDIO/VIDEO/IMAGE/FILE) are shown in the
    /// bottom bar. Defaults to off — opt in per-skin for PSP-style layouts.
    #[serde(default)]
    pub show_media_tabs: bool,
    /// Whether page dots are shown in the bottom bar.
    #[serde(default = "yes")]
    pub show_page_dots: bool,
    /// Render the clock + date in the bottom-right of the taskbar instead of
    /// the top-right status bar (Windows XP style). The top-right clock is
    /// suppressed when this is set. Defaults to `true` for consistency
    /// across skins; opt out with `clock_in_bottombar = false`.
    #[serde(default = "yes")]
    pub clock_in_bottombar: bool,
    /// Custom fade transition duration in frames (default 15).
    #[serde(default)]
    pub transition_fade_frames: Option<u32>,
    /// Custom slide transition duration in frames (default 20).
    #[serde(default)]
    pub transition_slide_frames: Option<u32>,
    /// Disable animations for accessibility (defaults to false).
    #[serde(default)]
    pub reduced_motion: bool,
}

fn yes() -> bool {
    true
}
fn default_pages() -> u32 {
    1
}
fn default_icons_per_page() -> u32 {
    9
}
fn default_icon_layout() -> String {
    "grid".to_string()
}
fn default_grid_cols() -> u32 {
    3
}
fn default_grid_rows() -> u32 {
    3
}

impl Default for SkinFeatures {
    fn default() -> Self {
        Self {
            dashboard: true,
            terminal: true,
            file_browser: true,
            browser: true,
            window_manager: false,
            dashboard_pages: 1,
            icons_per_page: 15,
            icon_layout: "grid".to_string(),
            snap_to_grid: true,
            launch_on_single_click: true,
            software_cursor: false,
            grid_cols: 5,
            grid_rows: 3,
            command_categories: Vec::new(),
            start_menu: true,
            corrupted: false,
            show_battery: true,
            show_clock: true,
            show_version: true,
            show_tabs: false,
            show_media_tabs: false,
            show_page_dots: true,
            clock_in_bottombar: true,
            transition_fade_frames: None,
            transition_slide_frames: None,
            reduced_motion: false,
        }
    }
}

/// A fully loaded skin ready for use.
///
/// Contains all configuration data parsed from the skin's TOML files:
/// manifest metadata, layout definitions, feature gates, theme colors,
/// display strings, and optional corrupted visual modifiers.
#[derive(Debug, Clone)]
pub struct Skin {
    /// Skin metadata (name, version, author, screen dimensions).
    pub manifest: SkinManifest,
    /// SDI object layout definitions.
    pub layout: SkinLayout,
    /// Feature gates (dashboard, terminal, browser, etc.).
    pub features: SkinFeatures,
    /// Color scheme and visual properties.
    pub theme: SkinTheme,
    /// Localized display strings.
    pub strings: SkinStrings,
    /// Optional corrupted visual effect modifiers.
    pub corrupted_modifiers: Option<CorruptedModifiers>,
    /// Unknown TOML keys encountered during parsing (`file: key` per entry).
    /// Unknown keys are ignored for forwards compatibility, but surfaced so
    /// skin authors can catch typos and unsupported fields.
    pub schema_warnings: Vec<String>,
    /// Decoded image assets keyed by skin-relative path
    /// (e.g. `"assets/bar_top.png"`). Populated from the skin directory's
    /// `assets/` folder or embedded bytes for built-in skins.
    pub assets: HashMap<String, SkinAsset>,
    /// Raw WAV sound assets keyed by skin-relative path
    /// (e.g. `"assets/click.wav"`), referenced by the theme's `[sounds]`
    /// table. Decoding to PCM happens at playback-load time in the shell,
    /// so `oasis-skin` only stores and header-validates the bytes.
    pub sound_assets: HashMap<String, Vec<u8>>,
}

/// Parse a TOML document, collecting any keys the target type ignores.
///
/// Unknown keys never fail the parse — they are recorded as `"{file}:
/// unknown key `{path}`"` warnings so authors get feedback instead of
/// silently dead configuration.
fn parse_toml_lint<T: serde::de::DeserializeOwned>(
    content: &str,
    file: &str,
    warnings: &mut Vec<String>,
) -> Result<T> {
    let de = toml::Deserializer::new(content);
    serde_ignored::deserialize(de, |path| {
        // serde_ignored renders Option-wrapped struct hops as `?` segments
        // (e.g. `wm_theme.?.close_button_color`); strip them for readability.
        let path = path.to_string().replace(".?.", ".");
        warnings.push(format!("{file}: unknown key `{path}`"));
    })
    .map_err(|e| OasisError::Config(format!("{file}: {e}").into()))
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
        let mut schema_warnings = Vec::new();
        let manifest: SkinManifest =
            parse_toml_lint(manifest_toml, "skin.toml", &mut schema_warnings)?;
        let layout: SkinLayout = parse_toml_lint(layout_toml, "layout.toml", &mut schema_warnings)?;
        let mut features: SkinFeatures =
            parse_toml_lint(features_toml, "features.toml", &mut schema_warnings)?;

        let theme: SkinTheme = if theme_toml.is_empty() {
            SkinTheme::default()
        } else {
            parse_toml_lint(theme_toml, "theme.toml", &mut schema_warnings)?
        };

        let strings: SkinStrings = if strings_toml.is_empty() {
            SkinStrings::default()
        } else {
            parse_toml_lint(strings_toml, "strings.toml", &mut schema_warnings)?
        };

        // Millisecond transition durations from the theme convert to frames
        // (60 fps); explicit frame counts in features.toml take precedence.
        if let Some(t) = theme.transition.as_ref() {
            if features.transition_fade_frames.is_none()
                && let Some(ms) = t.fade_ms
            {
                features.transition_fade_frames = Some((ms * 60 / 1000).max(1));
            }
            if features.transition_slide_frames.is_none()
                && let Some(ms) = t.slide_ms
            {
                features.transition_slide_frames = Some((ms * 60 / 1000).max(1));
            }
        }

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
            schema_warnings,
            assets: HashMap::new(),
            sound_assets: HashMap::new(),
        })
    }

    /// Decode a PNG and register it as a named asset (e.g.
    /// `"assets/bar_top.png"`). Used by generated built-in skins to attach
    /// `include_bytes!` payloads and by tests.
    pub fn add_asset_png(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        let asset = SkinAsset::from_png_bytes(bytes)
            .map_err(|e| OasisError::Config(format!("{name}: {e}").into()))?;
        self.assets.insert(name.to_string(), asset);
        Ok(())
    }

    /// Register raw WAV bytes as a named sound asset (e.g.
    /// `"assets/click.wav"`). The header is checked here so obviously
    /// broken files fail loudly at load time; full decode happens in the
    /// shell's SFX player.
    pub fn add_asset_wav(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        if super::assets::probe_wav(bytes).is_none() {
            return Err(OasisError::Config(
                format!("{name}: not an uncompressed PCM WAV").into(),
            ));
        }
        self.sound_assets.insert(name.to_string(), bytes.to_vec());
        Ok(())
    }

    /// Upload layout-referenced textures and attach them to their SDI
    /// objects. Call after `apply_layout`/`swap_scaled` once a backend is
    /// available. Returns the created texture ids; the caller owns them and
    /// must destroy them on skin swap-out (SDI object destruction does not
    /// free backend textures).
    ///
    /// Objects without explicit `w`/`h` in the layout inherit the asset's
    /// native pixel dimensions.
    pub fn upload_layout_textures(
        &self,
        sdi: &mut SdiRegistry,
        backend: &mut dyn SdiBackend,
    ) -> Vec<TextureId> {
        let mut ids = Vec::new();
        for (name, def) in &self.layout.objects {
            // Nine-patch takes precedence over a plain texture.
            let asset_name = match (&def.nine_patch, &def.texture) {
                (Some(np), _) => &np.image,
                (None, Some(tex)) => tex,
                (None, None) => continue,
            };
            let Some(asset) = self.assets.get(asset_name) else {
                log::warn!(
                    "skin '{}': object '{name}' references missing asset '{asset_name}'",
                    self.manifest.name
                );
                continue;
            };
            let tex = match backend.load_texture(asset.width, asset.height, &asset.rgba) {
                Ok(tex) => tex,
                Err(e) => {
                    log::warn!(
                        "skin '{}': texture upload for '{name}' failed: {e}",
                        self.manifest.name
                    );
                    continue;
                },
            };
            if let Ok(obj) = sdi.get_mut(name) {
                obj.texture = Some(tex);
                obj.nine_patch = def.nine_patch.as_ref().map(|np| {
                    let [left, top, right, bottom] = np.insets;
                    oasis_types::nine_patch::NinePatchSlices {
                        tex_width: asset.width,
                        tex_height: asset.height,
                        left,
                        top,
                        right,
                        bottom,
                    }
                });
                if def.w.is_none() {
                    obj.w = asset.width;
                }
                if def.h.is_none() {
                    obj.h = asset.height;
                }
                ids.push(tex);
            } else {
                // Layout applied and textures uploaded should always agree,
                // but never leak a texture if the object vanished.
                let _ = backend.destroy_texture(tex);
            }
        }
        ids
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
            let mut warnings = Vec::new();
            let modifiers: CorruptedModifiers =
                parse_toml_lint(corrupted_toml, "corrupted.toml", &mut warnings)?;
            skin.corrupted_modifiers = Some(modifiers);
            skin.schema_warnings.append(&mut warnings);
        }

        Ok(skin)
    }

    /// Apply this skin's layout to an SDI registry. Existing objects are
    /// updated, missing objects are created.
    pub fn apply_layout(&self, sdi: &mut SdiRegistry) {
        parsing::apply_layout_inner(&self.layout, sdi, 1.0, 1.0);
    }

    /// Apply layout scaled from the skin's native resolution to the target
    /// screen size. Positions and sizes are proportionally adjusted.
    pub fn apply_layout_scaled(&self, sdi: &mut SdiRegistry, target_w: u32, target_h: u32) {
        let base_w = self.manifest.screen_width.max(1) as f64;
        let base_h = self.manifest.screen_height.max(1) as f64;
        parsing::apply_layout_inner(
            &self.layout,
            sdi,
            target_w as f64 / base_w,
            target_h as f64 / base_h,
        );
    }

    /// Load a skin from a directory containing TOML files.
    ///
    /// Requires `skin.toml`, `layout.toml`, and `features.toml`.
    /// Optional files: `theme.toml`, `strings.toml`, `corrupted.toml`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_directory(dir: &Path) -> Result<Self> {
        let read = |name: &str| -> Result<String> {
            let p = dir.join(name);
            std::fs::read_to_string(&p)
                .map_err(|e| OasisError::Config(format!("{}: {e}", p.display()).into()))
        };
        let read_opt =
            |name: &str| -> String { std::fs::read_to_string(dir.join(name)).unwrap_or_default() };

        let manifest = read("skin.toml")?;
        let layout = read("layout.toml")?;
        let features = read("features.toml")?;
        let theme = read_opt("theme.toml");
        let strings = read_opt("strings.toml");
        let corrupted = read_opt("corrupted.toml");

        let mut skin = if corrupted.is_empty() {
            Self::from_toml_full(&manifest, &layout, &features, &theme, &strings)?
        } else {
            Self::from_toml_corrupted(&manifest, &layout, &features, &theme, &strings, &corrupted)?
        };

        // Load image assets from the skin's assets/ subdirectory.
        skin.load_assets_from(&dir.join("assets"));

        // Apply inheritance from a built-in parent skin if specified.
        if let Some(ref parent_name) = skin.manifest.inherits
            && let Ok(parent) = super::builtin::load_builtin(parent_name)
        {
            skin.merge_theme_from(&parent);
        }

        for warning in &skin.schema_warnings {
            log::warn!("skin '{}': {warning}", skin.manifest.name);
        }

        Ok(skin)
    }

    /// Decode every `*.png` (images) and `*.wav` (UI sounds) in an assets
    /// directory into `self.assets` / `self.sound_assets`, keyed
    /// `"assets/<file>"`. Files that fail to decode are recorded as
    /// schema warnings instead of failing the whole skin load.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_assets_from(&mut self, assets_dir: &Path) {
        let Ok(entries) = std::fs::read_dir(assets_dir) else {
            return; // No assets/ directory -- nothing to do.
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("png") || ext.eq_ignore_ascii_case("wav")
                })
            })
            .collect();
        files.sort();
        for path in files {
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let is_wav = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"));
            let key = format!("assets/{file_name}");
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let result = if is_wav {
                        self.add_asset_wav(&key, &bytes)
                    } else {
                        self.add_asset_png(&key, &bytes)
                    };
                    if let Err(e) = result {
                        self.schema_warnings.push(format!("{key}: {e}"));
                    }
                },
                Err(e) => {
                    self.schema_warnings.push(format!("{key}: {e}"));
                },
            }
        }
    }

    /// Scan a directory for skin subdirectories (those containing `skin.toml`).
    ///
    /// Returns `(name, path)` pairs sorted by name.
    #[cfg(not(target_arch = "wasm32"))]
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

    /// Tear down and rebuild with layout scaled to the target screen size.
    pub fn swap_scaled(
        current: &Skin,
        new_skin: Skin,
        sdi: &mut SdiRegistry,
        target_w: u32,
        target_h: u32,
    ) -> Skin {
        for name in current.layout.objects.keys() {
            let _ = sdi.destroy(name);
        }
        new_skin.apply_layout_scaled(sdi, target_w, target_h);
        new_skin
    }

    /// Merge missing theme fields from a parent skin.
    ///
    /// Only fills in fields that are `None` / empty in the child. Explicitly
    /// set fields in the child always win.
    pub fn merge_theme_from(&mut self, parent: &Skin) {
        let pt = &parent.theme;
        let ct = &mut self.theme;

        // Only override extended optional fields when the child hasn't set them.
        if ct.surface.is_none() {
            ct.surface.clone_from(&pt.surface);
        }
        if ct.accent.is_none() {
            ct.accent.clone_from(&pt.accent);
        }
        if ct.accent_hover.is_none() {
            ct.accent_hover.clone_from(&pt.accent_hover);
        }
        if ct.success.is_none() {
            ct.success.clone_from(&pt.success);
        }
        if ct.warning.is_none() {
            ct.warning.clone_from(&pt.warning);
        }
        if ct.border_radius.is_none() {
            ct.border_radius = pt.border_radius;
        }
        if ct.shadow_intensity.is_none() {
            ct.shadow_intensity = pt.shadow_intensity;
        }
        if ct.gradient_enabled.is_none() {
            ct.gradient_enabled = pt.gradient_enabled;
        }
        if ct.wm_theme.is_none() {
            ct.wm_theme.clone_from(&pt.wm_theme);
        }
        if ct.bar_overrides.is_none() {
            ct.bar_overrides.clone_from(&pt.bar_overrides);
        }
        if ct.icon_overrides.is_none() {
            ct.icon_overrides.clone_from(&pt.icon_overrides);
        }
        if ct.browser_overrides.is_none() {
            ct.browser_overrides.clone_from(&pt.browser_overrides);
        }
        if ct.app_overrides.is_none() {
            ct.app_overrides.clone_from(&pt.app_overrides);
        }
        if ct.osk_overrides.is_none() {
            ct.osk_overrides.clone_from(&pt.osk_overrides);
        }
        if ct.start_menu_overrides.is_none() {
            ct.start_menu_overrides.clone_from(&pt.start_menu_overrides);
        }
        if ct.wallpaper.is_none() {
            ct.wallpaper.clone_from(&pt.wallpaper);
        }
        if ct.cursor.is_none() {
            ct.cursor.clone_from(&pt.cursor);
        }
        if ct.background_layers.is_none() {
            ct.background_layers.clone_from(&pt.background_layers);
        }
        if ct.chrome_layers.is_none() {
            ct.chrome_layers.clone_from(&pt.chrome_layers);
        }
        if ct.geometry.is_none() {
            ct.geometry.clone_from(&pt.geometry);
        }
        if ct.typography.is_none() {
            ct.typography.clone_from(&pt.typography);
        }
        if ct.transition.is_none() {
            ct.transition.clone_from(&pt.transition);
        }
        if ct.scrollbar_overrides.is_none() {
            ct.scrollbar_overrides.clone_from(&pt.scrollbar_overrides);
        }
        // Merge collection fields: only fill if child has none.
        if ct.app_themes.is_none() {
            ct.app_themes.clone_from(&pt.app_themes);
        }
        if ct.gradients.is_none() {
            ct.gradients.clone_from(&pt.gradients);
        }
        if ct.animations.is_none() {
            ct.animations.clone_from(&pt.animations);
        }
        if ct.widget_states.is_none() {
            ct.widget_states.clone_from(&pt.widget_states);
        }

        // Merge layout: parent objects fill in missing child objects.
        for (name, def) in &parent.layout.objects {
            if !self.layout.objects.contains_key(name) {
                self.layout.objects.insert(name.clone(), def.clone());
            }
        }

        // Merge assets: parent images fill in missing child assets so
        // inherited layout objects keep their textures.
        for (name, asset) in &parent.assets {
            if !self.assets.contains_key(name) {
                self.assets.insert(name.clone(), asset.clone());
            }
        }

        // Merge UI sounds the same way: the parent's [sounds] table fills
        // in when the child has none, and parent WAVs back inherited paths.
        if ct.sounds.is_none() {
            ct.sounds.clone_from(&pt.sounds);
        }
        for (name, bytes) in &parent.sound_assets {
            if !self.sound_assets.contains_key(name) {
                self.sound_assets.insert(name.clone(), bytes.clone());
            }
        }
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
        assert_eq!(f.dashboard_pages, 1);
        assert_eq!(f.icons_per_page, 15);
        assert_eq!(f.grid_cols, 5);
        assert_eq!(f.grid_rows, 3);
        assert!(f.clock_in_bottombar);
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
            oasis_types::backend::Color::rgb(0, 0, 0)
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
        assert_eq!(skin.features.grid_rows, 3);
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
        assert_eq!(skin.features.dashboard_pages, 1);
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
    fn layout_object_text_shadow_properties() {
        let layout = r##"
[shadowed]
x = 0
y = 0
w = 100
h = 50
text_shadow_dx = 2
text_shadow_dy = 3
text_shadow_color = "#00000080"
shadow_color = "#FF000040"
"##;
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        let obj = &skin.layout.objects["shadowed"];
        assert_eq!(obj.text_shadow_dx, Some(2));
        assert_eq!(obj.text_shadow_dy, Some(3));
        assert_eq!(obj.text_shadow_color, Some("#00000080".to_string()));
        assert_eq!(obj.shadow_color, Some("#FF000040".to_string()));
    }

    #[test]
    fn apply_layout_text_shadow_properties() {
        let layout = r##"
[shadow_obj]
x = 0
y = 0
w = 100
h = 50
text_shadow_dx = 1
text_shadow_dy = 2
text_shadow_color = "#00000080"
shadow_color = "#FF000040"
"##;
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        let mut sdi = SdiRegistry::new();
        skin.apply_layout(&mut sdi);
        let obj = sdi.get("shadow_obj").unwrap();
        assert_eq!(obj.text_shadow_offset, Some((1, 2)));
        assert_eq!(
            obj.text_shadow_color,
            Some(oasis_types::backend::Color::rgba(0, 0, 0, 128))
        );
        assert_eq!(
            obj.shadow_color,
            Some(oasis_types::backend::Color::rgba(255, 0, 0, 64))
        );
    }

    #[test]
    fn apply_layout_text_shadow_defaults_dy() {
        let layout = r##"
[shadow_dx_only]
x = 0
y = 0
text_shadow_dx = 3
"##;
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        let mut sdi = SdiRegistry::new();
        skin.apply_layout(&mut sdi);
        let obj = sdi.get("shadow_dx_only").unwrap();
        assert_eq!(obj.text_shadow_offset, Some((3, 1)));
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

    // -- Robustness / edge cases ----------------------------------------

    #[test]
    fn completely_invalid_manifest_toml() {
        let result = Skin::from_toml("{{{{", LAYOUT, FEATURES);
        assert!(result.is_err());
    }

    #[test]
    fn completely_invalid_layout_toml() {
        let result = Skin::from_toml(MANIFEST, "{{{{", FEATURES);
        assert!(result.is_err());
    }

    #[test]
    fn completely_invalid_features_toml() {
        let result = Skin::from_toml(MANIFEST, LAYOUT, "{{{{");
        assert!(result.is_err());
    }

    #[test]
    fn empty_manifest_toml() {
        let result = Skin::from_toml("", LAYOUT, FEATURES);
        // Empty manifest lacks required `name` -- should fail or use defaults.
        let _ = result;
    }

    #[test]
    fn empty_layout_toml() {
        let skin = Skin::from_toml(MANIFEST, "", FEATURES).unwrap();
        assert!(skin.layout.objects.is_empty());
    }

    #[test]
    fn empty_features_toml() {
        let skin = Skin::from_toml(MANIFEST, LAYOUT, "").unwrap();
        // Empty features should use defaults.
        let _ = skin.features;
    }

    #[test]
    fn manifest_with_extra_fields() {
        let manifest = r#"
name = "test"
version = "1.0"
future_field = "should be ignored"
another_unknown = 42
"#;
        let skin = Skin::from_toml(manifest, LAYOUT, FEATURES).unwrap();
        assert_eq!(skin.manifest.name, "test");
        // Unknown keys parse fine but are recorded as schema warnings.
        assert!(
            skin.schema_warnings
                .iter()
                .any(|w| w.contains("skin.toml") && w.contains("future_field")),
            "missing schema warning: {:?}",
            skin.schema_warnings
        );
    }

    #[test]
    fn unknown_theme_keys_are_recorded() {
        let theme = r##"
background = "#000000"
not_a_real_field = true

[wm_theme]
titlebar_height = 20
close_button_colour = "#FF0000"

[transition]
bogus_ms = 300
"##;
        let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
        let has = |s: &str| skin.schema_warnings.iter().any(|w| w.contains(s));
        assert!(has("not_a_real_field"), "{:?}", skin.schema_warnings);
        assert!(
            has("wm_theme.close_button_colour"),
            "{:?}",
            skin.schema_warnings
        );
        assert!(has("transition.bogus_ms"), "{:?}", skin.schema_warnings);
        // Warnings also flow through validate().
        assert!(skin.validate().iter().any(|w| w.contains("bogus_ms")));
    }

    #[test]
    fn transition_ms_converts_to_frames() {
        let theme = r#"
[transition]
fade_ms = 300
slide_ms = 400
"#;
        let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
        assert_eq!(skin.features.transition_fade_frames, Some(18));
        assert_eq!(skin.features.transition_slide_frames, Some(24));
    }

    #[test]
    fn explicit_frames_beat_transition_ms() {
        let features = r#"
transition_fade_frames = 5
"#;
        let theme = r#"
[transition]
fade_ms = 1000
"#;
        let skin = Skin::from_toml_full(MANIFEST, LAYOUT, features, theme, "").unwrap();
        assert_eq!(skin.features.transition_fade_frames, Some(5));
    }

    #[test]
    fn bar_text_color_fallback() {
        let theme = r##"
[bar_overrides]
text_color = "#123456"
clock_color = "#654321"
"##;
        let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
        let at = crate::ActiveTheme::from_skin(&skin.theme);
        // Specific color wins; unset elements fall back to text_color.
        assert_eq!(
            at.bar.clock_color,
            oasis_types::backend::Color::rgb(0x65, 0x43, 0x21)
        );
        assert_eq!(
            at.bar.version_color,
            oasis_types::backend::Color::rgb(0x12, 0x34, 0x56)
        );
        assert_eq!(
            at.bar.url_color,
            oasis_types::backend::Color::rgb(0x12, 0x34, 0x56)
        );
    }

    #[test]
    fn all_shipped_skins_lint_clean() {
        let mut report = String::new();
        for name in crate::builtin::builtin_names() {
            let skin = crate::builtin::load_builtin(name).unwrap();
            for w in &skin.schema_warnings {
                report.push_str(&format!("builtin '{name}': {w}\n"));
            }
        }
        for name in crate::builtin::generated::GENERATED_SKIN_NAMES {
            let skin = crate::builtin::generated::load_generated_skin(name)
                .unwrap()
                .unwrap();
            for w in &skin.schema_warnings {
                report.push_str(&format!("generated '{name}': {w}\n"));
            }
        }
        assert!(
            report.is_empty(),
            "shipped skins have schema warnings:\n{report}"
        );
    }

    #[test]
    fn layout_object_invalid_color() {
        let layout = r##"
[obj]
x = 0
y = 0
w = 100
h = 50
color = "not_a_color"
"##;
        // Invalid color should not crash -- may use fallback or ignore.
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        assert_eq!(skin.layout.objects.len(), 1);
    }

    #[test]
    fn layout_object_empty_color() {
        let layout = r##"
[obj]
x = 0
y = 0
w = 100
h = 50
color = ""
"##;
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        assert_eq!(skin.layout.objects.len(), 1);
    }

    #[test]
    fn layout_object_zero_dimensions() {
        let layout = r##"
[zero_obj]
x = 0
y = 0
w = 0
h = 0
color = "#FF0000"
"##;
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        assert_eq!(skin.layout.objects.len(), 1);
    }

    #[test]
    fn layout_object_negative_position() {
        let layout = r##"
[neg_obj]
x = -100
y = -50
w = 200
h = 100
color = "#00FF00"
"##;
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        assert_eq!(skin.layout.objects.len(), 1);
    }

    #[test]
    fn layout_many_objects() {
        let mut layout = String::new();
        for i in 0..100 {
            layout.push_str(&format!(
                r##"
[obj_{i}]
x = {x}
y = {y}
w = 10
h = 10
color = "#AABBCC"
"##,
                x = (i % 48) * 10,
                y = (i / 48) * 10,
            ));
        }
        let skin = Skin::from_toml(MANIFEST, &layout, FEATURES).unwrap();
        assert_eq!(skin.layout.objects.len(), 100);
    }

    #[test]
    fn features_boolean_fields() {
        let features = r#"
dashboard = false
terminal = false
file_browser = false
browser = false
window_manager = true
"#;
        let skin = Skin::from_toml(MANIFEST, LAYOUT, features).unwrap();
        assert!(!skin.features.dashboard);
        assert!(skin.features.window_manager);
    }

    #[test]
    fn manifest_unicode_name() {
        let manifest = "name = \"\u{30b9}\u{30ad}\u{30f3}\"\nversion = \"1.0\"\nauthor = \"\u{30c6}\u{30b9}\u{30c8}\"\n";
        let skin = Skin::from_toml(manifest, LAYOUT, FEATURES).unwrap();
        assert_eq!(skin.manifest.name, "\u{30b9}\u{30ad}\u{30f3}");
    }

    #[test]
    fn manifest_very_long_name() {
        let name = "x".repeat(1000);
        let manifest = format!("name = \"{name}\"");
        let skin = Skin::from_toml(&manifest, LAYOUT, FEATURES).unwrap();
        assert_eq!(skin.manifest.name.len(), 1000);
    }

    // -- Skin inheritance tests --

    #[test]
    fn manifest_inherits_field() {
        let manifest = r#"
name = "child"
inherits = "classic"
"#;
        let skin = Skin::from_toml(manifest, LAYOUT, FEATURES).unwrap();
        assert_eq!(skin.manifest.inherits.as_deref(), Some("classic"));
    }

    #[test]
    fn manifest_inherits_default_none() {
        let skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
        assert!(skin.manifest.inherits.is_none());
    }

    #[test]
    fn merge_theme_fills_missing_fields() {
        let parent_theme = r##"
border_radius = 8
shadow_intensity = 2

[gradients.accent]
from = "#FF0000"
to = "#880000"
"##;
        let mut parent =
            Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, parent_theme, "").unwrap();
        // Give parent a unique name.
        parent.manifest.name = "parent".to_string();

        // Child has no border_radius or gradients.
        let mut child = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
        assert!(child.theme.border_radius.is_none());
        assert!(child.theme.gradients.is_none());

        child.merge_theme_from(&parent);
        assert_eq!(child.theme.border_radius, Some(8));
        assert_eq!(child.theme.shadow_intensity, Some(2));
        assert!(child.theme.gradients.is_some());
    }

    #[test]
    fn merge_theme_child_overrides_win() {
        let parent_theme = r##"
border_radius = 8
shadow_intensity = 2
"##;
        let child_theme = r##"
border_radius = 4
"##;
        let parent = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, parent_theme, "").unwrap();
        let mut child = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, child_theme, "").unwrap();

        child.merge_theme_from(&parent);
        // Child's border_radius should win over parent's.
        assert_eq!(child.theme.border_radius, Some(4));
        // Parent's shadow_intensity fills in.
        assert_eq!(child.theme.shadow_intensity, Some(2));
    }

    // -- External skin loading tests --

    #[test]
    fn load_macos_skin() {
        let skin = Skin::from_toml_full(
            include_str!("../../../../skins/macos/skin.toml"),
            include_str!("../../../../skins/macos/layout.toml"),
            include_str!("../../../../skins/macos/features.toml"),
            include_str!("../../../../skins/macos/theme.toml"),
            "",
        )
        .unwrap();
        assert_eq!(skin.manifest.name, "macos");
        assert_eq!(skin.manifest.screen_width, 800);
        assert_eq!(skin.manifest.screen_height, 600);
        assert!(skin.features.window_manager);
        assert!(skin.features.start_menu);
        assert!(skin.theme.wm_theme.is_some());
        let wm = skin.theme.wm_theme.as_ref().unwrap();
        assert_eq!(wm.button_side.as_deref(), Some("left"));
    }

    #[test]
    fn load_gnome_skin() {
        let skin = Skin::from_toml_full(
            include_str!("../../../../skins/gnome/skin.toml"),
            include_str!("../../../../skins/gnome/layout.toml"),
            include_str!("../../../../skins/gnome/features.toml"),
            include_str!("../../../../skins/gnome/theme.toml"),
            "",
        )
        .unwrap();
        assert_eq!(skin.manifest.name, "gnome");
        assert_eq!(skin.manifest.screen_width, 800);
        assert!(skin.features.window_manager);
        assert_eq!(skin.theme.border_radius, Some(12));
        assert_eq!(skin.theme.gradient_enabled, Some(false));
    }

    #[test]
    fn load_retro_cga_skin() {
        let skin = Skin::from_toml_full(
            include_str!("../../../../skins/retro-cga/skin.toml"),
            include_str!("../../../../skins/retro-cga/layout.toml"),
            include_str!("../../../../skins/retro-cga/features.toml"),
            include_str!("../../../../skins/retro-cga/theme.toml"),
            "",
        )
        .unwrap();
        assert_eq!(skin.manifest.name, "retro-cga");
        assert_eq!(skin.manifest.screen_width, 480);
        assert!(!skin.features.window_manager);
        assert_eq!(skin.theme.border_radius, Some(0));
        assert_eq!(skin.theme.shadow_intensity, Some(0));
        assert_eq!(skin.theme.gradient_enabled, Some(false));
        assert_eq!(skin.theme.background, "#000000");
    }

    #[test]
    fn load_balatro_skin() {
        let skin = Skin::from_toml_full(
            include_str!("../../../../skins/balatro/skin.toml"),
            include_str!("../../../../skins/balatro/layout.toml"),
            include_str!("../../../../skins/balatro/features.toml"),
            include_str!("../../../../skins/balatro/theme.toml"),
            "",
        )
        .unwrap();
        assert_eq!(skin.manifest.name, "balatro");
        assert_eq!(skin.manifest.screen_width, 800);
        assert!(skin.features.window_manager);
        assert_eq!(skin.theme.primary, "#00F0FF");
        assert_eq!(skin.theme.shadow_intensity, Some(2));
        assert!(skin.theme.wm_theme.is_some());
    }

    #[test]
    fn load_paper_skin() {
        let skin = Skin::from_toml_full(
            include_str!("../../../../skins/paper/skin.toml"),
            include_str!("../../../../skins/paper/layout.toml"),
            include_str!("../../../../skins/paper/features.toml"),
            include_str!("../../../../skins/paper/theme.toml"),
            "",
        )
        .unwrap();
        assert_eq!(skin.manifest.name, "paper");
        assert_eq!(skin.manifest.screen_width, 480);
        assert!(!skin.features.window_manager);
        assert_eq!(skin.theme.border_radius, Some(0));
        assert_eq!(skin.theme.shadow_intensity, Some(0));
        assert_eq!(skin.theme.gradient_enabled, Some(false));
        assert_eq!(skin.theme.background, "#FAF8F0");
    }

    #[test]
    fn load_win95_skin() {
        let skin = Skin::from_toml_full(
            include_str!("../../../../skins/win95/skin.toml"),
            include_str!("../../../../skins/win95/layout.toml"),
            include_str!("../../../../skins/win95/features.toml"),
            include_str!("../../../../skins/win95/theme.toml"),
            "",
        )
        .unwrap();
        assert_eq!(skin.manifest.name, "win95");
        assert_eq!(skin.manifest.screen_width, 640);
        assert_eq!(skin.manifest.screen_height, 480);
        assert!(skin.features.window_manager);
        assert_eq!(skin.theme.border_radius, Some(0));
        assert_eq!(skin.theme.gradient_enabled, Some(false));
        assert_eq!(skin.theme.background, "#008080");
    }

    #[test]
    fn load_solarized_skin() {
        let skin = Skin::from_toml_full(
            include_str!("../../../../skins/solarized/skin.toml"),
            include_str!("../../../../skins/solarized/layout.toml"),
            include_str!("../../../../skins/solarized/features.toml"),
            include_str!("../../../../skins/solarized/theme.toml"),
            "",
        )
        .unwrap();
        assert_eq!(skin.manifest.name, "solarized");
        assert_eq!(skin.manifest.screen_width, 800);
        assert_eq!(skin.manifest.screen_height, 600);
        assert!(skin.features.window_manager);
        assert_eq!(skin.theme.background, "#002B36");
    }

    #[test]
    fn load_vaporwave_skin() {
        let skin = Skin::from_toml_full(
            include_str!("../../../../skins/vaporwave/skin.toml"),
            include_str!("../../../../skins/vaporwave/layout.toml"),
            include_str!("../../../../skins/vaporwave/features.toml"),
            include_str!("../../../../skins/vaporwave/theme.toml"),
            "",
        )
        .unwrap();
        assert_eq!(skin.manifest.name, "vaporwave");
        assert_eq!(skin.manifest.screen_width, 480);
        assert!(!skin.features.window_manager);
        assert_eq!(skin.theme.gradient_enabled, Some(true));
        assert_eq!(skin.theme.background, "#1A0A2E");
    }

    #[test]
    fn load_highcontrast_skin() {
        let skin = Skin::from_toml_full(
            include_str!("../../../../skins/highcontrast/skin.toml"),
            include_str!("../../../../skins/highcontrast/layout.toml"),
            include_str!("../../../../skins/highcontrast/features.toml"),
            include_str!("../../../../skins/highcontrast/theme.toml"),
            "",
        )
        .unwrap();
        assert_eq!(skin.manifest.name, "highcontrast");
        assert_eq!(skin.theme.border_radius, Some(0));
        assert_eq!(skin.theme.shadow_intensity, Some(0));
        assert_eq!(skin.theme.gradient_enabled, Some(false));
        assert_eq!(skin.theme.background, "#000000");
        assert_eq!(skin.theme.text, "#FFFFFF");
    }

    #[test]
    fn merge_theme_layout_merges() {
        let parent_layout = r#"
[parent_object]
x = 10
y = 20
w = 100
h = 50
"#;
        let parent = Skin::from_toml(MANIFEST, parent_layout, FEATURES).unwrap();
        let mut child = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
        let had_parent_obj = child.layout.objects.contains_key("parent_object");
        assert!(!had_parent_obj);

        child.merge_theme_from(&parent);
        assert!(child.layout.objects.contains_key("parent_object"));
    }

    // -- Validation tests --

    #[test]
    fn validate_valid_skin() {
        let skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
        let warnings = skin.validate();
        assert!(warnings.is_empty(), "expected no warnings: {warnings:?}");
    }

    #[test]
    fn validate_empty_name() {
        let manifest = r#"name = """#;
        let skin = Skin::from_toml(manifest, LAYOUT, FEATURES).unwrap();
        let warnings = skin.validate();
        assert!(
            warnings.iter().any(|w| w.contains("name is empty")),
            "missing empty name warning: {warnings:?}"
        );
    }

    #[test]
    fn validate_invalid_theme_color() {
        let theme_toml = "background = \"not-a-color\"\nprimary = \"#FF0000\"\n";
        let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme_toml, "").unwrap();
        let warnings = skin.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("background") && w.contains("not-a-color")),
            "missing invalid color warning: {warnings:?}"
        );
    }

    #[test]
    fn validate_invalid_layout_color() {
        let layout = "[bad_obj]\nx = 10\ny = 20\ncolor = \"xyz\"\n";
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        let warnings = skin.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("bad_obj") && w.contains("xyz")),
            "missing invalid layout color warning: {warnings:?}"
        );
    }

    #[test]
    fn validate_extreme_coordinates() {
        let layout = "[far_away]\nx = 999999\ny = 0\n";
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        let warnings = skin.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("far_away") && w.contains("x=")),
            "missing out-of-bounds warning: {warnings:?}"
        );
    }

    #[test]
    fn validate_icons_exceed_grid() {
        let features = "dashboard = true\nicons_per_page = 20\ngrid_cols = 3\ngrid_rows = 3\n";
        let skin = Skin::from_toml(MANIFEST, LAYOUT, features).unwrap();
        let warnings = skin.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("icons_per_page") && w.contains("grid capacity")),
            "missing grid capacity warning: {warnings:?}"
        );
    }

    #[test]
    fn validate_chrome_layer_unsupported_kind() {
        let theme_toml = r#"
[[chrome_layers]]
kind = "image"
source = "assets/x.png"

[[chrome_layers]]
kind = "crosshair"
size = 12
"#;
        let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme_toml, "").unwrap();
        let warnings = skin.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("chrome_layers[0]") && w.contains("image")),
            "missing chrome layer kind warning: {warnings:?}"
        );
        assert!(
            !warnings.iter().any(|w| w.contains("chrome_layers[1]")),
            "vector kind should not warn: {warnings:?}"
        );
    }

    #[test]
    fn validate_zero_screen_dimensions() {
        let manifest = "name = \"zero\"\nscreen_width = 0\nscreen_height = 0\n";
        let skin = Skin::from_toml(manifest, LAYOUT, FEATURES).unwrap();
        let warnings = skin.validate();
        assert!(
            warnings.iter().any(|w| w.contains("screen_width is 0")),
            "missing zero width warning: {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("screen_height is 0")),
            "missing zero height warning: {warnings:?}"
        );
    }

    // -- Asset tests --

    fn png_bytes() -> Vec<u8> {
        crate::assets::tests::encode_png(8, 8, png::ColorType::Rgba)
    }

    #[test]
    fn add_asset_png_registers_asset() {
        let mut skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
        skin.add_asset_png("assets/logo.png", &png_bytes()).unwrap();
        let asset = &skin.assets["assets/logo.png"];
        assert_eq!(asset.width, 8);
        assert_eq!(asset.height, 8);
    }

    #[test]
    fn add_asset_png_invalid_bytes_errors() {
        let mut skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
        let err = skin.add_asset_png("assets/bad.png", b"nope").unwrap_err();
        assert!(format!("{err}").contains("assets/bad.png"));
    }

    #[test]
    fn from_directory_loads_assets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("skin.toml"), MANIFEST).unwrap();
        std::fs::write(dir.path().join("layout.toml"), LAYOUT).unwrap();
        std::fs::write(dir.path().join("features.toml"), FEATURES).unwrap();
        let assets = dir.path().join("assets");
        std::fs::create_dir(&assets).unwrap();
        std::fs::write(assets.join("logo.png"), png_bytes()).unwrap();
        std::fs::write(assets.join("broken.png"), b"not a png").unwrap();
        std::fs::write(assets.join("readme.txt"), b"ignored").unwrap();

        let skin = Skin::from_directory(dir.path()).unwrap();
        assert!(skin.assets.contains_key("assets/logo.png"));
        assert!(!skin.assets.contains_key("assets/readme.txt"));
        // Broken PNG is skipped with a warning, not a hard failure.
        assert!(!skin.assets.contains_key("assets/broken.png"));
        assert!(
            skin.schema_warnings
                .iter()
                .any(|w| w.contains("assets/broken.png")),
            "{:?}",
            skin.schema_warnings
        );
    }

    #[test]
    fn layout_texture_field_parses() {
        let layout = r#"
[bar_top]
x = 0
y = 0
w = 480
h = 24
texture = "assets/bar_top.png"
"#;
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        assert_eq!(
            skin.layout.objects["bar_top"].texture.as_deref(),
            Some("assets/bar_top.png")
        );
    }

    #[test]
    fn upload_layout_textures_attaches_and_sizes() {
        let layout = r#"
[logo]
x = 10
y = 20
texture = "assets/logo.png"

[bar]
x = 0
y = 0
w = 480
h = 24
texture = "assets/logo.png"

[plain]
x = 0
y = 0
w = 10
h = 10
"#;
        let mut skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        skin.add_asset_png("assets/logo.png", &png_bytes()).unwrap();
        let mut sdi = SdiRegistry::new();
        skin.apply_layout(&mut sdi);
        let mut backend = oasis_test_backend::MockSdiCore::new(480, 272);
        let ids = skin.upload_layout_textures(&mut sdi, &mut backend);
        assert_eq!(ids.len(), 2);
        // No explicit w/h -> asset native dims.
        let logo = sdi.get("logo").unwrap();
        assert!(logo.texture.is_some());
        assert_eq!((logo.w, logo.h), (8, 8));
        // Explicit w/h wins over asset dims.
        let bar = sdi.get("bar").unwrap();
        assert!(bar.texture.is_some());
        assert_eq!((bar.w, bar.h), (480, 24));
        assert!(sdi.get("plain").unwrap().texture.is_none());
    }

    #[test]
    fn upload_layout_nine_patch_attaches_slices() {
        let layout = r#"
[panel]
x = 0
y = 0
w = 200
h = 60
nine_patch = { image = "assets/logo.png", insets = [2, 3, 2, 3] }
"#;
        let mut skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        skin.add_asset_png("assets/logo.png", &png_bytes()).unwrap();
        let mut sdi = SdiRegistry::new();
        skin.apply_layout(&mut sdi);
        let mut backend = oasis_test_backend::MockSdiCore::new(480, 272);
        let ids = skin.upload_layout_textures(&mut sdi, &mut backend);
        assert_eq!(ids.len(), 1);
        let panel = sdi.get("panel").unwrap();
        assert!(panel.texture.is_some());
        let slices = panel.nine_patch.expect("nine_patch slices attached");
        assert_eq!((slices.tex_width, slices.tex_height), (8, 8));
        assert_eq!(
            (slices.left, slices.top, slices.right, slices.bottom),
            (2, 3, 2, 3)
        );
        assert_eq!((panel.w, panel.h), (200, 60));
    }

    #[test]
    fn validate_nine_patch_missing_asset_and_bad_insets() {
        let layout = r#"
[panel]
x = 0
y = 0
w = 100
h = 40
nine_patch = { image = "assets/missing.png", insets = [4, 4, 4, 4] }

[chunky]
x = 0
y = 0
w = 100
h = 40
nine_patch = { image = "assets/logo.png", insets = [5, 5, 5, 5] }
"#;
        let mut skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        // logo.png is 8x8, so 5+5 insets cannot fit.
        skin.add_asset_png("assets/logo.png", &png_bytes()).unwrap();
        let warnings = skin.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("panel") && w.contains("missing asset")),
            "missing nine_patch asset warning: {warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("chunky") && w.contains("don't fit")),
            "missing insets warning: {warnings:?}"
        );
    }

    #[test]
    fn upload_layout_textures_missing_asset_skipped() {
        let layout = r#"
[ghost]
x = 0
y = 0
texture = "assets/missing.png"
"#;
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        let mut sdi = SdiRegistry::new();
        skin.apply_layout(&mut sdi);
        let mut backend = oasis_test_backend::MockSdiCore::new(480, 272);
        let ids = skin.upload_layout_textures(&mut sdi, &mut backend);
        assert!(ids.is_empty());
        assert!(sdi.get("ghost").unwrap().texture.is_none());
    }

    #[test]
    fn validate_missing_texture_asset() {
        let layout = r#"
[ghost]
x = 0
y = 0
texture = "assets/missing.png"
"#;
        let skin = Skin::from_toml(MANIFEST, layout, FEATURES).unwrap();
        let warnings = skin.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("ghost") && w.contains("assets/missing.png")),
            "{warnings:?}"
        );
    }

    #[test]
    fn validate_image_wallpaper_sources() {
        let theme = r#"
[wallpaper]
style = "image"
fit = "sideways"
"#;
        let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
        let warnings = skin.validate();
        assert!(
            warnings.iter().any(|w| w.contains("no source set")),
            "{warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("unknown fit")),
            "{warnings:?}"
        );
    }

    #[test]
    fn validate_image_layer_sources() {
        let theme = r#"
[[background_layers]]
kind = "image"
source = "assets/nope.png"

[[background_layers]]
kind = "image"
"#;
        let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
        let warnings = skin.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("background_layers[0]") && w.contains("assets/nope.png")),
            "{warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("background_layers[1]") && w.contains("no source set")),
            "{warnings:?}"
        );
    }

    #[test]
    fn validate_non_pot_asset_warns() {
        let mut skin = Skin::from_toml(MANIFEST, LAYOUT, FEATURES).unwrap();
        let bytes = crate::assets::tests::encode_png(10, 8, png::ColorType::Rgba);
        skin.add_asset_png("assets/odd.png", &bytes).unwrap();
        let warnings = skin.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("assets/odd.png") && w.contains("power-of-two")),
            "{warnings:?}"
        );
    }

    #[test]
    fn image_layer_theme_derivation() {
        let theme = r#"
[[background_layers]]
kind = "image"
source = "assets/logo.png"
alpha = 128

[background_layers.position]
anchor = "bottom_right"
offset_x = -0.05

[background_layers.animation]
pulse_speed = 0.5

[[background_layers]]
kind = "grid"
"#;
        let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
        let at = crate::ActiveTheme::from_skin(&skin.theme);
        // Image layers are split out; the grid stays a vector layer.
        assert_eq!(at.image_layers.len(), 1);
        assert_eq!(at.background_layers.len(), 1);
        let layer = &at.image_layers[0];
        assert_eq!(layer.source, "assets/logo.png");
        assert_eq!(layer.alpha, 128);
        assert!((layer.animation.pulse_speed - 0.5).abs() < f32::EPSILON);
        assert!(matches!(
            layer.position.anchor,
            oasis_vector::background::Anchor::BottomRight
        ));
    }

    #[test]
    fn wallpaper_image_theme_derivation() {
        let theme = r#"
[wallpaper]
style = "image"
source = "assets/wall.png"
fit = "tile"
"#;
        let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme, "").unwrap();
        let at = crate::ActiveTheme::from_skin(&skin.theme);
        assert_eq!(at.wallpaper.style, "image");
        assert_eq!(at.wallpaper.source.as_deref(), Some("assets/wall.png"));
        assert_eq!(at.wallpaper.fit, "tile");
    }

    #[test]
    fn validate_optional_theme_color() {
        let theme_toml = "surface = \"bad\"\naccent_hover = \"#FF0000\"\n";
        let skin = Skin::from_toml_full(MANIFEST, LAYOUT, FEATURES, theme_toml, "").unwrap();
        let warnings = skin.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("surface") && w.contains("bad")),
            "missing invalid optional color warning: {warnings:?}"
        );
        assert!(
            !warnings.iter().any(|w| w.contains("accent_hover")),
            "unexpected accent_hover warning: {warnings:?}"
        );
    }
}
