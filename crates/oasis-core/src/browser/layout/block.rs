//! Block-level layout algorithm.
//!
//! Implements CSS 2.1 block formatting context (BFC) layout. Block
//! boxes are stacked vertically; their widths expand to fill the
//! containing block and heights are determined by content.

use super::box_model::*;
use super::inline::layout_inline;
use crate::browser::css::values::{ComputedStyle, Dimension, Display, ListStyleType};
use crate::browser::html::dom::{Document, ElementData, NodeId, NodeKind, TagName};

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
// Build layout tree from DOM
// -------------------------------------------------------------------

/// Build a layout tree from a styled DOM tree.
///
/// Starts from the `<body>` element (or the document root if no body
/// is found). The returned `LayoutBox` is the root block box with its
/// dimensions laid out to fit the given viewport.
pub fn build_layout_tree(
    doc: &Document,
    styles: &[Option<ComputedStyle>],
    measurer: &dyn TextMeasurer,
    viewport_width: f32,
    _viewport_height: f32,
) -> LayoutBox {
    let start_node = doc.body().unwrap_or(doc.root);
    let style = styles
        .get(start_node)
        .and_then(|s| s.clone())
        .unwrap_or_else(|| ComputedStyle {
            display: Display::Block,
            ..ComputedStyle::default()
        });

    let mut root = LayoutBox::new(BoxType::Block, style, Some(start_node));

    // Recursively build children.
    let children = doc.get(start_node).children.clone();
    let child_boxes = build_children(doc, &children, styles);
    root.children = wrap_anonymous(child_boxes);

    // Layout from the root.
    root.dimensions.content.x = 0.0;
    root.dimensions.content.y = 0.0;
    layout_block(&mut root, viewport_width, measurer);

    root
}

/// Recursively build child layout boxes for a list of DOM node IDs.
fn build_children(
    doc: &Document,
    children: &[NodeId],
    styles: &[Option<ComputedStyle>],
) -> Vec<LayoutBox> {
    let mut boxes = Vec::new();
    for &child_id in children {
        if let Some(lb) = build_box_for_node(doc, child_id, styles) {
            boxes.push(lb);
        }
    }
    boxes
}

/// Build a single layout box for a DOM node. Returns `None` for
/// `display: none`, comments, and nodes without styles.
fn build_box_for_node(
    doc: &Document,
    node_id: NodeId,
    styles: &[Option<ComputedStyle>],
) -> Option<LayoutBox> {
    let node = doc.get(node_id);

    match &node.kind {
        NodeKind::Element(elem) => {
            let style = styles.get(node_id)?.clone()?;
            if style.display == Display::None {
                return None;
            }

            // Determine box type.
            let box_type = box_type_for_element(elem, &style);

            // Handle replaced elements.
            if let Some(replaced) = replaced_content(elem) {
                let mut lb = LayoutBox::new(BoxType::Replaced(replaced), style, Some(node_id));
                lb.children = Vec::new();
                return Some(lb);
            }

            let mut lb = LayoutBox::new(box_type, style, Some(node_id));

            // Recursively build children.
            let child_ids = node.children.clone();
            let child_boxes = build_children(doc, &child_ids, styles);
            lb.children = wrap_anonymous(child_boxes);

            Some(lb)
        },
        NodeKind::Text(text) => {
            // Skip whitespace-only text nodes.
            if text.trim().is_empty() {
                return None;
            }
            let style = find_inherited_style(doc, node_id, styles);
            let mut inline_style = style;
            inline_style.display = Display::Inline;
            Some(LayoutBox::new(BoxType::Inline, inline_style, Some(node_id)))
        },
        NodeKind::Comment(_) | NodeKind::Document => None,
    }
}

