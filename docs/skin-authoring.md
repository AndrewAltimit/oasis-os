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

# Desktop render size override. By default, PSP-native (480x272) skins
# are upscaled to 1280x720 on desktop and other sizes render as-is;
# set both fields to pin an explicit desktop resolution instead.
# desktop_width = 960
# desktop_height = 544

# Base this skin on a built-in parent: only overrides live in this
# skin's files, everything else comes from the parent (see
# "Skin Inheritance" below).
# inherits = "classic"
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
# Success/positive status color (toasts, badges; default #50C878)
success = "#8CEB32"
# Warning/caution status color (default #FFB432)
warning = "#FFC81E"
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
# LED accent on the vector "data" icon (default "#00C864").
data_led_color = "#00C864"
# Colors cycled for discovered apps without an ICON0 (default: 6-color
# steel-blue/green/gold/plum/indian-red/cornflower cycle).
fallback_colors = ["#4682B4", "#3CB371", "#DAA520"]
# Document-icon emblem anchor: "top" (default, tinted inset block below
# the stripe) or "badge" (solid square overlapping the page's
# bottom-right corner, PSIX-style).
gfx_anchor = "badge"
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

#### ANSI Terminal Palette

Terminal output may contain SGR foreground escape sequences
(`ESC[31m` red .. `ESC[0m` reset — see `docs/terminal-commands.md`).
The 16 slots are derived from the base colors automatically: the six
standard hues (red, yellow, green, cyan, blue, magenta) are tinted
toward the skin — saturation from `primary`, lightness from `output` —
`white`/`bright_white` reuse `output`/`text`, and the blacks are
primary-tinted darks. Override any slot explicitly:

```toml
[palette]
black = "#101010"
red = "#FF5555"
green = "#50FA7B"
yellow = "#F1FA8C"
blue = "#6272A4"
magenta = "#FF79C6"
cyan = "#8BE9FD"
white = "#CCCCCC"
bright_black = "#44475A"
bright_red = "#FF6E6E"
bright_green = "#69FF94"
bright_yellow = "#FFFFA5"
bright_blue = "#D6ACFF"
bright_magenta = "#FF92DF"
bright_cyan = "#A4FFFF"
bright_white = "#FFFFFF"
```

The derived palette only affects SGR-colored output — plain terminal
text keeps using `output`, so existing skins render identically unless
a command emits color.

#### Mouse Cursor

The procedural arrow cursor (desktop/WASM software cursor and
screenshots) can be recolored. These fields extend the same `[cursor]`
table that holds `texture`/`hotspot` (see "Themed software cursor"
under Image Assets); they only apply when no `texture` is set:

```toml
[cursor]
fill = "#FFFFFF"     # arrow interior (default white)
outline = "#000000"  # arrow border (default black)
```

#### Boot Splash

The desktop boot splash (BIOS banner + synthwave horizon) defaults to
the PSIX purple look. Because the skin is resolved before the splash
starts, every phase can be themed:

```toml
[boot]
# Sky gradient, 6 stops top-to-bottom (positions fixed at
# 0.0 / 0.2 / 0.5 / 0.8 / 0.95 / 1.0).
sky_stops = ["#02001A", "#050044", "#150088", "#5500CC", "#AA55FF", "#FFFFFF"]
# Ground gradient, 4 stops top-to-bottom (0.0 / 0.05 / 0.3 / 1.0).
ground_stops = ["#FFFFFF", "#6A00CC", "#150088", "#02001A"]
banner_bg = "#120E24"      # BIOS banner interior
banner_border = "#AA88FF"  # BIOS banner frame
chrome = "#AA88FF"         # status-line prefix + progress bar
text = "#E6DCFF"           # status line text
bios_text = "#CCCCCC"      # BIOS log lines + banner subtitle
```

#### Start Menu Overrides

```toml
[start_menu_overrides]
# Fallback color for items beyond the item_colors list (default "#646464").
item_fallback_color = "#646464"
# Also: panel_bg, panel_border, item_text, highlight_color, button_bg,
#   button_text, item_colors, layout_mode, columns, ... (see
#   StartMenuOverrides in oasis-skin for the full list)
```

#### Background Layer Defaults

