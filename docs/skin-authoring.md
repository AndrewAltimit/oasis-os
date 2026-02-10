# Skin Authoring Guide

This guide covers creating custom skins for OASIS_OS. A skin controls the
visual identity: colors, layout, feature flags, typography, and effects.

## Directory Structure

A skin is a directory containing TOML configuration files:

```
skins/my_skin/
  skin.toml          # Required: manifest (name, version, screen size)
  layout.toml        # Required: SDI object definitions (positions, colors)
  features.toml      # Required: feature flags (dashboard, terminal, WM)
  theme.toml         # Optional: color palette and visual properties
  strings.toml       # Optional: terminal strings (prompts, boot text)
  corrupted.toml     # Optional: corrupted effect modifiers
```

Only `skin.toml`, `layout.toml`, and `features.toml` are required. Missing
optional files use built-in defaults.

## Quick Start

```bash
# Copy classic as a starting point
cp -r skins/classic skins/my_skin

# Edit theme colors
$EDITOR skins/my_skin/theme.toml

# Run with your skin
OASIS_SKIN=my_skin cargo run -p oasis-app

# Or pass as CLI argument
cargo run -p oasis-app -- my_skin
```

## File Reference

### skin.toml (Manifest)

```toml
name = "my_skin"
version = "1.0"
author = "Your Name"
description = "A custom skin for OASIS_OS"
screen_width = 480      # Virtual resolution width (default: 480)
screen_height = 272     # Virtual resolution height (default: 272)
```

### layout.toml (SDI Object Definitions)

Each top-level key defines a named SDI object. These are the building blocks
of the visual scene.

```toml
[status_bar]
x = 0
y = 0
w = 480
h = 24
color = "#283C5A"
text = "OASIS_OS"
font_size = 8
text_color = "#FFFFFF"

[content_bg]
x = 0
y = 24
w = 480
h = 224
color = "#1A1A2D"
# Optional extended properties:
border_radius = 4
gradient_top = "#181828"
gradient_bottom = "#10101A"
shadow_level = 1
stroke_width = 1
stroke_color = "#44446640"
```

Available fields per object:
| Field | Type | Description |
|-------|------|-------------|
| `x`, `y` | i32 | Position (pixels from top-left) |
| `w`, `h` | u32 | Size (pixels) |
| `color` | "#RRGGBB" or "#RRGGBBAA" | Fill color |
| `text` | string | Text content |
| `text_color` | hex color | Text color |
| `font_size` | u16 | Font size (8 = default bitmap) |
| `alpha` | u8 | Object alpha (0-255) |
| `visible` | bool | Initial visibility |
| `border_radius` | u16 | Rounded corner radius |
| `gradient_top` | hex color | Top gradient color |
| `gradient_bottom` | hex color | Bottom gradient color |
| `shadow_level` | u8 | Drop shadow intensity (0-3) |
| `stroke_width` | u16 | Border stroke width |
| `stroke_color` | hex color | Border stroke color |

### features.toml (Feature Flags)

```toml
dashboard = true          # Show icon grid dashboard
terminal = true           # Enable command terminal
file_browser = true       # Enable ls/cd/cat commands
browser = true            # Enable HTML/CSS browser widget
window_manager = false    # Enable windowed desktop mode
dashboard_pages = 3       # Number of icon grid pages
icons_per_page = 6        # Icons per page
grid_cols = 3             # Grid columns
grid_rows = 2             # Grid rows
corrupted = false         # Enable corrupted visual effects
command_categories = []   # Restrict to specific command categories
```

### theme.toml (Color Palette)

The 9 base colors drive the entire UI. All bar colors, icon colors, browser
chrome, and WM decorations are derived from these.

```toml
# -- 9 Base Colors --
background = "#1A1A2D"    # Main background
primary = "#3264C8"       # Accent (highlights, active elements)
secondary = "#505050"     # Borders, separators
text = "#FFFFFF"          # Primary text
dim_text = "#808080"      # Secondary/dimmed text
status_bar = "#283C5A"    # Status bar background
prompt = "#00FF00"        # Terminal prompt color
output = "#CCCCCC"        # Terminal output color
error = "#FF4444"         # Error text color
```

#### Derivation Table

The 9 base colors automatically derive ~30 UI element colors:

| UI Element | Derived From | Transform |
|-----------|-------------|-----------|
| Status bar BG | `status_bar` | alpha 80 |
| Bottom bar BG | `status_bar` | alpha 90 |
| Separator | `secondary` | alpha 50 |
| Battery text | `primary` | lighten 30% |
| Version/Clock | `text` | direct |
| URL label | `dim_text` | direct |
| USB indicator | `dim_text` | direct |
| Tab active fill | `primary` | alpha 30 |
| Media tab active | `text` | direct |
| Media tab inactive | `dim_text` | direct |
| Pipe separator | `text` | alpha 60 |
| Page dot active | `text` | alpha 200 |
| Page dot inactive | `text` | alpha 50 |
| Icon body | `text` | direct |
| Icon label | `text` | alpha 230 |
| Cursor highlight | `primary` | alpha 80 |
| Browser chrome BG | `background` | lighten 10% |
| Browser chrome text | `text` | direct |
| Browser URL bar BG | `background` | darken 80% |
| Browser link color | `primary` | direct |
| WM titlebar active | via `[wm_theme]` overrides | |

#### Extended Visual Properties

