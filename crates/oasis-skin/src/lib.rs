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
pub use loader::{Skin, SkinFeatures, SkinLayout, SkinManifest, SkinObjectDef};
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

use oasis_types::error::Result;

/// Resolve a skin by name or path.
///
/// Resolution order:
/// 1. Built-in name match (e.g. "classic", "modern")
/// 2. Path containing `skin.toml` (e.g. "skins/classic")
/// 3. Subdirectory under `./skins/{name}/`
/// 4. Fallback to "classic" built-in skin with a warning
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_skin(name_or_path: &str) -> Result<Skin> {
    // 1. Try built-in name.
    if let Ok(skin) = builtin::load_builtin(name_or_path) {
        return Ok(skin);
    }

    // 2. Try as a directory path.
    let path = Path::new(name_or_path);
    if path.join("skin.toml").is_file() {
        let skin = Skin::from_directory(path)?;
        log_validation_warnings(name_or_path, &skin);
        return Ok(skin);
    }

    // 3. Try ./skins/{name}/.
    let skins_dir = Path::new("skins").join(name_or_path);
    if skins_dir.join("skin.toml").is_file() {
        let skin = Skin::from_directory(&skins_dir)?;
        log_validation_warnings(name_or_path, &skin);
        return Ok(skin);
    }

    // 4. Fallback to classic built-in skin.
    log::warn!("Skin '{name_or_path}' not found -- falling back to classic");
    builtin::classic_skin()
}

/// Resolve a skin by name (WASM -- no filesystem, built-in skins only).
///
/// Tries built-in name match, then falls back to "classic".
#[cfg(target_arch = "wasm32")]
pub fn resolve_skin(name_or_path: &str) -> Result<Skin> {
    // 1. Try built-in name.
    if let Ok(skin) = builtin::load_builtin(name_or_path) {
        return Ok(skin);
    }

    // 2. Fallback to classic built-in skin.
    log::warn!("Skin '{name_or_path}' not found -- falling back to classic");
    builtin::classic_skin()
}

/// Log any validation warnings for an external skin.
fn log_validation_warnings(name: &str, skin: &Skin) {
    let warnings = skin.validate();
    for w in &warnings {
        log::warn!("Skin '{name}' validation: {w}");
    }
}
