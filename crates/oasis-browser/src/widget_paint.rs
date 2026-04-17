//! Rendering and paint methods for [`BrowserWidget`].

use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;
use oasis_vfs::Vfs;

use crate::css::values::ComputedStyle;
use crate::html::dom::NodeId;
use crate::layout::box_model::{BoxType, LayoutBox, Rect, ReplacedContent};
use crate::paint;
use crate::{BrowserWidget, Focus, LoadingState};

/// Truncate `text` to the longest UTF-8 prefix that renders in
/// `max_width_px` or fewer pixels at the given font size.
///
/// Binary search over character boundaries keeps the cost O(log n) in
/// string length and avoids the "cut through a multi-byte codepoint"
/// panic that a naive byte slice would hit. Used by the URL bar so
/// long URLs clip at a legible glyph edge instead of a garbled
/// half-character.
fn truncate_to_pixels(text: &str, font_size: u16, max_width_px: i32) -> &str {
    use oasis_types::backend::bitmap_measure_text;
    if max_width_px <= 0 {
        return "";
    }
    if bitmap_measure_text(text, font_size) as i32 <= max_width_px {
        return text;
    }
    // Binary-search for the largest char-boundary end offset that fits.
    let mut lo = 0;
    let mut hi = text.len();
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let mid = text.floor_char_boundary(mid);
        if mid == lo {
            break;
        }
        if bitmap_measure_text(&text[..mid], font_size) as i32 <= max_width_px {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    &text[..text.floor_char_boundary(lo)]
}

impl BrowserWidget {
    // ---------------------------------------------------------------
    // Painting
    // ---------------------------------------------------------------

    /// Paint the browser to the backend.
    ///
    /// Draws chrome (URL bar, navigation buttons, status bar) and
    /// the page content viewport.
    /// Per-frame update: process pending image loads within a time budget,
    /// advance CSS animations and transitions.
    ///
    /// Call this once per frame before `paint()`. Images stream in
    /// progressively so the page is never blocked waiting for all images.
    pub fn tick(&mut self, vfs: &dyn Vfs) {
        // Poll the I/O thread for completed network requests.
        #[cfg(not(any(target_arch = "wasm32", feature = "psp")))]
        self.poll_io_thread();

        self.load_next_image_batch(vfs, 8);

        // Load pending web fonts (first tick after page load).
        #[cfg(feature = "web-fonts")]
        self.load_web_fonts(vfs);

        // Compute frame delta for animations/transitions.
        let now = std::time::Instant::now();
        let dt_ms = self
            .last_tick_time
            .map_or(16.0, |prev| prev.elapsed().as_secs_f32() * 1000.0);
        self.last_tick_time = Some(now);

        // Advance CSS animations and transitions.
        let anim_active = self.animation_engine.tick(dt_ms);
        let trans_active = self.transition_engine.tick(dt_ms);

        // Apply transition overrides to styles so layout and paint see
        // the interpolated values.
        if trans_active {
            for (nid, prop) in self.transition_engine.active_node_properties() {
                if let Some(val) = self.transition_engine.get_node_value(nid, prop)
                    && let Some(Some(style)) = self.styles.get_mut(nid)
                {
                    apply_transition_value(style, prop, val);
                }
            }
        }

        // Apply animation overrides to styles. The animation engine
        // interpolates numeric keyframe values each frame — feed them
        // into ComputedStyle so layout and paint see the animated state.
        if anim_active {
            for (nid, _) in self.animation_engine.active_node_properties() {
                let overrides = self.animation_engine.get_overrides(nid);
                for (prop, val) in &overrides {
                    if let Some(Some(style)) = self.styles.get_mut(nid) {
                        apply_transition_value(style, prop, *val);
                    }
                }
            }
        }

        if anim_active || trans_active {
            // Check if all animated properties are visual-only (color changes
            // that don't affect layout). If so, use dirty-rect repainting
            // instead of a full layout rebuild.
            let mut all_visual = true;
            let mut nodes: Vec<usize> = Vec::new();

            for (nid, props) in self.animation_engine.active_node_properties() {
                if props.iter().all(|p| is_visual_only_property(p)) {
                    nodes.push(nid);
                } else {
                    all_visual = false;
                    break;
                }
            }
            if all_visual {
                for (nid, prop) in self.transition_engine.active_node_properties() {
                    if is_visual_only_property(prop) {
                        nodes.push(nid);
                    } else {
                        all_visual = false;
                        break;
                    }
                }
            }

            if all_visual && !nodes.is_empty() {
                // Compute dirty rects for animated nodes so the display list
                // can be patched and replayed without a full rebuild.
                if let Some(layout) = &self.layout_root {
                    for nid in nodes {
                        if let Some(rect) = Self::find_node_rect(layout, nid) {
                            self.dirty_rects.push(rect);
                        }
                    }
                }
            } else {
                self.layout_dirty = true;
            }
        }

        // Tick JS timers (setTimeout / setInterval).
        #[cfg(feature = "javascript")]
        if let Some(engine) = &self.js_engine {
            let fired = engine.tick_timers(dt_ms as f64);
            if fired > 0 {
                self.layout_dirty = true;
            }
        }

        // Execute deferred scripts after first paint.
        #[cfg(feature = "javascript")]
        if !self.deferred_scripts.is_empty() && !self.display_list.is_empty() {
            self.execute_deferred_scripts();
        }

        // Process pending JS navigation actions.
        #[cfg(feature = "javascript")]
        {
            use crate::js_dom;
            let actions = js_dom::drain_nav_actions(&self.js_nav_actions);
            for action in actions {
                match action {
                    js_dom::JsNavAction::Navigate(url) => {
                        self.navigate_to(&url, vfs);
                    },
                    js_dom::JsNavAction::Back => {
                        self.go_back(vfs);
                    },
                    js_dom::JsNavAction::Forward => {
                        self.go_forward(vfs);
                    },
                }
            }
        }
    }

    pub fn paint(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
        // Rebuild layout if the viewport was resized since last paint.
        let layout_changed = self.relayout_if_dirty();

        // Set clip to our window area.
        backend.set_clip_rect(self.window_x, self.window_y, self.window_w, self.window_h)?;

        // Paint chrome (URL bar + buttons).
        self.paint_chrome(backend)?;

        // Close the window clip before opening a tighter content clip.
        backend.reset_clip_rect()?;

        // Content viewport.
        let content_y = self.window_y + self.config.url_bar_height as i32;
        let content_h = self.config.content_height(self.window_h);

        backend.set_clip_rect(self.window_x, content_y, self.window_w, content_h)?;

        // Paint page background.
        backend.fill_rect(
            self.window_x,
            content_y,
            self.window_w,
            content_h,
            self.config.default_bg_color,
        )?;

        // Upload decoded images as GPU textures and assign to layout
        // nodes (first paint after navigation).
        self.ensure_image_textures(backend);

        // Collect @counter-style rules from author stylesheets for
        // list-marker rendering.
        let counter_styles: Vec<_> = self
            .cached_author_sheets
            .iter()
            .flat_map(|s| s.counter_styles.iter().cloned())
            .collect();

        // Paint layout tree via cached display list.
        if let Some(layout) = &self.layout_root {
            // Rebuild display list when layout changed or on first paint.
            // The display list is also cleared by load_html on navigation.
            let needs_rebuild =
                layout_changed || self.display_list.is_empty() || self.full_repaint_needed;

            // Capture dirty rects before clearing them.
            let has_dirty_rects = !self.dirty_rects.is_empty();

            if needs_rebuild {
                // Extend the viewport height by the scroll buffer zone so items
                // slightly beyond the visible area are recorded. This means small
                // scroll increments can replay the cached display list without a
                // full rebuild (the extra items are already present).
                let buffered_h = content_h as f32 + self.scroll.buffer_zone as f32;
                let viewport = paint::PaintViewport {
                    scroll_y: self.scroll.scroll_y as f32,
                    scroll_x: self.scroll.scroll_x as f32,
                    x: self.window_x,
                    y: content_y,
                    width: self.window_w as f32,
                    height: buffered_h,
                    visible_height: content_h as f32,
                    focused_node: self.focused_node,
                    counter_styles: counter_styles.clone(),
                };

                // Record to display list (no draw calls emitted).
                let links = paint::record::record_with_scroll(
                    layout,
                    viewport,
                    &self.href_map,
                    &mut self.display_list,
                    &self.nested_scroll_offsets,
                );
                // Compact and optimize the display list.
                self.display_list.compact();
                self.display_list.optimize();
                self.link_map = links;
                self.scroll
                    .set_content_height(layout.dimensions.margin_box().height as i32);
                self.display_list_scroll_y = self.scroll.scroll_y;
                self.display_list_scroll_x = self.scroll.scroll_x;
                self.link_map_scroll_y = self.scroll.scroll_y;
                self.link_map_scroll_x = self.scroll.scroll_x;

                // Update tile grid on layout change.
                let ch = layout.dimensions.margin_box().height as u32;
                match &mut self.tile_grid {
                    Some(grid) => grid.resize(self.window_w, ch),
                    None => {
                        self.tile_grid = Some(paint::tiling::TileGrid::new(self.window_w, ch));
                    },
                }

                // Replay from the freshly built display list.
                self.replay_display_list(
                    backend,
                    0,
                    0,
                    Some((self.window_x, content_y, self.window_w, content_h)),
                )?;
                self.dirty_rects.clear();
                self.full_repaint_needed = false;
            } else if has_dirty_rects {
                // Visual-only change (e.g. hover color) with known dirty rects.
                // If the display list already has items, patch colors in-place
                // for affected nodes instead of rebuilding the entire list.
                if !self.display_list.is_empty() {
                    // Patch colors for affected nodes from their updated styles.
                    for dirty_rect in &self.dirty_rects {
                        // Find nodes whose layout boxes overlap this dirty rect.
                        if let Some(layout_root) = &self.layout_root {
                            let affected = Self::find_nodes_in_rect(layout_root, dirty_rect);
                            for nid in affected {
                                if let Some(Some(style)) = self.styles.get(nid) {
                                    self.display_list.patch_node_colors(
                                        nid,
                                        style.background_color,
                                        style.color,
                                    );
                                }
                            }
                        }
                    }

                    // Replay only items intersecting the dirty rectangles.
                    let dirty_copy: Vec<_> = self.dirty_rects.clone();
                    for dirty in &dirty_copy {
                        self.replay_dirty_display_list(
                            backend,
                            dirty,
                            self.display_list_scroll_y - self.scroll.scroll_y,
                            self.display_list_scroll_x - self.scroll.scroll_x,
                            Some((self.window_x, content_y, self.window_w, content_h)),
                        )?;
                    }
                } else {
                    // First paint: no display list yet, do a full record.
                    let buffered_h = content_h as f32 + self.scroll.buffer_zone as f32;
                    let viewport = paint::PaintViewport {
                        scroll_y: self.scroll.scroll_y as f32,
                        scroll_x: self.scroll.scroll_x as f32,
                        x: self.window_x,
                        y: content_y,
                        width: self.window_w as f32,
                        height: buffered_h,
                        visible_height: content_h as f32,
                        focused_node: self.focused_node,
                        counter_styles: counter_styles.clone(),
                    };
                    let links = paint::record::record(
                        layout,
                        viewport,
                        &self.href_map,
                        &mut self.display_list,
                    );
                    self.display_list.compact();
                    self.display_list.optimize();
                    self.link_map = links;
                    self.display_list_scroll_y = self.scroll.scroll_y;
                    self.display_list_scroll_x = self.scroll.scroll_x;
                    self.link_map_scroll_y = self.scroll.scroll_y;
                    self.link_map_scroll_x = self.scroll.scroll_x;

                    let dirty_copy2: Vec<_> = self.dirty_rects.clone();
                    for dirty in &dirty_copy2 {
                        self.replay_dirty_display_list(
                            backend,
                            dirty,
                            0,
                            0,
                            Some((self.window_x, content_y, self.window_w, content_h)),
                        )?;
                    }
                }
                self.dirty_rects.clear();
            } else {
                // Scroll changed but layout didn't — replay with scroll delta.
                let dy = self.display_list_scroll_y - self.scroll.scroll_y;
                let dx = self.display_list_scroll_x - self.scroll.scroll_x;

                if dx != 0 || dy != 0 {
                    // Check if we can reuse the cached display list by replaying
                    // with a translation offset instead of re-recording.
                    //
                    // This is safe when the scroll delta is within the buffer
                    // zone (items beyond the visible area were already recorded).
                    // Sticky elements are handled via PushSticky/PopSticky
                    // which recompute their offset during replay.
                    let abs_dy = dy.unsigned_abs() as f32;
                    let abs_dx = dx.unsigned_abs() as f32;
                    let can_reuse =
                        abs_dy <= self.scroll.buffer_zone as f32 && abs_dx <= self.window_w as f32;

                    if can_reuse {
                        // Shift link_map regions by the per-frame scroll delta
                        // so hit testing remains accurate without re-recording.
                        // link_map_scroll_y/x tracks what the link positions
                        // are currently adjusted to; the per-frame delta is the
                        // difference between the current scroll and that value.
                        let link_dy = self.link_map_scroll_y - self.scroll.scroll_y;
                        let link_dx = self.link_map_scroll_x - self.scroll.scroll_x;
                        if link_dx != 0 || link_dy != 0 {
                            for link in &mut self.link_map {
                                link.rect.x += link_dx as f32;
                                link.rect.y += link_dy as f32;
                            }
                            self.link_map_scroll_y = self.scroll.scroll_y;
                            self.link_map_scroll_x = self.scroll.scroll_x;
                        }

                        // Replay the cached display list with the TOTAL scroll
                        // delta from recording time applied as a translation
                        // offset. display_list_scroll_y/x is NOT updated here
                        // — it stays at the recording-time value so the
                        // cumulative offset remains correct across frames.
                        self.replay_display_list(
                            backend,
                            dx,
                            dy,
                            Some((self.window_x, content_y, self.window_w, content_h)),
                        )?;
                    } else {
                        // Scroll exceeded the buffer zone or page has sticky
                        // elements — must rebuild the display list.
                        let buffered_h = content_h as f32 + self.scroll.buffer_zone as f32;
                        let viewport = paint::PaintViewport {
                            scroll_y: self.scroll.scroll_y as f32,
                            scroll_x: self.scroll.scroll_x as f32,
                            x: self.window_x,
                            y: content_y,
                            width: self.window_w as f32,
                            height: buffered_h,
                            visible_height: content_h as f32,
                            focused_node: self.focused_node,
                            counter_styles: counter_styles.clone(),
                        };
                        let links = paint::record::record(
                            layout,
                            viewport,
                            &self.href_map,
                            &mut self.display_list,
                        );
                        self.display_list.compact();
                        self.display_list.optimize();
                        self.link_map = links;
                        self.display_list_scroll_y = self.scroll.scroll_y;
                        self.display_list_scroll_x = self.scroll.scroll_x;
                        self.link_map_scroll_y = self.scroll.scroll_y;
                        self.link_map_scroll_x = self.scroll.scroll_x;

                        self.replay_display_list(
                            backend,
                            0,
                            0,
                            Some((self.window_x, content_y, self.window_w, content_h)),
                        )?;
                    }

                    // Mark newly visible tiles as dirty on scroll.
                    if let Some(grid) = &mut self.tile_grid {
                        let (vis_start, vis_end) =
                            grid.visible_range(self.scroll.scroll_y, content_h);
                        for idx in vis_start..vis_end {
                            if grid.is_dirty(idx) {
                                // Tile is already dirty — will be re-rendered.
                                // Future: skip replay for clean tiles.
                            }
                        }
                    }
                } else {
                    // Same scroll, same layout — replay cached display list.
                    self.replay_display_list(
                        backend,
                        0,
                        0,
                        Some((self.window_x, content_y, self.window_w, content_h)),
                    )?;
                }
            }
        }

        // Paint SVG/Canvas elements that can't be represented in the display list.
        if let Some(layout) = &self.layout_root {
            Self::paint_svg_canvas_elements(
                layout,
                backend,
                self.scroll.scroll_x as f32,
                self.scroll.scroll_y as f32,
                content_h as f32,
                self.window_x,
                content_y,
            )?;
        }

        // Paint link highlight if a link is selected.
        if self.selected_link >= 0 {
            let idx = self.selected_link as usize;
            if idx < self.link_map.len() {
                let link = self.link_map[idx].clone();
                paint::paint_link_highlight(&link, backend, Color::rgb(255, 200, 0))?;
            }
        }

        // Paint focus indicator around the focused form element.
        if let Some(focused_nid) = self.focused_node {
            self.paint_focus_indicator(backend, focused_nid, content_y)?;
        }

        // Paint scrollbar when content overflows viewport.
        if self.scroll.max_scroll() > 0 {
            let sb_w: u32 = 6;
            let sb_x = self.window_x + self.window_w as i32 - sb_w as i32 - 1;
            let track_y = content_y;
            let track_h = content_h;

            // Track.
            backend.fill_rect(sb_x, track_y, sb_w, track_h, Color::rgba(255, 255, 255, 20))?;

            // Thumb: proportional to viewport/content ratio.
            let ratio = self.scroll.viewport_height as f32 / self.scroll.content_height as f32;
            let thumb_h = ((track_h as f32 * ratio) as u32).max(12).min(track_h);
            let scrollable = track_h - thumb_h;
            let frac = self.scroll.scroll_fraction();
            let thumb_y = track_y + (scrollable as f32 * frac) as i32;
            backend.fill_rect(
                sb_x,
                thumb_y,
                sb_w,
                thumb_h,
                Color::rgba(255, 255, 255, 100),
            )?;
        }

        // Paint status bar.
        self.paint_status_bar(backend)?;

        backend.reset_clip_rect()?;

        Ok(())
    }

    /// Helper: replay the display list with web font support when available.
    #[cfg(feature = "web-fonts")]
    fn replay_display_list(
        &mut self,
        backend: &mut dyn SdiBackend,
        dx: i32,
        dy: i32,
        clip: Option<(i32, i32, u32, u32)>,
    ) -> Result<()> {
        if self.font_registry.borrow().has_fonts() {
            let mut renderer = crate::font::BrowserWebFontRenderer {
                registry: &self.font_registry,
                tex_cache: &mut self.glyph_tex_cache,
            };
            self.display_list
                .replay_with_fonts(backend, dx, dy, clip, &mut renderer)
        } else {
            self.display_list.replay(backend, dx, dy, clip)
        }
    }

    /// Helper: replay the display list (no web font support).
    #[cfg(not(feature = "web-fonts"))]
    fn replay_display_list(
        &mut self,
        backend: &mut dyn SdiBackend,
        dx: i32,
        dy: i32,
        clip: Option<(i32, i32, u32, u32)>,
    ) -> Result<()> {
        self.display_list.replay(backend, dx, dy, clip)
    }

    /// Helper: replay dirty rects with web font support when available.
    #[cfg(feature = "web-fonts")]
    fn replay_dirty_display_list(
        &mut self,
        backend: &mut dyn SdiBackend,
        dirty: &crate::layout::box_model::Rect,
        dx: i32,
        dy: i32,
        clip: Option<(i32, i32, u32, u32)>,
    ) -> Result<()> {
        if self.font_registry.borrow().has_fonts() {
            let mut renderer = crate::font::BrowserWebFontRenderer {
                registry: &self.font_registry,
                tex_cache: &mut self.glyph_tex_cache,
            };
            self.display_list
                .replay_dirty_with_fonts(backend, dirty, dx, dy, clip, &mut renderer)
        } else {
            self.display_list.replay_dirty(backend, dirty, dx, dy, clip)
        }
    }

    /// Helper: replay dirty rects (no web font support).
    #[cfg(not(feature = "web-fonts"))]
    fn replay_dirty_display_list(
        &mut self,
        backend: &mut dyn SdiBackend,
        dirty: &crate::layout::box_model::Rect,
        dx: i32,
        dy: i32,
        clip: Option<(i32, i32, u32, u32)>,
    ) -> Result<()> {
        self.display_list.replay_dirty(backend, dirty, dx, dy, clip)
    }

    /// Paint only the browser chrome (URL bar + status bar), skipping page
    /// content. Used when an external iframe is rendering the page.
    pub fn paint_chrome_only(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
        backend.set_clip_rect(self.window_x, self.window_y, self.window_w, self.window_h)?;
        self.paint_chrome(backend)?;
        self.paint_status_bar(backend)?;
        backend.reset_clip_rect()?;
        Ok(())
    }

    /// Paint the URL bar and navigation buttons.
    pub fn paint_chrome(&self, backend: &mut dyn SdiBackend) -> Result<()> {
        use oasis_types::backend::bitmap_measure_text;

        let h = self.config.url_bar_height;
        let bw = self.config.button_width;
        let themed = self.config.use_themed_chrome;
        let r: u16 = 4; // Chrome element border radius.

        // Font size for chrome labels. 14 fits comfortably in the 28-px
        // bar and is tall enough that the caret/selection don't clip.
        const LABEL_FS: u16 = 14;
        let label_h = LABEL_FS as i32;

        // Vertical center for label baselines inside the chrome bar.
        let label_y = self.window_y + (h as i32 - label_h) / 2;

        // Right-edge button layout: star (bookmark) + home.
        let home_x = self.window_x + self.window_w as i32 - bw as i32;
        let bookmark_x = home_x - bw as i32;

        // Chrome background.
        if themed {
            backend.fill_rounded_rect(
                self.window_x,
                self.window_y,
                self.window_w,
                h,
                r,
                self.config.chrome_bg,
            )?;
        } else {
            backend.fill_rect(
                self.window_x,
                self.window_y,
                self.window_w,
                h,
                self.config.chrome_bg,
            )?;
        }

        // Helper: paint a chrome button with a centered text label.
        let paint_button = |backend: &mut dyn SdiBackend,
                            x: i32,
                            label: &str,
                            enabled: bool,
                            highlight: Option<Color>|
         -> Result<()> {
            let fill = if let Some(hi) = highlight {
                hi
            } else if enabled {
                self.config.chrome_button_bg
            } else {
                self.config.chrome_bg
            };
            if themed {
                backend.fill_rounded_rect(x, self.window_y + 2, bw, h - 4, r, fill)?;
            } else {
                backend.fill_rect(x, self.window_y, bw, h, fill)?;
            }
            let label_w = bitmap_measure_text(label, LABEL_FS) as i32;
            let tx = x + (bw as i32 - label_w) / 2;
            let text_color = if enabled {
                self.config.chrome_text
            } else {
                // Dim disabled labels so their grey-on-grey doesn't
                // look like a rendering bug.
                Color::rgba(
                    self.config.chrome_text.r,
                    self.config.chrome_text.g,
                    self.config.chrome_text.b,
                    110,
                )
            };
            backend.draw_text(label, tx, label_y, LABEL_FS, text_color)?;
            Ok(())
        };

        // Back / Forward buttons.
        paint_button(backend, self.window_x, "<", self.nav.can_go_back(), None)?;
        paint_button(
            backend,
            self.window_x + bw as i32,
            ">",
            self.nav.can_go_forward(),
            None,
        )?;

        // URL bar.
        let url_x = self.window_x + (bw * 2) as i32;
        // Reserve room for two buttons on the right (bookmark + home).
        let url_w = self.window_w.saturating_sub(bw * 4);

        let bar_bg = if self.focus == Focus::UrlBar {
            Color::rgb(60, 60, 80)
        } else {
            self.config.url_bar_bg
        };
        if themed {
            backend.fill_rounded_rect(url_x, self.window_y + 2, url_w, h - 4, r, bar_bg)?;
            backend.stroke_rounded_rect(
                url_x,
                self.window_y + 2,
                url_w,
                h - 4,
                r,
                1,
                Color::rgba(255, 255, 255, 30),
            )?;
        } else {
            backend.fill_rect(url_x, self.window_y + 2, url_w, h - 4, bar_bg)?;
        }

        // URL text: editing buffer when focused, navigation URL otherwise.
        //
        // Cursor and selection are positioned with `bitmap_measure_text`
        // (the same measurer the paint pass uses for content text),
        // so the caret lands exactly on the glyph edge regardless of
        // variable character widths. The old hardcoded 8-px-per-char
        // was what made the caret feel misaligned.
        let text_x = url_x + 4;
        let text_max_w = url_w.saturating_sub(8) as i32;
        if self.focus == Focus::UrlBar {
            // Truncate the buffer to fit the visible bar, preserving
            // UTF-8 boundaries.
            let display = truncate_to_pixels(&self.url_input, LABEL_FS, text_max_w);

            // Selection highlight (behind the text).
            if let Some((lo, hi)) = self.url_selection_range() {
                let lo_px = bitmap_measure_text(&self.url_input[..lo], LABEL_FS) as i32;
                let hi_px = bitmap_measure_text(&self.url_input[..hi], LABEL_FS) as i32;
                let sel_x = (text_x + lo_px).max(text_x);
                let sel_end = (text_x + hi_px).min(url_x + url_w as i32 - 4);
                let sel_w = (sel_end - sel_x).max(0);
                if sel_w > 0 {
                    backend.fill_rect(
                        sel_x,
                        label_y - 1,
                        sel_w as u32,
                        (label_h + 2) as u32,
                        Color::rgb(66, 133, 244),
                    )?;
                }
            }

            backend.draw_text(display, text_x, label_y, LABEL_FS, self.config.url_bar_text)?;

            // Caret: 2-px wide vertical bar so it's actually visible at
            // 14-px font height.
            let cursor_px =
                bitmap_measure_text(&self.url_input[..self.url_cursor], LABEL_FS) as i32;
            let caret_x = text_x + cursor_px;
            if caret_x < url_x + url_w as i32 - 4 {
                backend.fill_rect(
                    caret_x,
                    label_y - 1,
                    2,
                    (label_h + 2) as u32,
                    self.config.url_bar_text,
                )?;
            }
        } else {
            let url_text = self.nav.current_url().unwrap_or("about:blank");
            let display_url = truncate_to_pixels(url_text, LABEL_FS, text_max_w);
            backend.draw_text(
                display_url,
                text_x,
                label_y,
                LABEL_FS,
                self.config.url_bar_text,
            )?;
        }

        // Bookmark button: highlighted when the current page is
        // bookmarked. A click navigates to `vfs://bookmarks` (the saved
        // list). We label it "B" rather than a unicode star because the
        // bitmap font in use only reliably covers ASCII — ★/☆ render
        // as tofu on several of our backends.
        let is_bookmarked = self.nav.is_bookmarked();
        let bookmark_highlight = if is_bookmarked {
            Some(Color::rgb(231, 176, 43))
        } else {
            None
        };
        paint_button(backend, bookmark_x, "B", true, bookmark_highlight)?;

        // Home button.
        paint_button(backend, home_x, "H", true, None)?;

        Ok(())
    }

    /// Paint the status bar at the bottom.
    pub fn paint_status_bar(&self, backend: &mut dyn SdiBackend) -> Result<()> {
        let sh = self.config.status_bar_height;
        let sy = self.window_y + self.window_h as i32 - sh as i32;

        backend.fill_rect(
            self.window_x,
            sy,
            self.window_w,
            sh,
            self.config.status_bar_bg,
        )?;

        // Status text.
        let status = match self.state {
            LoadingState::Idle => {
                if self.reader_mode {
                    "Reader mode"
                } else {
                    "Ready"
                }
            },
            LoadingState::Loading => "Loading...",
            LoadingState::Error => "Error",
        };
        backend.draw_text(
            status,
            self.window_x + 4,
            sy + 2,
            10,
            self.config.status_bar_text,
        )?;

        // Error count indicator (left of scroll).
        let mut right_edge = self.window_x + self.window_w as i32 - 4;
        let error_count = self.page_errors.len();
        if error_count > 0 {
            let err_text = format!(
                "{} error{}",
                error_count,
                if error_count == 1 { "" } else { "s" }
            );
            let err_w = oasis_types::backend::bitmap_measure_text(&err_text, 10) as i32;
            let err_color = Color::rgb(220, 50, 50);
            right_edge -= err_w + 8;
            backend.draw_text(&err_text, right_edge, sy + 2, 10, err_color)?;
        }

        // Scroll indicator on the right.
        let frac = self.scroll.scroll_fraction();
        let pct = (frac * 100.0) as u32;
        let scroll_text = format!("{}%", pct);
        let text_w = oasis_types::backend::bitmap_measure_text(&scroll_text, 10) as i32;
        backend.draw_text(
            &scroll_text,
            right_edge - text_w,
            sy + 2,
            10,
            self.config.status_bar_text,
        )?;

        Ok(())
    }

    /// Draw a 2px focus outline around the layout box for `node_id`.
    fn paint_focus_indicator(
        &self,
        backend: &mut dyn SdiBackend,
        node_id: NodeId,
        content_y: i32,
    ) -> Result<()> {
        let layout = match &self.layout_root {
            Some(l) => l,
            None => return Ok(()),
        };
        let Some(rect) = Self::find_node_rect(layout, node_id) else {
            return Ok(());
        };

        let scroll_y = self.scroll.scroll_y as f32;
        let scroll_x = self.scroll.scroll_x as f32;
        let x = (rect.x - scroll_x + self.window_x as f32) as i32 - 2;
        let y = (rect.y - scroll_y + content_y as f32) as i32 - 2;
        let w = rect.width as u32 + 4;
        let h = rect.height as u32 + 4;
        let focus_color = Color::rgb(0, 120, 212); // Blue focus ring.
        let thickness: u32 = 2;

        // Top edge
        backend.fill_rect(x, y, w, thickness, focus_color)?;
        // Bottom edge
        backend.fill_rect(x, y + h as i32, w, thickness, focus_color)?;
        // Left edge
        backend.fill_rect(x, y, thickness, h + thickness, focus_color)?;
        // Right edge
        backend.fill_rect(x + w as i32, y, thickness, h + thickness, focus_color)?;

        Ok(())
    }

    /// Find all DOM node IDs whose layout boxes overlap the given rectangle.
    fn find_nodes_in_rect(layout_box: &LayoutBox, rect: &Rect) -> Vec<NodeId> {
        let mut result = Vec::new();
        Self::collect_nodes_in_rect(layout_box, rect, &mut result);
        result
    }

    /// Recursive helper for `find_nodes_in_rect`.
    fn collect_nodes_in_rect(layout_box: &LayoutBox, rect: &Rect, result: &mut Vec<NodeId>) {
        let border = layout_box.dimensions.border_box();
        // Check overlap.
        if border.x + border.width >= rect.x
            && border.x <= rect.x + rect.width
            && border.y + border.height >= rect.y
            && border.y <= rect.y + rect.height
        {
            if let Some(nid) = layout_box.node {
                result.push(nid);
            }
            for child in &layout_box.children {
                Self::collect_nodes_in_rect(child, rect, result);
            }
        }
    }

    /// Find the border-box rectangle of a layout box associated with a DOM node.
    pub(crate) fn find_node_rect(layout_box: &LayoutBox, node_id: NodeId) -> Option<Rect> {
        if layout_box.node == Some(node_id) {
            return Some(layout_box.dimensions.border_box());
        }
        for child in &layout_box.children {
            if let Some(rect) = Self::find_node_rect(child, node_id) {
                return Some(rect);
            }
        }
        None
    }

    /// Paint SVG and Canvas replaced elements via immediate-mode rendering.
    ///
    /// The display list recorder skips these elements because they have
    /// complex internal drawing pipelines that can't be represented as
    /// display items. This method walks the layout tree after display list
    /// replay to paint them directly to the backend.
    ///
    /// Note: this post-pass means SVG/Canvas always render on top of
    /// display-list content. For correct z-ordering, these elements would
    /// need dedicated `DisplayItem` variants (future work).
    fn paint_svg_canvas_elements(
        layout_box: &LayoutBox,
        backend: &mut dyn SdiBackend,
        scroll_x: f32,
        scroll_y: f32,
        viewport_height: f32,
        offset_x: i32,
        offset_y: i32,
    ) -> Result<()> {
        use crate::css::values::{Dimension, Position};

        // Compute sticky offset (same logic as paint_box / record_box).
        let sticky_dy = if layout_box.style.position == Position::Sticky {
            if let Dimension::Px(top) = layout_box.style.top {
                let natural = layout_box.dimensions.content.y - scroll_y + offset_y as f32;
                if natural < top {
                    (top - natural) as i32
                } else {
                    0
                }
            } else if let Dimension::Px(bottom) = layout_box.style.bottom {
                let natural = layout_box.dimensions.content.y - scroll_y + offset_y as f32;
                let box_h = layout_box.dimensions.margin_box().height;
                let threshold = viewport_height - bottom - box_h;
                if natural > threshold {
                    (threshold - natural) as i32
                } else {
                    0
                }
            } else {
                0
            }
        } else {
            0
        };
        let offset_y = offset_y + sticky_dy;

        // Compute transform offsets (translate, scale, rotate, skew).
        let (tx_x, tx_y) = paint::compute_transform_offsets(
            &layout_box.style.transforms,
            layout_box.style.transform_origin.as_ref(),
            &layout_box.dimensions.content,
            offset_x,
            offset_y,
        );

        if let BoxType::Replaced(replaced) = &layout_box.box_type {
            let content = &layout_box.dimensions.content;
            let x = (content.x - scroll_x + tx_x as f32) as i32;
            let y = (content.y - scroll_y + tx_y as f32) as i32;
            match replaced {
                ReplacedContent::Svg { element } => {
                    crate::svg::paint_svg(element, backend, x, y, content.width, content.height)?;
                },
                ReplacedContent::Canvas { state } => {
                    let s = state.borrow();
                    crate::canvas::paint_canvas(
                        &s,
                        backend,
                        x,
                        y,
                        content.width as u32,
                        content.height as u32,
                    )?;
                },
                _ => {},
            }
        }
        for child in &layout_box.children {
            Self::paint_svg_canvas_elements(
                child,
                backend,
                scroll_x,
                scroll_y,
                viewport_height,
                tx_x,
                tx_y,
            )?;
        }
        Ok(())
    }
}

/// Whether a CSS property only affects visual appearance (color, opacity)
/// without changing layout geometry. Properties in this set can be updated
/// via dirty-rect repainting instead of triggering a full layout rebuild.
fn is_visual_only_property(prop: &str) -> bool {
    // NOTE: `opacity` is intentionally excluded — while it doesn't affect
    // layout, `patch_node_colors` only updates color fields, not PushLayer
    // opacity. Treating it as visual-only would cause opacity transitions
    // to stall because the display list never gets rebuilt.
    matches!(
        prop,
        "color"
            | "background-color"
            | "background"
            | "border-color"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "box-shadow"
            | "outline-color"
            | "visibility"
            | "text-decoration-color"
    )
}

/// Apply a single interpolated transition value to a [`ComputedStyle`].
fn apply_transition_value(style: &mut ComputedStyle, property: &str, value: f32) {
    match property {
        "opacity" => style.opacity = value,
        "font-size" => style.font_size = value,
        "line-height" => style.line_height = value,
        "letter-spacing" => style.letter_spacing = value,
        "word-spacing" => style.word_spacing = value,
        "margin-top" => style.margin_top = value,
        "margin-right" => style.margin_right = value,
        "margin-bottom" => style.margin_bottom = value,
        "margin-left" => style.margin_left = value,
        "padding-top" => style.padding_top = value,
        "padding-right" => style.padding_right = value,
        "padding-bottom" => style.padding_bottom = value,
        "padding-left" => style.padding_left = value,
        "border-top-width" => style.border_top_width = value,
        "border-right-width" => style.border_right_width = value,
        "border-bottom-width" => style.border_bottom_width = value,
        "border-left-width" => style.border_left_width = value,
        "border-radius" => style.border_radius = crate::css::values::BorderRadius::uniform(value),
        "border-spacing" => style.border_spacing = value,
        "outline-width" => style.outline_width = value,
        "outline-offset" => style.outline_offset = value,
        "text-indent" => style.text_indent = value,
        "gap" => style.gap = value,
        "column-gap" => style.column_gap = value,
        "row-gap" => style.row_gap = value,
        "flex-grow" => style.flex_grow = value,
        "flex-shrink" => style.flex_shrink = value,
        _ => {},
    }
}
