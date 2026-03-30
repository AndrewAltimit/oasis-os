//! Special and stylized built-in skins.
//!
//! Skins: corrupted, balatro, paper, solarized, vaporwave, highcontrast, altimit.

use oasis_types::error::Result;

use crate::loader::Skin;

// ---------------------------------------------------------------------------
// Corrupted skin: garbled Terminal variant with glitch effects.
// ---------------------------------------------------------------------------

const CORRUPTED_MANIFEST: &str = r#"
name = "corrupted"
version = "1.0"
author = "OASIS_OS"
description = "Damaged terminal with visual corruption and garbled output"
screen_width = 480
screen_height = 272
"#;

const CORRUPTED_LAYOUT: &str = r##"
[terminal_bg]
x = 0
y = 0
w = 480
h = 272
z = -1
color = "#050005"

[glitch_overlay]
x = 0
y = 0
w = 480
h = 272
color = "#FF000008"
alpha = 20

[terminal_output]
x = 4
y = 4
w = 472
h = 252
color = "#00000000"
text = ""
font_size = 8
text_color = "#CC00CC"

[terminal_prompt]
x = 4
y = 256
w = 472
h = 12
color = "#00000000"
text = "?> "
font_size = 8
text_color = "#FF00FF"
"##;

const CORRUPTED_FEATURES: &str = r#"
dashboard = false
terminal = true
file_browser = true
window_manager = false
show_tabs = false
corrupted = true
"#;

const CORRUPTED_THEME: &str = r##"
background = "#050005"
primary = "#FF00FF"
secondary = "#330033"
text = "#CC00CC"
dim_text = "#660066"
status_bar = "#1A001A"
prompt = "#FF00FF"
output = "#CC00CC"
error = "#FF3333"

[geometry]
toast_slide_in = false

[[background_layers]]
kind = "scanlines"
spacing = 1
color = "#FF000030"

[[background_layers]]
kind = "floating_polygons"
count = 2
sides = 3
color = "#FF000038"
[background_layers.animation]
drift_x = 15.0
drift_y = 10.0
"##;

const CORRUPTED_STRINGS: &str = r#"
boot_text = [
    "O@S!S_OS v?.? [c0rrupt3d]",
    "W4RNING: syst3m int3grity compromis3d",
    "M0dules: [DAMAGED]",
    "VFS: m0unt3d (errors detected)",
    "R3ady... maybe.",
]
prompt_format = "?> "
title = "???_OS"
welcome_message = "Syst3m unst4ble. Proc33d with c4ution."
error_prefix = "3RR: "
shutdown_message = "signal l0st..."
"#;

const CORRUPTED_MODIFIERS: &str = r#"
position_jitter = 2
alpha_flicker_chance = 0.15
alpha_flicker_min = 60
text_garble_chance = 0.08
intensity = 1.0
"#;

// ---------------------------------------------------------------------------
// Balatro skin: dark with neon cyan/magenta accents and glow effects.
// ---------------------------------------------------------------------------

const CYBERPUNK_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [balatro]",
    "Neon subsystems: online",
    "Neural link: established",
    "Ready.",
]
prompt_format = "neon> "
title = "OASIS Balatro"
home_label = "NEON"
welcome_message = "Welcome to the neon grid. Type 'help' for commands."
error_prefix = "FAULT: "
shutdown_message = "Signal lost."
"#;

// ---------------------------------------------------------------------------
// Paper skin: minimal cream/white, no shadows, maximum readability.
// ---------------------------------------------------------------------------

const PAPER_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [paper]",
    "Loading minimal interface...",
    "Ready.",
]
prompt_format = "> "
title = "OASIS Paper"
home_label = "Home"
welcome_message = "Welcome. Type 'help' for commands."
error_prefix = "error: "
shutdown_message = "Done."
"#;

// ---------------------------------------------------------------------------
// Solarized skin: Solarized Dark color palette, developer-focused.
// ---------------------------------------------------------------------------

