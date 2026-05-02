//! `SdiBatch` extension trait.
//!
//! Extracted from the historical monolithic `extensions.rs`.

use crate::backend::{Color, SdiCore};
use crate::error::Result;

// ---------------------------------------------------------------------------
// SdiBatch
// ---------------------------------------------------------------------------

/// A text item for batched submission via [`SdiBatch::submit_text_batch`].
#[derive(Debug, Clone)]
pub struct BatchText<'a> {
    /// The text string to render.
    pub text: &'a str,
    /// X position in screen pixels.
    pub x: i32,
    /// Y position in screen pixels.
    pub y: i32,
    /// Fill color.
    pub color: Color,
}

/// A rectangle for batched submission via [`SdiBatch::submit_rect_batch`].
#[derive(Debug, Clone, Copy)]
pub struct BatchRect {
    /// X position in screen pixels.
    pub x: i32,
    /// Y position in screen pixels.
    pub y: i32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
    /// Fill color.
    pub color: Color,
}

/// Batch rendering operations (begin/flush command queues).
pub trait SdiBatch: SdiCore {
    /// Begin recording draw commands into a batch.
    fn begin_batch(&mut self) -> Result<()> {
        Ok(())
    }

    /// Flush and execute all batched draw commands.
    fn flush_batch(&mut self) -> Result<()> {
        Ok(())
    }

    /// Submit a batch of solid colored rectangles in a single call.
    ///
    /// Backends can override this to submit all rectangles as GPU geometry
    /// (e.g. `SDL_RenderGeometry`, PSP `sceGumDrawArray`) reducing draw call
    /// overhead and command buffer usage. The default issues individual
    /// `fill_rect` calls which is correct but not batched.
    fn submit_rect_batch(&mut self, rects: &[BatchRect]) -> Result<()> {
        for r in rects {
            self.fill_rect(r.x, r.y, r.w, r.h, r.color)?;
        }
        Ok(())
    }

    /// Submit a batch of text items sharing the same font style.
    ///
    /// All items in the batch share `font_size`, `bold`, and `italic`
    /// but may have different positions and colors. Backends can override
    /// this to coalesce glyph atlas lookups. The default issues individual
    /// `draw_text` calls with faux-bold double-strike.
    fn submit_text_batch(
        &mut self,
        texts: &[BatchText<'_>],
        font_size: u16,
        bold: bool,
        _italic: bool,
    ) -> Result<()> {
        for t in texts {
            self.draw_text(t.text, t.x, t.y, font_size, t.color)?;
            if bold {
                self.draw_text(t.text, t.x + 1, t.y, font_size, t.color)?;
            }
        }
        Ok(())
    }
}
