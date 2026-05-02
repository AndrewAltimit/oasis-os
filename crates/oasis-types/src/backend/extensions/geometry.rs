//! `SdiGeometry` extension trait.
//!
//! Extracted from the historical monolithic `extensions.rs`.

use crate::backend::{Color, SdiCore, TextureId};
use crate::error::Result;

// ---------------------------------------------------------------------------
// SdiGeometry
// ---------------------------------------------------------------------------

/// Raw geometry submission for GPU-accelerated rendering.
///
/// Enables arbitrary textured/colored triangles for diagonal gradients,
/// CSS transforms, and custom shapes. Maps to `SDL_RenderGeometry` on
/// SDL3 backends.
pub trait SdiGeometry: SdiCore {
    /// Submit raw triangle geometry to the GPU.
    ///
    /// `vertices` contains position + color + optional UV data.
    /// `indices` indexes into `vertices` to form triangles (3 per tri).
    /// `texture` is an optional texture to sample; `None` uses vertex colors.
    fn render_geometry(
        &mut self,
        _vertices: &[GeometryVertex],
        _indices: &[u32],
        _texture: Option<TextureId>,
    ) -> Result<()> {
        // Default: no-op. Backends without geometry support fall back to
        // fill_rect-based approximations in the caller.
        Ok(())
    }

    /// Query whether this backend supports raw geometry submission.
    fn supports_geometry(&self) -> bool {
        false
    }
}

/// A vertex for [`SdiGeometry::render_geometry`].
#[derive(Debug, Clone, Copy)]
pub struct GeometryVertex {
    /// X position in screen pixels.
    pub x: f32,
    /// Y position in screen pixels.
    pub y: f32,
    /// Texture U coordinate (0.0..1.0). Ignored if no texture.
    pub u: f32,
    /// Texture V coordinate (0.0..1.0). Ignored if no texture.
    pub v: f32,
    /// Vertex color (premultiplied alpha).
    pub color: Color,
}
