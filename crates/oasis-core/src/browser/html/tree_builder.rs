//! HTML tree builder.
//!
//! Consumes a token stream and constructs an arena-based DOM tree.
//! Implements a simplified subset of the WHATWG HTML parsing algorithm
//! with implicit element insertion, auto-closing, formatting elements,
//! and basic table handling.

use super::dom::{Attribute as DomAttribute, Document, ElementData, NodeId, NodeKind, TagName};
use super::tokenizer::{StartTagToken, Token};

// ------------------------------------------------------------------
// Insertion mode
// ------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum InsertionMode {
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

/// Builds a DOM tree from a token stream.
pub struct TreeBuilder {
    doc: Document,
    mode: InsertionMode,
    /// Stack of open element node IDs.
    open_elements: Vec<NodeId>,
    /// Active formatting element node IDs.
    active_formatting: Vec<NodeId>,
    head_element: Option<NodeId>,
    form_element: Option<NodeId>,
    frameset_ok: bool,
    /// Saved mode for returning from `Text` insertion mode.
    original_mode: InsertionMode,
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
            builder.process_token(token);
        }
        builder.finish()
    }

    // =============================================================
    // Token dispatch
    // =============================================================

    fn process_token(&mut self, token: Token) {
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

    // =============================================================
    // Insertion-mode handlers
    // =============================================================

    fn handle_initial(&mut self, token: Token) {
        match token {
            Token::Doctype(_) => {
                // Ignore doctype for DOM purposes; switch mode.
                self.mode = InsertionMode::BeforeHtml;
            },
            Token::Comment(text) => {
                let id = self.doc.add_node(NodeKind::Comment(text));
                let root = self.doc.root;
                self.doc.append_child(root, id);
            },
            Token::Character(ref s) if is_all_whitespace(s) => {
                // Ignore leading whitespace.
            },
            _ => {
                // Anything else: switch to BeforeHtml and reprocess.
                self.mode = InsertionMode::BeforeHtml;
                self.process_token(token);
            },
        }
    }

    fn handle_before_html(&mut self, token: Token) {
        match &token {
            Token::StartTag(tag) if tag.name == "html" => {
                let id = self.create_element_from_start_tag(tag);
                let root = self.doc.root;
                self.doc.append_child(root, id);
                self.open_elements.push(id);
                self.mode = InsertionMode::BeforeHead;
            },
            Token::Character(s) if is_all_whitespace(s) => {
                // Ignore.
            },
            _ => {
                // Implicitly create <html>.
                let id = self.create_element(TagName::Html);
                let root = self.doc.root;
                self.doc.append_child(root, id);
                self.open_elements.push(id);
                self.mode = InsertionMode::BeforeHead;
                self.process_token(token);
            },
        }
    }

    fn handle_before_head(&mut self, token: Token) {
        match &token {
            Token::StartTag(tag) if tag.name == "head" => {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.head_element = Some(id);
                self.mode = InsertionMode::InHead;
            },
            Token::StartTag(tag) if tag.name == "body" => {
                // Implicitly create <head>, then reprocess.
                let head = self.create_element(TagName::Head);
                self.insert_element(head);
                self.head_element = Some(head);
                self.pop_open_element(); // pop <head>
                self.mode = InsertionMode::AfterHead;
                self.process_token(token);
            },
            Token::Character(s) if is_all_whitespace(s) => {
                // Ignore.
            },
            _ => {
                // Implicitly create <head>.
                let head = self.create_element(TagName::Head);
                self.insert_element(head);
                self.head_element = Some(head);
                self.pop_open_element(); // pop <head>
                self.mode = InsertionMode::AfterHead;
                self.process_token(token);
            },
        }
    }

    fn handle_in_head(&mut self, token: Token) {
        match &token {
            Token::Character(s) if is_all_whitespace(s) => {
                self.insert_text(s);
            },
            Token::Comment(text) => {
                let id = self.doc.add_node(NodeKind::Comment(text.clone()));
                let parent = self.current_node();
                self.doc.append_child(parent, id);
            },
            Token::StartTag(tag) if tag.name == "title" => {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.original_mode = self.mode;
                self.mode = InsertionMode::Text;
            },
            Token::StartTag(tag)
                if tag.name == "style" || tag.name == "script" || tag.name == "noscript" =>
            {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.original_mode = self.mode;
                self.mode = InsertionMode::Text;
            },
            Token::StartTag(tag) if tag.name == "meta" || tag.name == "link" => {
                let id = self.create_element_from_start_tag(tag);
                let parent = self.current_node();
                self.doc.append_child(parent, id);
                // Void elements: do not push onto open stack.
            },
            Token::EndTag(tag) if tag.name == "head" => {
                self.pop_open_element();
                self.mode = InsertionMode::AfterHead;
            },
            Token::StartTag(tag) if tag.name == "body" => {
                // Implicitly close <head>.
                self.pop_open_element();
                self.mode = InsertionMode::AfterHead;
                self.process_token(token);
            },
            _ => {
                // Implicitly close <head> and reprocess.
                self.pop_open_element();
                self.mode = InsertionMode::AfterHead;
                self.process_token(token);
            },
        }
    }

    fn handle_after_head(&mut self, token: Token) {
        match &token {
            Token::StartTag(tag) if tag.name == "body" => {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.frameset_ok = false;
                self.mode = InsertionMode::InBody;
            },
            Token::Character(s) if is_all_whitespace(s) => {
                // Ignore whitespace between head and body.
            },
            _ => {
                // Implicitly create <body>.
                let body = self.create_element(TagName::Body);
                self.insert_element(body);
                self.mode = InsertionMode::InBody;
                self.process_token(token);
            },
        }
    }

    fn handle_in_body(&mut self, token: Token) {
        match token {
            Token::Character(ref s) => {
                self.reconstruct_formatting();
                self.insert_text(s);
            },
            Token::Comment(text) => {
                let id = self.doc.add_node(NodeKind::Comment(text));
                let parent = self.current_node();
                self.doc.append_child(parent, id);
            },
            Token::StartTag(ref tag) => {
                let tag_name = TagName::from_str(&tag.name.to_ascii_lowercase());
                self.handle_start_tag_in_body(&tag_name, tag);
            },
            Token::EndTag(ref tag) => {
                let tag_name = TagName::from_str(&tag.name.to_ascii_lowercase());
                self.handle_end_tag_in_body(&tag_name);
            },
            Token::Eof => {
                // Implicitly close everything.
            },
            Token::Doctype(_) => {
                // Ignore in body.
            },
        }
    }

    /// Process a start tag while in InBody mode.
    fn handle_start_tag_in_body(&mut self, tag_name: &TagName, tag: &StartTagToken) {
        match tag_name {
            TagName::Html | TagName::Head | TagName::Body => {
                // Ignore duplicates.
            },
            TagName::Table => {
                self.close_p_if_in_scope();
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.frameset_ok = false;
                self.mode = InsertionMode::InTable;
            },
            TagName::P => {
                self.close_p_if_in_scope();
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
            },
            TagName::H1 | TagName::H2 | TagName::H3 | TagName::H4 | TagName::H5 | TagName::H6 => {
                self.close_p_if_in_scope();
                if self.current_node_is_heading() {
                    self.pop_open_element();
                }
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
            },
            TagName::Li => {
                self.close_li_if_in_scope();
                self.close_p_if_in_scope();
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
            },
            TagName::Dt | TagName::Dd => {
                self.close_dt_dd_if_in_scope();
                self.close_p_if_in_scope();
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
            },
            TagName::Pre | TagName::Blockquote => {
                self.close_p_if_in_scope();
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
            },
            TagName::Ul | TagName::Ol | TagName::Dl => {
                self.close_p_if_in_scope();
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
            },
            TagName::Div
            | TagName::Nav
            | TagName::Header
            | TagName::Footer
            | TagName::Main
            | TagName::Section
            | TagName::Article
            | TagName::Aside
            | TagName::Figure
            | TagName::Figcaption
            | TagName::Details
            | TagName::Summary => {
                self.close_p_if_in_scope();
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
            },
            TagName::Form => {
                if self.form_element.is_some() {
                    return; // Ignore nested forms.
                }
                self.close_p_if_in_scope();
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.form_element = Some(id);
            },
            TagName::Hr => {
                self.close_p_if_in_scope();
                let id = self.create_element_from_start_tag(tag);
                let parent = self.current_node();
                self.doc.append_child(parent, id);
                // Void: do not push.
            },
            TagName::Br | TagName::Img | TagName::Input | TagName::Source | TagName::Col => {
                self.reconstruct_formatting();
                let id = self.create_element_from_start_tag(tag);
                let parent = self.current_node();
                self.doc.append_child(parent, id);
                // Void: do not push.
            },
            TagName::Meta | TagName::Link => {
                let id = self.create_element_from_start_tag(tag);
                let parent = self.current_node();
                self.doc.append_child(parent, id);
            },
            TagName::Script | TagName::Style => {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.original_mode = self.mode;
                self.mode = InsertionMode::Text;
            },
            TagName::A => {
                // Close existing <a> in active formatting first.
                self.close_formatting_a_if_active();
                self.reconstruct_formatting();
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.active_formatting.push(id);
            },
            _ if tag_name.is_formatting() => {
                self.reconstruct_formatting();
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.active_formatting.push(id);
            },
            _ => {
                // Generic start tag.
                self.reconstruct_formatting();
                let id = self.create_element_from_start_tag(tag);
                if tag_name.is_void() || tag.self_closing {
                    let parent = self.current_node();
                    self.doc.append_child(parent, id);
                } else {
                    self.insert_element(id);
                }
            },
        }
    }

    /// Process an end tag while in InBody mode.
    fn handle_end_tag_in_body(&mut self, tag_name: &TagName) {
        match tag_name {
            TagName::Body => {
                self.mode = InsertionMode::AfterBody;
            },
            TagName::Html => {
                self.mode = InsertionMode::AfterBody;
                // Reprocess </html> in AfterBody.
                self.process_token(Token::EndTag(super::tokenizer::EndTagToken {
                    name: "html".to_string(),
                }));
            },
            TagName::P => {
                if !self.has_in_scope(&TagName::P) {
                    let p = self.create_element(TagName::P);
                    self.insert_element(p);
                }
                self.close_to_tag(&TagName::P);
            },
            TagName::H1 | TagName::H2 | TagName::H3 | TagName::H4 | TagName::H5 | TagName::H6 => {
                if self.has_heading_in_scope() {
                    self.close_to_first_heading();
                }
            },
            TagName::Li => {
                if self.has_in_list_scope(&TagName::Li) {
                    self.close_to_tag(&TagName::Li);
                }
            },
            TagName::Dt | TagName::Dd => {
                if self.has_in_scope(tag_name) {
                    self.close_to_tag(tag_name);
                }
            },
            TagName::Form => {
                self.form_element = None;
                if self.has_in_scope(&TagName::Form) {
                    self.close_to_tag(&TagName::Form);
                }
            },
            TagName::Table => {
                // Misplaced end tag; ignore in body mode.
            },
            _ if tag_name.is_formatting() => {
                self.close_formatting_element(tag_name);
            },
            _ => {
                self.close_to_tag_any_scope(tag_name);
            },
        }
    }

    fn handle_in_table(&mut self, token: Token) {
        match &token {
            Token::StartTag(tag) if tag.name == "caption" => {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
            },
            Token::StartTag(tag) if tag.name == "colgroup" => {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
            },
            Token::StartTag(tag) if tag.name == "col" => {
                // Implicit <colgroup>.
                let cg = self.create_element(TagName::Colgroup);
                self.insert_element(cg);
                let id = self.create_element_from_start_tag(tag);
                let parent = self.current_node();
                self.doc.append_child(parent, id);
                // Void: don't push col.
            },
            Token::StartTag(tag)
                if tag.name == "tbody" || tag.name == "thead" || tag.name == "tfoot" =>
            {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.mode = InsertionMode::InTableBody;
            },
            Token::StartTag(tag) if tag.name == "tr" => {
                // Implicit <tbody>.
                let tbody = self.create_element(TagName::Tbody);
                self.insert_element(tbody);
                self.mode = InsertionMode::InTableBody;
                self.process_token(token);
            },
            Token::StartTag(tag) if tag.name == "td" || tag.name == "th" => {
                // Implicit <tbody> + <tr>.
                let tbody = self.create_element(TagName::Tbody);
                self.insert_element(tbody);
                let tr = self.create_element(TagName::Tr);
                self.insert_element(tr);
                self.mode = InsertionMode::InRow;
                self.process_token(token);
            },
            Token::EndTag(tag) if tag.name == "table" => {
                self.close_to_tag(&TagName::Table);
                self.reset_mode_after_table();
            },
            Token::EndTag(tag)
                if matches!(
                    tag.name.as_str(),
                    "body"
                        | "html"
                        | "caption"
                        | "col"
                        | "colgroup"
                        | "tbody"
                        | "tfoot"
                        | "thead"
                        | "tr"
                        | "td"
                        | "th"
                ) =>
            {
                // Ignore these end tags in InTable mode.
            },
            _ => {
                // Foster parenting.
                self.foster_parent(token);
            },
        }
    }

    fn handle_in_table_body(&mut self, token: Token) {
        match &token {
            Token::StartTag(tag) if tag.name == "tr" => {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.mode = InsertionMode::InRow;
            },
            Token::StartTag(tag) if tag.name == "td" || tag.name == "th" => {
                // Implicit <tr>.
                let tr = self.create_element(TagName::Tr);
                self.insert_element(tr);
                self.mode = InsertionMode::InRow;
                self.process_token(token);
            },
            Token::EndTag(tag)
                if tag.name == "tbody" || tag.name == "thead" || tag.name == "tfoot" =>
            {
                let tn = TagName::from_str(&tag.name.to_ascii_lowercase());
                if self.has_in_table_scope(&tn) {
                    self.close_to_tag(&tn);
                    self.mode = InsertionMode::InTable;
                }
            },
            Token::EndTag(tag) if tag.name == "table" => {
                // Close the current table body section, then
                // reprocess in InTable.
                self.close_current_table_body();
                self.mode = InsertionMode::InTable;
                self.process_token(token);
            },
            _ => {
                self.handle_in_table(token);
            },
        }
    }

    fn handle_in_row(&mut self, token: Token) {
        match &token {
            Token::StartTag(tag) if tag.name == "td" || tag.name == "th" => {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.mode = InsertionMode::InCell;
            },
            Token::EndTag(tag) if tag.name == "tr" => {
                if self.has_in_table_scope(&TagName::Tr) {
                    self.close_to_tag(&TagName::Tr);
                    self.mode = InsertionMode::InTableBody;
                }
            },
            Token::StartTag(tag) if tag.name == "tr" => {
                // Close current row, open new one.
                if self.has_in_table_scope(&TagName::Tr) {
                    self.close_to_tag(&TagName::Tr);
                }
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.mode = InsertionMode::InRow;
            },
            Token::EndTag(tag) if tag.name == "table" => {
                if self.has_in_table_scope(&TagName::Tr) {
                    self.close_to_tag(&TagName::Tr);
                }
                self.mode = InsertionMode::InTableBody;
                self.process_token(token);
            },
            Token::EndTag(tag)
                if tag.name == "tbody" || tag.name == "thead" || tag.name == "tfoot" =>
            {
                if self.has_in_table_scope(&TagName::Tr) {
                    self.close_to_tag(&TagName::Tr);
                }
                self.mode = InsertionMode::InTableBody;
                self.process_token(token);
            },
            _ => {
                self.handle_in_table(token);
            },
        }
    }

    fn handle_in_cell(&mut self, token: Token) {
        match &token {
            Token::EndTag(tag) if tag.name == "td" || tag.name == "th" => {
                let tn = TagName::from_str(&tag.name.to_ascii_lowercase());
                if self.has_in_table_scope(&tn) {
                    self.close_to_tag(&tn);
                    self.active_formatting.clear();
                    self.mode = InsertionMode::InRow;
                }
            },
            Token::StartTag(tag) if tag.name == "td" || tag.name == "th" || tag.name == "tr" => {
                // Close the current cell, reprocess.
                self.close_current_cell();
                self.mode = InsertionMode::InRow;
                self.process_token(token);
            },
            Token::EndTag(tag)
                if tag.name == "table"
                    || tag.name == "tbody"
                    || tag.name == "thead"
                    || tag.name == "tfoot"
                    || tag.name == "tr" =>
            {
                self.close_current_cell();
                self.mode = InsertionMode::InRow;
                self.process_token(token);
            },
            _ => {
                // Process as InBody.
                self.handle_in_body(token);
            },
        }
    }

    fn handle_after_body(&mut self, token: Token) {
        match &token {
            Token::EndTag(tag) if tag.name == "html" => {
                self.mode = InsertionMode::AfterAfterBody;
            },
            Token::Character(s) if is_all_whitespace(s) => {
                // Ignore trailing whitespace.
            },
            Token::Comment(text) => {
                let id = self.doc.add_node(NodeKind::Comment(text.clone()));
                let root = self.doc.root;
                self.doc.append_child(root, id);
            },
            Token::Eof => {},
            _ => {
                // Reprocess in InBody.
                self.mode = InsertionMode::InBody;
                self.process_token(token);
            },
        }
    }

    fn handle_after_after_body(&mut self, token: Token) {
        match &token {
            Token::Comment(text) => {
                let id = self.doc.add_node(NodeKind::Comment(text.clone()));
                let root = self.doc.root;
                self.doc.append_child(root, id);
            },
            Token::Character(s) if is_all_whitespace(s) => {},
            Token::Eof => {},
            _ => {
                // Reprocess in InBody.
                self.mode = InsertionMode::InBody;
                self.process_token(token);
            },
        }
    }

    fn handle_text(&mut self, token: Token) {
        match token {
            Token::Character(ref s) => {
                self.insert_text(s);
            },
            Token::EndTag(_) => {
                self.pop_open_element();
                self.mode = self.original_mode;
            },
            Token::Eof => {
                self.pop_open_element();
                self.mode = self.original_mode;
                self.process_token(Token::Eof);
            },
            _ => {},
        }
    }

    // =============================================================
    // Element creation helpers
    // =============================================================

    /// Create a DOM element node from a start tag token.
    fn create_element_from_start_tag(&mut self, tag: &StartTagToken) -> NodeId {
        let tag_name = TagName::from_str(&tag.name.to_ascii_lowercase());
        let mut data = ElementData::new(tag_name);
        for attr in &tag.attributes {
            data.attributes.push(DomAttribute {
                name: attr.name.clone(),
                value: attr.value.clone(),
            });
        }
        self.doc.add_node(NodeKind::Element(data))
    }

    /// Create a bare element node with the given tag name and no
    /// attributes.
    fn create_element(&mut self, tag: TagName) -> NodeId {
        self.doc.add_node(NodeKind::Element(ElementData::new(tag)))
    }

    /// Insert an element as the last child of the current node and
    /// push it onto the open elements stack.
    fn insert_element(&mut self, id: NodeId) {
        let parent = self.current_node();
        self.doc.append_child(parent, id);
        self.open_elements.push(id);
    }

    /// Insert text, coalescing into an existing trailing text node
    /// when possible.
    fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let parent = self.current_node();
        let children = &self.doc.nodes[parent].children;
        if let Some(&last_child) = children.last()
            && let NodeKind::Text(ref mut existing) = self.doc.nodes[last_child].kind
        {
            existing.push_str(text);
            return;
        }
        let id = self.doc.add_node(NodeKind::Text(text.to_string()));
        self.doc.append_child(parent, id);
    }

    // =============================================================
    // Stack helpers
    // =============================================================

    /// The node ID at the top of the open elements stack, or the
    /// document root if the stack is empty.
    fn current_node(&self) -> NodeId {
        self.open_elements.last().copied().unwrap_or(self.doc.root)
    }

    /// Pop the top of the open elements stack.
    fn pop_open_element(&mut self) -> Option<NodeId> {
        self.open_elements.pop()
    }

    /// Get the tag name of the element at `node_id`.
    fn tag_of(&self, node_id: NodeId) -> Option<&TagName> {
        self.doc.element(node_id).map(|e| &e.tag)
    }

    fn current_node_is_heading(&self) -> bool {
        self.tag_of(self.current_node())
            .map(|t| {
                matches!(
                    t,
                    TagName::H1
                        | TagName::H2
                        | TagName::H3
                        | TagName::H4
                        | TagName::H5
                        | TagName::H6
                )
            })
            .unwrap_or(false)
    }

    // =============================================================
    // Scope helpers
    // =============================================================

    /// Check if an element with the given tag is in scope.
    fn has_in_scope(&self, tag: &TagName) -> bool {
        for &id in self.open_elements.iter().rev() {
            if let Some(t) = self.tag_of(id) {
                if t == tag {
                    return true;
                }
                if matches!(
                    t,
                    TagName::Html | TagName::Table | TagName::Td | TagName::Th | TagName::Caption
                ) {
                    return false;
                }
            }
        }
        false
    }

    fn has_in_list_scope(&self, tag: &TagName) -> bool {
        for &id in self.open_elements.iter().rev() {
            if let Some(t) = self.tag_of(id) {
                if t == tag {
                    return true;
                }
                if matches!(
                    t,
                    TagName::Html
                        | TagName::Table
                        | TagName::Td
                        | TagName::Th
                        | TagName::Caption
                        | TagName::Ol
                        | TagName::Ul
                ) {
                    return false;
                }
            }
        }
        false
    }

    fn has_in_table_scope(&self, tag: &TagName) -> bool {
        for &id in self.open_elements.iter().rev() {
            if let Some(t) = self.tag_of(id) {
                if t == tag {
                    return true;
                }
                if matches!(t, TagName::Html | TagName::Table) {
                    return false;
                }
            }
        }
        false
    }

    fn has_heading_in_scope(&self) -> bool {
        for &id in self.open_elements.iter().rev() {
            if let Some(t) = self.tag_of(id) {
                if matches!(
                    t,
                    TagName::H1
                        | TagName::H2
                        | TagName::H3
                        | TagName::H4
                        | TagName::H5
                        | TagName::H6
                ) {
                    return true;
                }
                if matches!(
                    t,
                    TagName::Html | TagName::Table | TagName::Td | TagName::Th | TagName::Caption
                ) {
                    return false;
                }
            }
        }
        false
    }

    // =============================================================
    // Auto-close helpers
    // =============================================================

    /// If there is a `<p>` in scope, pop elements until it is closed.
    fn close_p_if_in_scope(&mut self) {
        if self.has_in_scope(&TagName::P) {
            self.close_to_tag(&TagName::P);
        }
    }

    /// If there is an open `<li>`, close it.
    fn close_li_if_in_scope(&mut self) {
        if self.has_in_list_scope(&TagName::Li) {
            self.close_to_tag(&TagName::Li);
        }
    }

    /// If there is an open `<dt>` or `<dd>`, close it.
    fn close_dt_dd_if_in_scope(&mut self) {
        if self.has_in_scope(&TagName::Dd) {
            self.close_to_tag(&TagName::Dd);
        }
        if self.has_in_scope(&TagName::Dt) {
            self.close_to_tag(&TagName::Dt);
        }
    }

    /// Pop elements from the stack until we pop one with the given
    /// tag.
    fn close_to_tag(&mut self, tag: &TagName) {
        while let Some(id) = self.open_elements.pop() {
            if self.tag_of(id) == Some(tag) {
                return;
            }
        }
    }

    /// Pop elements until we pop the first heading element.
    fn close_to_first_heading(&mut self) {
        while let Some(id) = self.open_elements.pop() {
            if let Some(t) = self.tag_of(id)
                && matches!(
                    t,
                    TagName::H1
                        | TagName::H2
                        | TagName::H3
                        | TagName::H4
                        | TagName::H5
                        | TagName::H6
                )
            {
                return;
            }
        }
    }

    /// Pop elements looking for a match (without scope boundaries).
    fn close_to_tag_any_scope(&mut self, tag: &TagName) {
        let idx = self
            .open_elements
            .iter()
            .rposition(|&id| self.tag_of(id) == Some(tag));
        if let Some(idx) = idx {
            self.open_elements.truncate(idx);
        }
    }

    /// Close the current cell (`<td>` or `<th>`).
    fn close_current_cell(&mut self) {
        if self.has_in_table_scope(&TagName::Td) {
            self.close_to_tag(&TagName::Td);
        } else if self.has_in_table_scope(&TagName::Th) {
            self.close_to_tag(&TagName::Th);
        }
    }

    /// Close the current table body section.
    fn close_current_table_body(&mut self) {
        for tag in &[TagName::Tbody, TagName::Thead, TagName::Tfoot] {
            if self.has_in_table_scope(tag) {
                self.close_to_tag(tag);
                return;
            }
        }
    }

    /// After closing `</table>`, reset mode based on the new current
    /// element.
    fn reset_mode_after_table(&mut self) {
        if let Some(tag) = self.tag_of(self.current_node()) {
            match tag {
                TagName::Tbody | TagName::Thead | TagName::Tfoot => {
                    self.mode = InsertionMode::InTableBody;
                    return;
                },
                TagName::Tr => {
                    self.mode = InsertionMode::InRow;
                    return;
                },
                TagName::Td | TagName::Th => {
                    self.mode = InsertionMode::InCell;
                    return;
                },
                _ => {},
            }
        }
        self.mode = InsertionMode::InBody;
    }

    // =============================================================
    // Formatting element helpers
    // =============================================================

    /// Close an `<a>` element from the active formatting list if one
    /// is present, before opening a new `<a>`.
    fn close_formatting_a_if_active(&mut self) {
        let idx = self
            .active_formatting
            .iter()
            .rposition(|&id| self.tag_of(id) == Some(&TagName::A));
        if let Some(fmt_idx) = idx {
            let node_id = self.active_formatting[fmt_idx];
            self.active_formatting.remove(fmt_idx);
            if let Some(pos) = self.open_elements.iter().rposition(|&id| id == node_id) {
                self.open_elements.remove(pos);
            }
        }
    }

    /// Close a formatting element by tag name.
    fn close_formatting_element(&mut self, tag: &TagName) {
        let fmt_idx = self
            .active_formatting
            .iter()
            .rposition(|&id| self.tag_of(id) == Some(tag));
        if let Some(fi) = fmt_idx {
            let node_id = self.active_formatting[fi];
            self.active_formatting.remove(fi);
            if let Some(pos) = self.open_elements.iter().rposition(|&id| id == node_id) {
                self.open_elements.truncate(pos);
            }
        } else {
            self.close_to_tag_any_scope(tag);
        }
    }

    /// Simplified reconstruction of active formatting elements.
    fn reconstruct_formatting(&mut self) {
        if self.active_formatting.is_empty() {
            return;
        }
        let to_reopen: Vec<NodeId> = self
            .active_formatting
            .iter()
            .filter(|&&id| !self.open_elements.contains(&id))
            .copied()
            .collect();

        for id in to_reopen {
            let (tag, attrs) = if let Some(data) = self.doc.element(id) {
                (data.tag.clone(), data.attributes.clone())
            } else {
                continue;
            };
            let mut new_data = ElementData::new(tag);
            new_data.attributes = attrs;
            let new_id = self.doc.add_node(NodeKind::Element(new_data));
            self.insert_element(new_id);

            if let Some(pos) = self.active_formatting.iter().position(|&fid| fid == id) {
                self.active_formatting[pos] = new_id;
            }
        }
    }

    // =============================================================
    // Foster parenting
    // =============================================================

    /// Foster-parent a token: insert before the table's parent
    /// (simplified).
    fn foster_parent(&mut self, token: Token) {
        let table_idx = self
            .open_elements
            .iter()
            .rposition(|&id| self.tag_of(id) == Some(&TagName::Table));

        let foster_target = if let Some(idx) = table_idx {
            let table_id = self.open_elements[idx];
            self.doc.nodes[table_id].parent.unwrap_or_else(|| {
                if idx > 0 {
                    self.open_elements[idx - 1]
                } else {
                    self.doc.root
                }
            })
        } else {
            self.current_node()
        };

        match token {
            Token::Character(ref s) => {
                let children = &self.doc.nodes[foster_target].children;
                if let Some(&last) = children.last()
                    && let NodeKind::Text(ref mut t) = self.doc.nodes[last].kind
                {
                    t.push_str(s);
                    return;
                }
                let id = self.doc.add_node(NodeKind::Text(s.clone()));
                self.doc.append_child(foster_target, id);
            },
            Token::Comment(text) => {
                let id = self.doc.add_node(NodeKind::Comment(text));
                self.doc.append_child(foster_target, id);
            },
            Token::StartTag(ref tag) => {
                let id = self.create_element_from_start_tag(tag);
                self.doc.append_child(foster_target, id);
                let tag_name = TagName::from_str(&tag.name.to_ascii_lowercase());
                if !tag_name.is_void() && !tag.self_closing {
                    self.open_elements.push(id);
                }
            },
            Token::EndTag(ref tag) => {
                let tag_name = TagName::from_str(&tag.name.to_ascii_lowercase());
                self.close_to_tag_any_scope(&tag_name);
            },
            _ => {},
        }
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
fn is_all_whitespace(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_whitespace())
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::tokenizer::{Attribute as TokAttr, EndTagToken, StartTagToken};
    use super::*;

    // Convenience helpers for building token streams.

    fn start(name: &str) -> Token {
        Token::StartTag(StartTagToken {
            name: name.to_string(),
            self_closing: false,
            attributes: Vec::new(),
        })
    }

    fn start_with_attrs(name: &str, attrs: Vec<(&str, &str)>) -> Token {
        Token::StartTag(StartTagToken {
            name: name.to_string(),
            self_closing: false,
            attributes: attrs
                .into_iter()
                .map(|(n, v)| TokAttr {
                    name: n.to_string(),
                    value: v.to_string(),
                })
                .collect(),
        })
    }

    fn end(name: &str) -> Token {
        Token::EndTag(EndTagToken {
            name: name.to_string(),
        })
    }

    fn text(s: &str) -> Token {
        Token::Character(s.to_string())
    }

    fn tag_at(doc: &Document, id: NodeId) -> Option<&TagName> {
        doc.element(id).map(|e| &e.tag)
    }

    // ---- Test 1: Simple document structure ----

    #[test]
    fn simple_document() {
        let tokens = vec![
            start("html"),
            start("body"),
            start("p"),
            text("Hello"),
            end("p"),
            end("body"),
            end("html"),
            Token::Eof,
        ];
        let doc = TreeBuilder::build(tokens);

        let body = doc.body().expect("has body");
        let body_children = &doc.get(body).children;
        assert_eq!(body_children.len(), 1);

        let p = body_children[0];
        assert_eq!(tag_at(&doc, p), Some(&TagName::P));
        assert_eq!(doc.text_content(p), "Hello");
    }

    // ---- Test 2: Implicit elements ----

    #[test]
    fn implicit_elements() {
        let tokens = vec![start("p"), text("Hello"), end("p"), Token::Eof];
        let doc = TreeBuilder::build(tokens);

        assert!(doc.head().is_some());
        assert!(doc.body().is_some());

        let body = doc.body().unwrap();
        let body_children = &doc.get(body).children;
        assert!(!body_children.is_empty());

        let p = body_children[0];
        assert_eq!(tag_at(&doc, p), Some(&TagName::P));
        assert_eq!(doc.text_content(p), "Hello");
    }

    // ---- Test 3: Void elements ----

    #[test]
    fn void_elements() {
        let tokens = vec![
            start("p"),
            text("Hello"),
            start("br"),
            text("World"),
            end("p"),
            Token::Eof,
        ];
        let doc = TreeBuilder::build(tokens);

        let body = doc.body().unwrap();
        let p = doc.get(body).children[0];
        assert_eq!(tag_at(&doc, p), Some(&TagName::P));

        // p should have: Text("Hello"), <br>, Text("World")
        let p_children = &doc.get(p).children;
        assert_eq!(p_children.len(), 3);

        assert!(matches!(
            &doc.get(p_children[0]).kind,
            NodeKind::Text(t) if t == "Hello"
        ));
        assert_eq!(tag_at(&doc, p_children[1]), Some(&TagName::Br),);
        assert!(doc.get(p_children[1]).children.is_empty());
        assert!(matches!(
            &doc.get(p_children[2]).kind,
            NodeKind::Text(t) if t == "World"
        ));
    }

    // ---- Test 4: Auto-close p ----

    #[test]
    fn auto_close_p() {
        let tokens = vec![
            start("p"),
            text("First"),
            start("p"),
            text("Second"),
            Token::Eof,
        ];
        let doc = TreeBuilder::build(tokens);

        let body = doc.body().unwrap();
        let body_children = &doc.get(body).children;

        let ps: Vec<NodeId> = body_children
            .iter()
            .filter(|&&id| tag_at(&doc, id) == Some(&TagName::P))
            .copied()
            .collect();
        assert_eq!(ps.len(), 2);
        assert_eq!(doc.text_content(ps[0]), "First");
        assert_eq!(doc.text_content(ps[1]), "Second");
    }

    // ---- Test 5: Auto-close li ----

    #[test]
    fn auto_close_li() {
        let tokens = vec![
            start("ul"),
            start("li"),
            text("One"),
            start("li"),
            text("Two"),
            end("ul"),
            Token::Eof,
        ];
        let doc = TreeBuilder::build(tokens);

        let body = doc.body().unwrap();
        let ul = doc.get(body).children[0];
        assert_eq!(tag_at(&doc, ul), Some(&TagName::Ul));

        let lis: Vec<NodeId> = doc
            .get(ul)
            .children
            .iter()
            .filter(|&&id| tag_at(&doc, id) == Some(&TagName::Li))
            .copied()
            .collect();
        assert_eq!(lis.len(), 2);
        assert_eq!(doc.text_content(lis[0]), "One");
        assert_eq!(doc.text_content(lis[1]), "Two");
    }

    // ---- Test 6: Nested divs ----

    #[test]
    fn nested_divs() {
        let tokens = vec![
            start("div"),
            start("div"),
            start("p"),
            text("text"),
            end("p"),
            end("div"),
            end("div"),
            Token::Eof,
        ];
        let doc = TreeBuilder::build(tokens);

        let body = doc.body().unwrap();
        let outer = doc.get(body).children[0];
        assert_eq!(tag_at(&doc, outer), Some(&TagName::Div));

        let inner = doc.get(outer).children[0];
        assert_eq!(tag_at(&doc, inner), Some(&TagName::Div));

        let p = doc.get(inner).children[0];
        assert_eq!(tag_at(&doc, p), Some(&TagName::P));
        assert_eq!(doc.text_content(p), "text");
    }

    // ---- Test 7: Formatting elements ----

    #[test]
    fn formatting_elements() {
        let tokens = vec![
            start("p"),
            start("b"),
            text("bold "),
            start("i"),
            text("bold-italic"),
            end("i"),
            end("b"),
            end("p"),
            Token::Eof,
        ];
        let doc = TreeBuilder::build(tokens);

        let body = doc.body().unwrap();
        let p = doc.get(body).children[0];
        assert_eq!(tag_at(&doc, p), Some(&TagName::P));

        let b = doc.get(p).children[0];
        assert_eq!(tag_at(&doc, b), Some(&TagName::B));

        let b_children = &doc.get(b).children;
        assert!(b_children.len() >= 2);

        assert!(matches!(
            &doc.get(b_children[0]).kind,
            NodeKind::Text(t) if t == "bold "
        ));

        let i = b_children[1];
        assert_eq!(tag_at(&doc, i), Some(&TagName::I));
        assert_eq!(doc.text_content(i), "bold-italic");
    }

    // ---- Test 8: Text coalescing ----

    #[test]
    fn text_coalescing() {
        // Multiple consecutive Character tokens should coalesce into
        // a single text node.
        let tokens = vec![
            start("p"),
            text("H"),
            text("i"),
            text("!"),
            end("p"),
            Token::Eof,
        ];
        let doc = TreeBuilder::build(tokens);

        let body = doc.body().unwrap();
        let p = doc.get(body).children[0];
        let p_children = &doc.get(p).children;
        assert_eq!(p_children.len(), 1);
        assert!(matches!(
            &doc.get(p_children[0]).kind,
            NodeKind::Text(t) if t == "Hi!"
        ));
    }

    // ---- Test 9: Table structure ----

    #[test]
    fn table_structure() {
        let tokens = vec![
            start("table"),
            start("tr"),
            start("td"),
            text("cell"),
            end("td"),
            end("tr"),
            end("table"),
            Token::Eof,
        ];
        let doc = TreeBuilder::build(tokens);

        let body = doc.body().unwrap();
        let table = doc.get(body).children[0];
        assert_eq!(tag_at(&doc, table), Some(&TagName::Table));

        // table -> tbody (implicit)
        let tbody = doc.get(table).children[0];
        assert_eq!(tag_at(&doc, tbody), Some(&TagName::Tbody));

        // tbody -> tr
        let tr = doc.get(tbody).children[0];
        assert_eq!(tag_at(&doc, tr), Some(&TagName::Tr));

        // tr -> td
        let td = doc.get(tr).children[0];
        assert_eq!(tag_at(&doc, td), Some(&TagName::Td));

        assert_eq!(doc.text_content(td), "cell");
    }

    // ---- Test 10: Mixed content ----

    #[test]
    fn mixed_content() {
        let tokens = vec![
            start("h1"),
            text("Title"),
            end("h1"),
            start("p"),
            text("A paragraph with "),
            start_with_attrs("a", vec![("href", "https://example.com")]),
            text("a link"),
            end("a"),
            text("."),
            end("p"),
            start("ul"),
            start("li"),
            text("Item 1"),
            end("li"),
            start("li"),
            text("Item 2"),
            end("li"),
            end("ul"),
            Token::Eof,
        ];
        let doc = TreeBuilder::build(tokens);

        let body = doc.body().unwrap();
        let children = &doc.get(body).children;

        // h1, p, ul
        assert!(children.len() >= 3);

        let h1 = children[0];
        assert_eq!(tag_at(&doc, h1), Some(&TagName::H1));
        assert_eq!(doc.text_content(h1), "Title");

        let p = children[1];
        assert_eq!(tag_at(&doc, p), Some(&TagName::P));
        assert_eq!(doc.text_content(p), "A paragraph with a link.",);

        let a = doc
            .get(p)
            .children
            .iter()
            .find(|&&id| tag_at(&doc, id) == Some(&TagName::A))
            .copied()
            .expect("has <a>");
        assert_eq!(doc.element(a).unwrap().href(), Some("https://example.com"),);

        let ul = children[2];
        assert_eq!(tag_at(&doc, ul), Some(&TagName::Ul));
        let lis: Vec<NodeId> = doc
            .get(ul)
            .children
            .iter()
            .filter(|&&id| tag_at(&doc, id) == Some(&TagName::Li))
            .copied()
            .collect();
        assert_eq!(lis.len(), 2);
        assert_eq!(doc.text_content(lis[0]), "Item 1");
        assert_eq!(doc.text_content(lis[1]), "Item 2");
    }

    // ---- Doctype is handled ----

    #[test]
    fn doctype_skipped() {
        let tokens = vec![
            Token::Doctype(super::super::tokenizer::DoctypeToken {
                name: Some("html".to_string()),
                force_quirks: false,
            }),
            start("html"),
            start("head"),
            end("head"),
            start("body"),
            end("body"),
            end("html"),
            Token::Eof,
        ];
        let doc = TreeBuilder::build(tokens);
        assert!(doc.body().is_some());
        assert!(doc.head().is_some());
    }

    // ---- Empty document ----

    #[test]
    fn empty_document() {
        let tokens = vec![Token::Eof];
        let doc = TreeBuilder::build(tokens);
        // Should have at least Document root + implicit html.
        assert!(doc.nodes.len() >= 2);
    }

    // ---- Heading auto-close ----

    #[test]
    fn heading_auto_close() {
        let tokens = vec![
            start("h1"),
            text("First"),
            start("h2"),
            text("Second"),
            Token::Eof,
        ];
        let doc = TreeBuilder::build(tokens);

        let body = doc.body().unwrap();
        let headings: Vec<NodeId> = doc
            .get(body)
            .children
            .iter()
            .filter(|&&id| matches!(tag_at(&doc, id), Some(TagName::H1) | Some(TagName::H2)))
            .copied()
            .collect();
        assert_eq!(headings.len(), 2);
        assert_eq!(doc.text_content(headings[0]), "First");
        assert_eq!(doc.text_content(headings[1]), "Second");
    }

    // ---- Title in head ----

    #[test]
    fn title_in_head() {
        let tokens = vec![
            start("html"),
            start("head"),
            start("title"),
            text("My Page"),
            end("title"),
            end("head"),
            start("body"),
            end("body"),
            end("html"),
            Token::Eof,
        ];
        let doc = TreeBuilder::build(tokens);
        assert_eq!(doc.title(), Some("My Page".to_string()));
    }

    // ---- Attributes preserved ----

    #[test]
    fn attributes_preserved() {
        let tokens = vec![
            start_with_attrs("div", vec![("id", "main"), ("class", "container")]),
            end("div"),
            Token::Eof,
        ];
        let doc = TreeBuilder::build(tokens);

        let found = doc.get_element_by_id("main").expect("found by id");
        let data = doc.element(found).unwrap();
        assert_eq!(data.id(), Some("main"));
        assert!(data.has_class("container"));
    }

    // ---- Default trait ----

    #[test]
    fn default_trait() {
        let builder = TreeBuilder::default();
        assert_eq!(builder.mode, InsertionMode::Initial);
        assert!(builder.open_elements.is_empty());
    }
}
