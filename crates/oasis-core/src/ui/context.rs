//! Theme-aware drawing context.
//!
//! All oasis-ui widgets render through `DrawContext`, which wraps a
//! `&mut dyn SdiBackend` and provides access to the active theme.

use crate::backend::{Color, SdiBackend, TextureId};
use crate::error::Result;
use crate::ui::layout::Padding;
use crate::ui::shadow::Shadow;
use crate::ui::theme::Theme;

/// Drawing context wrapping a backend and theme.
pub struct DrawContext<'a> {
    pub backend: &'a mut dyn SdiBackend,
    pub theme: &'a Theme,
}

impl<'a> DrawContext<'a> {
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
        let color = if pressed {
            self.theme.button_bg_pressed
        } else if hover {
            self.theme.button_bg_hover
        } else {
            self.theme.button_bg
        };
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
        let color = if pressed {
            self.theme.accent_pressed
        } else if hover {
            self.theme.accent_hover
        } else {
            self.theme.accent
        };
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
    pub backend: &'a mut dyn SdiBackend,
    pub theme: &'a Theme,
}

impl Drop for Region<'_> {
    fn drop(&mut self) {
        let _ = self.backend.pop_region();
    }
}