```toml
[background_performance]
max_layers = 8
reduced_motion = false
complexity_budget = 200
# Color used by [[background_layers]] entries that omit `color`
# (default "#FFFFFF12").
default_layer_color = "#FFFFFF12"
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

## Named SDI Slots

Every element on screen is a named SDI object. `layout.toml` objects are
skin-owned (any name you like); the component names below are runtime-owned
but *targetable*: a `layout.toml` entry with the same name pre-seeds the
object, and `texture =` / `nine_patch =` assignments on them survive the
component's per-frame updates (that is how shaped bars work).

| Name(s) | Owner | Notes |
|---------|-------|-------|
| `wallpaper` | shell | Full-screen texture, z = -1000 |
| `content_bg` | skin layout | Conventional content backdrop between the bars |
| `bar_top`, `bar_top_line` | status bar | Background + separator; textureable |
| `bar_tab_0..2`, `bar_tab_bg_0..2` | status bar | Top tab labels + pills (see tab pill textures) |
| `bar_battery`, `bar_clock`, `bar_version`, `bar_usb` | status bar | Info text slots |
| `bar_bottom`, `bar_bottom_line` | bottom bar | Background + separator; textureable |
| `bar_btab_0..3`, `bar_bpipe_0..2` | bottom bar | Media category tabs + separators |
| `bar_page_0..3`, `bar_bottom_clock`, `bar_url` | bottom bar | Page dots, clock, URL |
| `icon_*` | dashboard | Per-icon families (body, stripe, fold, label, shadow, …) |
| `cursor_highlight` | dashboard | Free-layout selection highlight |
| `image_layer_*` | shell | `[[background_layers]]` image decals |
| `sm_*` | start menu | Panel, items, footer |
| `taskbar_*`, `start_btn_*` | taskbar | Window buttons, start button |
| `toast_*` | toasts | Notification cards |
| `term_*`, `terminal_bg` | terminal | Output lines, prompt, input bar |
| `mouse_cursor` | shell | Software cursor (top-most) |

Z-ordering: objects without an explicit `z` get an auto-incrementing value
in **sorted name order** of the layout file, so stacking is deterministic;
equal z resolves alphabetically. Free-layout icons render in a fixed band —
selection highlight at z 40, icons at 50, the dragged icon at 60 — above
layout chrome and below the overlay pass (bars, start menu, toasts).

## Geometry Overrides

`[geometry]` in theme.toml adjusts metrics without touching code:

```toml
[geometry]
statusbar_height = 28      # default 24
bottombar_height = 28      # default 24
tab_row_height = 18        # 0 hides the top tab row
icon_width = 26            # dashboard icon size
icon_height = 30
# Document-icon anatomy (icon_style = "document"):
icon_stripe_h = 13         # header band height (default 12)
icon_fold_size = 9         # corner fold size (default 10)
icon_gfx_h = 22            # graphic area height
icon_gfx_pad = 4           # padding around the graphic
icon_label_pad = 4         # gap between icon and label
# Also: tab_w, tab_h, tab_gap, tab_start_x, font_small/body/hint/heading,
#   cursor_pad, toast_* sizing, scrollbar_width, terminal_line_height,
#   page_slide_duration, …
# Widget keyboard-focus rings (oasis-ui, via the derived ui theme):
focus_ring_color = "#3264C8B0"  # default: derived from the accent color
focus_ring_width = 2            # stroke width in px
focus_ring_offset = 2           # gap from the widget edge in px
# These style widget focus indicators only; the dashboard icon cursor
# keeps its own [icon_overrides] focus_glow_* theming.
```

## Typography

`[geometry]`'s `font_*` keys size the *shell* (bars, icons, terminal).
`[typography]` sizes the *widget toolkit* — the font-size ladder and spacing
tokens every `oasis-ui` widget (buttons, lists, tables, modals, the apps built
on them) reads off the derived theme. Before this existed they were hardcoded,
so a skin could restyle every color in the shell but not make its text one
pixel larger.

```toml
[typography]
font_size_xs = 8           # default 8
font_size_sm = 8           # default 8
font_size_md = 10          # body text — default 8
font_size_lg = 18          # default 16
font_size_xl = 16          # default 16
font_size_xxl = 24         # display — default 24

