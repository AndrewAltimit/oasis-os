//! Insertion mode handlers outside tables (WHATWG §13.2.6.4); the
//! table and select modes live in `tables.rs`.

use super::super::dom::{NodeId, NodeKind, QuirksMode, TagName};
use super::super::tokenizer::{StartTagToken, Token};
use super::{InsertionMode, TreeBuilder, is_all_whitespace};

// The tag sets below are `matches!` rather than slices: a `contains`
// scan string-compared every entry for each tag (the in-body start-tag
// path ran three of them).

/// Block-ish start tags that close an open `<p>` (WHATWG §13.2.6.4.7).
fn closes_p(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "center"
            | "details"
            | "dialog"
            | "dir"
            | "div"
            | "dl"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "header"
            | "hgroup"
            | "main"
            | "menu"
            | "nav"
            | "ol"
            | "p"
            | "search"
            | "section"
            | "summary"
            | "ul"
    )
}

/// End tags that close their element when it is in scope.
fn is_block_end_tag(name: &str) -> bool {
    matches!(name, "button" | "listing" | "pre") || (name != "p" && closes_p(name))
}

/// Start tags handled by the "in head" rules wherever they appear.
fn is_head_content(name: &str) -> bool {
    matches!(
        name,
        "base"
            | "basefont"
            | "bgsound"
            | "link"
            | "meta"
            | "noframes"
            | "script"
            | "style"
            | "template"
            | "title"
    )
}

/// [`TagName::is_formatting`] by name, without building a `TagName`
/// (which allocates for unknown names).
fn is_formatting_name(name: &str) -> bool {
    matches!(
        name,
        "a" | "b"
            | "big"
            | "code"
            | "em"
            | "font"
            | "i"
            | "nobr"
            | "s"
            | "small"
            | "strike"
            | "strong"
            | "tt"
            | "u"
    )
}