```toml
# Surface color (default: lighten(background, 5%))
surface = "#1E1E30"
# Accent hover (default: lighten(primary, 15%))
accent_hover = "#8B7CF7"
# Default border radius for UI elements
border_radius = 6
# Shadow intensity (0=none, 1=subtle, 2=medium, 3=heavy)
shadow_intensity = 2
# Enable gradient fills
gradient_enabled = true
```

#### WM Theme Overrides

```toml
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
titlebar_gradient = true
frame_shadow_level = 1
frame_border_radius = 2
button_radius = 8
```

### Fine-Grained Overrides

Override any specific UI element without changing the base color derivation.

#### Bar Overrides

```toml
[bar_overrides]
bar_bg = "#00000060"
statusbar_bg = "#00000050"
battery_color = "#78FF78"
tab_active_fill = "#FFFFFF1E"
tab_active_alpha = 200
tab_inactive_alpha = 80
page_dot_active = "#FFFFFFC8"
page_dot_inactive = "#FFFFFF32"
# Also: separator_color, version_color, clock_color, url_color,
#   usb_color, media_tab_active, media_tab_inactive, pipe_color,
#   r_hint_color, category_label_color
```

#### Icon Overrides

```toml
[icon_overrides]
body_color = "#FAFAF8"
fold_color = "#D2D2CD"
label_color = "#FFFFFFE6"
cursor_color = "#FFFFFF32"
icon_border_radius = 6
cursor_border_radius = 8
cursor_stroke_width = 2
# Also: outline_color, shadow_color
```

#### Browser Overrides

```toml
[browser_overrides]
chrome_bg = "#303030"
chrome_text = "#CCCCCC"
chrome_button_bg = "#404040"
url_bar_bg = "#202020"
link_color = "#0066CC"
# Also: url_bar_text, status_bar_bg, status_bar_text
```

### strings.toml (Terminal Strings)

```toml
boot_text = [
    "OASIS_OS v2.2",
    "Loading...",
    "Ready.",
]
prompt_format = "> "
title = "My Skin"
home_label = "HOME"
welcome_message = "Welcome! Type 'help' for commands."
error_prefix = "error: "
shutdown_message = "Goodbye."
```

### corrupted.toml (Effect Configuration)

```toml
position_jitter = 2        # Max pixel jitter per frame
alpha_flicker_chance = 0.15 # Probability of alpha flicker
alpha_flicker_min = 60      # Minimum alpha during flicker
text_garble_chance = 0.08   # Probability of character garbling
intensity = 1.0             # Overall effect intensity (0.0-1.0)
```

## Effect System

Effects are pluggable visual modifiers applied each frame. Built-in effects:

- **corrupted**: Position jitter, alpha flicker, text garbling
- **scanlines**: CRT-style horizontal line overlay

Effects are enabled via `features.toml`:
```toml
corrupted = true    # Enable corrupted effect
```

Custom effects implement the `SkinEffect` trait:
```rust
pub trait SkinEffect: Debug {
    fn name(&self) -> &str;
    fn intensity(&self) -> f32;
    fn set_intensity(&mut self, intensity: f32);
    fn apply(&mut self, sdi: &mut SdiRegistry);
}
```

## Runtime Switching

Switch skins at runtime from the terminal:

```
> skin list             # List all available skins
> skin modern           # Switch to the "modern" skin
> skin current          # Show current skin info
> skin skins/my_skin    # Load from a directory path
```

## Testing Your Skin

```bash
# Run with your skin
OASIS_SKIN=my_skin cargo run -p oasis-app

# Take screenshots for comparison
OASIS_SKIN=my_skin cargo run -p oasis-app --bin oasis-screenshot

# Compare against reference
ls screenshots/
```

## Built-In Skins

| Name | Style | Features |
|------|-------|----------|
| classic | PSP icon grid | Dashboard + terminal |
| terminal | Green-on-black CRT | Terminal only |
| tactical | Military console | Terminal + restricted commands |
| corrupted | Glitched terminal | Terminal + corruption effects |
| desktop | Windowed desktop | WM + terminal |
| agent-terminal | AI agent console | Terminal + agent/MCP commands |
| modern | Purple accent, rounded | Dashboard + WM + browser |

## Worked Example: "Neon" Skin

Create `skins/neon/skin.toml`:
```toml
name = "neon"
version = "1.0"
author = "Example"
description = "Cyberpunk neon aesthetic"
```

Create `skins/neon/features.toml`:
```toml
dashboard = true
terminal = true
browser = true
dashboard_pages = 2
icons_per_page = 4
grid_cols = 2
grid_rows = 2
```

Create `skins/neon/layout.toml`:
```toml
[content_bg]
x = 0
y = 24
w = 480
h = 224
color = "#0A0014"
gradient_top = "#0D0018"
gradient_bottom = "#060010"
```

Create `skins/neon/theme.toml`:
```toml
background = "#0A0014"
primary = "#FF00FF"
secondary = "#440044"
text = "#FF88FF"
dim_text = "#884488"
status_bar = "#1A0028"
prompt = "#FF00FF"
output = "#CC66CC"
error = "#FF3333"
border_radius = 8
shadow_intensity = 2

[bar_overrides]
battery_color = "#FF00FF"
page_dot_active = "#FF00FFC8"

[browser_overrides]
link_color = "#FF44FF"
```

Run it:
```bash
OASIS_SKIN=neon cargo run -p oasis-app
```
