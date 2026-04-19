//! Vector graphics layer for OASIS_OS.
//!
//! Provides a lightweight scene graph of drawing operations (`VectorOp`) that
//! map directly to `SdiBackend` primitives. Icons and background elements are
//! defined as reusable `VectorOp` sequences parameterized by theme colors.
//!
//! The rasterizer dispatches ops to any `SdiBackend` implementation, making
//! vector content work identically on SDL2, WASM, PSP, and UE5 backends.

pub mod anim;
pub mod background;
pub mod backgrounds;
pub mod icon_set;
pub mod icons;
pub mod op;
pub mod render;
pub mod scene;

pub use anim::AnimClock;
pub use background::{BackgroundLayer, BackgroundScene};
pub use icon_set::{IconCategory, outline_icon, pixel_icon, solid_icon};
pub use icons::IconDef;
pub use oasis_types::shader::ShaderParams;
pub use op::VectorOp;
pub use render::render_scene;
pub use scene::VectorScene;
