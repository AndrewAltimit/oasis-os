//! CSS Grid layout algorithm.
//!
//! Implements a basic CSS Grid Layout for the OASIS browser engine.
//! Supports `grid-template-columns`, `grid-template-rows`, `gap`,
//! `fr` units, `px` sizes, `auto` sizing, and auto-placement.

use super::block::{TextMeasurer, layout_block, resolve_edge_sizes};
use super::box_model::*;
use crate::css::values::{Dimension, GridTrackSize};

/// Lay out a grid container and all its children.
///
/// The grid container's `content.x`, `content.y`, and `content.width`
/// must already be set by the caller. This function resolves track
/// sizes, places children into grid cells, and positions them.
pub fn layout_grid(container: &mut LayoutBox, _containing_width: f32, measurer: &dyn TextMeasurer) {
    let content_width = container.dimensions.content.width;
    let content_x = container.dimensions.content.x;
    let content_y = container.dimensions.content.y;
    let pad_top = container.dimensions.padding.top;
    let pad_left = container.dimensions.padding.left;
    let col_gap = container.style.column_gap;
    let row_gap = container.style.row_gap;
    let col_templates = container.style.grid_template_columns.clone();
    let row_templates = container.style.grid_template_rows.clone();

    let num_children = container.children.len();
    if num_children == 0 {
        if matches!(container.style.height, Dimension::Auto) {
            container.dimensions.content.height = container.dimensions.padding.vertical();
        }
        return;
    }

    // -- Phase 1: Resolve each child's base size -------------------------
    for child in &mut container.children {
        resolve_edge_sizes(child, content_width);
    }

    // -- Phase 2: Determine grid dimensions (columns x rows) -------------
    let num_cols = if col_templates.is_empty() {
        // If no columns are specified, use a single column.
        1
    } else {
        col_templates.len()
    };

    let num_rows_explicit = row_templates.len();
    let num_rows_needed = num_children.div_ceil(num_cols);
    let num_rows = num_rows_needed.max(num_rows_explicit);

    // -- Phase 3: Layout each child to determine intrinsic sizes ----------
    let mut child_widths: Vec<f32> = Vec::with_capacity(num_children);
    let mut child_heights: Vec<f32> = Vec::with_capacity(num_children);

    for child in &mut container.children {
        // Tentatively lay out at a fraction of the container width to
        // get intrinsic sizes.
        let temp_width = content_width / num_cols as f32;
        child.dimensions.content.x = 0.0;
        child.dimensions.content.y = 0.0;
        layout_block(child, temp_width, measurer);
        child_widths.push(child.dimensions.margin_box().width);
        child_heights.push(child.dimensions.margin_box().height);
    }

    // -- Phase 4: Auto-placement (assign grid cells) ---------------------
    // Each placement is (col, row, col_span, row_span).
    let mut placements: Vec<(usize, usize, usize, usize)> = Vec::with_capacity(num_children);
    // Track occupied cells for auto-placement around spans.
    let mut occupied = vec![vec![false; num_cols]; num_rows];

    for (i, child) in container.children.iter().enumerate() {
        let explicit_col = child
            .style
            .grid_column_start
            .map(|c| ((c - 1).max(0) as usize).min(num_cols - 1));
        let explicit_row = child.style.grid_row_start.map(|r| (r - 1).max(0) as usize);

        let (col, row) = match (explicit_col, explicit_row) {
            (Some(c), Some(r)) => (c, r),
            (Some(c), None) => (c, i / num_cols),
            (None, Some(r)) => (i % num_cols, r),
            (None, None) => {
                // Auto-place: find next unoccupied cell.
                let mut found = (i % num_cols, i / num_cols);
                'search: for (r, occ_row) in occupied.iter().enumerate() {
                    for (c, &is_occ) in occ_row.iter().enumerate().take(num_cols) {
                        if !is_occ {
                            found = (c, r);
                            break 'search;
                        }
                    }
                }
                found
            },
        };

        // Determine span from grid-column-end / grid-row-end.
        let col_span = if let (Some(start), Some(end)) =
            (child.style.grid_column_start, child.style.grid_column_end)
        {
            ((end - start).max(1) as usize).min(num_cols - col)
        } else if let Some(end) = child.style.grid_column_end {
            let start_line = (col + 1) as i32;
            ((end - start_line).max(1) as usize).min(num_cols - col)
        } else {
            1
        };

        let row_span = if let (Some(start), Some(end)) =
            (child.style.grid_row_start, child.style.grid_row_end)
        {
            (end - start).max(1) as usize
        } else if let Some(end) = child.style.grid_row_end {
            let start_line = (row + 1) as i32;
            (end - start_line).max(1) as usize
        } else {
            1
        };

        // Mark cells as occupied.
        for occ_row in occupied
            .iter_mut()
            .take(num_rows.min(row + row_span))
            .skip(row)
        {
            for cell in occ_row
                .iter_mut()
                .take(num_cols.min(col + col_span))
                .skip(col)
            {
                *cell = true;
            }
        }

        placements.push((col, row, col_span, row_span));
    }

    // -- Phase 5: Resolve column widths ----------------------------------
    let total_col_gaps = if num_cols > 1 {
        col_gap * (num_cols as f32 - 1.0)
    } else {
        0.0
    };
    let available_for_cols = (content_width - total_col_gaps).max(0.0);

    let col_widths = resolve_track_sizes(
        &col_templates,
        num_cols,
        available_for_cols,
        &placements,
        &child_widths,
        true,
    );

    // -- Phase 6: Resolve row heights ------------------------------------
    // For row heights, we need the max content height per row.
    let total_row_gaps = if num_rows > 1 {
        row_gap * (num_rows as f32 - 1.0)
    } else {
        0.0
    };

    // Determine the max intrinsic height in each row.
    let mut row_content_heights: Vec<f32> = vec![0.0; num_rows];
    for (i, &(_, row, _, _)) in placements.iter().enumerate() {
        if row < num_rows {
            row_content_heights[row] = row_content_heights[row].max(child_heights[i]);
        }
    }

    let row_heights = resolve_track_sizes(
        &row_templates,
        num_rows,
        0.0, // rows don't have a fixed container height by default
        &placements,
        &child_heights,
        false,
    );

    // For rows with auto sizing and no explicit template, use the
    // content heights we calculated.
    let row_heights: Vec<f32> = row_heights
        .iter()
        .enumerate()
        .map(|(i, &h)| {
            if h <= 0.0 {
                row_content_heights.get(i).copied().unwrap_or(0.0)
            } else {
                h
            }
        })
        .collect();

    // -- Phase 7: Position children in their grid cells -------------------
    // Calculate cumulative offsets.
    let col_offsets = cumulative_offsets(&col_widths, col_gap);
    let row_offsets = cumulative_offsets(&row_heights, row_gap);

    for (i, child) in container.children.iter_mut().enumerate() {
        let (col, row, col_span, row_span) = placements[i];
        let cell_x = col_offsets.get(col).copied().unwrap_or(0.0);
        let cell_y = row_offsets.get(row).copied().unwrap_or(0.0);

        // Calculate the total width across spanned columns including gaps.
        let cell_w = {
            let mut w = 0.0f32;
            for c in col..(col + col_span).min(num_cols) {
                w += col_widths.get(c).copied().unwrap_or(0.0);
            }
            // Add gaps between spanned columns.
            if col_span > 1 {
                w += col_gap * (col_span as f32 - 1.0);
            }
            w
        };

        // Calculate the total height across spanned rows including gaps.
        let _cell_h = {
            let mut h = 0.0f32;
            for r in row..(row + row_span).min(num_rows) {
                h += row_heights.get(r).copied().unwrap_or(0.0);
            }
            if row_span > 1 {
                h += row_gap * (row_span as f32 - 1.0);
            }
            h
        };

        // Grid items stretch to fill their cell by default. Override
        // the child's width to auto so that `calculate_block_width`
        // expands it to fill the cell (containing width).
        child.style.width = Dimension::Auto;
        layout_block(child, cell_w, measurer);

        // Position the child within the grid cell. We must do this
        // after layout_block so that edge sizes are resolved.
        child.dimensions.content.x = content_x
            + pad_left
            + cell_x
            + child.dimensions.margin.left
            + child.dimensions.border.left
            + child.dimensions.padding.left;
        child.dimensions.content.y = content_y
            + pad_top
            + cell_y
            + child.dimensions.margin.top
            + child.dimensions.border.top
            + child.dimensions.padding.top;
    }

    // -- Phase 8: Container height = sum of row heights + gaps -----------
    if matches!(container.style.height, Dimension::Auto) {
        let total_height: f32 = row_heights.iter().sum::<f32>() + total_row_gaps;
        container.dimensions.content.height =
            total_height + container.dimensions.padding.vertical();
    }
}

