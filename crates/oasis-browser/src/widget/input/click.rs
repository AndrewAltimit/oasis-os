//! Pointer clicks: links, form controls, `<label for>` and
//! `<details>` toggling.

use super::*;

impl BrowserWidget {
    /// Handle a pointer click at window-relative coordinates.
    pub fn handle_click(&mut self, x: i32, y: i32, vfs: &dyn Vfs) {
        let rel_y = y - self.window_y;
        let chrome_h = self.config.url_bar_height as i32;

        // Click in chrome area?
        if rel_y < chrome_h {
            let rel_x = x - self.window_x;
            let bw = self.config.button_width as i32;
            // Rightmost button group, left→right: bookmark then home.
            let home_x = self.window_w as i32 - bw;
            let bookmark_x = home_x - bw;

            if rel_x < bw {
                // Back button.
                self.focus = Focus::Content;
                self.url_selection_anchor = None;
                self.go_back(vfs);
            } else if rel_x < bw * 2 {
                // Forward button.
                self.focus = Focus::Content;
                self.url_selection_anchor = None;
                self.go_forward(vfs);
            } else if rel_x >= home_x {
                // Home button.
                self.focus = Focus::Content;
                self.url_selection_anchor = None;
                self.go_home(vfs);
            } else if rel_x >= bookmark_x {
                // Bookmark button: a quick left-click opens the
                // bookmarks listing (vfs://bookmarks, served by
                // NavigationController::bookmarks_page_html). A future
                // follow-up can add long-press / right-click to toggle
                // the current page, but just getting to the saved list
                // is the more useful affordance for now and mirrors
                // what Ctrl-Shift-O does on Firefox.
                self.focus = Focus::Content;
                self.url_selection_anchor = None;
                self.navigate_to("vfs://bookmarks", vfs);
            } else {
                // URL bar area -- enter edit mode and select the whole
                // URL so the next keystroke replaces it (Firefox/Chrome
                // address-bar behaviour). Users reported "hard to
                // highlight and replace text" — this is the fix.
                self.focus_url_bar();
            }
            return;
        }

        // Click in content area: leave URL bar editing.
        self.focus = Focus::Content;

        // Dispatch mousedown, mouseup, then click to JS.
        #[cfg(feature = "javascript")]
        let default_prevented = {
            self.dispatch_js_mouse_event(x, y, "mousedown");
            self.dispatch_js_mouse_event(x, y, "mouseup");
            let prevented = self.dispatch_js_click(x, y);
            // If any handler mutated the DOM, re-cascade + relayout so
            // collapse/expand, hidden/shown, and other class/style
            // toggles actually show on screen.
            self.apply_js_dom_mutations();
            prevented
        };
        #[cfg(not(feature = "javascript"))]
        let default_prevented = false;

        // Check link hit regions (unless JS prevented default navigation).
        if default_prevented {
            return;
        }
        for link in &self.link_map {
            let lx = link.rect.x;
            let ly = link.rect.y;
            let lw = link.rect.width;
            let lh = link.rect.height;
            if (x as f32) >= lx && (x as f32) < lx + lw && (y as f32) >= ly && (y as f32) < ly + lh
            {
                let href = link.href.clone();
                self.navigate_to(&href, vfs);
                return;
            }
        }

        // Handle <summary> click: toggle the parent <details> open state.
        self.handle_details_toggle(x, y);

        // Handle <label> clicks: focus / activate the labeled control.
        // A click that lands on the control itself is handled below
        // instead, so a checkbox inside its own label toggles once.
        if self.handle_label_for_click(x, y, vfs) {
            return;
        }

        // Handle direct clicks on <input> elements: focus text inputs,
        // fire the form submission for submit buttons, toggle
        // checkbox/radio state. Needed so Google's search box takes
        // focus when clicked directly (not just via a <label>).
        self.handle_form_element_click(x, y, vfs);
    }

    /// Convert a screen-space `(x, y)` click position into the layout
    /// tree's coordinate space.
    ///
    /// Paint shifts the layout tree by `window_origin + url_bar_height
    /// − scroll_offset` before drawing. `hit_test` expects raw layout
    /// coords, so every click-into-layout path has to undo that
    /// transform. Without this the hit rectangle of every element is
    /// off by the URL-bar height and clicks on inputs miss.
    pub(super) fn screen_to_layout(&self, screen_x: i32, screen_y: i32) -> (f32, f32) {
        let lx = (screen_x - self.window_x) as f32 + self.scroll.scroll_x as f32;
        let ly = (screen_y - self.window_y - self.config.url_bar_height as i32) as f32
            + self.scroll.scroll_y as f32;
        (lx, ly)
    }

