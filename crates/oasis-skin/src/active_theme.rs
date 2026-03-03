//! Runtime theme derived from the active skin.
//!
//! `ActiveTheme` replaces the hardcoded constants in `theme.rs` with a runtime
//! struct whose fields are derived from the skin's 9 base colors. Consumers
//! receive `&ActiveTheme` instead of reading `theme::CONST` directly, allowing
//! skins to actually drive the UI appearance.

use oasis_types::backend::Color;
use oasis_types::color::{darken, lighten, with_alpha};

use crate::SkinTheme;
use crate::theme::parse_hex_color;

/// Runtime theme derived from the active skin's color palette.
///
/// All fields default to the same values as the legacy `theme.rs` constants.
/// `from_skin()` derives them from the skin's 9 base colors instead.
#[derive(Debug, Clone)]
pub struct ActiveTheme {
    // -- Bar colors --
    /// Status bar background.
    pub statusbar_bg: Color,
    /// Bottom bar background.
    pub bar_bg: Color,
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

    // -- Icon colors --
    /// Document body color (white paper).
    pub icon_body_color: Color,
    /// Folded corner color.
    pub icon_fold_color: Color,
    /// Icon outline color.
    pub icon_outline_color: Color,
    /// Icon shadow color.
    pub icon_shadow_color: Color,
    /// Icon label text color.
    pub icon_label_color: Color,
    /// Icon label text shadow color (None = no shadow).
    pub icon_label_shadow: Option<Color>,
    /// Cursor highlight stroke color.
    pub cursor_color: Color,

    // -- Bar gradients --
    /// Status bar gradient top color (None = flat fill).
    pub statusbar_gradient_top: Option<Color>,
    /// Status bar gradient bottom color.
    pub statusbar_gradient_bottom: Option<Color>,
    /// Bottom bar gradient top color (None = flat fill).
    pub bar_gradient_top: Option<Color>,
    /// Bottom bar gradient bottom color.
    pub bar_gradient_bottom: Option<Color>,

    // -- Start menu colors --
    /// Start menu panel background.
    pub sm_panel_bg: Color,
    /// Start menu panel gradient top (None = flat fill).
    pub sm_panel_gradient_top: Option<Color>,
    /// Start menu panel gradient bottom.
    pub sm_panel_gradient_bottom: Option<Color>,
    /// Start menu panel border color.
    pub sm_panel_border: Color,
    /// Start menu item text color.
    pub sm_item_text: Color,
    /// Start menu active/selected item text color.
    pub sm_item_text_active: Color,
    /// Start menu selection highlight color.
    pub sm_highlight_color: Color,
    /// Start button background color.
    pub sm_button_bg: Color,
    /// Start button text color.
    pub sm_button_text: Color,
    /// Start menu panel border radius.
    pub sm_panel_border_radius: u16,
    /// Start menu panel shadow level.
    pub sm_panel_shadow_level: u8,

    // -- Start menu layout --
    /// Layout mode: "grid" or "list".
    pub sm_layout_mode: String,
    /// Start button label text.
    pub sm_button_label: String,
    /// Start button width.
    pub sm_button_width: u32,
    /// Start button height.
    pub sm_button_height: u32,
    /// Start button shape: "pill" or "rect".
    pub sm_button_shape: String,
    /// Start menu panel width.
    pub sm_panel_width: u32,
    /// Number of columns in the menu grid.
    pub sm_columns: usize,
    /// Start button gradient top color (None = flat fill).
    pub sm_button_gradient_top: Option<Color>,
    /// Start button gradient bottom color.
    pub sm_button_gradient_bottom: Option<Color>,
    /// Header text (None = no header).
    pub sm_header_text: Option<String>,
    /// Header background color.
    pub sm_header_bg: Color,
    /// Header text color.
    pub sm_header_text_color: Color,
    /// Header height.
    pub sm_header_height: u32,
    /// Whether footer is enabled.
    pub sm_footer_enabled: bool,
    /// Footer background color.
    pub sm_footer_bg: Color,
    /// Footer text color.
    pub sm_footer_text_color: Color,
    /// Footer height.
    pub sm_footer_height: u32,
    /// Item icon size.
    pub sm_item_icon_size: u32,
    /// Item row height.
    pub sm_item_row_height: i32,

    // -- Icon geometry --
    /// Icon card border radius (pixels).
    pub icon_border_radius: u16,
    /// Cursor highlight border radius (pixels).
    pub cursor_border_radius: u16,
    /// Cursor highlight stroke width (pixels).
    pub cursor_stroke_width: u16,

    // -- Icon/cursor style --
    /// Icon style variant: "document" (default), "card", or "circle".
    pub icon_style: String,
    /// Cursor style variant: "stroke" (default), "fill", or "underline".
    pub cursor_style: String,

    // -- App screen colors --
    /// App screen background color.
    pub app_bg: Color,
    /// App screen divider/separator color.
    pub app_divider: Color,
    /// App screen selected text color.
    pub app_selected_text: Color,
    /// App screen normal text color.
    pub app_text: Color,
    /// App screen dim/hint text color.
    pub app_dim_text: Color,
    /// App screen title bar background color.
    pub app_title_bar_bg: Color,
    /// App screen title bar text color.
    pub app_title_bar_text: Color,
    /// App screen title bar height.
    pub app_title_bar_height: u32,
    /// Terminal output text color.
    pub terminal_output_color: Color,
    /// Terminal prompt text color.
    pub terminal_prompt_color: Color,
    /// Input bar border radius.
    pub input_border_radius: u16,
    /// App screen selected row background color.
    pub app_selected_bg: Color,

    /// Clear/background color for the frame.
    pub clear_color: Color,

    // -- OSK colors --
    /// OSK key background color.
    pub osk_key_bg: Color,
    /// OSK key text color.
    pub osk_key_text: Color,
    /// OSK focused key highlight color.
    pub osk_key_focus: Color,
    /// OSK active key background color.
    pub osk_key_active: Color,
    /// OSK dim text color (mode indicator, buffer display).
    pub osk_key_dim_text: Color,

    // -- Dashboard geometry --
    /// Dashboard grid horizontal padding (default 16).
    pub grid_padding_x: u16,
    /// Dashboard grid vertical padding (default 6).
    pub grid_padding_y: u16,
    /// Dashboard icon shadow level (default 1).
    pub icon_shadow_level: u8,
    /// Terminal background border radius (default 4).
    pub terminal_border_radius: u16,

    // -- Start menu item palette --
    /// Start menu item icon colors (6 colors derived from primary).
    pub sm_item_colors: Vec<Color>,

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
    tab_w_override: Option<i32>,
    /// Explicit tab height override (None = auto-scaled).
    tab_h_override: Option<i32>,
    /// Explicit tab gap override (None = auto-scaled).
    tab_gap_override: Option<i32>,
    /// Explicit tab start X override (None = auto-scaled).
    tab_start_x_override: Option<i32>,

    // -- Screen dimensions --
    /// Screen width (default 480, PSP native).
    pub screen_w: u32,
    /// Screen height (default 272, PSP native).
    pub screen_h: u32,

    // -- Wallpaper config --
    /// Wallpaper style: "gradient" (default), "solid", or "none".
    pub wallpaper_style: String,
    /// Wallpaper gradient color stops (default: PSIX 5-stop palette).
    pub wallpaper_stops: Vec<Color>,
    /// Whether PSIX arc ripple waves are enabled.
    pub wallpaper_wave: bool,
    /// Wave intensity 0.0-1.0.
    pub wallpaper_wave_intensity: f32,
    /// Gradient angle in degrees.
    pub wallpaper_angle: f32,
    /// Grid/dot spacing for pattern wallpapers (default 16).
    pub wallpaper_grid_spacing: u32,
    /// Grid/dot line color.
    pub wallpaper_grid_color: Color,
    /// Noise intensity for "noise" wallpaper (default 0.3).
    pub wallpaper_noise_intensity: f32,
    /// Whether the wallpaper animates (wave phase shift).
    pub wallpaper_animated: bool,

