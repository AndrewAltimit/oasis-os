//! Element creation, insertion location, stack, scope, and auto-close
//! helpers.

use super::super::dom::{Attribute as DomAttribute, ElementData, NodeId, NodeKind, TagName};
use super::super::tokenizer::{Attribute as TokAttribute, ContentState, StartTagToken};
use super::{InsertionMode, MAX_NESTING_DEPTH, Ns, TreeBuilder};

/// Which "has an element in ... scope" variant (WHATWG §13.2.4.2).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Default,
    ListItem,
    Button,
    Table,
    Select,
}

impl TreeBuilder {
    // =============================================================
    // Element creation helpers
    // =============================================================

    /// Create an HTML element node from a start tag token, and note the
    /// tokenizer state it switches to (`<script>`, `<textarea>`, ...).
    pub(crate) fn create_element_from_start_tag(&mut self, tag: &StartTagToken) -> NodeId {
        let lower = tag.name.to_ascii_lowercase();
        self.pending_content_state = ContentState::for_html_element(&lower, self.scripting);
        let mut data = ElementData::new(TagName::from_str(&lower));
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

    /// Add the attributes of a stray `<html>` / `<body>` start tag that
    /// `id` does not have yet (WHATWG §13.2.6.4.7).
    ///
    /// The element keeps at most `MAX_ATTRIBUTES_PER_TAG` attributes, as a
    /// single tag does: repeated `<body a0 .. a255>` tags would otherwise
    /// pile up attributes without bound (and each merge scans them all).
    pub(crate) fn merge_attributes(&mut self, id: NodeId, attrs: &[TokAttribute]) {
        use crate::tokenizer::MAX_ATTRIBUTES_PER_TAG;
        let mut new_id = None;
        if let NodeKind::Element(ref mut data) = self.doc.nodes[id].kind {
            for attr in attrs {
                if data.attributes.len() >= MAX_ATTRIBUTES_PER_TAG {
                    break;
                }
                if data.get_attribute(&attr.name).is_none() {
                    if attr.name == "id" {
                        new_id = Some(attr.value.clone());
                    }
                    data.attributes.push(DomAttribute {
                        name: attr.name.clone(),
                        value: attr.value.clone(),
                    });
                }
            }
        }
        if let Some(value) = new_id {
            self.doc.update_id_index(id, None, Some(&value));
        }
    }

    // =============================================================
    // Namespaces
    // =============================================================

    /// The namespace (and integration-point kind) of `id`.
    pub(crate) fn ns_of(&self, id: NodeId) -> Ns {
        self.node_ns.get(id).copied().unwrap_or_default()
    }

    pub(crate) fn set_ns(&mut self, id: NodeId, ns: Ns) {
        if ns == Ns::Html && id >= self.node_ns.len() {
            return;
        }
        if id >= self.node_ns.len() {
            self.node_ns.resize(id + 1, Ns::Html);
        }
        self.node_ns[id] = ns;
    }

    /// Whether the adjusted current node is an SVG / MathML element
    /// (enables CDATA sections in the tokenizer).
    pub(crate) fn adjusted_current_node_is_foreign(&self) -> bool {
        self.open_elements
            .last()
            .is_some_and(|&id| self.ns_of(id).is_foreign())
    }

    /// Whether `id` is the HTML element `name`.
    pub(crate) fn is_html_named(&self, id: NodeId, name: &str) -> bool {
        self.ns_of(id) == Ns::Html && self.tag_of(id).is_some_and(|t| t.as_str() == name)
    }

    /// Whether the current node is the HTML element `name`.
    pub(crate) fn current_is(&self, name: &str) -> bool {
        self.open_elements
            .last()
            .is_some_and(|&id| self.is_html_named(id, name))
    }

    /// Whether the current node is one of the HTML elements `names`.
    pub(crate) fn current_is_any(&self, names: &[&str]) -> bool {
        names.iter().any(|n| self.current_is(n))
    }

    // =============================================================
    // Insertion
    // =============================================================

    /// The appropriate place for inserting a node (WHATWG §13.2.6.1):
    /// `(parent, insert_before)`. With foster parenting on and a table
    /// element as the target, that is just before the last open table.
    fn insertion_place(&self) -> (NodeId, Option<NodeId>) {
        let target = self.current_node();
        let table_like = self.ns_of(target) == Ns::Html
            && matches!(
                self.tag_of(target),
                Some(
                    TagName::Table | TagName::Tbody | TagName::Tfoot | TagName::Thead | TagName::Tr
                )
            );
        if !self.foster_parenting || !table_like {
            return (target, None);
        }
        let last_table = self
            .open_elements
            .iter()
            .rposition(|&id| self.is_html_named(id, "table"));
        let last_template = self
            .open_elements
            .iter()
            .rposition(|&id| self.is_html_named(id, "template"));
        match last_table {
            Some(t) if last_template.is_none_or(|tp| tp < t) => {
                let table = self.open_elements[t];
                match self.doc.nodes[table].parent {
                    Some(parent) => (parent, Some(table)),
                    None => (self.open_elements[t.saturating_sub(1)], None),
                }
            },
            Some(_) | None => match last_template {
                Some(tp) => (self.open_elements[tp], None),
                None => (
                    self.open_elements.first().copied().unwrap_or(self.doc.root),
                    None,
                ),
            },
        }
    }

    /// Insert `id` at the appropriate place without pushing it.
    pub(crate) fn insert_leaf(&mut self, id: NodeId) {
        match self.insertion_place() {
            (parent, Some(before)) => self.doc.insert_before(parent, id, before),
            (parent, None) => self.doc.append_child(parent, id),
        }
    }

    /// Insert an element at the appropriate place and push it onto the
    /// open elements stack.
    ///
    /// When the stack has reached [`MAX_NESTING_DEPTH`], the element
    /// is still inserted but is **not** pushed onto the stack. This
    /// means subsequent content will be attached to the current parent
    /// rather than nesting deeper, preventing stack exhaustion from
    /// pathologically deep HTML.
    pub(crate) fn insert_element(&mut self, id: NodeId) {
        self.insert_leaf(id);
        self.push_open(id);
    }

    /// Insert a comment at the appropriate place.
    pub(crate) fn insert_comment(&mut self, text: String) {
        let id = self.doc.add_node(NodeKind::Comment(text));
        self.insert_leaf(id);
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

    /// Insert text at the appropriate place, coalescing into an adjacent
    /// text node when possible.
    pub(crate) fn insert_text(&mut self, text: &str) {
        match self.insertion_place() {
            (parent, None) => self.append_text_to(parent, text),
            (parent, Some(before)) => {
                if text.is_empty() {
                    return;
                }
                let children = &self.doc.nodes[parent].children;
                let prev = children
                    .iter()
                    .position(|&c| c == before)
                    .and_then(|pos| pos.checked_sub(1))
                    .map(|pos| children[pos]);
                if let Some(prev) = prev
                    && let NodeKind::Text(ref mut existing) = self.doc.nodes[prev].kind
                {
                    existing.push_str(text);
                    return;
                }
                let id = self.doc.add_node(NodeKind::Text(text.to_string()));
                self.doc.insert_before(parent, id, before);
            },
        }
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
        let cur = self.current_node();
        self.ns_of(cur) == Ns::Html && self.tag_of(cur).is_some_and(is_heading)
    }

    // =============================================================
    // Scope helpers
    // =============================================================

    /// Walk the open elements from the current node: `true` at the
    /// first HTML element `target` accepts, `false` at the first
    /// boundary of `scope` (WHATWG §13.2.4.2 "has an element in the
    /// specific scope").
    pub(crate) fn in_scope_by(&self, target: impl Fn(&TagName) -> bool, scope: Scope) -> bool {
        for &id in self.open_elements.iter().rev() {
            let Some(t) = self.tag_of(id) else { continue };
            let ns = self.ns_of(id);
            if ns == Ns::Html && target(t) {
                return true;
            }
            let boundary = if ns != Ns::Html {
                matches!(scope, Scope::Default | Scope::ListItem | Scope::Button)
                    && ns.bounds_scope()
            } else {
                match scope {
                    Scope::Default => is_default_scope_boundary(t),
                    Scope::ListItem => {
                        is_default_scope_boundary(t) || matches!(t, TagName::Ol | TagName::Ul)
                    },
                    Scope::Button => is_default_scope_boundary(t) || *t == TagName::Button,
                    Scope::Table => {
                        matches!(t, TagName::Html | TagName::Table | TagName::Template)
                    },
                    Scope::Select => !matches!(t.as_str(), "optgroup" | "option"),
                }
            };
            if boundary {
                return false;
            }
        }
        false
    }

    /// Check if an element with the given tag is in (default) scope.
    pub(crate) fn has_in_scope(&self, tag: &TagName) -> bool {
        self.in_scope_by(|t| t == tag, Scope::Default)
    }

    /// List-item scope: the default boundaries plus `ol` / `ul`.
    pub(crate) fn has_in_list_scope(&self, tag: &TagName) -> bool {
        self.in_scope_by(|t| t == tag, Scope::ListItem)
    }

    /// Button scope: the default boundaries plus `button`.
    pub(crate) fn has_in_button_scope(&self, tag: &TagName) -> bool {
        self.in_scope_by(|t| t == tag, Scope::Button)
    }

    /// Table scope: only `html`, `table` and `template` bound it.
    pub(crate) fn has_in_table_scope(&self, tag: &TagName) -> bool {
        self.in_scope_by(|t| t == tag, Scope::Table)
    }

    /// Select scope: everything but `optgroup` / `option` bounds it.
    pub(crate) fn has_in_select_scope(&self, tag: &TagName) -> bool {
        self.in_scope_by(|t| t == tag, Scope::Select)
    }

    /// Whether any heading (`h1`..`h6`) is in default scope.
    pub(crate) fn has_heading_in_scope(&self) -> bool {
        self.in_scope_by(is_heading, Scope::Default)
    }

    // =============================================================
    // Auto-close helpers
    // =============================================================

    /// "Close a p element" if a `<p>` is in button scope.
    pub(crate) fn close_p_if_in_scope(&mut self) {
        if self.has_in_button_scope(&TagName::P) {
            self.generate_implied_end_tags(Some("p"));
            self.close_to_tag(&TagName::P);
        }
    }

    /// Generate implied end tags (WHATWG §13.2.6.3), except for `except`.
    pub(crate) fn generate_implied_end_tags(&mut self, except: Option<&str>) {
        while let Some(&cur) = self.open_elements.last() {
            if self.ns_of(cur) != Ns::Html {
                return;
            }
            let Some(name) = self.tag_of(cur).map(TagName::as_str) else {
                return;
            };
            let implied = matches!(
                name,
                "dd" | "dt" | "li" | "optgroup" | "option" | "p" | "rb" | "rp" | "rt" | "rtc"
            );
            if !implied || except == Some(name) {
                return;
            }
            self.open_elements.pop();
        }
    }

    /// Pop elements from the stack through the topmost HTML element with
    /// the given tag. Does nothing if no such element is open.
    ///
    /// Regression: this popped until it found the tag, so when it was
    /// not there (an implied `<p>` that hit the nesting cap, `</table>`
    /// with no table) the whole stack went and later content landed
    /// outside `<html>`.
    pub(crate) fn close_to_tag(&mut self, tag: &TagName) {
        if let Some(idx) = self
            .open_elements
            .iter()
            .rposition(|&id| self.ns_of(id) == Ns::Html && self.tag_of(id) == Some(tag))
        {
            self.open_elements.truncate(idx);
        }
    }

    /// Pop elements until we pop the first heading element.
    pub(crate) fn close_to_first_heading(&mut self) {
        while let Some(id) = self.open_elements.pop() {
            if self.ns_of(id) == Ns::Html && self.tag_of(id).is_some_and(is_heading) {
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
                self.generate_implied_end_tags(Some(tag.as_str()));
                self.close_to_tag(tag);
            }
            return;
        }
        for idx in (0..self.open_elements.len()).rev() {
            let id = self.open_elements[idx];
            let html = self.ns_of(id) == Ns::Html;
            match self.tag_of(id) {
                Some(t) if html && t == tag => {
                    self.generate_implied_end_tags(Some(tag.as_str()));
                    self.open_elements.truncate(idx);
                    return;
                },
                Some(t) if t.is_special() || self.ns_of(id).bounds_scope() => return,
                _ => {},
            }
        }
    }

    /// Pop until the current node is one of the HTML elements `names`
    /// (or `html`): "clear the stack back to a table / table body / table
    /// row context".
    pub(crate) fn clear_stack_back_to(&mut self, names: &[&str]) {
        while let Some(&cur) = self.open_elements.last() {
            if self.is_html_named(cur, "html") || names.iter().any(|n| self.is_html_named(cur, n)) {
                return;
            }
            self.open_elements.pop();
        }
    }

    /// Reset the insertion mode appropriately (WHATWG §13.2.4.1).
    pub(crate) fn reset_insertion_mode(&mut self) {
        self.mode = self.appropriate_mode();
    }

    fn appropriate_mode(&self) -> InsertionMode {
        for (i, &id) in self.open_elements.iter().enumerate().rev() {
            let last = i == 0;
            if self.ns_of(id) != Ns::Html {
                continue;
            }
            let Some(name) = self.tag_of(id).map(TagName::as_str) else {
                continue;
            };
            match name {
                "select" => {
                    for &anc in self.open_elements[..i].iter().rev() {
                        if self.is_html_named(anc, "template") {
                            break;
                        }
                        if self.is_html_named(anc, "table") {
                            return InsertionMode::InSelectInTable;
                        }
                    }
                    return InsertionMode::InSelect;
                },
                "td" | "th" if !last => return InsertionMode::InCell,
                "tr" => return InsertionMode::InRow,
                "tbody" | "thead" | "tfoot" => return InsertionMode::InTableBody,
                "caption" => return InsertionMode::InCaption,
                "colgroup" => return InsertionMode::InColumnGroup,
                "table" => return InsertionMode::InTable,
                // Template contents parse in "in body" here.
                "template" => return InsertionMode::InBody,
                "head" if !last => return InsertionMode::InHead,
                "body" => return InsertionMode::InBody,
                "html" => {
                    return if self.head_element.is_none() {
                        InsertionMode::BeforeHead
                    } else {
                        InsertionMode::AfterHead
                    };
                },
                _ => {},
            }
        }
        InsertionMode::InBody
    }
}

/// The default scope's HTML boundaries. They include `template`, so
/// scope queries cannot walk across a template into its enclosing tree.
fn is_default_scope_boundary(t: &TagName) -> bool {
    matches!(
        t,
        TagName::Html
            | TagName::Table
            | TagName::Td
            | TagName::Th
            | TagName::Caption
            | TagName::Template
    ) || matches!(t.as_str(), "applet" | "marquee" | "object")
}

/// Whether `t` is a heading element (`h1`..`h6`).
fn is_heading(t: &TagName) -> bool {
    matches!(
        t,
        TagName::H1 | TagName::H2 | TagName::H3 | TagName::H4 | TagName::H5 | TagName::H6
    )
}
