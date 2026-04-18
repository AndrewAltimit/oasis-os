//! Block-level layout algorithm.
//!
//! Implements CSS 2.1 block formatting context (BFC) layout. Block
//! boxes are stacked vertically; their widths expand to fill the
//! containing block and heights are determined by content.
//!
//! ## Block formatting context (BFC)
//!
//! A BFC is established by the root element, floats, absolutely
//! positioned boxes, and `overflow` values other than `visible`.
//! Inside a BFC the algorithm works as follows:
//!
//! ```text
//!   containing block (available width W)
//!   +------------------------------------+
//!   | child A  (width = W, height = ?)   |  <- laid out first
//!   +------------------------------------+
//!   | collapsed margin                   |  <- max(A.margin-bottom, B.margin-top)
//!   +------------------------------------+
//!   | child B  (width = W, height = ?)   |  <- cursor advances by A.height + margin
//!   +------------------------------------+
//! ```
//!
//! ## Margin collapsing
//!
//! Adjacent vertical margins between siblings collapse to the larger
//! of the two (see [`collapse_margins`]). Parent-child margins also
//! collapse when no border, padding, or clearance separates them.
//!
//! ## Float interaction
//!
//! Floated boxes are removed from normal flow and placed left or right
//! within the BFC. Subsequent block boxes' content area shrinks to
//! avoid overlap, while `clear` forces the cursor past earlier floats.
//!
//! ## Incremental layout
//!
//! Each [`LayoutBox`] carries a `dirty` flag. When only a subtree
//! changes, callers can mark individual boxes dirty via
//! [`LayoutBox::mark_dirty`] and then call [`layout_block_incremental`]
//! to relayout only the dirty subtree while preserving previously
//! computed dimensions for clean branches. A [`StyleCache`] avoids
//! redundant style computations during incremental passes.

mod tree_builder;

#[cfg(test)]
mod tests;

// Re-export public API so external paths don't change.
pub use tree_builder::build_layout_tree;

use super::box_model::*;
use super::flex::layout_flex;
use super::float::{ClearSide, FloatContext, FloatSide};
use super::grid::layout_grid;
use super::inline::layout_inline;
use super::multicol::layout_multicol;
use super::table::layout_table;
use crate::css::values::{
    BoxSizing, Clear, ComputedStyle, Dimension, Display, Float, Overflow, Position,
};
use crate::html::dom::NodeId;

// -------------------------------------------------------------------
// TextMeasurer trait
// -------------------------------------------------------------------

/// Trait for measuring text width at a given font size.
///
/// Backends supply concrete implementations. The layout engine calls
/// [`measure_text`](TextMeasurer::measure_text) to determine how much
/// horizontal space a text run occupies so it can compute line breaks
/// and box dimensions.
pub trait TextMeasurer {
    /// Return the width in pixels of `text` rendered at `font_size`.
    fn measure_text(&self, text: &str, font_size: u16) -> u32;
}

// -------------------------------------------------------------------
// Style cache
// -------------------------------------------------------------------

/// Cache of computed edge sizes (padding/border/margin) keyed by DOM
/// node ID. Since `NodeId` is `usize`, a `Vec<Option<...>>` provides
/// O(1) lookup without hashing overhead.
#[derive(Debug, Default)]
pub struct StyleCache {
    edges: Vec<Option<CachedEdges>>,
}

/// Cached resolved edge sizes for a single layout box.
#[derive(Debug, Clone)]
struct CachedEdges {
    padding: EdgeSizes,
    border: EdgeSizes,
    margin: EdgeSizes,
}

impl StyleCache {
    /// Create an empty style cache.
    pub fn new() -> Self {
        Self { edges: Vec::new() }
    }

    /// Store resolved edge sizes for a node.
    pub fn insert_edges(
        &mut self,
        node: NodeId,
        padding: EdgeSizes,
        border: EdgeSizes,
        margin: EdgeSizes,
    ) {
        if node >= self.edges.len() {
            self.edges.resize(node + 1, None);
        }
        self.edges[node] = Some(CachedEdges {
            padding,
            border,
            margin,
        });
    }

    /// Retrieve cached edges for a node. Returns `None` on cache miss.
    pub fn get_edges(&self, node: NodeId) -> Option<(&EdgeSizes, &EdgeSizes, &EdgeSizes)> {
        self.edges
            .get(node)
            .and_then(|slot| slot.as_ref())
            .map(|c| (&c.padding, &c.border, &c.margin))
    }

    /// Number of cached entries (useful for benchmarking).
    pub fn len(&self) -> usize {
        self.edges.iter().filter(|e| e.is_some()).count()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.edges.iter().all(|e| e.is_none())
    }

    /// Clear all cached entries.
    pub fn clear(&mut self) {
        self.edges.clear();
    }
}

// -------------------------------------------------------------------
// Incremental layout
// -------------------------------------------------------------------

