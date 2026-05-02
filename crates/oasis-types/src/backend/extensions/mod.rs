//! Extension traits: fine-grained capability groupings.
//!
//! Each extension trait mirrors a subset of the old monolithic `SdiBackend`
//! methods, with `SdiCore` as the only supertrait.  Default implementations
//! use `SdiCore` primitives only, so a type that implements `SdiCore` can
//! opt into any extension trait with an empty `impl` block.
//!
//! `SdiBackend` is now a marker super-trait defined as
//! `SdiCore + SdiShapes + SdiGradients + SdiAlpha + SdiText + SdiTextures
//!  + SdiClipTransform + SdiVector + SdiBatch + SdiRenderTarget`
//! with a blanket impl, so any type satisfying all extension traits
//! automatically implements `SdiBackend`.
//!
//! Each trait lives in its own file under `extensions/`; this `mod.rs`
//! is just module wiring + re-exports so the historical
//! `oasis_types::backend::*` import surface keeps working.

mod alpha;
mod batch;
mod blend_mode;
mod clip_transform;
mod geometry;
mod gradients;
mod render_target;
mod shapes;
mod text;
mod textures;
mod vector;

pub use alpha::SdiAlpha;
pub use batch::{BatchRect, BatchText, SdiBatch};
pub use blend_mode::SdiBlendMode;
pub use clip_transform::SdiClipTransform;
pub use geometry::{GeometryVertex, SdiGeometry};
pub use gradients::SdiGradients;
pub use render_target::SdiRenderTarget;
pub use shapes::SdiShapes;
pub use text::SdiText;
pub use textures::SdiTextures;
pub use vector::SdiVector;