// -------------------------------------------------------------------
// Internal helpers
// -------------------------------------------------------------------

/// Resolve track sizes for a given axis.
///
/// Fixed (`Px`) tracks get their requested size. `Auto` tracks get the
/// maximum content size for items in that track. Remaining space is
/// distributed proportionally among `Fr` tracks.
fn resolve_track_sizes(
    templates: &[GridTrackSize],
    num_tracks: usize,
    available_space: f32,
    placements: &[(usize, usize, usize, usize)],
    child_sizes: &[f32],
    is_column: bool,
) -> Vec<f32> {
    let mut sizes: Vec<f32> = vec![0.0; num_tracks];
    let mut fr_total: f32 = 0.0;
    let mut fixed_total: f32 = 0.0;

    // First pass: assign fixed and auto sizes, accumulate fr totals.
    for (i, size) in sizes.iter_mut().enumerate() {
        let track = templates.get(i).copied().unwrap_or(GridTrackSize::Auto);
        match track {
            GridTrackSize::Px(px) => {
                *size = px;
                fixed_total += px;
            },
            GridTrackSize::Fr(fr) => {
                fr_total += fr;
                // Will be resolved in second pass.
            },
            GridTrackSize::Auto => {
                // Use the maximum content size for non-spanning items in
                // this track.
                let max_content = max_content_for_track(placements, child_sizes, i, is_column);
                *size = max_content;
                fixed_total += max_content;
            },
            GridTrackSize::Minmax(min, max) => {
                // Use the maximum content size clamped to [min, max].
                let max_content = max_content_for_track(placements, child_sizes, i, is_column);
                let clamped = max_content.clamp(min, max);
                *size = clamped;
                fixed_total += clamped;
            },
        }
    }

    // Second pass: distribute remaining space to fr tracks, then expand
    // Minmax tracks towards their max.
    if fr_total > 0.0 {
        let remaining = (available_space - fixed_total).max(0.0);
        for (i, size) in sizes.iter_mut().enumerate() {
            let track = templates.get(i).copied().unwrap_or(GridTrackSize::Auto);
            if let GridTrackSize::Fr(fr) = track {
                *size = (fr / fr_total) * remaining;
            }
        }
    } else if available_space > 0.0 && fixed_total < available_space {
        // Expand Minmax tracks towards their max with leftover space.
        let mut leftover = available_space - fixed_total;
        let mut expanded_any = false;
        for (i, size) in sizes.iter_mut().enumerate() {
            let track = templates.get(i).copied().unwrap_or(GridTrackSize::Auto);
            if let GridTrackSize::Minmax(_, max) = track
                && max > *size
                && leftover > 0.0
            {
                let grow = (max - *size).min(leftover);
                *size += grow;
                leftover -= grow;
                expanded_any = true;
            }
        }

        // If no Minmax tracks were expanded, distribute evenly among
        // auto tracks with zero size.
        if !expanded_any {
            let auto_count = sizes.iter().filter(|&&s| s == 0.0).count();
            if auto_count > 0 {
                let per_auto = (available_space - fixed_total) / auto_count as f32;
                for size in &mut sizes {
                    if *size == 0.0 {
                        *size = per_auto;
                    }
                }
            }
        }
    }

    sizes
}

