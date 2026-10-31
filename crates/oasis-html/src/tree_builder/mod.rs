//! HTML tree builder.
//!
//! Consumes a token stream and constructs an arena-based DOM tree,
//! following the WHATWG HTML tree construction algorithm (§13.2.6):
//! the insertion modes including the table, caption, column-group and
//! select modes, the active formatting list with markers and the
//! adoption agency, foster parenting, SVG / MathML foreign content with
//! integration points, and `<noscript>` per the scripting flag. Not
//! modelled: quirks mode, framesets, the "in template" mode (template
//! contents parse in "in body"), and element namespaces in the DOM.

mod foreign;
mod formatting;
mod helpers;
#[cfg(test)]
mod hostile_tests;
mod insertion_modes;
#[cfg(test)]
mod spec_tests;
mod tables;
#[cfg(test)]
mod tests;

use super::dom::{Document, NodeId};
use super::tokenizer::{ContentState, Token, Tokenizer};

// ---------------------------------------------------------------------------
// Diagnostic progress + yield hooks
// ---------------------------------------------------------------------------

use super::hooks::Hook;

type TreeProgressFn = fn(u64, usize, usize);
type TreeYieldFn = fn();
type TreeRawLogFn = fn(&str);
static TREE_PROGRESS_HOOK: Hook<TreeProgressFn> = Hook::new();
static TREE_YIELD_HOOK: Hook<TreeYieldFn> = Hook::new();
static TREE_RAW_LOG_HOOK: Hook<TreeRawLogFn> = Hook::new();

/// Install a raw log hook fired at tree builder entry points to
/// distinguish "function never called" from "function called but
/// progress hook silently failed". Used by PSP backend wired to
/// `vlog_force`.
///
/// The first installation wins; later calls are ignored.
pub fn set_tree_builder_raw_log_hook(hook: TreeRawLogFn) {
    TREE_RAW_LOG_HOOK.set(hook);
}

fn tree_raw_log(msg: &str) {
    if let Some(hook) = TREE_RAW_LOG_HOOK.get() {
        hook(msg);
    }
}

/// Install a tree-builder progress hook. Called every 256 tokens with
/// `(tokens_processed, total_tokens, dom_nodes)`.
///
/// The first installation wins; later calls are ignored.
pub fn set_tree_builder_progress_hook(hook: TreeProgressFn) {
    TREE_PROGRESS_HOOK.set(hook);
}

/// Install a cooperative yield hook fired every 128 tree builder iters.
///
/// The first installation wins; later calls are ignored.
pub fn set_tree_builder_yield_hook(hook: TreeYieldFn) {
    TREE_YIELD_HOOK.set(hook);
}

fn tree_progress_log(idx: u64, total: usize, nodes: usize) {
    if let Some(hook) = TREE_PROGRESS_HOOK.get() {
        hook(idx, total, nodes);
    }
}

fn tree_yield() {
    if let Some(hook) = TREE_YIELD_HOOK.get() {
        hook();
    }
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
    InHeadNoscript,
    AfterHead,
    InBody,
    InTable,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InSelect,
    InSelectInTable,
    AfterBody,
    AfterAfterBody,
    Text,
}

/// An element's namespace, with the integration-point kinds the tree
/// construction dispatcher and the scope algorithms care about
/// (WHATWG §13.2.4.2, §13.2.6). The DOM does not store namespaces; the
/// builder keeps them per node while parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Ns {
    #[default]
    Html,
    Svg,
    /// SVG `foreignObject`, `desc`, `title`: HTML integration points.
    SvgHtmlIp,
    Math,
    /// MathML `mi`, `mo`, `mn`, `ms`, `mtext`: text integration points.
    MathTextIp,
    /// MathML `annotation-xml` without an HTML `encoding`.
    MathAnnotationXml,
    /// MathML `annotation-xml encoding="text/html"` (or XHTML): an HTML
    /// integration point.
    MathHtmlIp,
}