    /// Handle clicks that fall on a form `<input>` / `<button>` element
    /// directly (i.e. outside any `<label for="...">` wrapper).
    ///
    /// Text-like inputs grab focus so subsequent keystrokes flow into
    /// them through `dispatch_form_key`. Submit buttons trigger form
    /// submission with the clicked button's `name`/`value` pair
    /// (propagated by `FormManager::submit_with_button`). Checkboxes
    /// and radios toggle / select.
    pub(super) fn handle_form_element_click(&mut self, x: i32, y: i32, vfs: &dyn Vfs) {
        use crate::html::dom::{NodeKind, TagName};

        // Translate from screen-space click coordinates into layout
        // space. The layout tree is built at (0, 0); paint shifts it
        // down by the URL bar height and offsets by the current scroll
        // position. hit_test expects layout-space coords.
        let (lx, ly) = self.screen_to_layout(x, y);
        let hit = self
            .layout_root
            .as_ref()
            .and_then(|root| root.hit_test(lx, ly));
        let Some(doc) = &self.document else { return };

        // Walk up to the nearest form-control ancestor — the click may
        // land on a `<span>` wrapper like Google's
        // `<span class="lsbb"><input ...>`.
        let mut form_elem_nid = None;
        let mut cur = hit;
        while let Some(id) = cur {
            if let NodeKind::Element(ref e) = doc.nodes[id].kind
                && matches!(
                    e.tag,
                    TagName::Input | TagName::Button | TagName::Textarea | TagName::Select
                )
            {
                form_elem_nid = Some(id);
                break;
            }
            cur = doc.nodes[id].parent;
        }
        let Some(target_nid) = form_elem_nid else {
            // Clicking anywhere else blurs the focused control, so typed
            // keys and Enter go back to page shortcuts / links.
            if self.form_manager.focused_element.is_some() {
                self.form_manager.focused_element = None;
                self.form_manager.focused_form = None;
                self.layout_dirty = true;
            }
            return;
        };
        self.activate_form_control(target_nid, vfs);
    }

    /// Activate the form control `target_nid` as if the user clicked it:
    /// focus it, toggle checkboxes, select radios, and submit / reset the
    /// owning form for buttons.
    pub(super) fn activate_form_control(&mut self, target_nid: NodeId, vfs: &dyn Vfs) {
        use crate::html::dom::{NodeKind, TagName};

        let Some(doc) = &self.document else { return };

        let (tag, input_type, value, name_or_id, name_attr) = match &doc.nodes[target_nid].kind {
            NodeKind::Element(elem) => (
                elem.tag.clone(),
                elem.get_attribute("type")
                    .unwrap_or("text")
                    .to_ascii_lowercase(),
                elem.get_attribute("value").map(str::to_string),
                elem.get_attribute("name")
                    .or_else(|| elem.get_attribute("id"))
                    .map(|s| s.to_string()),
                elem.get_attribute("name").unwrap_or("").to_string(),
            ),
            _ => return,
        };
        // `name_or_id` is optional: anonymous reset/submit buttons
        // (no `name`/`id`) still need to fire their form action. Only
        // the radio and focused-element-tracking paths require a name.
        let name = name_or_id;

        // Focus tracking: mirror the hover/:focus pseudo-class state
        // even when the form manager doesn't own this element.
        self.focused_node = Some(target_nid);

        let is_submit = matches!(input_type.as_str(), "submit" | "image")
            || (tag == TagName::Button && !matches!(input_type.as_str(), "button" | "reset"));

        // Resolve the owning form by DOM ancestry rather than by
        // element name. Name-based matching fails for
        // `FormElement::ResetButton` (no `name` field — `has_element`
        // only matches the sentinel `"__reset__"`), for anonymous
        // submit buttons, and for named elements shared across
        // sibling forms (position returns the first match, not the
        // actual owner). `populate_forms_from_dom` registers forms
        // in document order, so the N-th `<form>` node corresponds
        // to `form_manager.forms[N]`.
        let fi_owning = {
            let mut form_ancestor = None;
            let mut cur = Some(target_nid);
            while let Some(id) = cur {
                if let NodeKind::Element(ref e) = doc.nodes[id].kind
                    && e.tag == TagName::Form
                {
                    form_ancestor = Some(id);
                    break;
                }
                cur = doc.nodes[id].parent;
            }
            form_ancestor.and_then(|target| {
                doc.nodes
                    .iter()
                    .enumerate()
                    .filter(
                        |(_, n)| matches!(&n.kind, NodeKind::Element(e) if e.tag == TagName::Form),
                    )
                    .position(|(nid, _)| nid == target)
            })
        };

        if let Some(fi) = fi_owning {
            match name {
                // Caret at the end of the value, like a browser.
                Some(ref n) => self.form_manager.focus_element(fi, n),
                None => self.form_manager.focused_form = Some(fi),
            }
        }

        let is_reset = input_type == "reset";

        match input_type.as_str() {
            "checkbox" if tag == TagName::Input => {
                if let (Some(fi), Some(n)) = (fi_owning, name.as_deref()) {
                    // Match on name *and* value: checkbox groups share a
                    // name (`tags=a`, `tags=b`).
                    let v = value.as_deref().unwrap_or("on");
                    self.form_manager.toggle_checkbox_value(fi, n, v);
                    self.sync_form_values_to_dom();
                }
            },
            "radio" if tag == TagName::Input => {
                if let (Some(fi), Some(n)) = (fi_owning, name.as_deref()) {
                    let v = value.as_deref().unwrap_or("on");
                    self.form_manager.select_radio(fi, n, v);
                    self.sync_form_values_to_dom();
                }
            },
            _ if is_submit => {
                let submitter_value = value.unwrap_or_default();
                if let Some(fi) = fi_owning
                    && let Some(data) = self
                        .form_manager
                        .submit_with_submitter(fi, Some((&name_attr, &submitter_value)))
                {
                    self.handle_form_submit(&data, vfs);
                }
            },
            _ if is_reset => {
                if let Some(fi) = fi_owning {
                    self.form_manager.reset(fi);
                    self.sync_form_values_to_dom();
                }
            },
            _ => {
                // Plain text input — focus is enough. Typed characters
                // now flow through `handle_input`'s focused-element
                // routing.
                self.layout_dirty = true;
            },
        }
    }

