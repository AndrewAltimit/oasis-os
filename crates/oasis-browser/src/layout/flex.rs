//! CSS Flexbox layout algorithm.
//!
//! Implements a simplified CSS Flexible Box Layout (Level 1) for the
//! OASIS browser engine. Supports `flex-direction`, `justify-content`,
//! `align-items`, `flex-grow`, `flex-shrink`, `flex-basis`, `gap`,
//! and `flex-wrap`.
//!
//! ## Algorithm phases
//!
//! The implementation follows the spec's algorithm in four phases:
//!
//! 1. **Resolve base sizes** -- Each flex item's hypothetical main size
//!    is determined from `flex-basis` (or `width`/`height` fallback).
//!    Items are laid out at this basis to discover their intrinsic
//!    cross size.
//!
//! 2. **Split into flex lines** -- When `flex-wrap` is enabled, items
//!    are distributed across lines so that each line does not exceed
//!    the container's main size.
//!
//! 3. **Resolve flexible lengths** -- Free space (or deficit) on each
//!    line is distributed among items proportional to their
//!    `flex-grow` / `flex-shrink` factors.
//!
//! 4. **Position items** -- Items are placed along the main axis
//!    according to `justify-content` (and reversed if `*-reverse`),
//!    then aligned on the cross axis per `align-items`.

use super::block::{TextMeasurer, layout_block, offset_descendant, resolve_edge_sizes};
use super::box_model::*;
use crate::css::values::{
    AlignContent, AlignItems, AlignSelf, BoxSizing, Dimension, FlexDirection, FlexWrap,
    JustifyContent,
};

