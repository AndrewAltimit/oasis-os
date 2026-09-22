//! Custom skin loading example.
//!
//! Shows how to resolve a skin by built-in name or directory path, derive
//! the runtime [`ActiveTheme`], and parse a skin from inline TOML strings.
//! A skin directory contains at least `skin.toml`, `layout.toml`, and
//! `features.toml` (optionally `theme.toml`, `strings.toml`, `assets/`).
//!
//! ```bash
//! cargo run -p oasis-skin --example custom_skin                 # built-in "classic"
//! cargo run -p oasis-skin --example custom_skin -- xp           # another built-in
//! cargo run -p oasis-skin --example custom_skin -- skins/paper  # external TOML skin
//! ```

use oasis_skin::builtin::builtin_names;
use oasis_skin::{ActiveTheme, Skin, resolve_skin};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let skin_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "classic".to_string());

    println!("Built-in skins: {}", builtin_names().join(", "));
    println!("Loading skin:   {skin_name}");

    // resolve_skin() tries, in order:
    // 1. A built-in name ("classic", "modern", "xp", ...)
    // 2. A directory path containing skin.toml
    // 3. ./skins/{name}/
    // 4. Falls back to the built-in "classic" skin (with a log warning)
    let skin = resolve_skin(&skin_name)?;

    println!("Skin name:      {}", skin.manifest.name);
    println!("Skin version:   {}", skin.manifest.version);
    println!(
        "Resolution:     {}x{}",
        skin.manifest.screen_width, skin.manifest.screen_height
    );
    println!(
        "Features:       dashboard={} terminal={} window_manager={}",
        skin.features.dashboard, skin.features.terminal, skin.features.window_manager
    );
    for warning in skin.validate() {
        println!("Validation:     {warning}");
    }

    // Derive the runtime theme (colors for bars, icons, terminal, ...) from
    // the skin's base colors.
    let theme = ActiveTheme::from_skin(&skin.theme)
        .with_screen_size(skin.manifest.screen_width, skin.manifest.screen_height)
        .with_features(&skin.features);
    let bg = theme.clear_color;
    println!(
        "Clear color:    rgba({}, {}, {}, {})",
        bg.r, bg.g, bg.b, bg.a
    );

    // Skins can also be parsed from inline TOML strings (useful for skins
    // embedded in a binary).
    let inline_skin = Skin::from_toml(
        include_str!("../skins/classic/skin.toml"),
        include_str!("../skins/classic/layout.toml"),
        include_str!("../skins/classic/features.toml"),
    )?;
    println!(
        "Inline skin:    {} v{}",
        inline_skin.manifest.name, inline_skin.manifest.version
    );
    Ok(())
}
