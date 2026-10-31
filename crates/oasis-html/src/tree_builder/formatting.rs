//! Formatting element helpers and foster parenting.

use super::super::dom::{ElementData, NodeId, NodeKind, TagName};
use super::super::tokenizer::Token;
use super::{MAX_CLONED_ATTRIBUTES, TreeBuilder};

impl TreeBuilder {
    // =============================================================
    // Formatting element helpers
    // =============================================================

    /// Start of the active formatting entries after the last marker.
    pub(crate) fn fmt_scope_start(&self) -> usize {
        self.fmt_markers
            .last()
            .copied()
            .unwrap_or(0)
            .min(self.active_formatting.len())
    }

    /// Push a marker onto the list of active formatting elements.
    pub(crate) fn insert_formatting_marker(&mut self) {
        self.fmt_markers.push(self.active_formatting.len());
    }

    /// Clear the list of active formatting elements up to the last
    /// marker (cell, caption, template, applet / object / marquee close).
    pub(crate) fn clear_formatting_to_last_marker(&mut self) {
        let keep = self.fmt_markers.pop().unwrap_or(0);
        self.active_formatting.truncate(keep);
    }

    /// The last active formatting element (after the last marker) that
    /// is the HTML element `tag`, as a list index.
    fn last_formatting_entry(&self, tag: &TagName) -> Option<usize> {
        let start = self.fmt_scope_start();
        self.active_formatting[start..]
            .iter()
            .rposition(|&id| self.tag_of(id) == Some(tag))
            .map(|i| start + i)
    }

