//! HTML tree builder.
//!
//! Consumes a token stream and constructs an arena-based DOM tree.
//! Implements a simplified subset of the WHATWG HTML parsing algorithm
//! with implicit element insertion, auto-closing, formatting elements,
//! and basic table handling.

mod formatting;
mod helpers;
mod insertion_modes;
#[cfg(test)]
mod tests;

use super::dom::{Document, NodeId};
use super::tokenizer::Token;

// ------------------------------------------------------------------
// Insertion mode
// ------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum InsertionMode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    AfterHead,
    InBody,
    InTable,
    InTableBody,
    InRow,
    InCell,
    AfterBody,
    AfterAfterBody,
    Text,
}

// ------------------------------------------------------------------
// TreeBuilder
// ------------------------------------------------------------------

/// Maximum nesting depth for the open elements stack. When the stack
/// reaches this limit, new elements are appended as children of the
/// current node instead of being pushed onto the stack. This prevents
/// stack exhaustion from pathologically deeply nested HTML.
const MAX_NESTING_DEPTH: usize = 256;

/// Maximum number of DOM nodes allowed in a document. Beyond this
/// limit, new elements are silently dropped to prevent memory
/// exhaustion from pathologically large documents.
const MAX_DOM_NODES: usize = 100_000;

/// Builds a DOM tree from a token stream.
pub struct TreeBuilder {
    pub(crate) doc: Document,
    pub(crate) mode: InsertionMode,
    /// Stack of open element node IDs.
    pub(crate) open_elements: Vec<NodeId>,
    /// Active formatting element node IDs.
    pub(crate) active_formatting: Vec<NodeId>,
    pub(crate) head_element: Option<NodeId>,
    pub(crate) form_element: Option<NodeId>,
    pub(crate) frameset_ok: bool,
    /// Saved mode for returning from `Text` insertion mode.
    pub(crate) original_mode: InsertionMode,
}

impl TreeBuilder {
    /// Create a new tree builder with an empty document.
    pub fn new() -> Self {
        Self {
            doc: Document::new(),
            mode: InsertionMode::Initial,
            open_elements: Vec::new(),
            active_formatting: Vec::new(),
            head_element: None,
            form_element: None,
            frameset_ok: true,
            original_mode: InsertionMode::InBody,
        }
    }

    /// Build a DOM tree from a token stream.
    pub fn build(tokens: Vec<Token>) -> Document {
        let mut builder = TreeBuilder::new();
        for token in tokens {
            // Enforce DOM node limit to prevent memory exhaustion.
            if builder.doc.nodes.len() >= MAX_DOM_NODES {
                break;
            }
            builder.process_token(token);
        }
        builder.finish()
    }

    // =============================================================
    // Token dispatch
    // =============================================================

    pub(crate) fn process_token(&mut self, token: Token) {
        match self.mode {
            InsertionMode::Initial => {
                self.handle_initial(token);
            },
            InsertionMode::BeforeHtml => {
                self.handle_before_html(token);
            },
            InsertionMode::BeforeHead => {
                self.handle_before_head(token);
            },
            InsertionMode::InHead => {
                self.handle_in_head(token);
            },
            InsertionMode::AfterHead => {
                self.handle_after_head(token);
            },
            InsertionMode::InBody => {
                self.handle_in_body(token);
            },
            InsertionMode::InTable => {
                self.handle_in_table(token);
            },
            InsertionMode::InTableBody => {
                self.handle_in_table_body(token);
            },
            InsertionMode::InRow => {
                self.handle_in_row(token);
            },
            InsertionMode::InCell => {
                self.handle_in_cell(token);
            },
            InsertionMode::AfterBody => {
                self.handle_after_body(token);
            },
            InsertionMode::AfterAfterBody => {
                self.handle_after_after_body(token);
            },
            InsertionMode::Text => {
                self.handle_text(token);
            },
        }
    }

    fn finish(self) -> Document {
        self.doc
    }
}

impl Default for TreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ------------------------------------------------------------------
// Utility
// ------------------------------------------------------------------

/// Returns `true` if the string consists entirely of ASCII whitespace.
pub(crate) fn is_all_whitespace(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_whitespace())
}
