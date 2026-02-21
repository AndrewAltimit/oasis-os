// WIP: positioned layout is implemented but not yet wired into the main layout engine.
#![allow(dead_code)]

//! CSS positioned layout.
//!
//! Implements `position: relative`, `position: absolute`, and
//! `position: fixed` per CSS 2.1 visual formatting model. Positioned
//! elements are laid out in normal flow first, then offset (relative)
//! or removed from flow and placed against their containing block
//! (absolute/fixed).

use super::box_model::{LayoutBox, Rect};
use crate::css::values::{Dimension, Position};

/// Apply positioning offsets to a layout tree after normal-flow layout
/// is complete.
///
/// This is a post-layout pass that walks the tree and:
/// - Offsets `position: relative` boxes by their `top`/`left`/`bottom`/`right`
/// - Positions `position: absolute` boxes relative to their nearest
///   positioned ancestor's padding box
/// - Positions `position: fixed` boxes relative to the viewport
///
/// `viewport` is the full viewport rect (typically 0,0,480,272).
pub fn apply_positioning(root: &mut LayoutBox, viewport: Rect) {
    apply_positioning_recursive(root, &viewport, &viewport);
}

/// Recursively apply positioning. `containing_block` is the padding box
/// of the nearest positioned ancestor (or viewport for the root).
fn apply_positioning_recursive(
    layout_box: &mut LayoutBox,
    containing_block: &Rect,
    viewport: &Rect,
) {
    let position = layout_box.style.position;

    match position {
        Position::Relative => {
            apply_relative_offset(layout_box, containing_block);
        },
        Position::Absolute => {
            apply_absolute_position(layout_box, containing_block);
        },
        Position::Fixed => {
            apply_absolute_position(layout_box, viewport);
        },
        Position::Static => {},
    }

    // Determine the containing block for positioned descendants.
    let child_cb = if is_positioned(layout_box) {
        layout_box.dimensions.padding_box()
    } else {
        *containing_block
    };

    for child in &mut layout_box.children {
        apply_positioning_recursive(child, &child_cb, viewport);
    }
}

/// Returns true if this box establishes a containing block for
/// absolutely positioned descendants (i.e. position != static).
fn is_positioned(layout_box: &LayoutBox) -> bool {
    !matches!(layout_box.style.position, Position::Static)
}

/// Apply relative positioning offsets.
///
/// The element keeps its position in normal flow but is visually
/// offset by `top`/`left`/`bottom`/`right`. If both `top` and
/// `bottom` are specified, `top` wins. If both `left` and `right`
/// are specified, `left` wins.
fn apply_relative_offset(layout_box: &mut LayoutBox, containing_block: &Rect) {
    let dx = resolve_offset_h(
        &layout_box.style.left,
        &layout_box.style.right,
        containing_block.width,
    );
    let dy = resolve_offset_v(
        &layout_box.style.top,
        &layout_box.style.bottom,
        containing_block.height,
    );

    offset_box(layout_box, dx, dy);
}

