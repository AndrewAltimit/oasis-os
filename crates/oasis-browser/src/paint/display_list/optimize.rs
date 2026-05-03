//! Display-list compaction and optimization passes.
//!
//! Run after recording finishes, before replay. Reduces the number of
//! draw calls without changing pixels:
//!
//! - [`DisplayList::compact`] removes zero-size items and merges
//!   horizontally abutting same-color `FillRect` items.
//! - [`DisplayList::optimize`] additionally merges vertical strips and
//!   eliminates rects fully occluded by a later opaque `FillRect`.
//!
//! Both passes respect compositing-layer boundaries: rects inside a
//! `PushCompositingLayer` … `PopCompositingLayer` pair draw to a
//! different surface than rects outside, so they must never merge or
//! eliminate each other.

use super::{DisplayItem, DisplayList};

impl DisplayList {
    /// Compact the display list by removing degenerate items and merging
    /// consecutive `FillRect` items that share the same color and form a
    /// horizontal strip (same y, same height, abutting edges).
    ///
    /// This reduces draw call count without changing visual output or
    /// violating paint order — only truly consecutive same-type items
    /// are merged.
    pub fn compact(&mut self) {
        // Pass 1: remove zero-size items (no visual contribution).
        self.items.retain(|item| match item {
            DisplayItem::FillRect { w, h, .. }
            | DisplayItem::FillRoundedRect { w, h, .. }
            | DisplayItem::StrokeRoundedRect { w, h, .. }
            | DisplayItem::Blit { w, h, .. }
            | DisplayItem::Gradient { w, h, .. }
            | DisplayItem::BorderEdge { w, h, .. } => *w > 0 && *h > 0,
            // PushClip must never be removed — its PopClip is always
            // retained, and removing one without the other corrupts the
            // clip stack.  Zero-size clips are harmless (they just clip
            // everything inside to nothing).
            DisplayItem::PushClip { .. } => true,
            DisplayItem::BlitSub { dst_w, dst_h, .. } => *dst_w > 0 && *dst_h > 0,
            // Shadows are always retained: a 0x0 source with large spread/blur
            // still produces visible pixels.
            DisplayItem::Shadow { .. } => true,
            DisplayItem::DrawText { text, width, .. } => !text.is_empty() && *width > 0,
            // PopClip, PushLayer, PopLayer, BlurHint — always keep.
            _ => true,
        });

        // Pass 2: merge consecutive FillRect items with the same color and
        // height that form a horizontal strip (same y, abutting x + w == next x).
        if self.items.len() < 2 {
            return;
        }
        let mut merged: Vec<DisplayItem> = Vec::with_capacity(self.items.len());
        let mut drain = self.items.drain(..);
        let mut current = drain.next().expect("len >= 2");

        // Track compositing-layer depth: rects inside a layer target a
        // different surface from rects outside, so they must never
        // merge with each other. `current` sits at `current_layer`
        // depth; `next` sits at `next_layer` — if they differ the rects
        // are on different surfaces and merging would be incorrect.
        let mut current_layer: usize =
            matches!(&current, DisplayItem::PushCompositingLayer { .. }) as usize;
        for next in drain {
            let next_layer = match &next {
                DisplayItem::PushCompositingLayer { .. } => current_layer + 1,
                DisplayItem::PopCompositingLayer => current_layer.saturating_sub(1),
                _ => current_layer,
            };

            if current_layer == next_layer
                && let (
                    DisplayItem::FillRect {
                        x: cx,
                        y: cy,
                        w: cw,
                        h: ch,
                        color: cc,
                        ..
                    },
                    DisplayItem::FillRect {
                        x: nx,
                        y: ny,
                        w: nw,
                        h: nh,
                        color: nc,
                        ..
                    },
                ) = (&current, &next)
            {
                // Same color, same y, same height, horizontally abutting?
                // Note: node_id is intentionally NOT compared here so that
                // adjacent rects from different DOM nodes still merge.
                // This keeps the display list compact, which is critical on
                // PSP where the GU command buffer is limited.
                if cc == nc && cy == ny && ch == nh && cx + *cw as i32 == *nx {
                    current = DisplayItem::FillRect {
                        x: *cx,
                        y: *cy,
                        w: cw + nw,
                        h: *ch,
                        color: *cc,
                        // Merged rects lose their node association since they
                        // span multiple nodes. patch_node_colors will skip them.
                        node_id: None,
                    };
                    continue;
                }
            }
            merged.push(current);
            current = next;
            current_layer = next_layer;
        }
        merged.push(current);
        self.items = merged;
    }

