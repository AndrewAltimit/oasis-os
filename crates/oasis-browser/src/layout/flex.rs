//! CSS Flexbox layout algorithm.
//!
//! Implements a simplified CSS Flexible Box Layout (Level 1) for the
//! OASIS browser engine. Supports `flex-direction`, `justify-content`,
//! `align-items`, `flex-grow`, `flex-shrink`, `flex-basis`, `gap`,
//! and `flex-wrap`.

use super::block::{TextMeasurer, layout_block, resolve_edge_sizes};
use super::box_model::*;
use crate::css::values::{AlignItems, Dimension, FlexDirection, FlexWrap, JustifyContent};

/// Lay out a flex container and all its children.
///
/// The flex container's `content.x`, `content.y`, and `content.width`
/// must already be set by the caller. This function resolves flex item
/// sizes, distributes free space, and positions children according to
/// the container's flex properties.
pub fn layout_flex(container: &mut LayoutBox, _containing_width: f32, measurer: &dyn TextMeasurer) {
    let direction = container.style.flex_direction;
    let justify = container.style.justify_content;
    let align = container.style.align_items;
    let wrap = container.style.flex_wrap;
    let gap = container.style.gap;

    let content_width = container.dimensions.content.width;
    let content_x = container.dimensions.content.x;
    let content_y = container.dimensions.content.y;
    let pad_top = container.dimensions.padding.top;
    let pad_left = container.dimensions.padding.left;

    let is_row = matches!(direction, FlexDirection::Row | FlexDirection::RowReverse);
    let is_reverse = matches!(
        direction,
        FlexDirection::RowReverse | FlexDirection::ColumnReverse
    );
    let main_size = if is_row { content_width } else { f32::MAX };

    // -- Phase 1: Resolve each child's base (hypothetical) size ----------
    for child in &mut container.children {
        resolve_edge_sizes(child, content_width);
    }

    // Collect flex item info.
    let mut items: Vec<FlexItem> = Vec::with_capacity(container.children.len());
    for (i, child) in container.children.iter_mut().enumerate() {
        let basis = resolve_flex_basis(child, is_row, content_width);
        // Layout child at its basis width to determine its intrinsic cross
        // size. We'll reposition it later.
        let temp_width = if is_row { basis } else { content_width };
        child.dimensions.content.x = 0.0;
        child.dimensions.content.y = 0.0;
        layout_block(child, temp_width, measurer);
        let intrinsic_main = if is_row {
            child.dimensions.margin_box().width
        } else {
            child.dimensions.margin_box().height
        };
        let intrinsic_cross = if is_row {
            child.dimensions.margin_box().height
        } else {
            child.dimensions.margin_box().width
        };
        items.push(FlexItem {
            index: i,
            grow: child.style.flex_grow,
            shrink: child.style.flex_shrink,
            main: intrinsic_main,
            cross: intrinsic_cross,
        });
    }

    // -- Phase 2: Split items into flex lines ----------------------------
    let lines = split_into_lines(&items, main_size, gap, wrap);

    // -- Phase 3: Resolve flexible lengths per line ----------------------
    let mut resolved_lines: Vec<Vec<(usize, f32, f32)>> = Vec::new();
    for line in &lines {
        let resolved = resolve_flex_line(line, main_size, gap);
        resolved_lines.push(resolved);
    }

    // -- Phase 4: Position items -----------------------------------------
    let mut cross_offset: f32 = 0.0;
    for resolved_line in &resolved_lines {
        // Find the max cross size in this line for alignment.
        let line_cross = resolved_line
            .iter()
            .map(|&(_, _, cross)| cross)
            .fold(0.0f32, f32::max);

        // Calculate total main size used by items + gaps.
        let n = resolved_line.len();
        let total_gaps = if n > 1 { gap * (n as f32 - 1.0) } else { 0.0 };
        let total_main: f32 = resolved_line.iter().map(|&(_, m, _)| m).sum::<f32>() + total_gaps;
        let free_space = (main_size - total_main).max(0.0);

        // Compute starting offset and inter-item spacing from
        // justify-content.
        let (mut main_offset, inter_gap) =
            compute_justification(justify, free_space, n, is_reverse);

        // If reversed, iterate in reverse order.
        let order: Vec<usize> = if is_reverse {
            (0..n).rev().collect()
        } else {
            (0..n).collect()
        };

        for (seq, &idx) in order.iter().enumerate() {
            let (child_idx, item_main, item_cross) = resolved_line[idx];
            let child = &mut container.children[child_idx];

            // Cross-axis alignment.
            let cross_pos = compute_cross_alignment(align, cross_offset, line_cross, item_cross);

            if is_row {
                let margin_h = child.dimensions.margin.horizontal()
                    + child.dimensions.border.horizontal()
                    + child.dimensions.padding.horizontal();
                let new_w = (item_main - margin_h).max(0.0);
                // Override the child's style width so layout_block
                // uses the flex-resolved size instead of the original.
                child.style.width = Dimension::Px(new_w);
                child.dimensions.content.x = content_x
                    + pad_left
                    + main_offset
                    + child.dimensions.margin.left
                    + child.dimensions.border.left
                    + child.dimensions.padding.left;
                child.dimensions.content.y = content_y
                    + pad_top
                    + cross_pos
                    + child.dimensions.margin.top
                    + child.dimensions.border.top
                    + child.dimensions.padding.top;
                // Re-layout with the resolved width.
                layout_block(child, new_w, measurer);
            } else {
                let margin_v = child.dimensions.margin.vertical()
                    + child.dimensions.border.vertical()
                    + child.dimensions.padding.vertical();
                let new_h = (item_main - margin_v).max(0.0);
                child.style.height = Dimension::Px(new_h);
                child.dimensions.content.x = content_x
                    + pad_left
                    + cross_pos
                    + child.dimensions.margin.left
                    + child.dimensions.border.left
                    + child.dimensions.padding.left;
                child.dimensions.content.y = content_y
                    + pad_top
                    + main_offset
                    + child.dimensions.margin.top
                    + child.dimensions.border.top
                    + child.dimensions.padding.top;
                child.dimensions.content.height = new_h;
            }

            main_offset += item_main;
            // Add gap unless this is the last item.
            if seq < n - 1 {
                main_offset += gap + inter_gap;
            }
        }

        cross_offset += line_cross + gap;
    }

    // -- Phase 5: Container height = sum of line cross sizes -------------
    let total_cross = if resolved_lines.is_empty() {
        0.0
    } else {
        let n_lines = resolved_lines.len();
        let lines_gap = if n_lines > 1 {
            gap * (n_lines as f32 - 1.0)
        } else {
            0.0
        };
        resolved_lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|&(_, _, cross)| cross)
                    .fold(0.0f32, f32::max)
            })
            .sum::<f32>()
            + lines_gap
    };

    // Only set auto height; explicit height is already set.
    if matches!(container.style.height, Dimension::Auto) {
        container.dimensions.content.height = total_cross + container.dimensions.padding.vertical();
    }
}

