//! Skin system -- data-driven configuration of visual and behavioral personality.
//!
//! A skin is a TOML manifest referencing layout definitions, theme colors,
//! feature flags, strings, and optional corrupted modifiers. The core
//! framework interprets skins at runtime. Skins can be hot-swapped.

pub mod active_theme;
pub mod assets;
pub mod builtin;
pub mod corrupted;
pub mod effects;
pub mod legacy_theme;
mod loader;
#[cfg(feature = "serialize")]
mod serialize;
pub mod strings;
pub mod theme;
mod variants;

pub use active_theme::{
    ActiveTheme, AnsiPalette, AppScreenTheme, BarTheme, IconTheme, ImageLayerTheme, OskTheme,
    ScrollbarTheme, StartMenuTheme, ToastTheme, WallpaperTheme,
};
pub use assets::SkinAsset;
pub use corrupted::{CorruptedModifiers, SimpleRng};
pub use effects::{CorruptedEffect, ScanlineEffect, SkinEffect};
pub use loader::{Skin, SkinFeatures, SkinLayout, SkinManifest, SkinObjectDef, SkinSources};
#[cfg(feature = "serialize")]
pub use serialize::SkinTomlParts;
pub use strings::SkinStrings;
pub use theme::{
    AppOverrides, BarOverrides, BootOverrides, BrowserOverrides, CursorConfig, IconOverrides,
    OskOverrides, PaletteOverrides, SkinTheme, WmThemeOverrides, parse_hex_color,
};
pub use variants::{SkinVariant, VARIANT_REQUEST_PREFIX, resolve_skin_request};

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use oasis_types::error::{OasisError, Result};

/// Look a skin up by name or path without any fallback.
///
/// Resolution order:
/// 1. Built-in name (e.g. `"classic"`, `"xp"`; see [`builtin::builtin_names`])
/// 2. A directory path containing `skin.toml` (e.g. `"skins/classic"`)
/// 3. A subdirectory of `./skins/` (e.g. `"my-skin"` -> `./skins/my-skin/`)
///
/// Steps 2 and 3 are skipped on WASM (no filesystem).
///
/// Returns `None` when nothing matches, and `Some(Err(..))` when a match was
/// found but failed to load (a built-in that fails to parse is reported, not
/// skipped). Use this when "not found" and "broken" need different handling
/// (e.g. `skin lint`); otherwise use [`resolve_skin`].
pub fn lookup_skin(name_or_path: &str) -> Option<Result<Skin>> {
    if builtin::is_builtin(name_or_path) {
        return Some(builtin::load_builtin(name_or_path));
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = Path::new(name_or_path);
        if path.join("skin.toml").is_file() {
            return Some(Skin::from_directory(path));
        }
        let skins_dir = Path::new("skins").join(name_or_path);
        if skins_dir.join("skin.toml").is_file() {
            return Some(Skin::from_directory(&skins_dir));
        }
    }

    None
}

/// Resolve a skin by name or path (see [`lookup_skin`] for the order).
///
/// Strict: an unknown name or a skin that fails to load is an error. For an
/// external (directory) skin, validation warnings are logged.
pub fn resolve_skin(name_or_path: &str) -> Result<Skin> {
    let skin = lookup_skin(name_or_path).unwrap_or_else(|| {
        Err(OasisError::Config(
            format!(
                "unknown skin '{name_or_path}' (not a built-in name, a directory with                  skin.toml, or a skins/ subdirectory)"
            )
            .into(),
        ))
    })?;
    if !builtin::is_builtin(name_or_path) {
        log_validation_warnings(name_or_path, &skin);
    }
    Ok(skin)
}

/// Resolve a skin like [`resolve_skin`], falling back to the built-in
/// `classic` skin (with a logged warning) when it cannot be resolved.
///
/// Meant for startup paths where showing *some* shell beats failing to boot
/// (e.g. a stale persisted skin name). Interactive paths (`skin <name>`,
/// `skin export`) should use the strict [`resolve_skin`] so a typo is
/// reported instead of silently switching to classic.
pub fn resolve_skin_or_classic(name_or_path: &str) -> Result<Skin> {
    resolve_skin(name_or_path).or_else(|e| {
        log::warn!("Skin '{name_or_path}' could not be loaded ({e}) -- falling back to classic");
        builtin::classic_skin()
    })
}

/// Log any validation warnings for an external skin.
fn log_validation_warnings(name: &str, skin: &Skin) {
    let warnings = skin.validate();
    for w in &warnings {
        log::warn!("Skin '{name}' validation: {w}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_skin_builtin_names() {
        for name in builtin::builtin_names() {
            let skin = resolve_skin(name).expect("built-in resolves");
            assert_eq!(skin.manifest.name, *name);
        }
    }

    #[test]
    fn resolve_skin_unknown_is_an_error() {
        let err = resolve_skin("no-such-skin-xyz").expect_err("unknown name must fail");
        assert!(err.to_string().contains("no-such-skin-xyz"), "{err}");
        assert!(lookup_skin("no-such-skin-xyz").is_none());
    }

    #[test]
    fn resolve_skin_or_classic_falls_back() {
        let skin = resolve_skin_or_classic("no-such-skin-xyz").expect("falls back");
        assert_eq!(skin.manifest.name, "classic");
        let xp = resolve_skin_or_classic("xp").expect("xp resolves");
        assert_eq!(xp.manifest.name, "xp");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn resolve_skin_directory_path() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../skins/paper");
        let skin = resolve_skin(dir).expect("directory resolves");
        assert_eq!(skin.manifest.name, "paper");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn lookup_skin_reports_broken_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("skin.toml"), "name = [broken").expect("write");
        let path = tmp.path().to_string_lossy().into_owned();
        assert!(matches!(lookup_skin(&path), Some(Err(_))));
        assert!(resolve_skin(&path).is_err());
    }

    /// `skins/<name>/` (directory form) and the built-in `<name>` are the
    /// same skin -- the built-ins are generated from those directories.
    #[cfg(all(not(target_arch = "wasm32"), feature = "serialize"))]
    #[test]
    fn builtin_and_directory_forms_are_identical() {
        for name in builtin::builtin_names() {
            let dir = format!("{}/../../skins/{name}", env!("CARGO_MANIFEST_DIR"));
            let from_dir = Skin::from_directory(Path::new(&dir)).expect("directory loads");
            let built_in = builtin::load_builtin(name).expect("built-in loads");
            assert_eq!(
                builtin::snapshot_dump(&from_dir),
                builtin::snapshot_dump(&built_in),
                "skins/{name} and built-in '{name}' differ"
            );
        }
    }
}
