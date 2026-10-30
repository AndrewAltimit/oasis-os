//! Gemini protocol support for the OASIS_OS browser.
//!
//! Gemini is a lightweight protocol with mandatory TLS, single-line
//! requests, and a simple text-based content format (text/gemini).
//! The protocol framing (status codes, response parsing) lives in
//! `oasis-http` and is re-exported here; this module adds the
//! `text/gemini` document parser and the layout-tree renderer.

pub use oasis_http::gemini::{GeminiResponse, GeminiStatus, build_request, parse_response};

pub mod parser;
pub mod renderer;
