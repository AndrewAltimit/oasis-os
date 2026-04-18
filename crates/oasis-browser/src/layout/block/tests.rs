//! Tests for the block layout engine.

use super::*;
use crate::css::values::{Dimension, Float};

/// Fixed-width text measurer: each character is 8 pixels wide.
struct FixedMeasurer;

impl TextMeasurer for FixedMeasurer {
    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        oasis_types::backend::bitmap_measure_text(text, font_size)
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
    // Sub-pixel: h(7*12/8)+e(7*12/8)+l(5*12/8)+l(5*12/8)+o(7*12/8)
    //          = 10+10+7+7+10 = 44
    assert_eq!(m.measure_text("hello", 12), 44);
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
    style.margin_left_auto = true;
    style.margin_right_auto = true;
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
fn explicit_width_no_auto_margins_left_aligned() {
    let m = FixedMeasurer;
    let mut style = block_style();
    style.width = Dimension::Px(200.0);
    // No auto margins -- should NOT be centered.
    let mut lb = LayoutBox::new(BoxType::Block, style, None);
    lb.dimensions.content.x = 0.0;
    lb.dimensions.content.y = 0.0;
    layout_block(&mut lb, 480.0, &m);
    assert_eq!(lb.dimensions.content.width, 200.0);
    assert!(
        lb.dimensions.margin.left.abs() < f32::EPSILON,
        "left margin should be 0 (left-aligned), got {}",
        lb.dimensions.margin.left,
    );
    assert!(
        (lb.dimensions.margin.right - 280.0).abs() < f32::EPSILON,
        "right margin should absorb remaining space: got {}",
        lb.dimensions.margin.right,
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

    let ps = block_style();
    let wrapped = tree_builder::wrap_anonymous(vec![inline_box, block_box, inline_box2], &ps);

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
    let ps = block_style();
    let wrapped = tree_builder::wrap_anonymous(vec![b1, b2], &ps);
    // No wrapping needed.
    assert_eq!(wrapped.len(), 2);
    assert!(matches!(wrapped[0].box_type, BoxType::Block));
    assert!(matches!(wrapped[1].box_type, BoxType::Block));
}

#[test]
fn wrap_anonymous_all_inline() {
    let i1 = LayoutBox::new(BoxType::Inline, ComputedStyle::default(), None);
    let i2 = LayoutBox::new(BoxType::Inline, ComputedStyle::default(), None);
    let ps = ComputedStyle::default();
    let wrapped = tree_builder::wrap_anonymous(vec![i1, i2], &ps);
    // No wrapping needed (all inline).
    assert_eq!(wrapped.len(), 2);
    assert!(matches!(wrapped[0].box_type, BoxType::Inline));
}

// -- incremental layout -------------------------------------------

#[test]
fn incremental_layout_matches_full_layout() {
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

    // Full layout.
    let mut full = parent.clone();
    layout_block(&mut full, 480.0, &m);

    // Incremental layout (all dirty initially).
    let mut cache = StyleCache::new();
    layout_block_incremental(&mut parent, 480.0, &m, &mut cache);

    assert_eq!(
        parent.dimensions.content.height,
        full.dimensions.content.height,
    );
    assert_eq!(
        parent.dimensions.content.width,
        full.dimensions.content.width,
    );
}

#[test]
fn clean_subtree_skipped_in_incremental() {
    let m = FixedMeasurer;
    let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
    let mut s1 = block_style();
    s1.height = Dimension::Px(30.0);
    parent.children = vec![LayoutBox::new(BoxType::Block, s1, None)];
    parent.dimensions.content.x = 0.0;
    parent.dimensions.content.y = 0.0;

    // First pass: full incremental layout.
    let mut cache = StyleCache::new();
    layout_block_incremental(&mut parent, 480.0, &m, &mut cache);

    // Mark everything clean, then call again -- should be no-op.
    parent.mark_clean();
    let old_height = parent.dimensions.content.height;
    layout_block_incremental(&mut parent, 480.0, &m, &mut cache);
    assert_eq!(parent.dimensions.content.height, old_height);
}

#[test]
fn dirty_child_triggers_relayout() {
    let m = FixedMeasurer;
    let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
    let mut s1 = block_style();
    s1.height = Dimension::Px(30.0);
    parent.children = vec![LayoutBox::new(BoxType::Block, s1, Some(1))];
    parent.dimensions.content.x = 0.0;
    parent.dimensions.content.y = 0.0;

    let mut cache = StyleCache::new();
    layout_block_incremental(&mut parent, 480.0, &m, &mut cache);
    parent.mark_clean();

    // Dirty the child and change its height.
    parent.children[0].dirty = true;
    parent.dirty = true;
    parent.children[0].style.height = Dimension::Px(60.0);
    layout_block_incremental(&mut parent, 480.0, &m, &mut cache);

    assert_eq!(parent.dimensions.content.height, 60.0);
}

// -- style cache --------------------------------------------------

#[test]
fn style_cache_insert_and_get() {
    let mut cache = StyleCache::new();
    assert!(cache.is_empty());
    let pad = EdgeSizes::new(1.0, 2.0, 3.0, 4.0);
    let bdr = EdgeSizes::new(5.0, 6.0, 7.0, 8.0);
    let mar = EdgeSizes::new(9.0, 10.0, 11.0, 12.0);
    cache.insert_edges(42, pad, bdr, mar);
    assert_eq!(cache.len(), 1);
    let (p, b, m) = cache.get_edges(42).expect("cache hit");
    assert_eq!(*p, pad);
    assert_eq!(*b, bdr);
    assert_eq!(*m, mar);
}

#[test]
fn style_cache_miss() {
    let cache = StyleCache::new();
    assert!(cache.get_edges(99).is_none());
}

#[test]
fn style_cache_clear() {
    let mut cache = StyleCache::new();
    cache.insert_edges(
        1,
        EdgeSizes::default(),
        EdgeSizes::default(),
        EdgeSizes::default(),
    );
    cache.clear();
    assert!(cache.is_empty());
}

#[test]
fn mark_dirty_propagation() {
    let mut lb = LayoutBox::new(BoxType::Block, block_style(), None);
    lb.mark_clean();
    assert!(!lb.dirty);
    lb.mark_dirty();
    assert!(lb.dirty);
}

#[test]
fn any_child_dirty_detection() {
    let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
    let mut child = LayoutBox::new(BoxType::Block, block_style(), None);
    child.dirty = false;
    parent.children = vec![child];
    parent.dirty = false;
    assert!(!any_child_dirty(&parent));

    parent.children[0].dirty = true;
    assert!(any_child_dirty(&parent));
}

// -- BUG 1: nested padding not double-counted ---------------------

#[test]
fn nested_padding_not_double_counted() {
    let m = FixedMeasurer;

    // Grandparent with padding 10.
    let mut gp_style = block_style();
    gp_style.padding_left = 10.0;
    gp_style.padding_top = 10.0;
    let mut grandparent = LayoutBox::new(BoxType::Block, gp_style, None);

    // Parent with padding 5.
    let mut p_style = block_style();
    p_style.padding_left = 5.0;
    p_style.padding_top = 5.0;
    p_style.height = Dimension::Px(20.0);
    let mut parent = LayoutBox::new(BoxType::Block, p_style, None);

    // Leaf child with no padding.
    let mut c_style = block_style();
    c_style.height = Dimension::Px(10.0);
    let child = LayoutBox::new(BoxType::Block, c_style, None);

    parent.children = vec![child];
    grandparent.children = vec![parent];
    grandparent.dimensions.content.x = 0.0;
    grandparent.dimensions.content.y = 0.0;
    layout_block(&mut grandparent, 480.0, &m);

    // Apply root offset (same as build_layout_tree does).
    let dx = grandparent.dimensions.margin.left
        + grandparent.dimensions.border.left
        + grandparent.dimensions.padding.left;
    let dy = grandparent.dimensions.margin.top
        + grandparent.dimensions.border.top
        + grandparent.dimensions.padding.top;
    offset_descendant(&mut grandparent, dx, dy);

    // Leaf should be at grandparent_padding(10) + parent_padding(5)
    // = 15, NOT grandparent_padding + 2*parent_padding = 20.
    let leaf = &grandparent.children[0].children[0];
    assert!(
        (leaf.dimensions.content.x - 15.0).abs() < 0.01,
        "leaf x should be 15 (10+5), got {}",
        leaf.dimensions.content.x,
    );
    assert!(
        (leaf.dimensions.content.y - 15.0).abs() < 0.01,
        "leaf y should be 15 (10+5), got {}",
        leaf.dimensions.content.y,
    );
}

// -- BUG 7: parent-child margin collapsing ------------------------

#[test]
fn parent_child_margin_collapsing() {
    let m = FixedMeasurer;

    // Parent with margin-top 10, no padding/border.
    let mut p_style = block_style();
    p_style.margin_top = 10.0;
    let mut parent = LayoutBox::new(BoxType::Block, p_style, None);

    // Child with margin-top 20.
    let mut c_style = block_style();
    c_style.margin_top = 20.0;
    c_style.height = Dimension::Px(30.0);
    let child = LayoutBox::new(BoxType::Block, c_style, None);

    parent.children = vec![child];
    parent.dimensions.content.x = 0.0;
    parent.dimensions.content.y = 0.0;
    layout_block(&mut parent, 480.0, &m);

    // Parent margin-top should collapse with child: max(10, 20) = 20.
    assert!(
        (parent.dimensions.margin.top - 20.0).abs() < 0.01,
        "collapsed parent margin-top should be 20, got {}",
        parent.dimensions.margin.top,
    );

    // Child's margin was absorbed into parent: child sits at
    // parent's content.y without extra gap.
    assert!(
        parent.children[0].dimensions.margin.top.abs() < 0.01,
        "child margin-top should be 0 (absorbed), got {}",
        parent.children[0].dimensions.margin.top,
    );

    // Parent height should be just the child's content (30px),
    // not child content + child margin (50px).
    assert!(
        (parent.dimensions.content.height - 30.0).abs() < 0.01,
        "parent height should be 30 (child only), got {}",
        parent.dimensions.content.height,
    );
}

#[test]
fn parent_child_margin_no_collapse_with_padding() {
    let m = FixedMeasurer;

    // Parent with padding-top (separates margins).
    let mut p_style = block_style();
    p_style.margin_top = 10.0;
    p_style.padding_top = 1.0;
    let mut parent = LayoutBox::new(BoxType::Block, p_style, None);

    let mut c_style = block_style();
    c_style.margin_top = 20.0;
    c_style.height = Dimension::Px(30.0);
    let child = LayoutBox::new(BoxType::Block, c_style, None);

    parent.children = vec![child];
    parent.dimensions.content.x = 0.0;
    parent.dimensions.content.y = 0.0;
    layout_block(&mut parent, 480.0, &m);

    // With padding, no collapsing: parent margin stays 10.
    assert!(
        (parent.dimensions.margin.top - 10.0).abs() < 0.01,
        "parent margin-top should stay 10 with padding, got {}",
        parent.dimensions.margin.top,
    );
}

// -- percentage height resolution ---------------------------------

#[test]
fn test_percentage_height_resolves() {
    let m = FixedMeasurer;
    let mut style = block_style();
    style.height = Dimension::Percent(50.0);

    let mut lb = LayoutBox::new(BoxType::Block, style, None);
    lb.dimensions.content.x = 0.0;
    lb.dimensions.content.y = 0.0;

    // Use layout_block_with_height to pass containing height.
    layout_block_with_height(&mut lb, 480.0, Some(200.0), &m);

    assert!(
        (lb.dimensions.content.height - 100.0).abs() < 0.01,
        "50% of 200px containing height should be 100, got {}",
        lb.dimensions.content.height,
    );
}

// -- min/max height clamping --------------------------------------

#[test]
fn test_min_max_height_clamps() {
    let m = FixedMeasurer;

    // min-height enforced when content is smaller.
    let mut style = block_style();
    style.min_height = Dimension::Px(50.0);
    let mut lb = LayoutBox::new(BoxType::Block, style, None);
    lb.dimensions.content.x = 0.0;
    lb.dimensions.content.y = 0.0;
    layout_block(&mut lb, 480.0, &m);
    assert!(
        lb.dimensions.content.height >= 50.0,
        "min-height 50 should enforce minimum, got {}",
        lb.dimensions.content.height,
    );

    // max-height caps growth.
    let mut style2 = block_style();
    style2.height = Dimension::Px(200.0);
    style2.max_height = Dimension::Px(80.0);
    let mut lb2 = LayoutBox::new(BoxType::Block, style2, None);
    lb2.dimensions.content.x = 0.0;
    lb2.dimensions.content.y = 0.0;
    layout_block(&mut lb2, 480.0, &m);
    assert!(
        (lb2.dimensions.content.height - 80.0).abs() < 0.01,
        "max-height 80 should cap height 200, got {}",
        lb2.dimensions.content.height,
    );
}

// -- min/max width applied ----------------------------------------

#[test]
fn test_min_max_width_applied() {
    let m = FixedMeasurer;

    // min-width enforced.
    let mut style = block_style();
    style.width = Dimension::Auto;
    style.min_width = Dimension::Px(200.0);
    let mut lb = LayoutBox::new(BoxType::Block, style, None);
    lb.dimensions.content.x = 0.0;
    lb.dimensions.content.y = 0.0;
    layout_block(&mut lb, 100.0, &m);
    assert!(
        lb.dimensions.content.width >= 200.0,
        "min-width 200 should override auto width in 100px container, got {}",
        lb.dimensions.content.width,
    );

    // max-width caps.
    let mut style2 = block_style();
    style2.width = Dimension::Auto;
    style2.max_width = Dimension::Px(150.0);
    let mut lb2 = LayoutBox::new(BoxType::Block, style2, None);
    lb2.dimensions.content.x = 0.0;
    lb2.dimensions.content.y = 0.0;
    layout_block(&mut lb2, 400.0, &m);
    assert!(
        (lb2.dimensions.content.width - 150.0).abs() < 0.01,
        "max-width 150 should cap auto width in 400px container, got {}",
        lb2.dimensions.content.width,
    );
}

// -- block-level replaced elements --------------------------------

#[test]
fn hr_gets_container_width() {
    let m = FixedMeasurer;
    let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
    let hr_style = block_style();
    let hr = LayoutBox::new(
        BoxType::Replaced(ReplacedContent::HorizontalRule),
        hr_style,
        None,
    );
    parent.children = vec![hr];
    parent.dimensions.content.x = 0.0;
    parent.dimensions.content.y = 0.0;
    layout_block(&mut parent, 200.0, &m);

    let hr_box = &parent.children[0];
    assert!(
        hr_box.dimensions.content.width > 100.0,
        "HR should stretch to near container width, got {}",
        hr_box.dimensions.content.width,
    );
    assert!(
        hr_box.dimensions.content.height > 0.0,
        "HR should have positive height, got {}",
        hr_box.dimensions.content.height,
    );
}

#[test]
fn hr_is_block_level() {
    let hr = LayoutBox::new(
        BoxType::Replaced(ReplacedContent::HorizontalRule),
        block_style(),
        None,
    );
    assert!(
        hr.is_block_level(),
        "HorizontalRule replaced box should be block-level"
    );
}

// -- empty block self-collapsing -----------------------------------

#[test]
fn empty_block_collapses_own_margins() {
    let m = FixedMeasurer;
    let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);

    // First child: 20px tall.
    let mut s1 = block_style();
    s1.height = Dimension::Px(20.0);
    s1.margin_bottom = 15.0;

    // Empty block between siblings: margins 10 top, 10 bottom.
    // Should self-collapse to 10, then collapse with siblings.
    let mut s_empty = block_style();
    s_empty.margin_top = 10.0;
    s_empty.margin_bottom = 10.0;

    // Third child: 20px tall.
    let mut s3 = block_style();
    s3.height = Dimension::Px(20.0);
    s3.margin_top = 5.0;

    parent.children = vec![
        LayoutBox::new(BoxType::Block, s1, None),
        LayoutBox::new(BoxType::Block, s_empty, None),
        LayoutBox::new(BoxType::Block, s3, None),
    ];
    parent.dimensions.content.x = 0.0;
    parent.dimensions.content.y = 0.0;
    layout_block(&mut parent, 480.0, &m);

    // The empty block self-collapses: max(10, 10) = 10.
    // Collapsing chain: prev_margin_bottom(15) vs empty(10) vs
    // next_top(5) → the whole chain collapses to max(15, 10, 5) = 15.
    let c0_bb = parent.children[0].dimensions.border_box();
    let c0_bottom = c0_bb.y + c0_bb.height;
    let c2_bb = parent.children[2].dimensions.border_box();
    let c2_top = c2_bb.y;
    let gap = c2_top - c0_bottom;

    assert!(
        (gap - 15.0).abs() < 0.01,
        "gap through empty block should be 15 (collapsed chain), got {gap}",
    );
}

// -- BFC inhibits parent-child collapsing -------------------------

#[test]
fn overflow_hidden_inhibits_parent_child_collapsing() {
    let m = FixedMeasurer;

    // Parent with overflow:hidden creates a new BFC.
    let mut p_style = block_style();
    p_style.margin_top = 10.0;
    p_style.overflow = Overflow::Hidden;
    let mut parent = LayoutBox::new(BoxType::Block, p_style, None);

    // Child with margin-top 20.
    let mut c_style = block_style();
    c_style.margin_top = 20.0;
    c_style.height = Dimension::Px(30.0);
    let child = LayoutBox::new(BoxType::Block, c_style, None);

    parent.children = vec![child];
    parent.dimensions.content.x = 0.0;
    parent.dimensions.content.y = 0.0;
    layout_block(&mut parent, 480.0, &m);

    // With overflow:hidden, margins should NOT collapse.
    assert!(
        (parent.dimensions.margin.top - 10.0).abs() < 0.01,
        "parent margin-top should stay 10 with overflow:hidden, got {}",
        parent.dimensions.margin.top,
    );

    // Child keeps its own margin inside the parent.
    assert!(
        (parent.children[0].dimensions.margin.top - 20.0).abs() < 0.01,
        "child margin-top should stay 20 (no collapsing), got {}",
        parent.children[0].dimensions.margin.top,
    );
}

// -- parent-last-child bottom margin collapsing --------------------

// -- real-world layout compliance tests -------------------------------

#[test]
fn nested_percentage_widths() {
    // 50% of 50% of 400px = 100px.
    let m = FixedMeasurer;
    let mut outer = LayoutBox::new(BoxType::Block, block_style(), None);

    let mut mid_style = block_style();
    mid_style.width = Dimension::Percent(50.0);
    let mut mid = LayoutBox::new(BoxType::Block, mid_style, None);

    let mut inner_style = block_style();
    inner_style.width = Dimension::Percent(50.0);
    inner_style.height = Dimension::Px(10.0);
    let inner = LayoutBox::new(BoxType::Block, inner_style, None);

    mid.children = vec![inner];
    outer.children = vec![mid];
    outer.dimensions.content.x = 0.0;
    outer.dimensions.content.y = 0.0;
    layout_block(&mut outer, 400.0, &m);

    let inner_width = outer.children[0].children[0].dimensions.content.width;
    assert!(
        (inner_width - 100.0).abs() < 0.01,
        "50% of 50% of 400 should be 100, got {inner_width}",
    );
}

#[test]
fn min_width_overrides_small_container() {
    let m = FixedMeasurer;
    let mut style = block_style();
    style.width = Dimension::Auto;
    style.min_width = Dimension::Px(300.0);
    let mut lb = LayoutBox::new(BoxType::Block, style, None);
    lb.dimensions.content.x = 0.0;
    lb.dimensions.content.y = 0.0;
    layout_block(&mut lb, 200.0, &m);
    assert!(
        lb.dimensions.content.width >= 300.0,
        "min-width 300 should override 200px container, got {}",
        lb.dimensions.content.width,
    );
}

#[test]
fn max_width_caps_large_container() {
    let m = FixedMeasurer;
    let mut style = block_style();
    style.width = Dimension::Auto;
    style.max_width = Dimension::Px(200.0);
    let mut lb = LayoutBox::new(BoxType::Block, style, None);
    lb.dimensions.content.x = 0.0;
    lb.dimensions.content.y = 0.0;
    layout_block(&mut lb, 500.0, &m);
    assert!(
        (lb.dimensions.content.width - 200.0).abs() < 0.01,
        "max-width 200 should cap auto width in 500px container, got {}",
        lb.dimensions.content.width,
    );
}

#[test]
fn display_none_excluded_from_layout_tree() {
    // Elements with display:none are excluded from the layout tree
    // entirely. If only a visible child is present, the parent
    // height should reflect just that child.
    let m = FixedMeasurer;
    let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);