    /// A new `<a>` while one is still active (after the last marker):
    /// run the adoption agency for it, then drop it from the list and
    /// the stack if it survived (WHATWG §13.2.6.4.7).
    pub(crate) fn close_formatting_a_if_active(&mut self) {
        let Some(fmt_idx) = self.last_formatting_entry(&TagName::A) else {
            return;
        };
        let node_id = self.active_formatting[fmt_idx];
        self.close_formatting_element(&TagName::A);
        if let Some(pos) = self.active_formatting.iter().rposition(|&id| id == node_id) {
            self.active_formatting.remove(pos);
        }
        if let Some(pos) = self.open_elements.iter().rposition(|&id| id == node_id) {
            self.open_elements.remove(pos);
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
            // Step 3: find the last formatting element with this tag
            // between the end of the list and the last marker.
            let Some(fmt_list_pos) = self.last_formatting_entry(subject) else {
                // No formatting entry — fall through to the generic
                // "any other end tag" path so misplaced end tags
                // still close their element.
                self.close_any_other_end_tag(subject);
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
            let Some(common_ancestor) = fmt_stack_pos.checked_sub(1).map(|i| self.open_elements[i])
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
                // continue the inner loop. `furthest_block_pos` would
                // go stale after this removal, but we only use
                // `furthest_block` as a NodeId past this point — its
                // index is never read again, so no recompute needed.
                let Some(nfp) = node_fmt_pos else {
                    self.open_elements.remove(node_stack_pos);
                    continue;
                };

                // Step 13.6: create a clone of node, replace the
                // entry in active formatting and open elements, then
                // let node be the clone.
                // Open elements are always element nodes; if that invariant
                // is ever broken, abandon the algorithm rather than panic on
                // untrusted markup. The tree is still well-formed here (every
                // move so far was a detach + append).
                let Some(clone) = self.clone_element(node) else {
                    log::warn!("adoption agency: non-element on open stack; aborting");
                    return;
                };
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
            let Some(fmt_clone) = self.clone_element(formatting_element) else {
                // Unreachable in practice (the entry matched `subject` by tag),
                // but degrade gracefully instead of panicking on hostile input.
                log::warn!("adoption agency: formatting entry is not an element; aborting");
                self.active_formatting.remove(fmt_list_pos);
                return;
            };

            // Step 16: take all of furthest block's children and
            // append them to the clone. Moved as one list: a detach per
            // child rescanned the shrinking list each time (quadratic
            // for a block with 90 000 children).
            let fb_children = std::mem::take(&mut self.doc.nodes[furthest_block].children);
            for &c in &fb_children {
                self.doc.nodes[c].parent = Some(fmt_clone);
            }
            self.doc.nodes[fmt_clone].children.extend(fb_children);

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

    /// Push a just-inserted formatting element onto the list of active
    /// formatting elements.
    ///
    /// - An element that could not be pushed onto the open-elements
    ///   stack (nesting cap reached) is not tracked: it can never be
    ///   "open", so tracking it would make every later
    ///   [`Self::reconstruct_formatting`] clone it again, blowing the
    ///   node count up quadratically on pathological input.
    /// - Noah's Ark clause (WHATWG §13.2.4.3): at most three entries
    ///   with the same tag and attributes; the earliest is dropped.
    pub(crate) fn push_active_formatting(&mut self, id: super::super::dom::NodeId) {
        if self.open_elements.last() != Some(&id) {
            return;
        }
        // Attribute sets compared order-independently via sorted refs:
        // `all(contains)` was O(attributes²) per list entry.
        fn sorted(e: &ElementData) -> Vec<(&str, &str)> {
            let mut v: Vec<(&str, &str)> = e
                .attributes
                .iter()
                .map(|a| (a.name.as_str(), a.value.as_str()))
                .collect();
            v.sort_unstable();
            v
        }
        let start = self.fmt_scope_start();
        if let Some(new) = self.doc.element(id) {
            let new_attrs = sorted(new);
            let matching: Vec<usize> = self
                .active_formatting
                .iter()
                .enumerate()
                .skip(start)
                .filter(|&(_, &fid)| {
                    self.doc.element(fid).is_some_and(|e| {
                        e.tag == new.tag
                            && e.attributes.len() == new.attributes.len()
                            && sorted(e) == new_attrs
                    })
                })
                .map(|(i, _)| i)
                .collect();
            if matching.len() >= 3 {
                self.active_formatting.remove(matching[0]);
            }
        }
        self.active_formatting.push(id);
    }

    /// Reconstruct the active formatting elements (WHATWG §13.2.4.3):
    /// reopen, in order, the entries after the last marker that follow
    /// the last entry still open.
    pub(crate) fn reconstruct_formatting(&mut self) {
        let start = self.fmt_scope_start();
        let len = self.active_formatting.len();
        if len == start
            || self
                .open_elements
                .contains(&self.active_formatting[len - 1])
        {
            return;
        }
        let mut first = len - 1;
        while first > start
            && !self
                .open_elements
                .contains(&self.active_formatting[first - 1])
        {
            first -= 1;
        }
        for pos in first..len {
            // At the nesting cap a clone would not be pushed onto the
            // stack either, and would just be cloned again next time.
            if self.open_elements.len() >= super::MAX_NESTING_DEPTH {
                break;
            }
            let Some(new_id) = self.clone_element(self.active_formatting[pos]) else {
                continue;
            };
            self.insert_element(new_id);
            self.active_formatting[pos] = new_id;
        }
    }

    /// Add a detached copy of element `id` (tag and attributes, no
    /// children), as the adoption agency and formatting reconstruction
    /// make. `None` if `id` is not an element.
    ///
    /// Clones share a document-wide budget of [`MAX_CLONED_ATTRIBUTES`]
    /// copied attributes; past it they are made without attributes.
    /// Reopening one `<b>` with 5 000 attributes after every `<p>` used
    /// to turn 37 KB of markup into ten million attributes.
    pub(crate) fn clone_element(&mut self, id: NodeId) -> Option<NodeId> {
        let data = self.doc.element(id)?;
        let mut new_data = ElementData::new(data.tag.clone());
        if self.cloned_attributes + data.attributes.len() <= MAX_CLONED_ATTRIBUTES {
            self.cloned_attributes += data.attributes.len();
            new_data.attributes = data.attributes.clone();
        }
        Some(self.doc.add_node(NodeKind::Element(new_data)))
    }

    // =============================================================
    // Foster parenting
    // =============================================================

    /// Process `token` with the in-body rules and foster parenting on
    /// (the "anything else" entry of "in table", WHATWG §13.2.6.4.9):
    /// nodes whose insertion target is a table element go just before
    /// the table instead (see `TreeBuilder::insertion_place`).
    pub(crate) fn foster_parent(&mut self, token: Token) {
        let was = std::mem::replace(&mut self.foster_parenting, true);
        self.handle_in_body(token);
        self.foster_parenting = was;
    }
}
