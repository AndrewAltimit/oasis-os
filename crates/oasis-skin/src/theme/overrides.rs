//! Override structs for themed UI components.
//!
//! Each struct provides per-element TOML-driven color and geometry overrides
//! for a specific subsystem (WM titlebar, bars, icons, browser, apps, OSK,
//! start menu, wallpaper, geometry, transitions, scrollbar, background layers).

use serde::Deserialize;

/// A nine-patch image reference:
/// `{ image = "assets/btn.png", insets = [4, 4, 4, 4] }`.
///
/// Used on layout objects (`nine_patch = ...`) and WM chrome slots
/// (`[wm_theme] titlebar_nine_patch = ...`).
#[derive(Debug, Clone, Deserialize)]
pub struct NinePatchDef {
    /// Image asset key (e.g. `"assets/btn.png"`).
    pub image: String,
    /// Slice insets in texture pixels: `[left, top, right, bottom]`.
    pub insets: [u16; 4],
}

/// Optional overrides for the window manager theme.
#[derive(Debug, Clone, Deserialize)]
pub struct WmThemeOverrides {
    pub titlebar_height: Option<u32>,
    pub border_width: Option<u32>,
    pub titlebar_active: Option<String>,
    pub titlebar_inactive: Option<String>,
    pub titlebar_text: Option<String>,
    /// Titlebar text color for the focused window. Synonym for
    /// `titlebar_text`; takes precedence when both are set.
    #[serde(default)]
    pub titlebar_text_active: Option<String>,
    /// Titlebar text color for unfocused windows (default: same as active).
    #[serde(default)]
    pub titlebar_text_inactive: Option<String>,
    pub frame_color: Option<String>,
    pub content_bg: Option<String>,
    pub btn_close: Option<String>,
    pub btn_minimize: Option<String>,
    pub btn_maximize: Option<String>,
    pub button_size: Option<u32>,
    pub resize_handle_size: Option<u32>,
    pub titlebar_font_size: Option<u16>,
    // Extended visual properties.
    #[serde(default)]
    pub titlebar_radius: Option<u16>,
    #[serde(default)]
    pub titlebar_gradient: Option<bool>,
    #[serde(default)]
    pub titlebar_gradient_top: Option<String>,
    #[serde(default)]
    pub titlebar_gradient_bottom: Option<String>,
    #[serde(default)]
    pub titlebar_inactive_gradient_top: Option<String>,
    #[serde(default)]
    pub titlebar_inactive_gradient_bottom: Option<String>,
    #[serde(default)]
    pub frame_shadow_level: Option<u8>,
    #[serde(default)]
    pub frame_border_radius: Option<u16>,
    #[serde(default)]
    pub button_radius: Option<u16>,
    // Tier 1
    #[serde(default)]
    pub button_side: Option<String>,
    #[serde(default)]
    pub glyph_close: Option<String>,
    #[serde(default)]
    pub glyph_minimize: Option<String>,
    #[serde(default)]
    pub glyph_maximize: Option<String>,
    #[serde(default)]
    pub title_align: Option<String>,
    // Tier 2
    #[serde(default)]
    pub separator_enabled: Option<bool>,
    #[serde(default)]
    pub separator_color: Option<String>,
    #[serde(default)]
    pub glyph_close_color: Option<String>,
    #[serde(default)]
    pub glyph_minimize_color: Option<String>,
    #[serde(default)]
    pub glyph_maximize_color: Option<String>,
    #[serde(default)]
    pub button_spacing: Option<i32>,
    // Tier 3
    #[serde(default)]
    pub btn_close_hover: Option<String>,
    #[serde(default)]
    pub btn_minimize_hover: Option<String>,
    #[serde(default)]
    pub btn_maximize_hover: Option<String>,
    #[serde(default)]
    pub title_text_shadow: Option<bool>,
    #[serde(default)]
    pub title_text_shadow_color: Option<String>,
    #[serde(default)]
    pub content_stroke_width: Option<u16>,
    #[serde(default)]
    pub content_stroke_color: Option<String>,
    #[serde(default)]
    pub maximize_top_inset: Option<u32>,
    #[serde(default)]
    pub maximize_bottom_inset: Option<u32>,
    #[serde(default)]
    pub modal_overlay_color: Option<String>,
    /// Alpha applied to inactive window frames (default 180).
    #[serde(default)]
    pub inactive_frame_alpha: Option<u8>,
    /// Nine-patch image for window titlebars (active and inactive).
    /// Corners stay fixed while the middle stretches with the window width.
    #[serde(default)]
    pub titlebar_nine_patch: Option<NinePatchDef>,
    /// Nine-patch image for the window frame (behind the content area).
    #[serde(default)]
    pub frame_nine_patch: Option<NinePatchDef>,
}