    let mut visible_style = block_style();
    visible_style.height = Dimension::Px(30.0);

    // Only the visible child is in the layout tree (display:none
    // elements are filtered out during tree construction).
    parent.children = vec![LayoutBox::new(BoxType::Block, visible_style, None)];
    parent.dimensions.content.x = 0.0;
    parent.dimensions.content.y = 0.0;
    layout_block(&mut parent, 480.0, &m);

    assert!(
        (parent.dimensions.content.height - 30.0).abs() < 0.01,
        "parent height should be 30 (only visible child), got {}",
        parent.dimensions.content.height,
    );
}

#[test]
fn deeply_nested_blocks_with_margins() {
    // 10 levels of nesting, each with 2px margin.
    let m = FixedMeasurer;
    let depth = 10;

    fn build_nested(depth: usize) -> LayoutBox {
        let mut style = ComputedStyle::default();
        style.display = Display::Block;
        style.margin_left = 2.0;
        style.margin_right = 2.0;
        if depth == 0 {
            style.height = Dimension::Px(10.0);
            LayoutBox::new(BoxType::Block, style, None)
        } else {
            let child = build_nested(depth - 1);
            let mut lb = LayoutBox::new(BoxType::Block, style, None);
            lb.children = vec![child];
            lb
        }
    }

    let mut root = build_nested(depth);
    root.dimensions.content.x = 0.0;
    root.dimensions.content.y = 0.0;
    layout_block(&mut root, 480.0, &m);

    // The innermost leaf should have width reduced by horizontal
    // margins at each nesting level. The root level itself also
    // consumes margins, so total reduction is (depth+1)*4px.
    let mut node = &root;
    for _ in 0..depth {
        node = &node.children[0];
    }
    // Verify the leaf is significantly narrower than the container
    // (each level shaves off 4px of horizontal margin).
    assert!(
        node.dimensions.content.width < 480.0,
        "leaf should be narrower than container",
    );
    assert!(
        node.dimensions.content.width > 400.0,
        "leaf should still have substantial width, got {}",
        node.dimensions.content.width,
    );
}