    // -- Scrollbar --
    /// Scrollbar track color.
    pub scrollbar_track_color: Color,
    /// Scrollbar thumb color.
    pub scrollbar_thumb_color: Color,
    /// Scrollbar thumb hover color.
    pub scrollbar_thumb_hover_color: Color,
    /// Scrollbar width in pixels.
    pub scrollbar_width: u32,
    /// Scrollbar corner radius.
    pub scrollbar_border_radius: u16,

    // -- Terminal --
    /// Terminal line height in pixels.
    pub terminal_line_height: u32,

    // -- Cursor --
    /// Cursor scale factor (1 at <1920px, 2 at 1920px+).
    pub cursor_scale: u32,

    // -- Transition --
    /// Transition fade overlay color (default: black).
    pub transition_fade_color: Color,

    // -- Focus ring --
    /// Focus ring/outline color for highlighted elements.
    pub focus_ring_color: Color,
    /// Focus ring stroke width (pixels).
    pub focus_ring_width: u16,
    /// Focus ring offset from element edge (pixels).
    pub focus_ring_offset: i32,

    // -- Configurable strings --
    /// Version label text for status bar.
    pub bar_version_text: String,
    /// Category label text for status bar.
    pub bar_category_label: String,
    /// URL text for bottom bar.
    pub bar_url_text: String,
    /// Start menu footer text.
    pub sm_footer_text: String,

    // -- Tab pill stroke colors --
    /// Active tab pill stroke color.
    pub tab_active_stroke: Color,
    /// Inactive tab pill stroke color.
    pub tab_inactive_stroke: Color,

    // -- Font sizes --
    /// Body text font size (terminal lines, app content).
    pub font_body: u16,
    /// Hint/metadata font size (scroll indicators, metadata).
    pub font_hint: u16,
    /// Heading font size (section headings, media page title).
    pub font_heading: u16,

    // -- Terminal cursor blink --
    /// Cursor blink rate in frames (0 = no blink, 30 = ~0.5s at 60fps).
    pub terminal_cursor_blink_rate: u32,

    // -- Start menu inner padding --
    /// Inner padding for start menu panel.
    pub sm_pad_inner: i32,

    // -- Selection highlight --
    /// Selection highlight border radius.
    pub app_selection_border_radius: u16,
    /// Selection left-accent bar color.
    pub app_selection_accent_color: Color,

    // -- Toast notification theme --
    /// Toast info background color.
    pub toast_info_bg: Color,
    /// Toast success background color.
    pub toast_success_bg: Color,
    /// Toast error background color.
    pub toast_error_bg: Color,
    /// Toast warning background color.
    pub toast_warning_bg: Color,
    /// Toast text color.
    pub toast_text_color: Color,
    /// Toast border radius.
    pub toast_border_radius: u16,
    /// Toast time-to-live in frames.
    pub toast_ttl: u32,

    // -- Bar text shadows --
    /// Whether bar text elements have drop shadows.
    pub bar_text_shadow: bool,
    /// Bar text shadow color.
    pub bar_text_shadow_color: Color,
    /// Whether toast text has drop shadows.
    pub toast_text_shadow: bool,

    // -- Title bar gradients --
    /// App title bar gradient top color (None = flat fill).
    pub app_title_bar_gradient_top: Option<Color>,
    /// App title bar gradient bottom color.
    pub app_title_bar_gradient_bottom: Option<Color>,

    // -- Visual depth --
    /// Toast notification shadow level.
    pub toast_shadow_level: u8,

    // -- Animation durations --
    /// Cursor lerp speed (0.0-1.0, default 0.18).
    pub cursor_lerp_speed: f32,
    /// Page slide animation duration in frames (default 12).
    pub page_slide_duration: u32,
    /// Start menu open/close animation speed (default 0.15).
    pub start_menu_anim_speed: f32,
    /// Toast fade in/out duration in frames (default 10).
    pub toast_fade_frames: u32,
    /// Press flash duration in frames (default 6).
    pub press_flash_duration: u32,

    // -- Phase 6A: exposed hardcoded values --
    /// Cursor highlight padding around icon (default 3).
    pub cursor_pad: i32,
    /// Press flash lighten factor 0.0-1.0 (default 0.25).
    pub press_flash_lighten: f32,
    /// App selection lerp speed 0.0-1.0 (default 0.25).
    pub app_selection_lerp_speed: f32,
    /// Start button X position on the bottom bar (default 4).
    pub sm_button_x: i32,
    /// Menu panel X position (default 2).
    pub sm_panel_x: i32,
    /// Whether text shadow is enabled on app title bar text.
    pub app_title_bar_text_shadow: bool,
    /// App title bar text shadow color.
    pub app_title_bar_text_shadow_color: Color,
    /// Page dot lerp speed 0.0-1.0 (default 0.2).
    pub page_dot_lerp_speed: f32,
    /// Toast margin from screen edge (default 8).
    pub toast_margin: i32,
    /// Toast height in pixels (default 24).
    pub toast_height: u32,
    /// Toast width as fraction of screen width (default 0.333).
    pub toast_width_fraction: f32,
    /// Gap between stacked toasts (default 4).
    pub toast_gap: i32,
    /// Whether toasts slide in from the right (default true).
    pub toast_slide_in: bool,
    /// Whether item separators are drawn between start menu rows.
    pub sm_item_separator: bool,
    /// Item separator color.
    pub sm_item_separator_color: Color,

    // -- Per-app theme overrides --
    /// App-specific color overrides (app_name -> (key -> Color)).
    ///
    /// Populated from `[app_themes.<name>]` sections in theme.toml.
    /// Query with `app_color("tv_guide", "bg")`.
    pub app_themes: std::collections::HashMap<String, std::collections::HashMap<String, Color>>,

    // -- Named gradient presets --
    /// Named gradient presets (name -> (from, to) colors).
    pub gradients: std::collections::HashMap<String, (Color, Color)>,

    // -- Named animation presets --
    /// Named animation presets (name -> (duration_ms, easing)).
    pub animations: std::collections::HashMap<String, (u32, String)>,

    // -- Widget state color overrides --
    /// Per-widget state color overrides (widget_name -> (state_key -> Color)).
    ///
    /// Populated from `[widget_states.<widget>]` sections in theme.toml.
    /// Query with `widget_state_color("button", "hover_bg")`.
    pub widget_states: std::collections::HashMap<String, std::collections::HashMap<String, Color>>,

    // -- UI toolkit theme --
    /// Unified UI theme derived from the skin palette.
    ///
    /// Callers should use this instead of `oasis_ui::theme::Theme::dark()` etc.
    pub ui_theme: oasis_ui::theme::Theme,
}

