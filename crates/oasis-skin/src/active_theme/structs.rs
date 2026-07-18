//! Sub-struct definitions for the theme system.
//!
//! Each struct represents a focused slice of the overall UI theme.

use oasis_types::backend::Color;

/// 16-color ANSI terminal palette.
///
/// Slots 0-7 map to SGR foreground codes 30-37 (black, red, green,
/// yellow, blue, magenta, cyan, white); slots 8-15 map to 90-97
/// (bright variants). Derived from the skin's base colors unless a
/// `[palette]` table overrides individual slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnsiPalette {
    /// The 16 slot colors in SGR order.
    pub colors: [Color; 16],
}

impl AnsiPalette {
    /// Slot names in SGR order (used for TOML keys and validation).
    pub const SLOT_NAMES: [&'static str; 16] = [
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "white",
        "bright_black",
        "bright_red",
        "bright_green",
        "bright_yellow",
        "bright_blue",
        "bright_magenta",
        "bright_cyan",
        "bright_white",
    ];

    /// Return the color for a palette slot (0-15). Out-of-range indices
    /// wrap into the table.
    pub fn color(&self, idx: usize) -> Color {
        self.colors[idx & 15]
    }

    /// Map an SGR foreground code (30-37, 90-97) to a palette color.
    /// Returns `None` for codes outside those ranges.
    pub fn from_sgr_code(&self, code: u8) -> Option<Color> {
        match code {
            30..=37 => Some(self.colors[(code - 30) as usize]),
            90..=97 => Some(self.colors[(code - 90 + 8) as usize]),
            _ => None,
        }
    }
}

/// Status bar, bottom bar, tab pills, and page dot theme.
#[derive(Debug, Clone)]
pub struct BarTheme {
    /// Status bar background.
    pub statusbar_bg: Color,
    /// Bottom bar background.
    pub bg: Color,
    /// Separator line color.
    pub separator_color: Color,
    /// Battery/power text color.
    pub battery_color: Color,
    /// Version label color.
    pub version_color: Color,
    /// Clock text color.
    pub clock_color: Color,
    /// URL label color.
    pub url_color: Color,
    /// USB indicator color.
    pub usb_color: Color,
    /// Active tab fill color.
    pub tab_active_fill: Color,
    /// Inactive tab fill color.
    pub tab_inactive_fill: Color,
    /// Active tab border alpha.
    pub tab_active_alpha: u8,
    /// Inactive tab border alpha.
    pub tab_inactive_alpha: u8,
    /// Active media tab text color.
    pub media_tab_active: Color,
    /// Inactive media tab text color.
    pub media_tab_inactive: Color,
    /// Active top-tab-row text color (defaults to `media_tab_active`).
    pub tab_text_active: Color,
    /// Inactive top-tab-row text color (defaults to `media_tab_inactive`).
    pub tab_text_inactive: Color,
    /// Pipe separator color.
    pub pipe_color: Color,
    /// R-shoulder hint color.
    pub r_hint_color: Color,
    /// Category label color.
    pub category_label_color: Color,
    /// Active page dot color.
    pub page_dot_active: Color,
    /// Inactive page dot color.
    pub page_dot_inactive: Color,
    /// Status bar gradient top color (None = flat fill).
    pub statusbar_gradient_top: Option<Color>,
    /// Status bar gradient bottom color.
    pub statusbar_gradient_bottom: Option<Color>,
    /// Bottom bar gradient top color (None = flat fill).
    pub gradient_top: Option<Color>,
    /// Bottom bar gradient bottom color.
    pub gradient_bottom: Option<Color>,
    /// Whether bar text elements have drop shadows.
    pub text_shadow: bool,
    /// Bar text shadow color.
    pub text_shadow_color: Color,
    /// Version label text for status bar.
    pub version_text: String,
    /// Category label text for status bar.
    pub category_label: String,
    /// URL text for bottom bar.
    pub url_text: String,
    /// Active tab pill stroke color.
    pub tab_active_stroke: Color,
    /// Inactive tab pill stroke color.
    pub tab_inactive_stroke: Color,
    /// Asset key for the active top-tab pill texture (None = pill fill).
    pub tab_texture_active: Option<String>,
    /// Asset key for inactive top-tab pill textures (None = pill fill).
    pub tab_texture_inactive: Option<String>,
    /// Media-dock transport button fill (`bottombar_style = "media_dock"`).
    pub dock_button_fill: Color,
    /// Media-dock transport glyph (triangle/rect) color.
    pub dock_button_glyph: Color,
    /// Media-dock progress track (background) color.
    pub dock_progress_track: Color,
    /// Media-dock progress fill (foreground) color.
    pub dock_progress_fill: Color,
    /// Media-dock volume track (background) color.
    pub dock_vol_track: Color,
    /// Media-dock volume fill (foreground) color.
    pub dock_vol_fill: Color,
}

