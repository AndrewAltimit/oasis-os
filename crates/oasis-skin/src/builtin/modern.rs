//! Modern and desktop built-in skins.
//!
//! Skins: desktop, modern, xp, macos, gnome, agent-terminal.

use oasis_types::error::Result;

use crate::loader::Skin;

// ---------------------------------------------------------------------------
// Desktop skin: WM-enabled with taskbar and windowed apps.
// ---------------------------------------------------------------------------

const DESKTOP_MANIFEST: &str = r#"
name = "desktop"
version = "1.0"
author = "OASIS_OS"
description = "Desktop-style interface with window manager and taskbar"
screen_width = 800
screen_height = 600
"#;

const DESKTOP_LAYOUT: &str = r##"
[desktop_bg]
x = 0
y = 0
w = 800
h = 600
color = "#1A1A2D"
gradient_top = "#22223A"
gradient_bottom = "#121220"

[taskbar_bg]
x = 0
y = 568
w = 800
h = 32
color = "#222233"
gradient_top = "#2A2A44"
gradient_bottom = "#1A1A33"

[taskbar_separator]
x = 0
y = 567
w = 800
h = 1
color = "#444466"

[task_area]
x = 68
y = 572
w = 658
h = 24
color = "#1E1E3060"

[clock_display]
x = 730
y = 572
w = 66
h = 24
color = "#00000000"
text = "00:00"
font_size = 10
text_color = "#AAAACC"
"##;

const DESKTOP_FEATURES: &str = r#"
dashboard = false
terminal = true
file_browser = true
window_manager = true
show_tabs = false
"#;

const DESKTOP_THEME: &str = r##"
background = "#1A1A2D"
primary = "#3264C8"
secondary = "#444466"
text = "#FFFFFF"
dim_text = "#8888AA"
status_bar = "#222233"
prompt = "#00FF00"
output = "#CCCCCC"
error = "#FF4444"
border_radius = 4
shadow_intensity = 1

[bar_overrides]
text_shadow = true

[geometry]
focus_ring_color = "#3264C8B0"
toast_slide_in = true

[wm_theme]
titlebar_height = 24
border_width = 1
titlebar_active = "#3264C8"
titlebar_inactive = "#555566"
titlebar_text = "#FFFFFF"
frame_color = "#333344"
content_bg = "#1E1E2E"
btn_close = "#C83232"
btn_minimize = "#C8B432"
btn_maximize = "#32C832"
button_size = 16
resize_handle_size = 6
titlebar_font_size = 12
titlebar_radius = 4
frame_shadow_level = 1
frame_border_radius = 2
button_radius = 8
"##;

const DESKTOP_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [desktop]",
    "Loading desktop environment...",
    "Window manager: active",
    "Ready.",
]
prompt_format = "{cwd} $ "
title = "OASIS Desktop"
home_label = "Desktop"
welcome_message = "Welcome to OASIS Desktop."
error_prefix = "error: "
shutdown_message = "Desktop session ended."
"#;

// ---------------------------------------------------------------------------
// Modern skin: showcases all v2 visual features (rounded, gradients, shadows).
// ---------------------------------------------------------------------------

const MODERN_MANIFEST: &str = r#"
name = "modern"
version = "1.0"
author = "OASIS_OS"
description = "Modern UI with rounded corners, gradients, and shadows"
screen_width = 480
screen_height = 272
"#;

const MODERN_LAYOUT: &str = r##"
[content_bg]
x = 0
y = 24
w = 480
h = 224
color = "#14141E"
gradient_top = "#181828"
gradient_bottom = "#10101A"
border_radius = 0
shadow_level = 0
"##;

const MODERN_FEATURES: &str = r#"
dashboard = true
terminal = true
file_browser = true
browser = true
window_manager = true
dashboard_pages = 2
icons_per_page = 9
grid_cols = 3
grid_rows = 3
show_tabs = false
"#;

