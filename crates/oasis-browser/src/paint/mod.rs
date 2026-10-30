//! Paint layer: turns the layout tree into a display list and replays it.
//!
//! There is one renderer. [`record`] walks the layout tree in CSS 2.1
//! painting order (appendix E: backgrounds and borders, negative
//! z-index stacking contexts, normal flow, positioned descendants,
//! non-negative z-index contexts) and emits [`display_list::DisplayItem`]s
//! — fills, text runs, image blits, gradients, SVG and `<canvas>`
//! content, plus clip, opacity, compositing-layer and sticky markers.
//! [`display_list::DisplayList`] then compacts and optimizes the list
//! and replays it against an [`SdiBackend`], carrying the clip stack,
//! opacity stack, render-target layers and sticky offsets.
//!
//! [`BrowserWidget::paint`](crate::BrowserWidget::paint) caches the
//! display list between frames and replays it with a scroll delta or a
//! dirty rectangle. [`paint`] is the one-shot form (record + replay) used
//! by tests, the visual-regression goldens and the paint benchmark, so
//! they all exercise the shipped path.
//!
//! The recorder also collects clickable link regions so the browser can
//! hit-test mouse clicks and PSP button presses.

mod background;
mod clip_path;
pub(crate) mod display_list;
pub(crate) mod filter_chain;
pub(crate) mod filters;
mod markers;
#[allow(clippy::too_many_arguments, clippy::collapsible_if)]
pub(crate) mod record;
mod replaced;
mod stacking;
#[cfg(test)]
mod tests;
mod transforms;

pub(crate) use clip_path::{intersect_rects, resolve_clip_path_rect};
pub(crate) use stacking::{creates_compositing_layer, creates_stacking_context, is_positioned};
pub(crate) use transforms::{
    resolve_perspective_origin, resolve_transform_origin, transforms_have_3d,
};

use std::collections::HashMap;

use crate::html::dom::NodeId;
use crate::layout::box_model::{LayoutBox, Rect};
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;

// -------------------------------------------------------------------
// Public types
// -------------------------------------------------------------------

/// Viewport and scroll parameters for painting.
#[derive(Debug, Clone)]
pub struct PaintViewport {
    /// Vertical scroll offset in pixels.
    pub scroll_y: f32,
    /// Horizontal scroll offset in pixels.
    pub scroll_x: f32,
    /// X origin of the viewport in screen coordinates.
    pub x: i32,
    /// Y origin of the viewport in screen coordinates.
    pub y: i32,
    /// Viewport width for culling off-screen content.
    pub width: f32,
    /// Viewport height for culling off-screen content (may include buffer zone).
    pub height: f32,
    /// True visible viewport height (excludes buffer zone).
    /// Used for sticky positioning. Defaults to `height` if not set.
    pub visible_height: f32,
    /// DOM node id that currently has keyboard focus, if any. Used by
    /// the recorder to draw a caret in focused text inputs and pick
    /// the `caret-color` from that element's computed style.
    pub focused_node: Option<NodeId>,
    /// `@counter-style` rules collected from stylesheets, used for
    /// custom list-marker formatting. Empty by default.
    pub counter_styles: Vec<crate::css::parser::CounterStyleRule>,
}

/// A clickable link region recorded during painting.
#[derive(Debug, Clone)]
pub struct LinkRegion {
    /// Screen-space bounding rectangle of the link.
    pub rect: Rect,
    /// The `href` attribute value.
    pub href: String,
    /// The DOM node this link originates from.
    pub node: NodeId,
}

/// The result of a paint pass.
pub struct PaintResult {
    /// Link hit regions recorded during this paint pass.
    pub links: Vec<LinkRegion>,
    /// Total content height in layout pixels (for scroll calculations).
    pub content_height: f32,
}

// -------------------------------------------------------------------
// Entry points
// -------------------------------------------------------------------

/// Record `layout` into `display_list` and run the post-record
/// compaction and optimization passes.
///
/// This is the single recording entry point shared by the browser
/// widget and [`paint`], so tests, goldens and benches exercise
/// exactly the display list production replays.
pub(crate) fn build_display_list(
    layout: &LayoutBox,
    viewport: PaintViewport,
    link_map: &HashMap<NodeId, String>,
    display_list: &mut display_list::DisplayList,
    nested_scroll_offsets: &HashMap<NodeId, (f32, f32)>,
) -> Vec<LinkRegion> {
    let links = record::record(
        layout,
        viewport,
        link_map,
        display_list,
        nested_scroll_offsets,
    );
    display_list.compact();
    display_list.optimize();
    links
}

/// Paint a layout tree to the backend.
///
/// Records a display list (the same path `BrowserWidget::paint` uses),
/// then replays it into `backend` with the viewport rectangle
/// `(x, y, width, visible_height)` as the base clip.
///
/// `link_map` maps DOM `NodeId`s of `<a>` elements to their `href`
/// attribute values. This is built by the style/layout phase and passed
/// in so the paint layer can record clickable regions without needing
/// access to the DOM.
pub fn paint(
    layout: &LayoutBox,
    backend: &mut dyn SdiBackend,
    viewport: PaintViewport,
    link_map: &HashMap<NodeId, String>,
) -> Result<PaintResult> {
    let base_clip = (
        viewport.x,
        viewport.y,
        viewport.width.max(0.0) as u32,
        viewport.visible_height.max(0.0) as u32,
    );
    let mut dl = display_list::DisplayList::new();
    let links = build_display_list(layout, viewport, link_map, &mut dl, &HashMap::new());
    dl.replay(backend, 0, 0, Some(base_clip))?;
    Ok(PaintResult {
        links,
        content_height: layout.scrollable_height(),
    })
}

/// Paint a highlight rectangle around a link region.
///
/// Used for PSP-style tab navigation where the currently focused link
/// is outlined with a visible border.
pub fn paint_link_highlight(
    link: &LinkRegion,
    backend: &mut dyn SdiBackend,
    highlight_color: Color,
) -> Result<()> {
    let r = &link.rect;
    let x = r.x as i32 - 2;
    let y = r.y as i32 - 1;
    let w = r.width as u32 + 4;
    let h = r.height as u32 + 2;

    // Top edge
    backend.fill_rect(x, y, w, 1, highlight_color)?;
    // Bottom edge
    backend.fill_rect(x, y + h as i32, w, 1, highlight_color)?;
    // Left edge
    backend.fill_rect(x, y, 1, h, highlight_color)?;
    // Right edge
    backend.fill_rect(x + w as i32, y, 1, h, highlight_color)?;

    Ok(())
}