/// Dashboard icon rendering and cursor highlight theme.
#[derive(Debug, Clone)]
pub struct IconTheme {
    /// Document body color (white paper).
    pub body_color: Color,
    /// Folded corner color.
    pub fold_color: Color,
    /// Icon outline color.
    pub outline_color: Color,
    /// Icon shadow color.
    pub shadow_color: Color,
    /// Icon label text color.
    pub label_color: Color,
    /// Icon label text shadow color (None = no shadow).
    pub label_shadow: Option<Color>,
    /// Cursor highlight stroke color.
    pub cursor_color: Color,
    /// Icon card border radius (pixels).
    pub border_radius: u16,
    /// Cursor highlight border radius (pixels).
    pub cursor_border_radius: u16,
    /// Cursor highlight stroke width (pixels).
    pub cursor_stroke_width: u16,
    /// Icon style variant: "document" (default), "card", "circle", or "vector".
    pub style: String,
    /// Cursor style variant: "stroke" (default), "fill", or "underline".
    pub cursor_style: String,
    /// Document-icon emblem anchor: "top" (default, inset block below the
    /// stripe) or "badge" (overlapping the bottom-right corner, PSIX-style).
    pub gfx_anchor: String,
    /// Dashboard icon shadow level (default 1).
    pub shadow_level: u8,
    /// Vector icon preset name (used when `style = "vector"`).
    /// Available presets: "altimit" (default).
    pub vector_preset: String,
    /// Enable idle float animation for vector icons.
    pub idle_float: bool,
    /// Float amplitude in pixels.
    pub float_amplitude: f32,
    /// Float speed (radians per frame).
    pub float_speed: f32,
    /// Enable spin on "the_world" inner element.
    pub spin_enabled: bool,
    /// Spin speed (radians per frame).
    pub spin_speed: f32,
    /// Enable pulse on "audio" inner element.
    pub pulse_enabled: bool,
    /// Pulse speed multiplier.
    pub pulse_speed: f32,
    /// Enable LED blink on "data" icon.
    pub blink_enabled: bool,
    /// LED blink interval in frames.
    pub blink_interval: u32,
    /// Container shape drawn behind vector glyphs: "none", "chip", "circle".
    /// When non-"none", the glyph sits inside a filled backdrop for legibility
    /// over shader wallpapers.
    pub container_style: String,
    /// Padding (px) between the container edge and the glyph bounding box.
    pub container_padding: u16,
    /// LED accent color on the vector "data" icon.
    pub data_led_color: Color,
    /// Fallback colors cycled for discovered apps without an ICON0.
    pub fallback_colors: Vec<Color>,
}