const MODERN_THEME: &str = r##"
background = "#14141E"
primary = "#6C5CE7"
secondary = "#3D3852"
text = "#F0F0FF"
dim_text = "#7E7A90"
status_bar = "#1A1A2D"
prompt = "#A29BFE"
output = "#DDD6FE"
error = "#FF6B6B"
surface = "#1E1E30"
accent_hover = "#8B7CF7"
border_radius = 6
shadow_intensity = 2
gradient_enabled = true

[bar_overrides]
text_shadow = true

[geometry]
tab_row_height = 0
icon_width = 26
icon_height = 30
font_body = 12
font_hint = 10
font_heading = 14
cursor_lerp_speed = 0.15
start_menu_anim_speed = 0.12
focus_ring_color = "#6C5CE7AA"
cursor_pad = 4
press_flash_lighten = 0.3
toast_slide_in = true

[app_overrides]
selection_accent_color = "#6C5CE780"
selection_border_radius = 4
title_bar_text_shadow = true

[wm_theme]
titlebar_height = 24
border_width = 1
titlebar_active = "#6C5CE7"
titlebar_inactive = "#3D3852"
titlebar_text = "#F0F0FF"
frame_color = "#2A2A40"
content_bg = "#181828"
btn_close = "#FF6B6B"
btn_minimize = "#FFD93D"
btn_maximize = "#6BCB77"
button_size = 16
resize_handle_size = 6
titlebar_font_size = 12
titlebar_radius = 6
titlebar_gradient = true
frame_shadow_level = 2
frame_border_radius = 4
button_radius = 8

[icon_overrides]
icon_style = "vector"
vector_preset = "outline"
icon_container = "chip"

[[background_layers]]
kind = "glass_shard"
color = "#FFFFFF20"
points = [[0.3, 0.25], [0.6, 0.1], [0.65, 0.45]]
"##;

const MODERN_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [modern]",
    "Loading modern interface...",
    "UI subsystems: active",
    "Ready.",
]
prompt_format = "> "
title = "OASIS Modern"
home_label = "Home"
welcome_message = "Welcome to OASIS Modern. Type 'help' for commands."
error_prefix = "error: "
shutdown_message = "Session ended."
"#;

// ---------------------------------------------------------------------------
// XP skin: Windows XP Luna blue theme.
// ---------------------------------------------------------------------------

const XP_MANIFEST: &str = r#"
name = "xp"
version = "1.0"
author = "OASIS_OS"
description = "Windows XP Luna-inspired blue theme with gradient titlebars and taskbar"
screen_width = 1024
screen_height = 768
"#;

const XP_LAYOUT: &str = r##"
[content_bg]
x = 0
y = 30
w = 1024
h = 698
color = "#ECE9D8"
"##;

const XP_FEATURES: &str = r#"
dashboard = true
terminal = true
file_browser = true
browser = true
window_manager = true
dashboard_pages = 2
icons_per_page = 15
grid_cols = 5
grid_rows = 3
start_menu = true
show_version = false
show_tabs = false
transition_fade_frames = 12
transition_slide_frames = 16
"#;

const XP_THEME: &str = r##"
background = "#003399"
primary = "#003399"
secondary = "#1F3E7B"
text = "#FFFFFF"
dim_text = "#8899BB"
status_bar = "#1F3E7B"
prompt = "#FFFFFF"
output = "#FFFFFF"
error = "#FF0000"
border_radius = 3
shadow_intensity = 1
gradient_enabled = true

