//! Finding the scroll container under the cursor.

use super::*;

impl BrowserWidget {
    /// Find the innermost nested scroll container under the current cursor
    /// position. Returns `None` if the cursor is not over any scroll container.
    pub(super) fn find_scroll_container_at_cursor(&self) -> Option<NodeId> {
        use crate::css::values::Overflow;

        let layout = self.layout_root.as_ref()?;
        let doc = self.document.as_ref()?;
        // Hit-test the last pointer position and walk up the DOM to the
        // nearest overflow container that actually has something to
        // scroll. (`hover_node` is only tracked for links, so it can't
        // stand in for the pointer position over ordinary content.)
        let (x, y) = self.last_cursor?;
        let (lx, ly) = self.screen_to_layout(x, y);
        let mut cur = layout.hit_test(lx, ly);
        while let Some(nid) = cur {
            let is_scroller = matches!(
                self.styles.get(nid),
                Some(Some(style)) if matches!(style.overflow, Overflow::Auto | Overflow::Scroll)
            );
            if is_scroller && Self::find_scroll_bounds(layout, nid).is_some_and(|b| b > 0.0) {
                return Some(nid);
            }
            cur = doc.nodes.get(nid).and_then(|n| n.parent);
        }
        None
    }

    /// Find the maximum scroll Y for a nested scroll container.
    ///
    /// Returns `content_height - box_height`, clamped to >= 0.
    pub(super) fn find_scroll_bounds(layout_box: &LayoutBox, nid: NodeId) -> Option<f32> {
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