/// Per-element overrides for status bar and bottom bar colors.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BarOverrides {
    pub bar_bg: Option<String>,
    pub statusbar_bg: Option<String>,
    /// Fallback text color for all bar text elements (battery, version,
    /// clock, URL, USB, pipes, hints, category label). Element-specific
    /// colors below take precedence.
    #[serde(default)]
    pub text_color: Option<String>,
    /// Fallback gradient top color for both bars. The bar-specific
    /// `statusbar_gradient_*` / `bar_gradient_*` fields take precedence.
    #[serde(default)]
    pub gradient_top: Option<String>,
    /// Fallback gradient bottom color for both bars.
    #[serde(default)]
    pub gradient_bottom: Option<String>,
    pub separator_color: Option<String>,
    pub battery_color: Option<String>,
    pub version_color: Option<String>,
    pub clock_color: Option<String>,
    pub url_color: Option<String>,
    pub usb_color: Option<String>,
    pub tab_active_fill: Option<String>,
    pub tab_active_alpha: Option<u8>,
    pub tab_inactive_alpha: Option<u8>,
    /// Image asset drawn as the active top-tab pill (e.g.
    /// `"assets/tab_active.png"`). The bitmap is alpha-blended, so shaped
    /// tab chrome works PSIX-style. Falls back to the pill fill when unset.
    #[serde(default)]
    pub tab_texture_active: Option<String>,
    /// Image asset drawn as inactive top-tab pills.
    #[serde(default)]
    pub tab_texture_inactive: Option<String>,
    pub media_tab_active: Option<String>,
    pub media_tab_inactive: Option<String>,
    pub pipe_color: Option<String>,
    pub r_hint_color: Option<String>,
    pub category_label_color: Option<String>,
    pub page_dot_active: Option<String>,
    pub page_dot_inactive: Option<String>,
    pub statusbar_gradient_top: Option<String>,
    pub statusbar_gradient_bottom: Option<String>,
    pub bar_gradient_top: Option<String>,
    pub bar_gradient_bottom: Option<String>,
    /// Whether text shadow is enabled on bar text elements.
    #[serde(default)]
    pub text_shadow: Option<bool>,
    /// Text shadow color (hex, default: "#00000080").
    #[serde(default)]
    pub text_shadow_color: Option<String>,
    /// Version label text (default: "Version 0.1").
    #[serde(default)]
    pub version_text: Option<String>,
    /// Category label text (default: "OSS").
    #[serde(default)]
    pub category_label: Option<String>,
    /// URL text for bottom bar (default: "HTTP://OASIS.LOCAL").
    #[serde(default)]
    pub url_text: Option<String>,
    /// Taskbar background color.
    #[serde(default)]
    pub taskbar_bg: Option<String>,
    /// Taskbar active button color.
    #[serde(default)]
    pub taskbar_btn_active: Option<String>,
    /// Taskbar inactive button color.
    #[serde(default)]
    pub taskbar_btn_inactive: Option<String>,
    /// Taskbar minimized button color.
    #[serde(default)]
    pub taskbar_btn_minimized: Option<String>,
    /// Taskbar hover button color.
    #[serde(default)]
    pub taskbar_btn_hover: Option<String>,
    /// Taskbar text color.
    #[serde(default)]
    pub taskbar_text_color: Option<String>,
    /// Taskbar separator line color.
    #[serde(default)]
    pub taskbar_separator: Option<String>,
    /// Taskbar active indicator color.
    #[serde(default)]
    pub taskbar_indicator: Option<String>,
}

