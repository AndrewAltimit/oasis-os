//! HTML tokenizer, parser, and DOM.

pub mod dom;
pub mod entities;
pub(crate) mod hooks;
pub mod tokenizer;
pub mod tree_builder;

#[cfg(feature = "javascript")]
pub use tree_builder::MAX_DOM_DEPTH;