impl Ns {
    pub(crate) fn is_foreign(self) -> bool {
        self != Self::Html
    }

    pub(crate) fn is_svg(self) -> bool {
        matches!(self, Self::Svg | Self::SvgHtmlIp)
    }

    pub(crate) fn is_html_integration_point(self) -> bool {
        matches!(self, Self::SvgHtmlIp | Self::MathHtmlIp)
    }

    /// The foreign elements that bound the default, list-item and
    /// button scopes.
    pub(crate) fn bounds_scope(self) -> bool {
        matches!(
            self,
            Self::SvgHtmlIp | Self::MathTextIp | Self::MathAnnotationXml | Self::MathHtmlIp
        )
    }
}

/// Parser options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseOptions {
    /// Whether scripting is enabled. With it on (the default),
    /// `<noscript>` content is raw text; with it off, `<noscript>` is
    /// parsed as ordinary markup, as a browser without JavaScript does.
    pub scripting: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self { scripting: true }
    }
}

// ------------------------------------------------------------------
// TreeBuilder
// ------------------------------------------------------------------

/// Maximum nesting depth for the open elements stack. When the stack
/// reaches this limit, new elements are appended as children of the
/// current node instead of being pushed onto the stack. This prevents
/// stack exhaustion from pathologically deeply nested HTML.
///
/// The cascade, layout-tree build, layout, paint and serialization
/// passes all recurse once per DOM level and are sized for this cap, so
/// DOM mutations from script enforce the matching
/// [`MAX_DOM_DEPTH`] too (`oasis-browser`'s `js_dom::node_api::insert_node`).
pub const MAX_NESTING_DEPTH: usize = 256;

/// Deepest node depth (the document root is depth 0) the parser can
/// produce: `MAX_NESTING_DEPTH` open elements below the root, plus one
/// more level for the children appended once the stack is full. Script
/// insertions that would place a node deeper are refused, and
/// `innerHTML` flattens markup at this depth. The only node that can sit
/// one level below it is the text leaf a `textContent` / `innerHTML`
/// write puts inside an element already at the cap.
pub const MAX_DOM_DEPTH: usize = MAX_NESTING_DEPTH + 1;

/// Maximum number of DOM nodes allowed in a document. Beyond this
/// limit, new elements are silently dropped to prevent memory
/// exhaustion from pathologically large documents.
const MAX_DOM_NODES: usize = 100_000;

/// Attributes the tree builder may copy onto formatting-element clones
/// (adoption agency, reconstruction) per document; see
/// `TreeBuilder::clone_element`.
const MAX_CLONED_ATTRIBUTES: usize = 200_000;

/// Builds a DOM tree from a token stream.
pub struct TreeBuilder {
    pub(crate) doc: Document,
    pub(crate) mode: InsertionMode,
    /// Stack of open element node IDs.
    pub(crate) open_elements: Vec<NodeId>,
    /// Active formatting element node IDs. Markers (WHATWG §13.2.4.3)
    /// are kept out of band in `fmt_markers`.
    pub(crate) active_formatting: Vec<NodeId>,
    /// Positions in `active_formatting` where a marker sits (pushed for
    /// `td`, `th`, `caption`, `template`, `applet`, `object`, `marquee`):
    /// a marker at `n` precedes `active_formatting[n]`. Only entries
    /// after the last marker are ever searched or removed, so the
    /// positions never go stale.
    pub(crate) fmt_markers: Vec<usize>,
    pub(crate) head_element: Option<NodeId>,
    pub(crate) form_element: Option<NodeId>,
    pub(crate) frameset_ok: bool,
    /// Saved mode for returning from `Text` insertion mode.
    pub(crate) original_mode: InsertionMode,
    /// Saved parser state captured when a `<template>` start tag is
    /// processed. The spec puts template contents into a separate
    /// DocumentFragment; the form pointer and insertion mode visible
    /// outside the template do not leak in, and state opened inside
    /// the template does not leak out. We mimic that isolation by
    /// saving and restoring on `<template>` / `</template>` instead
    /// of introducing a real DocumentFragment node kind.
    ///
    /// Each entry stores `(form_element, insertion_mode)` at the
    /// point the template was opened.
    pub(crate) template_form_stack: Vec<(Option<NodeId>, InsertionMode)>,
    /// Namespace of every foreign (SVG / MathML) node, by `NodeId`;
    /// nodes past the end (and every HTML node) are [`Ns::Html`].
    pub(crate) node_ns: Vec<Ns>,
    /// The raw-text / RCDATA element the `Text` insertion mode fills.
    pub(crate) text_element: Option<NodeId>,
    /// Attributes copied onto formatting-element clones so far (see
    /// `TreeBuilder::clone_element`).
    pub(crate) cloned_attributes: usize,
    /// Whether scripting is enabled (see [`ParseOptions::scripting`]).
    pub(crate) scripting: bool,
    /// Insert into the foster parent when the target is a table element
    /// (set while table modes process a token with the in-body rules).
    pub(crate) foster_parenting: bool,
    /// Drop a leading U+000A from the next token (after `<pre>`,
    /// `<listing>`, `<textarea>`).
    pub(crate) ignore_lf: bool,
    /// The tokenizer state the start tag just processed asks for; the
    /// pull parser applies it before reading on.
    pub(crate) pending_content_state: Option<ContentState>,
}