// -- aspect-ratio --------------------------------------------------

#[test]
fn aspect_ratio_derives_height_from_explicit_width() {
    let m = FixedMeasurer;
    let mut style = block_style();
    style.width = Dimension::Px(200.0);
    // 2:1 aspect ratio stored as width/height = 2.0.
    style.aspect_ratio = Some(2.0);
    let mut lb = LayoutBox::new(BoxType::Block, style, None);
    lb.dimensions.content.x = 0.0;
    lb.dimensions.content.y = 0.0;
    layout_block(&mut lb, 480.0, &m);
    assert_eq!(lb.dimensions.content.width, 200.0);
    // 200 / 2 = 100.
    assert!(
        (lb.dimensions.content.height - 100.0).abs() < 0.01,
        "expected derived height 100, got {}",
        lb.dimensions.content.height,
    );
}

#[test]
fn aspect_ratio_ignored_when_height_is_explicit() {
    let m = FixedMeasurer;
    let mut style = block_style();
    style.width = Dimension::Px(200.0);
    style.height = Dimension::Px(50.0);
    style.aspect_ratio = Some(2.0);
    let mut lb = LayoutBox::new(BoxType::Block, style, None);
    lb.dimensions.content.x = 0.0;
    lb.dimensions.content.y = 0.0;
    layout_block(&mut lb, 480.0, &m);
    // Explicit height wins; aspect-ratio would've said 100.
    assert_eq!(lb.dimensions.content.height, 50.0);
}