/// Determine the box type for an element node based on its tag and
/// computed style.
fn box_type_for_element(_elem: &ElementData, style: &ComputedStyle) -> BoxType {
    match style.display {
        Display::Block => BoxType::Block,
        Display::Inline => BoxType::Inline,
        Display::InlineBlock => BoxType::InlineBlock,
        Display::ListItem => {
            let marker = resolve_list_marker(style);
            BoxType::ListItem { marker }
        },
        Display::Table => BoxType::TableWrapper,
        Display::TableRow => BoxType::TableRow,
        Display::TableCell => BoxType::TableCell,
        Display::None => BoxType::Block, // unreachable in practice
    }
}

/// Resolve the list marker type from the computed style.
fn resolve_list_marker(style: &ComputedStyle) -> ListMarker {
    match style.list_style_type {
        ListStyleType::Disc => ListMarker::Disc,
        ListStyleType::Circle => ListMarker::Circle,
        ListStyleType::Square => ListMarker::Square,
        ListStyleType::Decimal => ListMarker::Decimal(1),
        ListStyleType::None => ListMarker::None,
    }
}

/// Check if an element is a replaced element and return its content.
fn replaced_content(elem: &ElementData) -> Option<ReplacedContent> {
    match elem.tag {
        TagName::Img => {
            let width = elem
                .get_attribute("width")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0);
            let height = elem
                .get_attribute("height")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0);
            let alt = elem.get_attribute("alt").unwrap_or("").to_string();
            Some(ReplacedContent::Image {
                width,
                height,
                texture: None,
                alt,
            })
        },
        TagName::Hr => Some(ReplacedContent::HorizontalRule),
        TagName::Br => Some(ReplacedContent::LineBreak),
        _ => None,
    }
}

/// Walk up the DOM to find an inherited style for a text node.
fn find_inherited_style(
    doc: &Document,
    node_id: NodeId,
    styles: &[Option<ComputedStyle>],
) -> ComputedStyle {
    if let Some(parent_id) = doc.get(node_id).parent
        && let Some(Some(style)) = styles.get(parent_id)
    {
        return style.clone();
    }
    ComputedStyle::default()
}

// -------------------------------------------------------------------
// Anonymous box wrapping
// -------------------------------------------------------------------

/// When a block box has a mix of block-level and inline-level children,
/// wrap consecutive runs of inline children in anonymous block boxes.
///
/// This ensures the block formatting context only contains block-level
/// boxes, as required by CSS 2.1.
fn wrap_anonymous(children: Vec<LayoutBox>) -> Vec<LayoutBox> {
    if children.is_empty() {
        return children;
    }

    let has_block = children.iter().any(|c| c.is_block_level());
    let has_inline = children.iter().any(|c| !c.is_block_level());

    // If all children are the same level, no wrapping needed.
    if !has_block || !has_inline {
        return children;
    }

    // Mixed: wrap runs of inline children in anonymous block boxes.
    let mut result = Vec::new();
    let mut inline_run: Vec<LayoutBox> = Vec::new();

    for child in children {
        if child.is_block_level() {
            if !inline_run.is_empty() {
                let anon = make_anonymous_block(std::mem::take(&mut inline_run));
                result.push(anon);
            }
            result.push(child);
        } else {
            inline_run.push(child);
        }
    }

    // Flush any trailing inline run.
    if !inline_run.is_empty() {
        result.push(make_anonymous_block(inline_run));
    }

    result
}

