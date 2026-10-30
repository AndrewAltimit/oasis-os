//! Full rendering backend: marker super-trait combining all extensions.
//!
//! `SdiBackend` is a thin super-trait that requires `SdiCore` plus all
//! nine extension traits.  A blanket implementation ensures that any type
//! satisfying those bounds automatically implements `SdiBackend`.
//!
//! # For backend implementors
//!
//! 1. Implement [`SdiCore`] with the 13 required methods
//! 2. Implement each extension trait you want to accelerate (override methods)
//! 3. For extension traits you do not override, add an empty `impl` block
//!    to pick up the default implementations, or write
//!    `oasis_types::impl_sdi_defaults!(MyBackend; except SdiShapes, ..)`
//!    for all the ones you do not write yourself
//! 4. For test mocks: implement `SdiCore` and invoke
//!    `oasis_types::impl_sdi_defaults!(MyMock)`.  The blanket impl gives
//!    you `SdiBackend` for free.
//!
//! The shape defaults are span-exact (they use the same generators as the
//! SDL and UE5 rasterizers), so a backend only needs to override them for
//! speed, not for correctness.

use super::{
    SdiAlpha, SdiBatch, SdiClipTransform, SdiCore, SdiGradients, SdiRenderTarget, SdiShapes,
    SdiText, SdiTextures, SdiVector,
};

/// Full rendering backend trait combining all extensions.
///
/// This is a marker super-trait: it has no methods of its own.  All
/// drawing methods live on the individual extension traits (`SdiShapes`,
/// `SdiGradients`, `SdiAlpha`, `SdiText`, `SdiTextures`,
/// `SdiClipTransform`, `SdiVector`, `SdiBatch`, `SdiRenderTarget`).
///
/// A blanket implementation ensures that any type implementing `SdiCore`
/// plus all nine extension traits automatically implements `SdiBackend`.
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