/// Lay out a flex container and all its children.
///
/// The flex container's `content.x`, `content.y`, and `content.width`
/// must already be set by the caller (`content.x/y` is the content-box
/// origin, inside the container's padding). This function resolves
/// flex item sizes, distributes free space, and positions children
/// according to the container's flex properties.
///
/// Each item is laid out at most twice: once to measure its base size
/// and, only when flexing changed its main size, once more at the
/// resolved size. Positioning then translates the laid-out subtree, so
/// nested flex containers cost a bounded number of layouts per level
/// (laying every item out again at its final position doubled the work
/// at each nesting level: 2^depth).
pub fn layout_flex(container: &mut LayoutBox, _containing_width: f32, measurer: &dyn TextMeasurer) {
    let direction = container.style.flex_direction;
    let justify = container.style.justify_content;
    let align = container.style.align_items;
    let align_content = container.style.align_content;
    let wrap = container.style.flex_wrap;
    let gap = container.style.gap;

    let content_width = container.dimensions.content.width;
    let content_x = container.dimensions.content.x;
    let content_y = container.dimensions.content.y;

    let is_row = matches!(direction, FlexDirection::Row | FlexDirection::RowReverse);
    let is_reverse = matches!(
        direction,
        FlexDirection::RowReverse | FlexDirection::ColumnReverse
    );
    let definite_height = definite_content_height(container);
    // The main size free space is measured against: the content width
    // for rows; for columns only an explicit height (otherwise the
    // container is as tall as its items and there is none to share).
    let definite_main = if is_row {
        Some(content_width)
    } else {
        definite_height
    };

    // -- Phase 1: Resolve each child's base (hypothetical) size ----------
    let mut items: Vec<FlexItem> = Vec::with_capacity(container.children.len());
    for (i, child) in container.children.iter_mut().enumerate() {
        resolve_edge_sizes(child, content_width);
        child.dimensions.content.x = 0.0;
        child.dimensions.content.y = 0.0;
        let main = match (is_row, resolve_flex_basis(child, is_row, content_width)) {
            (true, Some(basis)) => {
                layout_block(child, basis, measurer);
                child.dimensions.margin_box().width
            },
            (true, None) => {
                // Content-sized: fit-content, i.e. the item's extent when
                // laid out at the available width.
                layout_block(child, content_width, measurer);
                let fit = if child.children.is_empty()
                    && !matches!(child.box_type, BoxType::Replaced(_))
                {
                    0.0
                } else {
                    super::block::fit_content_width(child)
                };
                let d = &child.dimensions;
                fit + d.padding.horizontal() + d.border.horizontal() + d.margin.horizontal()
            },
            (false, _) => {
                layout_block(child, content_width, measurer);
                child.dimensions.margin_box().height
            },
        };
        items.push(FlexItem {
            index: i,
            order: child.style.order,
            grow: child.style.flex_grow,
            shrink: child.style.flex_shrink,
            main,
        });
    }

    // Sort by CSS `order` property (stable sort preserves DOM order for ties).
    items.sort_by_key(|item| item.order);

    // -- Phase 2: Split items into flex lines ----------------------------
    let lines = match definite_main {
        Some(main_size) => split_into_lines(&items, main_size, gap, wrap),
        None if items.is_empty() => Vec::new(),
        None => vec![items.iter().collect()],
    };

    // -- Phase 3: Resolve flexible lengths per line ----------------------
    // `(child index, resolved outer main size)` per item, plus the main
    // size each line is justified within.
    let mut resolved_lines: Vec<(f32, Vec<(usize, f32)>)> = Vec::with_capacity(lines.len());
    for line in &lines {
        let line_main = definite_main.unwrap_or_else(|| line_outer_main(line, gap));
        resolved_lines.push((line_main, resolve_flex_line(line, line_main, gap)));
    }

    // -- Phase 4a: Lay items out at their resolved main size -------------
    for (_, line) in &resolved_lines {
        for &(child_idx, item_main) in line {
            relayout_at_main_size(
                &mut container.children[child_idx],
                is_row,
                item_main,
                content_width,
                measurer,
            );
        }
    }

    // -- Phase 4b: Cross sizes from the final layouts --------------------
    let cross_of = |child: &LayoutBox| {
        let mb = child.dimensions.margin_box();
        if is_row { mb.height } else { mb.width }
    };
    // A single-line (`nowrap`) container with a definite cross size
    // gives its line that whole size, so `align-items` works against the
    // container (CSS Flexbox 9.4 step 15), not just the tallest item.
    let single_line_cross = match wrap {
        FlexWrap::NoWrap if is_row => definite_height,
        FlexWrap::NoWrap => Some(content_width),
        _ => None,
    };
    let line_cross_sizes: Vec<f32> = resolved_lines
        .iter()
        .map(|(_, line)| {
            let tallest = line
                .iter()
                .map(|&(ci, _)| cross_of(&container.children[ci]))
                .fold(0.0f32, f32::max);
            single_line_cross.unwrap_or(tallest)
        })
        .collect();
    let n_lines = resolved_lines.len();
    let total_line_cross: f32 = line_cross_sizes.iter().sum();
    let total_line_gaps = if n_lines > 1 {
        gap * (n_lines as f32 - 1.0)
    } else {
        0.0
    };
    let container_cross = if is_row {
        definite_height.unwrap_or(total_line_cross + total_line_gaps)
    } else {
        content_width
    };
    let free_cross = (container_cross - total_line_cross - total_line_gaps).max(0.0);
    let (mut cross_offset, cross_inter) = compute_align_content(align_content, free_cross, n_lines);

    // -- Phase 4c: Position items (translate the laid-out subtrees) ------
    let mut used_main = 0.0f32;
    for ((line_main, resolved_line), &line_cross) in resolved_lines.iter().zip(&line_cross_sizes) {
        let n = resolved_line.len();
        let total_gaps = if n > 1 { gap * (n as f32 - 1.0) } else { 0.0 };
        let total_main: f32 = resolved_line.iter().map(|&(_, m)| m).sum::<f32>() + total_gaps;
        used_main = used_main.max(total_main);
        let free_space = (line_main - total_main).max(0.0);

        let (mut main_offset, inter_gap) =
            compute_justification(justify, free_space, n, is_reverse);
        let order: Vec<usize> = if is_reverse {
            (0..n).rev().collect()
        } else {
            (0..n).collect()
        };

        // Auto margins on the main axis absorb free space before
        // justify-content (CSS Flexbox 8.1).
        let auto_flags = |child: &LayoutBox| {
            if is_row {
                (child.style.margin_left_auto, child.style.margin_right_auto)
            } else {
                (child.style.margin_top_auto, child.style.margin_bottom_auto)
            }
        };
        let total_auto: usize = resolved_line
            .iter()
            .map(|&(ci, _)| {
                let (s, e) = auto_flags(&container.children[ci]);
                usize::from(s) + usize::from(e)
            })
            .sum();
        let per_auto = if total_auto > 0 && free_space > 0.0 {
            main_offset = 0.0;
            free_space / total_auto as f32
        } else {
            0.0
        };

        for (seq, &idx) in order.iter().enumerate() {
            let (child_idx, item_main) = resolved_line[idx];
            let child = &mut container.children[child_idx];
            let (auto_start, auto_end) = auto_flags(child);
            main_offset += if auto_start { per_auto } else { 0.0 };

            let effective_align = resolve_align_self(child.style.align_self, align);
            let cross_pos =
                compute_cross_alignment(effective_align, cross_offset, line_cross, cross_of(child));

            let d = &child.dimensions;
            let lead_x = d.margin.left + d.border.left + d.padding.left;
            let lead_y = d.margin.top + d.border.top + d.padding.top;
            let (target_x, target_y) = if is_row {
                (
                    content_x + main_offset + lead_x,
                    content_y + cross_pos + lead_y,
                )
            } else {
                (
                    content_x + cross_pos + lead_x,
                    content_y + main_offset + lead_y,
                )
            };
            let (dx, dy) = (target_x - d.content.x, target_y - d.content.y);
            offset_descendant(child, dx, dy);

            main_offset += item_main + if auto_end { per_auto } else { 0.0 };
            if seq + 1 < n {
                main_offset += gap + inter_gap;
            }
        }

        cross_offset += line_cross + gap + cross_inter;
    }

    // -- Phase 5: Container height ---------------------------------------
    // Only an auto height is content-sized: rows stack their lines'
    // cross sizes, columns are as tall as their items. (The caller
    // resolves explicit heights.)
    if matches!(container.style.height, Dimension::Auto) {
        container.dimensions.content.height = if is_row {
            total_line_cross + total_line_gaps
        } else {
            used_main
        };
    }
}

