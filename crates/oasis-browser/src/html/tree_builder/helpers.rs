//! Element creation, stack, scope, and auto-close helpers.

use super::super::dom::{Attribute as DomAttribute, ElementData, NodeId, NodeKind, TagName};
use super::super::tokenizer::StartTagToken;
use super::{InsertionMode, MAX_NESTING_DEPTH, TreeBuilder};

impl TreeBuilder {
    // =============================================================
    // Element creation helpers
    // =============================================================

    /// Create a DOM element node from a start tag token.
    pub(crate) fn create_element_from_start_tag(&mut self, tag: &StartTagToken) -> NodeId {
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
    pub(crate) fn create_element(&mut self, tag: TagName) -> NodeId {
        self.doc.add_node(NodeKind::Element(ElementData::new(tag)))
    }

    /// Insert an element as the last child of the current node and
    /// push it onto the open elements stack.
    ///
    /// When the stack has reached [`MAX_NESTING_DEPTH`], the element
    /// is still appended as a child but is **not** pushed onto the
    /// stack. This means subsequent content will be attached to the
    /// current parent rather than nesting deeper, preventing stack
    /// exhaustion from pathologically deep HTML.
    pub(crate) fn insert_element(&mut self, id: NodeId) {
        let parent = self.current_node();
        self.doc.append_child(parent, id);
        if self.open_elements.len() < MAX_NESTING_DEPTH {
            self.open_elements.push(id);
        }
    }

    /// Insert text, coalescing into an existing trailing text node
    /// when possible.
    pub(crate) fn insert_text(&mut self, text: &str) {
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
    pub(crate) fn current_node(&self) -> NodeId {
        self.open_elements.last().copied().unwrap_or(self.doc.root)
    }

    /// Pop the top of the open elements stack.
    pub(crate) fn pop_open_element(&mut self) -> Option<NodeId> {
        self.open_elements.pop()
    }

    /// Get the tag name of the element at `node_id`.
    pub(crate) fn tag_of(&self, node_id: NodeId) -> Option<&TagName> {
        self.doc.element(node_id).map(|e| &e.tag)
    }

    pub(crate) fn current_node_is_heading(&self) -> bool {
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
    pub(crate) fn has_in_scope(&self, tag: &TagName) -> bool {
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

    pub(crate) fn has_in_list_scope(&self, tag: &TagName) -> bool {
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

    pub(crate) fn has_in_table_scope(&self, tag: &TagName) -> bool {
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

    pub(crate) fn has_heading_in_scope(&self) -> bool {
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
    pub(crate) fn close_p_if_in_scope(&mut self) {
        if self.has_in_scope(&TagName::P) {
            self.close_to_tag(&TagName::P);
        }
    }

    /// If there is an open `<li>`, close it.
    pub(crate) fn close_li_if_in_scope(&mut self) {
        if self.has_in_list_scope(&TagName::Li) {
            self.close_to_tag(&TagName::Li);
        }
    }

    /// If there is an open `<dt>` or `<dd>`, close it.
    pub(crate) fn close_dt_dd_if_in_scope(&mut self) {
        if self.has_in_scope(&TagName::Dd) {
            self.close_to_tag(&TagName::Dd);
        }
        if self.has_in_scope(&TagName::Dt) {
            self.close_to_tag(&TagName::Dt);
        }
    }

    /// Pop elements from the stack until we pop one with the given
    /// tag.
    pub(crate) fn close_to_tag(&mut self, tag: &TagName) {
        while let Some(id) = self.open_elements.pop() {
            if self.tag_of(id) == Some(tag) {
                return;
            }
        }
    }

    /// Pop elements until we pop the first heading element.
    pub(crate) fn close_to_first_heading(&mut self) {
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
    pub(crate) fn close_to_tag_any_scope(&mut self, tag: &TagName) {
        let idx = self
            .open_elements
            .iter()
            .rposition(|&id| self.tag_of(id) == Some(tag));
        if let Some(idx) = idx {
            self.open_elements.truncate(idx);
        }
    }

    /// Close the current cell (`<td>` or `<th>`).
    pub(crate) fn close_current_cell(&mut self) {
        if self.has_in_table_scope(&TagName::Td) {
            self.close_to_tag(&TagName::Td);
        } else if self.has_in_table_scope(&TagName::Th) {
            self.close_to_tag(&TagName::Th);
        }
    }

    /// Close the current table body section.
    pub(crate) fn close_current_table_body(&mut self) {
        for tag in &[TagName::Tbody, TagName::Thead, TagName::Tfoot] {
            if self.has_in_table_scope(tag) {
                self.close_to_tag(tag);
                return;
            }
        }
    }

    /// After closing `</table>`, reset mode based on the new current
    /// element.
    pub(crate) fn reset_mode_after_table(&mut self) {
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
}
