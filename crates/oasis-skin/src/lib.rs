//! Skin system -- data-driven configuration of visual and behavioral personality.
//!
//! A skin is a TOML manifest referencing layout definitions, theme colors,
//! feature flags, strings, and optional corrupted modifiers. The core
//! framework interprets skins at runtime. Skins can be hot-swapped.

pub mod active_theme;
pub mod builtin;
pub mod corrupted;
pub mod effects;
pub mod legacy_theme;
mod loader;
pub mod strings;
pub mod theme;

pub use active_theme::ActiveTheme;
pub use corrupted::{CorruptedModifiers, SimpleRng};
pub use effects::{CorruptedEffect, ScanlineEffect, SkinEffect};
pub use loader::{Skin, SkinFeatures, SkinLayout, SkinManifest, SkinObjectDef};
pub use strings::SkinStrings;
pub use theme::{BarOverrides, BrowserOverrides, IconOverrides, SkinTheme, WmThemeOverrides};

use std::path::Path;

use oasis_types::error::Result;

/// Resolve a skin by name or path.
///
/// Resolution order:
/// 1. Built-in name match (e.g. "terminal", "modern")
/// 2. Path containing `skin.toml` (e.g. "skins/classic")
/// 3. Subdirectory under `./skins/{name}/`
/// 4. Fallback to "classic" built-in skin with a warning
pub fn resolve_skin(name_or_path: &str) -> Result<Skin> {
    // 1. Try built-in name.
    if let Ok(skin) = builtin::load_builtin(name_or_path) {
        return Ok(skin);
    }

    // 2. Try as a directory path.
    let path = Path::new(name_or_path);
    if path.join("skin.toml").is_file() {
        return Skin::from_directory(path);
    }

    // 3. Try ./skins/{name}/.
    let skins_dir = Path::new("skins").join(name_or_path);
    if skins_dir.join("skin.toml").is_file() {
        return Skin::from_directory(&skins_dir);
    }

    // 4. Fallback to classic embedded skin.
    log::warn!("Skin '{name_or_path}' not found -- falling back to classic");
    Skin::from_toml(
        include_str!("../../../skins/classic/skin.toml"),
        include_str!("../../../skins/classic/layout.toml"),
        include_str!("../../../skins/classic/features.toml"),
    )
}