const SOLARIZED_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [solarized]",
    "Loading Solarized interface...",
    "Ready.",
]
prompt_format = "$ "
title = "OASIS Solarized"
home_label = "Home"
welcome_message = "Welcome to OASIS Solarized. Type 'help' for commands."
error_prefix = "error: "
shutdown_message = "Goodbye."
"#;

// ---------------------------------------------------------------------------
// Vaporwave skin: aesthetic purple/pink/cyan palette.
// ---------------------------------------------------------------------------

const VAPORWAVE_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [vaporwave]",
    "Loading aesthetic interface...",
    "Ready.",
]
prompt_format = "~ "
title = "OASIS Vaporwave"
home_label = "Home"
welcome_message = "Welcome to OASIS Vaporwave. Type 'help' for commands."
error_prefix = "error: "
shutdown_message = "Goodbye."
"#;

// ---------------------------------------------------------------------------
// High Contrast skin: accessibility theme, black/white/yellow.
// ---------------------------------------------------------------------------

const HIGHCONTRAST_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [highcontrast]",
    "Loading high contrast interface...",
    "Ready.",
]
prompt_format = "> "
title = "OASIS High Contrast"
home_label = "Home"
welcome_message = "Welcome to OASIS High Contrast. Type 'help' for commands."
error_prefix = "ERROR: "
shutdown_message = "Goodbye."
"#;

// ---------------------------------------------------------------------------
// Altimit skin: .hack//SIGN-inspired vector icon desktop.
// ---------------------------------------------------------------------------

const ALTIMIT_STRINGS: &str = r#"
boot_text = [
    "ALTIMIT OS v1.0",
    "Connecting to THE WORLD...",
    "Network initialized.",
    "Ready.",
]
prompt_format = ">> "
title = "ALTIMIT"
home_label = "THE WORLD"
welcome_message = "Welcome to ALTIMIT. Type 'help' for commands."
error_prefix = "ERROR: "
shutdown_message = "Logging out..."
"#;

/// Load the Corrupted skin.
pub fn corrupted_skin() -> Result<Skin> {
    Skin::from_toml_corrupted(
        CORRUPTED_MANIFEST,
        CORRUPTED_LAYOUT,
        CORRUPTED_FEATURES,
        CORRUPTED_THEME,
        CORRUPTED_STRINGS,
        CORRUPTED_MODIFIERS,
    )
}

/// Load the Balatro skin.
pub fn balatro_skin() -> Result<Skin> {
    Skin::from_toml_full(
        include_str!("../../../../skins/balatro/skin.toml"),
        include_str!("../../../../skins/balatro/layout.toml"),
        include_str!("../../../../skins/balatro/features.toml"),
        include_str!("../../../../skins/balatro/theme.toml"),
        CYBERPUNK_STRINGS,
    )
}

/// Load the Paper skin.
pub fn paper_skin() -> Result<Skin> {
    Skin::from_toml_full(
        include_str!("../../../../skins/paper/skin.toml"),
        include_str!("../../../../skins/paper/layout.toml"),
        include_str!("../../../../skins/paper/features.toml"),
        include_str!("../../../../skins/paper/theme.toml"),
        PAPER_STRINGS,
    )
}

/// Load the Solarized skin.
pub fn solarized_skin() -> Result<Skin> {
    Skin::from_toml_full(
        include_str!("../../../../skins/solarized/skin.toml"),
        include_str!("../../../../skins/solarized/layout.toml"),
        include_str!("../../../../skins/solarized/features.toml"),
        include_str!("../../../../skins/solarized/theme.toml"),
        SOLARIZED_STRINGS,
    )
}