/// Start button and popup panel theme.
#[derive(Debug, Clone)]
pub struct StartMenuTheme {
    /// Start menu panel background.
    pub panel_bg: Color,
    /// Start menu panel gradient top (None = flat fill).
    pub panel_gradient_top: Option<Color>,
    /// Start menu panel gradient bottom.
    pub panel_gradient_bottom: Option<Color>,
    /// Start menu panel border color.
    pub panel_border: Color,
    /// Start menu item text color.
    pub item_text: Color,
    /// Start menu active/selected item text color.
    pub item_text_active: Color,
    /// Start menu selection highlight color.
    pub highlight_color: Color,
    /// Start button background color.
    pub button_bg: Color,
    /// Start button text color.
    pub button_text: Color,
    /// Start menu panel border radius.
    pub panel_border_radius: u16,
    /// Start menu panel shadow level.
    pub panel_shadow_level: u8,
    /// Layout mode: "grid" or "list".
    pub layout_mode: String,
    /// Start button label text.
    pub button_label: String,
    /// Start button width.
    pub button_width: u32,
    /// Start button height.
    pub button_height: u32,
    /// Start button shape: "pill" or "rect".
    pub button_shape: String,
    /// Start menu panel width.
    pub panel_width: u32,
    /// Number of columns in the menu grid.
    pub columns: usize,
    /// Start button gradient top color (None = flat fill).
    pub button_gradient_top: Option<Color>,
    /// Start button gradient bottom color.
    pub button_gradient_bottom: Option<Color>,
    /// Header text (None = no header).
    pub header_text: Option<String>,
    /// Header background color.
    pub header_bg: Color,
    /// Header text color.
    pub header_text_color: Color,
    /// Header height.
    pub header_height: u32,
    /// Whether footer is enabled.
    pub footer_enabled: bool,
    /// Footer background color.
    pub footer_bg: Color,
    /// Footer text color.
    pub footer_text_color: Color,
    /// Footer height.
    pub footer_height: u32,
    /// Item icon size.
    pub item_icon_size: u32,
    /// Item row height.
    pub item_row_height: i32,
    /// Start menu item icon colors (6 colors derived from primary).
    pub item_colors: Vec<Color>,
    /// Inner padding for start menu panel.
    pub pad_inner: i32,
    /// Start menu footer text.
    pub footer_text: String,
    /// Start button X position on the bottom bar (default 4).
    pub button_x: i32,
    /// Menu panel X position (default 2).
    pub panel_x: i32,
    /// Whether item separators are drawn between start menu rows.
    pub item_separator: bool,
    /// Item separator color.
    pub item_separator_color: Color,
    /// Fallback color for items beyond the `item_colors` list.
    pub item_fallback_color: Color,
}

/// App content area, title bar, terminal, and selection theme.
#[derive(Debug, Clone)]
pub struct AppScreenTheme {
    /// App screen background color.
    pub bg: Color,
    /// App screen divider/separator color.
    pub divider: Color,
    /// App screen selected text color.
    pub selected_text: Color,
    /// App screen normal text color.
    pub text: Color,
    /// App screen dim/hint text color.
    pub dim_text: Color,
    /// App screen title bar background color.
    pub title_bar_bg: Color,
    /// App screen title bar text color.
    pub title_bar_text: Color,
    /// App screen title bar height.
    pub title_bar_height: u32,
    /// Terminal output text color.
    pub terminal_output_color: Color,
    /// Terminal prompt text color.
    pub terminal_prompt_color: Color,
    /// Input bar border radius.
    pub input_border_radius: u16,
    /// App screen selected row background color.
    pub selected_bg: Color,
    /// Selection highlight border radius.
    pub selection_border_radius: u16,
    /// Selection left-accent bar color.
    pub selection_accent_color: Color,
    /// App title bar gradient top color (None = flat fill).
    pub title_bar_gradient_top: Option<Color>,
    /// App title bar gradient bottom color.
    pub title_bar_gradient_bottom: Option<Color>,
    /// Whether text shadow is enabled on app title bar text.
    pub title_bar_text_shadow: bool,
    /// App title bar text shadow color.
    pub title_bar_text_shadow_color: Color,
}

/// On-screen keyboard theme.
#[derive(Debug, Clone)]
pub struct OskTheme {
    /// OSK key background color.
    pub key_bg: Color,
    /// OSK key text color.
    pub key_text: Color,
    /// OSK focused key highlight color.
    pub key_focus: Color,
    /// OSK active key background color.
    pub key_active: Color,
    /// OSK dim text color (mode indicator, buffer display).
    pub key_dim_text: Color,
}

/// Scrollbar track, thumb, and dimension theme.
#[derive(Debug, Clone)]
pub struct ScrollbarTheme {
    /// Scrollbar track color.
    pub track_color: Color,
    /// Scrollbar thumb color.
    pub thumb_color: Color,
    /// Scrollbar thumb hover color.
    pub thumb_hover_color: Color,
    /// Scrollbar width in pixels.
    pub width: u32,
    /// Scrollbar corner radius.
    pub border_radius: u16,
}

/// Wallpaper style, gradient, and effect theme.
#[derive(Debug, Clone)]
pub struct WallpaperTheme {
    /// Wallpaper style: "gradient" (default), "solid", or "none".
    pub style: String,
    /// Wallpaper gradient color stops (default: PSIX 5-stop palette).
    pub stops: Vec<Color>,
    /// Whether PSIX arc ripple waves are enabled.
    pub wave: bool,
    /// Wave intensity 0.0-1.0.
    pub wave_intensity: f32,
    /// Gradient angle in degrees.
    pub angle: f32,
    /// Grid/dot spacing for pattern wallpapers (default 16).
    pub grid_spacing: u32,
    /// Grid/dot line color.
    pub grid_color: Color,
    /// Noise intensity for "noise" wallpaper (default 0.3).
    pub noise_intensity: f32,
    /// Whether the wallpaper animates (wave phase shift).
    pub animated: bool,
    /// Image asset key for `style = "image"` (e.g. `"assets/wall.png"`).
    pub source: Option<String>,
    /// Image fit mode: "cover" (default), "contain", "stretch", "tile".
    pub fit: String,
}

