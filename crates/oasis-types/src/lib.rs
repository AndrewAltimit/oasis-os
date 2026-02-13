//! Foundation types and traits for OASIS_OS.
//!
//! This crate contains the platform-agnostic core types shared by all OASIS_OS
//! crates: colors, input events, backend trait definitions, configuration,
//! error types, and utility modules.

pub mod backend;
pub mod bitmap_font;
pub mod color;
pub mod config;
pub mod error;
pub mod input;
pub mod pbp;
pub mod shadow;
pub mod tls;