/// Load the Vaporwave skin.
pub fn vaporwave_skin() -> Result<Skin> {
    Skin::from_toml_full(
        include_str!("../../../../skins/vaporwave/skin.toml"),
        include_str!("../../../../skins/vaporwave/layout.toml"),
        include_str!("../../../../skins/vaporwave/features.toml"),
        include_str!("../../../../skins/vaporwave/theme.toml"),
        VAPORWAVE_STRINGS,
    )
}

/// Load the High Contrast skin.
pub fn highcontrast_skin() -> Result<Skin> {
    Skin::from_toml_full(
        include_str!("../../../../skins/highcontrast/skin.toml"),
        include_str!("../../../../skins/highcontrast/layout.toml"),
        include_str!("../../../../skins/highcontrast/features.toml"),
        include_str!("../../../../skins/highcontrast/theme.toml"),
        HIGHCONTRAST_STRINGS,
    )
}

// ---------------------------------------------------------------------------
// Protanopia-safe skin: avoids red-green confusion, uses blue/yellow/white.
// ---------------------------------------------------------------------------

const PROTANOPIA_MANIFEST: &str = r#"
name = "protanopia"
version = "1.0"
author = "OASIS_OS"
description = "Color-blind-safe theme for protanopia: avoids red-green, uses blue/yellow/white/gray"
screen_width = 480
screen_height = 272
"#;

const PROTANOPIA_LAYOUT: &str = r##"
# Top bar (24px) + content area (222px) + bottom bar (26px) = 272px
[content_bg]
x = 0
y = 24
w = 480
h = 222
color = "#0D1117"
"##;

const PROTANOPIA_FEATURES: &str = r#"
dashboard = true
terminal = true
file_browser = true
browser = true
window_manager = false
dashboard_pages = 2
icons_per_page = 9
grid_cols = 3
grid_rows = 3
start_menu = true
show_version = false
show_tabs = false
"#;

const PROTANOPIA_THEME: &str = r##"
# Protanopia-safe palette: no red/green distinction needed
# Success = blue, Warning = yellow/amber, Error = magenta/pink
background = "#0D1117"
primary = "#2F81F7"
secondary = "#30363D"
text = "#F0F6FC"
dim_text = "#8B949E"
status_bar = "#161B22"
prompt = "#2F81F7"
output = "#E6EDF3"
error = "#DA3B8A"

border_radius = 4
shadow_intensity = 1
gradient_enabled = false

[wallpaper]
style = "solid"
color_stops = ["#0D1117"]

[bar_overrides]
statusbar_bg = "#161B22"
text_shadow = false

[icon_overrides]
body_color = "#161B22"
icon_style = "card"
cursor_style = "fill"

[app_overrides]
app_bg = "#0D1117"
text_color = "#F0F6FC"
selection_accent_color = "#2F81F760"

[start_menu_overrides]
panel_bg = "#161B22"
button_bg = "#2F81F7"
button_text = "#FFFFFF"
button_gradient = false
panel_width = 200
panel_border_radius = 6
columns = 2

[browser_overrides]
chrome_bg = "#161B22"
url_bar_bg = "#0D1117"
link_color = "#58A6FF"

[scrollbar_overrides]
track_color = "#161B22"
thumb_color = "#2F81F7"

[geometry]
statusbar_height = 24
bottombar_height = 26
font_body = 12
font_hint = 10
font_heading = 14
"##;

const PROTANOPIA_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [protanopia]",
    "Loading protanopia-safe interface...",
    "Color-blind accessibility: enabled",
    "Ready.",
]
prompt_format = "> "
title = "OASIS Protanopia"
home_label = "Home"
welcome_message = "Welcome to OASIS (protanopia-safe). Type 'help' for commands."
error_prefix = "ERROR: "
shutdown_message = "Goodbye."
"#;

// ---------------------------------------------------------------------------
// Tritanopia-safe skin: avoids blue-yellow confusion, uses red/green/magenta.
// ---------------------------------------------------------------------------

