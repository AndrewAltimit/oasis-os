//! Formatting element helpers and foster parenting.

use super::super::dom::{ElementData, NodeKind, TagName};
use super::super::tokenizer::Token;
use super::TreeBuilder;

impl TreeBuilder {
    // =============================================================
    // Formatting element helpers
    // =============================================================

    /// Close an `<a>` element from the active formatting list if one
    /// is present, before opening a new `<a>`.
    pub(crate) fn close_formatting_a_if_active(&mut self) {
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
    pub(crate) fn close_formatting_element(&mut self, tag: &TagName) {
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
    pub(crate) fn reconstruct_formatting(&mut self) {
        if self.active_formatting.is_empty() {
            return;
        }
        let to_reopen: Vec<super::super::dom::NodeId> = self
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

    /// Foster-parent a token. Per WHATWG HTML §13.2.6.1 ("appropriate
    /// place for inserting a node"), when foster parenting is enabled
    /// and the current insertion point would be inside a `<table>`,
    /// `<tbody>`, `<thead>`, `<tfoot>`, or `<tr>`, the new node is
    /// inserted **immediately before** the foster-parented `<table>`
    /// in the table's parent — not appended to that parent. Only when
    /// the table has no parent (which can only happen if the table is
    /// the document root, which we never construct) do we fall back to
    /// the previous open element on the stack.
    pub(crate) fn foster_parent(&mut self, token: Token) {
        let table_idx = self
            .open_elements
            .iter()
            .rposition(|&id| self.tag_of(id) == Some(&TagName::Table));

        // (foster_parent_id, optional reference_child_id)
        let (foster_target, before_ref) = if let Some(idx) = table_idx {
            let table_id = self.open_elements[idx];
            if let Some(parent_id) = self.doc.nodes[table_id].parent {
                (parent_id, Some(table_id))
            } else if idx > 0 {
                (self.open_elements[idx - 1], None)
            } else {
                (self.doc.root, None)
            }
        } else {
            (self.current_node(), None)
        };

        let insert = |doc: &mut super::super::dom::Document, child: super::super::dom::NodeId| {
            if let Some(reference) = before_ref {
                doc.insert_before(foster_target, child, reference);
            } else {
                doc.append_child(foster_target, child);
            }
        };

        match token {
            Token::Character(ref s) => {
                // Coalesce into the immediately-previous sibling (the
                // child that comes right before the reference) when
                // it's a text node; otherwise insert a fresh text node.
                let prev_text_id = {
                    let children = &self.doc.nodes[foster_target].children;
                    let target_pos = match before_ref {
                        Some(reference) => {
                            let pos = children.iter().position(|&id| id == reference);
                            debug_assert!(
                                pos.is_some(),
                                "foster parent: before_ref not in parent children"
                            );
                            pos
                        },
                        None => Some(children.len()),
                    };
                    target_pos.and_then(|pos| {
                        if pos == 0 {
                            None
                        } else {
                            Some(children[pos - 1])
                        }
                    })
                };
                if let Some(prev_id) = prev_text_id
                    && let NodeKind::Text(ref mut t) = self.doc.nodes[prev_id].kind
                {
                    t.push_str(s);
                    return;
                }
                let id = self.doc.add_node(NodeKind::Text(s.clone()));
                insert(&mut self.doc, id);
            },
            Token::Comment(text) => {
                let id = self.doc.add_node(NodeKind::Comment(text));
                insert(&mut self.doc, id);
            },
            Token::StartTag(ref tag) => {
                let id = self.create_element_from_start_tag(tag);
                insert(&mut self.doc, id);
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
