//! Theme system for consistent UI styling.

use crate::shadow::Shadow;
use oasis_types::backend::Color;
use oasis_types::text_direction::TextDirection;

/// Complete visual theme for the UI toolkit.
#[derive(Debug, Clone)]
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
    /// Toggle track color when off.
    pub toggle_track_off: Color,
    /// Toggle track color when on.
    pub toggle_track_on: Color,
    /// Toggle thumb color.
    pub toggle_thumb: Color,
    /// Slider track (unfilled portion) background.
    pub slider_track: Color,
    /// Slider filled-portion color.
    pub slider_fill: Color,
    /// Slider thumb fill color.
    pub slider_thumb: Color,
    /// Menu bar background (`MenuBar` widget).
    ///
    /// The `menu_*` slots default to the classic Win95 grays the menu
    /// bar widget has always rendered with — in every built-in theme —
    /// so existing apps stay pixel-identical. Skins re-color them via
    /// `[widget_states.menu]`.
    pub menu_bg: Color,
    /// Menu bar bottom border.
    pub menu_border: Color,
    /// Menu bar label text.
    pub menu_text: Color,
    /// Highlight behind an open label / hovered drop-down item.
    pub menu_hover_bg: Color,
    /// Text on the menu hover highlight.
    pub menu_hover_text: Color,
    /// Drop-down panel background.
    pub menu_dropdown_bg: Color,
    /// Drop-down bezel highlight (top/left edge).
    pub menu_dropdown_border_light: Color,
    /// Drop-down bezel shadow (bottom/right edge).
    pub menu_dropdown_border_dark: Color,
    /// Drop-down item text.
    pub menu_item_text: Color,
    /// Disabled drop-down item text.
    pub menu_disabled_text: Color,
    /// Drop-down separator line.
    pub menu_separator: Color,
    /// Tooltip background.
    pub tooltip_bg: Color,
    /// Tooltip text color.
    pub tooltip_text: Color,

    /// Focus ring color override for keyboard-focus indicators.
    ///
    /// `None` means "not themed": `FocusStyle` derives the ring color
    /// from `accent` exactly as it always has. Skins set this via
    /// `[geometry] focus_ring_color`.
    pub focus_ring_color: Option<Color>,
    /// Focus ring stroke width override in pixels (`None` = default).
    pub focus_ring_width: Option<u16>,
    /// Focus ring offset from the widget edge in pixels
    /// (`None` = default).
    pub focus_ring_offset: Option<i32>,

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

    /// Whether to reduce or skip animations for accessibility.
    ///
    /// When `true`, animated widgets should snap to their target state
    /// instead of interpolating over time.
    pub reduced_motion: bool,

    /// Global font scale multiplier (default 1.0).
    ///
    /// Applied on top of the base font sizes. A value of 1.5 would
    /// make all text 50% larger. Clamped to `0.5..=3.0`.
    pub font_scale: f32,

    /// Text direction for the UI (LTR, RTL, or Auto).
    ///
    /// When RTL, widgets should mirror their inline layout: text
    /// alignment flips (start = right), and padding-left/right swap.
    pub text_direction: TextDirection,
}

impl Theme {
    /// Return a font size scaled by `font_scale`.
    ///
    /// The result is clamped to at least 1 and at most `u16::MAX`.
    pub fn scaled_font_size(&self, base: u16) -> u16 {
        let scaled = (base as f32 * self.font_scale.clamp(0.5, 3.0)).round();
        (scaled as u32).clamp(1, u16::MAX as u32) as u16
    }

    // -------------------------------------------------------------------
    // Interactive state color helpers
    // -------------------------------------------------------------------

    /// Border color for interactive controls (checkbox, radio, toggle).
    ///
    /// Returns `border_subtle` when disabled, `accent` when selected,
    /// and `input_border` otherwise.
    pub fn interactive_border(&self, disabled: bool, selected: bool) -> Color {
        if disabled {
            self.border_subtle
        } else if selected {
            self.accent
        } else {
            self.input_border
        }
    }

