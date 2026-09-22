//! Skin-derived palettes for the Music Player and Photo Viewer.
//!
//! Every color defaults to a field of the active skin (`ActiveTheme::app`
//! app-screen colors or the derived `ui_theme` widget colors) so the
//! media apps follow light / dark / stylized skins automatically. Skins
//! can recolor individual slots via `[app_themes.music_player]` and
//! `[app_themes.photo_viewer]` (see `docs/skin-authoring.md`).

use oasis_skin::ActiveTheme;
use oasis_types::backend::Color;

/// `app_themes` key for the Music Player.
pub const MUSIC_APP_THEME: &str = "music_player";
/// `app_themes` key for the Photo Viewer.
pub const PHOTO_APP_THEME: &str = "photo_viewer";

/// Colors used by the Music Player "now playing" screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MusicColors {
    /// Window background.
    pub bg: Color,
    /// Thin accent rule along the top edge.
    pub header_rule: Color,
    /// Album-art tile outer frame.
    pub art_frame: Color,
    /// Album-art tile accent ring.
    pub art_ring: Color,
    /// Album-art tile inner fill.
    pub art_fill: Color,
    /// Musical-note glyph on the album-art tile.
    pub art_glyph: Color,
    /// "Now Playing" label.
    pub label: Color,
    /// Track title.
    pub title: Color,
    /// Filename / metadata rows.
    pub meta: Color,
    /// "Shuffle: ON" indicator.
    pub shuffle_on: Color,
    /// Transport strip background.
    pub transport_bg: Color,
    /// Transport strip top rule.
    pub transport_rule: Color,
    /// Secondary transport button fill.
    pub button_bg: Color,
    /// Secondary transport button glyph.
    pub button_text: Color,
    /// Primary (play/pause) transport button fill.
    pub button_primary_bg: Color,
    /// Primary transport button glyph.
    pub button_primary_text: Color,
    /// Progress bar track.
    pub progress_track: Color,
    /// Progress bar filled portion.
    pub progress_fill: Color,
    /// Elapsed / total time readout.
    pub progress_text: Color,
}

impl MusicColors {
    /// Resolve the palette from the active skin plus
    /// `[app_themes.music_player]` overrides.
    pub fn from_theme(at: &ActiveTheme) -> Self {
        let c = |key: &str, default: Color| at.app_color(MUSIC_APP_THEME, key).unwrap_or(default);
        let ui = &at.ui_theme;
        Self {
            bg: c("bg", at.app.bg),
            header_rule: c("header_rule", ui.accent),
            art_frame: c("art_frame", at.app.divider),
            art_ring: c("art_ring", ui.accent),
            art_fill: c("art_fill", ui.surface),
            art_glyph: c("art_glyph", at.app.text),
            label: c("label", ui.accent),
            title: c("title", at.app.text),
            meta: c("meta", at.app.dim_text),
            shuffle_on: c("shuffle_on", ui.success),
            transport_bg: c("transport_bg", ui.surface),
            transport_rule: c("transport_rule", at.app.divider),
            button_bg: c("button_bg", ui.button_bg),
            button_text: c("button_text", ui.text_primary),
            button_primary_bg: c("button_primary_bg", ui.accent),
            button_primary_text: c("button_primary_text", ui.text_on_accent),
            progress_track: c("progress_track", ui.slider_track),
            progress_fill: c("progress_fill", ui.slider_fill),
            progress_text: c("progress_text", at.app.dim_text),
        }
    }
}

/// Colors used by the Photo Viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhotoColors {
    /// Matte behind the photo.
    pub bg: Color,
    /// "Preview not available" placeholder box.
    pub placeholder_bg: Color,
    /// Placeholder message.
    pub placeholder_text: Color,
    /// Footer strip (drawn over the photo).
    pub footer_bg: Color,
    /// Footer filename / dimensions text.
    pub footer_text: Color,
    /// Footer key-hint text.
    pub hint_text: Color,
}

impl PhotoColors {
    /// Resolve the palette from the active skin plus
    /// `[app_themes.photo_viewer]` overrides.
    pub fn from_theme(at: &ActiveTheme) -> Self {
        let c = |key: &str, default: Color| at.app_color(PHOTO_APP_THEME, key).unwrap_or(default);
        let ui = &at.ui_theme;
        let title_bg = at.app.title_bar_bg;
        Self {
            bg: c("bg", at.app.bg),
            placeholder_bg: c("placeholder_bg", ui.surface),
            placeholder_text: c("placeholder_text", at.app.dim_text),
            footer_bg: c(
                "footer_bg",
                Color::rgba(title_bg.r, title_bg.g, title_bg.b, 220),
            ),
            footer_text: c("footer_text", at.app.title_bar_text),
            hint_text: c("hint_text", at.app.title_bar_text.lerp(title_bg, 0.35)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_skin::SkinTheme;
    use std::collections::HashMap;

    fn slots(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn app_theme_overrides_win() {
        let mut app_themes = HashMap::new();
        app_themes.insert(
            MUSIC_APP_THEME.to_string(),
            slots(&[("bg", "#102030"), ("progress_fill", "#FF0000")]),
        );
        app_themes.insert(
            PHOTO_APP_THEME.to_string(),
            slots(&[("footer_text", "#00FF00")]),
        );
        let skin = SkinTheme {
            app_themes: Some(app_themes),
            ..SkinTheme::default()
        };
        let at = ActiveTheme::from_skin(&skin);
        let m = MusicColors::from_theme(&at);
        assert_eq!(m.bg, Color::rgb(0x10, 0x20, 0x30));
        assert_eq!(m.progress_fill, Color::rgb(255, 0, 0));
        // Unset slots keep their theme-derived default.
        assert_eq!(m.title, at.app.text);
        let p = PhotoColors::from_theme(&at);
        assert_eq!(p.footer_text, Color::rgb(0, 255, 0));
        assert_eq!(p.bg, at.app.bg);
    }
}