#[test]
fn aspect_ratio_ignored_when_width_is_auto() {
    // We only honour aspect-ratio when the author has set width
    // explicitly — with `width: auto`, the block fills its container
    // and we want content to drive the height as normal.
    let m = FixedMeasurer;
    let mut style = block_style();
    style.aspect_ratio = Some(2.0);
    let mut lb = LayoutBox::new(BoxType::Block, style, None);
    lb.dimensions.content.x = 0.0;
    lb.dimensions.content.y = 0.0;
    layout_block(&mut lb, 480.0, &m);
    // No children, auto height → content.height = 0.
    assert_eq!(lb.dimensions.content.height, 0.0);
}

#[test]
fn replaced_aspect_ratio_derives_height() {
    // A block-level <hr> with aspect-ratio should derive height from
    // the laid-out width and the ratio.
    let m = FixedMeasurer;
    let mut style = block_style();
    style.aspect_ratio = Some(2.0); // width/height = 2 → h = w/2
    let mut hr = LayoutBox::new(
        BoxType::Replaced(ReplacedContent::HorizontalRule),
        style,
        None,
    );
    hr.children = Vec::new();

    let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
    parent.dimensions.content.x = 0.0;
    parent.dimensions.content.y = 0.0;
    parent.dimensions.content.width = 320.0;
    parent.children = vec![hr];
    layout_block_children(&mut parent, &m);
    // HorizontalRule is block-level, stretches to 320px, aspect-ratio
    // 2:1 should produce height = 160.
    let h = parent.children[0].dimensions.content.height;
    assert!(
        (h - 160.0).abs() < 1.0,
        "expected ~160 for 320px wide 2:1, got {h}"
    );
}

