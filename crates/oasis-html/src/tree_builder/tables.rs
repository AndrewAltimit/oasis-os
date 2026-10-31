//! The table and select insertion modes (WHATWG §13.2.6.4.9 –
//! §13.2.6.4.17): "in table", "in caption", "in column group", "in table
//! body", "in row", "in cell", "in select", "in select in table".

use super::super::dom::TagName;
use super::super::tokenizer::Token;
use super::{InsertionMode, TreeBuilder, is_all_whitespace};

const TABLE_CONTEXT: &[&str] = &["table", "template"];
const TABLE_BODY_CONTEXT: &[&str] = &["tbody", "tfoot", "thead", "template"];
const TABLE_ROW_CONTEXT: &[&str] = &["tr", "template"];

/// Start tag `name`?
fn is_start(token: &Token, names: &[&str]) -> bool {
    matches!(token, Token::StartTag(t) if names.contains(&t.name.as_str()))
}

/// End tag `name`?
fn is_end(token: &Token, names: &[&str]) -> bool {
    matches!(token, Token::EndTag(t) if names.contains(&t.name.as_str()))
}

fn tag_name(token: &Token) -> &str {
    match token {
        Token::StartTag(t) => &t.name,
        Token::EndTag(t) => &t.name,
        _ => "",
    }
}

impl TreeBuilder {
    // =============================================================
    // In table
    // =============================================================

    pub(crate) fn handle_in_table(&mut self, token: Token) {
        match &token {
            Token::Character(s) => {
                let table_target =
                    self.current_is_any(&["table", "tbody", "template", "tfoot", "thead", "tr"]);
                if table_target && is_all_whitespace(s) {
                    // Whitespace between table parts stays in the table.
                    self.insert_text(s);
                } else {
                    // Other text is foster-parented before the table.
                    self.foster_parent(token);
                }
            },
            Token::Comment(text) => self.insert_comment(text.clone()),
            Token::Doctype(_) => {},
            Token::StartTag(tag) => match tag.name.as_str() {
                "caption" => {
                    self.clear_stack_back_to(TABLE_CONTEXT);
                    self.insert_formatting_marker();
                    let id = self.create_element_from_start_tag(tag);
                    self.insert_element(id);
                    // Only enter the mode if the caption is open (past the
                    // nesting cap it is not, and the mode would never end).
                    if self.current_node() == id {
                        self.mode = InsertionMode::InCaption;
                    } else {
                        self.fmt_markers.pop();
                    }
                },
                "colgroup" => {
                    self.clear_stack_back_to(TABLE_CONTEXT);
                    let id = self.create_element_from_start_tag(tag);
                    self.insert_element(id);
                    if self.current_node() == id {
                        self.mode = InsertionMode::InColumnGroup;
                    }
                },
                "col" => {
                    self.clear_stack_back_to(TABLE_CONTEXT);
                    let id = self.create_element(TagName::Colgroup);
                    self.insert_element(id);
                    if self.current_node() == id {
                        self.mode = InsertionMode::InColumnGroup;
                        self.process_token(token);
                    } else {
                        let col = self.create_element_from_start_tag(tag);
                        self.doc.append_child(id, col);
                    }
                },
                "tbody" | "tfoot" | "thead" => {
                    self.clear_stack_back_to(TABLE_CONTEXT);
                    let id = self.create_element_from_start_tag(tag);
                    self.insert_element(id);
                    self.mode = InsertionMode::InTableBody;
                },
                "td" | "th" | "tr" => {
                    self.clear_stack_back_to(TABLE_CONTEXT);
                    let id = self.create_element(TagName::Tbody);
                    self.insert_element(id);
                    self.mode = InsertionMode::InTableBody;
                    self.process_token(token);
                },
                "table" => {
                    // A table start tag inside a table closes the first.
                    if self.has_in_table_scope(&TagName::Table) {
                        self.close_to_tag(&TagName::Table);
                        self.reset_insertion_mode();
                        self.process_token(token);
                    }
                },
                "style" | "script" | "template" => self.handle_in_head(token),
                "input"
                    if tag
                        .attributes
                        .iter()
                        .any(|a| a.name == "type" && a.value.eq_ignore_ascii_case("hidden")) =>
                {
                    let id = self.create_element_from_start_tag(tag);
                    self.insert_leaf(id);
                },
                "form" => {
                    let template = self
                        .open_elements
                        .iter()
                        .any(|&id| self.is_html_named(id, "template"));
                    if template || self.form_element.is_some() {
                        return;
                    }
                    let id = self.create_element_from_start_tag(tag);
                    self.insert_leaf(id);
                    self.form_element = Some(id);
                },
                _ => self.foster_parent(token),
            },
            Token::EndTag(tag) => match tag.name.as_str() {
                "table" => {
                    if self.has_in_table_scope(&TagName::Table) {
                        self.close_to_tag(&TagName::Table);
                        self.reset_insertion_mode();
                    }
                },
                "body" | "caption" | "col" | "colgroup" | "html" | "tbody" | "td" | "tfoot"
                | "th" | "thead" | "tr" => {},
                _ => self.foster_parent(token),
            },
            Token::Eof => self.handle_in_body(token),
        }
    }

