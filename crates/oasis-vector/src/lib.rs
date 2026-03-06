//! Vector graphics layer for OASIS_OS.
//!
//! Provides a lightweight scene graph of drawing operations (`VectorOp`) that
//! map directly to `SdiBackend` primitives. Icons and background elements are
//! defined as reusable `VectorOp` sequences parameterized by theme colors.
//!
//! The rasterizer dispatches ops to any `SdiBackend` implementation, making
//! vector content work identically on SDL2, WASM, PSP, and UE5 backends.

pub mod icons;
pub mod op;
pub mod render;
pub mod scene;

pub use icons::IconDef;
pub use op::VectorOp;
pub use render::render_scene;
pub use scene::VectorScene;