/// An image background layer: a bitmap decal (watermark, logo) composited
/// between the wallpaper and the icon layer, positioned by anchor and
/// optionally drifting/pulsing via the shared layer animation system.
#[derive(Debug, Clone)]
pub struct ImageLayerTheme {
    /// Asset key into `Skin::assets` (e.g. `"assets/logo.png"`).
    pub source: String,
    /// Anchor + fractional offset within the viewport.
    pub position: oasis_vector::background::LayerPosition,
    /// Drift / pulse animation parameters.
    pub animation: oasis_vector::background::LayerAnimation,
    /// Base opacity 0-255.
    pub alpha: u8,
    /// Whether the layer renders.
    pub enabled: bool,
}

/// Toast notification theme.
#[derive(Debug, Clone)]
pub struct ToastTheme {
    /// Toast info background color.
    pub info_bg: Color,
    /// Toast success background color.
    pub success_bg: Color,
    /// Toast error background color.
    pub error_bg: Color,
    /// Toast warning background color.
    pub warning_bg: Color,
    /// Toast text color.
    pub text_color: Color,
    /// Toast border radius.
    pub border_radius: u16,
    /// Toast time-to-live in frames.
    pub ttl: u32,
    /// Whether toast text has drop shadows.
    pub text_shadow: bool,
    /// Toast notification shadow level.
    pub shadow_level: u8,
    /// Toast fade in/out duration in frames (default 10).
    pub fade_frames: u32,
    /// Toast margin from screen edge (default 8).
    pub margin: i32,
    /// Toast height in pixels (default 24).
    pub height: u32,
    /// Toast width as fraction of screen width (default 0.333).
    pub width_fraction: f32,
    /// Gap between stacked toasts (default 4).
    pub gap: i32,
    /// Whether toasts slide in from the right (default true).
    pub slide_in: bool,
}

/// Runtime theme derived from the active skin's color palette.
///
/// All fields default to the same values as the legacy `theme.rs` constants.
/// `from_skin()` derives them from the skin's 9 base colors instead.
///
/// The theme is decomposed into focused sub-structs accessed through
/// `bar`, `icon`, `menu`, `app`, `osk`, `scrollbar`, `wallpaper`, and `toast`.
#[derive(Debug, Clone)]
pub struct ActiveTheme {
    /// Status bar, bottom bar, tab pills, page dots.
    pub bar: BarTheme,
    /// Dashboard icon rendering and cursor highlight.
    pub icon: IconTheme,
    /// Start button and popup panel.
    pub menu: StartMenuTheme,
    /// App content area, title bar, terminal, selection.
    pub app: AppScreenTheme,
    /// On-screen keyboard.
    pub osk: OskTheme,
    /// Scrollbar track, thumb, dimensions.
    pub scrollbar: ScrollbarTheme,
    /// Wallpaper style, gradient, effects.
    pub wallpaper: WallpaperTheme,
    /// Toast notifications.
    pub toast: ToastTheme,

    // -- Background layers --
    /// Data-driven background decoration layers.
    pub background_layers: Vec<oasis_vector::BackgroundLayer>,
    /// Chrome decoration layers rendered in the overlay pass (on top of
    /// bars and windows) — procedurally shaped chrome without shipped art.
    pub chrome_layers: Vec<oasis_vector::BackgroundLayer>,
    /// Image decal layers (`kind = "image"`), rendered between the
    /// wallpaper and the vector background pass.
    pub image_layers: Vec<ImageLayerTheme>,
    /// Maximum number of background layers to render (default 8).
    pub background_max_layers: u8,
    /// Whether to suppress background layer animations (default false).
    pub background_reduced_motion: bool,
    /// Maximum VectorOp budget for background rendering (default 200).
    pub background_complexity_budget: u32,