[wm_theme]
titlebar_height = 30
border_width = 1
titlebar_active = "#0054E3"
titlebar_inactive = "#7B7B7B"
titlebar_text = "#FFFFFF"
frame_color = "#0054E3"
content_bg = "#ECE9D8"
btn_close = "#C75050"
btn_minimize = "#406BBD"
btn_maximize = "#406BBD"
button_size = 20
titlebar_font_size = 14
titlebar_radius = 4
titlebar_gradient = true
titlebar_gradient_top = "#3A6EA5"
titlebar_gradient_bottom = "#0A246A"
titlebar_inactive_gradient_top = "#B4B4B4"
titlebar_inactive_gradient_bottom = "#7B7B7B"
frame_shadow_level = 1
frame_border_radius = 3
button_radius = 2
button_side = "right"
glyph_close = "x"
glyph_minimize = "-"
glyph_maximize = "□"
title_align = "left"
separator_enabled = true
separator_color = "#0054E340"
glyph_close_color = "#FFFFFF"
glyph_minimize_color = "#FFFFFF"
glyph_maximize_color = "#FFFFFF"
btn_close_hover = "#E66060"
btn_minimize_hover = "#5080D0"
btn_maximize_hover = "#5080D0"
title_text_shadow = true
title_text_shadow_color = "#00000060"
content_stroke_width = 1
content_stroke_color = "#0054E320"
maximize_top_inset = 52
maximize_bottom_inset = 40
inactive_frame_alpha = 160

[bar_overrides]
statusbar_bg = "#1F3E7B"
statusbar_gradient_top = "#3169C6"
statusbar_gradient_bottom = "#1F3E7B"
bar_bg = "#1F3E7B"
bar_gradient_top = "#3169C6"
bar_gradient_bottom = "#1F3E7B"
battery_color = "#FFFFFF"
version_color = "#FFFFFF"
clock_color = "#FFFFFF"
separator_color = "#4080D0"
text_shadow = true

[wallpaper]
style = "gradient"
color_stops = ["#003399", "#1F5FC2", "#3A8AE0", "#5BB5FF"]
wave_enabled = false
angle = 90

[geometry]
statusbar_height = 30
bottombar_height = 40
icon_width = 48
icon_height = 56
font_small = 12
tab_row_height = 0
font_body = 14
font_hint = 12
font_heading = 16
cursor_blink_rate = 25
focus_ring_color = "#0054E3A0"
cursor_pad = 4
toast_slide_in = true

[app_overrides]
text_color = "#333333"
dim_text = "#666666"
terminal_output_color = "#000000"
terminal_prompt_color = "#003399"
selection_accent_color = "#0054E380"
selection_border_radius = 2
title_bar_text_shadow = true

[icon_overrides]
body_color = "#ECE9D8"
fold_color = "#C8C2AD"
outline_color = "#0054E380"
label_color = "#000000E6"
cursor_color = "#5B9BD5A0"
cursor_stroke_width = 2
icon_border_radius = 3
cursor_border_radius = 5
icon_style = "vector"
vector_preset = "pixel"
icon_container = "none"
cursor_style = "stroke"

[start_menu_overrides]
panel_bg = "#1F3E7B"
panel_gradient_top = "#3169C6"
panel_gradient_bottom = "#0A246A"
panel_border = "#4080D0"
item_text = "#FFFFFF"
item_text_active = "#FFFFFF"
highlight_color = "#3A6EA580"
button_bg = "#309E30"
button_text = "#FFFFFF"
panel_border_radius = 4
panel_shadow_level = 1
button_label = "start"
button_width = 80
button_height = 30
button_shape = "rect"
button_gradient = true
button_gradient_top = "#4DA54D"
button_gradient_bottom = "#2D852D"
panel_width = 360
columns = 2
header_text = "User"
header_bg = "#003399"
header_text_color = "#FFFFFF"
header_height = 36
footer_enabled = true
footer_bg = "#1F3E7B"
footer_text_color = "#FFFFFF"
footer_height = 30
item_icon_size = 24
item_row_height = 32
item_separator = true
item_separator_color = "#4080D040"

[browser_overrides]
chrome_bg = "#D6D2C2"
chrome_text = "#000000"
chrome_button_bg = "#ECE9D8"
url_bar_bg = "#FFFFFF"
url_bar_text = "#000000"
link_color = "#0066CC"
"##;

