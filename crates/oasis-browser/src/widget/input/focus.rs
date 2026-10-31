//! Keyboard link selection, Tab focus traversal and focus-driven
//! restyling.

use super::*;

impl BrowserWidget {
    /// Select the next link in the link map.
    pub fn select_next_link(&mut self) {
        if self.link_map.is_empty() {
            return;
        }
        self.selected_link += 1;
        if self.selected_link >= self.link_map.len() as i32 {
            self.selected_link = 0;
        }
        self.scroll_to_selected_link();
    }

    /// Select the previous link in the link map.
    pub fn select_prev_link(&mut self) {
        if self.link_map.is_empty() {
            return;
        }
        self.selected_link -= 1;
        if self.selected_link < 0 {
            self.selected_link = self.link_map.len() as i32 - 1;
        }
        self.scroll_to_selected_link();
    }

    /// Scroll to make the currently selected link visible.
    pub(super) fn scroll_to_selected_link(&mut self) {
        if self.selected_link < 0 {
            return;
        }
        let idx = self.selected_link as usize;
        if idx < self.link_map.len() {
            let link = &self.link_map[idx];
            self.scroll
                .scroll_to_visible(link.rect.y as i32, link.rect.height as i32);
        }
    }

    /// Cycle tab focus forward through focusable elements (links).
    ///
    /// Wraps from the last element back to the first.
    pub(super) fn tab_focus_forward(&mut self) {
        if self.link_map.is_empty() {
            return;
        }
        self.selected_link += 1;
        if self.selected_link >= self.link_map.len() as i32 {
            self.selected_link = 0;
        }
        self.update_focused_node();
        self.scroll_to_selected_link();
    }

    /// Cycle tab focus backward through focusable elements (links).
    ///
    /// Wraps from the first element to the last.
    pub(super) fn tab_focus_backward(&mut self) {
        if self.link_map.is_empty() {
            return;
        }
        self.selected_link -= 1;
        if self.selected_link < 0 {
            self.selected_link = self.link_map.len() as i32 - 1;
        }
        self.update_focused_node();
        self.scroll_to_selected_link();
    }

    /// Update the `focused_node` to match the currently selected link,
    /// triggering a `:focus` restyle if the node changed.
    pub(super) fn update_focused_node(&mut self) {
        let new_focus = if self.selected_link >= 0 {
            let idx = self.selected_link as usize;
            self.link_map.get(idx).map(|link| link.node)
        } else {
            None
        };

        if new_focus != self.focused_node {
            let old_focus = self.focused_node;
            self.focused_node = new_focus;
            self.restyle_focus_affected(old_focus);
        }
    }

    /// Re-run the CSS cascade on focus-affected nodes only.
    ///
    /// Similar to `restyle_hover_affected`, but updates the `focused_node`
    /// in the cascade context so `:focus` / `:focus-visible` rules apply.
    pub(super) fn restyle_focus_affected(&mut self, old_focus: Option<NodeId>) {
        let Some(doc) = &self.document else { return };

        let affected = super::hover::state_change_affected(doc, old_focus, self.focused_node);
        if affected.is_empty() {
            return;
        }

        let ua_sheet = css::default::default_stylesheet();
        let mut all_sheets: Vec<&css::parser::Stylesheet> = vec![ua_sheet];
        all_sheets.extend(Self::merge_author_sheets(
            &self.cached_author_sheets,
            &self.cached_author_sheet_positions,
            &self.external_stylesheets,
            &self.external_stylesheet_positions,
        ));

        // Reuse cached selector index if available, otherwise build fresh.
        let fresh_index;
        let index = if let Some(ref cached) = self.cached_selector_index {
            cached
        } else {
            fresh_index = css::cascade::SelectorIndex::build(&all_sheets);
            &fresh_index
        };
        let inline_map: FxHashMap<NodeId, &[css::parser::Declaration]> = self
            .cached_inline_styles
            .iter()
            .map(|(nid, decls)| (*nid, decls.as_slice()))
            .collect();
        let ctx = css::cascade::CascadeContext {
            hover_node: self.hover_node,
            visited_urls: Some(&self.visited_urls),
            focused_node: self.focused_node,
            containers: self.container_lookup.as_ref(),
            global_layers: None,
        };

        let mut any_changed = false;
        let mut tag_cache = FxHashMap::<String, String>::default();
        for &nid in &affected {
            let node = &doc.nodes[nid];
            if !matches!(node.kind, html::dom::NodeKind::Element(_)) {
                continue;
            }
            let parent_style = node.parent.and_then(|pid| self.styles[pid].as_ref());
            let new_style = css::cascade::compute_style(
                doc,
                nid,
                parent_style,
                &all_sheets,
                index,
                &inline_map,
                &ctx,
                &mut tag_cache,
            );
            if self.styles[nid].as_ref() != Some(&new_style) {
                self.styles[nid] = Some(new_style);
                any_changed = true;
            }
        }

        if any_changed {
            self.layout_dirty = true;
        }
    }

    /// Activate the currently selected link.
    pub fn activate_selected_link(&mut self, vfs: &dyn Vfs) {
        if self.selected_link < 0 {
            return;
        }
        let idx = self.selected_link as usize;
        if idx < self.link_map.len() {
            let href = self.link_map[idx].href.clone();
            self.navigate_to(&href, vfs);
        }
    }
}