/// Per-element overrides for dashboard icon rendering.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IconOverrides {
    pub body_color: Option<String>,
    pub fold_color: Option<String>,
    pub outline_color: Option<String>,
    pub shadow_color: Option<String>,
    pub label_color: Option<String>,
    pub cursor_color: Option<String>,
    pub icon_border_radius: Option<u16>,
    pub cursor_border_radius: Option<u16>,
    pub cursor_stroke_width: Option<u16>,
    /// Icon style variant: "document" (default), "card", or "circle".
    #[serde(default)]
    pub icon_style: Option<String>,
    /// Cursor style variant: "stroke" (default), "fill", or "underline".
    #[serde(default)]
    pub cursor_style: Option<String>,
    /// Vector icon preset name (used when `icon_style = "vector"`).
    /// Available presets: "altimit" (default).
    #[serde(default)]
    pub vector_preset: Option<String>,
    /// Enable idle float animation (gentle sine-wave bob) for vector icons.
    #[serde(default)]
    pub vector_idle_float: Option<bool>,
    /// Float animation amplitude in pixels (default 2.0).
    #[serde(default)]
    pub vector_float_amplitude: Option<f32>,
    /// Float animation speed multiplier (default 0.04).
    #[serde(default)]
    pub vector_float_speed: Option<f32>,
    /// Enable spin animation on "the_world" inner element (default true).
    #[serde(default)]
    pub vector_spin_enabled: Option<bool>,
    /// Spin speed in radians per frame (default 0.03).
    #[serde(default)]
    pub vector_spin_speed: Option<f32>,
    /// Enable pulse animation on "audio" inner element (default true).
    #[serde(default)]
    pub vector_pulse_enabled: Option<bool>,
    /// Pulse speed multiplier (default 0.06).
    #[serde(default)]
    pub vector_pulse_speed: Option<f32>,
    /// Enable LED blink on "data" icon (default true).
    #[serde(default)]
    pub vector_blink_enabled: Option<bool>,
    /// LED blink interval in frames (default 45).
    #[serde(default)]
    pub vector_blink_interval: Option<u32>,
    /// Vector icon container shape drawn behind the glyph: "none" (default),
    /// "chip" (filled rounded square), "circle". Used to keep glyphs legible
    /// over busy shader wallpapers.
    #[serde(default)]
    pub icon_container: Option<String>,
    /// Pixels of padding between the container edge and the glyph (default 3).
    #[serde(default)]
    pub icon_container_padding: Option<u16>,
    /// Entrance animation style: "none", "fade_in", "scale_up", "slide_up".
    #[serde(default)]
    pub entrance_style: Option<String>,
    /// Entrance animation duration in milliseconds (default 200).
    #[serde(default)]
    pub entrance_duration_ms: Option<u32>,
    /// Per-icon entrance stagger delay in milliseconds (default 30).
    #[serde(default)]
    pub entrance_stagger_ms: Option<u32>,
    /// Focus scale factor (default 1.0; 1.15 = 15% grow on focus).
    #[serde(default)]
    pub focus_scale: Option<f32>,
    /// Whether a glow ring is drawn around the focused icon.
    #[serde(default)]
    pub focus_glow: Option<bool>,
    /// Focus glow color (hex, default: accent at 40% alpha).
    #[serde(default)]
    pub focus_glow_color: Option<String>,
}

/// Software mouse cursor theming (`[cursor]` in theme.toml).
///
/// Only used when the skin enables `features.software_cursor`. Without a
/// `texture`, the built-in procedural arrow cursor is drawn.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CursorConfig {
    /// Asset path for the cursor bitmap (e.g. `"assets/cursor.png"`).
    #[serde(default)]
    pub texture: Option<String>,
    /// Click hotspot `[x, y]` within the cursor image (default `[0, 0]`,
    /// the top-left corner — correct for arrow cursors).
    #[serde(default)]
    pub hotspot: Option<[i32; 2]>,
}

/// Wallpaper generation configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WallpaperConfig {
    /// Style: "gradient" (default), "solid", "none", "grid", "noise",
    /// "scanlines", "dots", or "image".
    pub style: Option<String>,
    /// Image asset for `style = "image"` (e.g. `"assets/wall.png"`).
    /// Composited over a solid base from the first color stop, so
    /// transparent regions show through.
    #[serde(default)]
    pub source: Option<String>,
    /// Image fit mode: "cover" (default), "contain", "stretch", or "tile".
    #[serde(default)]
    pub fit: Option<String>,
    /// Hex color stops for gradient wallpaper.
    pub color_stops: Option<Vec<String>>,
    /// Whether PSIX arc ripple waves are enabled (default true).
    pub wave_enabled: Option<bool>,
    /// Wave intensity 0.0-1.0 (default 1.0).
    pub wave_intensity: Option<f32>,
    /// Gradient angle in degrees: 0=horizontal, 90=vertical (default 0).
    pub angle: Option<f32>,
    /// Grid/dot spacing in pixels (default 16).
    pub grid_spacing: Option<u32>,
    /// Hex color for grid lines/dots (default: lighten(bg, 0.08)).
    pub grid_color: Option<String>,
    /// Noise intensity 0.0-1.0 for "noise" style (default 0.3).
    pub noise_intensity: Option<f32>,
    /// Whether the wallpaper should animate (default false).
    pub animated: Option<bool>,
}

