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

// ---------------------------------------------------------------------------
// Diagnostic progress + yield hooks
// ---------------------------------------------------------------------------

type TreeProgressFn = fn(u64, usize, usize);
type TreeYieldFn = fn();
type TreeRawLogFn = fn(&str);
static TREE_PROGRESS_HOOK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static TREE_YIELD_HOOK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static TREE_RAW_LOG_HOOK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Install a raw log hook fired at tree builder entry points to
/// distinguish "function never called" from "function called but
/// progress hook silently failed". Used by PSP backend wired to
/// `vlog_force`.
pub fn set_tree_builder_raw_log_hook(hook: TreeRawLogFn) {
    TREE_RAW_LOG_HOOK.store(hook as usize, std::sync::atomic::Ordering::Relaxed);
}

#[allow(dead_code)] // Only called from per-step bisect builds.
pub(crate) fn tree_raw_log(msg: &str) {
    let raw = TREE_RAW_LOG_HOOK.load(std::sync::atomic::Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    // SAFETY: raw is either 0 or a `TreeRawLogFn` we stored.
    let hook: TreeRawLogFn = unsafe { std::mem::transmute::<usize, TreeRawLogFn>(raw) };
    hook(msg);
}

/// Install a tree-builder progress hook. Called every 256 tokens with
/// `(tokens_processed, total_tokens, dom_nodes)`.
pub fn set_tree_builder_progress_hook(hook: TreeProgressFn) {
    TREE_PROGRESS_HOOK.store(hook as usize, std::sync::atomic::Ordering::Relaxed);
}

/// Install a cooperative yield hook fired every 128 tree builder iters.
pub fn set_tree_builder_yield_hook(hook: TreeYieldFn) {
    TREE_YIELD_HOOK.store(hook as usize, std::sync::atomic::Ordering::Relaxed);
}

fn tree_progress_log(idx: u64, total: usize, nodes: usize) {
    let raw = TREE_PROGRESS_HOOK.load(std::sync::atomic::Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    // SAFETY: raw is either 0 or a `TreeProgressFn` we stored.
    let hook: TreeProgressFn = unsafe { std::mem::transmute::<usize, TreeProgressFn>(raw) };
    hook(idx, total, nodes);
}

fn tree_yield() {
    let raw = TREE_YIELD_HOOK.load(std::sync::atomic::Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    // SAFETY: raw is either 0 or a `TreeYieldFn` we stored.
    let hook: TreeYieldFn = unsafe { std::mem::transmute::<usize, TreeYieldFn>(raw) };
    hook();
}

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
        let total = tokens.len();
        for (idx, token) in tokens.into_iter().enumerate() {
            // Enforce DOM node limit to prevent memory exhaustion.
            if builder.doc.nodes.len() >= MAX_DOM_NODES {
                break;
            }
            if idx.is_multiple_of(256) {
                tree_progress_log(idx as u64, total, builder.doc.nodes.len());
            }
            if idx.is_multiple_of(128) {
                tree_yield();
            }
            builder.process_token(token);
        }
        builder.finish()
    }

    /// Build a DOM tree, reusing a previous document's arena allocation.
    ///
    /// The old document is cleared via [`Document::clear()`] so its
    /// `Vec<Node>` capacity is preserved, avoiding reallocations for
    /// pages of similar size.
    pub fn build_reuse(tokens: Vec<Token>, mut old_doc: Document) -> Document {
        old_doc.clear();
        let mut builder = Self {
            doc: old_doc,
            mode: InsertionMode::Initial,
            open_elements: Vec::new(),
            active_formatting: Vec::new(),
            head_element: None,
            form_element: None,
            frameset_ok: true,
            original_mode: InsertionMode::InBody,
        };
        let total = tokens.len();
        for (idx, token) in tokens.into_iter().enumerate() {
            if builder.doc.nodes.len() >= MAX_DOM_NODES {
                break;
            }
            if idx.is_multiple_of(256) {
                tree_progress_log(idx as u64, total, builder.doc.nodes.len());
            }
            if idx.is_multiple_of(128) {
                tree_yield();
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
