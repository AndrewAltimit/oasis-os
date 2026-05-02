//! Backend trait definitions.
//!
//! Every platform implements these traits. The core framework dispatches all
//! I/O through trait boundaries -- it never calls platform-specific APIs.
//!
//! `SdiBackend` is a marker super-trait combining `SdiCore` with eight
//! extension traits (`SdiShapes`, `SdiGradients`, `SdiAlpha`, `SdiText`,
//! `SdiTextures`, `SdiClipTransform`, `SdiVector`, `SdiBatch`).  A blanket
//! impl ensures any type implementing `SdiCore` + all extensions
//! automatically satisfies `SdiBackend`.

mod audio;
mod clipboard;
mod extensions;
mod input;
mod network;
mod sdi_backend;
mod sdi_core;
pub mod stacks;
mod types;

/// Default viewport width (PSP native resolution).
pub const DEFAULT_VIEWPORT_WIDTH: u32 = 480;
/// Default viewport height (PSP native resolution).
pub const DEFAULT_VIEWPORT_HEIGHT: u32 = 272;

// Re-export everything so that `oasis_types::backend::*` continues to work.

// -- types --
pub use types::{
    ArcParams, BITMAP_GLYPH_HEIGHT, BITMAP_GLYPH_WIDTH, BackendErrExt, BlendMode, Color, DashStyle,
    DrawCommand, GradientStyle, RenderTargetId, StrokeStyle, TextMetrics, TextureId, arc_segments,
    backend_require, bitmap_measure_text, cos_approx_f32, sin_approx_f32, texture_not_found,
    validate_rgba_data,
};

// -- core trait --
pub use sdi_core::SdiCore;

// -- extended backend trait --
pub use sdi_backend::SdiBackend;

// -- extension traits --
pub use extensions::{
    BatchRect, BatchText, GeometryVertex, SdiAlpha, SdiBatch, SdiBlendMode, SdiClipTransform,
    SdiGeometry, SdiGradients, SdiRenderTarget, SdiShapes, SdiText, SdiTextures, SdiVector,
};

// -- input --
pub use input::InputBackend;

// -- network --
pub use network::{NetworkBackend, NetworkStream};

// -- audio --
pub use audio::{AudioBackend, AudioTrackId};

// -- clipboard --
pub use clipboard::{ClipboardBackend, InMemoryClipboard};

// -- shared stacks --
pub use stacks::{ClipPush, ClipStack, TranslateStack};

#[cfg(test)]
mod tests;