/// Typography scale for the widget toolkit: the font-size ladder and spacing
/// tokens every `oasis-ui` widget reads off the derived `Theme`.
///
/// These were hardcoded in the theme derivation, which meant a skin could
/// restyle every color in the shell but not make its text one pixel larger.
/// Unset fields keep the historical defaults (an 8 px body size, a 2/4/8/12/16
/// spacing ramp), so an existing skin renders identically.
///
/// ```toml
/// [typography]
/// font_size_md = 10
/// font_size_lg = 18
/// spacing_md = 10
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TypographyOverrides {
    /// Skin-relative path to a TTF/OTF font file (e.g. `"assets/skin.ttf"`).
    ///
    /// When set, backends with a TTF rasterizer render all shell text with
    /// this font instead of the built-in bitmap font; backends without one
    /// (and any character the font lacks) keep the bitmap glyphs.
    #[serde(default)]
    pub font: Option<String>,
    /// Extra-small font size (default 8).
    #[serde(default)]
    pub font_size_xs: Option<u16>,
    /// Small font size (default 8).
    #[serde(default)]
    pub font_size_sm: Option<u16>,
    /// Medium/body font size (default 8).
    #[serde(default)]
    pub font_size_md: Option<u16>,
    /// Large font size (default 16).
    #[serde(default)]
    pub font_size_lg: Option<u16>,
    /// Extra-large font size (default 16).
    #[serde(default)]
    pub font_size_xl: Option<u16>,
    /// Display font size (default 24).
    #[serde(default)]
    pub font_size_xxl: Option<u16>,
    /// Extra-small spacing step (default 2).
    #[serde(default)]
    pub spacing_xs: Option<u16>,
    /// Small spacing step (default 4).
    #[serde(default)]
    pub spacing_sm: Option<u16>,
    /// Medium spacing step (default 8).
    #[serde(default)]
    pub spacing_md: Option<u16>,
    /// Large spacing step (default 12).
    #[serde(default)]
    pub spacing_lg: Option<u16>,
    /// Extra-large spacing step (default 16).
    #[serde(default)]
    pub spacing_xl: Option<u16>,
}

