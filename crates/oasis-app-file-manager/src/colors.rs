//! File Manager color palette, populated from the active theme.
//!
//! Follows the TV Guide pattern: a plain struct of [`Color`] slots built
//! by [`FileManagerColors::from_theme`], where each slot falls back to the
//! color the app has always used (theme-derived `at.app.*` fields or the
//! historical hardcoded literals). Skins can override any slot via
//! `[app_themes.file_manager]` in theme.toml.

use oasis_skin::ActiveTheme;
use oasis_types::backend::Color;

/// Color roles drawn by the File Manager, overridable per skin.
#[derive(Debug, Clone)]
pub struct FileManagerColors {
    /// Windowed content background and Explorer address-bar strip.
    pub bg: Color,
    /// Title text ("File Manager [L: ..] [R: ..]").
    pub title_text: Color,
    /// Regular panel line text in dual-panel view.
    pub text: Color,
    /// Hint line and the "Address:" label.
    pub dim_text: Color,
    /// Selected tree row / icon tile background.
    pub selected_bg: Color,
    /// Cursor row text, selected tile label, active-panel accent line.
    pub selected_text: Color,
    /// Panel divider and the 1px outlines around panes, icons and fields.
    pub divider: Color,
    /// Explorer status strip background.
    pub status_bg: Color,
    /// Explorer status strip text.
    pub status_text: Color,
    /// Sunken pane background (tree, icon grid, address field).
    pub pane_bg: Color,
    /// Text drawn on sunken panes (tree entries, tile labels, address).
    pub pane_text: Color,
    /// Folder icon body.
    pub folder_icon: Color,
    /// Folder icon tab accent.
    pub folder_icon_tab: Color,
    /// File icon body.
    pub file_icon: Color,
    /// File icon page-fold accent.
    pub file_icon_fold: Color,
}

impl FileManagerColors {
    /// Build colors from the active theme, using app_color overrides.
    ///
    /// Defaults are the exact colors the app used before per-app theming:
    /// mostly `at.app.*` fields, plus the hardcoded white panes and the
    /// yellow folder / white file icon literals.
    pub fn from_theme(at: &ActiveTheme) -> Self {
        let c = |key: &str, default: Color| -> Color {
            at.app_color("file_manager", key).unwrap_or(default)
        };
        Self {
            bg: c("bg", at.app.bg),
            title_text: c("title_text", at.app.title_bar_text),
            text: c("text", at.app.text),
            dim_text: c("dim_text", at.app.dim_text),
            selected_bg: c("selected_bg", at.app.selected_bg),
            selected_text: c("selected_text", at.app.selected_text),
            divider: c("divider", at.app.divider),
            status_bg: c("status_bg", at.app.title_bar_bg),
            status_text: c("status_text", at.app.title_bar_text),
            pane_bg: c("pane_bg", Color::WHITE),
            pane_text: c("pane_text", Color::BLACK),
            folder_icon: c("folder_icon", Color::rgb(255, 207, 87)),
            folder_icon_tab: c("folder_icon_tab", Color::rgb(220, 170, 50)),
            file_icon: c("file_icon", Color::WHITE),
            file_icon_fold: c("file_icon_fold", Color::rgb(220, 220, 220)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use oasis_skin::SkinTheme;

    use super::*;

    #[test]
    fn defaults_match_previous_derivation() {
        let at = ActiveTheme::default();
        let colors = FileManagerColors::from_theme(&at);
        // Theme-derived slots must equal the `at.app.*` fields the renderer
        // read directly before per-app theming existed.
        assert_eq!(colors.bg, at.app.bg);
        assert_eq!(colors.title_text, at.app.title_bar_text);
        assert_eq!(colors.text, at.app.text);
        assert_eq!(colors.dim_text, at.app.dim_text);
        assert_eq!(colors.selected_bg, at.app.selected_bg);
        assert_eq!(colors.selected_text, at.app.selected_text);
        assert_eq!(colors.divider, at.app.divider);
        assert_eq!(colors.status_bg, at.app.title_bar_bg);
        assert_eq!(colors.status_text, at.app.title_bar_text);
        // Literal slots must equal the historical hardcoded colors.
        assert_eq!(colors.pane_bg, Color::WHITE);
        assert_eq!(colors.pane_text, Color::BLACK);
        assert_eq!(colors.folder_icon, Color::rgb(255, 207, 87));
        assert_eq!(colors.folder_icon_tab, Color::rgb(220, 170, 50));
        assert_eq!(colors.file_icon, Color::WHITE);
        assert_eq!(colors.file_icon_fold, Color::rgb(220, 220, 220));
    }

    #[test]
    fn app_theme_override_changes_slot() {
        let mut fm = HashMap::new();
        fm.insert("folder_icon".to_string(), "#FF0000".to_string());
        fm.insert("pane_bg".to_string(), "#101010".to_string());
        let skin = SkinTheme {
            app_themes: Some(HashMap::from([("file_manager".to_string(), fm)])),
            ..SkinTheme::default()
        };
        let at = ActiveTheme::from_skin(&skin);
        let colors = FileManagerColors::from_theme(&at);
        assert_eq!(colors.folder_icon, Color::rgb(255, 0, 0));
        assert_eq!(colors.pane_bg, Color::rgb(16, 16, 16));
        // Untouched slots still fall back to their defaults.
        assert_eq!(colors.file_icon, Color::WHITE);
        assert_eq!(colors.folder_icon_tab, Color::rgb(220, 170, 50));
        assert_eq!(colors.text, at.app.text);
    }
}
