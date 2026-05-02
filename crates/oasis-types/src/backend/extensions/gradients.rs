//! `SdiGradients` extension trait.
//!
//! Extracted from the historical monolithic `extensions.rs`.

use crate::backend::{GradientStyle, SdiCore};
use crate::error::Result;

// ---------------------------------------------------------------------------
// SdiGradients
// ---------------------------------------------------------------------------

/// Gradient fill operations.
pub trait SdiGradients: SdiCore {
    /// Draw a filled rectangle with a gradient.
    fn fill_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gradient: &GradientStyle,
    ) -> Result<()> {
        self.fill_rect(x, y, w, h, gradient.primary_color())
    }

    /// Draw a filled rounded rectangle with a gradient.
    fn fill_rounded_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        _radius: u16,
        gradient: &GradientStyle,
    ) -> Result<()> {
        self.fill_rect(x, y, w, h, gradient.primary_color())
    }
}