const XP_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [xp]",
    "Loading Windows XP Luna theme...",
    "Desktop environment: active",
    "Ready.",
]
prompt_format = "C:\\> "
title = "OASIS XP"
home_label = "My Computer"
welcome_message = "Welcome to OASIS XP. Type 'help' for commands."
error_prefix = "error: "
shutdown_message = "Windows is shutting down..."
"#;

// ---------------------------------------------------------------------------
// macOS skin: light translucent theme with traffic-light window buttons.
// ---------------------------------------------------------------------------

const MACOS_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [macos]",
    "Loading macOS interface...",
    "Window manager: active",
    "Ready.",
]
prompt_format = "~ $ "
title = "OASIS macOS"
home_label = "Finder"
welcome_message = "Welcome to OASIS macOS. Type 'help' for commands."
error_prefix = "error: "
shutdown_message = "Session ended."
"#;

// ---------------------------------------------------------------------------
// GNOME skin: dark Adwaita-inspired with top bar and rounded corners.
// ---------------------------------------------------------------------------

const GNOME_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [gnome]",
    "Loading GNOME desktop...",
    "Activities overlay: ready",
    "Ready.",
]
prompt_format = "$ "
title = "OASIS GNOME"
home_label = "Activities"
welcome_message = "Welcome to OASIS GNOME. Type 'help' for commands."
error_prefix = "error: "
shutdown_message = "Session ended."
"#;

// ---------------------------------------------------------------------------
// Agent Terminal skin: briefcase field terminal for AI agent management.
// ---------------------------------------------------------------------------

const AGENT_TERMINAL_MANIFEST: &str = r#"
name = "agent-terminal"
version = "1.0"
author = "OASIS_OS"
description = "Briefcase field terminal for AI agent management"
screen_width = 480
screen_height = 272
"#;

const AGENT_TERMINAL_LAYOUT: &str = r##"
[status_bar]
x = 0
y = 0
w = 480
h = 18
color = "#0A1A2A"
text = "AGENT TERMINAL"
font_size = 8
text_color = "#00CCCC"

[tamper_indicator]
x = 380
y = 1
w = 96
h = 16
color = "#00000000"
text = "[?]"
font_size = 8
text_color = "#808080"

[separator_top]
x = 0
y = 18
w = 480
h = 1
color = "#006666"
stroke_width = 1
stroke_color = "#00666640"

[agent_panel]
x = 0
y = 19
w = 240
h = 80
color = "#0D1F2D"
text = "Agents: (loading...)"
font_size = 8
text_color = "#00AAAA"
border_radius = 4

[session_panel]
x = 240
y = 19
w = 240
h = 80
color = "#0D1F2D"
text = "Sessions: (none)"
font_size = 8
text_color = "#00AAAA"
border_radius = 4

[panel_divider]
x = 239
y = 19
w = 1
h = 80
color = "#006666"
stroke_width = 1
stroke_color = "#00666640"

[separator_mid]
x = 0
y = 99
w = 480
h = 1
color = "#006666"
stroke_width = 1
stroke_color = "#00666640"

[health_bar]
x = 0
y = 100
w = 480
h = 16
color = "#0A1520"
text = "CPU: -- | MEM: -- | NET: --"
font_size = 8
text_color = "#668888"
border_radius = 4

[separator_term]
x = 0
y = 116
w = 480
h = 1
color = "#006666"
stroke_width = 1
stroke_color = "#00666640"

[terminal_bg]
x = 0
y = 117
w = 480
h = 155
z = -1
color = "#060D15"
border_radius = 4

[terminal_output]
x = 4
y = 120
w = 472
h = 132
color = "#00000000"
text = ""
font_size = 8
text_color = "#00BBBB"