// -------------------------------------------------------------------
// Internal helpers
// -------------------------------------------------------------------

/// Intermediate flex item data.
struct FlexItem {
    index: usize,
    grow: f32,
    shrink: f32,
    main: f32,
    cross: f32,
}

/// Resolve the flex basis for an item.
fn resolve_flex_basis(child: &LayoutBox, is_row: bool, containing_width: f32) -> f32 {
    match child.style.flex_basis {
        Dimension::Px(px) => px,
        Dimension::Percent(pct) => containing_width * (pct / 100.0),
        Dimension::Auto => {
            // Fall back to width/height property.
            let dim = if is_row {
                child.style.width
            } else {
                child.style.height
            };
            match dim {
                Dimension::Px(px) => px,
                Dimension::Percent(pct) => containing_width * (pct / 100.0),
                Dimension::Auto => 0.0,
            }
        },
    }
}

/// Split flex items into lines based on wrap mode and available main size.
fn split_into_lines(
    items: &[FlexItem],
    main_size: f32,
    gap: f32,
    wrap: FlexWrap,
) -> Vec<Vec<&FlexItem>> {
    if items.is_empty() {
        return vec![];
    }

    if matches!(wrap, FlexWrap::NoWrap) || main_size == f32::MAX {
        // Single line.
        return vec![items.iter().collect()];
    }

    let mut lines: Vec<Vec<&FlexItem>> = Vec::new();
    let mut current_line: Vec<&FlexItem> = Vec::new();
    let mut line_main: f32 = 0.0;

    for item in items {
        let item_main = item.main;
        let needed = if current_line.is_empty() {
            item_main
        } else {
            item_main + gap
        };

        if !current_line.is_empty() && line_main + needed > main_size {
            lines.push(std::mem::take(&mut current_line));
            line_main = 0.0;
        }

        if !current_line.is_empty() {
            line_main += gap;
        }
        line_main += item_main;
        current_line.push(item);
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    if matches!(wrap, FlexWrap::WrapReverse) {
        lines.reverse();
    }

    lines
}

/// Resolve flexible lengths for a single flex line.
///
/// Returns `(child_index, resolved_main_size, cross_size)` for each item.
fn resolve_flex_line(line: &[&FlexItem], main_size: f32, gap: f32) -> Vec<(usize, f32, f32)> {
    let n = line.len();
    let total_gaps = if n > 1 { gap * (n as f32 - 1.0) } else { 0.0 };
    let total_base: f32 = line.iter().map(|item| item.main).sum();
    let free_space = main_size - total_base - total_gaps;

    let mut result: Vec<(usize, f32, f32)> = Vec::with_capacity(n);

    if free_space > 0.0 {
        // Positive free space: distribute via flex-grow.
        let total_grow: f32 = line.iter().map(|item| item.grow).sum();
        for item in line {
            let extra = if total_grow > 0.0 {
                (item.grow / total_grow) * free_space
            } else {
                0.0
            };
            result.push((item.index, item.main + extra, item.cross));
        }
    } else if free_space < 0.0 {
        // Negative free space: shrink via flex-shrink.
        let total_shrink: f32 = line.iter().map(|item| item.shrink * item.main).sum();
        for item in line {
            let reduction = if total_shrink > 0.0 {
                (item.shrink * item.main / total_shrink) * (-free_space)
            } else {
                0.0
            };
            result.push((item.index, (item.main - reduction).max(0.0), item.cross));
        }
    } else {
        for item in line {
            result.push((item.index, item.main, item.cross));
        }
    }

    result
}

/// Compute the starting main-axis offset and extra inter-item spacing
/// from the `justify-content` value.
fn compute_justification(
    justify: JustifyContent,
    free_space: f32,
    count: usize,
    is_reverse: bool,
) -> (f32, f32) {
    if count == 0 {
        return (0.0, 0.0);
    }
    let (start, spacing) = match justify {
        JustifyContent::FlexStart => {
            if is_reverse {
                (free_space, 0.0)
            } else {
                (0.0, 0.0)
            }
        },
        JustifyContent::FlexEnd => {
            if is_reverse {
                (0.0, 0.0)
            } else {
                (free_space, 0.0)
            }
        },
        JustifyContent::Center => (free_space / 2.0, 0.0),
        JustifyContent::SpaceBetween => {
            if count <= 1 {
                (0.0, 0.0)
            } else {
                (0.0, free_space / (count as f32 - 1.0))
            }
        },
        JustifyContent::SpaceAround => {
            let per = free_space / count as f32;
            (per / 2.0, per / 2.0)
        },
        JustifyContent::SpaceEvenly => {
            let per = free_space / (count as f32 + 1.0);
            (per, per)
        },
    };
    (start, spacing)
}

/// Compute the cross-axis position for an item given the line's cross
/// size and the container's `align-items` value.
fn compute_cross_alignment(
    align: AlignItems,
    cross_offset: f32,
    line_cross: f32,
    item_cross: f32,
) -> f32 {
    match align {
        AlignItems::FlexStart | AlignItems::Baseline => cross_offset,
        AlignItems::FlexEnd => cross_offset + (line_cross - item_cross),
        AlignItems::Center => cross_offset + (line_cross - item_cross) / 2.0,
        AlignItems::Stretch => cross_offset,
    }
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::{ComputedStyle, Display, FlexDirection};
    use crate::layout::block::TextMeasurer;

    struct FixedMeasurer;

    impl TextMeasurer for FixedMeasurer {
        fn measure_text(&self, text: &str, font_size: u16) -> u32 {
            oasis_types::backend::bitmap_measure_text(text, font_size)
        }
    }

    fn flex_style() -> ComputedStyle {
        let mut s = ComputedStyle::default();
        s.display = Display::Flex;
        s
    }

    fn item_style(width: f32, height: f32) -> ComputedStyle {
        let mut s = ComputedStyle::default();
        s.display = Display::Block;
        s.width = Dimension::Px(width);
        s.height = Dimension::Px(height);
        s
    }

    fn make_flex_container(style: ComputedStyle, children: Vec<LayoutBox>) -> LayoutBox {
        let mut lb = LayoutBox::new(BoxType::Flex, style, None);
        lb.children = children;
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;
        lb.dimensions.content.width = 480.0;
        lb
    }

    fn make_item(width: f32, height: f32) -> LayoutBox {
        LayoutBox::new(BoxType::Block, item_style(width, height), None)
    }

    // -- Row layout -------------------------------------------------------

    #[test]
    fn flex_row_items_placed_horizontally() {
        let m = FixedMeasurer;
        let mut container = make_flex_container(
            flex_style(),
            vec![make_item(100.0, 30.0), make_item(100.0, 30.0)],
        );
        layout_flex(&mut container, 480.0, &m);

        let x0 = container.children[0].dimensions.content.x;
        let x1 = container.children[1].dimensions.content.x;
        assert!(x0 < x1, "items should be side by side: x0={x0}, x1={x1}");
        assert!(
            (x1 - 100.0).abs() < 1.0,
            "second item should start at ~100: got {x1}",
        );
    }

    #[test]
    fn flex_row_items_same_y() {
        let m = FixedMeasurer;
        let mut container = make_flex_container(
            flex_style(),
            vec![make_item(100.0, 30.0), make_item(100.0, 50.0)],
        );
        layout_flex(&mut container, 480.0, &m);

        let y0 = container.children[0].dimensions.content.y;
        let y1 = container.children[1].dimensions.content.y;
        // Both should start at y=0 (flex-start alignment).
        assert!((y0).abs() < 1.0, "first item y should be ~0: got {y0}",);
        assert!((y1).abs() < 1.0, "second item y should be ~0: got {y1}",);
    }

    // -- Column layout ----------------------------------------------------

    #[test]
    fn flex_column_items_stacked_vertically() {
        let m = FixedMeasurer;
        let mut style = flex_style();
        style.flex_direction = FlexDirection::Column;
        let mut container =
            make_flex_container(style, vec![make_item(100.0, 30.0), make_item(100.0, 40.0)]);
        layout_flex(&mut container, 480.0, &m);

        let y0 = container.children[0].dimensions.content.y;
        let y1 = container.children[1].dimensions.content.y;
        assert!(
            y1 > y0,
            "second item should be below first: y0={y0}, y1={y1}",
        );
    }

    // -- Justify content --------------------------------------------------

    #[test]
    fn flex_justify_center() {
        let m = FixedMeasurer;
        let mut style = flex_style();
        style.justify_content = JustifyContent::Center;
        let mut container =
            make_flex_container(style, vec![make_item(100.0, 30.0), make_item(100.0, 30.0)]);
        layout_flex(&mut container, 480.0, &m);

        // Total items = 200px, free space = 280px, offset = 140px.
        let x0 = container.children[0].dimensions.content.x;
        assert!(
            (x0 - 140.0).abs() < 2.0,
            "centered first item should be at ~140: got {x0}",
        );
    }

    #[test]
    fn flex_justify_space_between() {
        let m = FixedMeasurer;
        let mut style = flex_style();
        style.justify_content = JustifyContent::SpaceBetween;
        let mut container = make_flex_container(
            style,
            vec![
                make_item(100.0, 30.0),
                make_item(100.0, 30.0),
                make_item(100.0, 30.0),
            ],
        );
        layout_flex(&mut container, 480.0, &m);

        let x0 = container.children[0].dimensions.content.x;
        let x2 = container.children[2].dimensions.content.x;
        // First item at x=0, last item ends at x=480.
        assert!(x0.abs() < 2.0, "first item should be at ~0: got {x0}",);
        assert!(
            (x2 + 100.0 - 480.0).abs() < 2.0,
            "last item should end at ~480: got {}",
            x2 + 100.0,
        );
    }

    // -- Flex grow --------------------------------------------------------

    #[test]
    fn flex_grow_distributes_space() {
        let m = FixedMeasurer;
        let style = flex_style();
        let mut item1 = make_item(100.0, 30.0);
        item1.style.flex_grow = 1.0;
        let mut item2 = make_item(100.0, 30.0);
        item2.style.flex_grow = 1.0;
        let mut container = make_flex_container(style, vec![item1, item2]);
        layout_flex(&mut container, 480.0, &m);

        // Each item should grow by 140px (280 free / 2).
        let w0 = container.children[0].dimensions.content.width;
        let w1 = container.children[1].dimensions.content.width;
        assert!(
            (w0 - 240.0).abs() < 2.0,
            "item 1 should be ~240px wide: got {w0}",
        );
        assert!(
            (w1 - 240.0).abs() < 2.0,
            "item 2 should be ~240px wide: got {w1}",
        );
    }

    #[test]
    fn flex_grow_weighted() {
        let m = FixedMeasurer;
        let style = flex_style();
        let mut item1 = make_item(100.0, 30.0);
        item1.style.flex_grow = 1.0;
        let mut item2 = make_item(100.0, 30.0);
        item2.style.flex_grow = 3.0;
        let mut container = make_flex_container(style, vec![item1, item2]);
        layout_flex(&mut container, 480.0, &m);

        // Free space = 280. item1 gets 70, item2 gets 210.
        let w0 = container.children[0].dimensions.content.width;
        let w1 = container.children[1].dimensions.content.width;
        assert!(
            (w0 - 170.0).abs() < 2.0,
            "item 1 should be ~170px: got {w0}",
        );
        assert!(
            (w1 - 310.0).abs() < 2.0,
            "item 2 should be ~310px: got {w1}",
        );
    }

    // -- Gap --------------------------------------------------------------

    #[test]
    fn flex_gap_adds_spacing() {
        let m = FixedMeasurer;
        let mut style = flex_style();
        style.gap = 10.0;
        let mut container =
            make_flex_container(style, vec![make_item(100.0, 30.0), make_item(100.0, 30.0)]);
        layout_flex(&mut container, 480.0, &m);

        let x0 = container.children[0].dimensions.content.x;
        let x1 = container.children[1].dimensions.content.x;
        let spacing = x1 - (x0 + 100.0);
        assert!(
            (spacing - 10.0).abs() < 2.0,
            "gap between items should be ~10px: got {spacing}",
        );
    }

    // -- Align items ------------------------------------------------------

    #[test]
    fn flex_align_center() {
        let m = FixedMeasurer;
        let mut style = flex_style();
        style.align_items = AlignItems::Center;
        let mut container =
            make_flex_container(style, vec![make_item(100.0, 20.0), make_item(100.0, 40.0)]);
        layout_flex(&mut container, 480.0, &m);

        // Line cross = 40. First item (20px) should be centered at y=10.
        let y0 = container.children[0].dimensions.content.y;
        assert!(
            (y0 - 10.0).abs() < 2.0,
            "shorter item should be centered: y={y0}",
        );
    }

    #[test]
    fn flex_align_flex_end() {
        let m = FixedMeasurer;
        let mut style = flex_style();
        style.align_items = AlignItems::FlexEnd;
        let mut container =
            make_flex_container(style, vec![make_item(100.0, 20.0), make_item(100.0, 40.0)]);
        layout_flex(&mut container, 480.0, &m);

        // Line cross = 40. First item (20px) should be at y=20.
        let y0 = container.children[0].dimensions.content.y;
        assert!(
            (y0 - 20.0).abs() < 2.0,
            "shorter item should be at flex-end: y={y0}",
        );
    }

    // -- Empty container --------------------------------------------------

    #[test]
    fn flex_empty_container() {
        let m = FixedMeasurer;
        let mut container = make_flex_container(flex_style(), vec![]);
        layout_flex(&mut container, 480.0, &m);
        assert!(
            container.dimensions.content.height.abs() < 1.0,
            "empty flex container height should be ~0",
        );
    }

    // -- Helper unit tests ------------------------------------------------

    #[test]
    fn compute_justification_flex_start() {
        let (start, spacing) = compute_justification(JustifyContent::FlexStart, 100.0, 3, false);
        assert!((start).abs() < f32::EPSILON);
        assert!((spacing).abs() < f32::EPSILON);
    }

    #[test]
    fn compute_justification_space_evenly() {
        let (start, spacing) = compute_justification(JustifyContent::SpaceEvenly, 100.0, 3, false);
        // 100 / (3+1) = 25
        assert!((start - 25.0).abs() < 0.01);
        assert!((spacing - 25.0).abs() < 0.01);
    }

    #[test]
    fn compute_cross_alignment_center() {
        let pos = compute_cross_alignment(AlignItems::Center, 0.0, 40.0, 20.0);
        assert!((pos - 10.0).abs() < f32::EPSILON);
    }
}