/// Perform incremental layout on a previously-built layout tree.
///
/// Only re-lays-out subtrees where at least one box has its `dirty`
/// flag set. Clean subtrees are skipped entirely. After layout
/// completes, all boxes are marked clean. The optional `cache` stores
/// resolved edge sizes to avoid redundant recomputation.
///
/// This is an additive optimisation -- when the entire tree is dirty
/// (e.g. initial layout), the result is identical to a full
/// `layout_block` pass.
pub fn layout_block_incremental(
    layout_box: &mut LayoutBox,
    containing_width: f32,
    measurer: &dyn TextMeasurer,
    cache: &mut StyleCache,
) {
    if !layout_box.dirty && !any_child_dirty(layout_box) {
        return;
    }

    // Resolve edge sizes, using cache when available.
    resolve_edge_sizes_cached(layout_box, containing_width, cache);

    calculate_block_width(layout_box, containing_width);

    if matches!(layout_box.box_type, BoxType::Flex) {
        layout_flex(layout_box, containing_width, measurer);
    } else if matches!(layout_box.box_type, BoxType::Grid) {
        layout_grid(layout_box, containing_width, measurer);
    } else if matches!(layout_box.box_type, BoxType::TableWrapper) {
        layout_table_children(layout_box, measurer);
    } else {
        layout_children_incremental(layout_box, measurer, cache);
        shrink_float_to_fit(layout_box);
        calculate_block_height(layout_box, None);
    }

    layout_box.dirty = false;
}

/// Check whether any child (or deeper descendant) is dirty.
fn any_child_dirty(layout_box: &LayoutBox) -> bool {
    for child in &layout_box.children {
        if child.dirty || any_child_dirty(child) {
            return true;
        }
    }
    false
}

/// Resolve edge sizes with caching. If the box has a DOM node and a
/// cache hit, the cached values are used directly. Otherwise the
/// values are resolved from the style and stored in the cache.
fn resolve_edge_sizes_cached(
    layout_box: &mut LayoutBox,
    containing_width: f32,
    cache: &mut StyleCache,
) {
    if let Some(node) = layout_box.node
        && !layout_box.dirty
        && let Some((p, b, m)) = cache.get_edges(node)
    {
        layout_box.dimensions.padding = *p;
        layout_box.dimensions.border = *b;
        layout_box.dimensions.margin = *m;
        return;
    }

    resolve_edge_sizes(layout_box, containing_width);

    if let Some(node) = layout_box.node {
        cache.insert_edges(
            node,
            layout_box.dimensions.padding,
            layout_box.dimensions.border,
            layout_box.dimensions.margin,
        );
    }
}

/// Incremental version of [`layout_block_children`]. Skips clean
/// children whose subtrees are also clean.
fn layout_children_incremental(
    parent: &mut LayoutBox,
    measurer: &dyn TextMeasurer,
    cache: &mut StyleCache,
) {
    let all_inline =
        !parent.children.is_empty() && parent.children.iter().all(|c| !c.is_block_level());
    if all_inline {
        // Inline formatting context -- relayout fully when dirty.
        if parent.dirty || any_child_dirty(parent) {
            layout_inline(parent, measurer);
        }
        return;
    }

    let content_x = parent.dimensions.content.x;
    let content_width = parent.dimensions.content.width;
    let mut cursor_y = parent.dimensions.content.y;
    let mut prev_margin_bottom: f32 = 0.0;

    let parent_bfc = establishes_bfc(&parent.style);
    let can_collapse_top =
        !parent_bfc && parent.dimensions.padding.top == 0.0 && parent.dimensions.border.top == 0.0;
    let can_collapse_bottom = !parent_bfc
        && parent.dimensions.padding.bottom == 0.0
        && parent.dimensions.border.bottom == 0.0
        && parent.style.height == Dimension::Auto;

    let mut is_first_in_flow = true;

    for child in &mut parent.children {
        match child.box_type {
            BoxType::Block | BoxType::ListItem { .. } | BoxType::TableWrapper => {
                resolve_edge_sizes_cached(child, content_width, cache);

                let child_margin_top = child.dimensions.margin.top;
                let collapsed = if is_first_in_flow && can_collapse_top {
                    let parent_mt = parent.dimensions.margin.top;
                    parent.dimensions.margin.top = collapse_margins(parent_mt, child_margin_top);
                    child.dimensions.margin.top = 0.0;
                    0.0
                } else {
                    collapse_margins(prev_margin_bottom, child_margin_top)
                };
                is_first_in_flow = false;

                child.dimensions.content.x = content_x
                    + child.dimensions.margin.left
                    + child.dimensions.border.left
                    + child.dimensions.padding.left;
                child.dimensions.content.y = cursor_y
                    + collapsed
                    + child.dimensions.border.top
                    + child.dimensions.padding.top;

                // `calculate_block_width` inside the recursive layout
                // call may resolve auto margins (the `margin: 0 auto`
                // centering pattern) only AFTER we've baked the
                // pre-resolution margin into `content.x` above. Snapshot
                // the x that was just written so we can apply the delta
                // — and shift the whole subtree — once the child is
                // done. Without this, `.central-textlogo` (Wikipedia's
                // centered logo wrapper) gets `margin-left = 254.8` on
                // an 800-wide viewport but paints at `x = 10.2`.
                let pre_x = child.dimensions.content.x;

                if child.dirty || any_child_dirty(child) {
                    layout_block_incremental(child, content_width, measurer, cache);
                }

                let resolved_x = content_x
                    + child.dimensions.margin.left
                    + child.dimensions.border.left
                    + child.dimensions.padding.left;
                let dx = resolved_x - pre_x;
                if dx != 0.0 {
                    offset_descendant(child, dx, 0.0);
                }

                let bb = child.dimensions.border_box();
                cursor_y = bb.y + bb.height;
                prev_margin_bottom = child.dimensions.margin.bottom;
            },
            BoxType::Anonymous => {
                is_first_in_flow = false;
                child.dimensions.content.x = content_x;
                child.dimensions.content.y = cursor_y;
                child.dimensions.content.width = content_width;

                if child.dirty || any_child_dirty(child) {
                    layout_inline(child, measurer);
                }

                cursor_y += child.dimensions.content.height;
                prev_margin_bottom = 0.0;
            },
            BoxType::Replaced(_) => {
                resolve_edge_sizes_cached(child, content_width, cache);
                let pad_h = child.dimensions.padding.horizontal();
                let bdr_h = child.dimensions.border.horizontal();
                let mar_h = child.dimensions.margin.horizontal();

                let child_margin_top = child.dimensions.margin.top;
                let collapsed = if is_first_in_flow && can_collapse_top {
                    let parent_mt = parent.dimensions.margin.top;
                    parent.dimensions.margin.top = collapse_margins(parent_mt, child_margin_top);
                    child.dimensions.margin.top = 0.0;
                    0.0
                } else {
                    collapse_margins(prev_margin_bottom, child_margin_top)
                };
                is_first_in_flow = false;

                child.dimensions.content.x = content_x
                    + child.dimensions.margin.left
                    + child.dimensions.border.left
                    + child.dimensions.padding.left;
                child.dimensions.content.y = cursor_y
                    + collapsed
                    + child.dimensions.border.top
                    + child.dimensions.padding.top;

                let mut w = (content_width - pad_h - bdr_h - mar_h).max(0.0);
                if let Dimension::Px(max) = child.style.max_width
                    && max < 999.0
                {
                    w = w.min(max);
                }
                child.dimensions.content.width = w;

                let h = match child.style.height {
                    Dimension::Px(h) => h,
                    _ => {
                        // CSS aspect-ratio on replaced elements: derive
                        // height from the laid-out width and the ratio.
                        if let Some(ratio) = child.style.aspect_ratio
                            && ratio > 0.0
                        {
                            w / ratio
                        } else if let BoxType::Replaced(ref rc) = child.box_type {
                            match rc {
                                ReplacedContent::HorizontalRule => 2.0,
                                ReplacedContent::Image { height, .. } => *height as f32,
                                ReplacedContent::Svg { element } => element.height,
                                ReplacedContent::Canvas { state } => state.borrow().height as f32,
                                _ => 0.0,
                            }
                        } else {
                            0.0
                        }
                    },
                };
                child.dimensions.content.height = h;

                let bb = child.dimensions.border_box();
                cursor_y = bb.y + bb.height;
                prev_margin_bottom = child.dimensions.margin.bottom;
            },
            _ => {},
        }
    }

    // Parent-last-child bottom margin collapsing.
    if can_collapse_bottom {
        let last_block = parent
            .children
            .iter()
            .rposition(|c| c.is_block_level() && !matches!(c.box_type, BoxType::Anonymous));
        if let Some(idx) = last_block {
            let child_mb = parent.children[idx].dimensions.margin.bottom;
            if child_mb > 0.0 {
                let parent_mb = parent.dimensions.margin.bottom;
                parent.dimensions.margin.bottom = collapse_margins(parent_mb, child_mb);
                parent.children[idx].dimensions.margin.bottom = 0.0;
            }
        }
    }
}

