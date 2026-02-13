//! Theme system for consistent UI styling.

use crate::shadow::Shadow;
use oasis_types::backend::Color;

/// Complete visual theme for the UI toolkit.
pub struct Theme {
    /// Main background color.
    pub background: Color,
    /// Surface/panel background color.
    pub surface: Color,
    /// Variant surface color for depth.
    pub surface_variant: Color,
    /// Overlay/modal backdrop color.
    pub overlay: Color,

    /// Primary text color.
    pub text_primary: Color,
    /// Secondary/muted text color.
    pub text_secondary: Color,
    /// Disabled text color.
    pub text_disabled: Color,
    /// Text on accent-colored backgrounds.
    pub text_on_accent: Color,

    /// Primary accent color.
    pub accent: Color,
    /// Accent color on hover.
    pub accent_hover: Color,
    /// Accent color when pressed.
    pub accent_pressed: Color,
    /// Subtle/transparent accent.
    pub accent_subtle: Color,

    /// Success/positive color.
    pub success: Color,
    /// Warning/caution color.
    pub warning: Color,
    /// Error/danger color.
    pub error: Color,
    /// Info/neutral color.
    pub info: Color,

    /// Default border color.
    pub border: Color,
    /// Subtle/faint border color.
    pub border_subtle: Color,
    /// Strong/emphasized border color.
    pub border_strong: Color,

    /// Button background color.
    pub button_bg: Color,
    /// Button background on hover.
    pub button_bg_hover: Color,
    /// Button background when pressed.
    pub button_bg_pressed: Color,
    /// Disabled button background.
    pub button_bg_disabled: Color,
    /// Input field background.
    pub input_bg: Color,
    /// Input field border.
    pub input_border: Color,
    /// Input field border when focused.
    pub input_border_focus: Color,
    /// Scrollbar track background.
    pub scrollbar_track: Color,
    /// Scrollbar thumb color.
    pub scrollbar_thumb: Color,
    /// Scrollbar thumb on hover.
    pub scrollbar_thumb_hover: Color,
    /// Tooltip background.
    pub tooltip_bg: Color,
    /// Tooltip text color.
    pub tooltip_text: Color,

    /// Extra-small font size.
    pub font_size_xs: u16,
    /// Small font size.
    pub font_size_sm: u16,
    /// Medium/default font size.
    pub font_size_md: u16,
    /// Large font size.
    pub font_size_lg: u16,
    /// Extra-large font size.
    pub font_size_xl: u16,
    /// Double extra-large font size.
    pub font_size_xxl: u16,

    /// Extra-small spacing.
    pub spacing_xs: u16,
    /// Small spacing.
    pub spacing_sm: u16,
    /// Medium spacing.
    pub spacing_md: u16,
    /// Large spacing.
    pub spacing_lg: u16,
    /// Extra-large spacing.
    pub spacing_xl: u16,

    /// Small border radius.
    pub border_radius_sm: u16,
    /// Medium border radius.
    pub border_radius_md: u16,
    /// Large border radius.
    pub border_radius_lg: u16,
    /// Extra-large border radius.
    pub border_radius_xl: u16,

    /// Card elevation shadow.
    pub shadow_card: Shadow,
    /// Dropdown elevation shadow.
    pub shadow_dropdown: Shadow,
    /// Modal elevation shadow.
    pub shadow_modal: Shadow,
    /// Tooltip elevation shadow.
    pub shadow_tooltip: Shadow,
}

impl Theme {
    /// Dark theme matching the OASIS cyberpunk aesthetic.
    pub fn dark() -> Self {
        Self {
            background: Color::rgb(18, 18, 24),
            surface: Color::rgb(30, 30, 40),
            surface_variant: Color::rgb(40, 40, 55),
            overlay: Color::rgba(0, 0, 0, 180),

            text_primary: Color::rgb(230, 230, 240),
            text_secondary: Color::rgb(160, 160, 180),
            text_disabled: Color::rgb(100, 100, 120),
            text_on_accent: Color::rgb(255, 255, 255),

            accent: Color::rgb(80, 160, 255),
            accent_hover: Color::rgb(110, 180, 255),
            accent_pressed: Color::rgb(60, 130, 220),
            accent_subtle: Color::rgba(80, 160, 255, 30),

            success: Color::rgb(80, 200, 120),
            warning: Color::rgb(255, 180, 50),
            error: Color::rgb(240, 80, 80),
            info: Color::rgb(80, 160, 255),

            border: Color::rgb(60, 60, 80),
            border_subtle: Color::rgb(45, 45, 60),
            border_strong: Color::rgb(80, 160, 255),

            button_bg: Color::rgb(50, 50, 70),
            button_bg_hover: Color::rgb(65, 65, 90),
            button_bg_pressed: Color::rgb(40, 40, 55),
            button_bg_disabled: Color::rgb(35, 35, 45),
            input_bg: Color::rgb(25, 25, 35),
            input_border: Color::rgb(60, 60, 80),
            input_border_focus: Color::rgb(80, 160, 255),
            scrollbar_track: Color::rgba(255, 255, 255, 10),
            scrollbar_thumb: Color::rgba(255, 255, 255, 40),
            scrollbar_thumb_hover: Color::rgba(255, 255, 255, 80),
            tooltip_bg: Color::rgb(50, 50, 65),
            tooltip_text: Color::rgb(220, 220, 230),

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

            shadow_card: Shadow::elevation(1),
            shadow_dropdown: Shadow::elevation(2),
            shadow_modal: Shadow::elevation(3),
            shadow_tooltip: Shadow::elevation(2),
        }
    }

