//! Skin serialization -- writing skins back to TOML.
//!
//! Compiled only with the default-on `serialize` feature. Provides:
//!
//! * [`SkinTheme::to_toml_string`] -- serialize a theme (`theme.toml` body).
//! * [`Skin::to_toml_parts`] -- serialize a full skin as the per-file TOML
//!   strings used by the `skins/<name>/` directory format.
//! * [`Skin::to_toml_string`] / [`Skin::from_toml_string`] -- single-document
//!   round-trippable form (`[manifest]` / `[features]` / `[theme]` /
//!   `[strings]` / `[layout]` tables) for exporting/inspecting a skin as one
//!   file.
//! * [`Skin::save_to_directory`] -- write the directory format to disk so the
//!   result is loadable by `resolve_skin` / `Skin::from_directory`.
//!
//! Output is minimal: unset optional fields are skipped entirely
//! (`skip_serializing_if = "Option::is_none"` on every `Option` field), so a
//! serialized skin only contains what it actually overrides.

use serde::{Deserialize, Serialize};

use oasis_types::error::{OasisError, Result};

use crate::corrupted::CorruptedModifiers;
use crate::loader::{Skin, SkinFeatures, SkinLayout, SkinManifest};
use crate::strings::SkinStrings;
use crate::theme::SkinTheme;

/// A full skin serialized as the per-file TOML strings of the
/// `skins/<name>/` directory format.
#[derive(Debug, Clone)]
pub struct SkinTomlParts {
    /// `skin.toml` content (manifest).
    pub manifest: String,
    /// `layout.toml` content.
    pub layout: String,
    /// `features.toml` content.
    pub features: String,
    /// `theme.toml` content.
    pub theme: String,
    /// `strings.toml` content.
    pub strings: String,
    /// `corrupted.toml` content, if the skin carries corrupted modifiers.
    pub corrupted: Option<String>,
}

/// Single-document form of a skin (used by `to_toml_string` /
/// `from_toml_string`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CombinedSkinDoc {
    manifest: SkinManifest,
    features: SkinFeatures,
    theme: SkinTheme,
    strings: SkinStrings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    corrupted: Option<CorruptedModifiers>,
    layout: SkinLayout,
}

fn ser<T: Serialize>(what: &str, value: &T) -> Result<String> {
    toml::to_string(value).map_err(|e| OasisError::Config(format!("serialize {what}: {e}").into()))
}

impl SkinTheme {
    /// Serialize this theme to a TOML string (the body of a `theme.toml`).
    ///
    /// Unset optional fields are omitted, so the output is minimal and
    /// re-loadable via `toml::from_str::<SkinTheme>`.
    pub fn to_toml_string(&self) -> Result<String> {
        ser("theme", self)
    }
}

impl Skin {
    /// Serialize this skin as the per-file TOML strings of the
    /// `skins/<name>/` directory format.
    pub fn to_toml_parts(&self) -> Result<SkinTomlParts> {
        Ok(SkinTomlParts {
            manifest: ser("skin.toml", &self.manifest)?,
            layout: ser("layout.toml", &self.layout)?,
            features: ser("features.toml", &self.features)?,
            theme: ser("theme.toml", &self.theme)?,
            strings: ser("strings.toml", &self.strings)?,
            corrupted: match self.corrupted_modifiers {
                Some(ref m) => Some(ser("corrupted.toml", m)?),
                None => None,
            },
        })
    }

    /// Serialize this skin as a single TOML document with `[manifest]`,
    /// `[features]`, `[theme]`, `[strings]`, and `[layout]` tables
    /// (plus `[corrupted]` when present).
    ///
    /// The result is re-loadable via [`Skin::from_toml_string`].
    pub fn to_toml_string(&self) -> Result<String> {
        let doc = CombinedSkinDoc {
            manifest: self.manifest.clone(),
            features: self.features.clone(),
            theme: self.theme.clone(),
            strings: self.strings.clone(),
            corrupted: self.corrupted_modifiers.clone(),
            layout: self.layout.clone(),
        };
        ser("skin", &doc)
    }

    /// Parse a skin from the single-document form produced by
    /// [`Skin::to_toml_string`].
    ///
    /// The combined form carries configuration only -- binary assets
    /// (`assets/*.png`, fonts) are not embedded and come back empty.
    pub fn from_toml_string(toml_str: &str) -> Result<Self> {
        let doc: CombinedSkinDoc = toml::from_str(toml_str)
            .map_err(|e| OasisError::Config(format!("skin document: {e}").into()))?;
        let corrupted_modifiers = doc
            .corrupted
            .or_else(|| doc.features.corrupted.then(CorruptedModifiers::default));
        Ok(Self {
            manifest: doc.manifest,
            layout: doc.layout,
            features: doc.features,
            theme: doc.theme,
            strings: doc.strings,
            corrupted_modifiers,
            schema_warnings: Vec::new(),
            assets: std::collections::HashMap::new(),
            font_assets: std::collections::HashMap::new(),
            sound_assets: std::collections::HashMap::new(),
        })
    }