impl Default for ActiveTheme {
    /// Returns legacy defaults identical to `theme.rs` constants.
    fn default() -> Self {
        Self {
            statusbar_bg: Color::rgba(0, 0, 0, 80),
            bar_bg: Color::rgba(0, 0, 0, 90),
            separator_color: Color::rgba(255, 255, 255, 50),
            battery_color: Color::rgb(120, 255, 120),
            version_color: Color::WHITE,
            clock_color: Color::WHITE,
            url_color: Color::rgb(200, 200, 200),
            usb_color: Color::rgb(140, 140, 140),
            tab_active_fill: Color::rgba(255, 255, 255, 30),
            tab_inactive_fill: Color::rgba(0, 0, 0, 0),
            tab_active_alpha: 180,
            tab_inactive_alpha: 60,
            media_tab_active: Color::WHITE,
            media_tab_inactive: Color::rgb(170, 170, 170),
            pipe_color: Color::rgba(255, 255, 255, 60),
            r_hint_color: Color::rgba(255, 255, 255, 140),
            category_label_color: Color::rgb(220, 220, 220),
            page_dot_active: Color::rgba(255, 255, 255, 200),
            page_dot_inactive: Color::rgba(255, 255, 255, 50),
            statusbar_gradient_top: None,
            statusbar_gradient_bottom: None,
            bar_gradient_top: None,
            bar_gradient_bottom: None,
            sm_panel_bg: Color::rgba(20, 20, 35, 220),
            sm_panel_gradient_top: None,
            sm_panel_gradient_bottom: None,
            sm_panel_border: Color::rgba(255, 255, 255, 40),
            sm_item_text: Color::rgb(220, 220, 220),
            sm_item_text_active: Color::WHITE,
            sm_highlight_color: Color::rgba(50, 100, 200, 80),
            sm_button_bg: Color::rgba(50, 100, 200, 200),
            sm_button_text: Color::WHITE,
            sm_panel_border_radius: 4,
            sm_panel_shadow_level: 1,
            sm_layout_mode: "grid".to_string(),
            sm_button_label: "START".to_string(),
            sm_button_width: 48,
            sm_button_height: 18,
            sm_button_shape: "pill".to_string(),
            sm_panel_width: 200,
            sm_columns: 2,
            sm_button_gradient_top: None,
            sm_button_gradient_bottom: None,
            sm_header_text: None,
            sm_header_bg: Color::rgba(30, 30, 50, 240),
            sm_header_text_color: Color::WHITE,
            sm_header_height: 0,
            sm_footer_enabled: false,
            sm_footer_bg: Color::rgba(30, 30, 50, 240),
            sm_footer_text_color: Color::WHITE,
            sm_footer_height: 0,
            sm_item_icon_size: 14,
            sm_item_row_height: 22,
            app_bg: Color::rgb(12, 12, 20),
            app_divider: Color::rgb(60, 60, 80),
            app_selected_text: Color::rgb(100, 200, 255),
            app_text: Color::rgb(180, 180, 200),
            app_dim_text: Color::rgb(100, 100, 130),
            app_title_bar_bg: Color::rgb(30, 50, 90),
            app_title_bar_text: Color::WHITE,
            app_title_bar_height: 22,
            terminal_output_color: Color::rgb(204, 204, 204),
            terminal_prompt_color: Color::rgb(0, 255, 0),
            input_border_radius: 3,
            app_selected_bg: Color::rgba(50, 100, 200, 40),
            clear_color: Color::rgb(10, 10, 18),
            osk_key_bg: Color::rgba(20, 20, 40, 220),
            osk_key_text: Color::WHITE,
            osk_key_focus: Color::rgb(100, 200, 255),
            osk_key_active: Color::rgb(60, 100, 180),
            osk_key_dim_text: Color::rgb(150, 150, 180),
            grid_padding_x: 16,
            grid_padding_y: 6,
            icon_shadow_level: 1,
            terminal_border_radius: 4,
            sm_item_colors: vec![
                Color::rgb(70, 130, 180),
                Color::rgb(60, 179, 113),
                Color::rgb(218, 165, 32),
                Color::rgb(186, 85, 211),
                Color::rgb(100, 149, 237),
                Color::rgb(205, 92, 92),
            ],
            icon_body_color: Color::rgb(250, 250, 248),
            icon_fold_color: Color::rgb(210, 210, 205),
            icon_outline_color: Color::rgba(255, 255, 255, 180),
            icon_shadow_color: Color::rgba(0, 0, 0, 70),
            icon_label_color: Color::rgba(255, 255, 255, 230),
            icon_label_shadow: Some(Color::rgba(0, 0, 0, 120)),
            cursor_color: Color::rgba(255, 255, 255, 90),
            icon_border_radius: 4,
            cursor_border_radius: 6,
            cursor_stroke_width: 2,
            icon_style: "document".to_string(),
            cursor_style: "stroke".to_string(),
            statusbar_height: 24,
            bottombar_height: 24,
            tab_row_height: 18,
            icon_width: 42,
            icon_height: 52,
            font_small: 8,
            tab_w: 45,
            tab_h: 16,
            tab_gap: 4,
            tab_start_x: 34,
            pipe_gap: 5,
            r_hint_w: 28,
            icon_stripe_h: 12,
            icon_fold_size: 10,
            icon_gfx_h: 22,
            icon_gfx_pad: 4,
            icon_label_pad: 4,
            tab_w_override: None,
            tab_h_override: None,
            tab_gap_override: None,
            tab_start_x_override: None,
            screen_w: 480,
            screen_h: 272,
            wallpaper_style: "gradient".to_string(),
            wallpaper_stops: vec![
                Color::rgb(245, 110, 15),
                Color::rgb(255, 230, 30),
                Color::rgb(230, 245, 40),
                Color::rgb(140, 235, 50),
                Color::rgb(200, 252, 130),
            ],
            wallpaper_wave: true,
            wallpaper_wave_intensity: 1.0,
            wallpaper_angle: 0.0,
            wallpaper_grid_spacing: 16,
            wallpaper_grid_color: Color::rgba(255, 255, 255, 20),
            wallpaper_noise_intensity: 0.3,
            wallpaper_animated: false,
            scrollbar_track_color: Color::rgba(255, 255, 255, 20),
            scrollbar_thumb_color: Color::rgba(255, 255, 255, 100),
            scrollbar_thumb_hover_color: Color::rgba(255, 255, 255, 160),
            scrollbar_width: 6,
            scrollbar_border_radius: 3,
            terminal_line_height: 16,
            cursor_scale: 1,
            transition_fade_color: Color::BLACK,
            focus_ring_color: Color::rgba(100, 200, 255, 180),
            focus_ring_width: 2,
            focus_ring_offset: 2,
            bar_version_text: "Version 0.1".to_string(),
            bar_category_label: "OSS".to_string(),
            bar_url_text: String::new(),
            sm_footer_text: "Log Off  Shut Down".to_string(),
            tab_active_stroke: Color::rgba(255, 255, 255, 180),
            tab_inactive_stroke: Color::rgba(255, 255, 255, 60),
            font_body: 12,
            font_hint: 10,
            font_heading: 14,
            terminal_cursor_blink_rate: 30,
            sm_pad_inner: 8,
            app_selection_border_radius: 2,
            app_selection_accent_color: Color::rgba(50, 100, 200, 128),
            toast_info_bg: Color::rgba(50, 100, 200, 220),
            toast_success_bg: Color::rgba(60, 180, 100, 220),
            toast_error_bg: Color::rgba(255, 68, 68, 220),
            toast_warning_bg: Color::rgba(230, 170, 40, 220),
            toast_text_color: Color::WHITE,
            toast_border_radius: 4,
            toast_ttl: 180,
            bar_text_shadow: false,
            bar_text_shadow_color: Color::rgba(0, 0, 0, 128),
            toast_text_shadow: false,
            app_title_bar_gradient_top: None,
            app_title_bar_gradient_bottom: None,
            toast_shadow_level: 1,
            cursor_lerp_speed: 0.35,
            page_slide_duration: 6,
            start_menu_anim_speed: 0.25,
            toast_fade_frames: 10,
            press_flash_duration: 6,
            cursor_pad: 3,
            press_flash_lighten: 0.25,
            app_selection_lerp_speed: 0.25,
            sm_button_x: 4,
            sm_panel_x: 2,
            app_title_bar_text_shadow: false,
            app_title_bar_text_shadow_color: Color::rgba(0, 0, 0, 128),
            page_dot_lerp_speed: 0.2,
            toast_margin: 8,
            toast_height: 24,
            toast_width_fraction: 0.333,
            toast_gap: 4,
            toast_slide_in: true,
            sm_item_separator: false,
            sm_item_separator_color: Color::rgba(255, 255, 255, 40),
            app_themes: std::collections::HashMap::new(),
            gradients: std::collections::HashMap::new(),
            animations: std::collections::HashMap::new(),
            widget_states: std::collections::HashMap::new(),
            ui_theme: oasis_ui::theme::Theme::dark(),
        }
    }
}

