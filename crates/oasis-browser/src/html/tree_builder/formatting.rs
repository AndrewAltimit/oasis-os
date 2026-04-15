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

    /// Run the adoption agency algorithm for the given end-tag subject.
    /// Implements WHATWG HTML §13.2.6.4.7 ("in body" insertion mode,
    /// "any other end tag" formatting-element branch).
    ///
    /// This replaces the previous simplified version which just
    /// truncated the open elements stack at the formatting element.
    /// The full algorithm reparents the "furthest block" under a clone
    /// of the formatting element, which is what lets adversarial
    /// markup like `<b><p></b></p>` round-trip through the tree in a
    /// way that matches real browsers.
    pub(crate) fn close_formatting_element(&mut self, subject: &TagName) {
        use super::super::dom::{ElementData, NodeKind};

        // §13.2.6.4.7 step 1: if the current node is an HTML element
        // whose tag name is subject, and the current node is not in
        // the list of active formatting elements, then pop the
        // current node off the stack of open elements and return.
        if let Some(&cur) = self.open_elements.last()
            && self.tag_of(cur) == Some(subject)
            && !self.active_formatting.contains(&cur)
        {
            self.open_elements.pop();
            return;
        }

        // Step 2: outer loop, at most 8 iterations.
        for _outer in 0..8 {
            // Step 3: find the last formatting element with this tag.
            // We don't model markers; the whole list is one scope.
            let fmt_list_pos = self
                .active_formatting
                .iter()
                .rposition(|&id| self.tag_of(id) == Some(subject));
            let Some(fmt_list_pos) = fmt_list_pos else {
                // No formatting entry — fall through to the generic
                // "any other end tag" path so misplaced end tags
                // still close their element.
                self.close_to_tag_any_scope(subject);
                return;
            };
            let formatting_element = self.active_formatting[fmt_list_pos];

            // Step 4: if formatting element is not in the open
            // elements stack, remove it from the active formatting
            // list and return (parse error, but recoverable).
            let Some(fmt_stack_pos) = self
                .open_elements
                .iter()
                .rposition(|&id| id == formatting_element)
            else {
                self.active_formatting.remove(fmt_list_pos);
                return;
            };

            // Step 5: if formatting element is in open elements but
            // not in scope, parse error; ignore and return.
            if !self.has_in_scope(subject) {
                log::trace!("adoption agency: formatting element not in scope");
                return;
            }

            // Step 6: if formatting element is not the current node,
            // that's a parse error; continue anyway.

            // Step 7: let furthest block be the topmost (deepest in
            // the stack) node below formatting element that is in
            // the special category.
            let furthest_block_pos = self.open_elements[fmt_stack_pos + 1..]
                .iter()
                .position(|&id| self.tag_of(id).map(|t| t.is_special()).unwrap_or(false))
                .map(|off| fmt_stack_pos + 1 + off);

            // Step 8: if there is no furthest block, pop everything
            // from the stack from the current node up to and
            // including formatting element, and drop it from the
            // active formatting list.
            let Some(furthest_block_pos) = furthest_block_pos else {
                self.open_elements.truncate(fmt_stack_pos);
                self.active_formatting.remove(fmt_list_pos);
                return;
            };
            let furthest_block = self.open_elements[furthest_block_pos];

            // Step 9: common ancestor is the element immediately above
            // formatting element on the open stack.
            let Some(common_ancestor) = fmt_stack_pos
                .checked_sub(1)
                .map(|i| self.open_elements[i])
            else {
                return;
            };

            // Step 10: bookmark is the position of formatting element
            // in the active formatting list.
            let mut bookmark = fmt_list_pos;

            // Step 11-12: node / last_node chain rebuild.
            let mut node_stack_pos = furthest_block_pos;
            let mut last_node = furthest_block;
            for inner in 1..=64 {
                node_stack_pos -= 1;
                let node = self.open_elements[node_stack_pos];

                // Step 13.3: if node is formatting element, stop.
                if node == formatting_element {
                    break;
                }

                // Step 13.4: if inner counter > 3 and node is in the
                // active formatting list, remove it from the list.
                let mut node_fmt_pos = self.active_formatting.iter().position(|&id| id == node);
                if inner > 3
                    && let Some(nfp) = node_fmt_pos
                {
                    self.active_formatting.remove(nfp);
                    if bookmark > nfp {
                        bookmark -= 1;
                    }
                    node_fmt_pos = None;
                }

                // Step 13.5: if node is not in active formatting,
                // remove it from the open elements stack and
                // continue the inner loop.
                let Some(nfp) = node_fmt_pos else {
                    self.open_elements.remove(node_stack_pos);
                    if node_stack_pos <= furthest_block_pos {
                        // Recompute last-node position tracking not
                        // needed — we only use furthest_block below.
                    }
                    continue;
                };

                // Step 13.6: create a clone of node, replace the
                // entry in active formatting and open elements, then
                // let node be the clone.
                let (tag, attrs) = {
                    let data = self
                        .doc
                        .element(node)
                        .expect("open element must be element");
                    (data.tag.clone(), data.attributes.clone())
                };
                let mut new_data = ElementData::new(tag);
                new_data.attributes = attrs;
                let clone = self.doc.add_node(NodeKind::Element(new_data));
                self.active_formatting[nfp] = clone;
                self.open_elements[node_stack_pos] = clone;

                // Step 13.7: if last node is furthest block, move the
                // bookmark to the position right after the clone in
                // the formatting list.
                if last_node == furthest_block {
                    bookmark = nfp + 1;
                }

                // Step 13.8: append last node to the clone. Detach
                // first so we don't end up in two children lists.
                self.doc.detach_node(last_node);
                self.doc.append_child(clone, last_node);

                // Step 13.9: let last node be the clone.
                last_node = clone;
            }

            // Step 14: insert last_node at the "appropriate place for
            // inserting a node" with common_ancestor as the override
            // target. For the common case that's just append_child.
            self.doc.detach_node(last_node);
            self.doc.append_child(common_ancestor, last_node);

            // Step 15: create a clone of formatting element.
            let (fmt_tag, fmt_attrs) = {
                let data = self
                    .doc
                    .element(formatting_element)
                    .expect("formatting element must be element");
                (data.tag.clone(), data.attributes.clone())
            };
            let mut new_fmt = ElementData::new(fmt_tag);
            new_fmt.attributes = fmt_attrs;
            let fmt_clone = self.doc.add_node(NodeKind::Element(new_fmt));

            // Step 16: take all of furthest block's children and
            // append them to the clone.
            let fb_children: Vec<_> = self.doc.nodes[furthest_block].children.clone();
            for c in fb_children {
                self.doc.detach_node(c);
                self.doc.append_child(fmt_clone, c);
            }

            // Step 17: append the clone to furthest block.
            self.doc.append_child(furthest_block, fmt_clone);

            // Step 18: remove formatting element from active
            // formatting list, insert the clone at the bookmark.
            self.active_formatting.remove(fmt_list_pos);
            if bookmark > fmt_list_pos {
                bookmark -= 1;
            }
            let insert_at = bookmark.min(self.active_formatting.len());
            self.active_formatting.insert(insert_at, fmt_clone);

            // Step 19: remove formatting element from open elements
            // and insert the clone immediately after furthest block.
            if let Some(pos) = self
                .open_elements
                .iter()
                .position(|&id| id == formatting_element)
            {
                self.open_elements.remove(pos);
            }
            if let Some(fb_pos) = self
                .open_elements
                .iter()
                .position(|&id| id == furthest_block)
            {
                self.open_elements.insert(fb_pos + 1, fmt_clone);
            }
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
