//! Color derivation logic for `ActiveTheme`.
//!
//! Contains `from_skin()`, which derives all UI colors from a skin theme, and
//! `from_base_colors()`, a thin wrapper that builds a base-colors-only
//! `SkinTheme` and calls `from_skin()`.
//!
//! The `from_skin()` method delegates to focused helper methods for each
//! component: `derive_bar_theme`, `derive_icon_theme`, `derive_start_menu_theme`,
//! `derive_app_screen_theme`, `derive_osk_theme`, `derive_scrollbar_theme`,
//! `derive_wallpaper_theme` and `derive_toast_theme`, plus `TaskbarColors`,
//! `TransitionSettings`, `IconMotion`, `CursorSettings` and the `parse_*`
//! preset-table helpers for the flat `ActiveTheme` fields. `BarKeys` holds
//! the `[bar_overrides]` fallback rules (own key, then `text_color` /
//! the generic gradient pair). Layer derivation
//! (`derive_background_layers` / `_chrome_layers` / `_image_layers`) lives
//! in `layers.rs`. Derived text colors that the skin did not set explicitly
//! are kept readable on their surface with `theme::readable_on`.

use std::collections::HashMap;

use oasis_types::backend::Color;
use oasis_types::color::{darken, lighten, with_alpha};

use crate::SkinTheme;
use crate::theme::{BarOverrides, bar_backdrop, composite_over, parse_hex_color, readable_on};

use super::{
    ActiveTheme, AnsiPalette, AppScreenTheme, BarTheme, IconTheme, OskTheme, ScrollbarTheme,
    StartMenuTheme, ToastTheme, WallpaperTheme,
};

/// Parse an optional hex color override, falling back to `fallback`.
pub(super) fn ov(opt: Option<&String>, fallback: Color) -> Color {
    opt.and_then(|s| parse_hex_color(s)).unwrap_or(fallback)
}

/// Default mouse cursor arrow fill (matches the legacy white fill).
pub(crate) const CURSOR_FILL_DEFAULT: Color = Color::rgb(255, 255, 255);
/// Default mouse cursor arrow outline (matches the legacy black outline).
pub(crate) const CURSOR_OUTLINE_DEFAULT: Color = Color::rgb(0, 0, 0);
/// Default LED accent on the vector "data" icon.
pub(crate) const DATA_LED_DEFAULT: Color = Color::rgb(0, 200, 100);
/// Default start-menu item fallback color.
pub(crate) const ITEM_FALLBACK_DEFAULT: Color = Color::rgb(100, 100, 100);
/// Default background layer color when a layer omits `color`.
pub(crate) const LAYER_COLOR_DEFAULT: Color = Color::rgba(255, 255, 255, 18);

/// Default fallback colors cycled for discovered apps without an ICON0.
/// Must stay identical to the historical `FALLBACK_COLORS` literals.
pub(crate) fn default_app_fallback_colors() -> Vec<Color> {
    vec![
        Color::rgb(70, 130, 180),
        Color::rgb(60, 179, 113),
        Color::rgb(218, 165, 32),
        Color::rgb(178, 102, 178),
        Color::rgb(205, 92, 92),
        Color::rgb(100, 149, 237),
    ]
}

/// Derive a 16-color ANSI palette from the skin's base colors.
///
/// Deterministic and simple: the six chromatic hues use the standard
/// ANSI hue angles (red 0°, yellow 60°, green 120°, cyan 180°,
/// blue 240°, magenta 300°) tinted toward the skin — saturation comes
/// from the primary color, lightness from the terminal output color
/// (both clamped into readable bands). Bright variants lift lightness.
/// `black`/`bright_black` are primary-tinted darks; `white` is the
/// terminal output color and `bright_white` the skin text color.
pub(crate) fn derive_ansi_palette(primary: Color, output: Color, text: Color) -> AnsiPalette {
    use oasis_types::color::{hsl_to_rgb, rgb_to_hsl};

    let (ph, ps, _) = rgb_to_hsl(primary);
    let (_, _, ol) = rgb_to_hsl(output);

    // Clamp so grey primaries still yield distinguishable hues and dim
    // or very bright output colors stay readable.
    let s = ps.clamp(0.45, 0.90);
    let l = ol.clamp(0.35, 0.65);
    let lb = (l + 0.15).min(0.80);

    let normal = |h: f32| hsl_to_rgb(h, s, l);
    let bright = |h: f32| hsl_to_rgb(h, s, lb);

    AnsiPalette {
        colors: [
            hsl_to_rgb(ph, s * 0.5, 0.10), // black
            normal(0.0),                   // red
            normal(120.0),                 // green
            normal(60.0),                  // yellow
            normal(240.0),                 // blue
            normal(300.0),                 // magenta
            normal(180.0),                 // cyan
            output,                        // white
            hsl_to_rgb(ph, s * 0.5, 0.35), // bright_black
            bright(0.0),                   // bright_red
            bright(120.0),                 // bright_green
            bright(60.0),                  // bright_yellow
            bright(240.0),                 // bright_blue
            bright(300.0),                 // bright_magenta
            bright(180.0),                 // bright_cyan
            text,                          // bright_white
        ],
    }
}

/// Apply `[palette]` overrides on top of a derived ANSI palette.
fn apply_palette_overrides(
    mut palette: AnsiPalette,
    overrides: Option<&crate::theme::PaletteOverrides>,
) -> AnsiPalette {
    if let Some(p) = overrides {
        for (idx, slot) in palette.colors.iter_mut().enumerate() {
            if let Some(c) = p.slot(idx).and_then(|s| parse_hex_color(s)) {
                *slot = c;
            }
        }
    }
    palette
}

/// Taskbar colors (`[bar_overrides] taskbar_*`, else derived from primary).
struct TaskbarColors {
    bg: Color,
    btn_active: Color,
    btn_inactive: Color,
    btn_minimized: Color,
    btn_hover: Color,
    text: Color,
    separator: Color,
    indicator: Color,
}