    /// Accent color respecting disabled state.
    ///
    /// Returns `text_disabled` when disabled, `accent` otherwise.
    pub fn interactive_accent(&self, disabled: bool) -> Color {
        if disabled {
            self.text_disabled
        } else {
            self.accent
        }
    }

    /// Text color respecting disabled state.
    ///
    /// Returns `text_disabled` when disabled, `text_primary` otherwise.
    pub fn interactive_text(&self, disabled: bool) -> Color {
        if disabled {
            self.text_disabled
        } else {
            self.text_primary
        }
    }

    /// Dark theme matching the OASIS balatro aesthetic.
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
            toggle_track_off: Color::rgba(255, 255, 255, 10),
            toggle_track_on: Color::rgb(80, 160, 255),
            toggle_thumb: Color::rgb(255, 255, 255),
            slider_track: Color::rgb(25, 25, 35),
            slider_fill: Color::rgb(80, 160, 255),
            slider_thumb: Color::rgb(30, 30, 40),
            menu_bg: Color::rgb(240, 240, 240),
            menu_border: Color::rgb(180, 180, 180),
            menu_text: Color::rgb(30, 30, 30),
            menu_hover_bg: Color::rgb(49, 106, 197),
            menu_hover_text: Color::rgb(255, 255, 255),
            menu_dropdown_bg: Color::rgb(236, 236, 236),
            menu_dropdown_border_light: Color::rgb(255, 255, 255),
            menu_dropdown_border_dark: Color::rgb(105, 105, 105),
            menu_item_text: Color::rgb(20, 20, 20),
            menu_disabled_text: Color::rgb(150, 150, 150),
            menu_separator: Color::rgb(170, 170, 170),
            tooltip_bg: Color::rgb(50, 50, 65),
            tooltip_text: Color::rgb(220, 220, 230),

            focus_ring_color: None,
            focus_ring_width: None,
            focus_ring_offset: None,

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

            reduced_motion: false,
            font_scale: 1.0,
            text_direction: TextDirection::Ltr,
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
            toggle_track_off: Color::rgba(0, 0, 0, 10),
            toggle_track_on: Color::rgb(50, 120, 220),
            toggle_thumb: Color::rgb(255, 255, 255),
            slider_track: Color::rgb(255, 255, 255),
            slider_fill: Color::rgb(50, 120, 220),
            slider_thumb: Color::rgb(255, 255, 255),
            menu_bg: Color::rgb(240, 240, 240),
            menu_border: Color::rgb(180, 180, 180),
            menu_text: Color::rgb(30, 30, 30),
            menu_hover_bg: Color::rgb(49, 106, 197),
            menu_hover_text: Color::rgb(255, 255, 255),
            menu_dropdown_bg: Color::rgb(236, 236, 236),
            menu_dropdown_border_light: Color::rgb(255, 255, 255),
            menu_dropdown_border_dark: Color::rgb(105, 105, 105),
            menu_item_text: Color::rgb(20, 20, 20),
            menu_disabled_text: Color::rgb(150, 150, 150),
            menu_separator: Color::rgb(170, 170, 170),
            tooltip_bg: Color::rgb(40, 40, 50),
            tooltip_text: Color::rgb(240, 240, 245),

            focus_ring_color: None,
            focus_ring_width: None,
            focus_ring_offset: None,

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