spacing_xs = 2             # default 2
spacing_sm = 4             # default 4
spacing_md = 10            # default 8
spacing_lg = 12            # default 12
spacing_xl = 16            # default 16
```

Every key is optional and unset keys keep the defaults above, so a skin written
before this table renders identically. `skin lint` warns on a zero font size
(renders nothing) and on values past the bitmap font's usable range.

**Per-skin TTF fonts are not supported.** Glyph advances come from
`oasis_types::bitmap_font::glyph_advance()`, which every layout pass in the
shell, the browser, and the terminal calls directly — a skin-supplied face
would need a font-metrics provider threaded through all of them before the
first glyph could be rasterized. The `@font-face` pipeline in `oasis-browser`
(fontdue) already does this for *page* content, and is the model to follow if
the shell ever grows the same capability.

## Accessibility & Density

Three top-level `theme.toml` keys tune size and spacing globally. Each defaults
to a pixel-identity no-op, so an existing skin is unchanged.

```toml
text_scale = 1.0        # accessibility text multiplier (default 1.0)
density = "comfortable" # "compact" | "comfortable" | "spacious" (default comfortable)
```

- **`text_scale`** multiplies the *resolved* font-size ladder (`font_size_*`)
  and the shell text sizes (`font_body`/`font_hint`/`font_heading`). `1.0` is
  identity; the value is clamped to `0.5..=3.0` and each result is at least 1px.
  It is orthogonal to `[geometry] font_scale` (the render-time multiplier) and
  composes multiplicatively with it, and with any `[typography]` overrides.
- **`density`** scales the *spacing* tokens only — `comfortable` is an exact
  integer identity, `compact` is ×0.85, `spacious` is ×1.15 (rounded to the
  nearest pixel). Font sizes are untouched. Unknown values fall back to
  `comfortable`.

## Elevation (semantic shadow ladder)

`[elevation]` overrides the six-step shadow ladder (levels 0–5) used by cards,
dropdowns, modals, tooltips, dashboard icons, the start-menu panel, and toasts.
Any level you omit falls back to the built-in shadow for that level, so an empty
or absent table reproduces today's shadows byte-for-byte. Each level is a list
of layers drawn back-to-front:

```toml
[[elevation.level_2]]
offset_x = 2
offset_y = 4
spread = 2
alpha = 50
color = "#000000"   # optional, defaults to black

[[elevation.level_2]]
offset_x = 2
offset_y = 4
spread = 5
alpha = 20
```

Runtime code resolves a level through `ActiveTheme::resolve_shadow(level)`,
which honors these overrides and otherwise delegates to the built-in
`Shadow::elevation` ladder.

## Widget States

`[widget_states.*]` overrides interactive widget colors (buttons, inputs,
toggles, scrollbars, sliders, menu bars) that are otherwise derived from
the base palette:

```toml
[widget_states.button]
normal_bg = "#3A3A40"
hover_bg = "#55555C"
pressed_bg = "#2A2A2E"
disabled_bg = "#26262A"
disabled_text = "#555555"

[widget_states.input]
bg = "#101014"
border = "#3A3A40"
focus_border = "#F5820F"

[widget_states.toggle]
track_off = "#FFFFFF14"
track_on = "#F5820F"
thumb = "#F0F0E8"

[widget_states.scrollbar]
track = "#FFFFFF0A"
thumb = "#FFFFFF28"
thumb_hover = "#FFFFFF50"

[widget_states.slider]
track = "#101014"   # unfilled track (defaults to the input background)
fill = "#F5820F"    # filled portion (defaults to the accent)
thumb = "#26262A"   # thumb fill (defaults to the surface color)

[widget_states.menu]
bg = "#F0F0F0"                       # menu bar background
border = "#B4B4B4"                   # menu bar bottom border
text = "#1E1E1E"                     # top-level label text
hover_bg = "#316AC5"                 # open-label / hovered-item highlight
hover_text = "#FFFFFF"               # text on the hover highlight
dropdown_bg = "#ECECEC"              # drop-down panel background
dropdown_border_light = "#FFFFFF"    # bezel highlight (top/left)
dropdown_border_dark = "#696969"     # bezel shadow (bottom/right)
item_text = "#141414"                # drop-down item text
disabled_text = "#969696"            # disabled item text
separator = "#AAAAAA"                # drop-down separator line
```

The `menu` slots feed `MenuStyle::from_theme` in `oasis-ui`; their
defaults are the classic Win95 grays the menu bar widget has always
used, so skins that never touch `[widget_states.menu]` render exactly
as before.

### Widget interaction states

The `[widget_states.button]` and `[widget_states.input]` slots above are
routed into every interactive widget through a single resolver,
`WidgetStateColors` (in `oasis-ui/src/states.rs`). A widget collapses its
hover / pressed / disabled booleans into a `WidgetState`
(priority `Disabled > Pressed > Hover > Normal`) and asks the resolver for
the background, text, and border color to paint. Buttons, the shared
button/accent fills used by many widgets, and the interaction-colored
controls all share this one mapping, so overriding `hover_bg`,
`pressed_bg`, `disabled_bg`, or `disabled_text` recolors them uniformly
rather than per-widget.

With no `[widget_states.*]` overrides the resolver reproduces each
widget's previous colors byte-for-byte — it is a pure rename of the
`Theme` fields the widgets already referenced — so default skins render
identically.

Focusable widgets (button, checkbox, radio, toggle, slider, spin box,
dropdown, tab bar, input field) additionally draw a keyboard **focus
ring** when focused, using `FocusStyle::from_theme`. The ring honors the
`[geometry] focus_ring_*` overrides (color / width / offset) and only
appears in the focused state, so resting screenshots are unaffected.

Unknown widget or slot names are flagged by `skin lint`.

## Per-App Palettes (app_themes)

`[app_themes.<app>]` recolors an app's interior without touching the
global palette. Every slot falls back to the app's theme-derived default,
so partial tables are fine.

```toml
[app_themes.terminal]
prompt = "#8CEB32"