// -------------------------------------------------------------------
// Block layout algorithm
// -------------------------------------------------------------------

/// Lay out a block-level box and all its children.
///
/// The block's `content.x` and `content.y` must be set by the caller
/// (the parent positions each child). This function calculates width,
/// lays out children, and determines height.
pub fn layout_block(
    layout_box: &mut LayoutBox,
    containing_width: f32,
    measurer: &dyn TextMeasurer,
) {
    layout_block_with_height(layout_box, containing_width, None, measurer);
}

/// Internal layout with optional containing height for percentage
/// height resolution.
pub(crate) fn layout_block_with_height(
    layout_box: &mut LayoutBox,
    containing_width: f32,
    containing_height: Option<f32>,
    measurer: &dyn TextMeasurer,
) {
    // 1. Resolve padding, border, and margin from the computed style.
    resolve_edge_sizes(layout_box, containing_width);

    // 2. Calculate width.
    calculate_block_width(layout_box, containing_width);

    // 3. Layout children.
    if matches!(layout_box.box_type, BoxType::Flex) {
        layout_flex(layout_box, containing_width, measurer);
    } else if matches!(layout_box.box_type, BoxType::Grid) {
        layout_grid(layout_box, containing_width, measurer);
    } else if matches!(layout_box.box_type, BoxType::TableWrapper) {
        layout_table_children(layout_box, measurer);
    } else if layout_box.style.column_count > 0 || layout_box.style.column_width > 0.0 {
        layout_multicol(layout_box, containing_width, measurer);
        calculate_block_height(layout_box, containing_height);
    } else {
        layout_block_children(layout_box, measurer);
        // 4. Float shrink-to-fit (CSS 2.1 §10.3.5): after children
        //    are laid out at the available width, clamp the float's
        //    content.width down to the actual rightmost child extent.
        //    This approximates `min(max-content, available)`.
        shrink_float_to_fit(layout_box);
        // 5. Calculate height.
        calculate_block_height(layout_box, containing_height);
    }
}