    /// If the click hits a `<label>`, activate its labeled control: the
    /// element named by `for`, or else the first form control inside
    /// the label. Returns `true` when the click was consumed.
    ///
    /// Clicks that land on a form control nested inside the label are
    /// left to [`Self::handle_form_element_click`] (returning `false`),
    /// so the control is activated exactly once.
    pub(super) fn handle_label_for_click(&mut self, x: i32, y: i32, vfs: &dyn Vfs) -> bool {
        use crate::html::dom::{NodeKind, TagName};

        let is_control = |tag: &TagName| {
            matches!(
                tag,
                TagName::Input | TagName::Button | TagName::Select | TagName::Textarea
            )
        };

        let (lx, ly) = self.screen_to_layout(x, y);
        let node_id = self
            .layout_root
            .as_ref()
            .and_then(|root| root.hit_test(lx, ly));

        let Some(nid) = node_id else { return false };
        let Some(doc) = &self.document else {
            return false;
        };

        // Walk up from the hit node to find a <label> ancestor, bailing
        // out if the click is on a control of its own.
        let mut label = None;
        let mut cur = Some(nid);
        while let Some(id) = cur {
            if let NodeKind::Element(ref elem) = doc.nodes[id].kind {
                if is_control(&elem.tag) {
                    return false;
                }
                if elem.tag == TagName::Label {
                    label = Some(id);
                    break;
                }
            }
            cur = doc.nodes[id].parent;
        }
        let Some(label_nid) = label else { return false };

        let target = match doc.element(label_nid).and_then(|e| e.get_attribute("for")) {
            Some(for_id) => doc.get_element_by_id(for_id),
            None => {
                // Implicit association: first labelable descendant.
                let mut stack: Vec<usize> = doc.nodes[label_nid]
                    .children
                    .iter()
                    .rev()
                    .copied()
                    .collect();
                let mut found = None;
                while let Some(id) = stack.pop() {
                    if let NodeKind::Element(ref e) = doc.nodes[id].kind
                        && is_control(&e.tag)
                        && e.get_attribute("type") != Some("hidden")
                    {
                        found = Some(id);
                        break;
                    }
                    stack.extend(doc.nodes[id].children.iter().rev().copied());
                }
                found
            },
        };
        let Some(target_nid) = target else {
            return false;
        };
        let is_labelable = doc.element(target_nid).is_some_and(|e| is_control(&e.tag));
        if !is_labelable {
            return false;
        }
        self.activate_form_control(target_nid, vfs);
        true
    }

    /// If the click hits a `<summary>` element, toggle the `open`
    /// attribute on its parent `<details>`.
    pub(super) fn handle_details_toggle(&mut self, x: i32, y: i32) {
        use crate::html::dom::{NodeKind, TagName};

        let (lx, ly) = self.screen_to_layout(x, y);
        let node_id = self
            .layout_root
            .as_ref()
            .and_then(|root| root.hit_test(lx, ly));

        let Some(nid) = node_id else { return };
        let Some(doc) = &mut self.document else {
            return;
        };

        // Walk up from the hit node to find a <summary> ancestor.
        let mut summary_nid = None;
        let mut cur = Some(nid);
        while let Some(id) = cur {
            if let NodeKind::Element(ref elem) = doc.nodes[id].kind
                && elem.tag == TagName::Summary
            {
                summary_nid = Some(id);
                break;
            }
            cur = doc.nodes[id].parent;
        }

        let Some(summary_id) = summary_nid else {
            return;
        };

        // Find the parent <details> element.
        let Some(parent_id) = doc.nodes[summary_id].parent else {
            return;
        };
        let is_details = matches!(
            doc.nodes[parent_id].kind,
            NodeKind::Element(ref e) if e.tag == TagName::Details
        );
        if !is_details {
            return;
        }

        // Toggle the `open` attribute.
        let has_open = doc
            .element(parent_id)
            .is_some_and(|e| e.get_attribute("open").is_some());
        if let NodeKind::Element(ref mut elem) = doc.nodes[parent_id].kind {
            if has_open {
                elem.remove_attribute("open");
            } else {
                elem.set_attribute("open", "");
            }
        }

        // Mark layout as dirty so the page re-renders.
        self.layout_dirty = true;
    }
}