[app_themes.file_manager]
selected_bg = "#F5820F"
```

Recognized slots per app:

- **`terminal`** — `bg`, `border`, `output`, `input_bg`, `prompt`,
  `scrollbar_track`, `scrollbar_thumb`
- **`file_manager`** — `bg`, `title_text`, `text`, `dim_text`,
  `selected_bg`, `selected_text`, `divider`, `status_bg`, `status_text`,
  `pane_bg`, `pane_text`, `folder_icon`, `folder_icon_tab`, `file_icon`,
  `file_icon_fold`
- **`settings`** — `bg`, `title_bar_bg`, `title_bar_text`, `text`,
  `selected_text`, `selected_bg`, `selection_accent`, `dim_text`,
  `divider`
- **`tv_guide`** — `bg`, `grid_line`, `header_bg`, `selected_bg`,
  `live_badge`, and the rest of the grid palette (see
  `TvGuideColors` in `oasis-app-tv-guide`)

## Skin Inheritance

`inherits = "<builtin-name>"` in skin.toml bases a skin on a built-in
parent — the recommended authoring pattern for variants. Merging is
fill-only and child-wins: any file/table/field the child sets is used
verbatim; anything it omits comes from the parent. Layout objects and
assets merge by name, collection tables (`app_themes`, `widget_states`,
`gradients`, `animations`) inherit whole-table only when the child
defines none. Maximum depth is 3.

The shipped `psix-tribute` skin demonstrates the pattern: it inherits
`classic` and its files contain only the PSIX-specific overrides.

## Hot Reload (skin-dev)

Build the desktop shell with the `skin-dev` feature to iterate on a skin
live — the running shell polls the active external skin directory once a
second and re-applies the skin when any file changes:

```bash
cargo run -p oasis-app --features skin-dev -- my_skin
# …edit skins/my_skin/theme.toml in another window; saving reloads it.
```

Reloads re-read the TOML and assets from disk (even for names that are
also compiled in as built-ins) and play the skin's entrance transition,
so what you see is exactly what a fresh boot would show.

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
> skin inspect my_skin  # Print a color/contrast/token contact sheet
```

`skin inspect <name>` is a read-only authoring aid: it resolves the skin
(same rules as `skin lint`) and prints a plain-text contact sheet with the
nine resolved base colors, a WCAG AA contrast report for the key
text/background pairs (ratio + PASS/FAIL at the 4.5:1 / 3:1 thresholds),
and a dump of the derived bar / icon / start-menu / app-screen token colors
plus the ANSI palette rows. It works on every backend (no color codes, no
extra deps), so you can eyeball a palette on PSP or WASM too.

### In-app Appearance editor

Settings → **Appearance** is a line-based editor for the nine base colors of
the running skin. Move the cursor to a role and press Confirm to edit it;
Left/Right pick the R/G/B channel and Up/Down step it. Each color row shows
an inline contrast readout against a sensible partner color — foreground
roles are measured against `Background`, while surface roles (`Background`,
`Status Bar`) are measured by the readability of `Text` drawn on them — with
an `AA` / `low` verdict at the WCAG AA threshold, updated live as you step a
channel. The action rows below apply the edited palette as an in-memory
preview, save it as a `custom-<skin>` skin, or derive a Dark / Light /
High-contrast variant.

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
- The lint also runs WCAG AA contrast checks over the pairs that
  actually render text: `text`/`dim_text`/`output`/`prompt`/`error` on
  the background, button text on the button background, selected text
  on the selection highlight, and status/bottom bar text on their bar
  backgrounds. Body text is held to 4.5:1, secondary/large text to
  3:1; translucent colors are composited over their backdrop first.
  These are advisory — deliberately stylized palettes (`corrupted`,
  `win95`) fail some of them by design and still load normally.

Lint your skin whenever a field appears to have no effect — a
misspelled key is the most common cause. All shipped skins are kept
free of schema warnings by a CI test (`all_shipped_skins_lint_clean`).

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
| psix-tribute | PSIX homage | Every theming capability (see below) |

## Showcase: the psix-tribute skin

