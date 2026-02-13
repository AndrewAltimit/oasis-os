//! Bitmap font re-export from the shared `oasis-types::bitmap_font` module.
//!
//! All glyph data and lookup lives in the shared crate to eliminate
//! duplication across backends. This module re-exports for backward
//! compatibility.

pub use oasis_types::bitmap_font::*;
