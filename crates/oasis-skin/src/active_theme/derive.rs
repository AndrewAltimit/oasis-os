//! Color derivation logic for `ActiveTheme`.
//!
//! Contains `from_skin()` and `from_base_colors()` -- the two constructors
//! that derive all UI colors from a base palette.
//!
//! The `from_skin()` method delegates to focused helper methods for each
//! component: `derive_bar_theme`, `derive_icon_theme`, `derive_start_menu_theme`,
//! `derive_app_screen_theme`, `derive_osk_theme`, `derive_scrollbar_theme`,
//! `derive_wallpaper_theme`, `derive_toast_theme`, and `derive_background_layers`.

use oasis_types::backend::Color;
use oasis_types::color::{darken, lighten, with_alpha};

use crate::SkinTheme;
use crate::theme::parse_hex_color;

use super::{
    ActiveTheme, AppScreenTheme, BarTheme, IconTheme, ImageLayerTheme, OskTheme, ScrollbarTheme,
    StartMenuTheme, ToastTheme, WallpaperTheme,
};

/// Parse an optional hex color override, falling back to `fallback`.
fn ov(opt: Option<&String>, fallback: Color) -> Color {
    opt.and_then(|s| parse_hex_color(s)).unwrap_or(fallback)
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

        let bar = Self::derive_bar_theme(skin, status_bar_color, primary, secondary, text, dim);
        let icon = Self::derive_icon_theme(skin, primary, text, dim);
        let menu = Self::derive_start_menu_theme(skin, primary, text);
        let app_screen = Self::derive_app_screen_theme(skin, status_bar_color, primary, text, dim);
        let osk_theme = Self::derive_osk_theme(skin, primary, text, dim);
        let scrollbar_theme = Self::derive_scrollbar_theme(skin, secondary);
        let wallpaper_theme = Self::derive_wallpaper_theme(skin);
        let toast_theme = Self::derive_toast_theme(skin, primary, text);
        let background_layers = Self::derive_background_layers(skin);
        let chrome_layers = Self::derive_chrome_layers(skin);
        let image_layers = Self::derive_image_layers(skin);

        let ico = skin.icon_overrides.as_ref();
        let bar_ov = skin.bar_overrides.as_ref();
        let bg_perf = skin.background_performance.as_ref();

        let focus_glow_color = ov(
            ico.and_then(|i| i.focus_glow_color.as_ref()),
            with_alpha(primary, 100),
        );

        Self {
            bar,
            icon,
            menu,
            app: app_screen,
            osk: osk_theme,
            scrollbar: scrollbar_theme,
            wallpaper: wallpaper_theme,
            toast: toast_theme,
            background_layers,
            chrome_layers,
            image_layers,
            background_max_layers: bg_perf.and_then(|p| p.max_layers).unwrap_or(8),
            background_reduced_motion: bg_perf.and_then(|p| p.reduced_motion).unwrap_or(false),
            background_complexity_budget: bg_perf.and_then(|p| p.complexity_budget).unwrap_or(200),
            entrance_style: ico
                .and_then(|i| i.entrance_style.clone())
                .unwrap_or_else(|| "none".to_string()),
            entrance_duration_ms: ico.and_then(|i| i.entrance_duration_ms).unwrap_or(200),
            entrance_stagger_ms: ico.and_then(|i| i.entrance_stagger_ms).unwrap_or(30),
            focus_scale: ico.and_then(|i| i.focus_scale).unwrap_or(1.0),
            focus_glow: ico.and_then(|i| i.focus_glow).unwrap_or(false),
            focus_glow_color,
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
            terminal_border_radius: skin
                .geometry
                .as_ref()
                .and_then(|g| g.terminal_border_radius)
                .unwrap_or(4),
            taskbar_height: skin
                .geometry
                .as_ref()
                .and_then(|g| g.taskbar_height)
                .unwrap_or(20),
            taskbar_bg: ov(
                bar_ov.and_then(|b| b.taskbar_bg.as_ref()),
                with_alpha(darken(primary, 0.6), 120),
            ),
            taskbar_gradient_top: None,
            taskbar_gradient_bottom: None,
            taskbar_btn_active: ov(
                bar_ov.and_then(|b| b.taskbar_btn_active.as_ref()),
                with_alpha(primary, 120),
            ),
            taskbar_btn_inactive: ov(
                bar_ov.and_then(|b| b.taskbar_btn_inactive.as_ref()),
                Color::rgba(255, 255, 255, 20),
            ),
            taskbar_btn_minimized: ov(
                bar_ov.and_then(|b| b.taskbar_btn_minimized.as_ref()),
                Color::rgba(255, 255, 255, 10),
            ),
            taskbar_btn_hover: ov(
                bar_ov.and_then(|b| b.taskbar_btn_hover.as_ref()),
                with_alpha(lighten(primary, 0.4), 60),
            ),
            taskbar_text_color: ov(
                bar_ov.and_then(|b| b.taskbar_text_color.as_ref()),
                Color::rgba(255, 255, 255, 220),
            ),
            taskbar_separator: ov(
                bar_ov.and_then(|b| b.taskbar_separator.as_ref()),
                Color::rgba(255, 255, 255, 50),
            ),
            taskbar_indicator: ov(
                bar_ov.and_then(|b| b.taskbar_indicator.as_ref()),
                lighten(primary, 0.3),
            ),
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
            icon_stripe_h: skin
                .geometry
                .as_ref()
                .and_then(|g| g.icon_stripe_h)
                .unwrap_or(12),
            icon_fold_size: skin
                .geometry
                .as_ref()
                .and_then(|g| g.icon_fold_size)
                .unwrap_or(10),
            icon_gfx_h: skin
                .geometry
                .as_ref()
                .and_then(|g| g.icon_gfx_h)
                .unwrap_or(22),
            icon_gfx_pad: skin
                .geometry
                .as_ref()
                .and_then(|g| g.icon_gfx_pad)
                .unwrap_or(4),
            icon_label_pad: skin
                .geometry
                .as_ref()
                .and_then(|g| g.icon_label_pad)
                .unwrap_or(4),
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
            clear_color: darken(skin.background_color(), 0.5),
            terminal_line_height: skin
                .geometry
                .as_ref()
                .and_then(|g| g.terminal_line_height)
                .unwrap_or(16),
            cursor_scale: 1,
            cursor_texture: skin.cursor.as_ref().and_then(|c| c.texture.clone()),
            cursor_hotspot: skin
                .cursor
                .as_ref()
                .and_then(|c| c.hotspot)
                .map(|[x, y]| (x, y))
                .unwrap_or((0, 0)),
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
            transition_entrance: skin
                .transition
                .as_ref()
                .and_then(|t| t.entrance.clone())
                .unwrap_or_else(|| "fade".to_string()),
            transition_entrance_frames: skin
                .transition
                .as_ref()
                .and_then(|t| t.entrance_ms)
                .map(|ms| (ms * 60 / 1000).max(1))
                .unwrap_or(45),
            transition_page_style: skin
                .transition
                .as_ref()
                .and_then(|t| t.page_style.clone())
                .unwrap_or_else(|| "slide".to_string()),
            transition_easing: skin
                .transition
                .as_ref()
                .and_then(|t| t.easing.clone())
                .unwrap_or_default(),
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
            font_scale: skin
                .geometry
                .as_ref()
                .and_then(|g| g.font_scale)
                .map(|s| s.clamp(0.5, 3.0))
                .unwrap_or(1.0),
            terminal_cursor_blink_rate: skin
                .geometry
                .as_ref()
                .and_then(|g| g.cursor_blink_rate)
                .unwrap_or(30),
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
            press_flash_duration: skin
                .geometry
                .as_ref()
                .and_then(|g| g.press_flash_duration)
                .unwrap_or(6),
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
            page_dot_lerp_speed: skin
                .geometry
                .as_ref()
                .and_then(|g| g.page_dot_lerp_speed)
                .unwrap_or(0.2),
            app_themes: {
                let mut map = std::collections::HashMap::new();
                if let Some(ref themes) = skin.app_themes {
                    for (app_name, colors) in themes {
                        let mut parsed = std::collections::HashMap::new();
                        for (key, hex) in colors {
                            if let Some(c) = parse_hex_color(hex) {
                                parsed.insert(key.clone(), c);
                            }
                        }
                        if !parsed.is_empty() {
                            map.insert(app_name.clone(), parsed);
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

    /// Derive the bar (status bar + bottom bar) theme from skin overrides.
    fn derive_bar_theme(
        skin: &SkinTheme,
        status_bar_color: Color,
        primary: Color,
        secondary: Color,
        text: Color,
        dim: Color,
    ) -> BarTheme {
        let bar_ov = skin.bar_overrides.as_ref();
        // Generic `text_color` is the fallback for all element-specific
        // bar text colors; `gradient_top/bottom` for both bar gradients.
        let bar_text = bar_ov.and_then(|b| b.text_color.as_ref());
        let grad_top = bar_ov.and_then(|b| b.gradient_top.as_ref());
        let grad_bottom = bar_ov.and_then(|b| b.gradient_bottom.as_ref());

        BarTheme {
            statusbar_bg: ov(
                bar_ov.and_then(|b| b.statusbar_bg.as_ref()),
                with_alpha(status_bar_color, 80),
            ),
            bg: ov(
                bar_ov.and_then(|b| b.bar_bg.as_ref()),
                with_alpha(status_bar_color, 90),
            ),
            separator_color: ov(
                bar_ov.and_then(|b| b.separator_color.as_ref()),
                with_alpha(secondary, 50),
            ),
            battery_color: ov(
                bar_ov.and_then(|b| b.battery_color.as_ref()).or(bar_text),
                lighten(primary, 0.3),
            ),
            version_color: ov(
                bar_ov.and_then(|b| b.version_color.as_ref()).or(bar_text),
                text,
            ),
            clock_color: ov(
                bar_ov.and_then(|b| b.clock_color.as_ref()).or(bar_text),
                text,
            ),
            url_color: ov(bar_ov.and_then(|b| b.url_color.as_ref()).or(bar_text), dim),
            usb_color: ov(bar_ov.and_then(|b| b.usb_color.as_ref()).or(bar_text), dim),
            tab_active_fill: ov(
                bar_ov.and_then(|b| b.tab_active_fill.as_ref()),
                with_alpha(primary, 30),
            ),
            tab_inactive_fill: Color::rgba(0, 0, 0, 0),
            tab_active_alpha: bar_ov.and_then(|b| b.tab_active_alpha).unwrap_or(180),
            tab_inactive_alpha: bar_ov.and_then(|b| b.tab_inactive_alpha).unwrap_or(60),
            media_tab_active: ov(bar_ov.and_then(|b| b.media_tab_active.as_ref()), text),
            media_tab_inactive: ov(bar_ov.and_then(|b| b.media_tab_inactive.as_ref()), dim),
            pipe_color: ov(
                bar_ov.and_then(|b| b.pipe_color.as_ref()).or(bar_text),
                with_alpha(text, 60),
            ),
            r_hint_color: ov(
                bar_ov.and_then(|b| b.r_hint_color.as_ref()).or(bar_text),
                with_alpha(text, 140),
            ),
            category_label_color: ov(
                bar_ov
                    .and_then(|b| b.category_label_color.as_ref())
                    .or(bar_text),
                with_alpha(text, 220),
            ),
            page_dot_active: ov(
                bar_ov.and_then(|b| b.page_dot_active.as_ref()),
                with_alpha(text, 200),
            ),
            page_dot_inactive: ov(
                bar_ov.and_then(|b| b.page_dot_inactive.as_ref()),
                with_alpha(text, 50),
            ),
            statusbar_gradient_top: Self::bar_gradient_pair(
                skin,
                bar_ov
                    .and_then(|b| b.statusbar_gradient_top.as_ref())
                    .or(grad_top),
                bar_ov
                    .and_then(|b| b.statusbar_gradient_bottom.as_ref())
                    .or(grad_bottom),
                status_bar_color,
            )
            .map(|(t, _)| t),
            statusbar_gradient_bottom: Self::bar_gradient_pair(
                skin,
                bar_ov
                    .and_then(|b| b.statusbar_gradient_top.as_ref())
                    .or(grad_top),
                bar_ov
                    .and_then(|b| b.statusbar_gradient_bottom.as_ref())
                    .or(grad_bottom),
                status_bar_color,
            )
            .map(|(_, b)| b),
            gradient_top: Self::bar_gradient_pair(
                skin,
                bar_ov
                    .and_then(|b| b.bar_gradient_top.as_ref())
                    .or(grad_top),
                bar_ov
                    .and_then(|b| b.bar_gradient_bottom.as_ref())
                    .or(grad_bottom),
                status_bar_color,
            )
            .map(|(t, _)| t),
            gradient_bottom: Self::bar_gradient_pair(
                skin,
                bar_ov
                    .and_then(|b| b.bar_gradient_top.as_ref())
                    .or(grad_top),
                bar_ov
                    .and_then(|b| b.bar_gradient_bottom.as_ref())
                    .or(grad_bottom),
                status_bar_color,
            )
            .map(|(_, b)| b),
            text_shadow: bar_ov
                .and_then(|b| b.text_shadow)
                .unwrap_or(skin.gradient_enabled == Some(true)),
            text_shadow_color: ov(
                bar_ov.and_then(|b| b.text_shadow_color.as_ref()),
                Color::rgba(0, 0, 0, 128),
            ),
            version_text: bar_ov
                .and_then(|b| b.version_text.clone())
                .unwrap_or_else(|| "Version 0.1".to_string()),
            category_label: bar_ov
                .and_then(|b| b.category_label.clone())
                .unwrap_or_else(|| "OSS".to_string()),
            url_text: bar_ov.and_then(|b| b.url_text.clone()).unwrap_or_default(),
            tab_active_stroke: with_alpha(
                text,
                bar_ov.and_then(|b| b.tab_active_alpha).unwrap_or(180),
            ),
            tab_inactive_stroke: with_alpha(
                text,
                bar_ov.and_then(|b| b.tab_inactive_alpha).unwrap_or(60),
            ),
            tab_texture_active: bar_ov.and_then(|b| b.tab_texture_active.clone()),
            tab_texture_inactive: bar_ov.and_then(|b| b.tab_texture_inactive.clone()),
        }
    }

    /// Derive the icon theme from skin overrides.
    fn derive_icon_theme(skin: &SkinTheme, primary: Color, text: Color, dim: Color) -> IconTheme {
        let ico = skin.icon_overrides.as_ref();

        let icon_label_color = ov(
            ico.and_then(|i| i.label_color.as_ref()),
            with_alpha(text, 230),
        );
        let icon_label_shadow = {
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

        AppScreenTheme {
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
        }
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

    /// Derive image decal layers (`kind = "image"`) from skin configuration.
    ///
    /// Image layers are not vector layers -- they carry an asset key that the
    /// shell resolves against `Skin::assets` and uploads as a texture, so
    /// they live in a separate list from `background_layers`.
    fn derive_image_layers(skin: &SkinTheme) -> Vec<ImageLayerTheme> {
        skin.background_layers
            .as_ref()
            .map(|layers| {
                layers
                    .iter()
                    .filter(|cfg| cfg.kind == "image")
                    .filter_map(|cfg| {
                        let source = cfg.source.clone()?;
                        Some(ImageLayerTheme {
                            source,
                            position: convert_layer_position(cfg),
                            animation: convert_layer_animation(cfg),
                            alpha: cfg.alpha.unwrap_or(255),
                            enabled: cfg.enabled.unwrap_or(true),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Derive background layers from skin configuration.
    fn derive_background_layers(skin: &SkinTheme) -> Vec<oasis_vector::BackgroundLayer> {
        Self::derive_layer_list(skin, skin.background_layers.as_ref())
    }

    /// Derive chrome layers (overlay-pass vector decorations) from skin
    /// configuration. Same conversion as background layers; `"image"` and
    /// `"shader"` kinds fall out naturally (the converter drops "image" and
    /// the overlay renderer emits no ops for "shader").
    fn derive_chrome_layers(skin: &SkinTheme) -> Vec<oasis_vector::BackgroundLayer> {
        Self::derive_layer_list(skin, skin.chrome_layers.as_ref())
    }

    /// Convert a TOML layer list to runtime layers, honoring the
    /// `background_performance.max_layers` cap.
    fn derive_layer_list(
        skin: &SkinTheme,
        layers: Option<&Vec<crate::theme::BackgroundLayerConfig>>,
    ) -> Vec<oasis_vector::BackgroundLayer> {
        let bg_perf = skin.background_performance.as_ref();
        let bg_max_layers = bg_perf.and_then(|p| p.max_layers).unwrap_or(8);

        layers
            .map(|layers| {
                layers
                    .iter()
                    .take(bg_max_layers as usize)
                    .filter_map(|cfg| Self::convert_background_layer(cfg, &ov))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    /// Derive an `ActiveTheme` directly from 9 base `Color` values.
    ///
    /// This is a lightweight alternative to [`Self::from_skin`] that avoids the
    /// TOML parser and `SkinTheme` struct entirely.  All sub-themes are derived
    /// from the same algorithm used by `from_skin`, but without any override
    /// maps or hex-string parsing.
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
        let dim = dim_text;

        // -- Bar theme (no overrides, pure derivation) --
        let bar = BarTheme {
            statusbar_bg: with_alpha(status_bar, 80),
            bg: with_alpha(status_bar, 90),
            separator_color: with_alpha(secondary, 50),
            battery_color: lighten(primary, 0.3),
            version_color: text,
            clock_color: text,
            url_color: dim,
            usb_color: dim,
            tab_active_fill: with_alpha(primary, 30),
            tab_inactive_fill: Color::rgba(0, 0, 0, 0),
            tab_active_alpha: 180,
            tab_inactive_alpha: 60,
            media_tab_active: text,
            media_tab_inactive: dim,
            pipe_color: with_alpha(text, 60),
            r_hint_color: with_alpha(text, 140),
            category_label_color: with_alpha(text, 220),
            page_dot_active: with_alpha(text, 200),
            page_dot_inactive: with_alpha(text, 50),
            statusbar_gradient_top: None,
            statusbar_gradient_bottom: None,
            gradient_top: None,
            gradient_bottom: None,
            text_shadow: false,
            text_shadow_color: Color::rgba(0, 0, 0, 128),
            version_text: "Version 0.1".to_string(),
            category_label: "OSS".to_string(),
            url_text: String::new(),
            tab_active_stroke: with_alpha(text, 180),
            tab_inactive_stroke: with_alpha(text, 60),
            tab_texture_active: None,
            tab_texture_inactive: None,
        };

        // -- Icon theme --
        let icon_label_color = with_alpha(text, 230);
        let brightness = icon_label_color.r as u16 * 3 / 10
            + icon_label_color.g as u16 * 6 / 10
            + icon_label_color.b as u16 / 10;
        let icon_label_shadow = if brightness > 140 {
            Some(Color::rgba(0, 0, 0, 120))
        } else {
            None
        };
        let icon = IconTheme {
            body_color: text,
            fold_color: dim,
            outline_color: with_alpha(text, 180),
            shadow_color: Color::rgba(0, 0, 0, 70),
            label_color: icon_label_color,
            label_shadow: icon_label_shadow,
            cursor_color: with_alpha(primary, 80),
            border_radius: 4,
            cursor_border_radius: 6,
            cursor_stroke_width: 2,
            style: "document".to_string(),
            cursor_style: "stroke".to_string(),
            shadow_level: 1,
            vector_preset: "altimit".to_string(),
            idle_float: false,
            float_amplitude: 2.0,
            float_speed: 0.04,
            spin_enabled: false,
            spin_speed: 0.03,
            pulse_enabled: false,
            pulse_speed: 0.06,
            blink_enabled: false,
            blink_interval: 45,
            container_style: "none".to_string(),
            container_padding: 3,
        };

        // -- Start menu theme --
        let menu = StartMenuTheme {
            panel_bg: Color::rgba(20, 20, 35, 220),
            panel_gradient_top: None,
            panel_gradient_bottom: None,
            panel_border: with_alpha(text, 40),
            item_text: with_alpha(text, 220),
            item_text_active: text,
            highlight_color: with_alpha(primary, 80),
            button_bg: with_alpha(primary, 200),
            button_text: text,
            panel_border_radius: 4,
            panel_shadow_level: 1,
            layout_mode: "grid".to_string(),
            button_label: "START".to_string(),
            button_width: 48,
            button_height: 18,
            button_shape: "pill".to_string(),
            panel_width: 200,
            columns: 2,
            button_gradient_top: None,
            button_gradient_bottom: None,
            header_text: None,
            header_bg: Color::rgba(30, 30, 50, 240),
            header_text_color: text,
            header_height: 0,
            footer_enabled: false,
            footer_bg: Color::rgba(30, 30, 50, 240),
            footer_text_color: text,
            footer_height: 0,
            item_icon_size: 14,
            item_row_height: 22,
            item_colors: Self::derive_item_palette(primary),
            pad_inner: 8,
            footer_text: "Log Off  Shut Down".to_string(),
            button_x: 4,
            panel_x: 2,
            item_separator: false,
            item_separator_color: with_alpha(with_alpha(text, 40), 64),
        };

        // -- App screen theme --
        let app_screen = AppScreenTheme {
            bg: lighten(background, 0.02),
            divider: lighten(background, 0.15),
            selected_text: lighten(primary, 0.3),
            text: lighten(dim, 0.2),
            dim_text: dim,
            title_bar_bg: lighten(background, 0.08),
            title_bar_text: text,
            title_bar_height: 22,
            terminal_output_color: output,
            terminal_prompt_color: prompt,
            input_border_radius: 4,
            selected_bg: with_alpha(primary, 40),
            selection_border_radius: 2,
            selection_accent_color: with_alpha(primary, 128),
            title_bar_gradient_top: None,
            title_bar_gradient_bottom: None,
            title_bar_text_shadow: false,
            title_bar_text_shadow_color: Color::rgba(0, 0, 0, 128),
        };

        // -- OSK theme --
        let osk_theme = OskTheme {
            key_bg: with_alpha(lighten(background, 0.05), 220),
            key_text: text,
            key_focus: lighten(primary, 0.3),
            key_active: primary,
            key_dim_text: dim,
        };

        // -- Scrollbar theme --
        let scrollbar_theme = ScrollbarTheme {
            track_color: with_alpha(secondary, 20),
            thumb_color: with_alpha(secondary, 100),
            thumb_hover_color: with_alpha(secondary, 160),
            width: 6,
            border_radius: 3,
        };

        // -- Wallpaper theme --
        let wallpaper_theme = WallpaperTheme {
            style: "gradient".to_string(),
            stops: vec![
                Color::rgb(245, 110, 15),
                Color::rgb(255, 230, 30),
                Color::rgb(230, 245, 40),
                Color::rgb(140, 235, 50),
                Color::rgb(200, 252, 130),
            ],
            wave: true,
            wave_intensity: 1.0,
            angle: 0.0,
            grid_spacing: 16,
            grid_color: lighten(background, 0.08),
            noise_intensity: 0.3,
            animated: false,
            source: None,
            fit: "cover".to_string(),
        };

        // -- Toast theme --
        let toast_theme = ToastTheme {
            info_bg: with_alpha(primary, 220),
            success_bg: Color::rgba(60, 180, 100, 220),
            error_bg: with_alpha(error, 220),
            warning_bg: Color::rgba(230, 170, 40, 220),
            text_color: text,
            border_radius: 4,
            ttl: 180,
            text_shadow: false,
            shadow_level: 1,
            fade_frames: 10,
            margin: 8,
            height: 24,
            width_fraction: 0.333,
            gap: 4,
            slide_in: true,
        };

        // -- UI toolkit theme (same derivation as SkinTheme::to_ui_theme) --
        let surface = lighten(background, 0.05);
        let surface_variant = lighten(background, 0.10);
        let accent_hover = lighten(primary, 0.15);
        let accent_pressed = darken(primary, 0.85);
        let accent_subtle = with_alpha(primary, 30);

        let ui_theme = oasis_ui::theme::Theme {
            background,
            surface,
            surface_variant,
            overlay: Color::rgba(0, 0, 0, 180),
            text_primary: text,
            text_secondary: dim,
            text_disabled: darken(dim, 0.6),
            text_on_accent: text,
            accent: primary,
            accent_hover,
            accent_pressed,
            accent_subtle,
            success: Color::rgb(80, 200, 120),
            warning: Color::rgb(255, 180, 50),
            error,
            info: primary,
            border: secondary,
            border_subtle: darken(secondary, 0.7),
            border_strong: primary,
            button_bg: secondary,
            button_bg_hover: lighten(secondary, 0.15),
            button_bg_pressed: darken(secondary, 0.85),
            button_bg_disabled: darken(secondary, 0.5),
            input_bg: darken(background, 0.8),
            input_border: secondary,
            input_border_focus: primary,
            scrollbar_track: Color::rgba(255, 255, 255, 10),
            scrollbar_thumb: Color::rgba(255, 255, 255, 40),
            scrollbar_thumb_hover: Color::rgba(255, 255, 255, 80),
            toggle_track_off: Color::rgba(255, 255, 255, 10),
            toggle_track_on: primary,
            toggle_thumb: text,
            tooltip_bg: lighten(background, 0.15),
            tooltip_text: text,
            font_size_xs: 8,
            font_size_sm: 8,
            font_size_md: 8,
            font_size_lg: 16,
            font_size_xl: 16,
            font_size_xxl: 24,
            spacing_xs: 2,
            spacing_sm: 4,
            spacing_md: 8,
            spacing_lg: 12,
            spacing_xl: 16,
            border_radius_sm: 2,
            border_radius_md: 4,
            border_radius_lg: 8,
            border_radius_xl: 12,
            shadow_card: oasis_types::shadow::Shadow::elevation(1),
            shadow_dropdown: oasis_types::shadow::Shadow::elevation(2),
            shadow_modal: oasis_types::shadow::Shadow::elevation(3),
            shadow_tooltip: oasis_types::shadow::Shadow::elevation(2),
            reduced_motion: false,
            font_scale: 1.0,
            text_direction: oasis_types::text_direction::TextDirection::Ltr,
        };

        Self {
            bar,
            icon,
            menu,
            app: app_screen,
            osk: osk_theme,
            scrollbar: scrollbar_theme,
            wallpaper: wallpaper_theme,
            toast: toast_theme,
            background_layers: Vec::new(),
            chrome_layers: Vec::new(),
            image_layers: Vec::new(),
            background_max_layers: 8,
            background_reduced_motion: false,
            background_complexity_budget: 200,
            entrance_style: "none".to_string(),
            entrance_duration_ms: 200,
            entrance_stagger_ms: 30,
            focus_scale: 1.0,
            focus_glow: false,
            focus_glow_color: with_alpha(primary, 100),
            taskbar_height: 20,
            taskbar_bg: with_alpha(darken(primary, 0.6), 120),
            taskbar_gradient_top: None,
            taskbar_gradient_bottom: None,
            taskbar_btn_active: with_alpha(primary, 120),
            taskbar_btn_inactive: Color::rgba(255, 255, 255, 20),
            taskbar_btn_minimized: Color::rgba(255, 255, 255, 10),
            taskbar_btn_hover: with_alpha(lighten(primary, 0.4), 60),
            taskbar_text_color: Color::rgba(255, 255, 255, 220),
            taskbar_separator: Color::rgba(255, 255, 255, 50),
            taskbar_indicator: lighten(primary, 0.3),
            grid_padding_x: 16,
            grid_padding_y: 6,
            terminal_border_radius: 4,
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
            clear_color: darken(background, 0.5),
            terminal_line_height: 16,
            cursor_scale: 1,
            cursor_texture: None,
            cursor_hotspot: (0, 0),
            focus_ring_color: with_alpha(primary, 180),
            focus_ring_width: 2,
            focus_ring_offset: 2,
            transition_fade_color: darken(background, 0.3),
            transition_entrance: "fade".to_string(),
            transition_entrance_frames: 45,
            transition_page_style: "slide".to_string(),
            transition_easing: String::new(),
            font_body: 12,
            font_hint: 10,
            font_heading: 14,
            font_scale: 1.0,
            terminal_cursor_blink_rate: 30,
            cursor_lerp_speed: 0.35,
            page_slide_duration: 6,
            start_menu_anim_speed: 0.25,
            press_flash_duration: 6,
            cursor_pad: 3,
            press_flash_lighten: 0.25,
            app_selection_lerp_speed: 0.25,
            page_dot_lerp_speed: 0.2,
            app_themes: std::collections::HashMap::new(),
            gradients: std::collections::HashMap::new(),
            animations: std::collections::HashMap::new(),
            widget_states: std::collections::HashMap::new(),
            ui_theme,
        }
    }

    /// Convert a TOML background layer config to a runtime `BackgroundLayer`.
    fn convert_background_layer(
        cfg: &crate::theme::BackgroundLayerConfig,
        ov_fn: &dyn Fn(Option<&String>, Color) -> Color,
    ) -> Option<oasis_vector::BackgroundLayer> {
        use oasis_vector::background::{BackgroundLayer, LayerKind};

        let color = ov_fn(cfg.color.as_ref(), Color::rgba(255, 255, 255, 18));

        let kind = match cfg.kind.as_str() {
            "grid" => LayerKind::Grid {
                spacing: cfg.spacing.unwrap_or(30),
            },
            "dot_grid" => LayerKind::DotGrid {
                spacing: cfg.spacing.unwrap_or(20),
                radius: cfg.dot_radius.unwrap_or(1),
            },
            "wireframe_sphere" => LayerKind::WireframeSphere {
                radius: cfg.radius.unwrap_or(60),
            },
            "radar_sweep" => LayerKind::RadarSweep {
                radius: cfg.radius.unwrap_or(65),
                sweep_angle: cfg.sweep_angle.unwrap_or(0.8),
            },
            "concentric_rings" => LayerKind::ConcentricRings {
                count: cfg.count.unwrap_or(3),
                radius: cfg.radius.unwrap_or(60),
                stroke_width: cfg.stroke_width.unwrap_or(1),
            },
            "glass_shard" => {
                let pts: Vec<(f32, f32)> = cfg
                    .points
                    .as_ref()
                    .map(|p| p.iter().map(|a| (a[0], a[1])).collect())
                    .unwrap_or_default();
                if pts.is_empty() {
                    return None;
                }
                LayerKind::GlassShard { points: pts }
            },
            "scanlines" => LayerKind::Scanlines {
                spacing: cfg.spacing.unwrap_or(2) as u16,
            },
            "eq_bars" => LayerKind::EqBars {
                count: cfg.count.unwrap_or(5),
                bar_width: cfg.bar_width.unwrap_or(8),
                max_height: cfg.max_height.unwrap_or(30),
            },
            "crosshair" => LayerKind::Crosshair {
                size: cfg.size.unwrap_or(20),
            },
            "floating_polygons" => LayerKind::FloatingPolygons {
                count: cfg.count.unwrap_or(3),
                sides: cfg.sides.unwrap_or(4),
            },
            "pulsing_core" => LayerKind::PulsingCore {
                radius: cfg.radius.unwrap_or(10),
            },
            "waves" => LayerKind::Waves {
                rows: cfg.count.unwrap_or(16),
                amplitude: cfg.max_height.unwrap_or(20) as u16,
                frequency: cfg.frequency.unwrap_or(3.0),
            },
            "shader" => {
                let name = cfg.shader.clone().unwrap_or_else(|| "balatro".to_string());
                let params = parse_shader_params(cfg);
                LayerKind::Shader { name, params }
            },
            _ => return None,
        };

        Some(BackgroundLayer {
            kind,
            color,
            position: convert_layer_position(cfg),
            animation: convert_layer_animation(cfg),
            enabled: cfg.enabled.unwrap_or(true),
        })
    }
}

/// Convert a layer's position sub-table to the runtime type.
fn convert_layer_position(
    cfg: &crate::theme::BackgroundLayerConfig,
) -> oasis_vector::background::LayerPosition {
    use oasis_vector::background::{Anchor, LayerPosition};
    cfg.position
        .as_ref()
        .map_or_else(LayerPosition::default, |p| {
            let anchor = match p.anchor.as_deref().unwrap_or("center") {
                "top_left" => Anchor::TopLeft,
                "top_center" => Anchor::TopCenter,
                "top_right" => Anchor::TopRight,
                "center_left" => Anchor::CenterLeft,
                "center_right" => Anchor::CenterRight,
                "bottom_left" => Anchor::BottomLeft,
                "bottom_center" => Anchor::BottomCenter,
                "bottom_right" => Anchor::BottomRight,
                _ => Anchor::Center,
            };
            LayerPosition {
                anchor,
                offset_x: p.offset_x.unwrap_or(0.0),
                offset_y: p.offset_y.unwrap_or(0.0),
            }
        })
}

/// Convert a layer's animation sub-table to the runtime type.
fn convert_layer_animation(
    cfg: &crate::theme::BackgroundLayerConfig,
) -> oasis_vector::background::LayerAnimation {
    use oasis_vector::background::LayerAnimation;
    cfg.animation
        .as_ref()
        .map_or_else(LayerAnimation::default, |a| LayerAnimation {
            rotate_speed: a.rotate_speed.unwrap_or(0.0),
            pulse_speed: a.pulse_speed.unwrap_or(0.0),
            pulse_min_alpha: a.pulse_min_alpha.unwrap_or(0.5),
            drift_x: a.drift_x.unwrap_or(0.0),
            drift_y: a.drift_y.unwrap_or(0.0),
            phase_offset: a.phase_offset.unwrap_or(0.0),
        })
}

/// Parse shader-specific parameters from a `BackgroundLayerConfig`.
fn parse_shader_params(cfg: &crate::theme::BackgroundLayerConfig) -> oasis_vector::ShaderParams {
    let mut params = oasis_vector::ShaderParams::default();

    if let Some(ref table) = cfg.shader_params {
        // Extract color_1, color_2, color_3 as hex -> [f32; 4].
        for key in &["color_1", "color_2", "color_3"] {
            if let Some(toml::Value::String(hex)) = table.get(*key)
                && let Some(c) = hex_to_f32_color(hex)
            {
                params.colors.push(c);
            }
        }

        // Extract named float parameters.
        for (key, value) in table {
            if key.starts_with("color_") {
                continue;
            }
            match value {
                toml::Value::Float(f) => {
                    params.floats.insert(key.clone(), *f as f32);
                },
                toml::Value::Integer(i) => {
                    params.floats.insert(key.clone(), *i as f32);
                },
                toml::Value::Boolean(b) => {
                    params
                        .floats
                        .insert(key.clone(), if *b { 1.0 } else { 0.0 });
                },
                _ => {},
            }
        }
    }

    params
}

/// Parse a hex color string (e.g. "#DE4440") to `[f32; 4]` (0.0-1.0 RGBA).
fn hex_to_f32_color(hex: &str) -> Option<[f32; 4]> {
    let c = parse_hex_color(hex)?;
    Some([
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        c.a as f32 / 255.0,
    ])
}