/// Position an absolutely positioned element relative to its
/// containing block.
///
/// The element is removed from normal flow and placed at the
/// coordinates specified by `top`/`left`/`bottom`/`right` relative
/// to the containing block's padding box.
fn apply_absolute_position(layout_box: &mut LayoutBox, containing_block: &Rect) {
    let style = &layout_box.style;
    let cb = containing_block;

    // Resolve horizontal position.
    let x = match (&style.left, &style.right) {
        (Dimension::Px(l), _) => cb.x + l,
        (Dimension::Percent(p), _) => cb.x + cb.width * (p / 100.0),
        (Dimension::Auto, Dimension::Px(r)) => {
            cb.x + cb.width - r - layout_box.dimensions.margin_box().width
        },
        (Dimension::Auto, Dimension::Percent(p)) => {
            cb.x + cb.width - cb.width * (p / 100.0) - layout_box.dimensions.margin_box().width
        },
        (Dimension::Auto, Dimension::Auto) => {
            // No offset specified: use the static position (current x).
            layout_box.dimensions.content.x
                - layout_box.dimensions.padding.left
                - layout_box.dimensions.border.left
                - layout_box.dimensions.margin.left
        },
    };

    // Resolve vertical position.
    let y = match (&style.top, &style.bottom) {
        (Dimension::Px(t), _) => cb.y + t,
        (Dimension::Percent(p), _) => cb.y + cb.height * (p / 100.0),
        (Dimension::Auto, Dimension::Px(b)) => {
            cb.y + cb.height - b - layout_box.dimensions.margin_box().height
        },
        (Dimension::Auto, Dimension::Percent(p)) => {
            cb.y + cb.height - cb.height * (p / 100.0) - layout_box.dimensions.margin_box().height
        },
        (Dimension::Auto, Dimension::Auto) => {
            layout_box.dimensions.content.y
                - layout_box.dimensions.padding.top
                - layout_box.dimensions.border.top
                - layout_box.dimensions.margin.top
        },
    };

    // Set the content position from the resolved margin-box position.
    layout_box.dimensions.content.x = x
        + layout_box.dimensions.margin.left
        + layout_box.dimensions.border.left
        + layout_box.dimensions.padding.left;
    layout_box.dimensions.content.y = y
        + layout_box.dimensions.margin.top
        + layout_box.dimensions.border.top
        + layout_box.dimensions.padding.top;
}

/// Resolve horizontal offset (left vs right).
/// `left` wins when both are specified.
fn resolve_offset_h(left: &Dimension, right: &Dimension, container_width: f32) -> f32 {
    match left {
        Dimension::Px(l) => *l,
        Dimension::Percent(p) => container_width * (p / 100.0),
        Dimension::Auto => match right {
            Dimension::Px(r) => -*r,
            Dimension::Percent(p) => -(container_width * (p / 100.0)),
            Dimension::Auto => 0.0,
        },
    }
}

/// Resolve vertical offset (top vs bottom).
/// `top` wins when both are specified.
fn resolve_offset_v(top: &Dimension, bottom: &Dimension, container_height: f32) -> f32 {
    match top {
        Dimension::Px(t) => *t,
        Dimension::Percent(p) => container_height * (p / 100.0),
        Dimension::Auto => match bottom {
            Dimension::Px(b) => -*b,
            Dimension::Percent(p) => -(container_height * (p / 100.0)),
            Dimension::Auto => 0.0,
        },
    }
}

/// Offset a layout box and all its descendants by (dx, dy).
fn offset_box(layout_box: &mut LayoutBox, dx: f32, dy: f32) {
    layout_box.dimensions.content.x += dx;
    layout_box.dimensions.content.y += dy;
    // Offset children too so they follow the parent's visual shift.
    for child in &mut layout_box.children {
        offset_box(child, dx, dy);
    }
}

/// Collect positioned boxes in z-index order for painting.
///
/// Returns a list of references sorted by z-index (ascending), then
/// tree order for equal z-index values. This can be used by the paint
/// module to establish stacking contexts.
pub fn collect_stacking_order(root: &LayoutBox) -> Vec<(i32, usize, &LayoutBox)> {
    let mut items = Vec::new();
    collect_recursive(root, &mut items, 0);
    // Stable sort by z-index; tree order preserved for equal z-index.
    items.sort_by_key(|&(z, order, _)| (z, order));
    items
}