impl ActiveTheme {
    /// Derive an `ActiveTheme` from the skin's base color palette.
    ///
    /// The 9 base colors (background, primary, secondary, text, dim_text,
    /// status_bar, prompt, output, error) drive all UI element colors.
    /// Fine-grained overrides (Phase 5) are checked first.
    pub fn from_skin(skin: &SkinTheme) -> Self {
        let status_bar_color =
            parse_hex_color(&skin.status_bar).unwrap_or(Color::rgba(0, 0, 0, 80));
        let primary = skin.primary_color();
        let secondary = skin.secondary_color();
        let text = skin.text_color();
        let dim = skin.dim_text_color();

        // Helper: parse an optional hex color override.
        let ov = |opt: Option<&String>, fallback: Color| -> Color {
            opt.and_then(|s| parse_hex_color(s)).unwrap_or(fallback)
        };

        let bar = skin.bar_overrides.as_ref();
        let ico = skin.icon_overrides.as_ref();
        let sm = skin.start_menu_overrides.as_ref();

        Self {
            statusbar_bg: ov(
                bar.and_then(|b| b.statusbar_bg.as_ref()),
                with_alpha(status_bar_color, 80),
            ),
            bar_bg: ov(
                bar.and_then(|b| b.bar_bg.as_ref()),
                with_alpha(status_bar_color, 90),
            ),
            separator_color: ov(
                bar.and_then(|b| b.separator_color.as_ref()),
                with_alpha(secondary, 50),
            ),
            battery_color: ov(
                bar.and_then(|b| b.battery_color.as_ref()),
                lighten(primary, 0.3),
            ),
            version_color: ov(bar.and_then(|b| b.version_color.as_ref()), text),
            clock_color: ov(bar.and_then(|b| b.clock_color.as_ref()), text),
            url_color: ov(bar.and_then(|b| b.url_color.as_ref()), dim),
            usb_color: ov(bar.and_then(|b| b.usb_color.as_ref()), dim),
            tab_active_fill: ov(
                bar.and_then(|b| b.tab_active_fill.as_ref()),
                with_alpha(primary, 30),
            ),
            tab_inactive_fill: Color::rgba(0, 0, 0, 0),
            tab_active_alpha: bar.and_then(|b| b.tab_active_alpha).unwrap_or(180),
            tab_inactive_alpha: bar.and_then(|b| b.tab_inactive_alpha).unwrap_or(60),
            media_tab_active: ov(bar.and_then(|b| b.media_tab_active.as_ref()), text),
            media_tab_inactive: ov(bar.and_then(|b| b.media_tab_inactive.as_ref()), dim),
            pipe_color: ov(
                bar.and_then(|b| b.pipe_color.as_ref()),
                with_alpha(text, 60),
            ),
            r_hint_color: ov(
                bar.and_then(|b| b.r_hint_color.as_ref()),
                with_alpha(text, 140),
            ),
            category_label_color: ov(
                bar.and_then(|b| b.category_label_color.as_ref()),
                with_alpha(text, 220),
            ),
            page_dot_active: ov(
                bar.and_then(|b| b.page_dot_active.as_ref()),
                with_alpha(text, 200),
            ),
            page_dot_inactive: ov(
                bar.and_then(|b| b.page_dot_inactive.as_ref()),
                with_alpha(text, 50),
            ),
            sm_panel_bg: ov(
                sm.and_then(|s| s.panel_bg.as_ref()),
                Color::rgba(20, 20, 35, 220),
            ),
            sm_panel_gradient_top: sm
                .and_then(|s| s.panel_gradient_top.as_ref())
                .and_then(|s| parse_hex_color(s)),
            sm_panel_gradient_bottom: sm
                .and_then(|s| s.panel_gradient_bottom.as_ref())
                .and_then(|s| parse_hex_color(s)),
            sm_panel_border: ov(
                sm.and_then(|s| s.panel_border.as_ref()),
                with_alpha(text, 40),
            ),
            sm_item_text: ov(sm.and_then(|s| s.item_text.as_ref()), with_alpha(text, 220)),
            sm_item_text_active: ov(sm.and_then(|s| s.item_text_active.as_ref()), text),
            sm_highlight_color: ov(
                sm.and_then(|s| s.highlight_color.as_ref()),
                with_alpha(primary, 80),
            ),
            sm_button_bg: ov(
                sm.and_then(|s| s.button_bg.as_ref()),
                with_alpha(primary, 200),
            ),
            sm_button_text: ov(sm.and_then(|s| s.button_text.as_ref()), text),
            sm_panel_border_radius: sm
                .and_then(|s| s.panel_border_radius)
                .unwrap_or_else(|| skin.border_radius.unwrap_or(4)),
            sm_panel_shadow_level: sm.and_then(|s| s.panel_shadow_level).unwrap_or(1),
            sm_layout_mode: sm
                .and_then(|s| s.layout_mode.clone())
                .unwrap_or_else(|| "grid".to_string()),
            sm_button_label: sm
                .and_then(|s| s.button_label.clone())
                .unwrap_or_else(|| "START".to_string()),
            sm_button_width: sm.and_then(|s| s.button_width).unwrap_or(48),
            sm_button_height: sm.and_then(|s| s.button_height).unwrap_or(18),
            sm_button_shape: sm
                .and_then(|s| s.button_shape.clone())
                .unwrap_or_else(|| "pill".to_string()),
            sm_panel_width: sm.and_then(|s| s.panel_width).unwrap_or(200),
            sm_columns: sm.and_then(|s| s.columns).unwrap_or(2).max(1),
            sm_button_gradient_top: sm
                .and_then(|s| s.button_gradient_top.as_ref())
                .and_then(|s| parse_hex_color(s)),
            sm_button_gradient_bottom: sm
                .and_then(|s| s.button_gradient_bottom.as_ref())
                .and_then(|s| parse_hex_color(s)),
            sm_header_text: sm.and_then(|s| s.header_text.clone()),
            sm_header_bg: ov(
                sm.and_then(|s| s.header_bg.as_ref()),
                Color::rgba(30, 30, 50, 240),
            ),
            sm_header_text_color: ov(sm.and_then(|s| s.header_text_color.as_ref()), text),
            sm_header_height: sm.and_then(|s| s.header_height).unwrap_or(0),
            sm_footer_enabled: sm.and_then(|s| s.footer_enabled).unwrap_or(false),
            sm_footer_bg: ov(
                sm.and_then(|s| s.footer_bg.as_ref()),
                Color::rgba(30, 30, 50, 240),
            ),
            sm_footer_text_color: ov(sm.and_then(|s| s.footer_text_color.as_ref()), text),
            sm_footer_height: sm.and_then(|s| s.footer_height).unwrap_or(0),
            sm_item_icon_size: sm.and_then(|s| s.item_icon_size).unwrap_or(14),
            sm_item_row_height: sm.and_then(|s| s.item_row_height).unwrap_or(22).max(1),
            // App screen colors: derive from skin background/primary/text.
            app_bg: {
                let ap = skin.app_overrides.as_ref();
                ov(
                    ap.and_then(|a| a.app_bg.as_ref()),
                    lighten(skin.background_color(), 0.02),
                )
            },
            app_divider: {
                let ap = skin.app_overrides.as_ref();
                ov(
                    ap.and_then(|a| a.divider_color.as_ref()),
                    lighten(skin.background_color(), 0.15),
                )
            },
            app_selected_text: {
                let ap = skin.app_overrides.as_ref();
                ov(
                    ap.and_then(|a| a.selected_text.as_ref()),
                    lighten(primary, 0.3),
                )
            },
            app_text: {
                let ap = skin.app_overrides.as_ref();
                ov(ap.and_then(|a| a.text_color.as_ref()), lighten(dim, 0.2))
            },
            app_dim_text: {
                let ap = skin.app_overrides.as_ref();
                ov(ap.and_then(|a| a.dim_text.as_ref()), dim)
            },
            app_title_bar_bg: {
                let ap = skin.app_overrides.as_ref();
                ov(
                    ap.and_then(|a| a.title_bar_bg.as_ref()),
                    lighten(skin.background_color(), 0.08),
                )
            },
            app_title_bar_text: {
                let ap = skin.app_overrides.as_ref();
                ov(ap.and_then(|a| a.title_bar_text.as_ref()), text)
            },
            app_title_bar_height: skin
                .app_overrides
                .as_ref()
                .and_then(|a| a.title_bar_height)
                .unwrap_or(22),
            terminal_output_color: {
                let ap = skin.app_overrides.as_ref();
                ov(
                    ap.and_then(|a| a.terminal_output_color.as_ref()),
                    skin.output_color(),
                )
            },
            terminal_prompt_color: {
                let ap = skin.app_overrides.as_ref();
                ov(
                    ap.and_then(|a| a.terminal_prompt_color.as_ref()),
                    skin.prompt_color(),
                )
            },
            input_border_radius: skin
                .app_overrides
                .as_ref()
                .and_then(|a| a.input_border_radius)
                .unwrap_or_else(|| {
                    skin.geometry
                        .as_ref()
                        .and_then(|g| g.terminal_border_radius)
                        .unwrap_or(4)
                }),
            app_selected_bg: with_alpha(primary, 40),
            clear_color: darken(skin.background_color(), 0.5),
            // OSK colors: derive from skin background/primary/text.
            osk_key_bg: {
                let ok = skin.osk_overrides.as_ref();
                ov(
                    ok.and_then(|o| o.key_bg.as_ref()),
                    with_alpha(lighten(skin.background_color(), 0.05), 220),
                )
            },
            osk_key_text: {
                let ok = skin.osk_overrides.as_ref();
                ov(ok.and_then(|o| o.key_text.as_ref()), text)
            },
            osk_key_focus: {
                let ok = skin.osk_overrides.as_ref();
                ov(ok.and_then(|o| o.key_focus.as_ref()), lighten(primary, 0.3))
            },
            osk_key_active: {
                let ok = skin.osk_overrides.as_ref();
                ov(ok.and_then(|o| o.key_active.as_ref()), primary)
            },
            osk_key_dim_text: {
                let ok = skin.osk_overrides.as_ref();
                ov(ok.and_then(|o| o.key_dim_text.as_ref()), dim)
            },
            // Dashboard geometry.
            grid_padding_x: skin
                .geometry
                .as_ref()
                .and_then(|g| g.grid_padding_x)
                .unwrap_or(16),
            grid_padding_y: skin
                .geometry
                .as_ref()
                .and_then(|g| g.grid_padding_y)
                .unwrap_or(6),
            icon_shadow_level: skin
                .geometry
                .as_ref()
                .and_then(|g| g.icon_shadow_level)
                .unwrap_or(1),
            terminal_border_radius: skin
                .geometry
                .as_ref()
                .and_then(|g| g.terminal_border_radius)
                .unwrap_or(4),
            // Start menu item colors.
            sm_item_colors: sm
                .and_then(|s| s.item_colors.as_ref())
                .map(|colors| {
                    colors
                        .iter()
                        .filter_map(|s| parse_hex_color(s))
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| Self::derive_item_palette(primary)),
            icon_body_color: ov(ico.and_then(|i| i.body_color.as_ref()), text),
            icon_fold_color: ov(ico.and_then(|i| i.fold_color.as_ref()), dim),
            icon_outline_color: ov(
                ico.and_then(|i| i.outline_color.as_ref()),
                with_alpha(text, 180),
            ),
            icon_shadow_color: ov(
                ico.and_then(|i| i.shadow_color.as_ref()),
                Color::rgba(0, 0, 0, 70),
            ),
            icon_label_color: ov(
                ico.and_then(|i| i.label_color.as_ref()),
                with_alpha(text, 230),
            ),
            icon_label_shadow: {
                let lc = ov(
                    ico.and_then(|i| i.label_color.as_ref()),
                    with_alpha(text, 230),
                );
                // Auto-derive: light labels get a dark shadow for readability.
                let brightness = lc.r as u16 * 3 / 10 + lc.g as u16 * 6 / 10 + lc.b as u16 / 10;
                if brightness > 140 {
                    Some(Color::rgba(0, 0, 0, 120))
                } else {
                    None
                }
            },
            cursor_color: ov(
                ico.and_then(|i| i.cursor_color.as_ref()),
                with_alpha(primary, 80),
            ),
            icon_border_radius: ico
                .and_then(|i| i.icon_border_radius)
                .unwrap_or_else(|| skin.border_radius.unwrap_or(4)),
            cursor_border_radius: ico
                .and_then(|i| i.cursor_border_radius)
                .unwrap_or_else(|| skin.border_radius.map(|r| r + 2).unwrap_or(6)),
            cursor_stroke_width: ico.and_then(|i| i.cursor_stroke_width).unwrap_or(2),
            icon_style: ico
                .and_then(|i| i.icon_style.clone())
                .unwrap_or_else(|| "document".to_string()),
            cursor_style: ico
                .and_then(|i| i.cursor_style.clone())
                .unwrap_or_else(|| "stroke".to_string()),
            statusbar_height: skin
                .geometry
                .as_ref()
                .and_then(|g| g.statusbar_height)
                .unwrap_or(24),
            bottombar_height: skin
                .geometry
                .as_ref()
                .and_then(|g| g.bottombar_height)
                .unwrap_or(24),
            tab_row_height: skin
                .geometry
                .as_ref()
                .and_then(|g| g.tab_row_height)
                .unwrap_or(18),
            icon_width: skin
                .geometry
                .as_ref()
                .and_then(|g| g.icon_width)
                .unwrap_or(42),
            icon_height: skin
                .geometry
                .as_ref()
                .and_then(|g| g.icon_height)
                .unwrap_or(52),
            font_small: skin
                .geometry
                .as_ref()
                .and_then(|g| g.font_small)
                .unwrap_or(8),
            tab_w: 45,
            tab_h: 16,
            tab_gap: 4,
            tab_start_x: 34,
            pipe_gap: 5,
            r_hint_w: 28,
            icon_stripe_h: 12,
            icon_fold_size: 10,
            icon_gfx_h: 22,
            icon_gfx_pad: 4,
            icon_label_pad: 4,
            wallpaper_style: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.style.clone())
                .unwrap_or_else(|| "gradient".to_string()),
            wallpaper_stops: skin
                .wallpaper
                .as_ref()
                .and_then(|w| {
                    w.color_stops.as_ref().map(|stops| {
                        stops
                            .iter()
                            .filter_map(|s| parse_hex_color(s))
                            .collect::<Vec<_>>()
                    })
                })
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| {
                    vec![
                        Color::rgb(245, 110, 15),
                        Color::rgb(255, 230, 30),
                        Color::rgb(230, 245, 40),
                        Color::rgb(140, 235, 50),
                        Color::rgb(200, 252, 130),
                    ]
                }),
            wallpaper_wave: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.wave_enabled)
                .unwrap_or(true),
            wallpaper_wave_intensity: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.wave_intensity)
                .unwrap_or(1.0),
            wallpaper_angle: skin.wallpaper.as_ref().and_then(|w| w.angle).unwrap_or(0.0),
            wallpaper_grid_spacing: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.grid_spacing)
                .unwrap_or(16),
            wallpaper_grid_color: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.grid_color.as_ref())
                .and_then(|s| parse_hex_color(s))
                .unwrap_or_else(|| lighten(skin.background_color(), 0.08)),
            wallpaper_noise_intensity: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.noise_intensity)
                .unwrap_or(0.3),
            wallpaper_animated: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.animated)
                .unwrap_or(false),
            scrollbar_track_color: {
                let sb = skin.scrollbar_overrides.as_ref();
                ov(
                    sb.and_then(|s| s.track_color.as_ref()),
                    with_alpha(secondary, 20),
                )
            },
            scrollbar_thumb_color: {
                let sb = skin.scrollbar_overrides.as_ref();
                ov(
                    sb.and_then(|s| s.thumb_color.as_ref()),
                    with_alpha(secondary, 100),
                )
            },
            scrollbar_thumb_hover_color: {
                let sb = skin.scrollbar_overrides.as_ref();
                ov(
                    sb.and_then(|s| s.thumb_hover_color.as_ref()),
                    with_alpha(secondary, 160),
                )
            },
            scrollbar_width: skin
                .scrollbar_overrides
                .as_ref()
                .and_then(|s| s.width)
                .or_else(|| skin.geometry.as_ref().and_then(|g| g.scrollbar_width))
                .unwrap_or(6),
            scrollbar_border_radius: skin
                .geometry
                .as_ref()
                .and_then(|g| g.scrollbar_border_radius)
                .unwrap_or(3),
            terminal_line_height: skin
                .geometry
                .as_ref()
                .and_then(|g| g.terminal_line_height)
                .unwrap_or(16),
            cursor_scale: 1, // Set by with_screen_size()
            focus_ring_color: skin
                .geometry
                .as_ref()
                .and_then(|g| g.focus_ring_color.as_ref())
                .and_then(|s| parse_hex_color(s))
                .unwrap_or_else(|| with_alpha(primary, 180)),
            focus_ring_width: skin
                .geometry
                .as_ref()
                .and_then(|g| g.focus_ring_width)
                .unwrap_or(2),
            focus_ring_offset: skin
                .geometry
                .as_ref()
                .and_then(|g| g.focus_ring_offset)
                .unwrap_or(2),
            transition_fade_color: skin
                .transition
                .as_ref()
                .and_then(|t| t.fade_color.as_ref())
                .and_then(|s| parse_hex_color(s))
                .unwrap_or_else(|| darken(skin.background_color(), 0.3)),
            tab_w_override: skin
                .geometry
                .as_ref()
                .and_then(|g| g.tab_w)
                .map(|v| v as i32),
            tab_h_override: skin
                .geometry
                .as_ref()
                .and_then(|g| g.tab_h)
                .map(|v| v as i32),
            tab_gap_override: skin
                .geometry
                .as_ref()
                .and_then(|g| g.tab_gap)
                .map(|v| v as i32),
            tab_start_x_override: skin.geometry.as_ref().and_then(|g| g.tab_start_x),
            screen_w: 480,
            screen_h: 272,
            statusbar_gradient_top: Self::bar_gradient_pair(
                skin,
                bar.and_then(|b| b.statusbar_gradient_top.as_ref()),
                bar.and_then(|b| b.statusbar_gradient_bottom.as_ref()),
                status_bar_color,
            )
            .map(|(t, _)| t),
            statusbar_gradient_bottom: Self::bar_gradient_pair(
                skin,
                bar.and_then(|b| b.statusbar_gradient_top.as_ref()),
                bar.and_then(|b| b.statusbar_gradient_bottom.as_ref()),
                status_bar_color,
            )
            .map(|(_, b)| b),
            bar_gradient_top: Self::bar_gradient_pair(
                skin,
                bar.and_then(|b| b.bar_gradient_top.as_ref()),
                bar.and_then(|b| b.bar_gradient_bottom.as_ref()),
                status_bar_color,
            )
            .map(|(t, _)| t),
            bar_gradient_bottom: Self::bar_gradient_pair(
                skin,
                bar.and_then(|b| b.bar_gradient_top.as_ref()),
                bar.and_then(|b| b.bar_gradient_bottom.as_ref()),
                status_bar_color,
            )
            .map(|(_, b)| b),
            bar_version_text: bar
                .and_then(|b| b.version_text.clone())
                .unwrap_or_else(|| "Version 0.1".to_string()),
            bar_category_label: bar
                .and_then(|b| b.category_label.clone())
                .unwrap_or_else(|| "OSS".to_string()),
            bar_url_text: bar.and_then(|b| b.url_text.clone()).unwrap_or_default(),
            sm_footer_text: sm
                .and_then(|s| s.footer_text.clone())
                .unwrap_or_else(|| "Log Off  Shut Down".to_string()),
            tab_active_stroke: with_alpha(
                text,
                bar.and_then(|b| b.tab_active_alpha).unwrap_or(180),
            ),
            tab_inactive_stroke: with_alpha(
                text,
                bar.and_then(|b| b.tab_inactive_alpha).unwrap_or(60),
            ),
            font_body: skin
                .geometry
                .as_ref()
                .and_then(|g| g.font_body)
                .unwrap_or(12),
            font_hint: skin
                .geometry
                .as_ref()
                .and_then(|g| g.font_hint)
                .unwrap_or(10),
            font_heading: skin
                .geometry
                .as_ref()
                .and_then(|g| g.font_heading)
                .unwrap_or(14),
            terminal_cursor_blink_rate: skin
                .geometry
                .as_ref()
                .and_then(|g| g.cursor_blink_rate)
                .unwrap_or(30),
            sm_pad_inner: sm.and_then(|s| s.pad_inner).unwrap_or(8),
            app_selection_border_radius: {
                let ap = skin.app_overrides.as_ref();
                ap.and_then(|a| a.selection_border_radius).unwrap_or(2)
            },
            app_selection_accent_color: {
                let ap = skin.app_overrides.as_ref();
                ov(
                    ap.and_then(|a| a.selection_accent_color.as_ref()),
                    with_alpha(primary, 128),
                )
            },
            toast_info_bg: with_alpha(primary, 220),
            toast_success_bg: Color::rgba(60, 180, 100, 220),
            toast_error_bg: with_alpha(skin.error_color(), 220),
            toast_warning_bg: Color::rgba(230, 170, 40, 220),
            toast_text_color: text,
            toast_border_radius: skin
                .geometry
                .as_ref()
                .and_then(|g| g.terminal_border_radius)
                .unwrap_or(4),
            toast_ttl: 180,
            // -- 5B: Bar text shadows --
            bar_text_shadow: bar
                .and_then(|b| b.text_shadow)
                .unwrap_or(skin.gradient_enabled == Some(true)),
            bar_text_shadow_color: ov(
                bar.and_then(|b| b.text_shadow_color.as_ref()),
                Color::rgba(0, 0, 0, 128),
            ),
            toast_text_shadow: skin.gradient_enabled == Some(true),
            // -- 5C: Title bar gradients --
            app_title_bar_gradient_top: {
                let ap = skin.app_overrides.as_ref();
                Self::bar_gradient_pair(
                    skin,
                    ap.and_then(|a| a.title_bar_gradient_top.as_ref()),
                    ap.and_then(|a| a.title_bar_gradient_bottom.as_ref()),
                    ov(
                        ap.and_then(|a| a.title_bar_bg.as_ref()),
                        darken(status_bar_color, 0.8),
                    ),
                )
                .map(|(t, _)| t)
            },
            app_title_bar_gradient_bottom: {
                let ap = skin.app_overrides.as_ref();
                Self::bar_gradient_pair(
                    skin,
                    ap.and_then(|a| a.title_bar_gradient_top.as_ref()),
                    ap.and_then(|a| a.title_bar_gradient_bottom.as_ref()),
                    ov(
                        ap.and_then(|a| a.title_bar_bg.as_ref()),
                        darken(status_bar_color, 0.8),
                    ),
                )
                .map(|(_, b)| b)
            },
            // -- 5D: Visual depth --
            toast_shadow_level: 1,
            // -- 5E: Animation durations --
            cursor_lerp_speed: skin
                .geometry
                .as_ref()
                .and_then(|g| g.cursor_lerp_speed)
                .unwrap_or(0.35),
            page_slide_duration: skin
                .geometry
                .as_ref()
                .and_then(|g| g.page_slide_duration)
                .unwrap_or(6),
            start_menu_anim_speed: skin
                .geometry
                .as_ref()
                .and_then(|g| g.start_menu_anim_speed)
                .unwrap_or(0.25),
            toast_fade_frames: skin
                .geometry
                .as_ref()
                .and_then(|g| g.toast_fade_frames)
                .unwrap_or(10),
            press_flash_duration: skin
                .geometry
                .as_ref()
                .and_then(|g| g.press_flash_duration)
                .unwrap_or(6),
            // -- Phase 6A: exposed hardcoded values --
            cursor_pad: skin
                .geometry
                .as_ref()
                .and_then(|g| g.cursor_pad)
                .unwrap_or(3),
            press_flash_lighten: skin
                .geometry
                .as_ref()
                .and_then(|g| g.press_flash_lighten)
                .unwrap_or(0.25),
            app_selection_lerp_speed: skin
                .geometry
                .as_ref()
                .and_then(|g| g.app_selection_lerp_speed)
                .unwrap_or(0.25),
            sm_button_x: sm.and_then(|s| s.button_x).unwrap_or(4),
            sm_panel_x: sm.and_then(|s| s.panel_x).unwrap_or(2),
            app_title_bar_text_shadow: {
                let ap = skin.app_overrides.as_ref();
                ap.and_then(|a| a.title_bar_text_shadow)
                    .unwrap_or(skin.gradient_enabled == Some(true))
            },
            app_title_bar_text_shadow_color: {
                let ap = skin.app_overrides.as_ref();
                ov(
                    ap.and_then(|a| a.title_bar_text_shadow_color.as_ref()),
                    Color::rgba(0, 0, 0, 128),
                )
            },
            page_dot_lerp_speed: skin
                .geometry
                .as_ref()
                .and_then(|g| g.page_dot_lerp_speed)
                .unwrap_or(0.2),
            toast_margin: skin
                .geometry
                .as_ref()
                .and_then(|g| g.toast_margin)
                .unwrap_or(8),
            toast_height: skin
                .geometry
                .as_ref()
                .and_then(|g| g.toast_height)
                .unwrap_or(24),
            toast_width_fraction: skin
                .geometry
                .as_ref()
                .and_then(|g| g.toast_width_fraction)
                .unwrap_or(0.333),
            toast_gap: skin
                .geometry
                .as_ref()
                .and_then(|g| g.toast_gap)
                .unwrap_or(4),
            toast_slide_in: skin
                .geometry
                .as_ref()
                .and_then(|g| g.toast_slide_in)
                .unwrap_or(true),
            sm_item_separator: sm.and_then(|s| s.item_separator).unwrap_or(false),
            sm_item_separator_color: {
                let border = ov(
                    sm.and_then(|s| s.panel_border.as_ref()),
                    with_alpha(text, 40),
                );
                ov(
                    sm.and_then(|s| s.item_separator_color.as_ref()),
                    with_alpha(border, 64),
                )
            },
            app_themes: {
                let mut map = std::collections::HashMap::new();
                if let Some(ref themes) = skin.app_themes {
                    for (app, colors) in themes {
                        let mut parsed = std::collections::HashMap::new();
                        for (key, hex) in colors {
                            if let Some(c) = parse_hex_color(hex) {
                                parsed.insert(key.clone(), c);
                            }
                        }
                        if !parsed.is_empty() {
                            map.insert(app.clone(), parsed);
                        }
                    }
                }
                map
            },
            gradients: {
                let mut map = std::collections::HashMap::new();
                if let Some(ref grads) = skin.gradients {
                    for (name, preset) in grads {
                        if let (Some(from), Some(to)) =
                            (parse_hex_color(&preset.from), parse_hex_color(&preset.to))
                        {
                            map.insert(name.clone(), (from, to));
                        }
                    }
                }
                map
            },
            animations: {
                let mut map = std::collections::HashMap::new();
                if let Some(ref anims) = skin.animations {
                    for (name, preset) in anims {
                        map.insert(name.clone(), (preset.duration_ms, preset.easing.clone()));
                    }
                }
                map
            },
            widget_states: {
                let mut map = std::collections::HashMap::new();
                if let Some(ref states) = skin.widget_states {
                    for (widget, colors) in states {
                        let mut parsed = std::collections::HashMap::new();
                        for (key, hex) in colors {
                            if let Some(c) = parse_hex_color(hex) {
                                parsed.insert(key.clone(), c);
                            }
                        }
                        if !parsed.is_empty() {
                            map.insert(widget.clone(), parsed);
                        }
                    }
                }
                map
            },
            ui_theme: skin.to_ui_theme(),
        }
    }

    /// Set the screen dimensions and scale layout constants (builder pattern).
    ///
    /// Layout constants scale proportionally to `screen_w / 480`. At the PSP
    /// native resolution (480px) the base values are returned unchanged.
    pub fn with_screen_size(mut self, w: u32, h: u32) -> Self {
        self.screen_w = w;
        self.screen_h = h;

        // Scale layout constants proportionally to screen width.
        let scale = |base: i32| -> i32 { (base * w as i32 + 240) / 480 };
        let scale_u = |base: u32| -> u32 { (base * w + 240) / 480 };

        self.tab_w = self.tab_w_override.unwrap_or_else(|| scale(45));
        self.tab_h = self.tab_h_override.unwrap_or_else(|| scale(16));
        self.tab_gap = self.tab_gap_override.unwrap_or_else(|| scale(4));
        self.tab_start_x = self.tab_start_x_override.unwrap_or_else(|| scale(34));
        self.pipe_gap = scale(5);
        self.r_hint_w = scale(28);
        self.icon_stripe_h = scale_u(12);
        self.icon_fold_size = scale_u(10);
        self.icon_gfx_h = scale_u(22);
        self.icon_gfx_pad = scale_u(4);
        self.icon_label_pad = scale(4);

        // Scale dashboard grid and icon dimensions.
        self.grid_padding_x = scale(self.grid_padding_x as i32) as u16;
        self.grid_padding_y = scale(self.grid_padding_y as i32) as u16;
        self.icon_width = scale_u(self.icon_width);
        self.icon_height = scale_u(self.icon_height);
        self.cursor_pad = scale(self.cursor_pad);

        // Resolution-aware cursor scaling.
        self.cursor_scale = if w >= 1920 { 2 } else { 1 };

        self
    }

    /// Derive a gradient pair for a bar element.
    ///
    /// Returns `Some((top, bottom))` if gradient is enabled (either via explicit
    /// overrides or via `gradient_enabled`), or `None` for flat fill.
    fn bar_gradient_pair(
        skin: &SkinTheme,
        top_override: Option<&String>,
        bot_override: Option<&String>,
        base: Color,
    ) -> Option<(Color, Color)> {
        // Explicit overrides always win.
        if let (Some(t), Some(b)) = (
            top_override.and_then(|s| parse_hex_color(s)),
            bot_override.and_then(|s| parse_hex_color(s)),
        ) {
            return Some((t, b));
        }
        // Auto-derive when gradient_enabled is set.
        if skin.gradient_enabled == Some(true) {
            return Some((lighten(base, 0.15), base));
        }
        None
    }

    /// Look up a per-app color override.
    ///
    /// Returns `Some(color)` if `[app_themes.<app_name>]` defines the key,
    /// or `None` to fall back to the app's default.
    pub fn app_color(&self, app_name: &str, key: &str) -> Option<Color> {
        self.app_themes
            .get(app_name)
            .and_then(|m| m.get(key))
            .copied()
    }

    /// Look up a named gradient preset.
    ///
    /// Returns `Some((from_color, to_color))` if the gradient is defined.
    pub fn gradient(&self, name: &str) -> Option<(Color, Color)> {
        self.gradients.get(name).copied()
    }

    /// Look up a named animation preset.
    ///
    /// Returns `Some((duration_ms, easing_name))` if the animation is defined.
    pub fn animation(&self, name: &str) -> Option<(u32, &str)> {
        self.animations
            .get(name)
            .map(|(dur, easing)| (*dur, easing.as_str()))
    }

    /// Resolve a named animation to `(duration_ms, easing_fn)`.
    ///
    /// If the named animation isn't defined, returns `(default_ms, linear)`.
    pub fn resolve_animation(&self, name: &str, default_ms: u32) -> (u32, fn(f32) -> f32) {
        if let Some((dur, easing_name)) = self.animation(name) {
            (dur, super::theme::resolve_easing(easing_name))
        } else {
            (default_ms, oasis_ui::animation::easing::linear)
        }
    }

    /// Look up a per-widget state color override.
    ///
    /// Returns `Some(color)` if `[widget_states.<widget>]` defines the key,
    /// or `None` to fall back to the computed value.
    pub fn widget_state_color(&self, widget: &str, state_key: &str) -> Option<Color> {
        self.widget_states
            .get(widget)
            .and_then(|m| m.get(state_key))
            .copied()
    }

    /// Derive a 6-color palette from the primary color using hue-shifted offsets.
    ///
    /// The palette is: primary itself, a green-shifted variant, a warm variant,
    /// a purple variant, a lighter variant, and a reddish variant.
    fn derive_item_palette(primary: Color) -> Vec<Color> {
        vec![
            // Base primary (slightly desaturated).
            primary,
            // Green-shifted: reduce red, boost green.
            Color::rgb(
                primary.r.saturating_sub(20),
                primary.g.saturating_add(40),
                primary.b.saturating_sub(30),
            ),
            // Warm/gold: boost red+green, reduce blue.
            Color::rgb(
                primary.r.saturating_add(60),
                primary.g.saturating_add(20),
                primary.b.saturating_sub(60),
            ),
            // Purple-shifted: boost red+blue.
            Color::rgb(
                primary.r.saturating_add(30),
                primary.g.saturating_sub(40),
                primary.b.saturating_add(40),
            ),
            // Lighter variant.
            lighten(primary, 0.2),
            // Reddish variant.
            Color::rgb(
                primary.r.saturating_add(50),
                primary.g.saturating_sub(30),
                primary.b.saturating_sub(30),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_legacy_theme() {
        let at = ActiveTheme::default();
        assert_eq!(at.statusbar_bg, Color::rgba(0, 0, 0, 80));
        assert_eq!(at.bar_bg, Color::rgba(0, 0, 0, 90));
        assert_eq!(at.battery_color, Color::rgb(120, 255, 120));
        assert_eq!(at.icon_border_radius, 4);
        assert_eq!(at.cursor_border_radius, 6);
    }

    #[test]
    fn from_skin_derives_colors() {
        let skin = SkinTheme::default();
        let at = ActiveTheme::from_skin(&skin);
        // Primary is #3264C8 -- tab_active_fill should use primary with alpha 30.
        assert_eq!(at.tab_active_fill.a, 30);
        // Cursor color should use primary with alpha 80.
        assert_eq!(at.cursor_color.a, 80);
        // Text color drives version/clock.
        assert_eq!(at.version_color, skin.text_color());
        assert_eq!(at.clock_color, skin.text_color());
    }

    #[test]
    fn from_skin_respects_bar_overrides() {
        let toml = r##"
background = "#000000"
primary = "#FF0000"
[bar_overrides]
battery_color = "#00FF00"
tab_active_alpha = 200
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.battery_color, Color::rgb(0, 255, 0));
        assert_eq!(at.tab_active_alpha, 200);
    }

    #[test]
    fn from_skin_respects_icon_overrides() {
        let toml = r##"
[icon_overrides]
body_color = "#AABBCC"
cursor_border_radius = 10
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.icon_body_color, Color::rgb(0xAA, 0xBB, 0xCC));
        assert_eq!(at.cursor_border_radius, 10);
    }

    #[test]
    fn from_skin_custom_theme() {
        let toml = r##"
background = "#000000"
primary = "#FF0000"
secondary = "#333333"
text = "#00FF00"
dim_text = "#006600"
status_bar = "#111111"
prompt = "#00FF00"
output = "#00CC00"
error = "#FF0000"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        // Text-derived fields should be green.
        assert_eq!(at.clock_color, Color::rgb(0, 255, 0));
        assert_eq!(at.media_tab_active, Color::rgb(0, 255, 0));
    }

    // -- Per-app theme override tests --

    #[test]
    fn app_color_returns_none_by_default() {
        let at = ActiveTheme::default();
        assert!(at.app_color("tv_guide", "bg").is_none());
    }

    #[test]
    fn app_color_from_theme_toml() {
        let toml = r##"
[app_themes.tv_guide]
bg = "#0A1628"
grid_line = "#1A3A5C"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.app_color("tv_guide", "bg"), Some(Color::rgb(10, 22, 40)));
        assert_eq!(
            at.app_color("tv_guide", "grid_line"),
            Some(Color::rgb(26, 58, 92))
        );
        assert!(at.app_color("tv_guide", "missing").is_none());
        assert!(at.app_color("unknown", "bg").is_none());
    }

    // -- Named gradient preset tests --

    #[test]
    fn gradient_returns_none_by_default() {
        let at = ActiveTheme::default();
        assert!(at.gradient("primary").is_none());
    }

    #[test]
    fn gradient_from_theme_toml() {
        let toml = r##"
[gradients.primary]
from = "#0066FF"
to = "#0044AA"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        let (from, to) = at.gradient("primary").unwrap();
        assert_eq!(from, Color::rgb(0x00, 0x66, 0xFF));
        assert_eq!(to, Color::rgb(0x00, 0x44, 0xAA));
    }

    // -- Named animation preset tests --

    #[test]
    fn animation_returns_none_by_default() {
        let at = ActiveTheme::default();
        assert!(at.animation("button_press").is_none());
    }

    #[test]
    fn animation_from_theme_toml() {
        let toml = r##"
[animations.button_press]
duration_ms = 100
easing = "ease_out_quad"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        let (dur, easing) = at.animation("button_press").unwrap();
        assert_eq!(dur, 100);
        assert_eq!(easing, "ease_out_quad");
    }

    #[test]
    fn resolve_animation_uses_preset() {
        let toml = r##"
[animations.cursor_move]
duration_ms = 150
easing = "ease_out_cubic"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        let (dur, easing_fn) = at.resolve_animation("cursor_move", 200);
        assert_eq!(dur, 150);
        // Verify the easing function is ease_out_cubic (not linear).
        let val = easing_fn(0.5);
        assert!(val > 0.5, "ease_out_cubic at 0.5 should be > 0.5");
    }

    #[test]
    fn resolve_animation_falls_back() {
        let at = ActiveTheme::default();
        let (dur, easing_fn) = at.resolve_animation("nonexistent", 300);
        assert_eq!(dur, 300);
        // Default is linear.
        assert!((easing_fn(0.5) - 0.5).abs() < f32::EPSILON);
    }

    // -- Widget state color override tests --

    #[test]
    fn widget_state_color_returns_none_by_default() {
        let at = ActiveTheme::default();
        assert!(at.widget_state_color("button", "hover_bg").is_none());
    }

    #[test]
    fn widget_state_color_from_theme_toml() {
        let toml = r##"
[widget_states.button]
normal_bg = "#505050"
hover_bg = "#656565"
pressed_bg = "#353535"
disabled_text = "#555555"
"##;
        let skin: SkinTheme = toml::from_str(toml).unwrap();
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(
            at.widget_state_color("button", "hover_bg"),
            Some(Color::rgb(0x65, 0x65, 0x65))
        );
        assert_eq!(
            at.widget_state_color("button", "disabled_text"),
            Some(Color::rgb(0x55, 0x55, 0x55))
        );
        assert!(at.widget_state_color("button", "missing_key").is_none());
        assert!(at.widget_state_color("slider", "hover_bg").is_none());
    }
}
