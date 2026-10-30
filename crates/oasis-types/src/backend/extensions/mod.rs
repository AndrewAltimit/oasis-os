//! Extension traits: fine-grained capability groupings.
//!
//! Every extension trait has `SdiCore` as its supertrait (`SdiVector`
//! builds on `SdiShapes`) and default implementations written in terms of
//! the supertrait methods, so a type that implements `SdiCore` can opt
//! into any extension with an empty `impl` block --
//! [`impl_sdi_defaults!`](crate::impl_sdi_defaults) writes all eight.
//!
//! `SdiBackend` is the marker super-trait
//! `SdiCore + SdiShapes + SdiGradients + SdiText + SdiTextures
//!  + SdiClipTransform + SdiVector + SdiBatch + SdiRenderTarget`
//! with a blanket impl, so any type satisfying all extension traits
//! automatically implements `SdiBackend`.
//!
//! Each trait lives in its own file under `extensions/`; this `mod.rs` is
//! module wiring plus the re-exports behind `oasis_types::backend::*`.

mod batch;
mod clip_transform;
mod gradients;
mod render_target;
mod shapes;
mod spans;
mod text;
mod textures;
mod vector;

pub use batch::{BatchRect, BatchText, SdiBatch};
pub use clip_transform::SdiClipTransform;
pub use gradients::SdiGradients;
pub use render_target::SdiRenderTarget;
pub use shapes::SdiShapes;
pub use text::SdiText;
pub use textures::SdiTextures;
pub use vector::SdiVector;