    /// Optimize the display list by merging and culling items.
    ///
    /// Call after [`compact()`](Self::compact) for additional optimizations:
    /// - Merge consecutive vertically abutting `FillRect` items (same x,
    ///   width, and color)
    /// - Eliminate opaque `FillRect` items fully occluded by a later opaque
    ///   `FillRect` within the same clip context
    ///
    /// These reduce draw call count and command buffer usage on all backends,
    /// which is critical on PSP where the GU command buffer is 1 MB.
    pub fn optimize(&mut self) {
        self.merge_vertical_strips();
        self.eliminate_occluded();
    }

    /// Merge consecutive `FillRect` items that form a vertical strip
    /// (same x, same width, same color, abutting y + h == next y).
    fn merge_vertical_strips(&mut self) {
        if self.items.len() < 2 {
            return;
        }
        let mut merged: Vec<DisplayItem> = Vec::with_capacity(self.items.len());
        let mut drain = self.items.drain(..);
        let mut current = drain.next().expect("len >= 2");

        let mut current_layer: usize =
            matches!(&current, DisplayItem::PushCompositingLayer { .. }) as usize;
        for next in drain {
            let next_layer = match &next {
                DisplayItem::PushCompositingLayer { .. } => current_layer + 1,
                DisplayItem::PopCompositingLayer => current_layer.saturating_sub(1),
                _ => current_layer,
            };
            if current_layer == next_layer
                && let (
                    DisplayItem::FillRect {
                        x: cx,
                        y: cy,
                        w: cw,
                        h: ch,
                        color: cc,
                        ..
                    },
                    DisplayItem::FillRect {
                        x: nx,
                        y: ny,
                        w: nw,
                        h: nh,
                        color: nc,
                        ..
                    },
                ) = (&current, &next)
            {
                // Same color, same x, same width, vertically abutting?
                if cc == nc && cx == nx && cw == nw && cy + *ch as i32 == *ny {
                    current = DisplayItem::FillRect {
                        x: *cx,
                        y: *cy,
                        w: *cw,
                        h: ch + nh,
                        color: *cc,
                        node_id: None,
                    };
                    continue;
                }
            }
            merged.push(current);
            current = next;
            current_layer = next_layer;
        }
        merged.push(current);
        self.items = merged;
    }

    /// Remove opaque `FillRect` items fully covered by a later opaque
    /// `FillRect` within the same clip level.
    ///
    /// Uses a backward scan with a small window (32 items) to keep the
    /// algorithm O(n × k) rather than O(n²). Only eliminates items in the
    /// same clip depth to preserve correctness.
    fn eliminate_occluded(&mut self) {
        if self.items.len() < 2 {
            return;
        }

        const SCAN_WINDOW: usize = 32;
        let mut clip_depth: usize = 0;
        let mut sticky_depth: usize = 0;
        let mut compositing_depth: usize = 0;
        let mut clip_depths: Vec<usize> = Vec::with_capacity(self.items.len());
        let mut sticky_depths: Vec<usize> = Vec::with_capacity(self.items.len());
        let mut compositing_depths: Vec<usize> = Vec::with_capacity(self.items.len());
        let mut in_translucent: Vec<bool> = Vec::with_capacity(self.items.len());

        // First pass: compute clip/sticky depth and translucent-layer flag.
        let mut translucent_layer_depth: usize = 0;
        for item in &self.items {
            match item {
                DisplayItem::PushClip { .. } => {
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                    clip_depth += 1;
                },
                DisplayItem::PopClip => {
                    clip_depth = clip_depth.saturating_sub(1);
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                },
                DisplayItem::PushLayer { opacity } => {
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    if *opacity < 1.0 {
                        translucent_layer_depth += 1;
                    }
                    in_translucent.push(translucent_layer_depth > 0);
                },
                DisplayItem::PopLayer => {
                    translucent_layer_depth = translucent_layer_depth.saturating_sub(1);
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                },
                DisplayItem::PushCompositingLayer { .. } => {
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    // Contents of a compositing layer draw into an
                    // offscreen surface and may be re-blended with a
                    // non-Normal blend mode or filter — treat them as
                    // translucent so they can neither be eliminated
                    // nor act as occluders.
                    compositing_depth += 1;
                    translucent_layer_depth += 1;
                    in_translucent.push(true);
                },
                DisplayItem::PopCompositingLayer => {
                    compositing_depth = compositing_depth.saturating_sub(1);
                    translucent_layer_depth = translucent_layer_depth.saturating_sub(1);
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                },
                DisplayItem::PushSticky { .. } => {
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                    sticky_depth += 1;
                },
                DisplayItem::PopSticky => {
                    sticky_depth = sticky_depth.saturating_sub(1);
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                },
                _ => {
                    clip_depths.push(clip_depth);
                    sticky_depths.push(sticky_depth);
                    compositing_depths.push(compositing_depth);
                    in_translucent.push(translucent_layer_depth > 0);
                },
            }
        }

        // Second pass: mark items fully occluded by later opaque rects.
        let mut remove = vec![false; self.items.len()];

        for i in 1..self.items.len() {
            // Only opaque FillRect items outside translucent layers can occlude.
            if in_translucent[i] {
                continue;
            }
            let (cx, cy, cw, ch) = match &self.items[i] {
                DisplayItem::FillRect {
                    x, y, w, h, color, ..
                } if color.a == 255 => (*x, *y, *w, *h),
                _ => continue,
            };

            let my_clip = clip_depths[i];
            let my_sticky = sticky_depths[i];
            let my_compositing = compositing_depths[i];
            let start = i.saturating_sub(SCAN_WINDOW);

            for j in start..i {
                if remove[j]
                    || clip_depths[j] != my_clip
                    || sticky_depths[j] != my_sticky
                    || compositing_depths[j] != my_compositing
                {
                    continue;
                }

                let (ox, oy, ow, oh) = match &self.items[j] {
                    DisplayItem::FillRect { x, y, w, h, .. } => (*x, *y, *w, *h),
                    _ => continue,
                };

                // Is the earlier rect fully contained within the covering rect?
                if ox >= cx
                    && oy >= cy
                    && ox + ow as i32 <= cx + cw as i32
                    && oy + oh as i32 <= cy + ch as i32
                {
                    remove[j] = true;
                }
            }
        }

        // Third pass: remove marked items.
        let mut idx = 0;
        self.items.retain(|_| {
            let keep = !remove[idx];
            idx += 1;
            keep
        });
    }
}

