//! `SdiRenderTarget` extension trait.
//!
//! Extracted from the historical monolithic `extensions.rs`.

use crate::backend::{BlendMode, RenderTargetId, SdiCore};
use crate::error::{OasisError, Result};

// ---------------------------------------------------------------------------
// SdiRenderTarget
// ---------------------------------------------------------------------------

/// Offscreen render target operations for compositing and tile caching.
///
/// This is the trait surface the browser compositor uses to implement
/// `mix-blend-mode`, `background-blend-mode`, `backdrop-filter`,
/// `mask-*`, `isolation: isolate`, and box-level `filter`.  All of those
/// properties need the same primitive: render-to-texture +
/// composite-back.
///
/// All `Result`-returning methods default to `Err(OasisError::Backend("...not
/// supported"))` except [`destroy_render_target`](Self::destroy_render_target)
/// which defaults to `Ok(())` for opt-out backends.  Capability probes
/// ([`supports_render_targets`](Self::supports_render_targets),
/// [`supports_render_target_readback`](Self::supports_render_target_readback))
/// return `bool` and default to `false`.  The browser checks support
/// before use and falls back to a no-op (drawing without the effect)
/// when unsupported.
///
/// # Bind stack
///
/// [`bind_render_target`](Self::bind_render_target) is *nestable*.
/// Backends maintain their own stack so a `mix-blend-mode` child of a
/// `backdrop-filter` parent composes correctly.  Each
/// `bind_render_target` must be paired with exactly one
/// [`unbind_render_target`](Self::unbind_render_target).
///
/// # Readback
///
/// [`read_render_target`](Self::read_render_target) is a separate
/// capability gated by
/// [`supports_render_target_readback`](Self::supports_render_target_readback).
/// It is required for `backdrop-filter` (sample the parent surface
/// before drawing the layer on top).  Backends that cannot afford a
/// per-frame readback (PSP) report `false` and the browser drops
/// `backdrop-filter` to a static-tint shim.
pub trait SdiRenderTarget: SdiCore {
    /// Allocate an offscreen RGBA8 surface of the given size.
    ///
    /// Returns a [`RenderTargetId`] that can be bound for drawing,
    /// composited back, read back, and finally destroyed.  Backends
    /// that cannot satisfy the request (e.g. PSP out of VRAM) return
    /// `Err`.
    fn create_render_target(&mut self, _w: u32, _h: u32) -> Result<RenderTargetId> {
        Err(OasisError::Backend("render targets not supported".into()))
    }

    /// Redirect all subsequent draw calls into the given render target.
    ///
    /// Backends save the current draw state (clip rect, translation,
    /// active target) onto an internal stack and clear the clip on the
    /// new target.  Calls are nestable.
    fn bind_render_target(&mut self, _id: RenderTargetId) -> Result<()> {
        Err(OasisError::Backend("render targets not supported".into()))
    }

    /// Pop the most recent [`bind_render_target`](Self::bind_render_target).
    ///
    /// Restores the draw state that was active when the corresponding
    /// `bind_render_target` was called.  After the outermost pop,
    /// drawing returns to the framebuffer.
    fn unbind_render_target(&mut self) -> Result<()> {
        Err(OasisError::Backend("render targets not supported".into()))
    }

    /// Composite a render target into the *currently bound* surface
    /// (framebuffer or another render target).
    ///
    /// `dst_x`/`dst_y`/`dst_w`/`dst_h` give the destination rectangle.
    /// `blend` selects one of the 16 CSS-aligned blend modes.
    /// `opacity` is in `[0.0, 1.0]` and multiplies the source alpha.
    #[allow(clippy::too_many_arguments)]
    fn composite_render_target(
        &mut self,
        _id: RenderTargetId,
        _dst_x: i32,
        _dst_y: i32,
        _dst_w: u32,
        _dst_h: u32,
        _blend: BlendMode,
        _opacity: f32,
    ) -> Result<()> {
        debug_assert!(
            (0.0..=1.0).contains(&_opacity),
            "opacity must be in [0.0, 1.0], got {_opacity}"
        );
        Err(OasisError::Backend("render targets not supported".into()))
    }

    /// Composite a render target whose pixels hold *premultiplied*
    /// alpha into the currently bound surface with source-over
    /// blending (`dst = src + dst * (1 - srcA)`).
    ///
    /// Drawing straight-alpha primitives onto a transparent-cleared
    /// target naturally leaves premultiplied pixels (`rgb * a` over
    /// zero), so replaying such a texture through this operator
    /// reproduces the original immediate-mode draws.  Compositing the
    /// same texture with [`composite_render_target`](Self::composite_render_target)
    /// and [`BlendMode::Normal`] would multiply by alpha a *second*
    /// time and visibly darken semi-transparent content.
    ///
    /// Used by the static vector-layer bake cache in `oasis-core`.
    /// Defaults to `Err`; callers must treat failure as "baking
    /// unsupported" and fall back to immediate-mode drawing.
    fn composite_render_target_premultiplied(
        &mut self,
        _id: RenderTargetId,
        _dst_x: i32,
        _dst_y: i32,
        _dst_w: u32,
        _dst_h: u32,
    ) -> Result<()> {
        Err(OasisError::Backend(
            "premultiplied render-target composite not supported".into(),
        ))
    }

    /// Read RGBA8 pixels back from a render target into a
    /// caller-supplied buffer.
    ///
    /// Required for `backdrop-filter`: the browser samples the parent
    /// surface, runs the filter chain on CPU, and draws the filtered
    /// backdrop into the layer before painting the contained items on
    /// top.  Backends that cannot afford a per-frame readback report
    /// `false` from
    /// [`supports_render_target_readback`](Self::supports_render_target_readback)
    /// and the browser falls back to a static-tint shim.
    ///
    /// `dst.len()` must equal the render target's width * height * 4
    /// (the dimensions passed to [`create_render_target`](Self::create_render_target)).
    fn read_render_target(&mut self, _id: RenderTargetId, _dst: &mut [u8]) -> Result<()> {
        Err(OasisError::Backend(
            "render-target readback not supported".into(),
        ))
    }

    /// Release a render target previously created with
    /// [`create_render_target`](Self::create_render_target).
    ///
    /// Backends that opt in should override this to release resources.
    /// The default no-op is safe for backends that never create render
    /// targets in the first place.
    fn destroy_render_target(&mut self, _id: RenderTargetId) -> Result<()> {
        Ok(())
    }

    /// Query whether this backend supports offscreen render targets.
    ///
    /// The browser compositor probes this once at startup and disables
    /// the slow path entirely on backends that return `false` —
    /// `mix-blend-mode`, `mask-*`, etc. degrade to "draw without the
    /// effect" so the page still renders.
    fn supports_render_targets(&self) -> bool {
        false
    }

    /// Query whether this backend can read pixels back from a render
    /// target.  Distinct from
    /// [`supports_render_targets`](Self::supports_render_targets)
    /// because PSP can render offscreen but not afford a per-frame
    /// readback.
    fn supports_render_target_readback(&self) -> bool {
        false
    }
}