impl TaskbarColors {
    fn derive(skin: &SkinTheme, primary: Color) -> Self {
        let bar_ov = skin.bar_overrides.as_ref();
        let pick = |get: fn(&BarOverrides) -> Option<&String>, fallback: Color| {
            ov(bar_ov.and_then(get), fallback)
        };
        Self {
            bg: pick(
                |b| b.taskbar_bg.as_ref(),
                with_alpha(darken(primary, 0.6), 120),
            ),
            btn_active: pick(|b| b.taskbar_btn_active.as_ref(), with_alpha(primary, 120)),
            btn_inactive: pick(
                |b| b.taskbar_btn_inactive.as_ref(),
                Color::rgba(255, 255, 255, 20),
            ),
            btn_minimized: pick(
                |b| b.taskbar_btn_minimized.as_ref(),
                Color::rgba(255, 255, 255, 10),
            ),
            btn_hover: pick(
                |b| b.taskbar_btn_hover.as_ref(),
                with_alpha(lighten(primary, 0.4), 60),
            ),
            text: pick(
                |b| b.taskbar_text_color.as_ref(),
                Color::rgba(255, 255, 255, 220),
            ),
            separator: pick(
                |b| b.taskbar_separator.as_ref(),
                Color::rgba(255, 255, 255, 50),
            ),
            indicator: pick(|b| b.taskbar_indicator.as_ref(), lighten(primary, 0.3)),
        }
    }
}

/// Screen-transition settings from `[transition]`.
struct TransitionSettings {
    fade_color: Color,
    entrance: String,
    /// Entrance length in 60 fps frames.
    entrance_frames: u32,
    page_style: String,
    easing: String,
}

impl TransitionSettings {
    fn derive(skin: &SkinTheme) -> Self {
        let tr = skin.transition.as_ref();
        Self {
            fade_color: tr
                .and_then(|t| t.fade_color.as_ref())
                .and_then(|s| parse_hex_color(s))
                .unwrap_or_else(|| darken(skin.background_color(), 0.3)),
            entrance: tr
                .and_then(|t| t.entrance.clone())
                .unwrap_or_else(|| "fade".to_string()),
            entrance_frames: tr
                .and_then(|t| t.entrance_ms)
                .map(|ms| (ms * 60 / 1000).max(1))
                .unwrap_or(45),
            page_style: tr
                .and_then(|t| t.page_style.clone())
                .unwrap_or_else(|| "slide".to_string()),
            easing: tr.and_then(|t| t.easing.clone()).unwrap_or_default(),
        }
    }
}

/// Dashboard icon entrance and focus motion from `[icon_overrides]`.
struct IconMotion {
    entrance_style: String,
    entrance_duration_ms: u32,
    entrance_stagger_ms: u32,
    focus_scale: f32,
    focus_glow: bool,
    focus_glow_color: Color,
}

impl IconMotion {
    fn derive(skin: &SkinTheme, primary: Color) -> Self {
        let ico = skin.icon_overrides.as_ref();
        Self {
            entrance_style: ico
                .and_then(|i| i.entrance_style.clone())
                .unwrap_or_else(|| "none".to_string()),
            entrance_duration_ms: ico.and_then(|i| i.entrance_duration_ms).unwrap_or(200),
            entrance_stagger_ms: ico.and_then(|i| i.entrance_stagger_ms).unwrap_or(30),
            focus_scale: ico.and_then(|i| i.focus_scale).unwrap_or(1.0),
            focus_glow: ico.and_then(|i| i.focus_glow).unwrap_or(false),
            focus_glow_color: ov(
                ico.and_then(|i| i.focus_glow_color.as_ref()),
                with_alpha(primary, 100),
            ),
        }
    }
}

/// Mouse cursor look from `[cursor]` (the scale comes from the screen).
struct CursorSettings {
    fill: Color,
    outline: Color,
    texture: Option<String>,
    hotspot: (i32, i32),
}

impl CursorSettings {
    fn derive(skin: &SkinTheme) -> Self {
        let cur = skin.cursor.as_ref();
        Self {
            fill: ov(cur.and_then(|c| c.fill.as_ref()), CURSOR_FILL_DEFAULT),
            outline: ov(cur.and_then(|c| c.outline.as_ref()), CURSOR_OUTLINE_DEFAULT),
            texture: cur.and_then(|c| c.texture.clone()),
            hotspot: cur
                .and_then(|c| c.hotspot)
                .map(|[x, y]| (x, y))
                .unwrap_or((0, 0)),
        }
    }
}

/// `[app_themes.<app>]` color tables; unparseable colors are dropped, and
/// apps left with none are omitted.
fn parse_app_themes(skin: &SkinTheme) -> HashMap<String, HashMap<String, Color>> {
    let Some(themes) = skin.app_themes.as_ref() else {
        return HashMap::new();
    };
    themes
        .iter()
        .filter_map(|(app_name, colors)| {
            let parsed: HashMap<String, Color> = colors
                .iter()
                .filter_map(|(key, hex)| parse_hex_color(hex).map(|c| (key.clone(), c)))
                .collect();
            (!parsed.is_empty()).then(|| (app_name.clone(), parsed))
        })
        .collect()
}

/// `[gradients.<name>]` presets whose endpoints both parse.
fn parse_gradients(skin: &SkinTheme) -> HashMap<String, (Color, Color)> {
    let Some(grads) = skin.gradients.as_ref() else {
        return HashMap::new();
    };
    grads
        .iter()
        .filter_map(|(name, preset)| {
            let from = parse_hex_color(&preset.from)?;
            let to = parse_hex_color(&preset.to)?;
            Some((name.clone(), (from, to)))
        })
        .collect()
}

/// `[animations.<name>]` presets as `(duration_ms, easing)`.
fn parse_animations(skin: &SkinTheme) -> HashMap<String, (u32, String)> {
    skin.animations
        .iter()
        .flatten()
        .map(|(name, preset)| (name.clone(), (preset.duration_ms, preset.easing.clone())))
        .collect()
}

/// A `[bar_overrides]` key: the field holding an element's color.
type BarKey = fn(&BarOverrides) -> Option<&String>;

/// `[bar_overrides]` lookups with the bars' fallback rules.
struct BarKeys<'a> {
    skin: &'a SkinTheme,
    overrides: Option<&'a BarOverrides>,
    /// Status-bar base color: the auto-derived gradients start from it.
    status_bar_color: Color,
}

impl<'a> BarKeys<'a> {
    fn new(skin: &'a SkinTheme, status_bar_color: Color) -> Self {
        Self {
            skin,
            overrides: skin.bar_overrides.as_ref(),
            status_bar_color,
        }
    }