    /// Light theme.
    pub fn light() -> Self {
        Self {
            background: Color::rgb(245, 245, 250),
            surface: Color::rgb(255, 255, 255),
            surface_variant: Color::rgb(235, 235, 240),
            overlay: Color::rgba(0, 0, 0, 120),

            text_primary: Color::rgb(20, 20, 30),
            text_secondary: Color::rgb(100, 100, 120),
            text_disabled: Color::rgb(170, 170, 180),
            text_on_accent: Color::rgb(255, 255, 255),

            accent: Color::rgb(50, 120, 220),
            accent_hover: Color::rgb(70, 140, 240),
            accent_pressed: Color::rgb(40, 100, 190),
            accent_subtle: Color::rgba(50, 120, 220, 20),

            success: Color::rgb(50, 170, 90),
            warning: Color::rgb(220, 150, 30),
            error: Color::rgb(210, 60, 60),
            info: Color::rgb(50, 120, 220),

            border: Color::rgb(210, 210, 220),
            border_subtle: Color::rgb(230, 230, 235),
            border_strong: Color::rgb(50, 120, 220),

            button_bg: Color::rgb(230, 230, 240),
            button_bg_hover: Color::rgb(220, 220, 230),
            button_bg_pressed: Color::rgb(200, 200, 215),
            button_bg_disabled: Color::rgb(240, 240, 245),
            input_bg: Color::rgb(255, 255, 255),
            input_border: Color::rgb(200, 200, 210),
            input_border_focus: Color::rgb(50, 120, 220),
            scrollbar_track: Color::rgba(0, 0, 0, 10),
            scrollbar_thumb: Color::rgba(0, 0, 0, 30),
            scrollbar_thumb_hover: Color::rgba(0, 0, 0, 60),
            tooltip_bg: Color::rgb(40, 40, 50),
            tooltip_text: Color::rgb(240, 240, 245),

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

            shadow_card: Shadow::elevation(1),
            shadow_dropdown: Shadow::elevation(2),
            shadow_modal: Shadow::elevation(3),
            shadow_tooltip: Shadow::elevation(2),
        }
    }

    /// Classic OASIS theme (orange/green).
    pub fn classic() -> Self {
        let mut theme = Self::dark();
        theme.accent = Color::rgb(255, 140, 30);
        theme.accent_hover = Color::rgb(255, 165, 60);
        theme.accent_pressed = Color::rgb(220, 120, 20);
        theme.accent_subtle = Color::rgba(255, 140, 30, 30);
        theme.border_strong = Color::rgb(255, 140, 30);
        theme.success = Color::rgb(100, 220, 80);
        theme
    }

    #[cfg(test)]
    fn accent_rgb(&self) -> (u8, u8, u8) {
        (self.accent.r, self.accent.g, self.accent.b)
    }

