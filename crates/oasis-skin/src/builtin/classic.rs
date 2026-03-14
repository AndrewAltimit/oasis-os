//! Classic and retro built-in skins.
//!
//! Skins: terminal, tactical, classic, retro-cga, win95.

use oasis_types::error::Result;

use crate::loader::Skin;

// ---------------------------------------------------------------------------
// Terminal skin: full-screen green-on-black command line.
// ---------------------------------------------------------------------------

const TERMINAL_MANIFEST: &str = r#"
name = "terminal"
version = "1.0"
author = "OASIS_OS"
description = "Full-screen command line terminal with CRT aesthetic"
screen_width = 480
screen_height = 272
"#;

const TERMINAL_LAYOUT: &str = r##"
[terminal_bg]
x = 0
y = 0
w = 480
h = 272
z = -1
color = "#000000"

[terminal_output]
x = 4
y = 4
w = 472
h = 252
color = "#00000000"
text = ""
font_size = 8
text_color = "#00CC00"

[terminal_prompt]
x = 4
y = 256
w = 472
h = 12
color = "#00000000"
text = "$> "
font_size = 8
text_color = "#00FF00"
"##;

const TERMINAL_FEATURES: &str = r#"
dashboard = false
terminal = true
file_browser = true
window_manager = false
show_tabs = false
"#;

const TERMINAL_THEME: &str = r##"
background = "#000000"
primary = "#00FF00"
secondary = "#003300"
text = "#00CC00"
dim_text = "#006600"
status_bar = "#001A00"
prompt = "#00FF00"
output = "#00CC00"
error = "#FF3333"

[geometry]
cursor_blink_rate = 20

[wallpaper]
style = "scanlines"

[[background_layers]]
kind = "scanlines"
spacing = 2
color = "#00FF0030"

[[background_layers]]
kind = "pulsing_core"
radius = 8
color = "#00FF0045"
[background_layers.position]
anchor = "center"
[background_layers.animation]
pulse_speed = 0.5
pulse_min_alpha = 0.3
"##;

const TERMINAL_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [terminal]",
    "Initializing subsystems...",
    "Network: standby",
    "VFS: mounted",
    "Ready.",
]
prompt_format = "$> "
title = "OASIS_OS Terminal"
welcome_message = "Type 'help' for available commands."
error_prefix = "error: "
shutdown_message = "Connection closed."
"#;

// ---------------------------------------------------------------------------
// Tactical skin: restricted military-style command console.
// ---------------------------------------------------------------------------

const TACTICAL_MANIFEST: &str = r#"
name = "tactical"
version = "1.0"
author = "OASIS_OS"
description = "Stripped-down tactical command console"
screen_width = 480
screen_height = 272
"#;

const TACTICAL_LAYOUT: &str = r##"
[status_bar]
x = 0
y = 0
w = 480
h = 16
color = "#1A1A1A"
text = "TACTICAL COMMAND SYSTEM"
font_size = 8
text_color = "#808080"

[separator]
x = 0
y = 16
w = 480
h = 1
color = "#333333"

[terminal_bg]
x = 0
y = 17
w = 480
h = 255
z = -1
color = "#0A0A0A"

[terminal_output]
x = 4
y = 20
w = 472
h = 236
color = "#00000000"
text = ""
font_size = 8
text_color = "#AAAAAA"

[terminal_prompt]
x = 4
y = 256
w = 472
h = 12
color = "#00000000"
text = "cmd> "
font_size = 8
text_color = "#CC8800"

[status_left]
x = 4
y = 1
w = 200
h = 14
color = "#00000000"
text = "STATUS: ONLINE"
font_size = 8
text_color = "#00AA00"

[status_right]
x = 330
y = 1
w = 146
h = 14
color = "#00000000"
text = "CLEARANCE: ALPHA"
font_size = 8
text_color = "#CC8800"
"##;

const TACTICAL_FEATURES: &str = r#"
dashboard = false
terminal = true
file_browser = true
window_manager = false
show_tabs = false
command_categories = ["system", "file", "network"]
"#;

const TACTICAL_THEME: &str = r##"
background = "#0A0A0A"
primary = "#CC8800"
secondary = "#333333"
text = "#AAAAAA"
dim_text = "#666666"
status_bar = "#1A1A1A"
prompt = "#CC8800"
output = "#AAAAAA"
error = "#CC3333"

[geometry]
press_flash_duration = 0

[wallpaper]
style = "dots"
grid_color = "#1A1A1A"
grid_spacing = 12

[[background_layers]]
kind = "grid"
spacing = 40
color = "#00FF0028"

[[background_layers]]
kind = "crosshair"
size = 20
color = "#00FF0038"
[background_layers.position]
anchor = "center"

[[background_layers]]
kind = "radar_sweep"
radius = 80
sweep_angle = 0.6
color = "#00FF0030"
[background_layers.position]
anchor = "center"
[background_layers.animation]
rotate_speed = 1.0
"##;

const TACTICAL_STRINGS: &str = r#"
boot_text = [
    "TACTICAL COMMAND SYSTEM v2.2",
    "Clearance level: ALPHA",
    "Secure channel established.",
    "Awaiting input.",
]
prompt_format = "cmd> "
title = "TACTICAL COMMAND"
home_label = "COMMAND"
welcome_message = "Tactical system online. Awaiting orders."
error_prefix = "ERR: "
shutdown_message = "Secure channel terminated."
"#;

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

/// Load the Terminal skin.
pub fn terminal_skin() -> Result<Skin> {
    Skin::from_toml_full(
        TERMINAL_MANIFEST,
        TERMINAL_LAYOUT,
        TERMINAL_FEATURES,
        TERMINAL_THEME,
        TERMINAL_STRINGS,
    )
}

/// Load the Tactical skin.
pub fn tactical_skin() -> Result<Skin> {
    Skin::from_toml_full(
        TACTICAL_MANIFEST,
        TACTICAL_LAYOUT,
        TACTICAL_FEATURES,
        TACTICAL_THEME,
        TACTICAL_STRINGS,
    )
}

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