    // =============================================================
    // In caption
    // =============================================================

    pub(crate) fn handle_in_caption(&mut self, token: Token) {
        if is_end(&token, &["caption"]) {
            self.close_caption();
            return;
        }
        if is_start(
            &token,
            &[
                "caption", "col", "colgroup", "tbody", "td", "tfoot", "th", "thead", "tr",
            ],
        ) || is_end(&token, &["table"])
        {
            if self.close_caption() {
                self.process_token(token);
            }
            return;
        }
        if is_end(
            &token,
            &[
                "body", "col", "colgroup", "html", "tbody", "td", "tfoot", "th", "thead", "tr",
            ],
        ) {
            return;
        }
        self.handle_in_body(token);
    }

    /// Close the open `<caption>`; `false` if none is in table scope.
    fn close_caption(&mut self) -> bool {
        if !self.has_in_table_scope(&TagName::Caption) {
            return false;
        }
        self.generate_implied_end_tags(None);
        self.close_to_tag(&TagName::Caption);
        self.clear_formatting_to_last_marker();
        self.mode = InsertionMode::InTable;
        true
    }

    // =============================================================
    // In column group
    // =============================================================

    pub(crate) fn handle_in_column_group(&mut self, token: Token) {
        match &token {
            Token::Character(s) if is_all_whitespace(s) => self.insert_text(s),
            Token::Comment(text) => self.insert_comment(text.clone()),
            Token::Doctype(_) => {},
            Token::StartTag(tag) if tag.name == "html" => self.handle_in_body(token),
            Token::StartTag(tag) if tag.name == "col" => {
                let id = self.create_element_from_start_tag(tag);
                self.insert_leaf(id);
            },
            Token::EndTag(tag) if tag.name == "colgroup" => {
                if self.current_is("colgroup") {
                    self.pop_open_element();
                    self.mode = InsertionMode::InTable;
                }
            },
            Token::EndTag(tag) if tag.name == "col" => {},
            Token::StartTag(tag) if tag.name == "template" => self.handle_in_head(token),
            Token::Eof => self.handle_in_body(token),
            _ => {
                if self.current_is("colgroup") {
                    self.pop_open_element();
                    self.mode = InsertionMode::InTable;
                    self.process_token(token);
                }
            },
        }
    }

    // =============================================================
    // In table body
    // =============================================================

