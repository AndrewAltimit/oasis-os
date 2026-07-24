//! Minimal Model Context Protocol (MCP) server for OASIS_OS.
//!
//! This crate is a transport- and protocol-only layer: it speaks the
//! [Streamable HTTP](https://modelcontextprotocol.io) transport over a
//! non-blocking [`NetworkStream`](oasis_types::backend::NetworkStream) and
//! dispatches JSON-RPC 2.0 / MCP messages. It knows nothing about the OASIS
//! UI — the embedding application supplies a [`ToolDispatcher`] whose
//! `call_tool` implementation actually drives the shell.
//!
//! # Design
//! The whole server is driven by a single [`McpServer::poll`] call per frame
//! from the host's main loop (mirroring `RemoteListener`/`FtpServer`), so it
//! needs no async runtime and no background threads. All tool calls run
//! inline on the caller's thread, so the dispatcher may hold `&mut` borrows of
//! non-`Send` application state.
//!
//! The minimal Streamable-HTTP subset implemented here: `POST /mcp` with a
//! JSON-RPC request returns a single `application/json` response; a
//! notification returns `202 Accepted`; `GET /mcp` returns `405` (no
//! server-initiated SSE stream); `DELETE`/`OPTIONS` are handled trivially.
//! No session IDs are issued (single local client).

mod base64;
mod dispatch;
mod http;
mod server;
mod tools;

pub use base64::encode as base64_encode;
pub use dispatch::{PROTOCOL_VERSION, SERVER_NAME, SERVER_VERSION};
pub use server::McpServer;
pub use tools::{ContentBlock, ToolDispatcher, ToolResult, ToolSpec};