#[test]
fn parent_child_bottom_margin_collapsing() {
    let m = FixedMeasurer;

    // Parent with margin-bottom 10, no padding/border, height auto.
    let mut p_style = block_style();
    p_style.margin_bottom = 10.0;
    let mut parent = LayoutBox::new(BoxType::Block, p_style, None);

    // Child with margin-bottom 25.
    let mut c_style = block_style();
    c_style.margin_bottom = 25.0;
    c_style.height = Dimension::Px(30.0);
    let child = LayoutBox::new(BoxType::Block, c_style, None);

    parent.children = vec![child];
    parent.dimensions.content.x = 0.0;
    parent.dimensions.content.y = 0.0;
    layout_block(&mut parent, 480.0, &m);

    // Parent bottom margin should collapse with child: max(10, 25) = 25.
    assert!(
        (parent.dimensions.margin.bottom - 25.0).abs() < 0.01,
        "collapsed parent margin-bottom should be 25, got {}",
        parent.dimensions.margin.bottom,
    );

    // Child's bottom margin was absorbed into parent.
    assert!(
        parent.children[0].dimensions.margin.bottom.abs() < 0.01,
        "child margin-bottom should be 0 (absorbed), got {}",
        parent.children[0].dimensions.margin.bottom,
    );
}

// -- float width: auto shrink-to-fit (CSS 2.1 §10.3.5) -------------