/// Apply CSS 2.1 §10.3.5 shrink-to-fit sizing to floats that declared
/// `width: auto`. Called after children have been laid out at the
/// provisional (available) width. Picks the greater of the rightmost
/// child border-box edge and the rightmost placed-float edge so the
/// float's own descendant floats stay inside its border box.
fn shrink_float_to_fit(layout_box: &mut LayoutBox) {
    if layout_box.style.float == Float::None {
        return;
    }
    if !matches!(
        layout_box.style.width,
        Dimension::Auto | Dimension::MinContent | Dimension::MaxContent | Dimension::FitContent
    ) {
        return;
    }
    let origin_x = layout_box.dimensions.content.x;
    let available = layout_box.dimensions.content.width;
    // Measure max-content ≈ rightmost child border-box edge. We do NOT
    // include margin-right because the normal-flow over-constrained
    // rule (§10.3.3) absorbs leftover containing-block width into the
    // child's margin-right — that slack is not real content extent.
    // Inline children were laid out against the available width, so
    // any line whose text happened to wrap reports the wrapped extent,
    // giving `min(max-content, available)` in one pass.
    let max_child_right = layout_box
        .children
        .iter()
        .map(|c| {
            let bb = c.dimensions.border_box();
            bb.x + bb.width - origin_x
        })
        .fold(0.0_f32, f32::max);
    // When the float has only inline content, the border-box walk
    // above reports 0 (anonymous inline wrappers are tracked via
    // `content.height`). Fall back to the float's own content.width
    // set by the inline layout pass.
    let measured = if max_child_right > 0.0 {
        max_child_right
    } else {
        layout_box.dimensions.content.width
    };
    let shrunk = measured.min(available).max(0.0);
    layout_box.dimensions.content.width = shrunk;
}

/// Resolve padding, border, and margin from the computed style into
/// the layout box's dimensions. Percentage padding/margin resolves
/// against the containing block's width (per CSS spec, even for
/// vertical padding/margin).
pub fn resolve_edge_sizes(layout_box: &mut LayoutBox, containing_width: f32) {
    let s = &layout_box.style;

    layout_box.dimensions.padding = EdgeSizes {
        top: s
            .padding_top_pct
            .map_or(s.padding_top, |p| containing_width * p / 100.0),
        right: s
            .padding_right_pct
            .map_or(s.padding_right, |p| containing_width * p / 100.0),
        bottom: s
            .padding_bottom_pct
            .map_or(s.padding_bottom, |p| containing_width * p / 100.0),
        left: s
            .padding_left_pct
            .map_or(s.padding_left, |p| containing_width * p / 100.0),
    };

    // Per CSS spec, border-style:none/hidden → border-width computes to 0.
    use crate::css::values::BorderStyle;
    layout_box.dimensions.border = EdgeSizes {
        top: if s.border_top_style == BorderStyle::None {
            0.0
        } else {
            s.border_top_width
        },
        right: if s.border_right_style == BorderStyle::None {
            0.0
        } else {
            s.border_right_width
        },
        bottom: if s.border_bottom_style == BorderStyle::None {
            0.0
        } else {
            s.border_bottom_width
        },
        left: if s.border_left_style == BorderStyle::None {
            0.0
        } else {
            s.border_left_width
        },
    };

    layout_box.dimensions.margin = EdgeSizes {
        top: s
            .margin_top_pct
            .map_or(s.margin_top, |p| containing_width * p / 100.0),
        right: s
            .margin_right_pct
            .map_or(s.margin_right, |p| containing_width * p / 100.0),
        bottom: s
            .margin_bottom_pct
            .map_or(s.margin_bottom, |p| containing_width * p / 100.0),
        left: s
            .margin_left_pct
            .map_or(s.margin_left, |p| containing_width * p / 100.0),
    };
}