    /// The raw value of `key`.
    fn get(&self, key: BarKey) -> Option<&'a String> {
        self.overrides.and_then(key)
    }

    /// An element color: its own key, else `fallback`.
    fn color(&self, key: BarKey, fallback: Color) -> Color {
        ov(self.get(key), fallback)
    }

    /// An element text color: its own key, else the generic `text_color`,
    /// else `fallback`.
    fn text(&self, key: BarKey, fallback: Color) -> Color {
        ov(
            self.get(key).or(self.get(|b| b.text_color.as_ref())),
            fallback,
        )
    }

    /// A bar's gradient: its own pair of keys, else the generic
    /// `gradient_top` / `gradient_bottom` pair (else auto-derived when the
    /// skin sets `gradient_enabled`).
    fn gradient(&self, top: BarKey, bottom: BarKey) -> Option<(Color, Color)> {
        ActiveTheme::bar_gradient_pair(
            self.skin,
            self.get(top).or(self.get(|b| b.gradient_top.as_ref())),
            self.get(bottom)
                .or(self.get(|b| b.gradient_bottom.as_ref())),
            self.status_bar_color,
        )
    }
}

impl ActiveTheme {
    /// Derive an `ActiveTheme` from the skin's base color palette.
    ///
    /// The 9 base colors (background, primary, secondary, text, dim_text,
    /// status_bar, prompt, output, error) drive all UI element colors.
    /// Fine-grained overrides (Phase 5) are checked first.
    ///
    /// Component themes come from the `derive_*_theme` helpers; this
    /// function assembles them with the taskbar colors, transition
    /// settings, `[geometry]` metrics and the named preset tables.
    pub fn from_skin(skin: &SkinTheme) -> Self {
        let status_bar_color =
            parse_hex_color(&skin.status_bar).unwrap_or(Color::rgba(0, 0, 0, 80));
        let primary = skin.primary_color();
        let secondary = skin.secondary_color();
        let text = skin.text_color();
        let dim = skin.dim_text_color();

        let bg_perf = skin.background_performance.as_ref();
        let geo = skin.geometry.as_ref();
        // `[geometry] <field>`, else `default`.
        macro_rules! geo {
            ($field:ident, $default:expr) => {
                geo.and_then(|g| g.$field).unwrap_or($default)
            };
        }

        let taskbar = TaskbarColors::derive(skin, primary);
        let transition = TransitionSettings::derive(skin);
        let motion = IconMotion::derive(skin, primary);
        let cursor = CursorSettings::derive(skin);
        let ansi = apply_palette_overrides(
            derive_ansi_palette(primary, skin.output_color(), text),
            skin.palette.as_ref(),
        );

        Self {
            ansi,
            cursor_fill: cursor.fill,
            cursor_outline: cursor.outline,

            // Component themes.
            bar: Self::derive_bar_theme(skin, status_bar_color, primary, secondary, text, dim),
            icon: Self::derive_icon_theme(skin, primary, text, dim),
            menu: Self::derive_start_menu_theme(skin, primary, text),
            app: Self::derive_app_screen_theme(skin, status_bar_color, primary, text, dim),
            osk: Self::derive_osk_theme(skin, primary, text, dim),
            scrollbar: Self::derive_scrollbar_theme(skin, secondary),
            wallpaper: Self::derive_wallpaper_theme(skin),
            toast: Self::derive_toast_theme(skin, primary, text),

            // Layers (layers.rs) and their performance budget.
            background_layers: Self::derive_background_layers(skin),
            chrome_layers: Self::derive_chrome_layers(skin),
            image_layers: Self::derive_image_layers(skin),
            background_max_layers: bg_perf.and_then(|p| p.max_layers).unwrap_or(8),
            background_reduced_motion: bg_perf.and_then(|p| p.reduced_motion).unwrap_or(false),
            background_complexity_budget: bg_perf.and_then(|p| p.complexity_budget).unwrap_or(200),

            // Dashboard icon motion and focus.
            entrance_style: motion.entrance_style,
            entrance_duration_ms: motion.entrance_duration_ms,
            entrance_stagger_ms: motion.entrance_stagger_ms,
            focus_scale: motion.focus_scale,
            focus_glow: motion.focus_glow,
            focus_glow_color: motion.focus_glow_color,

            // Taskbar.
            taskbar_height: geo!(taskbar_height, 20),
            taskbar_bg: taskbar.bg,
            taskbar_gradient_top: None,
            taskbar_gradient_bottom: None,
            taskbar_btn_active: taskbar.btn_active,
            taskbar_btn_inactive: taskbar.btn_inactive,
            taskbar_btn_minimized: taskbar.btn_minimized,
            taskbar_btn_hover: taskbar.btn_hover,
            taskbar_text_color: taskbar.text,
            taskbar_separator: taskbar.separator,
            taskbar_indicator: taskbar.indicator,

            // Layout metrics (`[geometry]`).
            grid_padding_x: geo!(grid_padding_x, 16),
            grid_padding_y: geo!(grid_padding_y, 6),
            terminal_border_radius: geo!(terminal_border_radius, 4),
            statusbar_height: geo!(statusbar_height, 24),
            bottombar_height: geo!(bottombar_height, 24),
            tab_row_height: geo!(tab_row_height, 18),
            icon_width: geo!(icon_width, 42),
            icon_height: geo!(icon_height, 52),
            font_small: geo!(font_small, 8),
            tab_w: 45,
            tab_h: 16,
            tab_gap: 4,
            tab_start_x: 34,
            pipe_gap: 5,
            r_hint_w: 28,
            icon_stripe_h: geo!(icon_stripe_h, 12),
            icon_fold_size: geo!(icon_fold_size, 10),
            icon_gfx_h: geo!(icon_gfx_h, 22),
            icon_gfx_pad: geo!(icon_gfx_pad, 4),
            icon_label_pad: geo!(icon_label_pad, 4),
            // Raw overrides, kept so `with_screen_size` scales only the
            // defaults.
            icon_stripe_h_override: geo.and_then(|g| g.icon_stripe_h),
            icon_fold_size_override: geo.and_then(|g| g.icon_fold_size),
            icon_gfx_h_override: geo.and_then(|g| g.icon_gfx_h),
            icon_gfx_pad_override: geo.and_then(|g| g.icon_gfx_pad),
            icon_label_pad_override: geo.and_then(|g| g.icon_label_pad),
            tab_w_override: geo.and_then(|g| g.tab_w).map(|v| v as i32),
            tab_h_override: geo.and_then(|g| g.tab_h).map(|v| v as i32),
            tab_gap_override: geo.and_then(|g| g.tab_gap).map(|v| v as i32),
            tab_start_x_override: geo.and_then(|g| g.tab_start_x),
            screen_w: 480,
            screen_h: 272,
            clear_color: darken(skin.background_color(), 0.5),
            terminal_line_height: geo!(terminal_line_height, 16),

            // Mouse cursor.
            cursor_scale: 1,
            cursor_texture: cursor.texture,
            cursor_hotspot: cursor.hotspot,

            // Screen transitions.
            transition_fade_color: transition.fade_color,
            transition_entrance: transition.entrance,
            transition_entrance_frames: transition.entrance_frames,
            transition_page_style: transition.page_style,
            transition_easing: transition.easing,

            // Fonts: accessibility `text_scale` multiplies the resolved
            // sizes globally (unchanged at the default 1.0).
            font_body: skin.scale_font(geo!(font_body, 12)),
            font_hint: skin.scale_font(geo!(font_hint, 10)),
            font_heading: skin.scale_font(geo!(font_heading, 14)),
            font_scale: geo
                .and_then(|g| g.font_scale)
                .map(|s| s.clamp(0.5, 3.0))
                .unwrap_or(1.0),

            // Animation tuning.
            terminal_cursor_blink_rate: geo!(cursor_blink_rate, 30),
            cursor_lerp_speed: geo!(cursor_lerp_speed, 0.35),
            page_slide_duration: geo!(page_slide_duration, 6),
            start_menu_anim_speed: geo!(start_menu_anim_speed, 0.25),
            press_flash_duration: geo!(press_flash_duration, 6),
            cursor_pad: geo!(cursor_pad, 3),
            press_flash_lighten: geo!(press_flash_lighten, 0.25),
            app_selection_lerp_speed: geo!(app_selection_lerp_speed, 0.25),
            page_dot_lerp_speed: geo!(page_dot_lerp_speed, 0.2),

            // Named preset tables.
            app_themes: parse_app_themes(skin),
            gradients: parse_gradients(skin),
            animations: parse_animations(skin),

            ui_theme: skin.to_ui_theme(),
            elevation: skin.elevation_ladder(),
        }
    }