            reduced_motion: false,
            font_scale: 1.0,
            text_direction: TextDirection::Ltr,
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
        theme.toggle_track_on = theme.accent;
        theme.slider_fill = theme.accent;
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
            toggle_track_off: Color::rgba(255, 255, 255, 30),
            toggle_track_on: Color::rgb(0, 255, 255),
            toggle_thumb: Color::rgb(0, 0, 0),
            slider_track: Color::rgb(0, 0, 0),
            slider_fill: Color::rgb(0, 255, 255),
            slider_thumb: Color::rgb(0, 0, 0),
            menu_bg: Color::rgb(240, 240, 240),
            menu_border: Color::rgb(180, 180, 180),
            menu_text: Color::rgb(30, 30, 30),
            menu_hover_bg: Color::rgb(49, 106, 197),
            menu_hover_text: Color::rgb(255, 255, 255),
            menu_dropdown_bg: Color::rgb(236, 236, 236),
            menu_dropdown_border_light: Color::rgb(255, 255, 255),
            menu_dropdown_border_dark: Color::rgb(105, 105, 105),
            menu_item_text: Color::rgb(20, 20, 20),
            menu_disabled_text: Color::rgb(150, 150, 150),
            menu_separator: Color::rgb(170, 170, 170),
            tooltip_bg: Color::rgb(0, 0, 0),
            tooltip_text: Color::rgb(255, 255, 255),

            focus_ring_color: None,
            focus_ring_width: None,
            focus_ring_offset: None,

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