const TRITANOPIA_MANIFEST: &str = r#"
name = "tritanopia"
version = "1.0"
author = "OASIS_OS"
description = "Color-blind-safe theme for tritanopia: avoids blue-yellow, uses red/green/magenta/cyan"
screen_width = 480
screen_height = 272
"#;

const TRITANOPIA_LAYOUT: &str = r##"
# Top bar (24px) + content area (222px) + bottom bar (26px) = 272px
[content_bg]
x = 0
y = 24
w = 480
h = 222
color = "#1A1A1A"
"##;

const TRITANOPIA_FEATURES: &str = r#"
dashboard = true
terminal = true
file_browser = true
browser = true
window_manager = false
dashboard_pages = 2
icons_per_page = 9
grid_cols = 3
grid_rows = 3
start_menu = true
show_version = false
show_tabs = false
"#;

const TRITANOPIA_THEME: &str = r##"
# Tritanopia-safe palette: no blue/yellow distinction needed
# Success = green, Warning = red/magenta, Error = red
background = "#1A1A1A"
primary = "#E040A0"
secondary = "#333333"
text = "#F0F0F0"
dim_text = "#999999"
status_bar = "#262626"
prompt = "#E040A0"
output = "#E0E0E0"
error = "#E03030"

border_radius = 4
shadow_intensity = 1
gradient_enabled = false

[wallpaper]
style = "solid"
color_stops = ["#1A1A1A"]

[bar_overrides]
statusbar_bg = "#262626"
text_shadow = false

[icon_overrides]
body_color = "#262626"
icon_style = "card"
cursor_style = "fill"

[app_overrides]
app_bg = "#1A1A1A"
text_color = "#F0F0F0"
selection_accent_color = "#E040A060"

[start_menu_overrides]
panel_bg = "#262626"
button_bg = "#E040A0"
button_text = "#FFFFFF"
button_gradient = false
panel_width = 200
panel_border_radius = 6
columns = 2

[browser_overrides]
chrome_bg = "#262626"
url_bar_bg = "#1A1A1A"
link_color = "#E060B0"

[scrollbar_overrides]
track_color = "#262626"
thumb_color = "#E040A0"

[geometry]
statusbar_height = 24
bottombar_height = 26
font_body = 12
font_hint = 10
font_heading = 14
"##;

const TRITANOPIA_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [tritanopia]",
    "Loading tritanopia-safe interface...",
    "Color-blind accessibility: enabled",
    "Ready.",
]
prompt_format = "> "
title = "OASIS Tritanopia"
home_label = "Home"
welcome_message = "Welcome to OASIS (tritanopia-safe). Type 'help' for commands."
error_prefix = "ERROR: "
shutdown_message = "Goodbye."
"#;

/// Load the Protanopia-safe skin (avoids red-green confusion).
pub fn protanopia_skin() -> Result<Skin> {
    Skin::from_toml_full(
        PROTANOPIA_MANIFEST,
        PROTANOPIA_LAYOUT,
        PROTANOPIA_FEATURES,
        PROTANOPIA_THEME,
        PROTANOPIA_STRINGS,
    )
}

/// Load the Tritanopia-safe skin (avoids blue-yellow confusion).
pub fn tritanopia_skin() -> Result<Skin> {
    Skin::from_toml_full(
        TRITANOPIA_MANIFEST,
        TRITANOPIA_LAYOUT,
        TRITANOPIA_FEATURES,
        TRITANOPIA_THEME,
        TRITANOPIA_STRINGS,
    )
}

/// Load the Altimit skin (vector icon style).
pub fn altimit_skin() -> Result<Skin> {
    Skin::from_toml_full(
        include_str!("../../../../skins/altimit/skin.toml"),
        include_str!("../../../../skins/altimit/layout.toml"),
        include_str!("../../../../skins/altimit/features.toml"),
        include_str!("../../../../skins/altimit/theme.toml"),
        ALTIMIT_STRINGS,
    )
}