    /// Write this skin to `dir` in the directory format understood by
    /// [`Skin::from_directory`] / `resolve_skin` (`skin.toml`,
    /// `layout.toml`, `features.toml`, `theme.toml`, `strings.toml`,
    /// optionally `corrupted.toml`, plus any image/font assets re-encoded
    /// under their skin-relative paths). Creates `dir` if needed.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_to_directory(&self, dir: &std::path::Path) -> Result<()> {
        let parts = self.to_toml_parts()?;
        std::fs::create_dir_all(dir)
            .map_err(|e| OasisError::Config(format!("{}: {e}", dir.display()).into()))?;
        let write = |name: &str, content: &[u8]| -> Result<()> {
            // Asset names are skin-relative (e.g. "assets/bar_top.png");
            // refuse anything that could escape the target directory.
            if name.split('/').any(|seg| seg == "..") || name.starts_with('/') {
                return Err(OasisError::Config(
                    format!("unsafe asset path '{name}'").into(),
                ));
            }
            let path = dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| OasisError::Config(format!("{}: {e}", parent.display()).into()))?;
            }
            std::fs::write(&path, content)
                .map_err(|e| OasisError::Config(format!("{}: {e}", path.display()).into()))
        };
        write("skin.toml", parts.manifest.as_bytes())?;
        write("layout.toml", parts.layout.as_bytes())?;
        write("features.toml", parts.features.as_bytes())?;
        write("theme.toml", parts.theme.as_bytes())?;
        write("strings.toml", parts.strings.as_bytes())?;
        if let Some(ref corrupted) = parts.corrupted {
            write("corrupted.toml", corrupted.as_bytes())?;
        }
        // Re-encode image assets and copy font bytes so an asset-carrying
        // source skin keeps its chrome when reloaded from the new directory.
        for (name, asset) in &self.assets {
            write(name, &encode_png(asset)?)?;
        }
        for (name, bytes) in &self.font_assets {
            write(name, bytes)?;
        }
        for (name, bytes) in &self.sound_assets {
            write(name, bytes)?;
        }
        Ok(())
    }
}

