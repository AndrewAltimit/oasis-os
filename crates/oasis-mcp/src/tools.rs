//! The application-facing callback boundary.
//!
//! The embedding app implements [`ToolDispatcher`] to expose its capabilities
//! as MCP tools. Nothing in this module references OASIS types, keeping the
//! protocol layer decoupled from the shell.

use serde::Serialize;
use serde_json::Value;

/// A single piece of content returned from a tool call.
///
/// Serializes to the MCP `content` block shape, e.g.
/// `{"type":"text","text":"..."}` or
/// `{"type":"image","data":"<base64>","mimeType":"image/png"}`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    /// Plain-text content.
    #[serde(rename = "text")]
    Text {
        /// The text payload.
        text: String,
    },
    /// Base64-encoded image content.
    #[serde(rename = "image")]
    Image {
        /// Base64-encoded image bytes.
        data: String,
        /// MIME type, e.g. `image/png`.
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

/// The result of a tool invocation.
///
/// Tool *execution* failures are reported in-band (`is_error = true`) rather
/// than as JSON-RPC protocol errors, so the model can see and react to them.
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// One or more content blocks describing the outcome.
    pub content: Vec<ContentBlock>,
    /// Whether the tool call failed (surfaced to the model as `isError`).
    pub is_error: bool,
}

impl ToolResult {
    /// A successful text result.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::Text { text: text.into() }],
            is_error: false,
        }
    }

    /// An error result carrying a human-readable message.
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::Text { text: text.into() }],
            is_error: true,
        }
    }

    /// A successful image result (base64-encoded bytes + MIME type).
    pub fn image(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::Image {
                data: data.into(),
                mime_type: mime_type.into(),
            }],
            is_error: false,
        }
    }
}

/// Metadata describing a tool for `tools/list`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    /// Unique tool name (the value passed to `tools/call`).
    pub name: String,
    /// Human-readable description shown to the model.
    pub description: String,
    /// JSON Schema for the tool's arguments object.
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

impl ToolSpec {
    /// Convenience constructor.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }
}

/// Implemented by the host application to expose tools to connected agents.
pub trait ToolDispatcher {
    /// Return the catalog of available tools.
    fn list_tools(&self) -> Vec<ToolSpec>;

    /// Invoke a tool by name with the given JSON arguments object.
    ///
    /// Implementations must not panic; failures should be returned as
    /// [`ToolResult::error`].
    fn call_tool(&mut self, name: &str, args: Value) -> ToolResult;
}