/// Get the maximum content size for items placed in a given track.
/// Only considers non-spanning items (span == 1) for accurate sizing.
fn max_content_for_track(
    placements: &[(usize, usize, usize, usize)],
    child_sizes: &[f32],
    track_idx: usize,
    is_column: bool,
) -> f32 {
    placements
        .iter()
        .enumerate()
        .filter(|&(_, &(col, row, col_span, row_span))| {
            if is_column {
                col == track_idx && col_span == 1
            } else {
                row == track_idx && row_span == 1
            }
        })
        .map(|(idx, _)| child_sizes.get(idx).copied().unwrap_or(0.0))
        .fold(0.0f32, f32::max)
}

/// Calculate cumulative offsets from track sizes and gap.
fn cumulative_offsets(sizes: &[f32], gap: f32) -> Vec<f32> {
    let mut offsets = Vec::with_capacity(sizes.len());
    let mut offset: f32 = 0.0;
    for (i, &size) in sizes.iter().enumerate() {
        offsets.push(offset);
        offset += size;
        if i < sizes.len() - 1 {
            offset += gap;
        }
    }
    offsets
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::{ComputedStyle, Display, GridTrackSize};
    use crate::layout::block::TextMeasurer;

    struct FixedMeasurer;

    impl TextMeasurer for FixedMeasurer {
        fn measure_text(&self, text: &str, font_size: u16) -> u32 {
            oasis_types::backend::bitmap_measure_text(text, font_size)
        }
    }

    fn grid_style() -> ComputedStyle {
        let mut s = ComputedStyle::default();
        s.display = Display::Grid;
        s
    }

    fn item_style(width: f32, height: f32) -> ComputedStyle {
        let mut s = ComputedStyle::default();
        s.display = Display::Block;
        s.width = Dimension::Px(width);
        s.height = Dimension::Px(height);
        s
    }

    fn make_grid_container(style: ComputedStyle, children: Vec<LayoutBox>) -> LayoutBox {
        let mut lb = LayoutBox::new(BoxType::Grid, style, None);
        lb.children = children;
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;
        lb.dimensions.content.width = 480.0;
        lb
    }

    fn make_item(width: f32, height: f32) -> LayoutBox {
        LayoutBox::new(BoxType::Block, item_style(width, height), None)
    }

    // -- Basic 2-column grid ---------------------------------------------

    #[test]
    fn grid_two_columns_px() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Px(200.0), GridTrackSize::Px(200.0)];
        let mut container = make_grid_container(
            style,
            vec![
                make_item(100.0, 30.0),
                make_item(100.0, 30.0),
                make_item(100.0, 40.0),
                make_item(100.0, 40.0),
            ],
        );
        layout_grid(&mut container, 480.0, &m);

        // First row: items at col 0 and col 1.
        let x0 = container.children[0].dimensions.content.x;
        let x1 = container.children[1].dimensions.content.x;
        assert!(
            x0 < x1,
            "col 0 item should be left of col 1: x0={x0}, x1={x1}",
        );

        // Second row: items below first row.
        let y0 = container.children[0].dimensions.content.y;
        let y2 = container.children[2].dimensions.content.y;
        assert!(y2 > y0, "row 1 should be below row 0: y0={y0}, y2={y2}",);
    }

    // -- Fr units --------------------------------------------------------

    #[test]
    fn grid_fr_units_distribute_space() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Fr(1.0), GridTrackSize::Fr(1.0)];
        let mut container =
            make_grid_container(style, vec![make_item(50.0, 30.0), make_item(50.0, 30.0)]);
        layout_grid(&mut container, 480.0, &m);

        // Each column should be 240px (480 / 2).
        let w0 = container.children[0].dimensions.content.width;
        let w1 = container.children[1].dimensions.content.width;
        assert!((w0 - 240.0).abs() < 2.0, "col 0 should be ~240px: got {w0}",);
        assert!((w1 - 240.0).abs() < 2.0, "col 1 should be ~240px: got {w1}",);
    }

    #[test]
    fn grid_mixed_px_and_fr() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Px(100.0), GridTrackSize::Fr(1.0)];
        let mut container =
            make_grid_container(style, vec![make_item(50.0, 30.0), make_item(50.0, 30.0)]);
        layout_grid(&mut container, 480.0, &m);

        // Col 0 = 100px fixed, Col 1 = 380px (remaining).
        let w0 = container.children[0].dimensions.content.width;
        let w1 = container.children[1].dimensions.content.width;
        assert!((w0 - 100.0).abs() < 2.0, "col 0 should be ~100px: got {w0}",);
        assert!((w1 - 380.0).abs() < 2.0, "col 1 should be ~380px: got {w1}",);
    }

    // -- Gap -------------------------------------------------------------

    #[test]
    fn grid_gap_adds_spacing() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Fr(1.0), GridTrackSize::Fr(1.0)];
        style.column_gap = 10.0;
        style.row_gap = 10.0;
        let mut container =
            make_grid_container(style, vec![make_item(50.0, 30.0), make_item(50.0, 30.0)]);
        layout_grid(&mut container, 480.0, &m);

        // Available = 480 - 10 (gap) = 470. Each col = 235.
        let w0 = container.children[0].dimensions.content.width;
        assert!(
            (w0 - 235.0).abs() < 2.0,
            "col width with gap should be ~235px: got {w0}",
        );

        // Second item should be offset by col_width + gap.
        let x0 = container.children[0].dimensions.content.x;
        let x1 = container.children[1].dimensions.content.x;
        let spacing = x1 - x0;
        assert!(
            (spacing - 245.0).abs() < 2.0,
            "spacing between items should be ~245px: got {spacing}",
        );
    }

    // -- Row gap ---------------------------------------------------------

    #[test]
    fn grid_row_gap() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Fr(1.0)];
        style.column_gap = 10.0;
        style.row_gap = 10.0;
        let mut container =
            make_grid_container(style, vec![make_item(100.0, 30.0), make_item(100.0, 30.0)]);
        layout_grid(&mut container, 480.0, &m);

        let y0 = container.children[0].dimensions.content.y;
        let y1 = container.children[1].dimensions.content.y;
        let row_spacing = y1 - y0;
        // Should be item_height + gap = 30 + 10 = 40.
        assert!(
            (row_spacing - 40.0).abs() < 2.0,
            "row spacing should be ~40px: got {row_spacing}",
        );
    }

    // -- Empty container -------------------------------------------------

    #[test]
    fn grid_empty_container() {
        let m = FixedMeasurer;
        let mut container = make_grid_container(grid_style(), vec![]);
        layout_grid(&mut container, 480.0, &m);
        assert!(
            container.dimensions.content.height.abs() < 1.0,
            "empty grid height should be ~0",
        );
    }

    // -- Auto-placement with more items than columns --------------------

    #[test]
    fn grid_auto_placement_wraps_rows() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![
            GridTrackSize::Fr(1.0),
            GridTrackSize::Fr(1.0),
            GridTrackSize::Fr(1.0),
        ];
        let mut container = make_grid_container(
            style,
            vec![
                make_item(50.0, 30.0),
                make_item(50.0, 30.0),
                make_item(50.0, 30.0),
                make_item(50.0, 30.0), // wraps to row 1
            ],
        );
        layout_grid(&mut container, 480.0, &m);

        // Item 3 (index 3) should be in row 1, col 0.
        let y0 = container.children[0].dimensions.content.y;
        let y3 = container.children[3].dimensions.content.y;
        assert!(y3 > y0, "item 3 should wrap to next row: y0={y0}, y3={y3}",);

        let x3 = container.children[3].dimensions.content.x;
        let x0 = container.children[0].dimensions.content.x;
        assert!(
            (x3 - x0).abs() < 2.0,
            "item 3 should be in col 0: x0={x0}, x3={x3}",
        );
    }

    // -- Container height calculation ------------------------------------

    #[test]
    fn grid_container_height_matches_rows() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Fr(1.0), GridTrackSize::Fr(1.0)];
        style.column_gap = 5.0;
        style.row_gap = 5.0;
        let mut container = make_grid_container(
            style,
            vec![
                make_item(50.0, 30.0),
                make_item(50.0, 30.0),
                make_item(50.0, 40.0),
                make_item(50.0, 40.0),
            ],
        );
        layout_grid(&mut container, 480.0, &m);

        // 2 rows: 30 + 5 (gap) + 40 = 75, plus padding (0).
        let h = container.dimensions.content.height;
        assert!(
            (h - 75.0).abs() < 2.0,
            "container height should be ~75px: got {h}",
        );
    }

    // -- Helper tests ----------------------------------------------------

    #[test]
    fn cumulative_offsets_basic() {
        let sizes = [100.0, 200.0, 150.0];
        let offsets = cumulative_offsets(&sizes, 10.0);
        assert_eq!(offsets.len(), 3);
        assert!((offsets[0]).abs() < f32::EPSILON);
        assert!((offsets[1] - 110.0).abs() < f32::EPSILON);
        assert!((offsets[2] - 320.0).abs() < f32::EPSILON);
    }

    #[test]
    fn resolve_track_sizes_all_fr() {
        let templates = vec![
            GridTrackSize::Fr(1.0),
            GridTrackSize::Fr(2.0),
            GridTrackSize::Fr(1.0),
        ];
        let placements = vec![(0, 0, 1, 1), (1, 0, 1, 1), (2, 0, 1, 1)];
        let child_sizes = vec![50.0, 50.0, 50.0];
        let sizes = resolve_track_sizes(&templates, 3, 400.0, &placements, &child_sizes, true);
        assert!((sizes[0] - 100.0).abs() < 0.1);
        assert!((sizes[1] - 200.0).abs() < 0.1);
        assert!((sizes[2] - 100.0).abs() < 0.1);
    }

    #[test]
    fn resolve_track_sizes_mixed() {
        let templates = vec![GridTrackSize::Px(100.0), GridTrackSize::Fr(1.0)];
        let placements = vec![(0, 0, 1, 1), (1, 0, 1, 1)];
        let child_sizes = vec![50.0, 50.0];
        let sizes = resolve_track_sizes(&templates, 2, 400.0, &placements, &child_sizes, true);
        assert!((sizes[0] - 100.0).abs() < 0.1);
        assert!((sizes[1] - 300.0).abs() < 0.1);
    }

    // -- Parse grid template tests (via ComputedStyle) -------------------

    #[test]
    fn parse_grid_template_fr_units() {
        use crate::css::parser::CssValue;
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "grid-template-columns",
            &CssValue::Keyword("1fr 2fr 1fr".into()),
            8.0,
        );
        assert_eq!(s.grid_template_columns.len(), 3);
        assert_eq!(s.grid_template_columns[0], GridTrackSize::Fr(1.0));
        assert_eq!(s.grid_template_columns[1], GridTrackSize::Fr(2.0));
        assert_eq!(s.grid_template_columns[2], GridTrackSize::Fr(1.0));
    }

    #[test]
    fn parse_grid_template_repeat() {
        use crate::css::parser::CssValue;
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "grid-template-columns",
            &CssValue::Keyword("repeat(3, 1fr)".into()),
            8.0,
        );
        assert_eq!(s.grid_template_columns.len(), 3);
        for track in &s.grid_template_columns {
            assert_eq!(*track, GridTrackSize::Fr(1.0));
        }
    }

    #[test]
    fn parse_grid_template_mixed() {
        use crate::css::parser::CssValue;
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "grid-template-columns",
            &CssValue::Keyword("100px 1fr auto".into()),
            8.0,
        );
        assert_eq!(s.grid_template_columns.len(), 3);
        assert_eq!(s.grid_template_columns[0], GridTrackSize::Px(100.0),);
        assert_eq!(s.grid_template_columns[1], GridTrackSize::Fr(1.0));
        assert_eq!(s.grid_template_columns[2], GridTrackSize::Auto);
    }
}