    pub(crate) fn handle_in_table_body(&mut self, token: Token) {
        if is_start(&token, &["tr"]) {
            self.clear_stack_back_to(TABLE_BODY_CONTEXT);
            if let Token::StartTag(tag) = &token {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
            }
            self.mode = InsertionMode::InRow;
        } else if is_start(&token, &["th", "td"]) {
            self.clear_stack_back_to(TABLE_BODY_CONTEXT);
            let id = self.create_element(TagName::Tr);
            self.insert_element(id);
            self.mode = InsertionMode::InRow;
            self.process_token(token);
        } else if is_end(&token, &["tbody", "tfoot", "thead"]) {
            let tag = TagName::from_str(tag_name(&token));
            if self.has_in_table_scope(&tag) {
                self.clear_stack_back_to(TABLE_BODY_CONTEXT);
                self.pop_open_element();
                self.mode = InsertionMode::InTable;
            }
        } else if is_start(
            &token,
            &["caption", "col", "colgroup", "tbody", "tfoot", "thead"],
        ) || is_end(&token, &["table"])
        {
            let any_section = [TagName::Tbody, TagName::Thead, TagName::Tfoot]
                .iter()
                .any(|t| self.has_in_table_scope(t));
            if any_section {
                self.clear_stack_back_to(TABLE_BODY_CONTEXT);
                self.pop_open_element();
                self.mode = InsertionMode::InTable;
                self.process_token(token);
            }
        } else if is_end(
            &token,
            &[
                "body", "caption", "col", "colgroup", "html", "td", "th", "tr",
            ],
        ) {
            // Ignored.
        } else {
            self.handle_in_table(token);
        }
    }

    // =============================================================
    // In row
    // =============================================================

    pub(crate) fn handle_in_row(&mut self, token: Token) {
        if is_start(&token, &["th", "td"]) {
            self.clear_stack_back_to(TABLE_ROW_CONTEXT);
            if let Token::StartTag(tag) = &token {
                let id = self.create_element_from_start_tag(tag);
                self.insert_element(id);
            }
            self.mode = InsertionMode::InCell;
            self.insert_formatting_marker();
        } else if is_end(&token, &["tr"]) {
            self.close_row();
        } else if is_start(
            &token,
            &[
                "caption", "col", "colgroup", "tbody", "tfoot", "thead", "tr",
            ],
        ) || is_end(&token, &["table"])
        {
            if self.close_row() {
                self.process_token(token);
            }
        } else if is_end(&token, &["tbody", "tfoot", "thead"]) {
            let tag = TagName::from_str(tag_name(&token));
            if self.has_in_table_scope(&tag) && self.close_row() {
                self.process_token(token);
            }
        } else if is_end(
            &token,
            &["body", "caption", "col", "colgroup", "html", "td", "th"],
        ) {
            // Ignored.
        } else {
            self.handle_in_table(token);
        }
    }

    /// Close the open `<tr>`; `false` if none is in table scope.
    fn close_row(&mut self) -> bool {
        if !self.has_in_table_scope(&TagName::Tr) {
            return false;
        }
        self.clear_stack_back_to(TABLE_ROW_CONTEXT);
        self.pop_open_element();
        self.mode = InsertionMode::InTableBody;
        true
    }

    // =============================================================
    // In cell
    // =============================================================

    pub(crate) fn handle_in_cell(&mut self, token: Token) {
        if is_end(&token, &["td", "th"]) {
            let tag = TagName::from_str(tag_name(&token));
            if self.has_in_table_scope(&tag) {
                self.generate_implied_end_tags(None);
                self.close_to_tag(&tag);
                self.clear_formatting_to_last_marker();
                self.mode = InsertionMode::InRow;
            }
        } else if is_start(
            &token,
            &[
                "caption", "col", "colgroup", "tbody", "td", "tfoot", "th", "thead", "tr",
            ],
        ) {
            if self.close_cell() {
                self.process_token(token);
            }
        } else if is_end(&token, &["body", "caption", "col", "colgroup", "html"]) {
            // Ignored.
        } else if is_end(&token, &["table", "tbody", "tfoot", "thead", "tr"]) {
            let tag = TagName::from_str(tag_name(&token));
            if self.has_in_table_scope(&tag) && self.close_cell() {
                self.process_token(token);
            }
        } else {
            self.handle_in_body(token);
        }
    }