    /// High-contrast theme for accessibility.
    pub fn high_contrast() -> Self {
        Self {
            background: Color::rgb(0, 0, 0),
            surface: Color::rgb(0, 0, 0),
            surface_variant: Color::rgb(20, 20, 20),
            overlay: Color::rgba(0, 0, 0, 220),

            text_primary: Color::rgb(255, 255, 255),
            text_secondary: Color::rgb(255, 255, 0),
            text_disabled: Color::rgb(128, 128, 128),
            text_on_accent: Color::rgb(0, 0, 0),

            accent: Color::rgb(0, 255, 255),
            accent_hover: Color::rgb(100, 255, 255),
            accent_pressed: Color::rgb(0, 200, 200),
            accent_subtle: Color::rgba(0, 255, 255, 50),

            success: Color::rgb(0, 255, 0),
            warning: Color::rgb(255, 255, 0),
            error: Color::rgb(255, 0, 0),
            info: Color::rgb(0, 255, 255),

            border: Color::rgb(255, 255, 255),
            border_subtle: Color::rgb(200, 200, 200),
            border_strong: Color::rgb(0, 255, 255),

            button_bg: Color::rgb(40, 40, 40),
            button_bg_hover: Color::rgb(60, 60, 60),
            button_bg_pressed: Color::rgb(20, 20, 20),
            button_bg_disabled: Color::rgb(30, 30, 30),
            input_bg: Color::rgb(0, 0, 0),
            input_border: Color::rgb(255, 255, 255),
            input_border_focus: Color::rgb(0, 255, 255),
            scrollbar_track: Color::rgba(255, 255, 255, 30),
            scrollbar_thumb: Color::rgba(255, 255, 255, 120),
            scrollbar_thumb_hover: Color::rgba(255, 255, 255, 200),
            tooltip_bg: Color::rgb(0, 0, 0),
            tooltip_text: Color::rgb(255, 255, 255),

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

            border_radius_sm: 0,
            border_radius_md: 0,
            border_radius_lg: 0,
            border_radius_xl: 0,

            shadow_card: Shadow::elevation(0),
            shadow_dropdown: Shadow::elevation(0),
            shadow_modal: Shadow::elevation(0),
            shadow_tooltip: Shadow::elevation(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_has_dark_background() {
        let t = Theme::dark();
        // Background should be dark (low RGB values).
        assert!(t.background.r < 50);
        assert!(t.background.g < 50);
        assert!(t.background.b < 50);
    }

    #[test]
    fn light_has_light_background() {
        let t = Theme::light();
        // Background should be light (high RGB values).
        assert!(t.background.r > 200);
        assert!(t.background.g > 200);
        assert!(t.background.b > 200);
    }

    #[test]
    fn classic_uses_orange_accent() {
        let t = Theme::classic();
        // Classic accent is orange (high red, medium green, low blue).
        assert!(t.accent.r > 200);
        assert!(t.accent.g > 100 && t.accent.g < 200);
        assert!(t.accent.b < 100);
    }

    #[test]
    fn classic_based_on_dark() {
        let dark = Theme::dark();
        let classic = Theme::classic();
        // Classic shares dark's background.
        assert_eq!(dark.background, classic.background);
        assert_eq!(dark.surface, classic.surface);
        // But has different accent.
        assert_ne!(dark.accent_rgb(), classic.accent_rgb());
    }

    #[test]
    fn high_contrast_pure_black_bg() {
        let t = Theme::high_contrast();
        assert_eq!(t.background, Color::rgb(0, 0, 0));
        assert_eq!(t.surface, Color::rgb(0, 0, 0));
    }

    #[test]
    fn high_contrast_white_text() {
        let t = Theme::high_contrast();
        assert_eq!(t.text_primary, Color::rgb(255, 255, 255));
    }

    #[test]
    fn high_contrast_no_rounded_corners() {
        let t = Theme::high_contrast();
        assert_eq!(t.border_radius_sm, 0);
        assert_eq!(t.border_radius_md, 0);
        assert_eq!(t.border_radius_lg, 0);
        assert_eq!(t.border_radius_xl, 0);
    }

    #[test]
    fn high_contrast_no_shadows() {
        let t = Theme::high_contrast();
        assert_eq!(t.shadow_card.layers.len(), 0);
        assert_eq!(t.shadow_dropdown.layers.len(), 0);
        assert_eq!(t.shadow_modal.layers.len(), 0);
        assert_eq!(t.shadow_tooltip.layers.len(), 0);
    }

    #[test]
    fn font_sizes_are_ordered() {
        let t = Theme::dark();
        assert!(t.font_size_xs <= t.font_size_sm);
        assert!(t.font_size_sm <= t.font_size_md);
        assert!(t.font_size_md <= t.font_size_lg);
        assert!(t.font_size_lg <= t.font_size_xl);
        assert!(t.font_size_xl <= t.font_size_xxl);
    }

    #[test]
    fn spacing_is_ordered() {
        let t = Theme::dark();
        assert!(t.spacing_xs <= t.spacing_sm);
        assert!(t.spacing_sm <= t.spacing_md);
        assert!(t.spacing_md <= t.spacing_lg);
        assert!(t.spacing_lg <= t.spacing_xl);
    }

    #[test]
    fn border_radius_is_ordered() {
        let t = Theme::dark();
        assert!(t.border_radius_sm <= t.border_radius_md);
        assert!(t.border_radius_md <= t.border_radius_lg);
        assert!(t.border_radius_lg <= t.border_radius_xl);
    }

    #[test]
    fn all_variants_have_consistent_font_sizes() {
        for theme in [
            Theme::dark(),
            Theme::light(),
            Theme::classic(),
            Theme::high_contrast(),
        ] {
            assert_eq!(theme.font_size_xs, 8);
            assert_eq!(theme.font_size_md, 8);
            assert_eq!(theme.font_size_lg, 16);
        }
    }

    #[test]
    fn dark_has_shadows() {
        let t = Theme::dark();
        assert!(!t.shadow_card.layers.is_empty());
        assert!(!t.shadow_modal.layers.is_empty());
    }
}
