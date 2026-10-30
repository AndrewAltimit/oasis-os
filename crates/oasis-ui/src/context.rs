//! Theme-aware drawing context.
//!
//! All oasis-ui widgets render through `DrawContext`, which wraps a
//! `&mut dyn SdiBackend` and provides access to the active theme.

use crate::layout::{self, HAlign, Padding};
use crate::shadow::Shadow;
use crate::theme::Theme;
use oasis_types::backend::{Color, SdiBackend, TextureId};
use oasis_types::error::Result;

/// Drawing context wrapping a backend and theme.
pub struct DrawContext<'a> {
    /// Backend for rendering operations.
    pub backend: &'a mut dyn SdiBackend,
    /// Theme for UI styling.
    pub theme: &'a Theme,
}

impl<'a> DrawContext<'a> {
    /// Create a new drawing context.
    pub fn new(backend: &'a mut dyn SdiBackend, theme: &'a Theme) -> Self {
        Self { backend, theme }
    }

    // -- Convenience drawing methods --

    /// Draw a themed panel background with optional elevation shadow.
    pub fn panel(&mut self, x: i32, y: i32, w: u32, h: u32, elevation: u8) -> Result<()> {
        let radius = self.theme.border_radius_lg;
        let shadow = Shadow::elevation(elevation);
        shadow.draw(self.backend, x, y, w, h, radius)?;
        self.backend
            .fill_rounded_rect(x, y, w, h, radius, self.theme.surface)
    }

    /// Draw a themed label with default font size and primary text color.
    pub fn label(&mut self, text: &str, x: i32, y: i32) -> Result<()> {
        self.backend
            .draw_text(text, x, y, self.theme.font_size_md, self.theme.text_primary)
    }

    /// Draw a themed label with a specific style.
    pub fn label_styled(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
    ) -> Result<()> {
        self.backend.draw_text(text, x, y, font_size, color)
    }

    /// Draw a themed heading.
    pub fn heading(&mut self, text: &str, x: i32, y: i32) -> Result<()> {
        self.backend
            .draw_text(text, x, y, self.theme.font_size_xl, self.theme.text_primary)
    }

    /// Draw a divider line.
    pub fn divider_h(&mut self, x: i32, y: i32, w: u32) -> Result<()> {
        self.backend
            .draw_line(x, y, x + w as i32, y, 1, self.theme.border_subtle)
    }

    /// Draw a vertical divider.
    pub fn divider_v(&mut self, x: i32, y: i32, h: u32) -> Result<()> {
        self.backend
            .draw_line(x, y, x, y + h as i32, 1, self.theme.border_subtle)
    }

    /// Draw a themed button background.
    pub fn button_bg(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        hover: bool,
        pressed: bool,
    ) -> Result<()> {
        let state = crate::states::WidgetState::from_flags(hover, pressed, false);
        let color = crate::states::WidgetStateColors::neutral_bg(self.theme, state);
        let radius = self.theme.border_radius_md;
        self.backend.fill_rounded_rect(x, y, w, h, radius, color)
    }

    /// Draw a themed accent button background.
    pub fn accent_button_bg(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        hover: bool,
        pressed: bool,
    ) -> Result<()> {
        let state = crate::states::WidgetState::from_flags(hover, pressed, false);
        let color = crate::states::WidgetStateColors::accent_bg(self.theme, state);
        let radius = self.theme.border_radius_md;
        self.backend.fill_rounded_rect(x, y, w, h, radius, color)
    }