    /// Close the open `<td>` / `<th>`; `false` if neither is in table
    /// scope.
    pub(crate) fn close_cell(&mut self) -> bool {
        let cell = [TagName::Td, TagName::Th]
            .into_iter()
            .find(|t| self.has_in_table_scope(t));
        let Some(cell) = cell else {
            return false;
        };
        self.generate_implied_end_tags(None);
        self.close_to_tag(&cell);
        self.clear_formatting_to_last_marker();
        self.mode = InsertionMode::InRow;
        true
    }

    // =============================================================
    // In select / in select in table
    // =============================================================

    pub(crate) fn handle_in_select(&mut self, token: Token) {
        match &token {
            Token::Character(s) => self.insert_text(s),
            Token::Comment(text) => self.insert_comment(text.clone()),
            Token::Doctype(_) => {},
            Token::StartTag(tag) => match tag.name.as_str() {
                "html" => self.handle_in_body(token),
                "option" => {
                    if self.current_is("option") {
                        self.pop_open_element();
                    }
                    let id = self.create_element_from_start_tag(tag);
                    self.insert_element(id);
                },
                "optgroup" => {
                    if self.current_is("option") {
                        self.pop_open_element();
                    }
                    if self.current_is("optgroup") {
                        self.pop_open_element();
                    }
                    let id = self.create_element_from_start_tag(tag);
                    self.insert_element(id);
                },
                "hr" => {
                    if self.current_is("option") {
                        self.pop_open_element();
                    }
                    if self.current_is("optgroup") {
                        self.pop_open_element();
                    }
                    let id = self.create_element_from_start_tag(tag);
                    self.insert_leaf(id);
                },
                "select" => {
                    // A nested `<select>` closes the open one.
                    self.close_select();
                },
                "input" | "keygen" | "textarea" => {
                    if self.close_select() {
                        self.process_token(token);
                    }
                },
                "script" | "template" => self.handle_in_head(token),
                _ => {},
            },
            Token::EndTag(tag) => match tag.name.as_str() {
                "optgroup" => {
                    let below_is_optgroup = self.open_elements.len() >= 2
                        && self.is_html_named(
                            self.open_elements[self.open_elements.len() - 2],
                            "optgroup",
                        );
                    if self.current_is("option") && below_is_optgroup {
                        self.pop_open_element();
                    }
                    if self.current_is("optgroup") {
                        self.pop_open_element();
                    }
                },
                "option" => {
                    if self.current_is("option") {
                        self.pop_open_element();
                    }
                },
                "select" => {
                    self.close_select();
                },
                _ => {},
            },
            Token::Eof => self.handle_in_body(token),
        }
    }

    /// Pop through the open `<select>` and reset the insertion mode;
    /// `false` if no select is in select scope.
    fn close_select(&mut self) -> bool {
        if !self.has_in_select_scope(&TagName::Select) {
            return false;
        }
        self.close_to_tag(&TagName::Select);
        self.reset_insertion_mode();
        true
    }

    pub(crate) fn handle_in_select_in_table(&mut self, token: Token) {
        const TABLE_TAGS: &[&str] = &[
            "caption", "table", "tbody", "tfoot", "thead", "tr", "td", "th",
        ];
        if is_start(&token, TABLE_TAGS) {
            self.close_to_tag(&TagName::Select);
            self.reset_insertion_mode();
            self.process_token(token);
        } else if is_end(&token, TABLE_TAGS) {
            let tag = TagName::from_str(tag_name(&token));
            if self.has_in_table_scope(&tag) {
                self.close_to_tag(&TagName::Select);
                self.reset_insertion_mode();
                self.process_token(token);
            }
        } else {
            self.handle_in_select(token);
        }
    }
}