// -------------------------------------------------------------------
// Internal helpers
// -------------------------------------------------------------------

/// Intermediate flex item data.
struct FlexItem {
    index: usize,
    order: i32,
    grow: f32,
    shrink: f32,
    /// Hypothetical outer (margin-box) main size.
    main: f32,
}

/// The container's content height when it is definite (an explicit
/// `px` height, resolved for `box-sizing`), else `None`.
fn definite_content_height(container: &LayoutBox) -> Option<f32> {
    let Dimension::Px(h) = container.style.height else {
        return None;
    };
    let d = &container.dimensions;
    Some(if container.style.box_sizing == BoxSizing::BorderBox {
        (h - d.padding.vertical() - d.border.vertical()).max(0.0)
    } else {
        h.max(0.0)
    })
}

/// Sum of a line's hypothetical outer main sizes plus its gaps.
fn line_outer_main(line: &[&FlexItem], gap: f32) -> f32 {
    let gaps = if line.len() > 1 {
        gap * (line.len() as f32 - 1.0)
    } else {
        0.0
    };
    line.iter().map(|item| item.main).sum::<f32>() + gaps
}

/// Lay `child` out again, at its origin, when its resolved outer main
/// size differs from the one phase 1 measured. The size goes in as an
/// explicit `width` / `height` (in the item's `box-sizing`).
fn relayout_at_main_size(
    child: &mut LayoutBox,
    is_row: bool,
    outer_main: f32,
    content_width: f32,
    measurer: &dyn TextMeasurer,
) {
    let d = &child.dimensions;
    let border_box = child.style.box_sizing == BoxSizing::BorderBox;
    if is_row {
        let edges = d.padding.horizontal() + d.border.horizontal();
        let new_w = (outer_main - d.margin.horizontal() - edges).max(0.0);
        if (new_w - d.content.width).abs() <= 0.01 {
            return;
        }
        child.style.width = Dimension::Px(if border_box { new_w + edges } else { new_w });
        child.dimensions.content.x = 0.0;
        child.dimensions.content.y = 0.0;
        layout_block(child, new_w, measurer);
    } else {
        let edges = d.padding.vertical() + d.border.vertical();
        let new_h = (outer_main - d.margin.vertical() - edges).max(0.0);
        if (new_h - d.content.height).abs() <= 0.01 {
            return;
        }
        child.style.height = Dimension::Px(if border_box { new_h + edges } else { new_h });
        child.dimensions.content.x = 0.0;
        child.dimensions.content.y = 0.0;
        layout_block(child, content_width, measurer);
    }
}

