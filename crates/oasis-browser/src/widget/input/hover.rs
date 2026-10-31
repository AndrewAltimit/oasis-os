//! Cursor movement, `:hover` restyling, CSS transitions and applying
//! script DOM mutations.

use super::*;

impl BrowserWidget {
    /// Handle a cursor move at window-relative coordinates.
    ///
    /// Hit-tests link regions to determine the hover target. If the
    /// hovered node changes, re-runs the CSS cascade so `:hover` rules
    /// take effect.
    pub(super) fn handle_cursor_move(&mut self, x: i32, y: i32) {
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
            let now = web_time::Instant::now();
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
            // If a hover handler mutated the DOM (e.g. a tooltip
            // shim toggling `aria-expanded`), flush the dirty flag
            // so the next frame reflects the change — symmetric with
            // the click and text-input dispatch paths.
            #[cfg(feature = "javascript")]
            self.apply_js_dom_mutations();

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

        let affected = state_change_affected(doc, old_hover, self.hover_node);
        if affected.is_empty() {
            return;
        }

        // Build sheet references from cache (no re-parsing).
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
            self.mark_hover_focus_dirty(&affected);
        }
    }

    /// Push dirty rectangles for a set of affected DOM nodes.
    ///
    /// Looks up each node's border-box in the layout tree and adds it
    /// to `dirty_rects`. If any node's rect cannot be found, falls back
    /// to a full repaint.
    pub(super) fn mark_hover_focus_dirty(&mut self, affected: &[NodeId]) {
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

    /// Re-run the CSS cascade and rebuild layout after a JS event
    /// handler mutated the DOM. No-op if the shared dirty flag is clear.
    ///
    /// The cheaper `restyle_hover_affected` / `relayout_if_dirty` paths
    /// assume stylesheets and the DOM tree haven't changed — after a
    /// `setAttribute`/`classList.add`/`innerHTML` the set of nodes
    /// matching rules like `.comment.collapsed .child` changes and the
    /// style vector needs a full re-cascade. We also sync `self.document`
    /// from the JS-shared document since mutations landed on the latter.
    #[cfg(feature = "javascript")]
    pub(crate) fn apply_js_dom_mutations(&mut self) {
        if !self.js_dom_dirty.get() {
            return;
        }
        self.js_dom_dirty.set(false);
        let Some(js_doc) = self.js_doc.as_ref() else {
            return;
        };
        // Clone the JS-shared (mutated) document into the widget's
        // authoritative slot so every downstream consumer — cascade,
        // layout, paint, link map, form manager — sees the new tree.
        let new_doc = js_doc.borrow().clone();
        self.body_node_id = new_doc.body();
        // `document.title = ...` shows up in the chrome / history.
        if let Some(title) = new_doc.title() {
            self.nav.update_title(&title);
        }
        self.document = Some(new_doc);
        // A script that wrote `input.value` now owns that value: it is
        // what gets submitted and what the next keystroke edits.
        self.adopt_form_values_from_dom();

        // Rebuild inline-style cache from the mutated DOM. JS may have
        // overwritten `style=""` attributes via `element.style.prop = ...`
        // (`__oasis_style_set` in js_dom/bindings.rs) or inserted new nodes with
        // inline styles via `innerHTML`; the cached list was captured at
        // page load and is now stale.
        if let Some(doc) = &self.document {
            self.cached_inline_styles = Self::collect_inline_styles(doc);
        }

        // Build sheet references from cache (no re-parsing).
        let ua_sheet = css::default::default_stylesheet();
        let mut all_sheets: Vec<&css::parser::Stylesheet> = vec![ua_sheet];
        all_sheets.extend(Self::merge_author_sheets(
            &self.cached_author_sheets,
            &self.cached_author_sheet_positions,
            &self.external_stylesheets,
            &self.external_stylesheet_positions,
        ));
        let ctx = css::cascade::CascadeContext {
            hover_node: self.hover_node,
            visited_urls: Some(&self.visited_urls),
            focused_node: self.focused_node,
            containers: self.container_lookup.as_ref(),
            global_layers: None,
        };
        if let Some(doc) = &self.document {
            self.styles =
                css::cascade::style_tree(doc, &all_sheets, &self.cached_inline_styles, &ctx);
            *self.js_styles.borrow_mut() = self.styles.clone();
            // Rebuild href_map (NodeId -> href) from the mutated DOM so
            // anchor fragment lookups see newly-inserted <a> elements.
            // Note: link_map (the Vec<LinkRegion> used by click hit-testing)
            // is rebuilt by the layout pass triggered via `layout_dirty`
            // below, not here.
            self.href_map = BrowserWidget::build_link_map(doc);
        }
        // Geometry may have changed (class toggles, display:none, new
        // nodes from innerHTML) — force both a fresh layout pass and a
        // full display-list rebuild on the next tick.
        self.layout_dirty = true;
        self.full_repaint_needed = true;
    }
}

/// Most descendants of a hovered / focused node restyled with it; past
/// this the rest keep their styles until the next full cascade.
const MAX_STATE_RESTYLE_DESCENDANTS: usize = 4096;

/// The nodes a `:hover` / `:focus` change between `old` and `new` can
/// restyle, in an order where every parent precedes its children, so each
/// node cascades against its parent's *updated* style: both ancestor
/// chains root-first, then both subtrees in pre-order (`a:hover span`
/// and inherited properties such as `color` reach descendants). Shared by
/// the hover and focus restyles.
pub(super) fn state_change_affected(
    doc: &html::dom::Document,
    old: Option<NodeId>,
    new: Option<NodeId>,
) -> Vec<NodeId> {
    let mut seen = rustc_hash::FxHashSet::default();
    let mut out = Vec::new();
    let starts: Vec<NodeId> = [old, new]
        .into_iter()
        .flatten()
        .filter(|&n| n < doc.nodes.len())
        .collect();
    for &start in &starts {
        let mut chain = Vec::new();
        let mut cur = Some(start);
        while let Some(nid) = cur {
            let Some(node) = doc.nodes.get(nid) else {
                break;
            };
            chain.push(nid);
            cur = node.parent;
        }
        for &nid in chain.iter().rev() {
            if seen.insert(nid) {
                out.push(nid);
            }
        }
    }
    for &start in &starts {
        let mut budget = MAX_STATE_RESTYLE_DESCENDANTS;
        let mut stack: Vec<NodeId> = doc.nodes[start].children.iter().rev().copied().collect();
        while let Some(nid) = stack.pop() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            if seen.insert(nid) {
                out.push(nid);
            }
            if let Some(node) = doc.nodes.get(nid) {
                stack.extend(node.children.iter().rev().copied());
            }
        }
    }
    out
}

/// All CSS properties that can be smoothly transitioned as a single f32.
pub(super) const TRANSITIONABLE_PROPERTIES: &[&str] = &[
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
pub(super) fn get_style_numeric_value(style: &ComputedStyle, property: &str) -> Option<f32> {
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
pub(super) fn try_start_property_transition(
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
pub(super) fn start_transitions_for_change(
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
