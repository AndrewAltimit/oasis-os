//! HTML tokenizer, tree builder, and arena DOM for the OASIS_OS browser.
//!
//! The leaf of the browser engine: a WHATWG-style tokenizer
//! ([`tokenizer::Tokenizer`]), a tree builder with the insertion-mode
//! state machine ([`tree_builder::TreeBuilder`]), and the arena-based
//! [`dom::Document`] that the cascade, layout and JS bindings share
//! (see `docs/adr/001-arena-based-dom.md`).
//!
//! `oasis-browser` re-exports this crate as its internal `html` module;
//! embedders normally reach it through `oasis_browser::internals`. The
//! API is shaped for the browser engine and may change without notice.

pub mod dom;
pub mod entities;
pub mod escape;
pub mod hooks;
pub mod tokenizer;
pub mod tree_builder;

pub use tree_builder::MAX_DOM_DEPTH;