/// Resolve the flex basis for an item: `Some(px)` for a definite
/// `flex-basis` (or `width` / `height` fallback), `None` when the item
/// is content-sized.
fn resolve_flex_basis(child: &LayoutBox, is_row: bool, containing_width: f32) -> Option<f32> {
    let definite = |dim: Dimension| match dim {
        Dimension::Px(px) => Some(px),
        Dimension::Percent(pct) => Some(containing_width * (pct / 100.0)),
        _ => None,
    };
    definite(child.style.flex_basis).or_else(|| {
        definite(if is_row {
            child.style.width
        } else {
            child.style.height
        })
    })
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

    if matches!(wrap, FlexWrap::NoWrap) {
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
/// Returns `(child_index, resolved_outer_main_size)` for each item.
fn resolve_flex_line(line: &[&FlexItem], main_size: f32, gap: f32) -> Vec<(usize, f32)> {
    let n = line.len();
    let total_gaps = if n > 1 { gap * (n as f32 - 1.0) } else { 0.0 };
    let total_base: f32 = line.iter().map(|item| item.main).sum();
    let free_space = main_size - total_base - total_gaps;

    let mut result: Vec<(usize, f32)> = Vec::with_capacity(n);

    if free_space > 0.0 {
        // Positive free space: distribute via flex-grow.
        let total_grow: f32 = line.iter().map(|item| item.grow).sum();
        for item in line {
            let extra = if total_grow > 0.0 {
                (item.grow / total_grow) * free_space
            } else {
                0.0
            };
            result.push((item.index, item.main + extra));
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
            result.push((item.index, (item.main - reduction).max(0.0)));
        }
    } else {
        for item in line {
            result.push((item.index, item.main));
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
            (per / 2.0, per)
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

/// Compute cross-axis line distribution from `align-content`.
///
/// Returns `(initial_offset, inter_line_spacing)` similar to `compute_justification`.
fn compute_align_content(align_content: AlignContent, free_space: f32, count: usize) -> (f32, f32) {
    if count == 0 || free_space <= 0.0 {
        return (0.0, 0.0);
    }
    match align_content {
        AlignContent::FlexStart | AlignContent::Stretch => (0.0, 0.0),
        AlignContent::FlexEnd => (free_space, 0.0),
        AlignContent::Center => (free_space / 2.0, 0.0),
        AlignContent::SpaceBetween => {
            if count <= 1 {
                (0.0, 0.0)
            } else {
                (0.0, free_space / (count as f32 - 1.0))
            }
        },
        AlignContent::SpaceAround => {
            let per = free_space / count as f32;
            (per / 2.0, per)
        },
        AlignContent::SpaceEvenly => {
            let per = free_space / (count as f32 + 1.0);
            (per, per)
        },
    }
}

/// Resolve `align-self` for a flex item: if `Auto`, fall back to the
/// container's `align-items` value.
fn resolve_align_self(align_self: AlignSelf, container_align: AlignItems) -> AlignItems {
    match align_self {
        AlignSelf::Auto => container_align,
        AlignSelf::FlexStart => AlignItems::FlexStart,
        AlignSelf::FlexEnd => AlignItems::FlexEnd,
        AlignSelf::Center => AlignItems::Center,
        AlignSelf::Stretch => AlignItems::Stretch,
        AlignSelf::Baseline => AlignItems::Baseline,
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
        ComputedStyle {
            display: Display::Flex,
            ..Default::default()
        }
    }

    fn item_style(width: f32, height: f32) -> ComputedStyle {
        ComputedStyle {
            display: Display::Block,
            width: Dimension::Px(width),
            height: Dimension::Px(height),
            ..Default::default()
        }
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