[terminal_prompt]
x = 4
y = 256
w = 472
h = 14
color = "#00000000"
text = "agent> "
font_size = 8
text_color = "#00FFCC"
"##;

const AGENT_TERMINAL_FEATURES: &str = r#"
dashboard = false
terminal = true
file_browser = true
window_manager = false
show_tabs = false
command_categories = ["agent", "mcp", "system", "file", "network"]
"#;

const AGENT_TERMINAL_THEME: &str = r##"
background = "#060D15"
primary = "#00CCCC"
secondary = "#006666"
text = "#00BBBB"
dim_text = "#336666"
status_bar = "#0A1A2A"
prompt = "#00FFCC"
output = "#00BBBB"
error = "#FF4444"

[wallpaper]
style = "grid"
grid_color = "#0D2233"
grid_spacing = 16

[[background_layers]]
kind = "scanlines"
spacing = 2
color = "#00FFFF28"

[[background_layers]]
kind = "grid"
spacing = 30
color = "#00FFFF28"

[[background_layers]]
kind = "pulsing_core"
radius = 10
color = "#00FFFF40"
[background_layers.position]
anchor = "center"
[background_layers.animation]
pulse_speed = 0.3
pulse_min_alpha = 0.4
"##;

const AGENT_TERMINAL_STRINGS: &str = r#"
boot_text = [
    "OASIS_OS v2.2 [agent-terminal]",
    "Briefcase secure terminal initializing...",
    "Loading agent registry...",
    "MCP servers: scanning...",
    "Tamper system: reading state...",
    "Remote terminal: standby",
    "Ready.",
]
prompt_format = "agent> "
title = "Agent Terminal"
home_label = "AGENTS"
welcome_message = "Briefcase agent terminal online. Type 'help' for commands."
error_prefix = "ERR: "
shutdown_message = "Agent terminal session ended."
"#;

/// Load the Desktop skin.
pub fn desktop_skin() -> Result<Skin> {
    Skin::from_toml_full(
        DESKTOP_MANIFEST,
        DESKTOP_LAYOUT,
        DESKTOP_FEATURES,
        DESKTOP_THEME,
        DESKTOP_STRINGS,
    )
}

/// Load the Modern skin.
pub fn modern_skin() -> Result<Skin> {
    Skin::from_toml_full(
        MODERN_MANIFEST,
        MODERN_LAYOUT,
        MODERN_FEATURES,
        MODERN_THEME,
        MODERN_STRINGS,
    )
}

/// Load the XP skin.
pub fn xp_skin() -> Result<Skin> {
    Skin::from_toml_full(XP_MANIFEST, XP_LAYOUT, XP_FEATURES, XP_THEME, XP_STRINGS)
}

/// Load the macOS skin.
pub fn macos_skin() -> Result<Skin> {
    Skin::from_toml_full(
        include_str!("../../../../skins/macos/skin.toml"),
        include_str!("../../../../skins/macos/layout.toml"),
        include_str!("../../../../skins/macos/features.toml"),
        include_str!("../../../../skins/macos/theme.toml"),
        MACOS_STRINGS,
    )
}

/// Load the GNOME skin.
pub fn gnome_skin() -> Result<Skin> {
    Skin::from_toml_full(
        include_str!("../../../../skins/gnome/skin.toml"),
        include_str!("../../../../skins/gnome/layout.toml"),
        include_str!("../../../../skins/gnome/features.toml"),
        include_str!("../../../../skins/gnome/theme.toml"),
        GNOME_STRINGS,
    )
}

/// Load the Agent Terminal skin.
pub fn agent_terminal_skin() -> Result<Skin> {
    Skin::from_toml_full(
        AGENT_TERMINAL_MANIFEST,
        AGENT_TERMINAL_LAYOUT,
        AGENT_TERMINAL_FEATURES,
        AGENT_TERMINAL_THEME,
        AGENT_TERMINAL_STRINGS,
    )
}
