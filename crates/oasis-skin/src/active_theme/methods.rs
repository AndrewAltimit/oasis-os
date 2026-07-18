//! Builder methods, lookup helpers, and utility functions for `ActiveTheme`.

use oasis_types::backend::Color;
use oasis_types::color::lighten;

use crate::SkinTheme;
use crate::theme::parse_hex_color;

use super::ActiveTheme;

impl ActiveTheme {
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

    /// Apply skin feature flags (builder pattern).
    ///
    /// When `features.reduced_motion` is set, all decorative motion is
    /// neutralized at the theme level so downstream render code falls through
    /// to the static / final-frame path without any per-call-site checks:
    /// dashboard icon `idle_float`/`spin`/`pulse`/`blink` are disabled, the
    /// icon entrance animation becomes instant (`entrance_style = "none"`),
    /// the focus glow pulse is dropped, and animated background layers are
    /// frozen. `reduced_motion` defaults to `false`, so skins that do not opt
    /// in stay pixel-identical.
    pub fn with_features(mut self, features: &crate::loader::SkinFeatures) -> Self {
        self.ui_theme.reduced_motion = features.reduced_motion;
        if features.reduced_motion {
            self.icon.idle_float = false;
            self.icon.spin_enabled = false;
            self.icon.pulse_enabled = false;
            self.icon.blink_enabled = false;
            self.entrance_style = "none".to_string();
            self.focus_glow = false;
            self.background_reduced_motion = true;
        }
        self
    }

    /// Resolve a semantic elevation level (0..=5) to a concrete shadow,
    /// honoring any `[elevation]` overrides on the active skin and falling
    /// back to the built-in ladder otherwise.
    ///
    /// This is the single resolution point for the scattered `*_shadow_level`
    /// fields (`icon.shadow_level`, `menu.panel_shadow_level`,
    /// `toast.shadow_level`, …).
    pub fn resolve_shadow(&self, level: u8) -> oasis_types::shadow::Shadow {
        self.elevation.resolve(level)
    }

    /// Derive a gradient pair for a bar element.
    ///
    /// Returns `Some((top, bottom))` if gradient is enabled (either via explicit
    /// overrides or via `gradient_enabled`), or `None` for flat fill.
    pub(crate) fn bar_gradient_pair(
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
            (dur, super::super::theme::resolve_easing(easing_name))
        } else {
            (default_ms, oasis_ui::animation::easing::linear)
        }
    }

    /// Apply the system-wide font scale factor to a raw font size.
    ///
    /// Returns the scaled font size as a `u16`, clamped to at least 1.
    /// The scale factor is clamped to 0.5-3.0 at construction time.
    #[inline]
    pub fn scaled_font_size(&self, raw_size: u16) -> u16 {
        let scaled = (f32::from(raw_size) * self.font_scale).round() as u16;
        scaled.max(1)
    }

    /// Derive a 6-color palette from the primary color using hue-shifted offsets.
    pub(crate) fn derive_item_palette(primary: Color) -> Vec<Color> {
        vec![
            primary,
            Color::rgb(
                primary.r.saturating_sub(20),
                primary.g.saturating_add(40),
                primary.b.saturating_sub(30),
            ),
            Color::rgb(
                primary.r.saturating_add(60),
                primary.g.saturating_add(20),
                primary.b.saturating_sub(60),
            ),
            Color::rgb(
                primary.r.saturating_add(30),
                primary.g.saturating_sub(40),
                primary.b.saturating_add(40),
            ),
            lighten(primary, 0.2),
            Color::rgb(
                primary.r.saturating_add(50),
                primary.g.saturating_sub(30),
                primary.b.saturating_sub(30),
            ),
        ]
    }
}
