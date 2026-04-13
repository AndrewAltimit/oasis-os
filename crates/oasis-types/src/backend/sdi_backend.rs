//! Full rendering backend: marker super-trait combining all extensions.
//!
//! `SdiBackend` is now a thin super-trait that requires `SdiCore` plus all
//! eight extension traits.  A blanket implementation ensures that any type
//! satisfying those bounds automatically implements `SdiBackend`.
//!
//! # For backend implementors
//!
//! 1. Implement [`SdiCore`] with the 13 required methods
//! 2. Implement each extension trait you want to accelerate (override methods)
//! 3. For extension traits you do not override, add an empty `impl` block
//!    to pick up the default implementations
//! 4. For test mocks: implement `SdiCore` + empty impls for all 8 extension
//!    traits.  The blanket impl gives you `SdiBackend` for free.

use super::{
    SdiAlpha, SdiBatch, SdiClipTransform, SdiCore, SdiGradients, SdiRenderTarget, SdiShapes,
    SdiText, SdiTextures, SdiVector,
};

/// Full rendering backend trait combining all extensions.
///
/// This is a marker super-trait: it has no methods of its own.  All
/// drawing methods live on the individual extension traits (`SdiShapes`,
/// `SdiGradients`, `SdiAlpha`, `SdiText`, `SdiTextures`,
/// `SdiClipTransform`, `SdiVector`, `SdiBatch`).
///
/// A blanket implementation ensures that any type implementing `SdiCore`
/// plus all eight extension traits automatically implements `SdiBackend`.
pub trait SdiBackend:
    SdiCore
    + SdiShapes
    + SdiGradients
    + SdiAlpha
    + SdiText
    + SdiTextures
    + SdiClipTransform
    + SdiVector
    + SdiBatch
    + SdiRenderTarget
{
}

/// Blanket: every type that implements `SdiCore` + all extension traits
/// automatically implements `SdiBackend`.
impl<T> SdiBackend for T where
    T: SdiCore
        + SdiShapes
        + SdiGradients
        + SdiAlpha
        + SdiText
        + SdiTextures
        + SdiClipTransform
        + SdiVector
        + SdiBatch
        + SdiRenderTarget
        + ?Sized
{
}