/// Geometry overrides for bar heights, icon sizes, and font sizes.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GeometryOverrides {
    pub statusbar_height: Option<u32>,
    pub bottombar_height: Option<u32>,
    /// Taskbar height in pixels (default 20, 0 = disabled).
    #[serde(default)]
    pub taskbar_height: Option<u32>,
    pub tab_row_height: Option<u32>,
    pub icon_width: Option<u32>,
    pub icon_height: Option<u32>,
    pub font_small: Option<u16>,
    /// Dashboard grid horizontal padding (default 16).
    #[serde(default)]
    pub grid_padding_x: Option<u16>,
    /// Dashboard grid vertical padding (default 6).
    #[serde(default)]
    pub grid_padding_y: Option<u16>,
    /// Shadow level for dashboard icons (default 1).
    #[serde(default)]
    pub icon_shadow_level: Option<u8>,
    /// Terminal background border radius (default 4).
    #[serde(default)]
    pub terminal_border_radius: Option<u16>,
    /// Scrollbar width in pixels (default 6).
    #[serde(default)]
    pub scrollbar_width: Option<u32>,
    /// Scrollbar corner radius (default 3).
    #[serde(default)]
    pub scrollbar_border_radius: Option<u16>,
    /// Terminal line height in pixels (default 16).
    #[serde(default)]
    pub terminal_line_height: Option<u32>,
    /// Top tab width (default: proportional to screen).
    #[serde(default)]
    pub tab_w: Option<u32>,
    /// Top tab height (default: proportional to screen).
    #[serde(default)]
    pub tab_h: Option<u32>,
    /// Gap between top tabs (default: proportional to screen).
    #[serde(default)]
    pub tab_gap: Option<u32>,
    /// X offset where top tabs start (default: proportional to screen).
    #[serde(default)]
    pub tab_start_x: Option<i32>,
    /// Document-icon stripe (header band) height (default 12).
    #[serde(default)]
    pub icon_stripe_h: Option<u32>,
    /// Document-icon corner fold size (default 10).
    #[serde(default)]
    pub icon_fold_size: Option<u32>,
    /// Icon graphic area height (default 22).
    #[serde(default)]
    pub icon_gfx_h: Option<u32>,
    /// Padding around the icon graphic area (default 4).
    #[serde(default)]
    pub icon_gfx_pad: Option<u32>,
    /// Gap between icon graphic and its label (default 4).
    #[serde(default)]
    pub icon_label_pad: Option<i32>,
    /// Body text font size (default 12).
    #[serde(default)]
    pub font_body: Option<u16>,
    /// Hint/metadata font size (default 10).
    #[serde(default)]
    pub font_hint: Option<u16>,
    /// Heading font size (default 14).
    #[serde(default)]
    pub font_heading: Option<u16>,
    /// Terminal cursor blink rate in frames (default 30; 0 = no blink).
    #[serde(default)]
    pub cursor_blink_rate: Option<u32>,
    /// Cursor lerp speed (0.0-1.0, default 0.18).
    #[serde(default)]
    pub cursor_lerp_speed: Option<f32>,
    /// Page slide animation duration in frames (default 12).
    #[serde(default)]
    pub page_slide_duration: Option<u32>,
    /// Start menu open/close animation speed (default 0.15).
    #[serde(default)]
    pub start_menu_anim_speed: Option<f32>,
    /// Toast fade in/out duration in frames (default 10).
    #[serde(default)]
    pub toast_fade_frames: Option<u32>,
    /// Press flash duration in frames (default 6; 0 = disabled).
    #[serde(default)]
    pub press_flash_duration: Option<u32>,
    /// Focus ring color (hex) for widget keyboard-focus indicators.
    ///
    /// Applies to `oasis-ui` widget focus rings (via
    /// `ui::Theme::focus_ring_color` / `FocusStyle::from_theme`), not
    /// to the dashboard icon cursor/glow, which keeps its own
    /// `[icon_overrides]` `focus_glow_*` theming. When unset, focus
    /// rings derive from the accent color.
    #[serde(default)]
    pub focus_ring_color: Option<String>,
    /// Focus ring stroke width in pixels (widget focus only).
    #[serde(default)]
    pub focus_ring_width: Option<u16>,
    /// Focus ring offset from the widget edge in pixels
    /// (widget focus only).
    #[serde(default)]
    pub focus_ring_offset: Option<i32>,
    /// Cursor highlight padding around icon (default 3).
    #[serde(default)]
    pub cursor_pad: Option<i32>,
    /// Press flash lighten factor 0.0-1.0 (default 0.25).
    #[serde(default)]
    pub press_flash_lighten: Option<f32>,
    /// App selection lerp speed 0.0-1.0 (default 0.25).
    #[serde(default)]
    pub app_selection_lerp_speed: Option<f32>,
    /// Page dot lerp speed 0.0-1.0 (default 0.2).
    #[serde(default)]
    pub page_dot_lerp_speed: Option<f32>,
    /// Toast margin from screen edge in pixels (default 8).
    #[serde(default)]
    pub toast_margin: Option<i32>,
    /// Toast height in pixels (default 24).
    #[serde(default)]
    pub toast_height: Option<u32>,
    /// Toast width as fraction of screen width (default 0.333).
    #[serde(default)]
    pub toast_width_fraction: Option<f32>,
    /// Gap between stacked toasts in pixels (default 4).
    #[serde(default)]
    pub toast_gap: Option<i32>,
    /// Whether toasts slide in from the right (default true).
    #[serde(default)]
    pub toast_slide_in: Option<bool>,
    /// System-wide font size scale factor (0.5-3.0, default 1.0).
    ///
    /// Applied as a multiplier wherever font sizes are used for rendering.
    /// Values below 0.5 are clamped to 0.5; values above 3.0 are clamped
    /// to 3.0.
    #[serde(default)]
    pub font_scale: Option<f32>,
}