#[test]
fn float_auto_width_shrinks_to_child_extent() {
    // A float with `width: auto` must shrink to its max-content width
    // (= rightmost child border-box edge), not fill the container.
    let m = FixedMeasurer;
    let mut style = block_style();
    style.float = Float::Left;
    // width defaults to Dimension::Auto.

    let mut child_style = block_style();
    child_style.width = Dimension::Px(120.0);
    let child = LayoutBox::new(BoxType::Block, child_style, None);

    let mut lb = LayoutBox::new(BoxType::Block, style, None);
    lb.children = vec![child];
    layout_block(&mut lb, 480.0, &m);

    assert!(
        (lb.dimensions.content.width - 120.0).abs() < 0.01,
        "auto-width float should shrink to 120 (max-content), got {}",
        lb.dimensions.content.width,
    );
}

#[test]
fn float_auto_width_clamped_to_available() {
    // A child that exceeds the container width (e.g. a 600px image
    // inside a 480px body) must not blow the float past the container
    // — shrink-to-fit = min(max-content, available).
    let m = FixedMeasurer;
    let mut style = block_style();
    style.float = Float::Right;

    let mut child_style = block_style();
    child_style.width = Dimension::Px(600.0);
    let child = LayoutBox::new(BoxType::Block, child_style, None);

    let mut lb = LayoutBox::new(BoxType::Block, style, None);
    lb.children = vec![child];
    layout_block(&mut lb, 480.0, &m);

    // Clamped to available (480) even though max-content is 600.
    assert!(
        (lb.dimensions.content.width - 480.0).abs() < 0.01,
        "float width should clamp to available (480), got {}",
        lb.dimensions.content.width,
    );
}

