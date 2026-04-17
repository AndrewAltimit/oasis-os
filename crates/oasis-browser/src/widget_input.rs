//! Input handling methods for [`BrowserWidget`].

use oasis_types::input::{Button, InputEvent, Trigger};
use oasis_vfs::Vfs;
use rustc_hash::FxHashMap;

use crate::css;
use crate::css::transition::TransitionEngine;
use crate::css::values::ComputedStyle;
use crate::css::values::types::Transition;
use crate::html;
use crate::html::dom::NodeId;
use crate::layout::box_model::LayoutBox;
use crate::loader::Url;
use crate::{BrowserWidget, Focus};

/// Compare two computed styles for geometry-affecting properties only.
///
/// Returns `true` if only visual properties (color, background-color,
/// opacity, text-decoration, outline, box-shadow, text-shadow) differ.
/// When this returns `true`, a relayout can be skipped -- only a
/// repaint is needed.
pub(crate) fn styles_geometry_equal(a: &ComputedStyle, b: &ComputedStyle) -> bool {
    // Display / positioning
    a.display == b.display
        && a.position == b.position
        && a.float == b.float
        // Box model
        && (a.margin_top - b.margin_top).abs() < f32::EPSILON
        && (a.margin_right - b.margin_right).abs() < f32::EPSILON
        && (a.margin_bottom - b.margin_bottom).abs() < f32::EPSILON
        && (a.margin_left - b.margin_left).abs() < f32::EPSILON
        && (a.padding_top - b.padding_top).abs() < f32::EPSILON
        && (a.padding_right - b.padding_right).abs() < f32::EPSILON
        && (a.padding_bottom - b.padding_bottom).abs() < f32::EPSILON
        && (a.padding_left - b.padding_left).abs() < f32::EPSILON
        && (a.border_top_width - b.border_top_width).abs() < f32::EPSILON
        && (a.border_right_width - b.border_right_width).abs() < f32::EPSILON
        && (a.border_bottom_width - b.border_bottom_width).abs() < f32::EPSILON
        && (a.border_left_width - b.border_left_width).abs() < f32::EPSILON
        // Dimensions
        && a.width == b.width
        && a.height == b.height
        && a.max_width == b.max_width
        && a.min_width == b.min_width
        && a.max_height == b.max_height
        && a.min_height == b.min_height
        // Font (affects text measurement, thus geometry)
        && (a.font_size - b.font_size).abs() < f32::EPSILON
        && a.font_weight == b.font_weight
        && a.font_style == b.font_style
        && a.font_family == b.font_family
        && (a.line_height - b.line_height).abs() < f32::EPSILON
        && (a.letter_spacing - b.letter_spacing).abs() < f32::EPSILON
        && (a.word_spacing - b.word_spacing).abs() < f32::EPSILON
        && a.white_space == b.white_space
        && a.word_break == b.word_break
        && a.overflow_wrap == b.overflow_wrap
        // Flex
        && a.flex_direction == b.flex_direction
        && a.flex_wrap == b.flex_wrap
        && a.justify_content == b.justify_content
        && a.align_items == b.align_items
        && a.align_content == b.align_content
        && a.align_self == b.align_self
        && a.order == b.order
        && (a.flex_grow - b.flex_grow).abs() < f32::EPSILON
        && (a.flex_shrink - b.flex_shrink).abs() < f32::EPSILON
        && a.flex_basis == b.flex_basis
        && (a.gap - b.gap).abs() < f32::EPSILON
        // Grid
        && a.grid_template_columns == b.grid_template_columns
        && a.grid_template_rows == b.grid_template_rows
        && a.grid_column_start == b.grid_column_start
        && a.grid_column_end == b.grid_column_end
        && a.grid_row_start == b.grid_row_start
        && a.grid_row_end == b.grid_row_end
        && (a.column_gap - b.column_gap).abs() < f32::EPSILON
        && (a.row_gap - b.row_gap).abs() < f32::EPSILON
        // Margin auto flags
        && a.margin_left_auto == b.margin_left_auto
        && a.margin_right_auto == b.margin_right_auto
        && a.margin_top_auto == b.margin_top_auto
        && a.margin_bottom_auto == b.margin_bottom_auto
        // Box sizing
        && a.box_sizing == b.box_sizing
        // Visibility (can affect layout in some cases)
        && a.visibility == b.visibility
        // Text indent, text-align, overflow
        && (a.text_indent - b.text_indent).abs() < f32::EPSILON
        && a.text_align == b.text_align
        && a.overflow == b.overflow
        // Clear
        && a.clear == b.clear
        // Percentage padding/margin
        && a.padding_top_pct == b.padding_top_pct
        && a.padding_right_pct == b.padding_right_pct
        && a.padding_bottom_pct == b.padding_bottom_pct
        && a.padding_left_pct == b.padding_left_pct
        && a.margin_top_pct == b.margin_top_pct
        && a.margin_right_pct == b.margin_right_pct
        && a.margin_bottom_pct == b.margin_bottom_pct
        && a.margin_left_pct == b.margin_left_pct
        // Positioning offsets
        && a.top == b.top
        && a.right == b.right
        && a.bottom == b.bottom
        && a.left == b.left
        // Vertical alignment (affects inline/table-cell layout)
        && a.vertical_align == b.vertical_align
        // Table layout
        && a.border_collapse == b.border_collapse
        && (a.border_spacing - b.border_spacing).abs() < f32::EPSILON
        && a.table_layout_fixed == b.table_layout_fixed
        // List markers (affect layout)
        && a.list_style_type == b.list_style_type
        && a.list_style_position == b.list_style_position
        // Text transform (can change measured text width)
        && a.text_transform == b.text_transform
        // Generated content (affects layout when present)
        && a.content == b.content
        && a.before_content == b.before_content
        && a.after_content == b.after_content
        // Multi-column
        && a.column_count == b.column_count
        && (a.column_width - b.column_width).abs() < f32::EPSILON
        // Tab size (affects preformatted text width)
        && a.tab_size == b.tab_size
        // Replaced element sizing
        && a.object_fit == b.object_fit
        // Grid extensions
        && a.grid_auto_flow_column == b.grid_auto_flow_column
        && a.grid_template_areas == b.grid_template_areas
        && a.grid_area == b.grid_area
        && a.grid_auto_rows == b.grid_auto_rows
        && a.grid_auto_columns == b.grid_auto_columns
}