    // -- Icon entrance/focus animations --
    /// Entrance animation style: "none", "fade_in", "scale_up", "slide_up".
    pub entrance_style: String,
    /// Entrance animation duration in milliseconds.
    pub entrance_duration_ms: u32,
    /// Per-icon stagger delay in milliseconds.
    pub entrance_stagger_ms: u32,
    /// Focus scale factor (1.0 = no scale, 1.15 = 15% grow).
    pub focus_scale: f32,
    /// Whether a glow ring is drawn around the focused icon.
    pub focus_glow: bool,
    /// Focus glow color (default: accent at 40% alpha).
    pub focus_glow_color: Color,

    // -- Dashboard geometry --
    /// Dashboard grid horizontal padding (default 16).
    pub grid_padding_x: u16,
    /// Dashboard grid vertical padding (default 6).
    pub grid_padding_y: u16,
    /// Terminal background border radius (default 4).
    pub terminal_border_radius: u16,

    // -- Taskbar (desktop window list) --
    /// Taskbar height in pixels (default 20, 0 = disabled).
    pub taskbar_height: u32,
    /// Taskbar background color.
    pub taskbar_bg: Color,
    /// Taskbar gradient top color (None = flat fill).
    pub taskbar_gradient_top: Option<Color>,
    /// Taskbar gradient bottom color.
    pub taskbar_gradient_bottom: Option<Color>,
    /// Taskbar button color for the active/focused window.
    pub taskbar_btn_active: Color,
    /// Taskbar button color for normal (non-active, non-minimized) windows.
    pub taskbar_btn_inactive: Color,
    /// Taskbar button color for minimized windows.
    pub taskbar_btn_minimized: Color,
    /// Taskbar button hover color.
    pub taskbar_btn_hover: Color,
    /// Taskbar button text color.
    pub taskbar_text_color: Color,
    /// Taskbar top separator line color.
    pub taskbar_separator: Color,
    /// Taskbar active window indicator (underline) color.
    pub taskbar_indicator: Color,

    // -- Geometry overrides --
    /// Status bar height (default 24).
    pub statusbar_height: u32,
    /// Bottom bar height (default 24).
    pub bottombar_height: u32,
    /// Tab row height (default 18).
    pub tab_row_height: u32,
    /// Icon width (default 42).
    pub icon_width: u32,
    /// Icon height (default 52).
    pub icon_height: u32,
    /// Small font size (default 8).
    pub font_small: u16,

    // -- Scaled layout constants (proportional to screen_w / 480) --
    /// Top tab width (base 45 at 480px).
    pub tab_w: i32,
    /// Top tab height (base 16 at 480px).
    pub tab_h: i32,
    /// Gap between top tabs (base 4 at 480px).
    pub tab_gap: i32,
    /// X offset where top tabs start (base 34 at 480px).
    pub tab_start_x: i32,
    /// Pipe gap between media tab labels (base 5 at 480px).
    pub pipe_gap: i32,
    /// Width reserved for "R>" hint (base 28 at 480px).
    pub r_hint_w: i32,
    /// Colored stripe height at top of document icon (base 12 at 480px).
    pub icon_stripe_h: u32,
    /// Folded corner size on document icon (base 10 at 480px).
    pub icon_fold_size: u32,
    /// App graphic height on document body (base 22 at 480px).
    pub icon_gfx_h: u32,
    /// App graphic horizontal padding inside icon (base 4 at 480px).
    pub icon_gfx_pad: u32,
    /// Gap between icon bottom and label text (base 4 at 480px).
    pub icon_label_pad: i32,

    // -- Geometry overrides (raw, applied after scaling in with_screen_size) --
    /// Explicit tab width override (None = auto-scaled).
    pub(crate) tab_w_override: Option<i32>,
    /// Explicit tab height override (None = auto-scaled).
    pub(crate) tab_h_override: Option<i32>,
    /// Explicit tab gap override (None = auto-scaled).
    pub(crate) tab_gap_override: Option<i32>,
    /// Explicit tab start X override (None = auto-scaled).
    pub(crate) tab_start_x_override: Option<i32>,
    /// Explicit icon stripe height override (None = auto-scaled).
    pub(crate) icon_stripe_h_override: Option<u32>,
    /// Explicit icon fold size override (None = auto-scaled).
    pub(crate) icon_fold_size_override: Option<u32>,
    /// Explicit icon graphic height override (None = auto-scaled).
    pub(crate) icon_gfx_h_override: Option<u32>,
    /// Explicit icon graphic padding override (None = auto-scaled).
    pub(crate) icon_gfx_pad_override: Option<u32>,
    /// Explicit icon label padding override (None = auto-scaled).
    pub(crate) icon_label_pad_override: Option<i32>,