fn collect_recursive<'a>(
    layout_box: &'a LayoutBox,
    items: &mut Vec<(i32, usize, &'a LayoutBox)>,
    order: usize,
) -> usize {
    let mut current_order = order;
    if !matches!(layout_box.style.position, Position::Static) {
        items.push((layout_box.style.z_index, current_order, layout_box));
    }
    current_order += 1;
    for child in &layout_box.children {
        current_order = collect_recursive(child, items, current_order);
    }
    current_order
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::{ComputedStyle, Display};

    fn block_style() -> ComputedStyle {
        let mut s = ComputedStyle::default();
        s.display = Display::Block;
        s
    }

    fn positioned_box(
        position: Position,
        top: Dimension,
        left: Dimension,
        bottom: Dimension,
        right: Dimension,
    ) -> LayoutBox {
        let mut s = block_style();
        s.position = position;
        s.top = top;
        s.left = left;
        s.bottom = bottom;
        s.right = right;
        let mut lb = LayoutBox::new(super::super::box_model::BoxType::Block, s, None);
        lb.dimensions.content.width = 50.0;
        lb.dimensions.content.height = 20.0;
        lb
    }

    fn viewport() -> Rect {
        Rect::new(0.0, 0.0, 480.0, 272.0)
    }

    // -- Relative positioning -------------------------------------------

    #[test]
    fn relative_offset_top_left() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let mut child = positioned_box(
            Position::Relative,
            Dimension::Px(10.0),
            Dimension::Px(20.0),
            Dimension::Auto,
            Dimension::Auto,
        );
        child.dimensions.content.x = 0.0;
        child.dimensions.content.y = 0.0;
        root.children = vec![child];

        apply_positioning(&mut root, viewport());

        let c = &root.children[0];
        assert!(
            (c.dimensions.content.x - 20.0).abs() < f32::EPSILON,
            "x should be 20: got {}",
            c.dimensions.content.x,
        );
        assert!(
            (c.dimensions.content.y - 10.0).abs() < f32::EPSILON,
            "y should be 10: got {}",
            c.dimensions.content.y,
        );
    }

    #[test]
    fn relative_offset_bottom_right() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let mut child = positioned_box(
            Position::Relative,
            Dimension::Auto,
            Dimension::Auto,
            Dimension::Px(5.0),
            Dimension::Px(15.0),
        );
        child.dimensions.content.x = 100.0;
        child.dimensions.content.y = 50.0;
        root.children = vec![child];

        apply_positioning(&mut root, viewport());

        let c = &root.children[0];
        // right: 15px => offset x by -15
        assert!(
            (c.dimensions.content.x - 85.0).abs() < f32::EPSILON,
            "x should be 85: got {}",
            c.dimensions.content.x,
        );
        // bottom: 5px => offset y by -5
        assert!(
            (c.dimensions.content.y - 45.0).abs() < f32::EPSILON,
            "y should be 45: got {}",
            c.dimensions.content.y,
        );
    }

    #[test]
    fn relative_top_wins_over_bottom() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let mut child = positioned_box(
            Position::Relative,
            Dimension::Px(10.0),
            Dimension::Auto,
            Dimension::Px(50.0),
            Dimension::Auto,
        );
        child.dimensions.content.x = 0.0;
        child.dimensions.content.y = 0.0;
        root.children = vec![child];

        apply_positioning(&mut root, viewport());

        let c = &root.children[0];
        // top wins: y += 10
        assert!(
            (c.dimensions.content.y - 10.0).abs() < f32::EPSILON,
            "top should win: y should be 10, got {}",
            c.dimensions.content.y,
        );
    }

    #[test]
    fn relative_left_wins_over_right() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let mut child = positioned_box(
            Position::Relative,
            Dimension::Auto,
            Dimension::Px(20.0),
            Dimension::Auto,
            Dimension::Px(100.0),
        );
        child.dimensions.content.x = 0.0;
        child.dimensions.content.y = 0.0;
        root.children = vec![child];

        apply_positioning(&mut root, viewport());

        let c = &root.children[0];
        // left wins: x += 20
        assert!(
            (c.dimensions.content.x - 20.0).abs() < f32::EPSILON,
            "left should win: x should be 20, got {}",
            c.dimensions.content.x,
        );
    }

    #[test]
    fn relative_no_offset() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let mut child = positioned_box(
            Position::Relative,
            Dimension::Auto,
            Dimension::Auto,
            Dimension::Auto,
            Dimension::Auto,
        );
        child.dimensions.content.x = 30.0;
        child.dimensions.content.y = 40.0;
        root.children = vec![child];

        apply_positioning(&mut root, viewport());

        let c = &root.children[0];
        assert!(
            (c.dimensions.content.x - 30.0).abs() < f32::EPSILON,
            "should stay at 30: got {}",
            c.dimensions.content.x,
        );
        assert!(
            (c.dimensions.content.y - 40.0).abs() < f32::EPSILON,
            "should stay at 40: got {}",
            c.dimensions.content.y,
        );
    }

    // -- Absolute positioning -------------------------------------------

    #[test]
    fn absolute_top_left() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let child = positioned_box(
            Position::Absolute,
            Dimension::Px(10.0),
            Dimension::Px(20.0),
            Dimension::Auto,
            Dimension::Auto,
        );
        root.children = vec![child];

        apply_positioning(&mut root, viewport());

        let c = &root.children[0];
        assert!(
            (c.dimensions.content.x - 20.0).abs() < f32::EPSILON,
            "abs x should be 20: got {}",
            c.dimensions.content.x,
        );
        assert!(
            (c.dimensions.content.y - 10.0).abs() < f32::EPSILON,
            "abs y should be 10: got {}",
            c.dimensions.content.y,
        );
    }

    #[test]
    fn absolute_bottom_right() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let child = positioned_box(
            Position::Absolute,
            Dimension::Auto,
            Dimension::Auto,
            Dimension::Px(10.0),
            Dimension::Px(20.0),
        );
        root.children = vec![child];

        apply_positioning(&mut root, viewport());

        let c = &root.children[0];
        // right: 20px => x = 480 - 20 - 50 (box width) = 410
        assert!(
            (c.dimensions.content.x - 410.0).abs() < f32::EPSILON,
            "abs x should be 410: got {}",
            c.dimensions.content.x,
        );
        // bottom: 10px => y = 272 - 10 - 20 (box height) = 242
        assert!(
            (c.dimensions.content.y - 242.0).abs() < f32::EPSILON,
            "abs y should be 242: got {}",
            c.dimensions.content.y,
        );
    }

    #[test]
    fn absolute_inside_positioned_ancestor() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        // Positioned ancestor at (50, 50) with 200x100 content.
        let mut ancestor_style = block_style();
        ancestor_style.position = Position::Relative;
        let mut ancestor = LayoutBox::new(
            super::super::box_model::BoxType::Block,
            ancestor_style,
            None,
        );
        ancestor.dimensions.content = Rect::new(50.0, 50.0, 200.0, 100.0);

        let child = positioned_box(
            Position::Absolute,
            Dimension::Px(5.0),
            Dimension::Px(10.0),
            Dimension::Auto,
            Dimension::Auto,
        );
        ancestor.children = vec![child];
        root.children = vec![ancestor];

        apply_positioning(&mut root, viewport());

        let c = &root.children[0].children[0];
        // Containing block is ancestor's padding box (50, 50, 200, 100).
        assert!(
            (c.dimensions.content.x - 60.0).abs() < f32::EPSILON,
            "abs inside ancestor: x should be 60, got {}",
            c.dimensions.content.x,
        );
        assert!(
            (c.dimensions.content.y - 55.0).abs() < f32::EPSILON,
            "abs inside ancestor: y should be 55, got {}",
            c.dimensions.content.y,
        );
    }

    // -- Fixed positioning ----------------------------------------------

    #[test]
    fn fixed_position_relative_to_viewport() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        // Even inside a positioned ancestor, fixed is relative to viewport.
        let mut ancestor_style = block_style();
        ancestor_style.position = Position::Relative;
        let mut ancestor = LayoutBox::new(
            super::super::box_model::BoxType::Block,
            ancestor_style,
            None,
        );
        ancestor.dimensions.content = Rect::new(100.0, 100.0, 200.0, 100.0);

        let child = positioned_box(
            Position::Fixed,
            Dimension::Px(0.0),
            Dimension::Px(0.0),
            Dimension::Auto,
            Dimension::Auto,
        );
        ancestor.children = vec![child];
        root.children = vec![ancestor];

        apply_positioning(&mut root, viewport());

        let c = &root.children[0].children[0];
        assert!(
            (c.dimensions.content.x).abs() < f32::EPSILON,
            "fixed x should be 0: got {}",
            c.dimensions.content.x,
        );
        assert!(
            (c.dimensions.content.y).abs() < f32::EPSILON,
            "fixed y should be 0: got {}",
            c.dimensions.content.y,
        );
    }

    #[test]
    fn fixed_bottom_right() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let child = positioned_box(
            Position::Fixed,
            Dimension::Auto,
            Dimension::Auto,
            Dimension::Px(0.0),
            Dimension::Px(0.0),
        );
        root.children = vec![child];

        apply_positioning(&mut root, viewport());

        let c = &root.children[0];
        // bottom: 0, right: 0 => snaps to bottom-right corner.
        // x = 480 - 0 - 50 = 430
        // y = 272 - 0 - 20 = 252
        assert!(
            (c.dimensions.content.x - 430.0).abs() < f32::EPSILON,
            "fixed bottom-right x should be 430: got {}",
            c.dimensions.content.x,
        );
        assert!(
            (c.dimensions.content.y - 252.0).abs() < f32::EPSILON,
            "fixed bottom-right y should be 252: got {}",
            c.dimensions.content.y,
        );
    }

    // -- Static positioning (no-op) -------------------------------------

    #[test]
    fn static_position_unchanged() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let mut child = positioned_box(
            Position::Static,
            Dimension::Px(999.0), // Should be ignored.
            Dimension::Px(999.0),
            Dimension::Auto,
            Dimension::Auto,
        );
        child.dimensions.content.x = 30.0;
        child.dimensions.content.y = 40.0;
        root.children = vec![child];

        apply_positioning(&mut root, viewport());

        let c = &root.children[0];
        assert!(
            (c.dimensions.content.x - 30.0).abs() < f32::EPSILON,
            "static: x should remain 30, got {}",
            c.dimensions.content.x,
        );
        assert!(
            (c.dimensions.content.y - 40.0).abs() < f32::EPSILON,
            "static: y should remain 40, got {}",
            c.dimensions.content.y,
        );
    }

    // -- Relative offsets children too ----------------------------------

    #[test]
    fn relative_offset_cascades_to_children() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let mut parent = positioned_box(
            Position::Relative,
            Dimension::Px(10.0),
            Dimension::Px(20.0),
            Dimension::Auto,
            Dimension::Auto,
        );
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;

        let mut grandchild =
            LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        grandchild.dimensions.content.x = 5.0;
        grandchild.dimensions.content.y = 5.0;
        parent.children = vec![grandchild];
        root.children = vec![parent];

        apply_positioning(&mut root, viewport());

        let gc = &root.children[0].children[0];
        // Grandchild should move by the same offset as parent.
        assert!(
            (gc.dimensions.content.x - 25.0).abs() < f32::EPSILON,
            "grandchild x: expected 25, got {}",
            gc.dimensions.content.x,
        );
        assert!(
            (gc.dimensions.content.y - 15.0).abs() < f32::EPSILON,
            "grandchild y: expected 15, got {}",
            gc.dimensions.content.y,
        );
    }

    // -- Percentage offsets ---------------------------------------------

    #[test]
    fn relative_percent_offset() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let mut child = positioned_box(
            Position::Relative,
            Dimension::Percent(10.0), // 10% of 272 = 27.2
            Dimension::Percent(50.0), // 50% of 480 = 240
            Dimension::Auto,
            Dimension::Auto,
        );
        child.dimensions.content.x = 0.0;
        child.dimensions.content.y = 0.0;
        root.children = vec![child];

        apply_positioning(&mut root, viewport());

        let c = &root.children[0];
        assert!(
            (c.dimensions.content.x - 240.0).abs() < 0.1,
            "50% left: expected 240, got {}",
            c.dimensions.content.x,
        );
        assert!(
            (c.dimensions.content.y - 27.2).abs() < 0.1,
            "10% top: expected 27.2, got {}",
            c.dimensions.content.y,
        );
    }

    #[test]
    fn absolute_percent_position() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let child = positioned_box(
            Position::Absolute,
            Dimension::Percent(50.0), // 50% of 272 = 136
            Dimension::Percent(25.0), // 25% of 480 = 120
            Dimension::Auto,
            Dimension::Auto,
        );
        root.children = vec![child];

        apply_positioning(&mut root, viewport());

        let c = &root.children[0];
        assert!(
            (c.dimensions.content.x - 120.0).abs() < 0.1,
            "25% left: expected 120, got {}",
            c.dimensions.content.x,
        );
        assert!(
            (c.dimensions.content.y - 136.0).abs() < 0.1,
            "50% top: expected 136, got {}",
            c.dimensions.content.y,
        );
    }

    // -- Z-index stacking order -----------------------------------------

    #[test]
    fn stacking_order_by_z_index() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let mut child_a = positioned_box(
            Position::Relative,
            Dimension::Auto,
            Dimension::Auto,
            Dimension::Auto,
            Dimension::Auto,
        );
        child_a.style.z_index = 10;

        let mut child_b = positioned_box(
            Position::Relative,
            Dimension::Auto,
            Dimension::Auto,
            Dimension::Auto,
            Dimension::Auto,
        );
        child_b.style.z_index = 5;

        let mut child_c = positioned_box(
            Position::Absolute,
            Dimension::Px(0.0),
            Dimension::Px(0.0),
            Dimension::Auto,
            Dimension::Auto,
        );
        child_c.style.z_index = 20;

        root.children = vec![child_a, child_b, child_c];

        let order = collect_stacking_order(&root);
        assert_eq!(order.len(), 3);
        // Should be sorted by z-index: 5, 10, 20.
        assert_eq!(order[0].0, 5);
        assert_eq!(order[1].0, 10);
        assert_eq!(order[2].0, 20);
    }

    #[test]
    fn stacking_order_excludes_static() {
        let mut root = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        root.dimensions.content = Rect::new(0.0, 0.0, 480.0, 272.0);

        let static_child =
            LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        let positioned_child = positioned_box(
            Position::Relative,
            Dimension::Auto,
            Dimension::Auto,
            Dimension::Auto,
            Dimension::Auto,
        );
        root.children = vec![static_child, positioned_child];

        let order = collect_stacking_order(&root);
        // Only the positioned child should appear.
        assert_eq!(order.len(), 1);
    }

    // -- Helper unit tests ----------------------------------------------

    #[test]
    fn resolve_offset_h_left_px() {
        let dx = resolve_offset_h(&Dimension::Px(10.0), &Dimension::Auto, 480.0);
        assert!((dx - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn resolve_offset_h_right_px() {
        let dx = resolve_offset_h(&Dimension::Auto, &Dimension::Px(10.0), 480.0);
        assert!((dx - (-10.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn resolve_offset_h_both_auto() {
        let dx = resolve_offset_h(&Dimension::Auto, &Dimension::Auto, 480.0);
        assert!((dx).abs() < f32::EPSILON);
    }

    #[test]
    fn resolve_offset_v_top_px() {
        let dy = resolve_offset_v(&Dimension::Px(5.0), &Dimension::Auto, 272.0);
        assert!((dy - 5.0).abs() < f32::EPSILON);
    }

    #[test]
    fn resolve_offset_v_bottom_px() {
        let dy = resolve_offset_v(&Dimension::Auto, &Dimension::Px(5.0), 272.0);
        assert!((dy - (-5.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn resolve_offset_v_percent() {
        let dy = resolve_offset_v(&Dimension::Percent(50.0), &Dimension::Auto, 272.0);
        assert!((dy - 136.0).abs() < 0.1);
    }

    #[test]
    fn is_positioned_checks() {
        let mut lb = LayoutBox::new(super::super::box_model::BoxType::Block, block_style(), None);
        assert!(!is_positioned(&lb));

        lb.style.position = Position::Relative;
        assert!(is_positioned(&lb));

        lb.style.position = Position::Absolute;
        assert!(is_positioned(&lb));

        lb.style.position = Position::Fixed;
        assert!(is_positioned(&lb));
    }
}
