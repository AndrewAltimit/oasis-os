//! Input handling methods for [`BrowserWidget`].

use oasis_types::input::{Button, InputEvent, Trigger};
use oasis_vfs::Vfs;
use rustc_hash::FxHashMap;

use crate::css;
use crate::css::values::ComputedStyle;
use crate::html;
use crate::html::dom::NodeId;
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
}

impl BrowserWidget {
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
                    self.url_input.insert(self.url_cursor, *ch);
                    self.url_cursor += ch.len_utf8();
                    return true;
                },
                InputEvent::Backspace => {
                    if self.url_cursor > 0 {
                        // Find the previous character boundary.
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
                    return true;
                },
                InputEvent::ButtonPress(Button::Left) => {
                    if self.url_cursor > 0 {
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
                    if self.url_cursor < self.url_input.len() {
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
                self.scroll.wheel_scroll(*delta);
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
            // Dispatch keydown + input events to JS for focused nodes.
            InputEvent::TextInput(ch) => {
                #[cfg(feature = "javascript")]
                if let (Some(nid), Some(engine)) = (self.focused_node, &self.js_engine) {
                    Self::dispatch_js_key_event(engine, nid, *ch);
                    Self::dispatch_js_event(engine, nid, "input");
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

        let mut affected: Vec<NodeId> = Vec::new();
        for start in [old_focus, self.focused_node].into_iter().flatten() {
            let mut cur = Some(start);
            while let Some(nid) = cur {
                if !affected.contains(&nid) {
                    affected.push(nid);
                }
                cur = doc.nodes[nid].parent;
            }
        }

        if affected.is_empty() {
            return;
        }

        let ua_sheet = css::default::default_stylesheet();
        let mut all_sheets: Vec<&css::parser::Stylesheet> = vec![&ua_sheet];
        for sheet in &self.cached_author_sheets {
            all_sheets.push(sheet);
        }

        let index = css::cascade::SelectorIndex::build(&all_sheets);
        let inline_map: FxHashMap<NodeId, &[css::parser::Declaration]> = self
            .cached_inline_styles
            .iter()
            .map(|(nid, decls)| (*nid, decls.as_slice()))
            .collect();
        let ctx = css::cascade::CascadeContext {
            hover_node: self.hover_node,
            visited_urls: Some(&self.visited_urls),
            focused_node: self.focused_node,
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
                &index,
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

            if rel_x < bw {
                // Back button.
                self.focus = Focus::Content;
                self.go_back(vfs);
            } else if rel_x < bw * 2 {
                // Forward button.
                self.focus = Focus::Content;
                self.go_forward(vfs);
            } else if rel_x >= self.window_w as i32 - bw {
                // Home button.
                self.focus = Focus::Content;
                self.go_home(vfs);
            } else {
                // URL bar area -- enter edit mode.
                self.focus = Focus::UrlBar;
                self.url_input = self.nav.current_url().unwrap_or("about:blank").to_string();
                self.url_cursor = self.url_input.len();
            }
            return;
        }

        // Click in content area: leave URL bar editing.
        self.focus = Focus::Content;

        // Dispatch click event to JS if an engine is retained.
        #[cfg(feature = "javascript")]
        self.dispatch_js_click(x, y);

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

    /// Dispatch a keydown event to JS with the key character as detail.
    #[cfg(feature = "javascript")]
    fn dispatch_js_key_event(engine: &oasis_js::JsEngine, node_id: NodeId, key: char) {
        // Escape single quotes in the key character for the JS string.
        let escaped = if key == '\'' { "\\'" } else { "" };
        let code = if escaped.is_empty() {
            format!(
                "if(typeof __oasis_dispatch_with_bubbling==='function')\
                 __oasis_dispatch_with_bubbling({},'keydown','{}')",
                node_id, key
            )
        } else {
            format!(
                "if(typeof __oasis_dispatch_with_bubbling==='function')\
                 __oasis_dispatch_with_bubbling({},'keydown','{}')",
                node_id, escaped
            )
        };
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

            // Dispatch mouseover/mouseout events to JS.
            #[cfg(feature = "javascript")]
            if let Some(engine) = &self.js_engine {
                if let Some(old_nid) = old_hover {
                    Self::dispatch_js_event(engine, old_nid, "mouseout");
                }
                if let Some(new_nid) = new_hover {
                    Self::dispatch_js_event(engine, new_nid, "mouseover");
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
        let mut affected: Vec<NodeId> = Vec::new();
        for start in [old_hover, self.hover_node].into_iter().flatten() {
            let mut cur = Some(start);
            while let Some(nid) = cur {
                if !affected.contains(&nid) {
                    affected.push(nid);
                }
                cur = doc.nodes[nid].parent;
            }
        }

        if affected.is_empty() {
            return;
        }

        // Build sheet references from cache (no re-parsing).
        let ua_sheet = css::default::default_stylesheet();
        let mut all_sheets: Vec<&css::parser::Stylesheet> = vec![&ua_sheet];
        for sheet in &self.cached_author_sheets {
            all_sheets.push(sheet);
        }

        let index = css::cascade::SelectorIndex::build(&all_sheets);
        let inline_map: FxHashMap<NodeId, &[css::parser::Declaration]> = self
            .cached_inline_styles
            .iter()
            .map(|(nid, decls)| (*nid, decls.as_slice()))
            .collect();
        let ctx = css::cascade::CascadeContext {
            hover_node: self.hover_node,
            visited_urls: Some(&self.visited_urls),
            focused_node: self.focused_node,
        };

        let mut any_changed = false;
        let mut geometry_changed = false;
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
                &index,
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
        }
        // If only visual properties changed, styles are updated but
        // layout_dirty remains false -- next paint uses existing layout.
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
    pub fn go_back(&mut self, vfs: &dyn Vfs) {
        // Save current scroll position.
        self.nav.update_scroll(self.scroll.scroll_y);

        if let Some(entry) = self.nav.go_back() {
            let url = entry.url.clone();
            let scroll_y = entry.scroll_y;
            self.navigate_vfs(&url, vfs);
            self.scroll.scroll_to(scroll_y);
        }
    }

    /// Go forward in history.
    pub fn go_forward(&mut self, vfs: &dyn Vfs) {
        self.nav.update_scroll(self.scroll.scroll_y);

        if let Some(entry) = self.nav.go_forward() {
            let url = entry.url.clone();
            let scroll_y = entry.scroll_y;
            self.navigate_vfs(&url, vfs);
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
}