/// Encode a decoded RGBA8 asset back to PNG bytes.
#[cfg(not(target_arch = "wasm32"))]
fn encode_png(asset: &crate::assets::SkinAsset) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, asset.width, asset.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| OasisError::Config(format!("png encode: {e}").into()))?;
        writer
            .write_image_data(&asset.rgba)
            .map_err(|e| OasisError::Config(format!("png encode: {e}").into()))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fmt::Write as _;

    use super::*;
    use crate::active_theme::ActiveTheme;
    use crate::builtin;

    /// Stable fingerprint of an `ActiveTheme` for equality comparison.
    ///
    /// `ActiveTheme` holds `HashMap` fields whose `Debug` iteration order is
    /// nondeterministic, so those are folded through sorted `BTreeMap`s while
    /// the plain sub-structs are compared via their `Debug` output.
    fn theme_fingerprint(t: &ActiveTheme) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            t.bar, t.icon, t.menu, t.app, t.osk, t.scrollbar, t.wallpaper, t.toast
        );
        let _ = write!(
            out,
            "|{}|{}|{}|{}|{}|{}|{}",
            normalize_shader_floats(&format!("{:?}", t.background_layers)),
            t.statusbar_height,
            t.bottombar_height,
            t.taskbar_height,
            t.icon_width,
            t.icon_height,
            t.font_small
        );
        let app_themes: BTreeMap<_, BTreeMap<_, _>> = t
            .app_themes
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.iter().map(|(k2, c)| (k2.clone(), *c)).collect(),
                )
            })
            .collect();
        let gradients: BTreeMap<_, _> = t.gradients.iter().collect();
        let animations: BTreeMap<_, _> = t.animations.iter().collect();
        let _ = write!(out, "|{app_themes:?}|{gradients:?}|{animations:?}");
        out
    }

    /// Sort the entries of every `floats: {...}` map in a Debug string.
    ///
    /// Shader background layers carry a `HashMap<String, f32>` of parameters
    /// whose Debug iteration order is nondeterministic; the content itself is
    /// what must round-trip.
    fn normalize_shader_floats(s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        const NEEDLE: &str = "floats: {";
        while let Some(idx) = rest.find(NEEDLE) {
            let split_at = idx + NEEDLE.len();
            out.push_str(&rest[..split_at]);
            rest = &rest[split_at..];
            if let Some(end) = rest.find('}') {
                let mut items: Vec<&str> = rest[..end].split(", ").collect();
                items.sort_unstable();
                out.push_str(&items.join(", "));
                rest = &rest[end..];
            }
        }
        out.push_str(rest);
        out
    }

    /// Stable fingerprint of a skin layout (sorted by object name).
    fn layout_fingerprint(layout: &SkinLayout) -> String {
        let sorted: BTreeMap<_, _> = layout
            .objects
            .iter()
            .map(|(k, v)| (k.clone(), format!("{v:?}")))
            .collect();
        format!("{sorted:?}")
    }

    #[test]
    fn every_builtin_skin_round_trips() {
        for name in builtin::builtin_names() {
            let original = builtin::load_builtin(name).expect("builtin loads");
            let parts = original.to_toml_parts().expect("serializes");
            let reloaded = match parts.corrupted {
                Some(ref corrupted) => Skin::from_toml_corrupted(
                    &parts.manifest,
                    &parts.layout,
                    &parts.features,
                    &parts.theme,
                    &parts.strings,
                    corrupted,
                ),
                None => Skin::from_toml_full(
                    &parts.manifest,
                    &parts.layout,
                    &parts.features,
                    &parts.theme,
                    &parts.strings,
                ),
            }
            .unwrap_or_else(|e| panic!("skin '{name}' failed to re-parse: {e}"));

            assert!(
                reloaded.schema_warnings.is_empty(),
                "skin '{name}': serialized output contains unknown keys: {:?}",
                reloaded.schema_warnings
            );
            assert_eq!(
                format!("{:?}", original.manifest),
                format!("{:?}", reloaded.manifest),
                "skin '{name}': manifest drifted through serialization"
            );
            assert_eq!(
                format!("{:?}", original.features),
                format!("{:?}", reloaded.features),
                "skin '{name}': features drifted through serialization"
            );
            assert_eq!(
                format!("{:?}", original.strings),
                format!("{:?}", reloaded.strings),
                "skin '{name}': strings drifted through serialization"
            );
            assert_eq!(
                layout_fingerprint(&original.layout),
                layout_fingerprint(&reloaded.layout),
                "skin '{name}': layout drifted through serialization"
            );
            assert_eq!(
                theme_fingerprint(&ActiveTheme::from_skin(&original.theme)),
                theme_fingerprint(&ActiveTheme::from_skin(&reloaded.theme)),
                "skin '{name}': derived ActiveTheme drifted through serialization"
            );
        }
    }

    #[test]
    fn combined_document_round_trips() {
        let original = builtin::load_builtin("classic").expect("classic loads");
        let doc = original.to_toml_string().expect("serializes");
        let reloaded = Skin::from_toml_string(&doc).expect("re-parses");
        assert_eq!(original.manifest.name, reloaded.manifest.name);
        assert_eq!(
            layout_fingerprint(&original.layout),
            layout_fingerprint(&reloaded.layout)
        );
        assert_eq!(
            theme_fingerprint(&ActiveTheme::from_skin(&original.theme)),
            theme_fingerprint(&ActiveTheme::from_skin(&reloaded.theme))
        );
    }

    #[test]
    fn combined_document_preserves_corrupted_modifiers() {
        let original = builtin::load_builtin("corrupted").expect("corrupted loads");
        assert!(original.corrupted_modifiers.is_some());
        let doc = original.to_toml_string().expect("serializes");
        let reloaded = Skin::from_toml_string(&doc).expect("re-parses");
        assert_eq!(
            format!("{:?}", original.corrupted_modifiers),
            format!("{:?}", reloaded.corrupted_modifiers)
        );
    }

    #[test]
    fn customized_theme_round_trips() {
        let mut skin = builtin::load_builtin("classic").expect("classic loads");
        skin.theme.background = "#102030".to_string();
        skin.theme.primary = "#FF8800".to_string();
        let toml_str = skin.theme.to_toml_string().expect("serializes");
        let reloaded: SkinTheme = toml::from_str(&toml_str).expect("re-parses");
        assert_eq!(reloaded.background, "#102030");
        assert_eq!(reloaded.primary, "#FF8800");
        assert_eq!(
            theme_fingerprint(&ActiveTheme::from_skin(&skin.theme)),
            theme_fingerprint(&ActiveTheme::from_skin(&reloaded))
        );
    }

    #[test]
    fn minimal_theme_serialization_skips_unset_fields() {
        let theme = SkinTheme::default();
        let toml_str = theme.to_toml_string().expect("serializes");
        // The 9 base colors are present...
        assert!(toml_str.contains("background"));
        assert!(toml_str.contains("primary"));
        // ...but no unset optional sections leak into the output.
        assert!(!toml_str.contains("wm_theme"));
        assert!(!toml_str.contains("bar_overrides"));
        assert!(!toml_str.contains("surface"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn save_to_directory_is_loadable() {
        let dir =
            std::env::temp_dir().join(format!("oasis-skin-serialize-test-{}", std::process::id()));
        let skin = builtin::load_builtin("classic").expect("classic loads");
        skin.save_to_directory(&dir).expect("saves");
        let reloaded = Skin::from_directory(&dir).expect("directory loads");
        assert_eq!(skin.manifest.name, reloaded.manifest.name);
        assert_eq!(
            theme_fingerprint(&ActiveTheme::from_skin(&skin.theme)),
            theme_fingerprint(&ActiveTheme::from_skin(&reloaded.theme))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
