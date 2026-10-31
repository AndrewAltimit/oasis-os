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
        self.push_open(id);
    }

    /// Push an already-inserted element onto the open elements stack if
    /// that stays within [`MAX_NESTING_DEPTH`]; returns whether it did.
    ///
    /// Both the stack size and the element's real DOM depth are checked:
    /// the stack can be shorter than the DOM is deep (`<a>` is pulled out
    /// of the middle of the stack when a new `<a>` opens, foster parenting
    /// inserts beside the table), and `<a><div>` repeated used to build a
    /// DOM twice as deep as the recursive passes are sized for.
    pub(crate) fn push_open(&mut self, id: NodeId) -> bool {
        if self.open_elements.len() >= MAX_NESTING_DEPTH || self.node_depth(id) > MAX_NESTING_DEPTH
        {
            return false;
        }
        self.open_elements.push(id);
        true
    }

    /// Depth of `id` below the document root (the root is 0).
    fn node_depth(&self, id: NodeId) -> usize {
        let mut depth = 0;
        let mut cur = self.doc.nodes[id].parent;
        while let Some(p) = cur {
            depth += 1;
            cur = self.doc.nodes[p].parent;
        }
        depth
    }

    /// Insert text, coalescing into an existing trailing text node
    /// when possible.
    pub(crate) fn insert_text(&mut self, text: &str) {
        let parent = self.current_node();
        self.append_text_to(parent, text);
    }

    /// Append text as the last child of `parent`, coalescing into an
    /// existing trailing text node.
    pub(crate) fn append_text_to(&mut self, parent: NodeId, text: &str) {
        if text.is_empty() {
            return;
        }
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
        self.tag_of(self.current_node()).is_some_and(is_heading)
    }

    // =============================================================
    // Scope helpers
    // =============================================================

    /// Walk the open elements from the current node: `true` at the
    /// first element `target` accepts, `false` at the first scope
    /// boundary `boundary` accepts (WHATWG §13.2.4.2 "has an element in
    /// the specific scope").
    fn in_specific_scope(
        &self,
        target: impl Fn(&TagName) -> bool,
        boundary: impl Fn(&TagName) -> bool,
    ) -> bool {
        for &id in self.open_elements.iter().rev() {
            if let Some(t) = self.tag_of(id) {
                if target(t) {
                    return true;
                }
                if boundary(t) {
                    return false;
                }
            }
        }
        false
    }

    /// The default scope boundaries. They include `template`, so scope
    /// queries cannot walk across a template into its enclosing tree.
    fn is_default_scope_boundary(t: &TagName) -> bool {
        matches!(
            t,
            TagName::Html
                | TagName::Table
                | TagName::Td
                | TagName::Th
                | TagName::Caption
                | TagName::Template
        )
    }

    /// Check if an element with the given tag is in (default) scope.
    pub(crate) fn has_in_scope(&self, tag: &TagName) -> bool {
        self.in_specific_scope(|t| t == tag, Self::is_default_scope_boundary)
    }

    /// List-item scope: the default boundaries plus `ol` / `ul`.
    pub(crate) fn has_in_list_scope(&self, tag: &TagName) -> bool {
        self.in_specific_scope(
            |t| t == tag,
            |t| Self::is_default_scope_boundary(t) || matches!(t, TagName::Ol | TagName::Ul),
        )
    }

    /// Table scope: only `html`, `table` and `template` bound it.
    pub(crate) fn has_in_table_scope(&self, tag: &TagName) -> bool {
        self.in_specific_scope(
            |t| t == tag,
            |t| matches!(t, TagName::Html | TagName::Table | TagName::Template),
        )
    }

    /// Whether any heading (`h1`..`h6`) is in default scope.
    pub(crate) fn has_heading_in_scope(&self) -> bool {
        self.in_specific_scope(is_heading, Self::is_default_scope_boundary)
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

    /// Pop elements from the stack through the topmost one with the
    /// given tag. Does nothing if no such element is open.
    ///
    /// Regression: this popped until it found the tag, so when it was
    /// not there (an implied `<p>` that hit the nesting cap, `</table>`
    /// with no table) the whole stack went and later content landed
    /// outside `<html>`.
    pub(crate) fn close_to_tag(&mut self, tag: &TagName) {
        if let Some(idx) = self
            .open_elements
            .iter()
            .rposition(|&id| self.tag_of(id) == Some(tag))
        {
            self.open_elements.truncate(idx);
        }
    }

    /// Pop elements until we pop the first heading element.
    pub(crate) fn close_to_first_heading(&mut self) {
        while let Some(id) = self.open_elements.pop() {
            if self.tag_of(id).is_some_and(is_heading) {
                return;
            }
        }
    }

    /// Close an end tag with no dedicated rule (WHATWG §13.2.6.4.7).
    ///
    /// A *special* tag (`</div>`, `</section>`, ...) closes its element
    /// only if it is in scope, so it cannot reach across a table cell.
    /// Any other tag walks the stack from the current node: the first
    /// element with the tag is popped with everything above it, but a
    /// special element met first means the end tag is ignored. Without
    /// these stops a stray `</div>` or `</span>` inside a table cell
    /// closed the cell, row and table around it.
    pub(crate) fn close_any_other_end_tag(&mut self, tag: &TagName) {
        if tag.is_special() {
            if self.has_in_scope(tag) {
                self.close_to_tag(tag);
            }
            return;
        }
        for idx in (0..self.open_elements.len()).rev() {
            match self.tag_of(self.open_elements[idx]) {
                Some(t) if t == tag => {
                    self.open_elements.truncate(idx);
                    return;
                },
                Some(t) if t.is_special() => return,
                _ => {},
            }
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

/// Whether `t` is a heading element (`h1`..`h6`).
fn is_heading(t: &TagName) -> bool {
    matches!(
        t,
        TagName::H1 | TagName::H2 | TagName::H3 | TagName::H4 | TagName::H5 | TagName::H6
    )
}