    /// Derive the bar (status bar + bottom bar) theme from skin overrides.
    ///
    /// Each color is its `[bar_overrides]` key when set, else derived from
    /// the base colors. Text-like colors also fall back to the generic
    /// `text_color` override first, and both bars' gradients to the
    /// generic `gradient_top` / `gradient_bottom`. Derived (not explicit)
    /// text is then lifted to a readable contrast by
    /// [`Self::keep_bar_text_readable`].
    fn derive_bar_theme(
        skin: &SkinTheme,
        status_bar_color: Color,
        primary: Color,
        secondary: Color,
        text: Color,
        dim: Color,
    ) -> BarTheme {
        let keys = BarKeys::new(skin, status_bar_color);
        let bar_ov = keys.overrides;
        let pick = |key: BarKey, fallback: Color| keys.color(key, fallback);
        let pick_text = |key: BarKey, fallback: Color| keys.text(key, fallback);
        let statusbar_gradient = keys.gradient(
            |b| b.statusbar_gradient_top.as_ref(),
            |b| b.statusbar_gradient_bottom.as_ref(),
        );
        let bar_gradient = keys.gradient(
            |b| b.bar_gradient_top.as_ref(),
            |b| b.bar_gradient_bottom.as_ref(),
        );
        let tab_active_alpha = bar_ov.and_then(|b| b.tab_active_alpha).unwrap_or(180);
        let tab_inactive_alpha = bar_ov.and_then(|b| b.tab_inactive_alpha).unwrap_or(60);
        let media_tab_active = pick(|b| b.media_tab_active.as_ref(), text);
        let media_tab_inactive = pick(|b| b.media_tab_inactive.as_ref(), dim);

        let mut bar = BarTheme {
            // Backgrounds.
            statusbar_bg: pick(
                |b| b.statusbar_bg.as_ref(),
                with_alpha(status_bar_color, 80),
            ),
            bg: pick(|b| b.bar_bg.as_ref(), with_alpha(status_bar_color, 90)),
            separator_color: pick(|b| b.separator_color.as_ref(), with_alpha(secondary, 50)),
            statusbar_gradient_top: statusbar_gradient.map(|(t, _)| t),
            statusbar_gradient_bottom: statusbar_gradient.map(|(_, b)| b),
            gradient_top: bar_gradient.map(|(t, _)| t),
            gradient_bottom: bar_gradient.map(|(_, b)| b),

            // Status-bar and footer text.
            battery_color: pick_text(|b| b.battery_color.as_ref(), lighten(primary, 0.3)),
            version_color: pick_text(|b| b.version_color.as_ref(), text),
            clock_color: pick_text(|b| b.clock_color.as_ref(), text),
            clock_offset_y: bar_ov.and_then(|b| b.clock_offset_y).unwrap_or(0),
            url_color: pick_text(|b| b.url_color.as_ref(), dim),
            usb_color: pick_text(|b| b.usb_color.as_ref(), dim),
            pipe_color: pick_text(|b| b.pipe_color.as_ref(), with_alpha(text, 60)),
            r_hint_color: pick_text(|b| b.r_hint_color.as_ref(), with_alpha(text, 140)),
            category_label_color: pick_text(
                |b| b.category_label_color.as_ref(),
                with_alpha(text, 220),
            ),
            text_shadow: bar_ov
                .and_then(|b| b.text_shadow)
                .unwrap_or(skin.gradient_enabled == Some(true)),
            text_shadow_color: pick(|b| b.text_shadow_color.as_ref(), Color::rgba(0, 0, 0, 128)),
            version_text: bar_ov
                .and_then(|b| b.version_text.clone())
                .unwrap_or_else(|| "Version 0.1".to_string()),
            category_label: bar_ov
                .and_then(|b| b.category_label.clone())
                .unwrap_or_else(|| "OSS".to_string()),
            url_text: bar_ov.and_then(|b| b.url_text.clone()).unwrap_or_default(),

            // Tabs. Top tab-row text follows the media-tab colors unless a
            // skin decouples them (e.g. dark top tabs over a light footer).
            tab_active_fill: pick(|b| b.tab_active_fill.as_ref(), with_alpha(primary, 30)),
            tab_inactive_fill: Color::rgba(0, 0, 0, 0),
            tab_active_alpha,
            tab_inactive_alpha,
            tab_active_stroke: with_alpha(text, tab_active_alpha),
            tab_inactive_stroke: with_alpha(text, tab_inactive_alpha),
            media_tab_active,
            media_tab_inactive,
            tab_text_active: pick(|b| b.tab_text_active.as_ref(), media_tab_active),
            tab_text_inactive: pick(|b| b.tab_text_inactive.as_ref(), media_tab_inactive),
            tab_texture_active: bar_ov.and_then(|b| b.tab_texture_active.clone()),
            tab_texture_inactive: bar_ov.and_then(|b| b.tab_texture_inactive.clone()),

            // Page dots.
            page_dot_active: pick(|b| b.page_dot_active.as_ref(), with_alpha(text, 200)),
            page_dot_inactive: pick(|b| b.page_dot_inactive.as_ref(), with_alpha(text, 50)),

            // Media dock (`bottombar_style = "media_dock"`).
            dock_button_fill: pick(|b| b.dock_button_fill.as_ref(), with_alpha(primary, 30)),
            dock_button_glyph: pick_text(|b| b.dock_button_glyph.as_ref(), with_alpha(text, 220)),
            dock_progress_track: pick(|b| b.dock_progress_track.as_ref(), with_alpha(text, 40)),
            dock_progress_fill: pick(|b| b.dock_progress_fill.as_ref(), with_alpha(primary, 200)),
            dock_vol_track: pick(|b| b.dock_vol_track.as_ref(), with_alpha(text, 40)),
            dock_vol_fill: pick(|b| b.dock_vol_fill.as_ref(), with_alpha(secondary, 160)),
        };
        Self::keep_bar_text_readable(&mut bar, skin);
        bar
    }

