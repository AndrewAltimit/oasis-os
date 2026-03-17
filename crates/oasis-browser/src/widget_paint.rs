//! Rendering and paint methods for [`BrowserWidget`].

use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;
use oasis_vfs::Vfs;

use crate::html::dom::NodeId;
use crate::layout::box_model::{BoxType, LayoutBox, Rect, ReplacedContent};
use crate::paint;
use crate::{BrowserWidget, Focus, LoadingState};

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
        self.load_next_image_batch(vfs, 8);

        // Compute frame delta for animations/transitions.
        let now = std::time::Instant::now();
        let dt_ms = self
            .last_tick_time
            .map_or(16.0, |prev| prev.elapsed().as_secs_f32() * 1000.0);
        self.last_tick_time = Some(now);

        // Advance CSS animations and transitions.
        let anim_active = self.animation_engine.tick(dt_ms);
        let trans_active = self.transition_engine.tick(dt_ms);
        if anim_active || trans_active {
            // Animations or transitions changed values — repaint needed.
            self.layout_dirty = true;
        }

        // Tick JS timers (setTimeout / setInterval).
        #[cfg(feature = "javascript")]
        if let Some(engine) = &self.js_engine {
            let fired = engine.tick_timers(dt_ms as f64);
            if fired > 0 {
                self.layout_dirty = true;
            }
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
                };

                // Record to display list (no draw calls emitted).
                let links =
                    paint::record::record(layout, viewport, &self.href_map, &mut self.display_list);
                // Compact the display list (merge adjacent rects, remove zero-size items).
                self.display_list.compact();
                self.link_map = links;
                self.scroll
                    .set_content_height(layout.dimensions.margin_box().height as i32);
                self.display_list_scroll_y = self.scroll.scroll_y;
                self.display_list_scroll_x = self.scroll.scroll_x;

                // Update tile grid on layout change.
                let ch = layout.dimensions.margin_box().height as u32;
                match &mut self.tile_grid {
                    Some(grid) => grid.resize(self.window_w, ch),
                    None => {
                        self.tile_grid = Some(paint::tiling::TileGrid::new(self.window_w, ch));
                    },
                }

                // Replay from the freshly built display list.
                self.display_list.replay(
                    backend,
                    0,
                    0,
                    Some((self.window_x, content_y, self.window_w, content_h)),
                )?;
                self.dirty_rects.clear();
                self.full_repaint_needed = false;
            } else if has_dirty_rects {
                // Visual-only change (e.g. hover color) with known dirty rects.
                // Rebuild display list (colors are baked in) and replay only
                // the dirty regions to reduce backend draw calls.
                let buffered_h = content_h as f32 + self.scroll.buffer_zone as f32;
                let viewport = paint::PaintViewport {
                    scroll_y: self.scroll.scroll_y as f32,
                    scroll_x: self.scroll.scroll_x as f32,
                    x: self.window_x,
                    y: content_y,
                    width: self.window_w as f32,
                    height: buffered_h,
                };
                let links =
                    paint::record::record(layout, viewport, &self.href_map, &mut self.display_list);
                self.display_list.compact();
                self.link_map = links;
                self.display_list_scroll_y = self.scroll.scroll_y;
                self.display_list_scroll_x = self.scroll.scroll_x;

                // Replay only items intersecting the dirty rectangles.
                for dirty in &self.dirty_rects {
                    self.display_list.replay_dirty(
                        backend,
                        dirty,
                        0,
                        0,
                        Some((self.window_x, content_y, self.window_w, content_h)),
                    )?;
                }
                self.dirty_rects.clear();
            } else {
                // Scroll changed but layout didn't — replay with scroll delta.
                let dy = self.display_list_scroll_y - self.scroll.scroll_y;
                let dx = self.display_list_scroll_x - self.scroll.scroll_x;

                if dx != 0 || dy != 0 {
                    // Scroll moved: rebuild display list with new scroll offsets.
                    // (True scroll-only optimization with dirty rects comes in
                    // Phase 2 — for now, rebuild so link regions are correct.)
                    let buffered_h = content_h as f32 + self.scroll.buffer_zone as f32;
                    let viewport = paint::PaintViewport {
                        scroll_y: self.scroll.scroll_y as f32,
                        scroll_x: self.scroll.scroll_x as f32,
                        x: self.window_x,
                        y: content_y,
                        width: self.window_w as f32,
                        height: buffered_h,
                    };
                    let links = paint::record::record(
                        layout,
                        viewport,
                        &self.href_map,
                        &mut self.display_list,
                    );
                    self.display_list.compact();
                    self.link_map = links;
                    self.display_list_scroll_y = self.scroll.scroll_y;
                    self.display_list_scroll_x = self.scroll.scroll_x;

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

                    self.display_list.replay(
                        backend,
                        0,
                        0,
                        Some((self.window_x, content_y, self.window_w, content_h)),
                    )?;
                } else {
                    // Same scroll, same layout — replay cached display list.
                    self.display_list.replay(
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
        let h = self.config.url_bar_height;
        let bw = self.config.button_width;
        let themed = self.config.use_themed_chrome;
        let r: u16 = 4; // Chrome element border radius.

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

        // Back button.
        let back_color = if self.nav.can_go_back() {
            self.config.chrome_button_bg
        } else {
            self.config.chrome_bg
        };
        if themed {
            backend.fill_rounded_rect(self.window_x, self.window_y, bw, h, r, back_color)?;
        } else {
            backend.fill_rect(self.window_x, self.window_y, bw, h, back_color)?;
        }
        backend.draw_text(
            "<",
            self.window_x + 6,
            self.window_y + 4,
            12,
            self.config.chrome_text,
        )?;

        // Forward button.
        let fwd_color = if self.nav.can_go_forward() {
            self.config.chrome_button_bg
        } else {
            self.config.chrome_bg
        };
        if themed {
            backend.fill_rounded_rect(
                self.window_x + bw as i32,
                self.window_y,
                bw,
                h,
                r,
                fwd_color,
            )?;
        } else {
            backend.fill_rect(self.window_x + bw as i32, self.window_y, bw, h, fwd_color)?;
        }
        backend.draw_text(
            ">",
            self.window_x + bw as i32 + 6,
            self.window_y + 4,
            12,
            self.config.chrome_text,
        )?;

        // URL bar.
        let url_x = self.window_x + (bw * 2) as i32;
        let url_w = self.window_w.saturating_sub(bw * 3);

        // Use a highlighted background when the URL bar is focused.
        let bar_bg = if self.focus == Focus::UrlBar {
            Color::rgb(60, 60, 80)
        } else {
            self.config.url_bar_bg
        };
        if themed {
            backend.fill_rounded_rect(
                url_x,
                self.window_y + 2,
                url_w,
                h.saturating_sub(4),
                r,
                bar_bg,
            )?;
            // Stroke around URL bar for definition.
            backend.stroke_rounded_rect(
                url_x,
                self.window_y + 2,
                url_w,
                h.saturating_sub(4),
                r,
                1,
                Color::rgba(255, 255, 255, 30),
            )?;
        } else {
            backend.fill_rect(url_x, self.window_y + 2, url_w, h.saturating_sub(4), bar_bg)?;
        }

        // URL text: show the editing buffer when focused, otherwise
        // the current navigation URL.
        let max_chars = (url_w / 8).saturating_sub(1) as usize;
        if self.focus == Focus::UrlBar {
            // Show editing buffer with cursor indicator.
            let display = if self.url_input.len() > max_chars {
                &self.url_input[..self.url_input.floor_char_boundary(max_chars)]
            } else {
                &self.url_input
            };
            backend.draw_text(
                display,
                url_x + 4,
                self.window_y + 4,
                12,
                self.config.url_bar_text,
            )?;

            // Draw cursor line.
            let cursor_chars = self.url_input[..self.url_cursor].chars().count();
            let cursor_px = url_x + 4 + cursor_chars as i32 * 8;
            if cursor_px < url_x + url_w as i32 - 4 {
                backend.fill_rect(
                    cursor_px,
                    self.window_y + 3,
                    1,
                    h.saturating_sub(6),
                    self.config.url_bar_text,
                )?;
            }
        } else {
            let url_text = self.nav.current_url().unwrap_or("about:blank");
            let display_url = if url_text.len() > max_chars {
                &url_text[..url_text.floor_char_boundary(max_chars)]
            } else {
                url_text
            };
            backend.draw_text(
                display_url,
                url_x + 4,
                self.window_y + 4,
                12,
                self.config.url_bar_text,
            )?;
        }

        // Home button (rightmost).
        let home_x = self.window_x + self.window_w as i32 - bw as i32;
        if themed {
            backend.fill_rounded_rect(
                home_x,
                self.window_y + 2,
                bw,
                h.saturating_sub(4),
                r,
                self.config.chrome_button_bg,
            )?;
        } else {
            backend.fill_rect(home_x, self.window_y, bw, h, self.config.chrome_button_bg)?;
        }
        backend.draw_text(
            "H",
            home_x + 6,
            self.window_y + 4,
            12,
            self.config.chrome_text,
        )?;

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
                let viewport_h = (scroll_y + 272.0).max(0.0); // approximate
                let threshold = viewport_h - bottom - box_h;
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
            Self::paint_svg_canvas_elements(child, backend, scroll_x, scroll_y, tx_x, tx_y)?;
        }
        Ok(())
    }
}
