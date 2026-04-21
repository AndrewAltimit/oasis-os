//! Classic and retro built-in skins.
//!
//! Skins: classic, retro-cga, win95.

use oasis_types::error::Result;

use crate::loader::Skin;

// ---------------------------------------------------------------------------
// Classic skin: loaded from external TOML files via include_str!.
// ---------------------------------------------------------------------------

const CLASSIC_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [classic]",
    "Initializing subsystems...",
    "VFS: mounted",
    "Network: standby",
    "Ready.",
]
prompt_format = "> "
title = "OASIS_OS"
home_label = "Home"
welcome_message = "Welcome to OASIS_OS. Type 'help' for commands."
error_prefix = "error: "
shutdown_message = "Session ended."
"#;

// ---------------------------------------------------------------------------
// Retro CGA skin: 4-color CGA palette, blocky, no frills.
// ---------------------------------------------------------------------------

const RETRO_CGA_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [CGA]",
    "VIDEO MODE: 320x200 4-COLOR",
    "LOADING...",
    "READY.",
]
prompt_format = "A> "
title = "OASIS CGA"
home_label = "HOME"
welcome_message = "TYPE 'HELP' FOR COMMANDS."
error_prefix = "ERR: "
shutdown_message = "SYSTEM HALTED."
"#;

// ---------------------------------------------------------------------------
// Win95 skin: Windows 95/98 classic look with raised 3D borders.
// ---------------------------------------------------------------------------

const WIN95_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [win95]",
    "Loading Windows 95 interface...",
    "Ready.",
]
prompt_format = "C:\\> "
title = "OASIS Win95"
home_label = "Start"
welcome_message = "Welcome to OASIS Win95. Type 'help' for commands."
error_prefix = "Error: "
shutdown_message = "It is now safe to turn off your computer."
"#;

/// Load the Classic skin.
pub fn classic_skin() -> Result<Skin> {
    Skin::from_toml_full(
        include_str!("../../../../skins/classic/skin.toml"),
        include_str!("../../../../skins/classic/layout.toml"),
        include_str!("../../../../skins/classic/features.toml"),
        include_str!("../../../../skins/classic/theme.toml"),
        CLASSIC_STRINGS,
    )
}

/// Load the Retro CGA skin.
pub fn retro_cga_skin() -> Result<Skin> {
    Skin::from_toml_full(
        include_str!("../../../../skins/retro-cga/skin.toml"),
        include_str!("../../../../skins/retro-cga/layout.toml"),
        include_str!("../../../../skins/retro-cga/features.toml"),
        include_str!("../../../../skins/retro-cga/theme.toml"),
        RETRO_CGA_STRINGS,
    )
}

/// Load the Win95 skin.
pub fn win95_skin() -> Result<Skin> {
    Skin::from_toml_full(
        include_str!("../../../../skins/win95/skin.toml"),
        include_str!("../../../../skins/win95/layout.toml"),
        include_str!("../../../../skins/win95/features.toml"),
        include_str!("../../../../skins/win95/theme.toml"),
        WIN95_STRINGS,
    )
}
