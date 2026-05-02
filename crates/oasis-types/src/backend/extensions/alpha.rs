//! `SdiAlpha` extension trait.
//!
//! Extracted from the historical monolithic `extensions.rs`.

use crate::backend::{Color, SdiCore};
use crate::error::Result;

// ---------------------------------------------------------------------------
// SdiAlpha
// ---------------------------------------------------------------------------

/// Alpha blending and viewport utilities.
pub trait SdiAlpha: SdiCore {
    /// Draw a filled rectangle with explicit alpha override.
    fn fill_rect_alpha(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
        alpha: u8,
    ) -> Result<()> {
        self.fill_rect(x, y, w, h, color.with_alpha(alpha))
    }

    /// Return the current viewport dimensions `(width, height)`.
    fn viewport_size(&self) -> (u32, u32) {
        (
            crate::backend::DEFAULT_VIEWPORT_WIDTH,
            crate::backend::DEFAULT_VIEWPORT_HEIGHT,
        )
    }

    /// Dim the entire viewport with a semi-transparent overlay.
    fn dim_screen(&mut self, alpha: u8) -> Result<()> {
        let (w, h) = self.viewport_size();
        self.fill_rect(0, 0, w, h, Color::rgba(0, 0, 0, alpha))
    }
}