    /// Blit a texture.
    pub fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.backend.blit(tex, x, y, w, h)
    }

    /// Measure text width using theme default font size.
    pub fn measure_text(&self, text: &str) -> u32 {
        self.backend.measure_text(text, self.theme.font_size_md)
    }

    /// Measure text extents with a specific font size.
    pub fn measure_text_sized(&self, text: &str, font_size: u16) -> (u32, u32) {
        self.backend.measure_text_extents(text, font_size)
    }

    /// Draw `text` inside `rect = (x, y, w, h)`: vertically centered,
    /// horizontally placed by `align`.
    ///
    /// The shared form of the widgets' "measure, `layout::center`,
    /// `draw_text`" idiom; positions are exactly
    /// `x + layout::align_x(w, text_w, align)` and
    /// `y + layout::center(h, measure_text_height(font_size))`. Text wider
    /// or taller than the rect starts at its left / top edge (it is not
    /// clipped).
    pub fn text_in(
        &mut self,
        rect: (i32, i32, u32, u32),
        text: &str,
        font_size: u16,
        color: Color,
        align: HAlign,
    ) -> Result<()> {
        let (x, y, w, h) = rect;
        let text_w = self.backend.measure_text(text, font_size);
        let text_h = self.backend.measure_text_height(font_size);
        let tx = x + layout::align_x(w, text_w, align);
        let ty = y + layout::center(h, text_h);
        self.backend.draw_text(text, tx, ty, font_size, color)
    }

    /// Draw a rounded box: `bg` fill, then a 1-pixel `border` stroke, both
    /// with corner `radius`, over `rect = (x, y, w, h)`.
    pub fn frame(
        &mut self,
        rect: (i32, i32, u32, u32),
        bg: Color,
        border: Color,
        radius: u16,
    ) -> Result<()> {
        let (x, y, w, h) = rect;
        self.backend.fill_rounded_rect(x, y, w, h, radius, bg)?;
        self.backend
            .stroke_rounded_rect(x, y, w, h, radius, 1, border)
    }

    /// Inner rect after applying padding.
    pub fn padded_rect(
        &self,
        padding: &Padding,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> (i32, i32, u32, u32) {
        padding.inner_rect(x, y, w, h)
    }

    /// Push a rendering sub-region (translate + clip).
    ///
    /// Returns a `Region` RAII guard that automatically pops the clip
    /// and translate when dropped. Use `region.backend` and
    /// `region.theme` to draw within the sub-region.
    pub fn push_region(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<Region<'_>> {
        self.backend.push_region(x, y, w, h)?;
        Ok(Region {
            backend: &mut *self.backend,
            theme: self.theme,
        })
    }
}

/// RAII guard that pops a pushed region (clip + translate) on drop.
///
/// Provides direct access to `backend` and `theme` for drawing within
/// the sub-region. When the guard is dropped, the region is popped
/// automatically.
pub struct Region<'a> {
    /// Backend for rendering operations within the region.
    pub backend: &'a mut dyn SdiBackend,
    /// Theme for UI styling.
    pub theme: &'a Theme,
}

impl Drop for Region<'_> {
    fn drop(&mut self) {
        let _ = self.backend.pop_region();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::RecordingBackend;
    use oasis_test_backend::DrawCommand;
    use oasis_types::backend::{SdiCore, SdiShapes, SdiText};

    fn text_at(rec: &RecordingBackend) -> (i32, i32) {
        let t = rec.text_positions();
        assert_eq!(t.len(), 1);
        (t[0].1, t[0].2)
    }

    #[test]
    fn text_in_matches_the_manual_idiom() {
        let theme = Theme::dark();
        for align in [HAlign::Left, HAlign::Center, HAlign::Right] {
            let mut rec = RecordingBackend::default();
            let text_w = rec.measure_text("Label", 12);
            let text_h = rec.measure_text_height(12);
            {
                let mut ctx = DrawContext::new(&mut rec, &theme);
                ctx.text_in((10, 20, 101, 31), "Label", 12, Color::WHITE, align)
                    .unwrap();
            }
            let x = match align {
                HAlign::Left => 10,
                HAlign::Center => 10 + layout::center(101, text_w),
                HAlign::Right => 10 + 101 - text_w as i32,
            };
            assert_eq!(text_at(&rec), (x, 20 + layout::center(31, text_h)));
        }
    }

    #[test]
    fn text_in_wider_than_rect_starts_at_left_edge() {
        let theme = Theme::dark();
        let mut rec = RecordingBackend::default();
        {
            let mut ctx = DrawContext::new(&mut rec, &theme);
            ctx.text_in(
                (5, 5, 4, 2),
                "Much too wide",
                12,
                Color::WHITE,
                HAlign::Right,
            )
            .unwrap();
        }
        assert_eq!(text_at(&rec), (5, 5));
    }

    #[test]
    fn frame_is_fill_then_one_pixel_stroke() {
        let theme = Theme::dark();
        let (bg, border) = (Color::rgb(1, 2, 3), Color::rgb(4, 5, 6));
        let mut framed = RecordingBackend::default();
        DrawContext::new(&mut framed, &theme)
            .frame((3, 4, 50, 20), bg, border, 4)
            .unwrap();
        let mut manual = RecordingBackend::default();
        manual.fill_rounded_rect(3, 4, 50, 20, 4, bg).unwrap();
        manual
            .stroke_rounded_rect(3, 4, 50, 20, 4, 1, border)
            .unwrap();
        assert_eq!(framed.commands(), manual.commands());
        assert!(matches!(framed.commands()[0], DrawCommand::FillRect { .. }));
    }
}
