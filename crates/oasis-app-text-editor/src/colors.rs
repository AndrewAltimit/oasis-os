//! Per-skin color palette for the Text Editor.
//!
//! Every slot comes from the active theme: the text area follows the
//! generic app-screen colors (`ActiveTheme::app`), the menu bar follows the
//! UI theme's `menu_*` slots (the same ones `MenuStyle::from_theme` reads),
//! and syntax colors come from the skin's ANSI palette, which is derived
//! from the skin's primary / terminal-output colors. Skins can override any
//! slot through `[app_themes.text_editor]` in theme.toml (keys are the field
//! names below; syntax keys are prefixed `syntax_`, e.g. `syntax_keyword`).

use oasis_skin::ActiveTheme;
use oasis_types::backend::Color;
use oasis_ui::menu_bar::MenuStyle;

use crate::highlight::SyntaxTheme;

/// `[app_themes.<APP_KEY>]` section name.
const APP_KEY: &str = "text_editor";

/// ANSI palette slots (SGR order) used for syntax roles.
const ANSI_GREEN: usize = 2;
const ANSI_YELLOW: usize = 3;
const ANSI_BLUE: usize = 4;
const ANSI_MAGENTA: usize = 5;
const ANSI_CYAN: usize = 6;
const ANSI_BRIGHT_BLUE: usize = 12;

/// Text Editor color palette, populated from the active theme.
#[derive(Debug, Clone)]
pub struct EditorColors {
    /// Text area background.
    pub bg: Color,
    /// Plain text.
    pub text: Color,
    /// Current-line highlight band.
    pub current_line_bg: Color,
    /// Selected-text band.
    pub selection_bg: Color,
    /// Caret in Insert mode.
    pub caret: Color,
    /// Caret in Normal mode.
    pub caret_normal: Color,
    /// Status strip background.
    pub status_bg: Color,
    /// Status strip text.
    pub status_text: Color,
    /// Status strip top border.
    pub border: Color,
    /// Menu bar / drop-down colors.
    pub menu: MenuStyle,
    /// Syntax-highlighting colors.
    pub syntax: SyntaxTheme,
}

impl EditorColors {
    /// Build colors from the active theme, honoring
    /// `[app_themes.text_editor]` overrides.
    pub fn from_theme(at: &ActiveTheme) -> Self {
        let c = |key: &str, default: Color| at.app_color(APP_KEY, key).unwrap_or(default);
        let bg = c("bg", at.app.bg);
        let text = c("text", at.app.text);
        let ansi = &at.ansi.colors;
        let syn = |key: &str, default: Color| c(key, default);
        let syntax = SyntaxTheme {
            normal: text,
            keyword: syn("syntax_keyword", ansi[ANSI_BLUE]),
            type_name: syn("syntax_type", ansi[ANSI_CYAN]),
            string_literal: syn("syntax_string", ansi[ANSI_GREEN]),
            number: syn("syntax_number", ansi[ANSI_MAGENTA]),
            comment: syn("syntax_comment", at.app.dim_text),
            attribute: syn("syntax_attribute", ansi[ANSI_YELLOW]),
            operator: syn("syntax_operator", text),
            tag: syn("syntax_tag", ansi[ANSI_BLUE]),
            tag_attribute: syn("syntax_tag_attribute", ansi[ANSI_CYAN]),
            section: syn("syntax_section", ansi[ANSI_YELLOW]),
            heading: syn("syntax_heading", ansi[ANSI_BRIGHT_BLUE]),
            emphasis: syn("syntax_emphasis", ansi[ANSI_YELLOW]),
            code_span: syn("syntax_code_span", ansi[ANSI_GREEN]),
            link: syn("syntax_link", ansi[ANSI_CYAN]),
        };
        let mut menu = MenuStyle::from_theme(&at.ui_theme);
        menu.bar_bg = c("menu_bg", menu.bar_bg);
        menu.label_text = c("menu_text", menu.label_text);
        Self {
            bg,
            text,
            // Blend toward the theme's selection color so the band stays
            // subtle and syntax colors keep their contrast on any skin.
            current_line_bg: c("current_line_bg", bg.lerp(at.app.selected_bg, 0.25)),
            selection_bg: c("selection_bg", bg.lerp(at.app.selected_bg, 0.6)),
            caret: c("caret", at.app.selection_accent_color),
            caret_normal: c("caret_normal", at.app.dim_text),
            status_bg: c("status_bg", at.app.title_bar_bg),
            status_text: c("status_text", at.app.title_bar_text),
            border: c("border", at.app.divider),
            menu,
            syntax,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dark_theme() -> ActiveTheme {
        let mut at = ActiveTheme::default();
        at.app.bg = Color::rgb(18, 18, 24);
        at.app.text = Color::rgb(220, 220, 230);
        at
    }

    #[test]
    fn defaults_follow_app_screen_theme() {
        let at = dark_theme();
        let colors = EditorColors::from_theme(&at);
        assert_eq!(colors.bg, at.app.bg);
        assert_eq!(colors.text, at.app.text);
        assert_eq!(colors.status_bg, at.app.title_bar_bg);
        assert_eq!(colors.syntax.normal, at.app.text);
        assert_eq!(colors.syntax.keyword, at.ansi.colors[ANSI_BLUE]);
        assert_eq!(colors.menu.bar_bg, at.ui_theme.menu_bg);
    }

    #[test]
    fn app_theme_overrides_apply() {
        let magenta = Color::rgb(255, 0, 255);
        let mut at = ActiveTheme::default();
        let section = at.app_themes.entry(APP_KEY.to_string()).or_default();
        section.insert("bg".into(), magenta);
        section.insert("syntax_keyword".into(), magenta);
        let colors = EditorColors::from_theme(&at);
        assert_eq!(colors.bg, magenta);
        assert_eq!(colors.syntax.keyword, magenta);
        assert_eq!(colors.text, at.app.text, "untouched slots keep defaults");
    }

    #[test]
    fn current_line_band_stays_close_to_background() {
        let at = dark_theme();
        let colors = EditorColors::from_theme(&at);
        let dist = |a: Color, b: Color| {
            (i32::from(a.r) - i32::from(b.r)).abs()
                + (i32::from(a.g) - i32::from(b.g)).abs()
                + (i32::from(a.b) - i32::from(b.b)).abs()
        };
        assert!(dist(colors.current_line_bg, colors.bg) <= dist(at.app.selected_bg, colors.bg));
    }
}