/// Calculate the width of a block-level box.
///
/// If width is `auto`, the box fills the available space in the
/// containing block. If explicit, auto margins are used for centering.
/// The constraint equation is:
///
///   margin-left + border-left + padding-left + width
///     + padding-right + border-right + margin-right
///     = containing_width
///
/// If over-constrained, `margin-right` absorbs the overflow.
fn calculate_block_width(layout_box: &mut LayoutBox, containing_width: f32) {
    let pad_h = layout_box.dimensions.padding.horizontal();
    let bdr_h = layout_box.dimensions.border.horizontal();
    let mar_h = layout_box.dimensions.margin.horizontal();
    let total_extra = pad_h + bdr_h + mar_h;

    let ml_auto = layout_box.style.margin_left_auto;
    let mr_auto = layout_box.style.margin_right_auto;

    let is_border_box = layout_box.style.box_sizing == BoxSizing::BorderBox;

    // Absolute and fixed positioning run their own constraint-solving
    // pass in `positioning::apply_absolute_position` using `top`/`left`/
    // `right`/`bottom`. For those boxes the normal-flow "over-constrained
    // → margin-right absorbs the overflow" rule does not apply — CSS
    // 2.1 §10.3.7 lets the `right`/`left` properties take the slack.
    // If we run that rule here anyway, a box that declares a fixed
    // width of 124.8 inside a 436.8-wide containing block ends up with
    // margin-right = 312, which later multiplies into the absolute-
    // positioning formula (margin_box.width) and drags the box
    // hundreds of pixels off-screen (Wikipedia's 10-language circle).
    let is_abs = matches!(
        layout_box.style.position,
        Position::Absolute | Position::Fixed
    );
    // CSS 2.1 §10.3.5: floated, non-replaced elements use the declared
    // width (shrink-to-fit if auto) and `auto` margins compute to 0.
    // The normal-flow over-constrained rule (§10.3.3) that absorbs
    // leftover space into `margin-right` must NOT apply to floats —
    // otherwise a `float: right; width: 300px` inside an 1280px
    // container gets `margin-right: 960px`, which makes its margin box
    // span the whole container and `place_float` anchors it at x=0
    // instead of the right edge. This is the old.reddit sidebar bug.
    let is_float = layout_box.style.float != Float::None;

    match layout_box.style.width {
        Dimension::Px(w) => {
            let content_w = if is_border_box {
                (w - pad_h - bdr_h).max(0.0)
            } else {
                w
            };
            layout_box.dimensions.content.width = content_w;
            let available_for_margins = containing_width - content_w - pad_h - bdr_h;
            if is_abs {
                // Auto margins on absolute boxes resolve against the
                // `top`/`left`/`right`/`bottom` constraints in the
                // positioning pass; here we just leave declared margins
                // in place (auto → treated as 0 for this pass).
            } else if is_float {
                if ml_auto {
                    layout_box.dimensions.margin.left = 0.0;
                }
                if mr_auto {
                    layout_box.dimensions.margin.right = 0.0;
                }
            } else if ml_auto && mr_auto {
                let half = available_for_margins.max(0.0) / 2.0;
                layout_box.dimensions.margin.left = half;
                layout_box.dimensions.margin.right = half;
            } else if ml_auto {
                layout_box.dimensions.margin.left =
                    (available_for_margins - layout_box.dimensions.margin.right).max(0.0);
            } else if mr_auto {
                layout_box.dimensions.margin.right =
                    (available_for_margins - layout_box.dimensions.margin.left).max(0.0);
            } else {
                // Over-constrained: margin-right absorbs overflow.
                layout_box.dimensions.margin.right =
                    available_for_margins - layout_box.dimensions.margin.left;
            }
        },
        Dimension::Percent(pct) => {
            let declared_w = containing_width * (pct / 100.0);
            let content_w = if is_border_box {
                (declared_w - pad_h - bdr_h).max(0.0)
            } else {
                declared_w
            };
            layout_box.dimensions.content.width = content_w;
            let available_for_margins = containing_width - content_w - pad_h - bdr_h;
            if is_abs {
                // See note above for the `Dimension::Px` branch.
            } else if is_float {
                if ml_auto {
                    layout_box.dimensions.margin.left = 0.0;
                }
                if mr_auto {
                    layout_box.dimensions.margin.right = 0.0;
                }
            } else if ml_auto && mr_auto {
                let half = available_for_margins.max(0.0) / 2.0;
                layout_box.dimensions.margin.left = half;
                layout_box.dimensions.margin.right = half;
            } else if ml_auto {
                layout_box.dimensions.margin.left =
                    (available_for_margins - layout_box.dimensions.margin.right).max(0.0);
            } else if mr_auto {
                layout_box.dimensions.margin.right =
                    (available_for_margins - layout_box.dimensions.margin.left).max(0.0);
            }
        },
        Dimension::Auto | Dimension::MinContent | Dimension::MaxContent | Dimension::FitContent => {
            // CSS 2.1 §10.3.5: floats with `width: auto` use shrink-to-
            // fit = min(max-content, max(min-content, available)). We
            // provisionally assign the available width here; after the
            // children are laid out, `layout_block_with_height` clamps
            // `content.width` down to the actual rightmost child extent
            // (see `shrink_float_to_fit`). Auto margins on floats
            // resolve to 0 per §10.3.5.
            let w = (containing_width - total_extra).max(0.0);
            layout_box.dimensions.content.width = w;
            if is_float {
                if ml_auto {
                    layout_box.dimensions.margin.left = 0.0;
                }
                if mr_auto {
                    layout_box.dimensions.margin.right = 0.0;
                }
            }
        },
    }

    // Apply min-width / max-width constraints.
    match layout_box.style.min_width {
        Dimension::Px(min) if layout_box.dimensions.content.width < min => {
            layout_box.dimensions.content.width = min;
        },
        Dimension::Percent(pct) => {
            let min = containing_width * (pct / 100.0);
            if layout_box.dimensions.content.width < min {
                layout_box.dimensions.content.width = min;
            }
        },
        _ => {},
    }
    match layout_box.style.max_width {
        Dimension::Px(max) if max < 999.0 && layout_box.dimensions.content.width > max => {
            // Don't clamp table rowspan/colspan encoding values (>= 1000).
            layout_box.dimensions.content.width = max;
        },
        Dimension::Percent(pct) => {
            let max = containing_width * (pct / 100.0);
            if layout_box.dimensions.content.width > max {
                layout_box.dimensions.content.width = max;
            }
        },
        _ => {},
    }
}

/// Lay out a table wrapper's children using the table layout algorithm.
///
/// Delegates to [`layout_table`] which computes column widths, row heights,
/// and positions cells. The results are copied back into the table's layout
/// box and offset into the table's content coordinate space.
fn layout_table_children(layout_box: &mut LayoutBox, measurer: &dyn TextMeasurer) {
    let content_width = layout_box.dimensions.content.width;
    let result = layout_table(
        &layout_box.children,
        &layout_box.style,
        content_width,
        measurer,
    );
    let offset_x = layout_box.dimensions.content.x;
    let offset_y = layout_box.dimensions.content.y;
    layout_box.dimensions.content.height = result.dimensions.content.height;
    layout_box.children = result.children;
    // Offset all children into the table's content coordinate space.
    for child in &mut layout_box.children {
        offset_descendant(child, offset_x, offset_y);
    }
}

/// Recursively offset a layout box and its descendants by `(dx, dy)`.
pub(crate) fn offset_descendant(layout_box: &mut LayoutBox, dx: f32, dy: f32) {
    layout_box.dimensions.content.x += dx;
    layout_box.dimensions.content.y += dy;
    for child in &mut layout_box.children {
        offset_descendant(child, dx, dy);
    }
}

