//! `SdiBatch` extension trait.

use crate::backend::{Color, SdiText};
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

/// Batched geometry submission.
///
/// Both methods take a slice of items that share one draw state, so a GPU
/// backend can submit them as one geometry call. The defaults loop over
/// [`SdiCore::fill_rect`](crate::backend::SdiCore::fill_rect) /
/// [`SdiText::draw_text_styled`], so a batch
/// draws exactly the pixels of the same items drawn one by one.
pub trait SdiBatch: SdiText {
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
    /// this to coalesce glyph atlas lookups; an override must keep the
    /// output of the default, one
    /// [`draw_text_styled`](SdiText::draw_text_styled) per item (so a
    /// backend with true bold / italic glyphs uses them here too, instead
    /// of a double-strike that blends translucent text twice).
    fn submit_text_batch(
        &mut self,
        texts: &[BatchText<'_>],
        font_size: u16,
        bold: bool,
        italic: bool,
    ) -> Result<()> {
        for t in texts {
            self.draw_text_styled(t.text, t.x, t.y, font_size, t.color, bold, italic)?;
        }
        Ok(())
    }
}
