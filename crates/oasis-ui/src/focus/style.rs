//! [`FocusStyle`] and the free focus-ring drawing helpers.

use super::Rect;
use oasis_types::backend::Color;

/// Visual style for focus indicators drawn around focused widgets.
#[derive(Debug, Clone, Copy)]
pub struct FocusStyle {
    /// Border color for the focus indicator.
    pub color: Color,
    /// Border width in pixels.
    pub width: u16,
    /// Corner radius for the focus border.
    pub radius: u16,
    /// Gap between the widget edge and the focus border.
    pub offset: i32,
}

impl FocusStyle {
    /// Create a focus style from a theme's accent color.
    pub fn from_accent(accent: Color) -> Self {
        Self {
            color: accent,
            width: 1,
            radius: 2,
            offset: 1,
        }
    }

    /// Create a focus style from a theme, honoring the skin-provided
    /// `focus_ring_*` overrides.
    ///
    /// When the theme sets no focus ring fields (the default), this is
    /// exactly [`FocusStyle::from_accent`] on the theme's accent color,
    /// so skins that don't author `[geometry] focus_ring_*` render
    /// identically to before.
    pub fn from_theme(theme: &crate::theme::Theme) -> Self {
        let mut style = Self::from_accent(theme.accent);
        if let Some(color) = theme.focus_ring_color {
            style.color = color;
        }
        if let Some(width) = theme.focus_ring_width {
            style.width = width;
        }
        if let Some(offset) = theme.focus_ring_offset {
            style.offset = offset;
        }
        style
    }

    /// Compute the focus indicator rectangle given a widget rect.
    pub fn indicator_rect(&self, x: i32, y: i32, w: u32, h: u32) -> (i32, i32, u32, u32) {
        let off = self.offset;
        let delta = off.saturating_mul(2);
        (
            x.saturating_sub(off),
            y.saturating_sub(off),
            w.saturating_add_signed(delta),
            h.saturating_add_signed(delta),
        )
    }

    /// Draw the focus indicator around a widget rectangle.
    pub fn draw(
        &self,
        backend: &mut dyn oasis_types::backend::SdiBackend,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> oasis_types::error::Result<()> {
        let (fx, fy, fw, fh) = self.indicator_rect(x, y, w, h);
        backend.stroke_rounded_rect(fx, fy, fw, fh, self.radius, self.width, self.color)
    }
}

/// Draw a focus ring indicator around the given bounds.
///
/// This is a convenience function that widgets can call when they
/// detect they are focused. Uses `FocusStyle` internally. For a ring
/// that honors skin-authored `focus_ring_*` theme fields, use
/// [`draw_focus_ring_themed`].
pub fn draw_focus_ring(
    backend: &mut dyn oasis_types::backend::SdiBackend,
    bounds: &Rect,
    color: Color,
) -> oasis_types::error::Result<()> {
    let style = FocusStyle {
        color,
        width: 2,
        radius: 3,
        offset: 2,
    };
    style.draw(backend, bounds.x, bounds.y, bounds.w, bounds.h)
}

/// Draw a focus ring indicator styled by the theme.
///
/// Uses the theme's `focus_ring_*` overrides when the skin sets them.
/// Unset fields fall back to [`draw_focus_ring`]'s visuals (accent
/// color, 2 px stroke, offset 2), so skins without focus ring theming
/// render identically to before.
pub fn draw_focus_ring_themed(
    backend: &mut dyn oasis_types::backend::SdiBackend,
    bounds: &Rect,
    theme: &crate::theme::Theme,
) -> oasis_types::error::Result<()> {
    let style = FocusStyle {
        color: theme.focus_ring_color.unwrap_or(theme.accent),
        width: theme.focus_ring_width.unwrap_or(2),
        radius: 3,
        offset: theme.focus_ring_offset.unwrap_or(2),
    };
    style.draw(backend, bounds.x, bounds.y, bounds.w, bounds.h)
}

#[cfg(test)]
mod tests {
    use crate::focus::*;
    #[allow(unused_imports)]
    use oasis_types::backend::Color;
    // -- FocusStyle tests --