impl TreeBuilder {
    /// Create a new tree builder with an empty document.
    pub fn new() -> Self {
        Self::with_options(ParseOptions::default())
    }

    /// Create a tree builder with the given options.
    pub fn with_options(options: ParseOptions) -> Self {
        Self {
            doc: Document::new(),
            mode: InsertionMode::Initial,
            open_elements: Vec::new(),
            active_formatting: Vec::new(),
            fmt_markers: Vec::new(),
            head_element: None,
            form_element: None,
            frameset_ok: true,
            original_mode: InsertionMode::InBody,
            template_form_stack: Vec::new(),
            node_ns: Vec::new(),
            text_element: None,
            cloned_attributes: 0,
            scripting: options.scripting,
            foster_parenting: false,
            ignore_lf: false,
            pending_content_state: None,
        }
    }

    /// Build a DOM tree from a token stream (scripting on).
    ///
    /// The tokenizer chose raw-text states by tag name alone; prefer
    /// [`TreeBuilder::parse`], which lets the tree builder drive the
    /// tokenizer as the spec does (no raw text inside SVG / MathML,
    /// CDATA sections in foreign content).
    pub fn build(tokens: Vec<Token>) -> Document {
        Self::build_with(tokens, ParseOptions::default())
    }

    /// Build a DOM tree from a token stream with the given options. The
    /// tokens must come from a tokenizer with the same scripting flag
    /// ([`Tokenizer::with_scripting`]).
    pub fn build_with(tokens: Vec<Token>, options: ParseOptions) -> Document {
        tree_raw_log("[BR/TREE] build enter");
        TreeBuilder::with_options(options).run(tokens)
    }

    /// Build a DOM tree, reusing a previous document's arena allocation.
    ///
    /// The old document is cleared via [`Document::clear()`] so its
    /// `Vec<Node>` capacity is preserved, avoiding reallocations for
    /// pages of similar size.
    pub fn build_reuse(tokens: Vec<Token>, mut old_doc: Document) -> Document {
        old_doc.clear();
        tree_raw_log("[BR/TREE] build_reuse enter");
        Self {
            doc: old_doc,
            ..Self::new()
        }
        .run(tokens)
    }

    /// Tokenize and parse `html` (scripting on).
    pub fn parse(html: &str) -> Document {
        Self::parse_with(html, ParseOptions::default())
    }

    /// Tokenize and parse `html` with the given options.
    ///
    /// The tree builder pulls tokens one at a time and sets the
    /// tokenizer's state itself, as WHATWG §13.2 does: raw-text, RCDATA,
    /// script-data and PLAINTEXT states only for HTML elements, and
    /// `<![CDATA[` sections only inside SVG / MathML.
    pub fn parse_with(html: &str, options: ParseOptions) -> Document {
        tree_raw_log("[BR/TREE] parse enter");
        TreeBuilder::with_options(options).run_pull(html)
    }