/// Per-element overrides for the start menu popup and button.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StartMenuOverrides {
    pub panel_bg: Option<String>,
    pub panel_gradient_top: Option<String>,
    pub panel_gradient_bottom: Option<String>,
    pub panel_border: Option<String>,
    pub item_text: Option<String>,
    pub item_text_active: Option<String>,
    pub highlight_color: Option<String>,
    pub button_bg: Option<String>,
    pub button_text: Option<String>,
    pub panel_border_radius: Option<u16>,
    pub panel_shadow_level: Option<u8>,
    // Tier 1: Layout
    #[serde(default)]
    pub layout_mode: Option<String>,
    #[serde(default)]
    pub button_label: Option<String>,
    #[serde(default)]
    pub button_width: Option<u32>,
    #[serde(default)]
    pub button_height: Option<u32>,
    #[serde(default)]
    pub button_shape: Option<String>,
    #[serde(default)]
    pub panel_width: Option<u32>,
    #[serde(default)]
    pub columns: Option<usize>,
    // Tier 2: Header/footer
    #[serde(default)]
    pub header_text: Option<String>,
    #[serde(default)]
    pub header_bg: Option<String>,
    #[serde(default)]
    pub header_text_color: Option<String>,
    #[serde(default)]
    pub header_height: Option<u32>,
    #[serde(default)]
    pub footer_enabled: Option<bool>,
    #[serde(default)]
    pub footer_bg: Option<String>,
    #[serde(default)]
    pub footer_text_color: Option<String>,
    #[serde(default)]
    pub footer_height: Option<u32>,
    // Tier 2: Item geometry + button gradient
    #[serde(default)]
    pub item_icon_size: Option<u32>,
    #[serde(default)]
    pub item_row_height: Option<i32>,
    #[serde(default)]
    pub button_gradient: Option<bool>,
    #[serde(default)]
    pub button_gradient_top: Option<String>,
    #[serde(default)]
    pub button_gradient_bottom: Option<String>,
    /// Hex color array for category icon placeholders (default: derived from primary).
    #[serde(default)]
    pub item_colors: Option<Vec<String>>,
    /// Footer text (default: "Log Off  Shut Down").
    #[serde(default)]
    pub footer_text: Option<String>,
    /// Inner padding for the menu panel (default 8).
    #[serde(default)]
    pub pad_inner: Option<i32>,
    /// Start button X position on the bottom bar (default 4).
    #[serde(default)]
    pub button_x: Option<i32>,
    /// Menu panel X position (default 2).
    #[serde(default)]
    pub panel_x: Option<i32>,
    /// Whether item separators are drawn between rows (default false).
    #[serde(default)]
    pub item_separator: Option<bool>,
    /// Item separator color (hex, default: derived from panel border).
    #[serde(default)]
    pub item_separator_color: Option<String>,
}

/// Per-element color overrides for app screens (File Manager, Photo Viewer, etc.).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AppOverrides {
    /// App screen background color.
    pub app_bg: Option<String>,
    /// Divider/separator line color.
    pub divider_color: Option<String>,
    /// Selected/highlighted text color.
    pub selected_text: Option<String>,
    /// Normal content text color.
    pub text_color: Option<String>,
    /// Dimmed hint text color.
    pub dim_text: Option<String>,
    /// Title bar background color.
    pub title_bar_bg: Option<String>,
    /// Terminal output text color (overrides skin.output).
    #[serde(default)]
    pub terminal_output_color: Option<String>,
    /// Terminal prompt text color (overrides skin.prompt).
    #[serde(default)]
    pub terminal_prompt_color: Option<String>,
    /// Input bar border radius (overrides terminal_border_radius).
    #[serde(default)]
    pub input_border_radius: Option<u16>,
    /// Title bar text color.
    #[serde(default)]
    pub title_bar_text: Option<String>,
    /// Title bar height in pixels.
    #[serde(default)]
    pub title_bar_height: Option<u32>,
    /// Selection highlight border radius (default 2).
    #[serde(default)]
    pub selection_border_radius: Option<u16>,
    /// Selection left-accent color (hex, default: with_alpha(primary, 128)).
    #[serde(default)]
    pub selection_accent_color: Option<String>,
    /// Title bar gradient top color (hex).
    #[serde(default)]
    pub title_bar_gradient_top: Option<String>,
    /// Title bar gradient bottom color (hex).
    #[serde(default)]
    pub title_bar_gradient_bottom: Option<String>,
    /// Whether text shadow is enabled on app title bar text.
    #[serde(default)]
    pub title_bar_text_shadow: Option<bool>,
    /// App title bar text shadow color (hex, default: "#00000080").
    #[serde(default)]
    pub title_bar_text_shadow_color: Option<String>,
}

