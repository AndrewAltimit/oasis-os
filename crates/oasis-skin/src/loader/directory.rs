//! Loading skins from `skins/<name>/` directories on disk (not on WASM).

use std::path::{Path, PathBuf};

use oasis_types::error::{OasisError, Result};

use super::{Skin, SkinSources};

impl Skin {
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

        let mut skin = Self::from_sources(&SkinSources {
            manifest: &manifest,
            layout: &layout,
            features: &features,
            theme: &theme,
            strings: &strings,
            corrupted: Some(&corrupted),
        })
        // Name the directory: "theme.toml: TOML parse error at line 3" alone
        // does not say which skin is broken.
        .map_err(|e| OasisError::Config(format!("{}: {e}", dir.display()).into()))?;

        // Load image assets from the skin's assets/ subdirectory.
        skin.load_assets_from(&dir.join("assets"));

        // Apply inheritance from a built-in parent skin if specified. An
        // unknown parent loads the child alone, so tell the author.
        if let Some(parent_name) = skin.manifest.inherits.clone() {
            match crate::builtin::load_builtin(&parent_name) {
                Ok(parent) => skin.merge_theme_from(&parent),
                Err(e) => skin.schema_warnings.push(format!(
                    "skin.toml: inherits = \"{parent_name}\" not loaded ({e})"
                )),
            }
        }

        for warning in &skin.schema_warnings {
            log::warn!("skin '{}': {warning}", skin.manifest.name);
        }

        Ok(skin)
    }

    /// Decode every `*.png` (images) and `*.wav` (UI sounds) in an assets
    /// directory into `self.assets` / `self.sound_assets`, and slurp every
    /// `*.ttf`/`*.otf` into `self.font_assets`, keyed `"assets/<file>"`.
    /// Files that fail to decode are recorded as schema warnings instead of
    /// failing the whole skin load.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_assets_from(&mut self, assets_dir: &Path) {
        let Ok(entries) = std::fs::read_dir(assets_dir) else {
            return; // No assets/ directory -- nothing to do.
        };
        let ext_is = |p: &Path, wanted: &str| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(wanted))
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                ext_is(p, "png") || ext_is(p, "ttf") || ext_is(p, "otf") || ext_is(p, "wav")
            })
            .collect();
        files.sort();
        for path in files {
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let key = format!("assets/{file_name}");
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let result = if ext_is(&path, "png") {
                        self.add_asset_png(&key, &bytes)
                    } else if ext_is(&path, "wav") {
                        self.add_asset_wav(&key, &bytes)
                    } else {
                        self.add_asset_font(&key, bytes);
                        Ok(())
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
}