`skins/psix-tribute/` is the acceptance test for the theming system —
read it alongside this guide as a worked example of every capability:

- **Inheritance**: `inherits = "classic"`, files carry only overrides.
- **Wallpaper**: orange→lime multi-stop gradient with wave arcs
  (`[wallpaper] color_stops`, `angle`, `wave_intensity`).
- **Decal**: drifting, pulsing watermark (`[[background_layers]]`
  `kind = "image"`).
- **Shaped chrome**: alpha-silhouette `bar_top` / `bar_bottom` textures
  assigned from layout.toml, plus `tab_texture_active/inactive` pills.
- **Nine-patch WM chrome**: `titlebar_nine_patch` / `frame_nine_patch`.
- **Vector chrome**: `[[chrome_layers]]` crosshair + corner rings.
- **Desktop icons**: `icon_layout = "free"` with document-style icons,
  tuned `[geometry]` icon anatomy, themed selection stroke, and a
  themed `[cursor]` software pointer.
- **Motion**: `entrance = "assemble"` with `ease_out_cubic`.
- **Track C**: `[widget_states.*]` and `[app_themes.*]` overrides.

Its PNG assets are generated by `scripts/gen-psix-tribute-assets.py`
(deterministic procedural art — no external sources).

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

## Custom Fonts

Skins may replace the built-in 8×8 bitmap font with a TrueType/OpenType
font. Drop the font file in the skin's `assets/` directory and reference
it from `theme.toml`:

```toml
[typography]
font = "assets/skin.ttf"
```

Behavior:

- **Per-character fallback** — any character the font lacks (for
  example the `▲`/`▼` status-bar triangles) keeps its bitmap glyph, so
  UI symbols never render as tofu.
- **Measurement stays exact** — glyph advances are rounded to whole
  pixels once and shared by both text measurement and drawing, so
  layouts cannot drift.
- **Backend support** — the SDL desktop backend rasterizes the font via
  `fontdue`; backends without a TTF rasterizer (WASM, UE5, PSP) ignore
  the field and keep the bitmap font. Omitting `font` keeps the bitmap
  font everywhere, pixel-identical to previous releases.
- **Formats** — raw `.ttf`/`.otf` only (no WOFF/WOFF2 containers).

`skin lint` validates the reference: the file must exist under
`assets/`, parse as a font, and stay within a 512 KB budget (subset
large fonts — an unsubsetted CJK face will blow the budget and slow
every skin swap). Only ship fonts whose license permits redistribution
(OFL or public-domain faces are safe choices).

## [sounds] — UI Sound Themes

Skins can ship short WAV one-shots that play on interface events —
clicks, app launches, window closes, toasts, and cursor navigation.
Add a `[sounds]` table to `theme.toml` and drop the WAV files in the
skin's `assets/` directory alongside its images:

```toml
[sounds]
click = "assets/click.wav"   # button / interactive element click
open = "assets/open.wav"     # app launch / window open
close = "assets/close.wav"   # app / window close
error = "assets/error.wav"   # error toasts
toast = "assets/toast.wav"   # non-error toast notifications
nav = "assets/nav.wav"       # d-pad / cursor move between icons
volume = 0.8                 # master volume for UI sounds, 0.0-1.0 (default 1.0)
```

Every key is optional — omit an event and it stays silent. A skin
without a `[sounds]` table is fully silent (the default for all
built-in skins).

Behavior and constraints:

- **Format**: uncompressed PCM WAV, mono or stereo, 8- or 16-bit, any
  sample rate (resampled to the 48 kHz mix rate at load time).
- **Length**: keep sounds under 2 seconds — these are one-shots, not
  loops. `skin lint` warns about longer files.
- **Polyphony**: up to 8 simultaneous voices; triggering a 9th steals
  the oldest. Sounds mix *over* music/radio playback without
  interrupting it.
- **Rate limiting**: the `nav` sound is throttled (~12 Hz) so holding
  a d-pad direction doesn't machine-gun the sample.
- **Volume**: the `[sounds] volume` master scales all UI sounds, and
  the system audio volume (the same one the `audio vol` command sets,
  0-100) applies on top — a muted system plays no UI sounds.
- **Lifecycle**: sounds load on skin swap-in and are freed on
  swap-out, exactly like image assets. Sound bytes count toward the
  same per-skin asset budget as decoded images.
- **Inheritance**: a child skin with no `[sounds]` table inherits the
  parent's table and WAV assets (`inherits = "..."` in `skin.toml`).

`skin lint` validates the table: referenced files must exist in
`assets/`, must parse as PCM WAV, and `volume` must be within
0.0-1.0.
