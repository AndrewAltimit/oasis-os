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

    /// Foster-parent a token: insert before the table's parent
    /// (simplified).
    pub(crate) fn foster_parent(&mut self, token: Token) {
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