impl BrowserWidget {
    // ---------------------------------------------------------------
    // URL-bar selection helpers
    // ---------------------------------------------------------------

    /// Return the `(lo, hi)` byte range of the current URL-bar
    /// selection — `lo` and `hi` are sorted so callers don't need to
    /// care which way the user dragged. Returns `None` if there is no
    /// selection or the selection is collapsed (anchor == cursor).
    pub(crate) fn url_selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.url_selection_anchor?;
        if anchor == self.url_cursor {
            None
        } else if anchor < self.url_cursor {
            Some((anchor, self.url_cursor))
        } else {
            Some((self.url_cursor, anchor))
        }
    }

    /// Delete the selected range from `url_input` and position the
    /// cursor at the deletion point. Returns `true` if anything was
    /// deleted. Callers use the return value to decide whether a
    /// follow-up action (like Backspace's single-char delete) is still
    /// needed.
    fn url_delete_selection(&mut self) -> bool {
        if let Some((lo, hi)) = self.url_selection_range() {
            self.url_input.replace_range(lo..hi, "");
            self.url_cursor = lo;
            self.url_selection_anchor = None;
            true
        } else {
            self.url_selection_anchor = None;
            false
        }
    }

    // ---------------------------------------------------------------
    // Input handling
    // ---------------------------------------------------------------

    /// Handle an input event. Returns `true` if the event was
    /// consumed.
    pub fn handle_input(&mut self, event: &InputEvent, vfs: &dyn Vfs) -> bool {
        // URL-bar editing mode intercepts most keys.
        if self.focus == Focus::UrlBar {
            match event {
                InputEvent::TextInput(ch) => {
                    // If there's an active selection, typing replaces it.
                    // This is the standard "click URL bar, type to
                    // replace" flow users expect from Firefox/Chrome.
                    self.url_delete_selection();
                    self.url_input.insert(self.url_cursor, *ch);
                    self.url_cursor += ch.len_utf8();
                    return true;
                },
                InputEvent::Backspace => {
                    if self.url_delete_selection() {
                        // Selection was non-empty — deletion already
                        // happened, nothing more to do.
                    } else if self.url_cursor > 0 {
                        // No selection: delete the character before
                        // the cursor on a character boundary.
                        let prev = self.url_input[..self.url_cursor]
                            .char_indices()
                            .next_back()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        self.url_input.remove(prev);
                        self.url_cursor = prev;
                    }
                    return true;
                },
                InputEvent::ButtonPress(Button::Confirm) => {
                    let url = self.url_input.clone();
                    self.focus = Focus::Content;
                    self.url_selection_anchor = None;
                    if !url.is_empty() {
                        self.navigate_to(&url, vfs);
                    }
                    return true;
                },
                InputEvent::ButtonPress(Button::Cancel) => {
                    // Discard edits.
                    self.focus = Focus::Content;
                    self.url_input.clear();
                    self.url_cursor = 0;
                    self.url_selection_anchor = None;
                    return true;
                },
                InputEvent::ButtonPress(Button::Left) => {
                    // If there's a selection, collapse to its left edge
                    // instead of moving further. Matches GTK/Chrome.
                    if let Some((lo, _hi)) = self.url_selection_range() {
                        self.url_cursor = lo;
                        self.url_selection_anchor = None;
                    } else if self.url_cursor > 0 {
                        let prev = self.url_input[..self.url_cursor]
                            .char_indices()
                            .next_back()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        self.url_cursor = prev;
                    }
                    return true;
                },
                InputEvent::ButtonPress(Button::Right) => {
                    if let Some((_lo, hi)) = self.url_selection_range() {
                        self.url_cursor = hi;
                        self.url_selection_anchor = None;
                    } else if self.url_cursor < self.url_input.len() {
                        let next = self.url_input[self.url_cursor..]
                            .chars()
                            .next()
                            .map(|c| self.url_cursor + c.len_utf8())
                            .unwrap_or(self.url_input.len());
                        self.url_cursor = next;
                    }
                    return true;
                },
                InputEvent::Tab => {
                    // Tab from URL bar enters content focus at the first
                    // focusable element.
                    self.focus = Focus::Content;
                    self.tab_focus_forward();
                    return true;
                },
                InputEvent::ShiftTab => {
                    self.focus = Focus::Content;
                    self.tab_focus_backward();
                    return true;
                },
                InputEvent::PointerClick { x, y } => {
                    self.handle_click(*x, *y, vfs);
                    return true;
                },
                _ => return false,
            }
        }

        match event {
            InputEvent::ButtonPress(Button::Up) => {
                self.scroll.scroll_up();
                true
            },
            InputEvent::ButtonPress(Button::Down) => {
                self.scroll.scroll_down();
                true
            },
            InputEvent::ButtonPress(Button::Left) => {
                self.select_prev_link();
                true
            },
            InputEvent::ButtonPress(Button::Right) => {
                self.select_next_link();
                true
            },
            InputEvent::ButtonPress(Button::Confirm) => {
                // Enter / Confirm on a focused text input submits its
                // form — the dominant interaction path for search
                // boxes. Falls through to link activation when no
                // form element has focus.
                if self.form_manager.focused_element.is_some() {
                    self.dispatch_form_key(crate::forms::FormKey::Enter, vfs);
                    return true;
                }
                self.activate_selected_link(vfs);
                true
            },
            InputEvent::ButtonPress(Button::Cancel) => {
                self.go_back(vfs);
                true
            },
            InputEvent::ButtonPress(Button::Triangle) => {
                self.toggle_reader_mode();
                true
            },
            InputEvent::ButtonPress(Button::Square) => {
                self.go_home(vfs);
                true
            },
            InputEvent::TriggerPress(Trigger::Left) => {
                self.scroll.page_up();
                true
            },
            InputEvent::TriggerPress(Trigger::Right) => {
                self.scroll.page_down();
                true
            },
            InputEvent::MouseWheel { delta } => {
                // Check if cursor is over a nested scroll container.
                if let Some(nid) = self.find_scroll_container_at_cursor() {
                    let amount = *delta as f32 * crate::scroll::SCROLL_WHEEL as f32;
                    let entry = self.nested_scroll_offsets.entry(nid).or_insert((0.0, 0.0));
                    let prev = entry.1;
                    entry.1 += amount;
                    // Clamp to content bounds (computed during layout).
                    if let Some(layout) = &self.layout_root
                        && let Some(bounds) = Self::find_scroll_bounds(layout, nid)
                    {
                        entry.1 = entry.1.clamp(0.0, bounds);
                    }
                    if (entry.1 - prev).abs() > f32::EPSILON {
                        self.layout_dirty = true;
                    } else {
                        // At scroll limit — bubble to main page scroll.
                        self.scroll.wheel_scroll(*delta);
                    }
                } else {
                    self.scroll.wheel_scroll(*delta);
                }
                true
            },
            InputEvent::CursorMove { x, y } => {
                self.handle_cursor_move(*x, *y);
                true
            },
            InputEvent::Tab => {
                self.tab_focus_forward();
                true
            },
            InputEvent::ShiftTab => {
                self.tab_focus_backward();
                true
            },
            InputEvent::PointerClick { x, y } => {
                self.handle_click(*x, *y, vfs);
                true
            },
            InputEvent::Backspace => {
                // Delete a char from the focused text input, if any.
                // Without this branch Backspace is silently dropped in
                // Content focus — forms that rely on physical keyboard
                // editing (e.g. the Google search box) appear broken.
                if self.form_manager.focused_element.is_some() {
                    self.dispatch_form_key(crate::forms::FormKey::Backspace, vfs);
                    self.layout_dirty = true;
                    return true;
                }
                false
            },
            // Dispatch keydown + keyup + input events to JS.
            InputEvent::TextInput(ch) => {
                #[cfg(feature = "javascript")]
                if let Some(engine) = &self.js_engine {
                    // Dispatch to focused node, or body as fallback.
                    let target = self.focused_node.or(self.body_node_id);
                    if let Some(nid) = target {
                        Self::dispatch_js_key_event(engine, nid, *ch);
                        Self::dispatch_js_key_event_typed(engine, nid, *ch, "keyup");
                        Self::dispatch_js_event(engine, nid, "input");
                    }
                }
                // When a text input is focused, deliver the character
                // to the form manager instead of treating it as a page
                // shortcut — typing in Google's search box shouldn't
                // trigger zoom because the user pressed `+`.
                if self.form_manager.focused_element.is_some() {
                    self.dispatch_form_key(crate::forms::FormKey::Char(*ch), vfs);
                    self.layout_dirty = true;
                    return true;
                }
                // Zoom: + / - / 0 keys when not in URL bar.
                match ch {
                    '+' | '=' => self.zoom_in(),
                    '-' => self.zoom_out(),
                    '0' => self.reset_zoom(),
                    _ => {},
                }
                true
            },
            _ => false,
        }
    }

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
    fn scroll_to_selected_link(&mut self) {
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
    fn tab_focus_forward(&mut self) {
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
    fn tab_focus_backward(&mut self) {
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
    fn update_focused_node(&mut self) {
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
    fn restyle_focus_affected(&mut self, old_focus: Option<NodeId>) {
        let Some(doc) = &self.document else { return };

        let mut affected = rustc_hash::FxHashSet::default();
        for start in [old_focus, self.focused_node].into_iter().flatten() {
            let mut cur = Some(start);
            while let Some(nid) = cur {
                affected.insert(nid);
                cur = doc.nodes[nid].parent;
            }
        }

        if affected.is_empty() {
            return;
        }

        let ua_sheet = css::default::default_stylesheet();
        let mut all_sheets: Vec<&css::parser::Stylesheet> = vec![ua_sheet];
        for sheet in &self.cached_author_sheets {
            all_sheets.push(sheet);
        }

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
                self.focus = Focus::UrlBar;
                self.url_input = self.nav.current_url().unwrap_or("about:blank").to_string();
                self.url_cursor = self.url_input.len();
                self.url_selection_anchor = if self.url_input.is_empty() {
                    None
                } else {
                    Some(0)
                };
            }
            return;
        }

        // Click in content area: leave URL bar editing.
        self.focus = Focus::Content;

        // Dispatch mousedown, mouseup, then click to JS.
        #[cfg(feature = "javascript")]
        {
            self.dispatch_js_mouse_event(x, y, "mousedown");
            self.dispatch_js_mouse_event(x, y, "mouseup");
            self.dispatch_js_click(x, y);
        }

        // Check link hit regions.
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

        // Handle <label for="..."> click: focus the associated form element.
        self.handle_label_for_click(x, y);

        // Handle direct clicks on <input> elements: focus text inputs,
        // fire the form submission for submit buttons, toggle
        // checkbox/radio state. Needed so Google's search box takes
        // focus when clicked directly (not just via a <label>).
        self.handle_form_element_click(x, y, vfs);
    }

    /// Handle clicks that fall on a form `<input>` / `<button>` element
    /// directly (i.e. outside any `<label for="...">` wrapper).
    ///
    /// Text-like inputs grab focus so subsequent keystrokes flow into
    /// them through `dispatch_form_key`. Submit buttons trigger form
    /// submission with the clicked button's `name`/`value` pair
    /// (propagated by `FormManager::submit_with_button`). Checkboxes
    /// and radios toggle / select.
    fn handle_form_element_click(&mut self, x: i32, y: i32, vfs: &dyn Vfs) {
        use crate::html::dom::{NodeKind, TagName};

        let Some(nid) = self
            .layout_root
            .as_ref()
            .and_then(|root| root.hit_test(x as f32, y as f32))
        else {
            return;
        };
        let Some(doc) = &self.document else { return };

        // Walk up to the nearest <input> / <button> ancestor — the
        // click may land on a `<span>` wrapper like Google's
        // `<span class="lsbb"><input ...>`.
        let mut form_elem_nid = None;
        let mut cur = Some(nid);
        while let Some(id) = cur {
            if let NodeKind::Element(ref e) = doc.nodes[id].kind
                && matches!(e.tag, TagName::Input | TagName::Button | TagName::Textarea)
            {
                form_elem_nid = Some(id);
                break;
            }
            cur = doc.nodes[id].parent;
        }
        let Some(target_nid) = form_elem_nid else {
            return;
        };

        let (tag, input_type, value, name_or_id) = match &doc.nodes[target_nid].kind {
            NodeKind::Element(elem) => (
                elem.tag.clone(),
                elem.get_attribute("type")
                    .unwrap_or("text")
                    .to_ascii_lowercase(),
                elem.get_attribute("value").unwrap_or("").to_string(),
                elem.get_attribute("name")
                    .or_else(|| elem.get_attribute("id"))
                    .map(|s| s.to_string()),
            ),
            _ => return,
        };
        let Some(name) = name_or_id else { return };

        // Focus tracking: mirror the hover/:focus pseudo-class state
        // even when the form manager doesn't own this element.
        self.focused_node = Some(target_nid);

        let is_submit = matches!(input_type.as_str(), "submit" | "image")
            || (tag == TagName::Button && !matches!(input_type.as_str(), "button" | "reset"));

        // Look up which form owns this element so we can focus it.
        let fi_owning = self
            .form_manager
            .forms
            .iter()
            .position(|f| f.has_element(&name));

        if let Some(fi) = fi_owning {
            self.form_manager.focused_form = Some(fi);
            self.form_manager.focused_element = Some(name.clone());
        }

        match input_type.as_str() {
            "checkbox" => {
                let _ = self.form_manager.handle_input(crate::forms::FormKey::Space);
                self.layout_dirty = true;
            },
            "radio" => {
                if let Some(fi) = fi_owning {
                    self.form_manager.select_radio(fi, &name, &value);
                    self.layout_dirty = true;
                }
            },
            _ if is_submit => {
                if let Some(fi) = fi_owning
                    && let Some(data) = self.form_manager.submit(fi)
                {
                    self.handle_form_submit(&data, vfs);
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

    /// If the click hits a `<label>` element with a `for` attribute,
    /// focus the form element whose `id` matches.
    fn handle_label_for_click(&mut self, x: i32, y: i32) {
        use crate::html::dom::{NodeKind, TagName};

        let node_id = self
            .layout_root
            .as_ref()
            .and_then(|root| root.hit_test(x as f32, y as f32));

        let Some(nid) = node_id else { return };
        let Some(doc) = &self.document else {
            return;
        };

        // Walk up from the hit node to find a <label> ancestor.
        let mut label_for = None;
        let mut cur = Some(nid);
        while let Some(id) = cur {
            if let NodeKind::Element(ref elem) = doc.nodes[id].kind
                && elem.tag == TagName::Label
            {
                if let Some(for_val) = elem.get_attribute("for") {
                    label_for = Some(for_val.to_string());
                }
                break;
            }
            cur = doc.nodes[id].parent;
        }

        let Some(for_id) = label_for else { return };

        // Find the target element by id.
        let Some(target_nid) = doc.get_element_by_id(&for_id) else {
            return;
        };

        // Get the target element's name attribute to match against
        // form elements.
        let target_name = match &doc.nodes[target_nid].kind {
            NodeKind::Element(elem) => elem
                .get_attribute("name")
                .or_else(|| elem.get_attribute("id"))
                .map(|s| s.to_string()),
            _ => None,
        };

        let Some(name) = target_name else { return };

        // Update focused_node so :focus CSS and JS keyboard events work.
        self.focused_node = Some(target_nid);

        // Determine the input type and value so we can toggle
        // checkbox/radio state.
        let (input_type, target_value) = match &doc.nodes[target_nid].kind {
            NodeKind::Element(elem) => (
                elem.get_attribute("type").unwrap_or("text"),
                elem.get_attribute("value").unwrap_or("").to_string(),
            ),
            _ => ("text", String::new()),
        };

        // Search form_manager for a form containing this element name
        // and focus it.
        for (fi, form) in self.form_manager.forms.iter().enumerate() {
            if form.has_element(&name) {
                self.form_manager.focused_form = Some(fi);
                self.form_manager.focused_element = Some(name.clone());

                // Toggle checkbox/radio on label click (standard HTML
                // behavior).
                if input_type == "checkbox" {
                    let _ = self.form_manager.handle_input(crate::forms::FormKey::Space);
                    self.layout_dirty = true;
                } else if input_type == "radio" {
                    // For radio buttons, select_radio uses the value to
                    // pick the correct option in the group, avoiding the
                    // index_of(name) ambiguity where all radios share
                    // the same name.
                    self.form_manager.select_radio(fi, &name, &target_value);
                    self.layout_dirty = true;
                }
                return;
            }
        }
    }

    /// If the click hits a `<summary>` element, toggle the `open`
    /// attribute on its parent `<details>`.
    fn handle_details_toggle(&mut self, x: i32, y: i32) {
        use crate::html::dom::{NodeKind, TagName};

        let node_id = self
            .layout_root
            .as_ref()
            .and_then(|root| root.hit_test(x as f32, y as f32));

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

    /// Dispatch a JS click event using the layout tree hit test.
    #[cfg(feature = "javascript")]
    fn dispatch_js_click(&mut self, x: i32, y: i32) {
        let node_id = self
            .layout_root
            .as_ref()
            .and_then(|root| root.hit_test(x as f32, y as f32));

        if let (Some(nid), Some(engine)) = (node_id, &self.js_engine) {
            Self::dispatch_js_event(engine, nid, "click");
        }
    }

    /// Dispatch a named event to JS with bubbling.
    #[cfg(feature = "javascript")]
    fn dispatch_js_event(engine: &oasis_js::JsEngine, node_id: NodeId, event_type: &str) {
        let code = format!(
            "if(typeof __oasis_dispatch_with_bubbling==='function')\
             __oasis_dispatch_with_bubbling({},'{}',null)",
            node_id, event_type
        );
        let _ = engine.eval(&code);
    }

    /// Dispatch a mouse event (mousedown, mouseup, etc.) to the node at
    /// the given coordinates, passing `clientX`/`clientY` as detail.
    #[cfg(feature = "javascript")]
    fn dispatch_js_mouse_event(&mut self, x: i32, y: i32, event_type: &str) {
        let node_id = self
            .layout_root
            .as_ref()
            .and_then(|root| root.hit_test(x as f32, y as f32));
        if let (Some(nid), Some(engine)) = (node_id, &self.js_engine) {
            Self::dispatch_js_mouse_event_to(engine, nid, x, y, event_type);
        }
    }

    /// Dispatch a mouse event to a specific node with coordinates.
    #[cfg(feature = "javascript")]
    fn dispatch_js_mouse_event_to(
        engine: &oasis_js::JsEngine,
        node_id: NodeId,
        x: i32,
        y: i32,
        event_type: &str,
    ) {
        let code = format!(
            "if(typeof __oasis_dispatch_with_bubbling==='function'){{\
             var __e={{clientX:{},clientY:{}}};\
             __oasis_dispatch_with_bubbling({},'{}',__e)}}",
            x, y, node_id, event_type
        );
        let _ = engine.eval(&code);
    }

    /// Dispatch a keydown event to JS with key info as detail object.
    #[cfg(feature = "javascript")]
    fn dispatch_js_key_event(engine: &oasis_js::JsEngine, node_id: NodeId, key: char) {
        Self::dispatch_js_key_event_typed(engine, node_id, key, "keydown");
    }

    #[cfg(feature = "javascript")]
    fn dispatch_js_key_event_typed(
        engine: &oasis_js::JsEngine,
        node_id: NodeId,
        key: char,
        event_type: &str,
    ) {
        // Escape characters that break JS single-quoted string literals.
        let escaped: String = match key {
            '\\' => "\\\\".into(),
            '\'' => "\\'".into(),
            '\n' => "\\n".into(),
            '\r' => "\\r".into(),
            c => c.to_string(),
        };
        let code = format!(
            "if(typeof __oasis_dispatch_with_bubbling==='function')\
             __oasis_dispatch_with_bubbling({},'{event_type}',{{key:'{}',code:'{}'}})",
            node_id, escaped, escaped
        );
        let _ = engine.eval(&code);
    }

    /// Handle a cursor move at window-relative coordinates.
    ///
    /// Hit-tests link regions to determine the hover target. If the
    /// hovered node changes, re-runs the CSS cascade so `:hover` rules
    /// take effect.
    fn handle_cursor_move(&mut self, x: i32, y: i32) {
        let mut new_hover: Option<NodeId> = None;
        for link in &self.link_map {
            let lx = link.rect.x;
            let ly = link.rect.y;
            let lw = link.rect.width;
            let lh = link.rect.height;
            if (x as f32) >= lx && (x as f32) < lx + lw && (y as f32) >= ly && (y as f32) < ly + lh
            {
                new_hover = Some(link.node);
                break;
            }
        }

        if new_hover != self.hover_node {
            // Throttle hover restyles to at most 20/sec.
            let now = std::time::Instant::now();
            if let Some(last) = self.last_hover_time
                && now.duration_since(last).as_millis() < 50
            {
                return;
            }
            self.last_hover_time = Some(now);

            let old_hover = self.hover_node;
            self.hover_node = new_hover;

            // Dispatch mouseover/mouseout/mousemove events to JS.
            #[cfg(feature = "javascript")]
            if let Some(engine) = &self.js_engine {
                if let Some(old_nid) = old_hover {
                    Self::dispatch_js_event(engine, old_nid, "mouseout");
                }
                if let Some(new_nid) = new_hover {
                    Self::dispatch_js_event(engine, new_nid, "mouseover");
                    Self::dispatch_js_mouse_event_to(engine, new_nid, x, y, "mousemove");
                }
            }

            self.restyle_hover_affected(old_hover);
        }
    }

    /// Re-run the CSS cascade on hover-affected nodes only.
    ///
    /// Instead of re-parsing stylesheets and re-cascading the entire DOM,
    /// this uses cached sheets and only re-computes styles for the ancestors
    /// of the old and new hover nodes -- typically ~10-20 nodes.
    ///
    /// If only visual properties changed (color, background, opacity, etc.)
    /// the layout tree is reused and only a repaint is needed.
    pub(crate) fn restyle_hover_affected(&mut self, old_hover: Option<NodeId>) {
        let Some(doc) = &self.document else { return };

        // Build the set of affected nodes: ancestors of old + new hover.
        // Using FxHashSet for O(1) dedup instead of Vec::contains O(n).
        let mut affected = rustc_hash::FxHashSet::default();
        for start in [old_hover, self.hover_node].into_iter().flatten() {
            let mut cur = Some(start);
            while let Some(nid) = cur {
                affected.insert(nid);
                cur = doc.nodes[nid].parent;
            }
        }

        if affected.is_empty() {
            return;
        }

        // Build sheet references from cache (no re-parsing).
        let ua_sheet = css::default::default_stylesheet();
        let mut all_sheets: Vec<&css::parser::Stylesheet> = vec![ua_sheet];
        for sheet in &self.cached_author_sheets {
            all_sheets.push(sheet);
        }

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
        let mut geometry_changed = false;
        let mut needs_full_repaint = false;
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
                // Check if geometry-affecting properties changed.
                if let Some(old_style) = &self.styles[nid] {
                    if !styles_geometry_equal(old_style, &new_style) {
                        geometry_changed = true;
                    }
                    // Check if non-patchable visual properties changed
                    // (border colors, opacity, box-shadow, outline, etc.).
                    // patch_node_colors only handles color + background_color,
                    // so other visual changes need a full display list rebuild.
                    if !needs_full_repaint
                        && (old_style.border_top_color != new_style.border_top_color
                            || old_style.border_right_color != new_style.border_right_color
                            || old_style.border_bottom_color != new_style.border_bottom_color
                            || old_style.border_left_color != new_style.border_left_color
                            || old_style.opacity != new_style.opacity
                            || old_style.outline_color != new_style.outline_color
                            || old_style.box_shadow != new_style.box_shadow)
                    {
                        needs_full_repaint = true;
                    }

                    // Start CSS transitions for numeric properties that
                    // changed, if the element declares transitions.
                    start_transitions_for_change(
                        &mut self.transition_engine,
                        nid,
                        old_style,
                        &new_style,
                    );
                } else {
                    geometry_changed = true;
                }
                self.styles[nid] = Some(new_style);
                any_changed = true;
            }
        }

        if any_changed && geometry_changed {
            // Geometry changed: need full relayout.
            self.layout_dirty = true;
        } else if any_changed && needs_full_repaint {
            // Non-patchable visual properties changed (border color,
            // opacity, etc.) — need full display list rebuild.
            self.full_repaint_needed = true;
        } else if any_changed {
            // Only color/background changed — patchable in-place.
            let affected_vec: Vec<NodeId> = affected.into_iter().collect();
            self.mark_hover_focus_dirty(&affected_vec);
        }
    }

    /// Push dirty rectangles for a set of affected DOM nodes.
    ///
    /// Looks up each node's border-box in the layout tree and adds it
    /// to `dirty_rects`. If any node's rect cannot be found, falls back
    /// to a full repaint.
    fn mark_hover_focus_dirty(&mut self, affected: &[NodeId]) {
        let Some(layout) = &self.layout_root else {
            self.full_repaint_needed = true;
            return;
        };

        for &nid in affected {
            // If the affected node has CSS transforms or sticky positioning,
            // the static layout rect doesn't reflect its actual screen position.
            // Fall back to full repaint since computing the transformed rect
            // would require replicating the full transform/sticky offset chain.
            if nid < self.styles.len()
                && let Some(ref style) = self.styles[nid]
                && (!style.transforms.is_empty()
                    || style.position == crate::css::values::Position::Sticky)
            {
                self.full_repaint_needed = true;
                return;
            }

            if let Some(rect) = Self::find_node_rect(layout, nid) {
                // Convert layout-space rect to screen-space by applying
                // scroll offset and window position.
                let content_y = self.window_y + self.config.url_bar_height as i32;
                let screen_rect = crate::layout::box_model::Rect {
                    x: rect.x - self.scroll.scroll_x as f32 + self.window_x as f32,
                    y: rect.y - self.scroll.scroll_y as f32 + content_y as f32,
                    width: rect.width,
                    height: rect.height,
                };
                self.dirty_rects.push(screen_rect);
            }
        }

        if self.dirty_rects.is_empty() {
            // Could not find rects for any affected node — force full repaint.
            self.full_repaint_needed = true;
        }
    }

    /// Navigate to a URL, resolving relative references against
    /// the current page.
    pub fn navigate_to(&mut self, href: &str, vfs: &dyn Vfs) {
        let resolved = if let Some(current) = self.nav.current_url() {
            if let Some(base) = Url::parse(current) {
                base.resolve(href)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| href.to_string())
            } else {
                href.to_string()
            }
        } else {
            href.to_string()
        };

        // Track this URL as visited for :visited pseudo-class.
        self.visited_urls.insert(resolved.clone());
        self.navigate_vfs(&resolved, vfs);
    }

    /// Go back in history.
    ///
    /// Prefers loading from cache when available to avoid re-fetching.
    pub fn go_back(&mut self, vfs: &dyn Vfs) {
        // Save current scroll position.
        self.nav.update_scroll(self.scroll.scroll_y);

        if let Some(entry) = self.nav.go_back() {
            let url = entry.url.clone();
            let scroll_y = entry.scroll_y;
            self.navigate_cached_or_fetch(&url, vfs);
            self.scroll.scroll_to(scroll_y);
        }
    }

    /// Go forward in history.
    ///
    /// Prefers loading from cache when available to avoid re-fetching.
    pub fn go_forward(&mut self, vfs: &dyn Vfs) {
        self.nav.update_scroll(self.scroll.scroll_y);

        if let Some(entry) = self.nav.go_forward() {
            let url = entry.url.clone();
            let scroll_y = entry.scroll_y;
            self.navigate_cached_or_fetch(&url, vfs);
            self.scroll.scroll_to(scroll_y);
        }
    }

    /// Navigate to the home page.
    pub fn go_home(&mut self, vfs: &dyn Vfs) {
        let url = self.nav.go_home();
        self.navigate_vfs(&url, vfs);
    }

    /// Dispatch a form key event to the form manager and handle the
    /// resulting action (submission, focus change, etc.).
    ///
    /// Returns `true` if the event was consumed by the form manager.
    pub fn dispatch_form_key(&mut self, key: crate::forms::FormKey, vfs: &dyn Vfs) -> bool {
        let action = self.form_manager.handle_input(key);
        match action {
            crate::forms::FormAction::Submit(ref data) => {
                self.handle_form_submit(data, vfs);
                true
            },
            crate::forms::FormAction::FocusChanged | crate::forms::FormAction::ValueChanged => true,
            crate::forms::FormAction::None => false,
        }
    }

    /// Handle a form submission.
    ///
    /// For GET forms, the encoded data is appended as a query string.
    /// For POST forms, the encoded data is sent as the request body.
    pub fn handle_form_submit(&mut self, data: &crate::forms::FormData, vfs: &dyn Vfs) {
        let encoded = data.encode();
        let action = &data.action;

        // Resolve the action URL against the current page.
        let resolved_action = if let Some(current) = self.nav.current_url() {
            if let Some(base) = Url::parse(current) {
                base.resolve(action)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| action.to_string())
            } else {
                action.to_string()
            }
        } else {
            action.to_string()
        };

        match data.method {
            crate::forms::FormMethod::Get => {
                // Append form data as query string.
                let url = if encoded.is_empty() {
                    resolved_action
                } else if resolved_action.contains('?') {
                    format!("{resolved_action}&{encoded}")
                } else {
                    format!("{resolved_action}?{encoded}")
                };
                self.visited_urls.insert(url.clone());
                self.navigate_vfs(&url, vfs);
            },
            crate::forms::FormMethod::Post => {
                let body = encoded.into_bytes();
                self.visited_urls.insert(resolved_action.clone());
                self.navigate_post(&resolved_action, body, vfs);
            },
        }
    }

    /// Find the innermost nested scroll container under the current cursor
    /// position. Returns `None` if the cursor is not over any scroll container.
    fn find_scroll_container_at_cursor(&self) -> Option<NodeId> {
        use crate::css::values::Overflow;

        let layout = self.layout_root.as_ref()?;
        // Use hover_node as a proxy for cursor position — walk its ancestors
        // looking for the nearest scroll container.
        let hover = self.hover_node?;
        Self::find_scroll_ancestor(layout, hover).filter(|&nid| {
            // Only return if this node is an overflow container.
            if let Some(Some(style)) = self.styles.get(nid) {
                matches!(style.overflow, Overflow::Auto | Overflow::Scroll)
            } else {
                false
            }
        })
    }

    /// Walk the layout tree to find the nearest ancestor of `target_nid`
    /// that has `overflow: auto/scroll`.
    fn find_scroll_ancestor(layout_box: &LayoutBox, target_nid: NodeId) -> Option<NodeId> {
        use crate::css::values::Overflow;

        // Check if this box IS the target node.
        if layout_box.node == Some(target_nid) {
            // The target itself may be a scroll container.
            if matches!(layout_box.style.overflow, Overflow::Auto | Overflow::Scroll) {
                return layout_box.node;
            }
            return None;
        }

        for child in &layout_box.children {
            // If child IS the target, return this box if it's a scroll container.
            if child.node == Some(target_nid) {
                if matches!(layout_box.style.overflow, Overflow::Auto | Overflow::Scroll) {
                    return layout_box.node;
                }
                return None;
            }

            // Recurse into child.
            if let Some(found) = Self::find_scroll_ancestor(child, target_nid) {
                return Some(found);
            }

            // Check if target is somewhere in this child's subtree.
            if Self::subtree_contains(child, target_nid) {
                // Target is inside this child. If this box is a scroll
                // container, return it.
                if matches!(layout_box.style.overflow, Overflow::Auto | Overflow::Scroll) {
                    return layout_box.node;
                }
                return None;
            }
        }
        None
    }

    /// Check if a layout subtree contains a node with the given ID.
    fn subtree_contains(layout_box: &LayoutBox, nid: NodeId) -> bool {
        if layout_box.node == Some(nid) {
            return true;
        }
        layout_box
            .children
            .iter()
            .any(|c| Self::subtree_contains(c, nid))
    }

    /// Find the maximum scroll Y for a nested scroll container.
    ///
    /// Returns `content_height - box_height`, clamped to >= 0.
    fn find_scroll_bounds(layout_box: &LayoutBox, nid: NodeId) -> Option<f32> {
        if layout_box.node == Some(nid) {
            let content_h: f32 = layout_box
                .children
                .iter()
                .map(|c| {
                    let mb = c.dimensions.margin_box();
                    mb.y + mb.height
                })
                .fold(0.0f32, f32::max);
            let box_h = layout_box.dimensions.content.height;
            let max_scroll = (content_h - layout_box.dimensions.content.y - box_h).max(0.0);
            return Some(max_scroll);
        }
        for child in &layout_box.children {
            if let Some(bounds) = Self::find_scroll_bounds(child, nid) {
                return Some(bounds);
            }
        }
        None
    }
}

/// All CSS properties that can be smoothly transitioned as a single f32.
const TRANSITIONABLE_PROPERTIES: &[&str] = &[
    "opacity",
    "font-size",
    "line-height",
    "letter-spacing",
    "word-spacing",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "border-radius",
    "border-spacing",
    "outline-width",
    "outline-offset",
    "text-indent",
    "gap",
    "column-gap",
    "row-gap",
    "flex-grow",
    "flex-shrink",
];

/// Extract a numeric value from a [`ComputedStyle`] for a transitionable
/// CSS property. Returns `None` for properties that are not numeric
/// (e.g. colors, enums) or not recognized.
fn get_style_numeric_value(style: &ComputedStyle, property: &str) -> Option<f32> {
    match property {
        "opacity" => Some(style.opacity),
        "font-size" => Some(style.font_size),
        "line-height" => Some(style.line_height),
        "letter-spacing" => Some(style.letter_spacing),
        "word-spacing" => Some(style.word_spacing),
        "margin-top" => Some(style.margin_top),
        "margin-right" => Some(style.margin_right),
        "margin-bottom" => Some(style.margin_bottom),
        "margin-left" => Some(style.margin_left),
        "padding-top" => Some(style.padding_top),
        "padding-right" => Some(style.padding_right),
        "padding-bottom" => Some(style.padding_bottom),
        "padding-left" => Some(style.padding_left),
        "border-top-width" => Some(style.border_top_width),
        "border-right-width" => Some(style.border_right_width),
        "border-bottom-width" => Some(style.border_bottom_width),
        "border-left-width" => Some(style.border_left_width),
        "border-radius" => Some(style.border_radius.max_radius()),
        "border-spacing" => Some(style.border_spacing),
        "outline-width" => Some(style.outline_width),
        "outline-offset" => Some(style.outline_offset),
        "text-indent" => Some(style.text_indent),
        "gap" => Some(style.gap),
        "column-gap" => Some(style.column_gap),
        "row-gap" => Some(style.row_gap),
        "flex-grow" => Some(style.flex_grow),
        "flex-shrink" => Some(style.flex_shrink),
        _ => None,
    }
}

/// Try to start a single transition for `prop` between `old_style` and
/// `new_style` using the given [`Transition`] declaration.
fn try_start_property_transition(
    engine: &mut TransitionEngine,
    nid: NodeId,
    prop: &str,
    old_style: &ComputedStyle,
    new_style: &ComputedStyle,
    trans: &Transition,
) {
    if trans.duration_ms <= 0.0 {
        return;
    }
    if let (Some(from), Some(to)) = (
        get_style_numeric_value(old_style, prop),
        get_style_numeric_value(new_style, prop),
    ) && (from - to).abs() > f32::EPSILON
    {
        engine.start_transition(nid, prop, from, to, trans);
    }
}

/// Start CSS transitions for all numeric properties that changed between
/// `old_style` and `new_style`, based on the transition declarations in
/// both styles.
///
/// The new style's transitions are checked first (entering a state like
/// `:hover`). The old style's transitions are also checked for properties
/// not covered by the new style (leaving a state).
fn start_transitions_for_change(
    engine: &mut TransitionEngine,
    nid: NodeId,
    old_style: &ComputedStyle,
    new_style: &ComputedStyle,
) {
    // Transitions declared on the new style (e.g. entering :hover).
    for trans in &new_style.transitions {
        if trans.property == "all" {
            for &prop in TRANSITIONABLE_PROPERTIES {
                try_start_property_transition(engine, nid, prop, old_style, new_style, trans);
            }
        } else {
            try_start_property_transition(
                engine,
                nid,
                &trans.property,
                old_style,
                new_style,
                trans,
            );
        }
    }

    // Transitions declared on the old style that are not covered by the
    // new style (e.g. leaving :hover -- the base style may not redeclare
    // the transition, but the old hover style's transition should still
    // animate back).
    for trans in &old_style.transitions {
        let prop_name = &trans.property;
        let already_covered = if prop_name == "all" {
            // If new_style already has an "all" transition, skip.
            new_style.transitions.iter().any(|t| t.property == "all")
        } else {
            new_style
                .transitions
                .iter()
                .any(|t| t.property == *prop_name || t.property == "all")
        };
        if already_covered {
            continue;
        }

        if *prop_name == "all" {
            for &prop in TRANSITIONABLE_PROPERTIES {
                // Skip properties already started from new_style.
                if new_style.transitions.iter().any(|t| t.property == prop) {
                    continue;
                }
                try_start_property_transition(engine, nid, prop, old_style, new_style, trans);
            }
        } else {
            try_start_property_transition(engine, nid, prop_name, old_style, new_style, trans);
        }
    }
}