    /// [`TreeBuilder::parse_with`], reusing a previous document's arena
    /// allocation (see [`TreeBuilder::build_reuse`]).
    pub fn parse_reuse(html: &str, options: ParseOptions, mut old_doc: Document) -> Document {
        old_doc.clear();
        tree_raw_log("[BR/TREE] parse_reuse enter");
        Self {
            doc: old_doc,
            ..Self::with_options(options)
        }
        .run_pull(html)
    }

    /// Feed every token (up to the DOM node cap) and finish the tree.
    fn run(mut self, tokens: Vec<Token>) -> Document {
        let total = tokens.len();
        // About one node per two tokens (end tags make none).
        self.reserve_nodes(total / 2);
        for (idx, token) in tokens.into_iter().enumerate() {
            // Enforce DOM node limit to prevent memory exhaustion.
            if self.doc.nodes.len() >= MAX_DOM_NODES {
                break;
            }
            if idx.is_multiple_of(256) {
                tree_progress_log(idx as u64, total, self.doc.nodes.len());
            }
            if idx.is_multiple_of(128) {
                tree_yield();
            }
            self.process_token(token);
            // The tokenizer already switched states by tag name.
            self.pending_content_state = None;
        }
        tree_raw_log("[BR/TREE] tokens consumed, finishing");
        self.finish()
    }

    /// Pull tokens from a tokenizer this builder drives (see
    /// [`TreeBuilder::parse_with`]). Consecutive character tokens are
    /// merged before processing, as [`Tokenizer::tokenize`] does.
    fn run_pull(mut self, html: &str) -> Document {
        let mut tokenizer = Tokenizer::with_scripting(html, self.scripting);
        tokenizer.auto_content_state = false;
        // A conservative node estimate (typical markup runs 20-60 bytes a
        // node); doubling covers the rest.
        self.reserve_nodes(html.len() / 64);
        let mut lookahead: Option<Token> = None;
        let mut idx: u64 = 0;
        let mut last_pos = usize::MAX;
        let mut stuck = 0u32;
        loop {
            if self.doc.nodes.len() >= MAX_DOM_NODES {
                break;
            }
            idx += 1;
            if idx.is_multiple_of(256) {
                tree_progress_log(idx, html.len(), self.doc.nodes.len());
            }
            if idx.is_multiple_of(128) {
                tree_yield();
            }
            // The same no-progress guard as `Tokenizer::tokenize`.
            if tokenizer.pos == last_pos && lookahead.is_none() {
                stuck += 1;
                if stuck > 100_000 {
                    break;
                }
            } else {
                stuck = 0;
                last_pos = tokenizer.pos;
            }
            tokenizer.allow_cdata = self.adjusted_current_node_is_foreign();
            let mut token = lookahead.take().unwrap_or_else(|| tokenizer.next_token());
            if let Token::Character(ref mut text) = token {
                // Characters never change the tokenizer state, so the
                // next token can be read before they are processed.
                loop {
                    match tokenizer.next_token() {
                        Token::Character(more) => text.push_str(&more),
                        other => {
                            lookahead = Some(other);
                            break;
                        },
                    }
                }
            }
            let is_eof = token == Token::Eof;
            self.process_token(token);
            if let Some(state) = self.pending_content_state.take() {
                tokenizer.switch_content_state(state);
            }
            if is_eof {
                break;
            }
        }
        tree_raw_log("[BR/TREE] tokens consumed, finishing");
        self.finish()
    }

    // =============================================================
    // Token dispatch
    // =============================================================