/// Returns true if the parent establishes a new block formatting
/// context (BFC), which inhibits margin collapsing with children.
fn establishes_bfc(style: &ComputedStyle) -> bool {
    style.overflow != Overflow::Visible
        || style.float != Float::None
        || matches!(style.position, Position::Absolute | Position::Fixed)
        || matches!(
            style.display,
            Display::InlineBlock
                | Display::Flex
                | Display::InlineFlex
                | Display::Grid
                | Display::InlineGrid
        )
}

/// Layout block-level children, stacking them vertically.
///
/// If all children are inline-level (no block-level siblings), the
/// parent establishes an inline formatting context and we delegate
/// to [`layout_inline`] directly. This handles the common case of
/// `<p>text</p>` or `<p><a>link</a></p>` where the block box
/// contains only inline content.
fn layout_block_children(parent: &mut LayoutBox, measurer: &dyn TextMeasurer) {
    // If all children are inline, establish an IFC on the parent.
    let all_inline =
        !parent.children.is_empty() && parent.children.iter().all(|c| !c.is_block_level());
    if all_inline {
        layout_inline(parent, measurer);
        return;
    }

    let content_x = parent.dimensions.content.x;
    let content_width = parent.dimensions.content.width;
    let mut cursor_y = parent.dimensions.content.y;

    let mut prev_margin_bottom: f32 = 0.0;
    let mut float_ctx = FloatContext::new();
    let mut list_counter: usize = 1;

    // Parent-child margin collapsing (CSS 2.1 §8.3.1):
    // Margins collapse when there's no padding, border, or BFC
    // between parent and first/last child.
    let parent_bfc = establishes_bfc(&parent.style);
    let can_collapse_top =
        !parent_bfc && parent.dimensions.padding.top == 0.0 && parent.dimensions.border.top == 0.0;
    let can_collapse_bottom = !parent_bfc
        && parent.dimensions.padding.bottom == 0.0
        && parent.dimensions.border.bottom == 0.0
        && parent.style.height == Dimension::Auto;

    let mut is_first_in_flow = true;

    for child in &mut parent.children {
        // Assign sequential numbers to ordered list items.
        if let BoxType::ListItem { ref mut marker } = child.box_type
            && let ListMarker::Ordered(_, n) = marker
        {
            *n = list_counter;
            list_counter += 1;
        }
        // Handle clear property: advance cursor below cleared floats.
        let clear_y = match child.style.clear {
            Clear::Left => float_ctx.clear_y(ClearSide::Left),
            Clear::Right => float_ctx.clear_y(ClearSide::Right),
            Clear::Both => float_ctx.clear_y(ClearSide::Both),
            Clear::None => 0.0,
        };
        if child.style.clear != Clear::None && clear_y > cursor_y {
            cursor_y = clear_y;
            prev_margin_bottom = 0.0;
        }

        // Handle floated children: position via float context.
        if child.style.float != Float::None {
            resolve_edge_sizes(child, content_width);
            layout_block(child, content_width, measurer);
            let margin_box = child.dimensions.margin_box();
            let side = match child.style.float {
                Float::Left => FloatSide::Left,
                Float::Right => FloatSide::Right,
                Float::None => unreachable!(),
            };
            let float_box = float_ctx.place_float(
                side,
                margin_box.width,
                margin_box.height,
                cursor_y,
                content_width,
            );
            // `place_float` returns a position in BFC-local coordinates
            // (origin at the containing block's content edge). To turn
            // that into absolute (layout-tree) coordinates we must add
            // the parent's `content_x`/content.y — otherwise a float
            // inside a comment at absolute x=15 would land at x=0, which
            // also prevents hit-testing from finding the float's
            // descendants (their rects then overhang the parent's
            // bounding box on the left). Same pattern we already use
            // for `margin: 0 auto` centering below.
            let pre_x = child.dimensions.content.x;
            let pre_y = child.dimensions.content.y;
            let resolved_x = content_x
                + float_box.rect.x
                + child.dimensions.margin.left
                + child.dimensions.border.left
                + child.dimensions.padding.left;
            let resolved_y = float_box.rect.y
                + child.dimensions.margin.top
                + child.dimensions.border.top
                + child.dimensions.padding.top;
            let dx = resolved_x - pre_x;
            let dy = resolved_y - pre_y;
            child.dimensions.content.x = resolved_x;
            child.dimensions.content.y = resolved_y;
            if dx != 0.0 || dy != 0.0 {
                for grandchild in &mut child.children {
                    offset_descendant(grandchild, dx, dy);
                }
            }
            continue;
        }

        match child.box_type {
            BoxType::Block | BoxType::ListItem { .. } | BoxType::TableWrapper => {
                // Resolve child's edge sizes first so we can read
                // margins for positioning.
                resolve_edge_sizes(child, content_width);

                let child_margin_top = child.dimensions.margin.top;

                // Parent-first-child top margin collapsing:
                // When this is the first in-flow block child and
                // there's no padding/border/BFC separating parent
                // from child, their top margins collapse. The
                // child's margin is absorbed into the parent.
                let did_collapse_top = is_first_in_flow && can_collapse_top;
                let collapsed = if did_collapse_top {
                    let parent_mt = parent.dimensions.margin.top;
                    parent.dimensions.margin.top = collapse_margins(parent_mt, child_margin_top);
                    0.0
                } else {
                    // Sibling margin collapsing: the collapsed
                    // margin replaces both the previous bottom and
                    // the current top margin.
                    collapse_margins(prev_margin_bottom, child_margin_top)
                };
                is_first_in_flow = false;

                // Position child's content area.
                child.dimensions.content.x = content_x
                    + child.dimensions.margin.left
                    + child.dimensions.border.left
                    + child.dimensions.padding.left;
                child.dimensions.content.y = cursor_y
                    + collapsed
                    + child.dimensions.border.top
                    + child.dimensions.padding.top;

                // Auto horizontal margins (`margin: 0 auto` centering)
                // are only resolved inside `layout_block → calculate_
                // block_width`, which runs *after* we've baked the pre-
                // resolution margin into `content.x` above. Snapshot x
                // so we can shift the child's subtree by the resolved
                // delta once layout returns — otherwise centered blocks
                // paint at their parent's origin.
                let pre_x = child.dimensions.content.x;

                layout_block(child, content_width, measurer);

                let resolved_x = content_x
                    + child.dimensions.margin.left
                    + child.dimensions.border.left
                    + child.dimensions.padding.left;
                let dx = resolved_x - pre_x;
                if dx != 0.0 {
                    offset_descendant(child, dx, 0.0);
                }

                // layout_block re-resolves edge sizes from the
                // style, so re-zero the top margin that was
                // absorbed into the parent.
                if did_collapse_top {
                    child.dimensions.margin.top = 0.0;
                }

                // Empty block self-collapsing: if a block has zero
                // content height, no padding, and no border, its own
                // top and bottom margins collapse into one margin.
                let is_empty = child.dimensions.content.height == 0.0
                    && child.dimensions.padding.vertical() == 0.0
                    && child.dimensions.border.vertical() == 0.0
                    && child.children.is_empty();

                if is_empty {
                    let self_collapsed = collapse_margins(
                        child.dimensions.margin.top,
                        child.dimensions.margin.bottom,
                    );
                    prev_margin_bottom = collapse_margins(prev_margin_bottom, self_collapsed);
                    // Empty block takes no vertical space beyond
                    // its collapsed margin.
                } else {
                    // Advance cursor_y to this child's border-box
                    // bottom (not margin-box). Margin collapsing
                    // will be handled when positioning the next
                    // sibling.
                    let bb = child.dimensions.border_box();
                    cursor_y = bb.y + bb.height;
                    prev_margin_bottom = child.dimensions.margin.bottom;
                }
            },
            BoxType::Anonymous => {
                // Anonymous box wrapping inline content -- prevents
                // parent-child margin collapsing with later siblings.
                is_first_in_flow = false;

                child.dimensions.content.x = content_x;
                child.dimensions.content.y = cursor_y;
                child.dimensions.content.width = content_width;

                // Layout as inline formatting context.
                layout_inline(child, measurer);

                cursor_y += child.dimensions.content.height;
                prev_margin_bottom = 0.0;
            },
            BoxType::Replaced(_) => {
                // Block-level replaced elements (e.g. <hr>) need to
                // participate in the block flow. They stretch to the
                // containing width and advance the cursor.
                resolve_edge_sizes(child, content_width);
                let pad_h = child.dimensions.padding.horizontal();
                let bdr_h = child.dimensions.border.horizontal();
                let mar_h = child.dimensions.margin.horizontal();

                let child_margin_top = child.dimensions.margin.top;

                // Parent-first-child collapsing for replaced elements.
                let collapsed = if is_first_in_flow && can_collapse_top {
                    let parent_mt = parent.dimensions.margin.top;
                    parent.dimensions.margin.top = collapse_margins(parent_mt, child_margin_top);
                    child.dimensions.margin.top = 0.0;
                    0.0
                } else {
                    collapse_margins(prev_margin_bottom, child_margin_top)
                };
                is_first_in_flow = false;

                child.dimensions.content.x = content_x
                    + child.dimensions.margin.left
                    + child.dimensions.border.left
                    + child.dimensions.padding.left;
                child.dimensions.content.y = cursor_y
                    + collapsed
                    + child.dimensions.border.top
                    + child.dimensions.padding.top;

                // Width: stretch to container (like a block), minus
                // edges. Respect max-width if set.
                let mut w = (content_width - pad_h - bdr_h - mar_h).max(0.0);
                if let Dimension::Px(max) = child.style.max_width
                    && max < 999.0
                {
                    w = w.min(max);
                }
                child.dimensions.content.width = w;

                // Height: use explicit CSS height or intrinsic height.
                let h = match child.style.height {
                    Dimension::Px(h) => h,
                    _ => {
                        // CSS aspect-ratio on replaced elements.
                        if let Some(ratio) = child.style.aspect_ratio
                            && ratio > 0.0
                        {
                            w / ratio
                        } else if let BoxType::Replaced(ref rc) = child.box_type {
                            match rc {
                                ReplacedContent::HorizontalRule => 2.0,
                                ReplacedContent::Image { height, .. } => *height as f32,
                                ReplacedContent::Svg { element } => element.height,
                                ReplacedContent::Canvas { state } => state.borrow().height as f32,
                                _ => 0.0,
                            }
                        } else {
                            0.0
                        }
                    },
                };
                child.dimensions.content.height = h;

                let bb = child.dimensions.border_box();
                cursor_y = bb.y + bb.height;
                prev_margin_bottom = child.dimensions.margin.bottom;
            },
            _ => {
                // Inline-level boxes inside a block context should
                // have been wrapped in anonymous boxes. If we get here,
                // just skip.
            },
        }
    }

    // Parent-last-child bottom margin collapsing (CSS 2.1 §8.3.1):
    // When height is auto and no padding/border separates parent
    // from its last in-flow block child, their bottom margins
    // collapse. The child's bottom margin is absorbed into the
    // parent's.
    if can_collapse_bottom {
        let last_block = parent
            .children
            .iter()
            .rposition(|c| c.is_block_level() && !matches!(c.box_type, BoxType::Anonymous));
        if let Some(idx) = last_block {
            let child_mb = parent.children[idx].dimensions.margin.bottom;
            if child_mb > 0.0 {
                let parent_mb = parent.dimensions.margin.bottom;
                parent.dimensions.margin.bottom = collapse_margins(parent_mb, child_mb);
                parent.children[idx].dimensions.margin.bottom = 0.0;
            }
        }
    }

    // Clearfix: ensure the parent's height includes all floats.
    if !float_ctx.is_empty() {
        let float_bottom = float_ctx.clear_y(ClearSide::Both);
        if float_bottom > cursor_y {
            // Extend content height to encompass floated children.
            parent.dimensions.content.height = parent.dimensions.content.height.max(float_bottom);
        }
    }
}