/// Per-element color overrides for the on-screen keyboard.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OskOverrides {
    /// Key background color.
    pub key_bg: Option<String>,
    /// Key text color.
    pub key_text: Option<String>,
    /// Focused key highlight color.
    pub key_focus: Option<String>,
    /// Active (selected) key background color.
    pub key_active: Option<String>,
    /// Inactive/secondary text color (mode indicator, buffer).
    pub key_dim_text: Option<String>,
}

/// Overrides for transition effects.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TransitionOverrides {
    /// Fade overlay color (hex). Default: derived from background.
    pub fade_color: Option<String>,
    /// Fade transition duration in milliseconds (converted to frames at
    /// 60 fps). `features.transition_fade_frames` takes precedence.
    #[serde(default)]
    pub fade_ms: Option<u32>,
    /// Slide transition duration in milliseconds (converted to frames at
    /// 60 fps). `features.transition_slide_frames` takes precedence.
    #[serde(default)]
    pub slide_ms: Option<u32>,
    /// Entrance played on boot and skin swap: "fade" (default), "assemble"
    /// (PSIX-style: bars slide in while an iris shrinks from center), or
    /// "none". `assemble` honors `background_performance.reduced_motion`
    /// by falling back to a fade.
    #[serde(default)]
    pub entrance: Option<String>,
    /// Entrance duration in milliseconds (default 750 for "assemble";
    /// "fade" keeps using `fade_ms` / `transition_fade_frames`).
    #[serde(default)]
    pub entrance_ms: Option<u32>,
    /// Dashboard page change style: "slide" (default) or "fade".
    #[serde(default)]
    pub page_style: Option<String>,
    /// Easing curve name applied to entrance transitions (see
    /// [`resolve_easing`] for supported names). Default: the effect's
    /// built-in curve.
    #[serde(default)]
    pub easing: Option<String>,
}

/// Per-element overrides for scrollbar appearance.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScrollbarOverrides {
    pub track_color: Option<String>,
    pub thumb_color: Option<String>,
    pub thumb_hover_color: Option<String>,
    pub width: Option<u32>,
}

/// Per-element overrides for browser chrome colors.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BrowserOverrides {
    pub chrome_bg: Option<String>,
    pub chrome_text: Option<String>,
    pub chrome_button_bg: Option<String>,
    pub url_bar_bg: Option<String>,
    pub url_bar_text: Option<String>,
    pub status_bar_bg: Option<String>,
    pub status_bar_text: Option<String>,
    pub link_color: Option<String>,
}

/// Configuration for a single background decoration layer.
///
/// Deserialized from `[[background_layers]]` TOML sections.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BackgroundLayerConfig {
    /// Layer kind: "grid", "dot_grid", "wireframe_sphere", "radar_sweep",
    /// "concentric_rings", "glass_shard", "scanlines", "eq_bars",
    /// "crosshair", "floating_polygons", "pulsing_core", "image".
    pub kind: String,
    /// Image asset for `kind = "image"` (e.g. `"assets/logo.png"`) --
    /// PSIX-style watermark decals positioned by the `position` table and
    /// animated (drift/pulse) by the `animation` table.
    #[serde(default)]
    pub source: Option<String>,
    /// Base opacity 0-255 for `kind = "image"` layers (default 255).
    #[serde(default)]
    pub alpha: Option<u8>,
    /// Element color (hex, default "#FFFFFF12").
    #[serde(default)]
    pub color: Option<String>,
    /// Whether this layer is active (default true).
    #[serde(default)]
    pub enabled: Option<bool>,
    // -- Kind-specific parameters --
    /// Grid/dot/scanline spacing in pixels.
    #[serde(default)]
    pub spacing: Option<u32>,
    /// Radius for circles, spheres, radar sweeps, cores.
    #[serde(default)]
    pub radius: Option<u16>,
    /// Dot radius for dot_grid.
    #[serde(default)]
    pub dot_radius: Option<u16>,
    /// Sweep angle for radar sweep (radians, default 0.8).
    #[serde(default)]
    pub sweep_angle: Option<f32>,
    /// Number of rings/bars/polygons.
    #[serde(default)]
    pub count: Option<u8>,
    /// Stroke width for rings.
    #[serde(default)]
    pub stroke_width: Option<u16>,
    /// Polygon sides for floating_polygons.
    #[serde(default)]
    pub sides: Option<u8>,
    /// Bar width for eq_bars.
    #[serde(default)]
    pub bar_width: Option<u32>,
    /// Max height for eq_bars.
    #[serde(default)]
    pub max_height: Option<u32>,
    /// Crosshair size.
    #[serde(default)]
    pub size: Option<u16>,
    /// Wave frequency (cycles across screen width).
    #[serde(default)]
    pub frequency: Option<f32>,
    /// Glass shard points (normalized 0..1).
    #[serde(default)]
    pub points: Option<Vec<[f32; 2]>>,
    // -- Shader --
    /// Shader name (for `kind = "shader"`).
    #[serde(default)]
    pub shader: Option<String>,
    /// Shader-specific float parameters.
    #[serde(default)]
    pub shader_params: Option<toml::Table>,
    // -- Position --
    /// Positioning sub-table.
    #[serde(default)]
    pub position: Option<LayerPositionConfig>,
    // -- Animation --
    /// Animation sub-table.
    #[serde(default)]
    pub animation: Option<LayerAnimationConfig>,
}