    #[test]
    fn focus_style_from_accent() {
        let s = FocusStyle::from_accent(Color::rgb(80, 160, 255));
        assert_eq!(s.color, Color::rgb(80, 160, 255));
        assert_eq!(s.width, 1);
        assert_eq!(s.radius, 2);
        assert_eq!(s.offset, 1);
    }

    #[test]
    fn focus_style_from_theme_unset_matches_from_accent() {
        // A theme without focus_ring_* overrides must produce exactly
        // the accent-derived style (pixel-identical no-override path).
        let theme = crate::theme::Theme::dark();
        let themed = FocusStyle::from_theme(&theme);
        let derived = FocusStyle::from_accent(theme.accent);
        assert_eq!(themed.color, derived.color);
        assert_eq!(themed.width, derived.width);
        assert_eq!(themed.radius, derived.radius);
        assert_eq!(themed.offset, derived.offset);
    }

    #[test]
    fn focus_style_from_theme_honors_overrides() {
        let mut theme = crate::theme::Theme::dark();
        theme.focus_ring_color = Some(Color::rgba(255, 0, 255, 160));
        theme.focus_ring_width = Some(3);
        theme.focus_ring_offset = Some(4);
        let s = FocusStyle::from_theme(&theme);
        assert_eq!(s.color, Color::rgba(255, 0, 255, 160));
        assert_eq!(s.width, 3);
        assert_eq!(s.offset, 4);
        // Radius stays at the accent-derived default.
        assert_eq!(s.radius, 2);
    }

    #[test]
    fn focus_style_from_theme_partial_override() {
        let mut theme = crate::theme::Theme::dark();
        theme.focus_ring_color = Some(Color::rgb(0, 255, 0));
        let s = FocusStyle::from_theme(&theme);
        assert_eq!(s.color, Color::rgb(0, 255, 0));
        // Unset fields keep the accent-derived defaults.
        assert_eq!(s.width, 1);
        assert_eq!(s.offset, 1);
    }

    #[test]
    fn draw_focus_ring_themed_calls_backend() {
        use crate::test_utils::RecordingBackend;
        let mut theme = crate::theme::Theme::dark();
        theme.focus_ring_color = Some(Color::rgb(255, 128, 0));
        let mut backend = RecordingBackend::default();
        let bounds = Rect::new(10, 20, 80, 40);
        draw_focus_ring_themed(&mut backend, &bounds, &theme).ok();
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn manager_draw_indicator_themed() {
        use crate::test_utils::RecordingBackend;
        let mut fm = FocusManager::new(3);
        fm.active = true;
        let theme = crate::theme::Theme::dark();
        let mut backend = RecordingBackend::default();
        fm.draw_indicator_themed(&mut backend, 0, &theme, 10, 20, 80, 30)
            .ok();
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn focus_style_indicator_rect() {
        let s = FocusStyle {
            color: Color::WHITE,
            width: 2,
            radius: 4,
            offset: 2,
        };
        let (fx, fy, fw, fh) = s.indicator_rect(10, 20, 100, 50);
        assert_eq!(fx, 8);
        assert_eq!(fy, 18);
        assert_eq!(fw, 104);
        assert_eq!(fh, 54);
    }

    #[test]
    fn focus_style_indicator_rect_zero_offset() {
        let s = FocusStyle {
            color: Color::WHITE,
            width: 1,
            radius: 0,
            offset: 0,
        };
        let r = s.indicator_rect(5, 10, 80, 30);
        assert_eq!(r, (5, 10, 80, 30));
    }

    #[test]
    fn focus_style_draw_calls_backend() {
        use crate::test_utils::RecordingBackend;
        let s = FocusStyle::from_accent(Color::rgb(0, 255, 0));
        let mut backend = RecordingBackend::default();
        s.draw(&mut backend, 10, 20, 100, 50).ok();
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_focus_ring_calls_backend() {
        use crate::test_utils::RecordingBackend;
        let mut backend = RecordingBackend::default();
        let bounds = Rect::new(10, 20, 80, 40);
        draw_focus_ring(&mut backend, &bounds, Color::rgb(0, 120, 255)).ok();
        assert!(backend.fill_rect_count() > 0);
    }
}