    // -- Screen dimensions --
    /// Screen width (default 480, PSP native).
    pub screen_w: u32,
    /// Screen height (default 272, PSP native).
    pub screen_h: u32,

    /// Clear/background color for the frame.
    pub clear_color: Color,

    // -- Terminal --
    /// Terminal line height in pixels.
    pub terminal_line_height: u32,

    // -- Cursor --
    /// Cursor scale factor (1 at <1920px, 2 at 1920px+).
    pub cursor_scale: u32,
    /// Asset path for a themed software cursor bitmap (from `[cursor]`
    /// in theme.toml). `None` = procedural arrow cursor.
    pub cursor_texture: Option<String>,
    /// Software cursor click hotspot (x, y) within the cursor image.
    pub cursor_hotspot: (i32, i32),
    /// Procedural mouse cursor arrow fill color (default white).
    pub cursor_fill: Color,
    /// Procedural mouse cursor arrow outline color (default black).
    pub cursor_outline: Color,

    // -- Terminal ANSI palette --
    /// 16-color ANSI palette for SGR-colored terminal output.
    pub ansi: AnsiPalette,

    // -- Transition --
    /// Transition fade overlay color (default: black).
    pub transition_fade_color: Color,
    /// Entrance transition on boot / skin swap: "fade", "assemble", "none".
    pub transition_entrance: String,
    /// Entrance duration in frames (used by "assemble"; default 45).
    pub transition_entrance_frames: u32,
    /// Dashboard page change style: "slide" (default) or "fade".
    pub transition_page_style: String,
    /// Easing curve name for entrance transitions ("" = built-in curve).
    pub transition_easing: String,

    // -- Font sizes --
    /// Body text font size (terminal lines, app content).
    pub font_body: u16,
    /// Hint/metadata font size (scroll indicators, metadata).
    pub font_hint: u16,
    /// Heading font size (section headings, media page title).
    pub font_heading: u16,
    /// System-wide font size scale factor (0.5-3.0, default 1.0).
    ///
    /// Applied as a multiplier wherever font sizes are used for rendering.
    /// Use `scaled_font_size()` to apply this factor to a raw font size.
    pub font_scale: f32,

    // -- Terminal cursor blink --
    /// Cursor blink rate in frames (0 = no blink, 30 = ~0.5s at 60fps).
    pub terminal_cursor_blink_rate: u32,

    // -- Animation durations --
    /// Cursor lerp speed (0.0-1.0, default 0.18).
    pub cursor_lerp_speed: f32,
    /// Page slide animation duration in frames (default 12).
    pub page_slide_duration: u32,
    /// Start menu open/close animation speed (default 0.15).
    pub start_menu_anim_speed: f32,
    /// Press flash duration in frames (default 6).
    pub press_flash_duration: u32,
    /// Cursor highlight padding around icon (default 3).
    pub cursor_pad: i32,
    /// Press flash lighten factor 0.0-1.0 (default 0.25).
    pub press_flash_lighten: f32,
    /// App selection lerp speed 0.0-1.0 (default 0.25).
    pub app_selection_lerp_speed: f32,
    /// Page dot lerp speed 0.0-1.0 (default 0.2).
    pub page_dot_lerp_speed: f32,

    // -- Per-app theme overrides --
    /// App-specific color overrides (app_name -> (key -> Color)).
    pub app_themes: std::collections::HashMap<String, std::collections::HashMap<String, Color>>,

    // -- Named gradient presets --
    /// Named gradient presets (name -> (from, to) colors).
    pub gradients: std::collections::HashMap<String, (Color, Color)>,

    // -- Named animation presets --
    /// Named animation presets (name -> (duration_ms, easing)).
    pub animations: std::collections::HashMap<String, (u32, String)>,

    // -- UI toolkit theme --
    /// Unified UI theme derived from the skin palette.
    pub ui_theme: oasis_ui::theme::Theme,

    // -- Semantic elevation ladder --
    /// Semantic shadow ladder (levels 0..=5). Built from the skin's
    /// `[elevation]` table; unset levels fall back to the built-in
    /// [`oasis_types::shadow::Shadow::elevation`] ladder. Resolve a level to a
    /// concrete shadow with [`ActiveTheme::resolve_shadow`].
    pub elevation: oasis_types::shadow::ElevationLadder,
}