/// Calculate the height of a block-level box.
///
/// If `height` is explicit, use it. Otherwise, height is the distance
/// from the top of the content area to the bottom of the last child's
/// margin box. Percentage heights resolve against `containing_height`
/// when available.
fn calculate_block_height(layout_box: &mut LayoutBox, containing_height: Option<f32>) {
    let is_border_box = layout_box.style.box_sizing == BoxSizing::BorderBox;
    let pad_v = layout_box.dimensions.padding.vertical();
    let bdr_v = layout_box.dimensions.border.vertical();

    match layout_box.style.height {
        Dimension::Px(h) => {
            let content_h = if is_border_box {
                (h - pad_v - bdr_v).max(0.0)
            } else {
                h
            };
            layout_box.dimensions.content.height = content_h;
        },
        Dimension::Percent(pct) => {
            if let Some(ch) = containing_height {
                let declared_h = ch * (pct / 100.0);
                let content_h = if is_border_box {
                    (declared_h - pad_v - bdr_v).max(0.0)
                } else {
                    declared_h
                };
                layout_box.dimensions.content.height = content_h;
            } else {
                // No definite containing height: treat as auto (CSS spec).
                calculate_auto_height(layout_box);
            }
        },
        Dimension::Auto | Dimension::MinContent | Dimension::MaxContent | Dimension::FitContent => {
            // Use `aspect-ratio` to derive height from width when
            // height is auto, matching the CSS Box Sizing Level 4
            // rule that `aspect-ratio` applies to non-replaced blocks
            // whose width is definite. Only honor the ratio when the
            // author explicitly set a width — with `width: auto` the
            // block fills its containing block and we'd rather let
            // content drive the height.
            if let Some(ratio) = layout_box.style.aspect_ratio
                && ratio > 0.0
                && !matches!(layout_box.style.width, Dimension::Auto)
            {
                let content_w = layout_box.dimensions.content.width;
                let derived = content_w / ratio;
                let content_h = if is_border_box {
                    (derived - pad_v - bdr_v).max(0.0)
                } else {
                    derived.max(0.0)
                };
                layout_box.dimensions.content.height = content_h;
            } else {
                calculate_auto_height(layout_box);
            }
        },
    }

    // Apply min-height / max-height constraints.
    match layout_box.style.min_height {
        Dimension::Px(min) => {
            layout_box.dimensions.content.height = layout_box.dimensions.content.height.max(min);
        },
        Dimension::Percent(pct) => {
            if let Some(ch) = containing_height {
                let min = ch * (pct / 100.0);
                layout_box.dimensions.content.height =
                    layout_box.dimensions.content.height.max(min);
            }
        },
        _ => {},
    }
    match layout_box.style.max_height {
        Dimension::Px(max) => {
            layout_box.dimensions.content.height = layout_box.dimensions.content.height.min(max);
        },
        Dimension::Percent(pct) => {
            if let Some(ch) = containing_height {
                let max = ch * (pct / 100.0);
                layout_box.dimensions.content.height =
                    layout_box.dimensions.content.height.min(max);
            }
        },
        _ => {},
    }
}

/// Compute auto height from children's occupied space.
fn calculate_auto_height(layout_box: &mut LayoutBox) {
    let content_top = layout_box.dimensions.content.y;
    let mut bottom = content_top;

    for child in &layout_box.children {
        let child_mb = child.dimensions.margin_box();
        let child_bottom = child_mb.y + child_mb.height;
        if child_bottom > bottom {
            bottom = child_bottom;
        }
    }

    layout_box.dimensions.content.height = (bottom - content_top).max(0.0);
}

// -------------------------------------------------------------------
// Margin collapsing
// -------------------------------------------------------------------

/// Collapse adjacent vertical margins between siblings.
///
/// - If both are positive: use the larger one.
/// - If one is negative: sum them.
/// - If both are negative: use the more negative one (min).
///
/// Returns the effective vertical gap to insert between the previous
/// sibling's bottom and the current sibling's top.
pub fn collapse_margins(prev_bottom: f32, next_top: f32) -> f32 {
    if prev_bottom >= 0.0 && next_top >= 0.0 {
        prev_bottom.max(next_top)
    } else if prev_bottom < 0.0 && next_top < 0.0 {
        prev_bottom.min(next_top)
    } else {
        prev_bottom + next_top
    }
}
