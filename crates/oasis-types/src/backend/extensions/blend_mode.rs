//! `SdiBlendMode` extension trait.
//!
//! Extracted from the historical monolithic `extensions.rs`.

use crate::backend::{BlendMode, SdiCore};
use crate::error::Result;

// ---------------------------------------------------------------------------
// SdiBlendMode
// ---------------------------------------------------------------------------

/// Alpha blending mode control for compositing layers.
///
/// This trait is the *low-level* counterpart to
/// [`crate::backend::SdiRenderTarget::composite_render_target`]: it
/// switches the active blend mode for subsequent immediate-mode draws
/// on the currently bound surface.  Most browser code uses the
/// higher-level composite-render-target path; this trait exists for
/// backends that want to expose blend modes outside the compositor
/// (e.g. SDL3's native `SDL_SetRenderDrawBlendMode`).
pub trait SdiBlendMode: SdiCore {
    /// Set the active blend mode for subsequent draw operations.
    fn set_blend_mode(&mut self, _mode: BlendMode) -> Result<()> {
        Ok(())
    }

    /// Query the current blend mode.
    fn current_blend_mode(&self) -> BlendMode {
        BlendMode::Normal
    }
}

// `BlendMode` moved to `super::types` so `DrawCommand` can reference it
// without creating a circular module dependency. Re-exported from
// `backend::mod.rs`.
