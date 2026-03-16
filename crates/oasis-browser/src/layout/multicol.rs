//! Multi-column layout algorithm.
//!
//! Implements CSS Multi-column Layout (Level 1) for the OASIS browser
//! engine. When `column-count` or `column-width` is set on a block
//! container, children are distributed across balanced columns.

use super::block::{TextMeasurer, layout_block, resolve_edge_sizes};
use super::box_model::*;
use crate::css::values::Dimension;

/// Lay out children in a multi-column context.
///
/// `column_count` and `column_width` from the computed style determine
/// the number and width of columns. Children are distributed across
/// columns to balance total height.
pub fn layout_multicol(
    container: &mut LayoutBox,
    _containing_width: f32,
    measurer: &dyn TextMeasurer,
) {
    let content_width = container.dimensions.content.width;
    let content_x = container.dimensions.content.x;
    let content_y = container.dimensions.content.y;

    let (num_cols, col_width) = resolve_column_count(
        container.style.column_count,
        container.style.column_width,
        content_width,
        container.style.column_gap,
    );

    if num_cols <= 1 || container.children.is_empty() {
        // Fall back to normal block layout.
        return;
    }

    let col_gap = container.style.column_gap;

    // Phase 1: Layout all children at column width to get heights.
    let mut child_heights: Vec<f32> = Vec::with_capacity(container.children.len());
    for child in &mut container.children {
        resolve_edge_sizes(child, col_width);
        child.dimensions.content.x = 0.0;
        child.dimensions.content.y = 0.0;
        layout_block(child, col_width, measurer);
        child_heights.push(child.dimensions.margin_box().height);
    }

    // Phase 2: Distribute children across columns (balanced).
    let total_height: f32 = child_heights.iter().sum();
    let target_col_height = total_height / num_cols as f32;

    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); num_cols];
    let mut col_heights: Vec<f32> = vec![0.0; num_cols];
    let mut current_col = 0;

    for (i, &h) in child_heights.iter().enumerate() {
        // Move to next column if current exceeds target (unless first item in column).
        if current_col < num_cols - 1
            && !columns[current_col].is_empty()
            && col_heights[current_col] + h > target_col_height
        {
            current_col += 1;
        }
        columns[current_col].push(i);
        col_heights[current_col] += h;
    }

    // Phase 3: Position children in their columns.
    for (col_idx, col_children) in columns.iter().enumerate() {
        let col_x = content_x + (col_width + col_gap) * col_idx as f32;
        let mut cursor_y = content_y;

        for &child_idx in col_children {
            let child = &mut container.children[child_idx];
            child.dimensions.content.x = col_x
                + child.dimensions.margin.left
                + child.dimensions.border.left
                + child.dimensions.padding.left;
            child.dimensions.content.y = cursor_y
                + child.dimensions.margin.top
                + child.dimensions.border.top
                + child.dimensions.padding.top;

            // Re-layout at column width.
            layout_block(child, col_width, measurer);

            let bb = child.dimensions.border_box();
            cursor_y = bb.y + bb.height + child.dimensions.margin.bottom;
        }
    }

    // Phase 4: Set container height to max column height.
    if matches!(container.style.height, Dimension::Auto) {
        let max_h = col_heights.iter().copied().fold(0.0f32, f32::max);
        container.dimensions.content.height = max_h;
    }
}

/// Resolve the number of columns and column width from CSS properties.
pub fn resolve_column_count(
    column_count: u32,
    column_width: f32,
    available_width: f32,
    gap: f32,
) -> (usize, f32) {
    if column_count > 0 && column_width > 0.0 {
        // Both specified: use count but respect minimum width.
        let count = column_count as usize;
        let total_gaps = if count > 1 {
            gap * (count as f32 - 1.0)
        } else {
            0.0
        };
        let w = ((available_width - total_gaps) / count as f32).max(column_width);
        (count, w)
    } else if column_count > 0 {
        let count = column_count as usize;
        let total_gaps = if count > 1 {
            gap * (count as f32 - 1.0)
        } else {
            0.0
        };
        let w = (available_width - total_gaps) / count as f32;
        (count, w.max(0.0))
    } else if column_width > 0.0 {
        let count = ((available_width + gap) / (column_width + gap))
            .floor()
            .max(1.0) as usize;
        let total_gaps = if count > 1 {
            gap * (count as f32 - 1.0)
        } else {
            0.0
        };
        let w = (available_width - total_gaps) / count as f32;
        (count, w.max(0.0))
    } else {
        (1, available_width)
    }
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_column_count_by_count() {
        let (count, width) = resolve_column_count(3, 0.0, 480.0, 10.0);
        assert_eq!(count, 3);
        // (480 - 20) / 3 = 153.33
        assert!((width - 153.33).abs() < 1.0);
    }

    #[test]
    fn resolve_column_count_by_width() {
        let (count, width) = resolve_column_count(0, 200.0, 480.0, 10.0);
        // floor((480 + 10) / (200 + 10)) = floor(2.33) = 2
        assert_eq!(count, 2);
        // (480 - 10) / 2 = 235
        assert!((width - 235.0).abs() < 1.0);
    }

    #[test]
    fn resolve_column_count_both_specified() {
        let (count, width) = resolve_column_count(3, 100.0, 480.0, 10.0);
        assert_eq!(count, 3);
        // (480 - 20) / 3 = 153.33, which is > 100.0 minimum
        assert!(width >= 100.0);
    }

    #[test]
    fn resolve_column_count_neither_specified() {
        let (count, width) = resolve_column_count(0, 0.0, 480.0, 10.0);
        assert_eq!(count, 1);
        assert!((width - 480.0).abs() < f32::EPSILON);
    }
}
