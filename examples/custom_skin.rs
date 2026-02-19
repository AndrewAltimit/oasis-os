//! Custom skin loading example.
//!
//! Shows how to load a TOML skin from a directory and apply it.
//! Place your skin files in a directory with `skin.toml`, `layout.toml`,
//! and `features.toml`, then pass the directory path as a CLI argument.
//!
//! ```bash
//! cargo run --example custom_skin -- ./skins/classic
//! ```

use oasis_skin::{ActiveTheme, Skin, resolve_skin};

fn main() {
    // Get skin path from CLI argument or use default.
    let skin_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "classic".to_string());

    println!("Loading skin: {skin_name}");

    // resolve_skin() tries:
    // 1. Built-in name ("classic", "modern", "terminal", etc.)
    // 2. Directory path containing skin.toml
    // 3. ./skins/{name}/ subdirectory
    // 4. Fallback to "classic"
    let skin = resolve_skin(&skin_name).expect("Failed to load skin");

    // Print skin metadata.
    println!("Skin name:    {}", skin.manifest.name);
    println!("Skin version: {}", skin.manifest.version);

    // Create an active theme from the skin.
    let theme = ActiveTheme::from_skin(&skin.theme);
    let bg = theme.background_color();
    println!(
        "Background:   rgba({}, {}, {}, {})",
        bg.r, bg.g, bg.b, bg.a
    );

    // Load skin from inline TOML strings (useful for embedded skins).
    let inline_skin = Skin::from_toml(
        include_str!("../skins/classic/skin.toml"),
        include_str!("../skins/classic/layout.toml"),
        include_str!("../skins/classic/features.toml"),
    )
    .expect("Failed to parse inline skin");
    println!(
        "Inline skin loaded: {} v{}",
        inline_skin.manifest.name, inline_skin.manifest.version
    );
}