            reduced_motion: false,
            font_scale: 1.0,
            text_direction: TextDirection::Ltr,
        }
    }

    /// Color-blind friendly theme optimized for protanopia (red-blind).
    ///
    /// Uses a blue/yellow axis for status differentiation:
    /// - Success: cyan (visible on blue/yellow axis)
    /// - Warning: yellow (high luminance, distinct)
    /// - Error: blue-violet (distinct from cyan/yellow)
    pub fn protanopia() -> Self {
        let mut theme = Self::dark();
        theme.success = Color::rgb(0, 200, 200); // cyan
        theme.warning = Color::rgb(255, 220, 50); // yellow
        theme.error = Color::rgb(130, 80, 220); // blue-violet
        theme.info = Color::rgb(100, 180, 255); // light blue
        theme
    }

    /// Color-blind friendly theme optimized for tritanopia (blue-blind).
    ///
    /// Uses a red/cyan axis for status differentiation:
    /// - Success: teal (visible without blue perception)
    /// - Warning: red-orange (high contrast, distinct)
    /// - Error: deep red (distinct from teal/orange)
    pub fn tritanopia() -> Self {
        let mut theme = Self::dark();
        theme.success = Color::rgb(0, 180, 160); // teal
        theme.warning = Color::rgb(255, 120, 40); // red-orange
        theme.error = Color::rgb(200, 30, 30); // deep red
        theme.info = Color::rgb(0, 160, 200); // dark cyan
        theme
    }

    /// Color-blind friendly theme optimized for deuteranopia.
    ///
    /// Replaces the standard success/warning/error color scheme with
    /// alternatives that are distinguishable by people with red-green
    /// color blindness:
    /// - Success: blue (instead of green)
    /// - Warning: orange (instead of amber)
    /// - Error: magenta (instead of red)
    pub fn colorblind() -> Self {
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

            // Deuteranopia-safe status colors:
            // Blue for success (clearly distinct from orange/magenta).
            success: Color::rgb(60, 140, 255),
            // Orange for warning (high luminance, distinct hue).
            warning: Color::rgb(255, 160, 40),
            // Magenta for error (distinct from blue and orange).
            error: Color::rgb(220, 60, 220),
            // Cyan for info.
            info: Color::rgb(0, 200, 220),

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
            toggle_track_off: Color::rgba(255, 255, 255, 10),
            toggle_track_on: Color::rgb(80, 160, 255),
            toggle_thumb: Color::rgb(255, 255, 255),
            slider_track: Color::rgb(25, 25, 35),
            slider_fill: Color::rgb(80, 160, 255),
            slider_thumb: Color::rgb(30, 30, 40),
            menu_bg: Color::rgb(240, 240, 240),
            menu_border: Color::rgb(180, 180, 180),
            menu_text: Color::rgb(30, 30, 30),
            menu_hover_bg: Color::rgb(49, 106, 197),
            menu_hover_text: Color::rgb(255, 255, 255),
            menu_dropdown_bg: Color::rgb(236, 236, 236),
            menu_dropdown_border_light: Color::rgb(255, 255, 255),
            menu_dropdown_border_dark: Color::rgb(105, 105, 105),
            menu_item_text: Color::rgb(20, 20, 20),
            menu_disabled_text: Color::rgb(150, 150, 150),
            menu_separator: Color::rgb(170, 170, 170),
            tooltip_bg: Color::rgb(50, 50, 65),
            tooltip_text: Color::rgb(220, 220, 230),

            focus_ring_color: None,
            focus_ring_width: None,
            focus_ring_offset: None,

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

            reduced_motion: false,
            font_scale: 1.0,
            text_direction: TextDirection::Ltr,
        }
    }

    /// Returns `true` if the theme's text direction is RTL.
    pub fn is_rtl(&self) -> bool {
        self.text_direction.is_rtl()
    }

    /// Map logical inline-start/inline-end padding to physical
    /// left/right based on text direction.
    pub fn resolve_inline_padding(&self, inline_start: u16, inline_end: u16) -> (u16, u16) {
        if self.is_rtl() {
            (inline_end, inline_start)
        } else {
            (inline_start, inline_end)
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
            Theme::colorblind(),
            Theme::protanopia(),
            Theme::tritanopia(),
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

    // -- Reduced-motion tests --

    #[test]
    fn default_reduced_motion_is_false() {
        assert!(!Theme::dark().reduced_motion);
        assert!(!Theme::light().reduced_motion);
        assert!(!Theme::classic().reduced_motion);
        assert!(!Theme::high_contrast().reduced_motion);
        assert!(!Theme::colorblind().reduced_motion);
        assert!(!Theme::protanopia().reduced_motion);
        assert!(!Theme::tritanopia().reduced_motion);
    }

    #[test]
    fn reduced_motion_can_be_enabled() {
        let mut t = Theme::dark();
        t.reduced_motion = true;
        assert!(t.reduced_motion);
    }

    // -- Font scale tests --

    #[test]
    fn default_font_scale_is_one() {
        assert!((Theme::dark().font_scale - 1.0).abs() < f32::EPSILON);
        assert!((Theme::colorblind().font_scale - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn scaled_font_size_identity() {
        let t = Theme::dark();
        assert_eq!(t.scaled_font_size(8), 8);
        assert_eq!(t.scaled_font_size(16), 16);
    }

    #[test]
    fn scaled_font_size_double() {
        let mut t = Theme::dark();
        t.font_scale = 2.0;
        assert_eq!(t.scaled_font_size(8), 16);
        assert_eq!(t.scaled_font_size(16), 32);
    }

    #[test]
    fn scaled_font_size_fractional() {
        let mut t = Theme::dark();
        t.font_scale = 1.5;
        assert_eq!(t.scaled_font_size(8), 12);
        assert_eq!(t.scaled_font_size(16), 24);
    }

    #[test]
    fn scaled_font_size_clamped_low() {
        let mut t = Theme::dark();
        t.font_scale = 0.01; // Below minimum 0.5
        // 8 * 0.5 = 4 (clamped to 0.5)
        assert_eq!(t.scaled_font_size(8), 4);
    }

    #[test]
    fn scaled_font_size_clamped_high() {
        let mut t = Theme::dark();
        t.font_scale = 10.0; // Above maximum 3.0
        // 8 * 3.0 = 24 (clamped to 3.0)
        assert_eq!(t.scaled_font_size(8), 24);
    }

    #[test]
    fn scaled_font_size_minimum_one() {
        let mut t = Theme::dark();
        t.font_scale = 0.5;
        // Even with small base, result is at least 1.
        assert!(t.scaled_font_size(1) >= 1);
    }

    // -- Color-blind theme tests --

    #[test]
    fn colorblind_has_distinct_status_colors() {
        let t = Theme::colorblind();
        // All four status colors should be different.
        assert_ne!(t.success, t.warning);
        assert_ne!(t.success, t.error);
        assert_ne!(t.warning, t.error);
        assert_ne!(t.success, t.info);
    }

    #[test]
    fn colorblind_success_is_blue() {
        let t = Theme::colorblind();
        // Success should be blue-dominant (high blue, low-ish red).
        assert!(t.success.b > t.success.r);
        assert!(t.success.b > 200);
    }

    #[test]
    fn colorblind_error_is_magenta() {
        let t = Theme::colorblind();
        // Error should be magenta (high red + high blue, low green).
        assert!(t.error.r > 200);
        assert!(t.error.b > 200);
        assert!(t.error.g < 100);
    }

    #[test]
    fn colorblind_warning_is_orange() {
        let t = Theme::colorblind();
        // Warning should be orange (high red, medium green, low blue).
        assert!(t.warning.r > 200);
        assert!(t.warning.g > 100 && t.warning.g < 200);
        assert!(t.warning.b < 100);
    }

    #[test]
    fn colorblind_shares_dark_base_colors() {
        let dark = Theme::dark();
        let cb = Theme::colorblind();
        assert_eq!(dark.background, cb.background);
        assert_eq!(dark.surface, cb.surface);
        assert_eq!(dark.text_primary, cb.text_primary);
    }

    #[test]
    fn colorblind_has_dark_background() {
        let t = Theme::colorblind();
        assert!(t.background.r < 50);
        assert!(t.background.g < 50);
        assert!(t.background.b < 50);
    }

    // -- Protanopia theme tests --

    #[test]
    fn protanopia_has_distinct_status_colors() {
        let t = Theme::protanopia();
        assert_ne!(t.success, t.warning);
        assert_ne!(t.success, t.error);
        assert_ne!(t.warning, t.error);
        assert_ne!(t.success, t.info);
    }

    #[test]
    fn protanopia_success_is_cyan() {
        let t = Theme::protanopia();
        assert!(t.success.g >= 200);
        assert!(t.success.b >= 200);
        assert!(t.success.r < 50);
    }

    #[test]
    fn protanopia_shares_dark_base() {
        let dark = Theme::dark();
        let p = Theme::protanopia();
        assert_eq!(dark.background, p.background);
        assert_eq!(dark.surface, p.surface);
        assert_eq!(dark.text_primary, p.text_primary);
    }

    // -- Tritanopia theme tests --

    #[test]
    fn tritanopia_has_distinct_status_colors() {
        let t = Theme::tritanopia();
        assert_ne!(t.success, t.warning);
        assert_ne!(t.success, t.error);
        assert_ne!(t.warning, t.error);
        assert_ne!(t.success, t.info);
    }

    #[test]
    fn tritanopia_success_is_teal() {
        let t = Theme::tritanopia();
        assert!(t.success.g >= 150);
        assert!(t.success.b >= 150);
        assert!(t.success.r < 50);
    }

    #[test]
    fn tritanopia_shares_dark_base() {
        let dark = Theme::dark();
        let tr = Theme::tritanopia();
        assert_eq!(dark.background, tr.background);
        assert_eq!(dark.surface, tr.surface);
        assert_eq!(dark.text_primary, tr.text_primary);
    }

    #[test]
    fn protanopia_default_font_scale() {
        assert!((Theme::protanopia().font_scale - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn tritanopia_default_font_scale() {
        assert!((Theme::tritanopia().font_scale - 1.0).abs() < f32::EPSILON);
    }

    // -- WCAG AA validation across all themes --

    #[test]
    fn all_themes_text_meets_wcag_aa() {
        use oasis_types::color::{contrast_ratio, meets_wcag_aa};
        let themes = [
            ("dark", Theme::dark()),
            ("light", Theme::light()),
            ("classic", Theme::classic()),
            ("high_contrast", Theme::high_contrast()),
            ("colorblind", Theme::colorblind()),
            ("protanopia", Theme::protanopia()),
            ("tritanopia", Theme::tritanopia()),
        ];
        for (name, theme) in &themes {
            let ratio = contrast_ratio(theme.text_primary, theme.background);
            assert!(
                meets_wcag_aa(theme.text_primary, theme.background),
                "{name}: text_primary vs background contrast {ratio:.2} < 4.5"
            );
        }
    }

    #[test]
    fn high_contrast_exceeds_wcag_aaa() {
        use oasis_types::color::contrast_ratio;
        let t = Theme::high_contrast();
        // White on black should be ~21:1, well above AAA (7:1).
        let ratio = contrast_ratio(t.text_primary, t.background);
        assert!(ratio >= 7.0);
    }

    // -- interactive state helpers --

    #[test]
    fn interactive_border_disabled() {
        let t = Theme::dark();
        assert_eq!(t.interactive_border(true, false), t.border_subtle);
        assert_eq!(t.interactive_border(true, true), t.border_subtle);
    }

    #[test]
    fn interactive_border_selected() {
        let t = Theme::dark();
        assert_eq!(t.interactive_border(false, true), t.accent);
    }

    #[test]
    fn interactive_border_default() {
        let t = Theme::dark();
        assert_eq!(t.interactive_border(false, false), t.input_border);
    }

    #[test]
    fn interactive_accent_disabled() {
        let t = Theme::dark();
        assert_eq!(t.interactive_accent(true), t.text_disabled);
    }

    #[test]
    fn interactive_accent_enabled() {
        let t = Theme::dark();
        assert_eq!(t.interactive_accent(false), t.accent);
    }

    #[test]
    fn interactive_text_disabled() {
        let t = Theme::dark();
        assert_eq!(t.interactive_text(true), t.text_disabled);
    }

    #[test]
    fn interactive_text_enabled() {
        let t = Theme::dark();
        assert_eq!(t.interactive_text(false), t.text_primary);
    }

    // -- Widget slot defaults (theming completeness sweep) --
    //
    // The slider/menu slots were extracted from hardcoded draw-path
    // colors; these tests pin the defaults to the legacy values so the
    // refactor stays pixel-identical for every built-in theme.

    #[test]
    fn slider_slots_default_to_legacy_sources() {
        for t in [
            Theme::dark(),
            Theme::light(),
            Theme::high_contrast(),
            Theme::colorblind(),
        ] {
            assert_eq!(t.slider_track, t.input_bg);
            assert_eq!(t.slider_fill, t.accent);
            assert_eq!(t.slider_thumb, t.surface);
        }
    }

    #[test]
    fn slider_dark_defaults_match_old_literals() {
        let t = Theme::dark();
        assert_eq!(t.slider_track, Color::rgb(25, 25, 35));
        assert_eq!(t.slider_fill, Color::rgb(80, 160, 255));
        assert_eq!(t.slider_thumb, Color::rgb(30, 30, 40));
    }

    #[test]
    fn classic_slider_fill_follows_accent() {
        let t = Theme::classic();
        assert_eq!(t.slider_fill, t.accent);
        assert_eq!(t.slider_fill, Color::rgb(255, 140, 30));
    }

    #[test]
    fn menu_slots_default_to_win95_grays_in_all_themes() {
        for t in [
            Theme::dark(),
            Theme::light(),
            Theme::classic(),
            Theme::high_contrast(),
            Theme::colorblind(),
            Theme::protanopia(),
            Theme::tritanopia(),
        ] {
            assert_eq!(t.menu_bg, Color::rgb(240, 240, 240));
            assert_eq!(t.menu_border, Color::rgb(180, 180, 180));
            assert_eq!(t.menu_text, Color::rgb(30, 30, 30));
            assert_eq!(t.menu_hover_bg, Color::rgb(49, 106, 197));
            assert_eq!(t.menu_hover_text, Color::rgb(255, 255, 255));
            assert_eq!(t.menu_dropdown_bg, Color::rgb(236, 236, 236));
            assert_eq!(t.menu_dropdown_border_light, Color::rgb(255, 255, 255));
            assert_eq!(t.menu_dropdown_border_dark, Color::rgb(105, 105, 105));
            assert_eq!(t.menu_item_text, Color::rgb(20, 20, 20));
            assert_eq!(t.menu_disabled_text, Color::rgb(150, 150, 150));
            assert_eq!(t.menu_separator, Color::rgb(170, 170, 170));
        }
    }
}
