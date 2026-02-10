//! Layout engine: box tree construction and geometry calculation.
//!
//! Implements CSS 2.1 visual formatting model with block formatting
//! contexts, inline formatting contexts, line breaking, and text
//! measurement. Operates at the PSP-native virtual resolution of
//! 480x272.

pub mod block;
pub mod box_model;
pub mod float;
pub mod inline;
pub mod table;
pub mod text;