/// Create an anonymous block box wrapping the given inline children.
fn make_anonymous_block(children: Vec<LayoutBox>) -> LayoutBox {
    LayoutBox {
        box_type: BoxType::Anonymous,
        dimensions: Dimensions::default(),
        children,
        node: None,
        style: ComputedStyle {
            display: Display::Block,
            ..ComputedStyle::default()
        },
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
    // 1. Resolve padding, border, and margin from the computed style.
    resolve_edge_sizes(layout_box, containing_width);

    // 2. Calculate width.
    calculate_block_width(layout_box, containing_width);

    // 3. Layout children.
    layout_block_children(layout_box, measurer);

    // 4. Calculate height.
    calculate_block_height(layout_box);
}

/// Resolve padding, border, and margin from the computed style into
/// the layout box's dimensions.
fn resolve_edge_sizes(layout_box: &mut LayoutBox, _containing_width: f32) {
    let s = &layout_box.style;

    layout_box.dimensions.padding = EdgeSizes {
        top: s.padding_top,
        right: s.padding_right,
        bottom: s.padding_bottom,
        left: s.padding_left,
    };

    layout_box.dimensions.border = EdgeSizes {
        top: s.border_top_width,
        right: s.border_right_width,
        bottom: s.border_bottom_width,
        left: s.border_left_width,
    };

    layout_box.dimensions.margin = EdgeSizes {
        top: s.margin_top,
        right: s.margin_right,
        bottom: s.margin_bottom,
        left: s.margin_left,
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
    let mar_l = layout_box.dimensions.margin.left;
    let total_extra = pad_h + bdr_h + mar_h;

    match layout_box.style.width {
        Dimension::Px(w) => {
            layout_box.dimensions.content.width = w;
            // Check if margins are auto for centering.
            let remaining = containing_width - w - total_extra + mar_h;
            if remaining > 0.0 {
                // Both margins auto => center.
                let half = remaining / 2.0;
                layout_box.dimensions.margin.left = half;
                layout_box.dimensions.margin.right = half;
            } else {
                // Over-constrained: margin-right absorbs overflow.
                layout_box.dimensions.margin.right = containing_width - w - pad_h - bdr_h - mar_l;
            }
        },
        Dimension::Percent(pct) => {
            let w = containing_width * (pct / 100.0);
            layout_box.dimensions.content.width = w;
            let remaining = containing_width - w - total_extra + mar_h;
            if remaining > 0.0 {
                let half = remaining / 2.0;
                layout_box.dimensions.margin.left = half;
                layout_box.dimensions.margin.right = half;
            }
        },
        Dimension::Auto => {
            // Width = containing_width minus all horizontal extras.
            let w = (containing_width - total_extra).max(0.0);
            layout_box.dimensions.content.width = w;
        },
    }
}

/// Layout block-level children, stacking them vertically.
fn layout_block_children(parent: &mut LayoutBox, measurer: &dyn TextMeasurer) {
    let content_x = parent.dimensions.content.x;
    let content_width = parent.dimensions.content.width;
    let mut cursor_y = parent.dimensions.content.y + parent.dimensions.padding.top;

    let mut prev_margin_bottom: f32 = 0.0;

    for child in &mut parent.children {
        match child.box_type {
            BoxType::Block | BoxType::ListItem { .. } | BoxType::TableWrapper => {
                // Resolve child's edge sizes first so we can read
                // margins for positioning.
                resolve_edge_sizes(child, content_width);

                // Margin collapsing between siblings: the collapsed
                // margin replaces both the previous bottom and the
                // current top margin. Since cursor_y tracks the end
                // of the previous child's border box, we add the
                // collapsed margin to get this child's margin-box
                // start, then offset by its own top edges.
                let child_margin_top = child.dimensions.margin.top;
                let collapsed = collapse_margins(prev_margin_bottom, child_margin_top);

                // Position child's content area.
                child.dimensions.content.x = content_x
                    + parent.dimensions.padding.left
                    + child.dimensions.margin.left
                    + child.dimensions.border.left
                    + child.dimensions.padding.left;
                child.dimensions.content.y = cursor_y
                    + collapsed
                    + child.dimensions.border.top
                    + child.dimensions.padding.top;

                layout_block(child, content_width, measurer);

                // Advance cursor_y to this child's border-box
                // bottom (not margin-box). Margin collapsing will
                // be handled when positioning the next sibling.
                let bb = child.dimensions.border_box();
                cursor_y = bb.y + bb.height;
                prev_margin_bottom = child.dimensions.margin.bottom;
            },
            BoxType::Anonymous => {
                // Anonymous box wrapping inline content.
                child.dimensions.content.x = content_x + parent.dimensions.padding.left;
                child.dimensions.content.y = cursor_y;
                child.dimensions.content.width = content_width;

                // Layout as inline formatting context.
                layout_inline(child, measurer);

                cursor_y += child.dimensions.content.height;
                prev_margin_bottom = 0.0;
            },
            _ => {
                // Inline-level boxes inside a block context should
                // have been wrapped in anonymous boxes. If we get here,
                // just skip.
            },
        }
    }
}

/// Calculate the height of a block-level box.
///
/// If `height` is explicit, use it. Otherwise, height is the distance
/// from the top of the content area to the bottom of the last child's
/// margin box.
fn calculate_block_height(layout_box: &mut LayoutBox) {
    match layout_box.style.height {
        Dimension::Px(h) => {
            layout_box.dimensions.content.height = h;
        },
        _ => {
            // Auto height: sum of children's occupied space.
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
        },
    }
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

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::css::values::Dimension;

    /// Fixed-width text measurer: 8 pixels per character.
    struct FixedMeasurer;

    impl TextMeasurer for FixedMeasurer {
        fn measure_text(&self, text: &str, _font_size: u16) -> u32 {
            text.len() as u32 * 8
        }
    }

    fn block_style() -> ComputedStyle {
        let mut s = ComputedStyle::default();
        s.display = Display::Block;
        s
    }

    // -- text measurer -------------------------------------------------

    #[test]
    fn fixed_measurer_returns_expected_width() {
        let m = FixedMeasurer;
        assert_eq!(m.measure_text("hello", 12), 40);
        assert_eq!(m.measure_text("", 12), 0);
    }

    // -- block width calculation --------------------------------------

    #[test]
    fn auto_width_fills_container() {
        let m = FixedMeasurer;
        let mut lb = LayoutBox::new(BoxType::Block, block_style(), None);
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;
        layout_block(&mut lb, 480.0, &m);
        assert_eq!(lb.dimensions.content.width, 480.0);
    }

    #[test]
    fn explicit_width_centering() {
        let m = FixedMeasurer;
        let mut style = block_style();
        style.width = Dimension::Px(200.0);
        let mut lb = LayoutBox::new(BoxType::Block, style, None);
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;
        layout_block(&mut lb, 480.0, &m);
        assert_eq!(lb.dimensions.content.width, 200.0);
        // Margins should be equal (centered): (480 - 200) / 2 = 140
        let ml = lb.dimensions.margin.left;
        let mr = lb.dimensions.margin.right;
        assert!(
            (ml - mr).abs() < f32::EPSILON,
            "margins should be equal: left={ml}, right={mr}",
        );
        assert!(
            (ml - 140.0).abs() < f32::EPSILON,
            "margin should be 140, got {ml}",
        );
    }

    #[test]
    fn nested_block_layout() {
        let m = FixedMeasurer;
        let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
        let child = LayoutBox::new(BoxType::Block, block_style(), None);
        parent.children = vec![child];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;
        layout_block(&mut parent, 480.0, &m);

        assert_eq!(parent.dimensions.content.width, 480.0);
        assert_eq!(parent.children[0].dimensions.content.width, 480.0,);
    }

    #[test]
    fn multiple_children_stacked_vertically() {
        let m = FixedMeasurer;
        let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
        let mut s1 = block_style();
        s1.height = Dimension::Px(30.0);
        let mut s2 = block_style();
        s2.height = Dimension::Px(50.0);

        parent.children = vec![
            LayoutBox::new(BoxType::Block, s1, None),
            LayoutBox::new(BoxType::Block, s2, None),
        ];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;
        layout_block(&mut parent, 480.0, &m);

        let c0_y = parent.children[0].dimensions.content.y;
        let c1_y = parent.children[1].dimensions.content.y;

        assert!(
            c1_y > c0_y,
            "second child should be below first: c0_y={c0_y}, c1_y={c1_y}",
        );
        assert_eq!(
            parent.dimensions.content.height, 80.0,
            "parent height should be sum of children",
        );
    }

    // -- margin collapsing --------------------------------------------

    #[test]
    fn collapse_both_positive() {
        assert_eq!(collapse_margins(10.0, 20.0), 20.0);
        assert_eq!(collapse_margins(20.0, 10.0), 20.0);
    }

    #[test]
    fn collapse_one_negative() {
        assert_eq!(collapse_margins(10.0, -5.0), 5.0);
        assert_eq!(collapse_margins(-5.0, 10.0), 5.0);
    }

    #[test]
    fn collapse_both_negative() {
        assert_eq!(collapse_margins(-10.0, -5.0), -10.0);
        assert_eq!(collapse_margins(-5.0, -10.0), -10.0);
    }

    #[test]
    fn collapse_zero() {
        assert_eq!(collapse_margins(0.0, 0.0), 0.0);
        assert_eq!(collapse_margins(10.0, 0.0), 10.0);
    }

    #[test]
    fn margin_collapsing_between_siblings() {
        let m = FixedMeasurer;
        let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);

        let mut s1 = block_style();
        s1.height = Dimension::Px(20.0);
        s1.margin_bottom = 15.0;

        let mut s2 = block_style();
        s2.height = Dimension::Px(20.0);
        s2.margin_top = 10.0;

        parent.children = vec![
            LayoutBox::new(BoxType::Block, s1, None),
            LayoutBox::new(BoxType::Block, s2, None),
        ];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;
        layout_block(&mut parent, 480.0, &m);

        // With margin collapsing, the space between the two
        // children's border boxes should be max(15, 10) = 15, not
        // the sum 15 + 10 = 25. We verify by checking the gap
        // between the first child's border-box bottom and the
        // second child's border-box top.
        let c0_bb = parent.children[0].dimensions.border_box();
        let c0_bb_bottom = c0_bb.y + c0_bb.height;

        let c1_bb = parent.children[1].dimensions.border_box();
        let c1_bb_top = c1_bb.y;

        let gap = c1_bb_top - c0_bb_bottom;
        assert!(
            (gap - 15.0).abs() < 0.01,
            "collapsed margin between siblings should be 15, got {gap}",
        );
    }

    // -- anonymous box wrapping ----------------------------------------

    #[test]
    fn wrap_anonymous_mixed_children() {
        let inline_box = LayoutBox::new(BoxType::Inline, ComputedStyle::default(), None);
        let block_box = LayoutBox::new(BoxType::Block, block_style(), None);
        let inline_box2 = LayoutBox::new(BoxType::Inline, ComputedStyle::default(), None);

        let wrapped = wrap_anonymous(vec![inline_box, block_box, inline_box2]);

        // Should be: anon(inline), block, anon(inline)
        assert_eq!(wrapped.len(), 3);
        assert!(matches!(wrapped[0].box_type, BoxType::Anonymous));
        assert!(matches!(wrapped[1].box_type, BoxType::Block));
        assert!(matches!(wrapped[2].box_type, BoxType::Anonymous));
    }

    #[test]
    fn wrap_anonymous_all_blocks() {
        let b1 = LayoutBox::new(BoxType::Block, block_style(), None);
        let b2 = LayoutBox::new(BoxType::Block, block_style(), None);
        let wrapped = wrap_anonymous(vec![b1, b2]);
        // No wrapping needed.
        assert_eq!(wrapped.len(), 2);
        assert!(matches!(wrapped[0].box_type, BoxType::Block));
        assert!(matches!(wrapped[1].box_type, BoxType::Block));
    }

    #[test]
    fn wrap_anonymous_all_inline() {
        let i1 = LayoutBox::new(BoxType::Inline, ComputedStyle::default(), None);
        let i2 = LayoutBox::new(BoxType::Inline, ComputedStyle::default(), None);
        let wrapped = wrap_anonymous(vec![i1, i2]);
        // No wrapping needed (all inline).
        assert_eq!(wrapped.len(), 2);
        assert!(matches!(wrapped[0].box_type, BoxType::Inline));
    }
}
