//! Insertion mode handlers for each parsing state.

use super::super::dom::{NodeKind, TagName};
use super::super::tokenizer::Token;
use super::{InsertionMode, TreeBuilder, is_all_whitespace};

impl TreeBuilder {
    // =============================================================
    // Insertion-mode handlers
    // =============================================================

    pub(crate) fn handle_initial(&mut self, token: Token) {
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

    pub(crate) fn handle_before_html(&mut self, token: Token) {
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

    pub(crate) fn handle_before_head(&mut self, token: Token) {
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

    pub(crate) fn handle_in_head(&mut self, token: Token) {
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
            Token::StartTag(tag) if tag.name == "template" => {
                // Simplified template handling: insert as a regular
                // element. The UA stylesheet hides `template` via
                // `display: none`, so the contents won't paint. Per
                // WHATWG spec the contents should live in a separate
                // DocumentFragment that is unaffected by the enclosing
                // form/scope; we don't model that here.
                // SECURITY: children share enclosing scope; <script>
                // inside a moved template can execute.
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
            },
            Token::EndTag(tag) if tag.name == "template" => {
                if self.has_in_scope(&TagName::Template) {
                    self.close_to_tag_any_scope(&TagName::Template);
                } else {
                    log::trace!("html parse error: stray </template>");
                }
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
                // If a `<template>` is currently the open element on
                // top of the stack, dispatch the token via the InBody
                // rules so its children parse normally instead of
                // implicitly closing `<head>`.
                if self
                    .tag_of(self.current_node())
                    .is_some_and(|t| matches!(t, TagName::Template))
                {
                    self.handle_in_body(token);
                    return;
                }
                // Implicitly close <head> and reprocess.
                self.pop_open_element();
                self.mode = InsertionMode::AfterHead;
                self.process_token(token);
            },
        }
    }

    pub(crate) fn handle_after_head(&mut self, token: Token) {
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

    pub(crate) fn handle_in_body(&mut self, token: Token) {
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
                // Parse error: doctype is only valid in the Initial
                // insertion mode.
                log::trace!("html parse error: stray doctype in body");
            },
        }
    }

    /// Process a start tag while in InBody mode.
    pub(crate) fn handle_start_tag_in_body(
        &mut self,
        tag_name: &TagName,
        tag: &super::super::tokenizer::StartTagToken,
    ) {
        match tag_name {
            TagName::Html | TagName::Head | TagName::Body => {
                // Parse error: nested document-structure tag.
                log::trace!("html parse error: stray <{}> in body", tag_name.as_str());
            },
            // Stray table-structure tags in body scope are parse errors
            // per WHATWG §13.2.6.4.7 ("in body" insertion mode). Ignore
            // them — real browsers drop these instead of building a
            // floating <tr>/<td> outside any table.
            TagName::Caption
            | TagName::Col
            | TagName::Colgroup
            | TagName::Tbody
            | TagName::Thead
            | TagName::Tfoot
            | TagName::Tr
            | TagName::Td
            | TagName::Th => {
                // Ignore. If a <table> is in scope we'd never reach
                // here — `handle_in_table` / `handle_in_row` etc.
                // handle them. Outside a table, dropping is correct.
                log::trace!(
                    "html parse error: stray <{}> outside table",
                    tag_name.as_str()
                );
            },
            // Unmatched </frame>, </frameset>, <frameset> etc. in body
            // scope are also parse errors — we intentionally do not
            // support framesets at all, so they fall to the generic
            // arm below and get ignored via `close_to_tag_any_scope`.
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
            TagName::Br | TagName::Img | TagName::Input | TagName::Source => {
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
            TagName::Template => {
                // Simplified: insert as a plain element. The UA
                // stylesheet hides `template { display: none }` so its
                // contents never paint. Children parse in InBody mode.
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
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
    pub(crate) fn handle_end_tag_in_body(&mut self, tag_name: &TagName) {
        match tag_name {
            TagName::Body => {
                self.mode = InsertionMode::AfterBody;
            },
            TagName::Html => {
                self.mode = InsertionMode::AfterBody;
                // Reprocess </html> in AfterBody.
                self.process_token(Token::EndTag(super::super::tokenizer::EndTagToken {
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
                log::trace!("html parse error: </table> with no open table");
            },
            TagName::Template => {
                if self.has_in_scope(&TagName::Template) {
                    self.close_to_tag_any_scope(&TagName::Template);
                } else {
                    log::trace!("html parse error: stray </template>");
                }
            },
            _ if tag_name.is_formatting() => {
                self.close_formatting_element(tag_name);
            },
            _ => {
                self.close_to_tag_any_scope(tag_name);
            },
        }
    }

    pub(crate) fn handle_in_table(&mut self, token: Token) {
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
                // Per WHATWG §13.2.6.1, foster parenting only applies
                // when the *adjusted current node* is one of the table
                // section/row elements. If the user has already pushed
                // an unrelated element onto the stack (e.g. via an
                // earlier foster-parented `<div>`), subsequent tokens
                // attach to that current node directly via the InBody
                // rules instead of being foster-parented.
                let current_in_table_context = self.tag_of(self.current_node()).is_some_and(|t| {
                    matches!(
                        t,
                        TagName::Table
                            | TagName::Tbody
                            | TagName::Thead
                            | TagName::Tfoot
                            | TagName::Tr
                    )
                });
                if current_in_table_context {
                    log::trace!("html parse error: foster-parented token in table");
                    self.foster_parent(token);
                } else {
                    self.handle_in_body(token);
                }
            },
        }
    }

    pub(crate) fn handle_in_table_body(&mut self, token: Token) {
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

    pub(crate) fn handle_in_row(&mut self, token: Token) {
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

    pub(crate) fn handle_in_cell(&mut self, token: Token) {
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

    pub(crate) fn handle_after_body(&mut self, token: Token) {
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

    pub(crate) fn handle_after_after_body(&mut self, token: Token) {
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

    pub(crate) fn handle_text(&mut self, token: Token) {
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
}
