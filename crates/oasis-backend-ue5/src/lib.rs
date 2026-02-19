//! UE5 render target backend for OASIS_OS.
//!
//! Implements `SdiBackend` as a software RGBA framebuffer that UE5 reads via
//! `oasis_get_buffer()`. Implements `InputBackend` as an event queue that the
//! FFI layer pushes events into. Implements `AudioBackend` with callback
//! support so the UE5 host can handle actual audio output.

pub mod audio;
mod font;
mod input;
mod renderer;

pub use audio::Ue5AudioBackend;
pub use input::FfiInputBackend;
pub use renderer::Ue5Backend;
