//! Per-skin color palette for the Settings app.

use oasis_skin::ActiveTheme;
use oasis_types::backend::Color;

/// Settings app color palette, populated from the active theme.
///
/// Every default is sourced from the generic app-screen theme
/// (`ActiveTheme::app`), so with no `[app_themes.settings]` section the
/// rendered output is pixel-identical to the shared content renderer in
/// `oasis-app-core`. Skins can override any slot via `[app_themes.settings]`
/// in theme.toml.
#[derive(Debug, Clone)]
pub struct SettingsColors {
    /// Fullscreen app background.
    pub bg: Color,
    /// Title bar background (flat fill; skin gradients still draw on top).
    pub title_bar_bg: Color,
    /// Title bar text ("Settings").
    pub title_bar_text: Color,
    /// Normal content line text (tabs, labels, values, hints).
    pub text: Color,
    /// Text of the cursor/selected line.
    pub selected_text: Color,
    /// Selection highlight row background (fullscreen SDI mode).
    pub selected_bg: Color,
    /// Selection left-accent bar (fullscreen SDI mode).
    pub selection_accent: Color,
    /// Dim/hint text (scroll indicator, "Cancel=back" footer).
    pub dim_text: Color,
    /// Separator line under the title bar (windowed mode).
    pub divider: Color,
}

impl SettingsColors {
    /// Build colors from the active theme, using app_color overrides.
    pub fn from_theme(at: &ActiveTheme) -> Self {
        let c = |key: &str, default: Color| -> Color {
            at.app_color("settings", key).unwrap_or(default)
        };
        Self {
            bg: c("bg", at.app.bg),
            title_bar_bg: c("title_bar_bg", at.app.title_bar_bg),
            title_bar_text: c("title_bar_text", at.app.title_bar_text),
            text: c("text", at.app.text),
            selected_text: c("selected_text", at.app.selected_text),
            selected_bg: c("selected_bg", at.app.selected_bg),
            selection_accent: c("selection_accent", at.app.selection_accent_color),
            dim_text: c("dim_text", at.app.dim_text),
            divider: c("divider", at.app.divider),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn override_theme(key: &str, color: Color) -> ActiveTheme {
        let mut at = ActiveTheme::default();
        at.app_themes
            .entry("settings".to_string())
            .or_default()
            .insert(key.to_string(), color);
        at
    }

    #[test]
    fn defaults_match_app_screen_theme() {
        // Without [app_themes.settings], every slot must equal the shared
        // app-screen theme value the renderer used before this struct
        // existed (screenshot-regression invariant).
        let at = ActiveTheme::default();
        let colors = SettingsColors::from_theme(&at);
        assert_eq!(colors.bg, at.app.bg);
        assert_eq!(colors.title_bar_bg, at.app.title_bar_bg);
        assert_eq!(colors.title_bar_text, at.app.title_bar_text);
        assert_eq!(colors.text, at.app.text);
        assert_eq!(colors.selected_text, at.app.selected_text);
        assert_eq!(colors.selected_bg, at.app.selected_bg);
        assert_eq!(colors.selection_accent, at.app.selection_accent_color);
        assert_eq!(colors.dim_text, at.app.dim_text);
        assert_eq!(colors.divider, at.app.divider);
    }

    #[test]
    fn override_changes_slot() {
        let magenta = Color::rgba(255, 0, 255, 255);
        let at = override_theme("selected_bg", magenta);
        let colors = SettingsColors::from_theme(&at);
        assert_eq!(colors.selected_bg, magenta);
        // Unrelated slots keep their theme-derived defaults.
        assert_eq!(colors.bg, at.app.bg);
        assert_eq!(colors.text, at.app.text);
    }

    #[test]
    fn override_ignores_other_apps() {
        let magenta = Color::rgba(255, 0, 255, 255);
        let mut at = ActiveTheme::default();
        at.app_themes
            .entry("tv_guide".to_string())
            .or_default()
            .insert("bg".to_string(), magenta);
        let colors = SettingsColors::from_theme(&at);
        assert_eq!(colors.bg, at.app.bg);
    }
}