    /// Lift the derived URL and battery colors to a 3:1 contrast on the
    /// bar each is drawn over. Colors the skin set explicitly (their own
    /// key or `text_color`) are left alone; `skin lint` reports those.
    fn keep_bar_text_readable(bar: &mut BarTheme, skin: &SkinTheme) {
        let bar_ov = skin.bar_overrides.as_ref();
        let bar_text = bar_ov.and_then(|b| b.text_color.as_ref());
        let explicit = |field: Option<&String>| {
            field
                .or(bar_text)
                .and_then(|s| parse_hex_color(s))
                .is_some()
        };
        let bg = skin.background_color();
        if !explicit(bar_ov.and_then(|b| b.url_color.as_ref())) {
            let backdrop = bar_backdrop(bar.bg, bar.gradient_top, bar.gradient_bottom, bg);
            bar.url_color = readable_on(bar.url_color, backdrop, 3.0);
        }
        if !explicit(bar_ov.and_then(|b| b.battery_color.as_ref())) {
            let backdrop = bar_backdrop(
                bar.statusbar_bg,
                bar.statusbar_gradient_top,
                bar.statusbar_gradient_bottom,
                bg,
            );
            bar.battery_color = readable_on(bar.battery_color, backdrop, 3.0);
        }
    }

    /// Derive the icon theme from skin overrides.
    fn derive_icon_theme(skin: &SkinTheme, primary: Color, text: Color, dim: Color) -> IconTheme {
        let ico = skin.icon_overrides.as_ref();

        let icon_label_color = ov(
            ico.and_then(|i| i.label_color.as_ref()),
            with_alpha(text, 230),
        );
        let icon_label_shadow = if let Some(c) = ico
            .and_then(|i| i.label_shadow_color.as_ref())
            .and_then(|s| parse_hex_color(s))
        {
            Some(c)
        } else {
            let brightness = icon_label_color.r as u16 * 3 / 10
                + icon_label_color.g as u16 * 6 / 10
                + icon_label_color.b as u16 / 10;
            if brightness > 140 {
                Some(Color::rgba(0, 0, 0, 120))
            } else {
                None
            }
        };

        IconTheme {
            body_color: ov(ico.and_then(|i| i.body_color.as_ref()), text),
            fold_color: ov(ico.and_then(|i| i.fold_color.as_ref()), dim),
            outline_color: ov(
                ico.and_then(|i| i.outline_color.as_ref()),
                with_alpha(text, 180),
            ),
            shadow_color: ov(
                ico.and_then(|i| i.shadow_color.as_ref()),
                Color::rgba(0, 0, 0, 70),
            ),
            label_color: icon_label_color,
            label_shadow: icon_label_shadow,
            cursor_color: ov(
                ico.and_then(|i| i.cursor_color.as_ref()),
                with_alpha(primary, 80),
            ),
            border_radius: ico
                .and_then(|i| i.icon_border_radius)
                .unwrap_or_else(|| skin.border_radius.unwrap_or(4)),
            cursor_border_radius: ico
                .and_then(|i| i.cursor_border_radius)
                .unwrap_or_else(|| skin.border_radius.map(|r| r + 2).unwrap_or(6)),
            cursor_stroke_width: ico.and_then(|i| i.cursor_stroke_width).unwrap_or(2),
            style: ico
                .and_then(|i| i.icon_style.clone())
                .unwrap_or_else(|| "document".to_string()),
            cursor_style: ico
                .and_then(|i| i.cursor_style.clone())
                .unwrap_or_else(|| "stroke".to_string()),
            gfx_anchor: ico
                .and_then(|i| i.gfx_anchor.clone())
                .unwrap_or_else(|| "top".to_string()),
            shadow_level: skin
                .geometry
                .as_ref()
                .and_then(|g| g.icon_shadow_level)
                .unwrap_or(1),
            vector_preset: ico
                .and_then(|i| i.vector_preset.clone())
                .unwrap_or_else(|| "altimit".to_string()),
            idle_float: ico.and_then(|i| i.vector_idle_float).unwrap_or(false),
            float_amplitude: ico.and_then(|i| i.vector_float_amplitude).unwrap_or(2.0),
            float_speed: ico.and_then(|i| i.vector_float_speed).unwrap_or(0.04),
            spin_enabled: ico.and_then(|i| i.vector_spin_enabled).unwrap_or(false),
            spin_speed: ico.and_then(|i| i.vector_spin_speed).unwrap_or(0.03),
            pulse_enabled: ico.and_then(|i| i.vector_pulse_enabled).unwrap_or(false),
            pulse_speed: ico.and_then(|i| i.vector_pulse_speed).unwrap_or(0.06),
            blink_enabled: ico.and_then(|i| i.vector_blink_enabled).unwrap_or(false),
            blink_interval: ico.and_then(|i| i.vector_blink_interval).unwrap_or(45),
            container_style: ico
                .and_then(|i| i.icon_container.clone())
                .unwrap_or_else(|| "none".to_string()),
            container_padding: ico.and_then(|i| i.icon_container_padding).unwrap_or(3),
            data_led_color: ov(
                ico.and_then(|i| i.data_led_color.as_ref()),
                DATA_LED_DEFAULT,
            ),
            fallback_colors: ico
                .and_then(|i| i.fallback_colors.as_ref())
                .map(|colors| {
                    colors
                        .iter()
                        .filter_map(|s| parse_hex_color(s))
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty())
                .unwrap_or_else(default_app_fallback_colors),
        }
    }

    /// Derive the start menu theme from skin overrides.
    fn derive_start_menu_theme(skin: &SkinTheme, primary: Color, text: Color) -> StartMenuTheme {
        let sm = skin.start_menu_overrides.as_ref();

        StartMenuTheme {
            panel_bg: ov(
                sm.and_then(|s| s.panel_bg.as_ref()),
                Color::rgba(20, 20, 35, 220),
            ),
            panel_gradient_top: sm
                .and_then(|s| s.panel_gradient_top.as_ref())
                .and_then(|s| parse_hex_color(s)),
            panel_gradient_bottom: sm
                .and_then(|s| s.panel_gradient_bottom.as_ref())
                .and_then(|s| parse_hex_color(s)),
            panel_border: ov(
                sm.and_then(|s| s.panel_border.as_ref()),
                with_alpha(text, 40),
            ),
            item_text: ov(sm.and_then(|s| s.item_text.as_ref()), with_alpha(text, 220)),
            item_text_active: ov(sm.and_then(|s| s.item_text_active.as_ref()), text),
            highlight_color: ov(
                sm.and_then(|s| s.highlight_color.as_ref()),
                with_alpha(primary, 80),
            ),
            button_bg: ov(
                sm.and_then(|s| s.button_bg.as_ref()),
                with_alpha(primary, 200),
            ),
            button_text: ov(sm.and_then(|s| s.button_text.as_ref()), text),
            panel_border_radius: sm
                .and_then(|s| s.panel_border_radius)
                .unwrap_or_else(|| skin.border_radius.unwrap_or(4)),
            panel_shadow_level: sm.and_then(|s| s.panel_shadow_level).unwrap_or(1),
            layout_mode: sm
                .and_then(|s| s.layout_mode.clone())
                .unwrap_or_else(|| "grid".to_string()),
            button_label: sm
                .and_then(|s| s.button_label.clone())
                .unwrap_or_else(|| "START".to_string()),
            button_width: sm.and_then(|s| s.button_width).unwrap_or(48),
            button_height: sm.and_then(|s| s.button_height).unwrap_or(18),
            button_shape: sm
                .and_then(|s| s.button_shape.clone())
                .unwrap_or_else(|| "pill".to_string()),
            panel_width: sm.and_then(|s| s.panel_width).unwrap_or(200),
            columns: sm.and_then(|s| s.columns).unwrap_or(2).max(1),
            button_gradient_top: sm
                .and_then(|s| s.button_gradient_top.as_ref())
                .and_then(|s| parse_hex_color(s)),
            button_gradient_bottom: sm
                .and_then(|s| s.button_gradient_bottom.as_ref())
                .and_then(|s| parse_hex_color(s)),
            header_text: sm.and_then(|s| s.header_text.clone()),
            header_bg: ov(
                sm.and_then(|s| s.header_bg.as_ref()),
                Color::rgba(30, 30, 50, 240),
            ),
            header_text_color: ov(sm.and_then(|s| s.header_text_color.as_ref()), text),
            header_height: sm.and_then(|s| s.header_height).unwrap_or(0),
            footer_enabled: sm.and_then(|s| s.footer_enabled).unwrap_or(false),
            footer_bg: ov(
                sm.and_then(|s| s.footer_bg.as_ref()),
                Color::rgba(30, 30, 50, 240),
            ),
            footer_text_color: ov(sm.and_then(|s| s.footer_text_color.as_ref()), text),
            footer_height: sm.and_then(|s| s.footer_height).unwrap_or(0),
            item_icon_size: sm.and_then(|s| s.item_icon_size).unwrap_or(14),
            item_row_height: sm.and_then(|s| s.item_row_height).unwrap_or(22).max(1),
            item_colors: sm
                .and_then(|s| s.item_colors.as_ref())
                .map(|colors| {
                    colors
                        .iter()
                        .filter_map(|s| parse_hex_color(s))
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| Self::derive_item_palette(primary)),
            pad_inner: sm.and_then(|s| s.pad_inner).unwrap_or(8),
            footer_text: sm
                .and_then(|s| s.footer_text.clone())
                .unwrap_or_else(|| "Log Off  Shut Down".to_string()),
            button_x: sm.and_then(|s| s.button_x).unwrap_or(4),
            panel_x: sm.and_then(|s| s.panel_x).unwrap_or(2),
            item_separator: sm.and_then(|s| s.item_separator).unwrap_or(false),
            item_fallback_color: ov(
                sm.and_then(|s| s.item_fallback_color.as_ref()),
                ITEM_FALLBACK_DEFAULT,
            ),
            item_separator_color: {
                let border = ov(
                    sm.and_then(|s| s.panel_border.as_ref()),
                    with_alpha(text, 40),
                );
                ov(
                    sm.and_then(|s| s.item_separator_color.as_ref()),
                    with_alpha(border, 64),
                )
            },
        }
    }

    /// Derive the app screen theme from skin overrides.
    fn derive_app_screen_theme(
        skin: &SkinTheme,
        status_bar_color: Color,
        primary: Color,
        text: Color,
        dim: Color,
    ) -> AppScreenTheme {
        let ap = skin.app_overrides.as_ref();

        let mut app = AppScreenTheme {
            bg: ov(
                ap.and_then(|a| a.app_bg.as_ref()),
                lighten(skin.background_color(), 0.02),
            ),
            divider: ov(
                ap.and_then(|a| a.divider_color.as_ref()),
                lighten(skin.background_color(), 0.15),
            ),
            selected_text: ov(
                ap.and_then(|a| a.selected_text.as_ref()),
                lighten(primary, 0.3),
            ),
            text: ov(ap.and_then(|a| a.text_color.as_ref()), lighten(dim, 0.2)),
            dim_text: ov(ap.and_then(|a| a.dim_text.as_ref()), dim),
            title_bar_bg: ov(
                ap.and_then(|a| a.title_bar_bg.as_ref()),
                lighten(skin.background_color(), 0.08),
            ),
            title_bar_text: ov(ap.and_then(|a| a.title_bar_text.as_ref()), text),
            title_bar_height: ap.and_then(|a| a.title_bar_height).unwrap_or(22),
            terminal_output_color: ov(
                ap.and_then(|a| a.terminal_output_color.as_ref()),
                skin.output_color(),
            ),
            terminal_prompt_color: ov(
                ap.and_then(|a| a.terminal_prompt_color.as_ref()),
                skin.prompt_color(),
            ),
            input_border_radius: ap.and_then(|a| a.input_border_radius).unwrap_or_else(|| {
                skin.geometry
                    .as_ref()
                    .and_then(|g| g.terminal_border_radius)
                    .unwrap_or(4)
            }),
            selected_bg: with_alpha(primary, 40),
            selection_border_radius: ap.and_then(|a| a.selection_border_radius).unwrap_or(2),
            selection_accent_color: ov(
                ap.and_then(|a| a.selection_accent_color.as_ref()),
                with_alpha(primary, 128),
            ),
            title_bar_gradient_top: {
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
            title_bar_gradient_bottom: {
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
            title_bar_text_shadow: ap
                .and_then(|a| a.title_bar_text_shadow)
                .unwrap_or(skin.gradient_enabled == Some(true)),
            title_bar_text_shadow_color: ov(
                ap.and_then(|a| a.title_bar_text_shadow_color.as_ref()),
                Color::rgba(0, 0, 0, 128),
            ),
        };

        // Derived (not skin-specified) app text must stay readable on the
        // app surface: `lighten(primary)` selection text washes out on
        // light skins, `lighten(dim)` body text on near-white app colors.
        let explicit = |field: Option<&String>| field.and_then(|s| parse_hex_color(s)).is_some();
        let surface = composite_over(app.bg, skin.background_color());
        if !explicit(ap.and_then(|a| a.selected_text.as_ref())) {
            let selection = composite_over(app.selected_bg, surface);
            app.selected_text = readable_on(app.selected_text, selection, 3.0);
        }
        if !explicit(ap.and_then(|a| a.text_color.as_ref())) {
            app.text = readable_on(app.text, surface, 4.5);
        }
        app
    }

    /// Derive the on-screen keyboard theme from skin overrides.
    fn derive_osk_theme(skin: &SkinTheme, primary: Color, text: Color, dim: Color) -> OskTheme {
        let ok = skin.osk_overrides.as_ref();

        OskTheme {
            key_bg: ov(
                ok.and_then(|o| o.key_bg.as_ref()),
                with_alpha(lighten(skin.background_color(), 0.05), 220),
            ),
            key_text: ov(ok.and_then(|o| o.key_text.as_ref()), text),
            key_focus: ov(ok.and_then(|o| o.key_focus.as_ref()), lighten(primary, 0.3)),
            key_active: ov(ok.and_then(|o| o.key_active.as_ref()), primary),
            key_dim_text: ov(ok.and_then(|o| o.key_dim_text.as_ref()), dim),
        }
    }

    /// Derive the scrollbar theme from skin overrides.
    fn derive_scrollbar_theme(skin: &SkinTheme, secondary: Color) -> ScrollbarTheme {
        let sb = skin.scrollbar_overrides.as_ref();

        ScrollbarTheme {
            track_color: ov(
                sb.and_then(|s| s.track_color.as_ref()),
                with_alpha(secondary, 20),
            ),
            thumb_color: ov(
                sb.and_then(|s| s.thumb_color.as_ref()),
                with_alpha(secondary, 100),
            ),
            thumb_hover_color: ov(
                sb.and_then(|s| s.thumb_hover_color.as_ref()),
                with_alpha(secondary, 160),
            ),
            width: sb
                .and_then(|s| s.width)
                .or_else(|| skin.geometry.as_ref().and_then(|g| g.scrollbar_width))
                .unwrap_or(6),
            border_radius: skin
                .geometry
                .as_ref()
                .and_then(|g| g.scrollbar_border_radius)
                .unwrap_or(3),
        }
    }

    /// Derive the wallpaper theme from skin configuration.
    fn derive_wallpaper_theme(skin: &SkinTheme) -> WallpaperTheme {
        WallpaperTheme {
            style: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.style.clone())
                .unwrap_or_else(|| "gradient".to_string()),
            stops: skin
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
            wave: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.wave_enabled)
                .unwrap_or(true),
            wave_intensity: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.wave_intensity)
                .unwrap_or(1.0),
            angle: skin.wallpaper.as_ref().and_then(|w| w.angle).unwrap_or(0.0),
            grid_spacing: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.grid_spacing)
                .unwrap_or(16),
            grid_color: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.grid_color.as_ref())
                .and_then(|s| parse_hex_color(s))
                .unwrap_or_else(|| lighten(skin.background_color(), 0.08)),
            noise_intensity: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.noise_intensity)
                .unwrap_or(0.3),
            animated: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.animated)
                .unwrap_or(false),
            source: skin.wallpaper.as_ref().and_then(|w| w.source.clone()),
            fit: skin
                .wallpaper
                .as_ref()
                .and_then(|w| w.fit.clone())
                .unwrap_or_else(|| "cover".to_string()),
        }
    }

    /// Derive the toast notification theme from skin configuration.
    fn derive_toast_theme(skin: &SkinTheme, primary: Color, text: Color) -> ToastTheme {
        ToastTheme {
            info_bg: with_alpha(primary, 220),
            success_bg: skin
                .success
                .as_ref()
                .and_then(|s| parse_hex_color(s))
                .map(|c| with_alpha(c, 220))
                .unwrap_or(Color::rgba(60, 180, 100, 220)),
            error_bg: with_alpha(skin.error_color(), 220),
            warning_bg: skin
                .warning
                .as_ref()
                .and_then(|s| parse_hex_color(s))
                .map(|c| with_alpha(c, 220))
                .unwrap_or(Color::rgba(230, 170, 40, 220)),
            text_color: text,
            border_radius: skin
                .geometry
                .as_ref()
                .and_then(|g| g.terminal_border_radius)
                .unwrap_or(4),
            ttl: 180,
            text_shadow: skin.gradient_enabled == Some(true),
            shadow_level: 1,
            fade_frames: skin
                .geometry
                .as_ref()
                .and_then(|g| g.toast_fade_frames)
                .unwrap_or(10),
            margin: skin
                .geometry
                .as_ref()
                .and_then(|g| g.toast_margin)
                .unwrap_or(8),
            height: skin
                .geometry
                .as_ref()
                .and_then(|g| g.toast_height)
                .unwrap_or(24),
            width_fraction: skin
                .geometry
                .as_ref()
                .and_then(|g| g.toast_width_fraction)
                .unwrap_or(0.333),
            gap: skin
                .geometry
                .as_ref()
                .and_then(|g| g.toast_gap)
                .unwrap_or(4),
            slide_in: skin
                .geometry
                .as_ref()
                .and_then(|g| g.toast_slide_in)
                .unwrap_or(true),
        }
    }

    /// Derive an `ActiveTheme` directly from 9 base `Color` values.
    ///
    /// Exactly [`Self::from_skin`] on a [`SkinTheme`] whose 9 base colors are
    /// these, with `shadow_intensity = 3` and every other override unset --
    /// one derivation, no copy to keep in sync (a full-equality test guards
    /// this). `SkinTheme` is a
    /// plain struct, so this needs no TOML parsing; it is what the PSP
    /// backend and the dashboard's PSP-preset path use.
    ///
    /// # Arguments
    /// * `background`  - Main background / wallpaper base
    /// * `primary`     - Primary accent (highlights, active elements)
    /// * `secondary`   - Secondary accent (borders, separators)
    /// * `text`        - Default text color
    /// * `dim_text`    - Dimmed / secondary text
    /// * `status_bar`  - Status bar background
    /// * `prompt`      - Terminal prompt color
    /// * `output`      - Terminal output text color
    /// * `error`       - Error / danger color
    #[allow(clippy::too_many_arguments)]
    pub fn from_base_colors(
        background: Color,
        primary: Color,
        secondary: Color,
        text: Color,
        dim_text: Color,
        status_bar: Color,
        prompt: Color,
        output: Color,
        error: Color,
    ) -> Self {
        fn hex(c: Color) -> String {
            format!("#{:02X}{:02X}{:02X}{:02X}", c.r, c.g, c.b, c.a)
        }
        Self::from_skin(&SkinTheme {
            background: hex(background),
            primary: hex(primary),
            secondary: hex(secondary),
            text: hex(text),
            dim_text: hex(dim_text),
            status_bar: hex(status_bar),
            prompt: hex(prompt),
            output: hex(output),
            error: hex(error),
            // The dropdown / modal / tooltip elevation (2 / 3 / 2) this
            // constructor has always produced.
            shadow_intensity: Some(3),
            ..SkinTheme::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::IconOverrides;

    fn skin_with_bar(bar: BarOverrides) -> SkinTheme {
        SkinTheme {
            bar_overrides: Some(bar),
            ..SkinTheme::default()
        }
    }

    /// Text colors: own key, then `text_color`, then the fallback; plain
    /// element colors ignore `text_color`.
    #[test]
    fn bar_keys_text_falls_back_through_text_color() {
        let skin = skin_with_bar(BarOverrides {
            text_color: Some("#112233".into()),
            url_color: Some("#445566".into()),
            ..BarOverrides::default()
        });
        let keys = BarKeys::new(&skin, Color::BLACK);
        let fallback = Color::rgb(9, 9, 9);
        assert_eq!(
            keys.text(|b| b.url_color.as_ref(), fallback),
            Color::rgb(0x44, 0x55, 0x66)
        );
        assert_eq!(
            keys.text(|b| b.clock_color.as_ref(), fallback),
            Color::rgb(0x11, 0x22, 0x33)
        );
        assert_eq!(keys.color(|b| b.bar_bg.as_ref(), fallback), fallback);
        let none = SkinTheme::default();
        let keys = BarKeys::new(&none, Color::BLACK);
        assert_eq!(keys.text(|b| b.clock_color.as_ref(), fallback), fallback);
    }

    /// A bar's own gradient pair wins over the generic pair; with neither
    /// the gradient is auto-derived only when `gradient_enabled`.
    #[test]
    fn bar_keys_gradient_precedence() {
        let skin = skin_with_bar(BarOverrides {
            gradient_top: Some("#010101".into()),
            gradient_bottom: Some("#020202".into()),
            bar_gradient_top: Some("#0A0A0A".into()),
            bar_gradient_bottom: Some("#0B0B0B".into()),
            ..BarOverrides::default()
        });
        let keys = BarKeys::new(&skin, Color::BLACK);
        let bar = keys.gradient(
            |b| b.bar_gradient_top.as_ref(),
            |b| b.bar_gradient_bottom.as_ref(),
        );
        assert_eq!(bar, Some((Color::rgb(10, 10, 10), Color::rgb(11, 11, 11))));
        let status = keys.gradient(
            |b| b.statusbar_gradient_top.as_ref(),
            |b| b.statusbar_gradient_bottom.as_ref(),
        );
        assert_eq!(status, Some((Color::rgb(1, 1, 1), Color::rgb(2, 2, 2))));

        let generic = |keys: &BarKeys<'_>| {
            keys.gradient(|b| b.gradient_top.as_ref(), |b| b.gradient_bottom.as_ref())
        };
        let plain = SkinTheme::default();
        assert_eq!(
            generic(&BarKeys::new(&plain, Color::rgb(100, 100, 100))),
            None
        );
        let auto = SkinTheme {
            gradient_enabled: Some(true),
            ..SkinTheme::default()
        };
        let (top, bottom) =
            generic(&BarKeys::new(&auto, Color::rgb(100, 100, 100))).expect("auto-derived");
        assert_eq!(bottom, Color::rgb(100, 100, 100));
        assert!(top.r > bottom.r);
    }

    #[test]
    fn icon_motion_and_cursor_defaults() {
        let skin = SkinTheme::default();
        let primary = Color::rgb(10, 20, 30);
        let motion = IconMotion::derive(&skin, primary);
        assert_eq!(motion.entrance_style, "none");
        assert_eq!(
            (motion.entrance_duration_ms, motion.entrance_stagger_ms),
            (200, 30)
        );
        assert!(!motion.focus_glow);
        assert_eq!(motion.focus_glow_color, with_alpha(primary, 100));
        let cursor = CursorSettings::derive(&skin);
        assert_eq!(cursor.fill, CURSOR_FILL_DEFAULT);
        assert_eq!(cursor.outline, CURSOR_OUTLINE_DEFAULT);
        assert_eq!(cursor.hotspot, (0, 0));
        assert!(cursor.texture.is_none());
    }

    /// `label_shadow_color` replaces the brightness-based default shadow.
    #[test]
    fn icon_label_shadow_override() {
        let mut skin = SkinTheme {
            icon_overrides: Some(IconOverrides {
                label_color: Some("#FFFFFF".into()),
                ..IconOverrides::default()
            }),
            ..SkinTheme::default()
        };
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.icon.label_shadow, Some(Color::rgba(0, 0, 0, 120)));
        if let Some(ico) = skin.icon_overrides.as_mut() {
            ico.label_shadow_color = Some("#000000".into());
        }
        let at = ActiveTheme::from_skin(&skin);
        assert_eq!(at.icon.label_shadow, Some(Color::rgb(0, 0, 0)));
    }
}