#[cfg(test)]
mod tests {
    use oasis_types::backend::Color;

    use super::super::{DisplayItem, DisplayList};
    use crate::css::values::types::BlendMode as CssBlendMode;
    use crate::layout::box_model::Rect;

    fn layer_rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    fn push_cl(bounds: Rect, opacity: f32) -> DisplayItem {
        DisplayItem::PushCompositingLayer {
            bounds,
            opacity,
            blend: CssBlendMode::Multiply,
            needs_backdrop: false,
            filters: Vec::new(),
            backdrop_filters: Vec::new(),
            mask: None,
        }
    }

    #[test]
    fn compact_removes_zero_size_items() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 0,
            h: 10,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 0,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color: Color::rgb(0, 255, 0),
            node_id: None,
        });
        assert_eq!(dl.len(), 3);
        dl.compact();
        assert_eq!(dl.len(), 1);
        if let DisplayItem::FillRect { color, .. } = &dl.items()[0] {
            assert_eq!(*color, Color::rgb(0, 255, 0));
        } else {
            panic!("expected FillRect");
        }
    }

    #[test]
    fn compact_merges_horizontal_fill_rects() {
        let mut dl = DisplayList::new();
        let color = Color::rgb(100, 100, 100);
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 20,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.compact();
        assert_eq!(dl.len(), 1);
        if let DisplayItem::FillRect { x, y, w, h, .. } = &dl.items()[0] {
            assert_eq!(*x, 0);
            assert_eq!(*y, 0);
            assert_eq!(*w, 30);
            assert_eq!(*h, 5);
        } else {
            panic!("expected FillRect");
        }
    }

    #[test]
    fn compact_does_not_merge_different_colors() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 5,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 0,
            w: 10,
            h: 5,
            color: Color::rgb(0, 255, 0),
            node_id: None,
        });
        dl.compact();
        assert_eq!(dl.len(), 2);
    }

    #[test]
    fn compact_does_not_merge_non_abutting() {
        let mut dl = DisplayList::new();
        let color = Color::rgb(100, 100, 100);
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 15,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.compact();
        assert_eq!(dl.len(), 2);
    }

    #[test]
    fn compact_preserves_non_fill_rect_items() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::PushClip {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
        });
        dl.push(DisplayItem::DrawText {
            text: "hello".into(),
            x: 5,
            y: 5,
            font_size: 12,
            color: Color::rgb(0, 0, 0),
            bold: false,
            italic: false,
            width: 1,
            node_id: None,
            #[cfg(feature = "web-fonts")]
            web_font_id: None,
        });
        dl.push(DisplayItem::PopClip);
        dl.compact();
        assert_eq!(dl.len(), 3);
    }

    #[test]
    fn compact_preserves_push_pop_layer() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::PushLayer { opacity: 0.5 });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        dl.push(DisplayItem::PopLayer);
        dl.compact();
        assert_eq!(dl.len(), 3);
    }

    #[test]
    fn optimize_merges_vertical_strips() {
        let mut dl = DisplayList::new();
        let color = Color::rgb(100, 100, 100);
        dl.push(DisplayItem::FillRect {
            x: 5,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 5,
            y: 5,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 5,
            y: 10,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.optimize();
        assert_eq!(dl.len(), 1);
        if let DisplayItem::FillRect { x, y, w, h, .. } = &dl.items()[0] {
            assert_eq!(*x, 5);
            assert_eq!(*y, 0);
            assert_eq!(*w, 10);
            assert_eq!(*h, 15);
        } else {
            panic!("expected FillRect");
        }
    }

    #[test]
    fn optimize_no_vertical_merge_different_width() {
        let mut dl = DisplayList::new();
        let color = Color::rgb(100, 100, 100);
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 5,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 5,
            w: 20,
            h: 5,
            color,
            node_id: None,
        });
        dl.optimize();
        assert_eq!(dl.len(), 2);
    }

    #[test]
    fn optimize_eliminates_occluded_rect() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 10,
            w: 20,
            h: 20,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
            color: Color::rgb(0, 255, 0),
            node_id: None,
        });
        dl.optimize();
        assert_eq!(dl.len(), 1);
        if let DisplayItem::FillRect { color, .. } = &dl.items()[0] {
            assert_eq!(*color, Color::rgb(0, 255, 0));
        } else {
            panic!("expected FillRect");
        }
    }

    #[test]
    fn optimize_does_not_eliminate_semi_transparent() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 10,
            w: 20,
            h: 20,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
            color: Color::rgba(0, 255, 0, 128),
            node_id: None,
        });
        dl.optimize();
        assert_eq!(dl.len(), 2);
    }

    #[test]
    fn optimize_respects_clip_depth() {
        let mut dl = DisplayList::new();
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 10,
            w: 20,
            h: 20,
            color: Color::rgb(255, 0, 0),
            node_id: None,
        });
        dl.push(DisplayItem::PushClip {
            x: 0,
            y: 0,
            w: 200,
            h: 200,
        });
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
            color: Color::rgb(0, 255, 0),
            node_id: None,
        });
        dl.push(DisplayItem::PopClip);
        dl.optimize();
        assert_eq!(dl.len(), 4);
    }

    #[test]
    fn compact_does_not_merge_fillrects_across_compositing_layer() {
        let mut dl = DisplayList::new();
        let color = Color::rgb(10, 20, 30);
        dl.push(push_cl(layer_rect(0.0, 0.0, 200.0, 200.0), 1.0));
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::PopCompositingLayer);
        dl.push(DisplayItem::FillRect {
            x: 10,
            y: 0,
            w: 10,
            h: 10,
            color,
            node_id: None,
        });

        dl.compact();
        let fill_count = dl
            .items()
            .iter()
            .filter(|it| matches!(it, DisplayItem::FillRect { .. }))
            .count();
        assert_eq!(
            fill_count,
            2,
            "rects must not merge across layers: {:#?}",
            dl.items()
        );
    }

    #[test]
    fn merge_vertical_strips_does_not_cross_compositing_layer() {
        let mut dl = DisplayList::new();
        let color = Color::rgb(10, 20, 30);
        dl.push(push_cl(layer_rect(0.0, 0.0, 200.0, 200.0), 1.0));
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color,
            node_id: None,
        });
        dl.push(DisplayItem::PopCompositingLayer);
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 10,
            w: 10,
            h: 10,
            color,
            node_id: None,
        });
        dl.optimize();
        let fill_count = dl
            .items()
            .iter()
            .filter(|it| matches!(it, DisplayItem::FillRect { .. }))
            .count();
        assert_eq!(fill_count, 2);
    }

    #[test]
    fn eliminate_occluded_does_not_cross_compositing_layer() {
        let mut dl = DisplayList::new();
        dl.push(push_cl(layer_rect(0.0, 0.0, 200.0, 200.0), 1.0));
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 50,
            h: 50,
            color: Color::rgba(255, 0, 0, 255),
            node_id: None,
        });
        dl.push(DisplayItem::PopCompositingLayer);
        dl.push(DisplayItem::FillRect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
            color: Color::rgba(0, 255, 0, 255),
            node_id: None,
        });
        let before = dl.len();
        dl.optimize();
        let after_fills = dl
            .items()
            .iter()
            .filter(|it| matches!(it, DisplayItem::FillRect { .. }))
            .count();
        assert_eq!(after_fills, 2, "both fills must survive: {:#?}", dl.items());
        assert!(dl.len() >= before.saturating_sub(1));
    }
}
