//! Runtime theme derived from the active skin.
//!
//! `ActiveTheme` replaces the hardcoded constants in `theme.rs` with a runtime
//! struct whose fields are derived from the skin's 9 base colors. Consumers
//! receive `&ActiveTheme` instead of reading `theme::CONST` directly, allowing
//! skins to actually drive the UI appearance.

use crate::backend::Color;
use crate::skin::SkinTheme;
use crate::skin::theme::parse_hex_color;
use crate::ui::color::{lighten, with_alpha};

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

    // -- Icon geometry --
    /// Icon card border radius (pixels).
    pub icon_border_radius: u16,
    /// Cursor highlight border radius (pixels).
    pub cursor_border_radius: u16,
    /// Cursor highlight stroke width (pixels).
    pub cursor_stroke_width: u16,
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
            icon_body_color: Color::rgb(250, 250, 248),
            icon_fold_color: Color::rgb(210, 210, 205),
            icon_outline_color: Color::rgba(255, 255, 255, 180),
            icon_shadow_color: Color::rgba(0, 0, 0, 70),
            icon_label_color: Color::rgba(255, 255, 255, 230),
            cursor_color: Color::rgba(255, 255, 255, 50),
            icon_border_radius: 4,
            cursor_border_radius: 6,
            cursor_stroke_width: 2,
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
        }
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
}