    /// The tree construction dispatcher (WHATWG §13.2.6): foreign
    /// content rules or the current insertion mode.
    pub(crate) fn process_token(&mut self, token: Token) {
        let token = if std::mem::take(&mut self.ignore_lf) {
            match token {
                Token::Character(s) => match s.strip_prefix('\n') {
                    Some("") => return,
                    Some(rest) => Token::Character(rest.to_string()),
                    None => Token::Character(s),
                },
                other => other,
            }
        } else {
            token
        };
        if self.mode != InsertionMode::Text && self.use_foreign_rules(&token) {
            self.handle_foreign_content(token);
            return;
        }
        self.process_html(token);
    }

    /// Whether `token` goes to the foreign-content rules: the adjusted
    /// current node is SVG / MathML and not an integration point that
    /// takes this token as HTML.
    fn use_foreign_rules(&self, token: &Token) -> bool {
        let Some(&current) = self.open_elements.last() else {
            return false;
        };
        let ns = self.ns_of(current);
        if !ns.is_foreign() {
            return false;
        }
        match token {
            Token::Eof => false,
            Token::Character(_) => !(ns == Ns::MathTextIp || ns.is_html_integration_point()),
            Token::StartTag(tag) => {
                let name = tag.name.as_str();
                if ns == Ns::MathTextIp && name != "mglyph" && name != "malignmark" {
                    return false;
                }
                if matches!(ns, Ns::MathAnnotationXml | Ns::MathHtmlIp) && name == "svg" {
                    return false;
                }
                !ns.is_html_integration_point()
            },
            _ => true,
        }
    }

    /// Process `token` by the current HTML insertion mode.
    pub(crate) fn process_html(&mut self, token: Token) {
        // `</template>` is handled the same in every HTML mode (the
        // spec's "in template" rules); `Text` mode closes its own element.
        if let Token::EndTag(ref t) = token
            && t.name == "template"
            && self.mode != InsertionMode::Text
        {
            self.close_template();
            return;
        }
        match self.mode {
            InsertionMode::Initial => self.handle_initial(token),
            InsertionMode::BeforeHtml => self.handle_before_html(token),
            InsertionMode::BeforeHead => self.handle_before_head(token),
            InsertionMode::InHead => self.handle_in_head(token),
            InsertionMode::InHeadNoscript => self.handle_in_head_noscript(token),
            InsertionMode::AfterHead => self.handle_after_head(token),
            InsertionMode::InBody => self.handle_in_body(token),
            InsertionMode::InTable => self.handle_in_table(token),
            InsertionMode::InCaption => self.handle_in_caption(token),
            InsertionMode::InColumnGroup => self.handle_in_column_group(token),
            InsertionMode::InTableBody => self.handle_in_table_body(token),
            InsertionMode::InRow => self.handle_in_row(token),
            InsertionMode::InCell => self.handle_in_cell(token),
            InsertionMode::InSelect => self.handle_in_select(token),
            InsertionMode::InSelectInTable => self.handle_in_select_in_table(token),
            InsertionMode::AfterBody => self.handle_after_body(token),
            InsertionMode::AfterAfterBody => self.handle_after_after_body(token),
            InsertionMode::Text => self.handle_text(token),
        }
    }

    /// Reserve arena room for about `estimate` nodes (capped at the DOM
    /// node limit): growing the arena by doubling copied every node
    /// several times on large pages.
    fn reserve_nodes(&mut self, estimate: usize) {
        let want = estimate.min(MAX_DOM_NODES);
        self.doc.nodes.reserve(want.saturating_sub(self.doc.nodes.len()));
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

/// `name` in ASCII lowercase, borrowed when it already is (the tokenizer
/// lowercases tag names; only hand-built tokens need a copy).
pub(crate) fn lowercase(name: &str) -> std::borrow::Cow<'_, str> {
    if name.bytes().any(|b| b.is_ascii_uppercase()) {
        std::borrow::Cow::Owned(name.to_ascii_lowercase())
    } else {
        std::borrow::Cow::Borrowed(name)
    }
}

/// Returns `true` if the string consists entirely of ASCII whitespace.
pub(crate) fn is_all_whitespace(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_whitespace())
}
