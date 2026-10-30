//! `SdiClipTransform` extension trait.

use crate::backend::SdiCore;
use crate::error::Result;
use crate::geometry::ClipRect;

// ---------------------------------------------------------------------------
// SdiClipTransform
// ---------------------------------------------------------------------------

/// Clip rectangle and coordinate translation stack operations.
///
/// # Contract
///
/// Unlike the absolute [`SdiCore::set_clip_rect`], the clip *stack* works
/// in translated coordinates and nests:
///
/// - [`push_clip_rect`](Self::push_clip_rect) translates its rectangle by
///   the current translation, intersects it with the clip pushed before it
///   (an empty intersection rejects every draw) and applies the result.
/// - [`pop_clip_rect`](Self::pop_clip_rect) re-applies the clip that was
///   pushed before, or removes the clip when the stack becomes empty.
/// - [`current_clip_rect`](Self::current_clip_rect) reports the top of the
///   stack in surface pixels (`None` when nothing is pushed).
///
/// The stack and `set_clip_rect` share one active clip. The last call wins:
/// `set_clip_rect` replaces the applied clip without touching the stack,
/// and the next push or pop applies the stack's clip again. Code that mixes
/// the two inside one scope should not rely on more than that.
///
/// A render-target bind starts the target with empty clip and translate
/// stacks and the unbind restores the parent's
/// ([`SdiRenderTarget`](crate::backend::SdiRenderTarget));
/// [`stacks::DrawState`](crate::backend::stacks::DrawState) implements
/// that save and restore.
pub trait SdiClipTransform: SdiCore {
    /// Push a clip rectangle (translated, intersected with the current
    /// one) onto the clip stack.
    ///
    /// The default has no stack: it applies the rectangle with
    /// `set_clip_rect`, untranslated (the default translation is always
    /// zero).
    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.set_clip_rect(x, y, w, h)
    }

    /// Pop the most recently pushed clip rectangle.
    ///
    /// The default removes the clip.
    fn pop_clip_rect(&mut self) -> Result<()> {
        self.reset_clip_rect()
    }

    /// Query the top of the clip stack, in surface pixels.
    fn current_clip_rect(&self) -> Option<ClipRect> {
        None
    }

    /// Push a coordinate origin translation.
    fn push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
        let _ = (dx, dy);
        Ok(())
    }

    /// Pop the most recently pushed translation.
    fn pop_translate(&mut self) -> Result<()> {
        Ok(())
    }

    /// Query the current cumulative translation offset.
    fn current_translate(&self) -> (i32, i32) {
        (0, 0)
    }

    /// The rectangle of draw coordinates that lands on the bound surface:
    /// the surface ([`viewport_size`](SdiCore::viewport_size)) moved back
    /// by the current translation.
    ///
    /// A draw outside it is invisible whatever the clip, so the span-based
    /// shape defaults ([`SdiShapes`](crate::backend::SdiShapes),
    /// [`SdiVector`](crate::backend::SdiVector)) generate spans only
    /// inside it: a line or polygon reaching far off-screen costs as much
    /// as its visible part. The clip is deliberately not intersected in:
    /// `set_clip_rect` can replace the stack's clip, so the stack top is
    /// not a safe bound.
    fn draw_bounds(&self) -> ClipRect {
        let (w, h) = self.viewport_size();
        let (tx, ty) = self.current_translate();
        ClipRect::new(0i32.saturating_sub(tx), 0i32.saturating_sub(ty), w, h)
    }

    /// Push a rendering region (translate + clip).
    fn push_region(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.push_translate(x, y)?;
        self.push_clip_rect(0, 0, w, h)
    }

    /// Pop a previously pushed region.
    fn pop_region(&mut self) -> Result<()> {
        self.pop_clip_rect()?;
        self.pop_translate()
    }
}
