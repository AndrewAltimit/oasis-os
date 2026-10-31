//! Skin loading from TOML configuration files.
//!
//! - `mod.rs` -- the [`Skin`] type, TOML parsing ([`Skin::from_sources`]),
//!   asset registration, layout application and swapping
//! - `manifest` -- [`SkinManifest`] (`skin.toml`) and [`SkinFeatures`]
//!   (`features.toml`)
//! - `parsing` -- [`SkinLayout`] / [`SkinObjectDef`] (`layout.toml`)
//! - `directory` -- [`Skin::from_directory`] / [`Skin::discover_skins`]
//! - `inherit` -- [`Skin::merge_theme_from`] (`inherits = "<parent>"`)
//! - `validation` -- [`Skin::validate`] (`skin lint`)

#[cfg(not(target_arch = "wasm32"))]
mod directory;
mod inherit;
mod manifest;
mod parsing;
mod validation;

use std::collections::HashMap;

use oasis_sdi::SdiRegistry;
use oasis_types::backend::{SdiBackend, TextureId};
use oasis_types::error::{OasisError, Result};

use super::assets::SkinAsset;
use super::corrupted::CorruptedModifiers;
use super::strings::SkinStrings;
use super::theme::SkinTheme;

pub use manifest::{SkinFeatures, SkinManifest};
pub use parsing::{SkinLayout, SkinObjectDef};

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
    /// Raw TTF/OTF font bytes keyed by skin-relative path
    /// (e.g. `"assets/skin.ttf"`) — the string `[typography] font` uses to
    /// reference them. Kept undecoded: parsing happens in the backend's
    /// rasterizer (and, feature-gated, in validation).
    pub font_assets: HashMap<String, Vec<u8>>,
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

/// The TOML text of one skin, one field per file of the `skins/<name>/`
/// directory format. Named fields, so the five same-typed documents cannot
/// be passed in the wrong order (unlike the positional
/// [`Skin::from_toml_full`]).
///
/// `manifest`, `layout` and `features` are required documents (an empty
/// string parses as an all-defaults table); an empty `theme` / `strings`
/// means "use the defaults", and `corrupted: None` (or `Some("")`) means "no
/// `corrupted.toml`".
#[derive(Debug, Clone, Copy, Default)]
pub struct SkinSources<'a> {
    /// `skin.toml` (manifest).
    pub manifest: &'a str,
    /// `layout.toml`.
    pub layout: &'a str,
    /// `features.toml`.
    pub features: &'a str,
    /// `theme.toml`.
    pub theme: &'a str,
    /// `strings.toml`.
    pub strings: &'a str,
    /// `corrupted.toml`, if the skin has one.
    pub corrupted: Option<&'a str>,
}

impl Skin {
    /// Load a skin from TOML strings (basic 3-file format for backwards compat).
    pub fn from_toml(manifest_toml: &str, layout_toml: &str, features_toml: &str) -> Result<Self> {
        Self::from_toml_full(manifest_toml, layout_toml, features_toml, "", "")
    }

    /// Load a skin from all TOML configuration strings (positional form of
    /// [`Skin::from_sources`]).
    pub fn from_toml_full(
        manifest_toml: &str,
        layout_toml: &str,
        features_toml: &str,
        theme_toml: &str,
        strings_toml: &str,
    ) -> Result<Self> {
        Self::from_sources(&SkinSources {
            manifest: manifest_toml,
            layout: layout_toml,
            features: features_toml,
            theme: theme_toml,
            strings: strings_toml,
            corrupted: None,
        })
    }

    /// Load a skin from its per-file TOML documents. This is the one parser
    /// behind [`Skin::from_directory`], the generated built-in loaders,
    /// [`Skin::from_toml_full`] and [`Skin::from_toml_corrupted`].
    pub fn from_sources(src: &SkinSources<'_>) -> Result<Self> {
        let SkinSources {
            manifest: manifest_toml,
            layout: layout_toml,
            features: features_toml,
            theme: theme_toml,
            strings: strings_toml,
            corrupted: corrupted_toml,
        } = *src;
        let mut schema_warnings = Vec::new();
        let manifest: SkinManifest =
            parse_toml_lint(manifest_toml, "skin.toml", &mut schema_warnings)?;
        let layout: SkinLayout = parse_toml_lint(layout_toml, "layout.toml", &mut schema_warnings)?;
        let mut features: SkinFeatures =
            parse_toml_lint(features_toml, "features.toml", &mut schema_warnings)?;

        let theme: SkinTheme = if theme_toml.is_empty() {
            SkinTheme::default()
        } else {
            let theme = parse_toml_lint(theme_toml, "theme.toml", &mut schema_warnings)?;
            validation::lint_color_strings(theme_toml, &mut schema_warnings);
            theme
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

        // An explicit corrupted.toml wins; otherwise `corrupted = true` in
        // features.toml gets the default modifiers.
        let corrupted_modifiers = match corrupted_toml.filter(|s| !s.is_empty()) {
            Some(text) => Some(parse_toml_lint::<CorruptedModifiers>(
                text,
                "corrupted.toml",
                &mut schema_warnings,
            )?),
            None => features.corrupted.then(CorruptedModifiers::default),
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
            font_assets: HashMap::new(),
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

    /// Register raw TTF/OTF bytes as a named font asset (e.g.
    /// `"assets/skin.ttf"`). Bytes are stored as-is; parse errors surface
    /// through `validate()` (feature `ttf`) and the backend rasterizer.
    pub fn add_asset_font(&mut self, name: &str, bytes: Vec<u8>) {
        self.font_assets.insert(name.to_string(), bytes);
    }

    /// The raw bytes of the font selected by `[typography] font`, if the
    /// skin sets one and the referenced asset was loaded. This is what the
    /// app hands to `SdiText::set_font` on skin swap.
    pub fn active_font_bytes(&self) -> Option<&[u8]> {
        let path = self.theme.typography.as_ref()?.font.as_ref()?;
        self.font_assets.get(path).map(Vec::as_slice)
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
        Self::from_sources(&SkinSources {
            manifest: manifest_toml,
            layout: layout_toml,
            features: features_toml,
            theme: theme_toml,
            strings: strings_toml,
            corrupted: Some(corrupted_toml),
        })
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
}

#[cfg(test)]
mod tests;