/// Position config for a background layer.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LayerPositionConfig {
    /// Anchor: "top_left", "top_center", "top_right", "center_left",
    /// "center", "center_right", "bottom_left", "bottom_center", "bottom_right".
    #[serde(default)]
    pub anchor: Option<String>,
    /// Horizontal offset as fraction of screen width.
    #[serde(default)]
    pub offset_x: Option<f32>,
    /// Vertical offset as fraction of screen height.
    #[serde(default)]
    pub offset_y: Option<f32>,
}

/// Animation config for a background layer.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LayerAnimationConfig {
    /// Rotation speed in radians per second.
    #[serde(default)]
    pub rotate_speed: Option<f32>,
    /// Pulse frequency in Hz.
    #[serde(default)]
    pub pulse_speed: Option<f32>,
    /// Minimum alpha when pulsing (0..1).
    #[serde(default)]
    pub pulse_min_alpha: Option<f32>,
    /// Horizontal drift in pixels per second.
    #[serde(default)]
    pub drift_x: Option<f32>,
    /// Vertical drift in pixels per second.
    #[serde(default)]
    pub drift_y: Option<f32>,
    /// Phase offset for staggering instances.
    #[serde(default)]
    pub phase_offset: Option<f32>,
}

/// Performance settings for background layers.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BackgroundPerformanceConfig {
    /// Maximum number of layers to render (default 8).
    #[serde(default)]
    pub max_layers: Option<u8>,
    /// Whether to disable animations (default false).
    #[serde(default)]
    pub reduced_motion: Option<bool>,
    /// Maximum number of VectorOps to emit (default 200).
    #[serde(default)]
    pub complexity_budget: Option<u32>,
}

/// A reusable gradient preset (two-color linear gradient).
#[derive(Debug, Clone, Deserialize)]
pub struct GradientPreset {
    /// Start color (hex).
    pub from: String,
    /// End color (hex).
    pub to: String,
}

/// A named animation timing preset.
#[derive(Debug, Clone, Deserialize)]
pub struct AnimationPreset {
    /// Duration in milliseconds.
    pub duration_ms: u32,
    /// Easing function name.
    ///
    /// Supported: "linear", "ease_in_quad", "ease_out_quad",
    /// "ease_in_out_quad", "ease_in_cubic", "ease_out_cubic",
    /// "ease_in_out_cubic".
    #[serde(default = "default_easing")]
    pub easing: String,
}

fn default_easing() -> String {
    "linear".to_string()
}

/// Resolve an easing function name to a function pointer.
///
/// Supported names: `"linear"`, `"ease_in_quad"`, `"ease_out_quad"`,
/// `"ease_in_out_quad"`, `"ease_out_cubic"`, `"ease_in_out_cubic"`,
/// `"ease_out_elastic"`, `"ease_out_bounce"`.
///
/// Returns `linear` for unknown names.
pub fn resolve_easing(name: &str) -> fn(f32) -> f32 {
    use oasis_ui::animation::easing;
    match name {
        "linear" => easing::linear,
        "ease_in_quad" => easing::ease_in_quad,
        "ease_out_quad" => easing::ease_out_quad,
        "ease_in_out_quad" => easing::ease_in_out_quad,
        "ease_out_cubic" => easing::ease_out_cubic,
        "ease_in_out_cubic" => easing::ease_in_out_cubic,
        "ease_out_elastic" => easing::ease_out_elastic,
        "ease_out_bounce" => easing::ease_out_bounce,
        _ => easing::linear,
    }
}
