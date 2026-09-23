//! End-to-end browser sessions.
//!
//! These tests drive a [`BrowserWidget`] exactly the way the shell does
//! (`set_window` → `navigate_vfs` → per-frame `tick` + `paint`, with
//! user input delivered through `handle_input`) against pages served by
//! an in-process HTTP/1.1 server bound to `127.0.0.1` on an ephemeral
//! port. Assertions are made on what a user would observe — the text
//! painted into the content viewport, the URL in the chrome, history —
//! and on what the server received on the wire.
//!
//! No test touches the internet. Every wait is bounded by a deadline so
//! a regression shows up as a failure, not a hung test run.

#![allow(clippy::unwrap_used)]

mod forms;
mod harness;
mod http;
#[cfg(feature = "javascript")]
mod js;
mod navigation;
mod render;
mod robustness;
