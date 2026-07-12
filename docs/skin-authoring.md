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
  assets/            # Optional: PNG images (chrome, wallpaper, decals)
    bar_top.png
    wall.png
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
| `texture` | string | Image asset to render instead of a fill (see [Image Assets](#image-assets)) |

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

# -- Desktop icon layout --
icon_layout = "grid"      # "grid" (uniform cells) or "free" (desktop icons)
snap_to_grid = true       # Free layout: snap dropped icons to a virtual grid
launch_on_single_click = true  # false = first click selects, second launches
software_cursor = false   # Draw a themed software cursor (hides host pointer)
```

#### Free icon layout

`icon_layout = "free"` turns the dashboard into a desktop: fixed-size icon
cells (2x the theme's icon dimensions) auto-flow **top-to-bottom in
columns** from the top-left, PSIX-style, instead of stretching to fill a
centered grid. Pointer users can drag icons anywhere in the content area;
drops snap to a virtual grid (unless `snap_to_grid = false`), are clamped
away from the bars, and persist per skin (settings key
`icon_positions.<skin>.<app path>`). The selected icon gets a themed
highlight driven by the `[icon_overrides]` cursor fields (`cursor_style =
"stroke"` outlines it, `"fill"` paints a backdrop, `"none"` disables).
D-pad navigation still walks icons in reading order, so keyboard and PSP
users lose nothing.

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
# Accent color (default: same as primary). Drives the accent family
# (hover/pressed/subtle) when set, letting a skin highlight with a
# color distinct from its primary.
accent = "#01CDFE"
# Accent hover (default: lighten(accent, 15%))
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
# Synonym for titlebar_text (takes precedence when both are set):
# titlebar_text_active = "#FFFFFF"
# Title text color for unfocused windows (default: same as active):
titlebar_text_inactive = "#AAAAAA"
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
# Button side: "right" (default) or "left" (macOS convention).
# Regardless of side, physical L-to-R order is always
# minimize → maximize → close.
button_side = "right"
# Glyphs drawn inside the buttons. Defaults: "-" / "\u25A1" / "x".
glyph_minimize = "-"
glyph_maximize = "□"
glyph_close = "x"
```

Double-clicking the titlebar body toggles maximize/restore for windows
that support it (AppWindow). The toggle uses a 500 ms / 6 px gate and
shares its code path with the maximize button.

### Fine-Grained Overrides

Override any specific UI element without changing the base color derivation.

#### Bar Overrides

```toml
[bar_overrides]
bar_bg = "#00000060"
statusbar_bg = "#00000050"
# Fallback for ALL bar text elements (battery, version, clock, URL,
# USB, pipes, hints, category label); element-specific colors below win.
text_color = "#000000"
# Fallback gradient for both bars; statusbar_gradient_* / bar_gradient_*
# take precedence.
gradient_top = "#3D2B79"
gradient_bottom = "#1A0A2E"
battery_color = "#78FF78"
tab_active_fill = "#FFFFFF1E"
tab_active_alpha = 200
tab_inactive_alpha = 80
page_dot_active = "#FFFFFFC8"
page_dot_inactive = "#FFFFFF32"
# Also: separator_color, version_color, clock_color, url_color,
#   usb_color, media_tab_active, media_tab_inactive, pipe_color,
#   r_hint_color, category_label_color
# Taskbar (desktop window list):
#   taskbar_bg, taskbar_btn_active, taskbar_btn_inactive,
#   taskbar_btn_minimized, taskbar_btn_hover, taskbar_text_color,
#   taskbar_separator, taskbar_indicator
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

## Image Assets

Skins can ship PNG images in an `assets/` subdirectory. Every `*.png` is
decoded to RGBA at load time and referenced by its skin-relative path
(`"assets/<file>.png"`). Skins under `skins/` that are compiled in as
built-ins embed their assets in the binary automatically.

Six things consume assets:

### 1. Textured layout objects (shaped chrome)

```toml
# layout.toml
[bar_top]
x = 0
y = 0
w = 480
h = 28
texture = "assets/bar_top.png"   # alpha-blended, any silhouette works
```

The bitmap is alpha-blended, so a notched or curved bar is just the alpha
silhouette of the PNG — no shape primitives needed (this is how PSIX does
its shaped chrome). When `w`/`h` are omitted the object takes the image's
native pixel size; otherwise the image is stretched to fit. The `alpha`
field still applies, so textured chrome can be translucent.

### 2. Image wallpapers

```toml
# theme.toml
[wallpaper]
style = "image"
source = "assets/wall.png"
fit = "cover"                 # cover | contain | stretch | tile
color_stops = ["#101018"]     # base color under transparent regions
```

`cover` fills the screen and crops overflow, `contain` letterboxes,
`stretch` ignores aspect ratio, `tile` repeats at native size. Scaled
modes sample bilinearly. The image composites over a solid base from the
first color stop, so a transparent PNG shows the base color through.

### 3. Image background layers (watermark decals)

```toml
# theme.toml
[[background_layers]]
kind = "image"
source = "assets/logo.png"
alpha = 96                    # base opacity 0-255

[background_layers.position]
anchor = "bottom_right"       # same anchors as vector layers
offset_x = -0.02              # fraction of screen width

[background_layers.animation]
pulse_speed = 0.25            # Hz; oscillates alpha
pulse_min_alpha = 0.5
drift_x = 4.0                 # px oscillation amplitude
```

Decals render between the wallpaper and the icon layer, scale uniformly
with the skin's native resolution, and animate without re-uploading
pixels. Set `reduced_motion = true` (features.toml) to freeze them.

### 4. Themed software cursor

```toml
# theme.toml
[cursor]
texture = "assets/cursor.png"
hotspot = [1, 1]              # click point within the image (default [0, 0])
```

Only used when the skin sets `software_cursor = true` in features.toml —
that hides the host OS pointer and draws the skin's cursor as a
top-most SDI object instead. Without a `texture` the built-in procedural
arrow is drawn, so `software_cursor = true` alone already gives a themed
resolution-scaled pointer.

### 5. Nine-patch chrome (scalable borders)

```toml
# layout.toml — any layout object
[side_panel]
x = 8
y = 40
w = 180
h = 200
nine_patch = { image = "assets/panel.png", insets = [6, 6, 6, 6] }

# theme.toml — window manager chrome
[wm_theme]
titlebar_nine_patch = { image = "assets/titlebar.png", insets = [8, 4, 8, 4] }
frame_nine_patch = { image = "assets/frame.png", insets = [4, 4, 4, 4] }
```

A nine-patch splits the image into a 3x3 grid using `insets`
(`[left, top, right, bottom]`, in texture pixels): corners render at
fixed size, edges stretch along one axis, and the center stretches in
both — one small bitmap scales to any panel, bar, or window size without
smearing its border. WM nine-patches apply to every window (titlebars
follow resizes live) and stay crisp because slicing happens at draw time.
`nine_patch` takes precedence over `texture` on the same object.

### 6. Top-tab pill textures

```toml
# theme.toml
[bar_overrides]
tab_texture_active = "assets/tab_active.png"
tab_texture_inactive = "assets/tab_inactive.png"
```

Replaces the procedural pill behind the APPS/MODS/NET tabs with
alpha-blended bitmaps (shaped tab chrome, PSIX-style). The texture swaps
between the two states as the active tab changes; either key may be set
alone — the other state keeps the pill fill.

### Asset guidelines

- **Power-of-two dimensions** (64, 128, 256, …) — required on PSP,
  flagged by `skin lint` otherwise.
- Stay under the **2 MB decoded budget** per skin (`skin lint` warns).
  PNG on disk compresses far smaller; the budget is about RAM/VRAM.
- `skin lint` also verifies every `texture =`, `nine_patch` image,
  wallpaper `source`, layer `source`, tab pill texture, and `[cursor]`
  texture resolves to a shipped asset, and that nine-patch insets fit
  inside their image.

## Chrome Layers (vector overlay decorations)

`[[chrome_layers]]` mirrors `[[background_layers]]` but renders in the
overlay pass — on top of the bars, tabs, and windows — for procedurally
shaped chrome accents without shipping art:

```toml
# theme.toml
[[chrome_layers]]
kind = "crosshair"
size = 12
color = "#FFFFFF30"
position = { anchor = "top_right", offset_x = -0.05, offset_y = 0.04 }

[[chrome_layers]]
kind = "scanlines"
spacing = 3
color = "#00000018"
```

All vector layer kinds work (`grid`, `dot_grid`, `wireframe_sphere`,
`radar_sweep`, `concentric_rings`, `glass_shard`, `scanlines`, `eq_bars`,
`crosshair`, `floating_polygons`, `pulsing_core`, `waves`) with the same
`position` / `animation` sub-tables; `"image"` and `"shader"` kinds are
background-only (`skin lint` flags them). Static layers are tessellated
once and cached; only animated layers rebuild per frame. The
`background_performance` table (`max_layers`, `complexity_budget`,
`reduced_motion`) applies to chrome layers too.

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
> skin lint my_skin     # Validate a skin and report warnings
```

## Validation & Linting

Unknown TOML keys never fail a skin load (forwards compatibility), but
they are recorded and surfaced so typos and unsupported fields don't
silently do nothing:

- Loading an external skin logs each unknown key as a warning.
- `skin lint <name|path>` prints the full report: unknown keys, invalid
  hex colors, out-of-bounds layout coordinates, feature-flag
  inconsistencies (e.g. `icons_per_page` exceeding the grid capacity),
  and asset problems (missing `texture`/`source` references,
  non-power-of-two images, decoded size over the per-skin budget).

Lint your skin whenever a field appears to have no effect — a
misspelled key is the most common cause. All shipped skins are kept
lint-clean by a CI test (`all_shipped_skins_lint_clean`).

## Transitions & Motion

Transition timing can be set in frames (features.toml) or milliseconds
(theme.toml). Explicit frame counts win when both are present:

```toml
# features.toml
transition_fade_frames = 15     # frames at 60 fps

# theme.toml
[transition]
fade_color = "#000000"
fade_ms = 300                   # converted to frames at 60 fps
slide_ms = 400
entrance = "assemble"           # fade (default) | assemble | none
entrance_ms = 750               # assemble duration (default 750ms)
page_style = "slide"            # slide (default) | fade
easing = "ease_out_cubic"       # entrance easing (see below)
```

- **`entrance`** plays on boot and skin swap. `"assemble"` is the
  PSIX signature move: the top bar slides down and the bottom bar
  slides up from off-screen while a dark iris shrinks from the center;
  bar text and tabs pop in when the chrome lands. It falls back to a
  plain fade when `background_performance.reduced_motion` is set.
- **`page_style = "fade"`** replaces the dashboard's horizontal page
  slide with a quick crossfade.
- **`easing`** overrides the entrance's built-in curve. Supported names:
  `linear`, `ease_in_quad`, `ease_out_quad`, `ease_in_out_quad`,
  `ease_out_cubic`, `ease_in_out_cubic`, `ease_out_elastic`,
  `ease_out_bounce`.
- In free icon layout, the selection highlight follows the mouse
  (hover focus), driving the `focus_scale` / `focus_glow` icon
  micro-motion from `[icon_overrides]`.

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
| corrupted | Glitched terminal | Terminal + corruption effects |
| desktop | Windowed desktop | WM + terminal |
| modern | Purple accent, rounded | Dashboard + WM + browser |

## Worked Example: "Neon" Skin

Create `skins/neon/skin.toml`:
```toml
name = "neon"
version = "1.0"
author = "Example"
description = "Balatro neon aesthetic"
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