#[test]
fn float_auto_margins_resolve_to_zero() {
    // Per CSS 2.1 §10.3.5, `auto` margins on floats compute to 0
    // (not centered or distributed like normal-flow blocks).
    let m = FixedMeasurer;
    let mut style = block_style();
    style.float = Float::Left;
    style.margin_left_auto = true;
    style.margin_right_auto = true;

    let mut child_style = block_style();
    child_style.width = Dimension::Px(80.0);
    let child = LayoutBox::new(BoxType::Block, child_style, None);

    let mut lb = LayoutBox::new(BoxType::Block, style, None);
    lb.children = vec![child];
    layout_block(&mut lb, 480.0, &m);

    assert_eq!(lb.dimensions.margin.left, 0.0);
    assert_eq!(lb.dimensions.margin.right, 0.0);
}

#[test]
fn non_float_auto_width_still_fills_container() {
    // Regression guard: shrink-to-fit must only fire for floats, not
    // for normal-flow blocks (which fill the container per §10.3.3).
    let m = FixedMeasurer;
    let mut child_style = block_style();
    child_style.width = Dimension::Px(100.0);
    let child = LayoutBox::new(BoxType::Block, child_style, None);

    let mut lb = LayoutBox::new(BoxType::Block, block_style(), None);
    lb.children = vec![child];
    layout_block(&mut lb, 480.0, &m);

    assert_eq!(lb.dimensions.content.width, 480.0);
}