fn is_heading_name(name: &str) -> bool {
    matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

impl TreeBuilder {
    // =============================================================
    // Before the body
    // =============================================================

    pub(crate) fn handle_initial(&mut self, token: Token) {
        match token {
            Token::Doctype(ref doctype) => {
                // The DOM keeps no doctype node, only the mode it selects.
                self.doc.quirks_mode = super::quirks::quirks_mode_for(doctype);
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
                // No DOCTYPE: quirks mode; reprocess in "before html".
                self.doc.quirks_mode = QuirksMode::Quirks;
                self.mode = InsertionMode::BeforeHtml;
                self.process_token(token);
            },
        }
    }

    pub(crate) fn handle_before_html(&mut self, token: Token) {
        match &token {
            Token::Doctype(_) => {},
            Token::Comment(text) => {
                let id = self.doc.add_node(NodeKind::Comment(text.clone()));
                let root = self.doc.root;
                self.doc.append_child(root, id);
            },
            Token::StartTag(tag) if tag.name == "html" => {
                let id = self.create_element_from_start_tag(tag);
                let root = self.doc.root;
                self.doc.append_child(root, id);
                self.open_elements.push(id);
                self.mode = InsertionMode::BeforeHead;
            },
            Token::Character(s) if is_all_whitespace(s) => {},
            Token::EndTag(tag) if !matches!(tag.name.as_str(), "head" | "body" | "html" | "br") => {
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
            Token::Character(s) if is_all_whitespace(s) => {},
            Token::Comment(text) => self.insert_comment(text.clone()),
            Token::Doctype(_) => {},
            Token::StartTag(tag) if tag.name == "html" => self.handle_in_body(token),
            Token::StartTag(tag) if tag.name == "head" => {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.head_element = Some(id);
                self.mode = InsertionMode::InHead;
            },
            Token::EndTag(tag) if !matches!(tag.name.as_str(), "head" | "body" | "html" | "br") => {
            },
            _ => {
                // Implicit <head>; the token is reprocessed "in head", so
                // a leading `<title>` / `<meta>` / `<link>` lands in it.
                let head = self.create_element(TagName::Head);
                self.insert_element(head);
                self.head_element = Some(head);
                self.mode = InsertionMode::InHead;
                self.process_token(token);
            },
        }
    }

    pub(crate) fn handle_in_head(&mut self, token: Token) {
        match &token {
            Token::Character(s) if is_all_whitespace(s) => {
                self.insert_text(s);
            },
            Token::Comment(text) => self.insert_comment(text.clone()),
            Token::Doctype(_) => {},
            Token::StartTag(tag) => match tag.name.as_str() {
                "html" => self.handle_in_body(token),
                "base" | "basefont" | "bgsound" | "link" | "meta" => {
                    let id = self.create_element_from_start_tag(tag);
                    self.insert_leaf(id);
                },
                "title" | "noframes" | "style" | "script" => self.insert_text_element(tag),
                "noscript" if self.scripting => self.insert_text_element(tag),
                "noscript" => {
                    let id = self.create_element_from_start_tag(tag);
                    self.insert_element(id);
                    self.mode = InsertionMode::InHeadNoscript;
                },
                "template" => self.open_template(tag),
                "head" => {},
                _ => self.head_anything_else(token),
            },
            Token::EndTag(tag) => match tag.name.as_str() {
                "template" => self.close_template(),
                "head" => {
                    self.pop_open_element();
                    self.mode = InsertionMode::AfterHead;
                },
                "body" | "html" | "br" => self.head_anything_else(token),
                _ => {},
            },
            _ => self.head_anything_else(token),
        }
    }

    /// "In head", anything else: pop `<head>` and reprocess "after head".
    fn head_anything_else(&mut self, token: Token) {
        if self.current_is("head") {
            self.pop_open_element();
        }
        self.mode = InsertionMode::AfterHead;
        self.process_token(token);
    }

    /// "In head noscript" (scripting off): only head content may follow.
    pub(crate) fn handle_in_head_noscript(&mut self, token: Token) {
        match &token {
            Token::Doctype(_) => {},
            Token::StartTag(tag) if tag.name == "html" => self.handle_in_body(token),
            Token::EndTag(tag) if tag.name == "noscript" => {
                self.pop_open_element();
                self.mode = InsertionMode::InHead;
            },
            Token::Character(s) if is_all_whitespace(s) => self.handle_in_head(token),
            Token::Comment(_) => self.handle_in_head(token),
            Token::StartTag(tag)
                if matches!(
                    tag.name.as_str(),
                    "basefont" | "bgsound" | "link" | "meta" | "noframes" | "style"
                ) =>
            {
                self.handle_in_head(token);
            },
            Token::StartTag(tag) if matches!(tag.name.as_str(), "head" | "noscript") => {},
            Token::EndTag(tag) if tag.name != "br" => {},
            _ => {
                self.pop_open_element();
                self.mode = InsertionMode::InHead;
                self.process_token(token);
            },
        }
    }

    pub(crate) fn handle_after_head(&mut self, token: Token) {
        match &token {
            Token::Character(s) if is_all_whitespace(s) => self.insert_text(s),
            Token::Comment(text) => self.insert_comment(text.clone()),
            Token::Doctype(_) => {},
            Token::StartTag(tag) if tag.name == "html" => self.handle_in_body(token),
            Token::StartTag(tag) if tag.name == "body" => {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
                self.frameset_ok = false;
                self.mode = InsertionMode::InBody;
            },
            Token::StartTag(tag) if is_head_content(&tag.name) => {
                // Head content after `</head>`: it still goes in the head.
                let Some(head) = self.head_element else {
                    self.after_head_anything_else(token);
                    return;
                };
                self.open_elements.push(head);
                self.handle_in_head(token);
                if let Some(pos) = self.open_elements.iter().rposition(|&id| id == head) {
                    self.open_elements.remove(pos);
                }
            },
            Token::StartTag(tag) if tag.name == "head" => {},
            Token::EndTag(tag) if !matches!(tag.name.as_str(), "body" | "html" | "br") => {},
            _ => self.after_head_anything_else(token),
        }
    }

    fn after_head_anything_else(&mut self, token: Token) {
        let body = self.create_element(TagName::Body);
        self.insert_element(body);
        self.mode = InsertionMode::InBody;
        self.process_token(token);
    }

    // =============================================================
    // In body
    // =============================================================

    pub(crate) fn handle_in_body(&mut self, token: Token) {
        match token {
            Token::Character(s) => {
                self.reconstruct_formatting();
                if !is_all_whitespace(&s) {
                    self.frameset_ok = false;
                }
                self.insert_text_owned(s);
            },
            Token::Comment(text) => self.insert_comment(text),
            Token::StartTag(ref tag) => self.handle_start_tag_in_body(tag, &token),
            Token::EndTag(ref tag) => {
                let name = super::lowercase(&tag.name);
                self.handle_end_tag_in_body(&name, &token);
            },
            Token::Eof => {
                // Open templates close by the "in template" rules; the
                // rest of the stack just ends.
                if !self.template_modes.is_empty() {
                    self.handle_in_template(Token::Eof);
                }
            },
            Token::Doctype(_) => {
                log::trace!("html parse error: stray doctype in body");
            },
        }
    }

    /// Process a start tag while in InBody mode.
    pub(crate) fn handle_start_tag_in_body(&mut self, tag: &StartTagToken, token: &Token) {
        let name = super::lowercase(&tag.name);
        let name = &*name;
        match name {
            "html" => {
                if let Some(&html) = self.open_elements.first()
                    && !self.template_open()
                {
                    self.merge_attributes(html, &tag.attributes);
                }
            },
            _ if is_head_content(name) => self.handle_in_head(token.clone()),
            "body" => {
                let body = self.open_elements.get(1).copied();
                if let Some(body) = body
                    && self.is_html_named(body, "body")
                    && !self.template_open()
                {
                    self.frameset_ok = false;
                    self.merge_attributes(body, &tag.attributes);
                }
            },
            "frameset" => {},
            _ if closes_p(name) => {
                self.close_p_if_in_scope();
                self.insert_html(tag);
            },
            _ if is_heading_name(name) => {
                self.close_p_if_in_scope();
                if self.current_node_is_heading() {
                    self.pop_open_element();
                }
                self.insert_html(tag);
            },
            "pre" | "listing" => {
                self.close_p_if_in_scope();
                self.insert_html(tag);
                self.ignore_lf = true;
                self.frameset_ok = false;
            },
            "form" => {
                if self.form_element.is_some() && !self.template_open() {
                    return; // Ignore nested forms.
                }
                self.close_p_if_in_scope();
                let id = self.insert_html(tag);
                if !self.template_open() {
                    self.form_element = Some(id);
                }
            },
            "li" | "dd" | "dt" => {
                self.frameset_ok = false;
                self.close_list_item_for(name);
                self.close_p_if_in_scope();
                self.insert_html(tag);
            },
            "plaintext" => {
                self.close_p_if_in_scope();
                self.insert_html(tag);
            },
            "button" => {
                if self.has_in_scope(&TagName::Button) {
                    self.generate_implied_end_tags(None);
                    self.close_to_tag(&TagName::Button);
                }
                self.reconstruct_formatting();
                self.insert_html(tag);
                self.frameset_ok = false;
            },
            "a" => {
                self.close_formatting_a_if_active();
                self.reconstruct_formatting();
                let id = self.insert_html(tag);
                self.push_active_formatting(id);
            },
            "nobr" => {
                self.reconstruct_formatting();
                let nobr = TagName::from_str("nobr");
                if self.has_in_scope(&nobr) {
                    self.close_formatting_element(&nobr);
                    self.reconstruct_formatting();
                }
                let id = self.insert_html(tag);
                self.push_active_formatting(id);
            },
            _ if is_formatting_name(name) => {
                self.reconstruct_formatting();
                let id = self.insert_html(tag);
                self.push_active_formatting(id);
            },
            "applet" | "marquee" | "object" => {
                self.reconstruct_formatting();
                self.insert_html(tag);
                self.insert_formatting_marker();
                self.frameset_ok = false;
            },
            "table" => {
                // In quirks mode a table may sit inside a paragraph.
                if self.doc.quirks_mode != QuirksMode::Quirks {
                    self.close_p_if_in_scope();
                }
                self.insert_html(tag);
                self.frameset_ok = false;
                self.mode = InsertionMode::InTable;
            },
            "area" | "br" | "embed" | "img" | "image" | "keygen" | "wbr" | "input" => {
                self.reconstruct_formatting();
                let id = if name == "image" {
                    // `<image>` is a misspelt `<img>`.
                    let mut img = tag.clone();
                    img.name = "img".to_string();
                    self.create_element_from_start_tag(&img)
                } else {
                    self.create_element_from_start_tag(tag)
                };
                self.insert_leaf(id);
                let hidden_input = name == "input"
                    && tag
                        .attributes
                        .iter()
                        .any(|a| a.name == "type" && a.value.eq_ignore_ascii_case("hidden"));
                if !hidden_input {
                    self.frameset_ok = false;
                }
            },
            "param" | "source" | "track" => {
                let id = self.create_element_from_start_tag(tag);
                self.insert_leaf(id);
            },
            "hr" => {
                self.close_p_if_in_scope();
                let id = self.create_element_from_start_tag(tag);
                self.insert_leaf(id);
                self.frameset_ok = false;
            },
            // RCDATA / raw-text elements take their text in the `Text`
            // mode: in-body text processing reopened formatting elements
            // inside them.
            "textarea" => {
                self.insert_text_element(tag);
                self.ignore_lf = true;
                self.frameset_ok = false;
            },
            "xmp" => {
                self.close_p_if_in_scope();
                self.reconstruct_formatting();
                self.frameset_ok = false;
                self.insert_text_element(tag);
            },
            "iframe" => {
                self.frameset_ok = false;
                self.insert_text_element(tag);
            },
            "noembed" => self.insert_text_element(tag),
            "noscript" if self.scripting => self.insert_text_element(tag),
            "select" => {
                self.reconstruct_formatting();
                let id = self.insert_html(tag);
                self.frameset_ok = false;
                if self.current_node() != id {
                    // Not open (nesting cap): "in select" would swallow
                    // the rest of the document.
                    return;
                }
                self.mode = if matches!(
                    self.mode,
                    InsertionMode::InTable
                        | InsertionMode::InCaption
                        | InsertionMode::InTableBody
                        | InsertionMode::InRow
                        | InsertionMode::InCell
                ) {
                    InsertionMode::InSelectInTable
                } else {
                    InsertionMode::InSelect
                };
            },
            "optgroup" | "option" => {
                if self.current_is("option") {
                    self.pop_open_element();
                }
                self.reconstruct_formatting();
                self.insert_html(tag);
            },
            "rb" | "rtc" => {
                if self.has_in_scope(&TagName::from_str("ruby")) {
                    self.generate_implied_end_tags(None);
                }
                self.insert_html(tag);
            },
            "rp" | "rt" => {
                if self.has_in_scope(&TagName::from_str("ruby")) {
                    self.generate_implied_end_tags(Some("rtc"));
                }
                self.insert_html(tag);
            },
            "svg" => {
                self.reconstruct_formatting();
                self.enter_foreign_root(tag, TagName::Svg);
            },
            "math" => {
                self.reconstruct_formatting();
                self.enter_foreign_root(tag, TagName::Math);
            },
            // Stray table-structure tags outside a table are parse
            // errors; real browsers drop them.
            "caption" | "col" | "colgroup" | "frame" | "head" | "tbody" | "td" | "tfoot" | "th"
            | "thead" | "tr" => {
                log::trace!("html parse error: stray <{name}> in body");
            },
            _ => {
                // Generic start tag. The self-closing flag means nothing
                // on non-void HTML elements (`<span/>` stays open).
                self.reconstruct_formatting();
                let id = self.create_element_from_start_tag(tag);
                if self.tag_of(id).is_some_and(TagName::is_void) {
                    self.insert_leaf(id);
                } else {
                    self.insert_element(id);
                }
            },
        }
    }

    /// Create the HTML element for `tag`, insert it and push it.
    fn insert_html(&mut self, tag: &StartTagToken) -> NodeId {
        let id = self.create_element_from_start_tag(tag);
        self.insert_element(id);
        id
    }

    /// Insert a raw-text / RCDATA element and enter the `Text` mode.
    fn insert_text_element(&mut self, tag: &StartTagToken) {
        let id = self.insert_html(tag);
        self.enter_text_mode(id);
    }

    /// Whether a `<template>` is open.
    fn template_open(&self) -> bool {
        self.open_elements
            .iter()
            .any(|&id| self.is_html_named(id, "template"))
    }

    /// The `li` / `dd` / `dt` start-tag loop: close an open list item of
    /// the same kind unless a special element (other than `address`,
    /// `div`, `p`) intervenes.
    fn close_list_item_for(&mut self, name: &str) {
        let targets: &[&str] = if name == "li" { &["li"] } else { &["dd", "dt"] };
        for idx in (0..self.open_elements.len()).rev() {
            let id = self.open_elements[idx];
            if let Some(&found) = targets.iter().find(|t| self.is_html_named(id, t)) {
                self.generate_implied_end_tags(Some(found));
                self.open_elements.truncate(idx);
                return;
            }
            let special = self.is_special_node(id);
            let exempt = ["address", "div", "p"]
                .iter()
                .any(|n| self.is_html_named(id, n));
            if special && !exempt {
                return;
            }
        }
    }

    /// Process an end tag (`name` lowercased) while in InBody mode.
    pub(crate) fn handle_end_tag_in_body(&mut self, name: &str, token: &Token) {
        let tag = TagName::from_str(name);
        match name {
            "template" => self.close_template(),
            "body" => {
                if self.has_in_scope(&TagName::Body) {
                    self.mode = InsertionMode::AfterBody;
                }
            },
            "html" => {
                if self.has_in_scope(&TagName::Body) {
                    self.mode = InsertionMode::AfterBody;
                    self.process_token(token.clone());
                }
            },
            _ if is_block_end_tag(name) => {
                if self.has_in_scope(&tag) {
                    self.generate_implied_end_tags(None);
                    self.close_to_tag(&tag);
                }
            },
            "form" => {
                if self.template_open() {
                    if self.has_in_scope(&TagName::Form) {
                        self.generate_implied_end_tags(None);
                        self.close_to_tag(&TagName::Form);
                    }
                    return;
                }
                let node = self.form_element.take();
                let Some(node) = node else { return };
                // "node in scope": a form is in scope and the nearest one
                // is the form element pointer.
                let in_scope = self.has_in_scope(&TagName::Form)
                    && self
                        .open_elements
                        .iter()
                        .rev()
                        .find(|&&id| self.is_html_named(id, "form"))
                        == Some(&node);
                if !in_scope {
                    return;
                }
                self.generate_implied_end_tags(None);
                if let Some(pos) = self.open_elements.iter().rposition(|&id| id == node) {
                    self.open_elements.remove(pos);
                }
            },
            "p" => {
                if !self.has_in_button_scope(&TagName::P) {
                    let p = self.create_element(TagName::P);
                    self.insert_element(p);
                }
                self.generate_implied_end_tags(Some("p"));
                self.close_to_tag(&TagName::P);
            },
            "li" => {
                if self.has_in_list_scope(&TagName::Li) {
                    self.generate_implied_end_tags(Some("li"));
                    self.close_to_tag(&TagName::Li);
                }
            },
            "dd" | "dt" => {
                if self.has_in_scope(&tag) {
                    self.generate_implied_end_tags(Some(name));
                    self.close_to_tag(&tag);
                }
            },
            _ if is_heading_name(name) => {
                if self.has_heading_in_scope() {
                    self.generate_implied_end_tags(None);
                    self.close_to_first_heading();
                }
            },
            "applet" | "marquee" | "object" => {
                if self.has_in_scope(&tag) {
                    self.generate_implied_end_tags(None);
                    self.close_to_tag(&tag);
                    self.clear_formatting_to_last_marker();
                }
            },
            "br" => {
                // `</br>` is treated as `<br>`.
                self.handle_start_tag_in_body(
                    &StartTagToken {
                        name: "br".to_string(),
                        self_closing: false,
                        attributes: Vec::new(),
                    },
                    token,
                );
            },
            "table" => {
                // Misplaced end tag; ignore in body mode.
                log::trace!("html parse error: </table> with no open table");
            },
            _ if tag.is_formatting() => self.close_formatting_element(&tag),
            _ => self.close_any_other_end_tag(&tag),
        }
    }

    // =============================================================
    // After the body
    // =============================================================

    pub(crate) fn handle_after_body(&mut self, token: Token) {
        match &token {
            Token::Character(s) if is_all_whitespace(s) => self.handle_in_body(token),
            Token::Comment(text) => {
                // Last child of the `<html>` element.
                let id = self.doc.add_node(NodeKind::Comment(text.clone()));
                let html = self.open_elements.first().copied().unwrap_or(self.doc.root);
                self.doc.append_child(html, id);
            },
            Token::Doctype(_) => {},
            Token::StartTag(tag) if tag.name == "html" => self.handle_in_body(token),
            Token::EndTag(tag) if tag.name == "html" => {
                self.mode = InsertionMode::AfterAfterBody;
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
            Token::Doctype(_) => self.handle_in_body(token),
            Token::Character(s) if is_all_whitespace(s) => self.handle_in_body(token),
            Token::StartTag(tag) if tag.name == "html" => self.handle_in_body(token),
            Token::Eof => {},
            _ => {
                // Reprocess in InBody.
                self.mode = InsertionMode::InBody;
                self.process_token(token);
            },
        }
    }

    // =============================================================
    // Text, template
    // =============================================================

    /// Switch to the `Text` insertion mode for the raw-text / RCDATA
    /// element `id` (`<script>`, `<style>`, `<title>`, ...).
    pub(crate) fn enter_text_mode(&mut self, id: NodeId) {
        self.original_mode = self.mode;
        self.mode = InsertionMode::Text;
        self.text_element = Some(id);
    }

    /// The `Text` insertion mode. Text goes into the raw-text element
    /// itself and its end tag pops only that element: at the nesting cap
    /// the element is not on the stack, and its contents used to become
    /// visible text of the parent (`<script>` source on the page) while
    /// the end tag popped the parent.
    pub(crate) fn handle_text(&mut self, token: Token) {
        let target = self.text_element.unwrap_or_else(|| self.current_node());
        match token {
            Token::Character(s) => self.append_text_owned(target, s),
            Token::EndTag(_) | Token::Eof => {
                if self.open_elements.last() == Some(&target) {
                    self.pop_open_element();
                }
                self.text_element = None;
                self.mode = self.original_mode;
                if token == Token::Eof {
                    self.process_token(Token::Eof);
                }
            },
            _ => {},
        }
    }

    /// `<template>` start tag (by the "in head" rules, wherever it
    /// appears): its contents start in the "in template" mode.
    pub(crate) fn open_template(&mut self, tag: &StartTagToken) {
        let id = self.insert_html(tag);
        if self.current_node() != id {
            // Not open (nesting cap): there is nothing to fill.
            return;
        }
        self.insert_formatting_marker();
        self.frameset_ok = false;
        self.mode = InsertionMode::InTemplate;
        self.template_modes.push(InsertionMode::InTemplate);
    }

    /// `</template>` in any HTML insertion mode (WHATWG §13.2.6.4.4):
    /// close the template, drop its template mode and reset the
    /// insertion mode for what encloses it.
    pub(crate) fn close_template(&mut self) {
        if !self.template_open() {
            log::trace!("html parse error: stray </template>");
            return;
        }
        self.generate_implied_end_tags_thoroughly();
        self.close_to_tag(&TagName::Template);
        self.clear_formatting_to_last_marker();
        self.template_modes.pop();
        self.reset_insertion_mode();
    }

    /// The "in template" insertion mode (WHATWG §13.2.6.4.18): the first
    /// start tag in a template picks the mode its contents parse in, so
    /// `<template><tr><td>` builds a row, not two dropped tags.
    pub(crate) fn handle_in_template(&mut self, token: Token) {
        let name = match &token {
            Token::Character(_) | Token::Comment(_) | Token::Doctype(_) => {
                self.handle_in_body(token);
                return;
            },
            Token::EndTag(_) => return,
            Token::Eof => {
                if !self.template_open() {
                    return;
                }
                self.close_to_tag(&TagName::Template);
                self.clear_formatting_to_last_marker();
                self.template_modes.pop();
                self.reset_insertion_mode();
                self.process_token(Token::Eof);
                return;
            },
            Token::StartTag(tag) => super::lowercase(&tag.name),
        };
        if is_head_content(&name) {
            self.handle_in_head(token);
            return;
        }
        let mode = match &*name {
            "caption" | "colgroup" | "tbody" | "tfoot" | "thead" => InsertionMode::InTable,
            "col" => InsertionMode::InColumnGroup,
            "tr" => InsertionMode::InTableBody,
            "td" | "th" => InsertionMode::InRow,
            _ => InsertionMode::InBody,
        };
        self.template_modes.pop();
        self.template_modes.push(mode);
        self.mode = mode;
        self.process_token(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_names_match_tag_name_is_formatting() {
        for name in [
            "a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong",
            "tt", "u", "mark", "span", "sub", "div", "x-custom",
        ] {
            assert_eq!(
                is_formatting_name(name),
                TagName::from_str(name).is_formatting(),
                "{name}"
            );
        }
    }

    #[test]
    fn block_end_tags_are_the_p_closers_plus_three() {
        assert!(is_block_end_tag("div") && is_block_end_tag("pre") && is_block_end_tag("button"));
        assert!(!is_block_end_tag("p") && !is_block_end_tag("span"));
        assert!(closes_p("p") && !closes_p("pre"));
    }
}
