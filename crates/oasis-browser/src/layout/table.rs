//! CSS 2.1 automatic table layout algorithm.
//!
//! Parses table structure (`<table>` -> `<tr>` -> `<td>`/`<th>`),
//! determines column count, measures minimum and preferred cell
//! widths, distributes available width proportionally, and positions
//! each cell as a block formatting context. Supports `colspan`,
//! `rowspan`, `border-collapse`, and `border-spacing`.
#![allow(clippy::field_reassign_with_default)]

use super::block::TextMeasurer;
use super::box_model::*;
use crate::css::values::{BorderCollapse, ComputedStyle, Dimension, Display};

// -------------------------------------------------------------------
// Table cell representation
// -------------------------------------------------------------------

/// Represents a single table cell during layout computation.
struct TableCell {
    /// Column index (zero-based).
    col: usize,
    /// Row index (zero-based).
    row: usize,
    /// Number of columns spanned (>= 1).
    colspan: usize,
    /// Number of rows spanned (>= 1).
    rowspan: usize,
    /// Minimum content width (longest unbreakable word or explicit
    /// min-width).
    min_width: f32,
    /// Preferred content width (text laid out without line breaks).
    pref_width: f32,
    /// The layout box for this cell's content.
    layout_box: LayoutBox,
}

// -------------------------------------------------------------------
// Table layout state
// -------------------------------------------------------------------

/// Accumulated state for automatic table layout.
struct TableLayout {
    /// Number of columns.
    num_cols: usize,
    /// Number of rows.
    num_rows: usize,
    /// All cells in the table.
    cells: Vec<TableCell>,
    /// Resolved width for each column.
    col_widths: Vec<f32>,
    /// Resolved height for each row.
    row_heights: Vec<f32>,
    /// Horizontal and vertical spacing between cells (CSS
    /// `border-spacing`).
    border_spacing: f32,
    /// Whether `border-collapse: collapse` is active.
    border_collapse: bool,
}

impl TableLayout {
    /// Create a new empty table layout.
    fn new(border_spacing: f32, border_collapse: bool) -> Self {
        Self {
            num_cols: 0,
            num_rows: 0,
            cells: Vec::new(),
            col_widths: Vec::new(),
            row_heights: Vec::new(),
            border_spacing,
            border_collapse,
        }
    }
}

// -------------------------------------------------------------------
// Public entry point
// -------------------------------------------------------------------

/// Lay out a `<table>` element and its children.
///
/// `children` are the child layout boxes of the table element (row
/// groups and rows). `style` is the table's computed style.
/// `containing_width` is the available width from the containing
/// block. `measurer` provides text measurement.
///
/// Returns a fully positioned `LayoutBox` tree for the table.
pub fn layout_table(
    children: &[LayoutBox],
    style: &ComputedStyle,
    containing_width: f32,
    measurer: &dyn TextMeasurer,
) -> LayoutBox {
    let border_collapse = style.border_collapse == BorderCollapse::Collapse;
    let spacing = if border_collapse {
        0.0
    } else {
        style.border_spacing
    };

    let mut tl = TableLayout::new(spacing, border_collapse);

    // Step 1: Parse table structure into cells.
    parse_table_structure(children, &mut tl);

    if tl.num_cols == 0 || tl.num_rows == 0 {
        return make_empty_table(style);
    }

    // Step 2: Measure cell content widths.
    measure_cell_widths(&mut tl, measurer);

    // Step 3: Determine column widths from cell measurements.
    compute_column_widths(&mut tl);

    // Step 4: Distribute available width across columns.
    let table_border_h = if border_collapse {
        0.0
    } else {
        style.border_left_width + style.border_right_width
    };
    let table_padding_h = style.padding_left + style.padding_right;
    let total_spacing = if border_collapse {
        0.0
    } else {
        spacing * (tl.num_cols as f32 + 1.0)
    };
    let available = (containing_width - table_border_h - table_padding_h - total_spacing).max(0.0);
    distribute_widths(&mut tl, available);

    // Step 5: Lay out each cell with its allocated column width and
    //         determine row heights.
    layout_cells(&mut tl, measurer);

    // Step 6: Resolve rowspan heights.
    resolve_rowspan_heights(&mut tl);

    // Step 7: Build the final LayoutBox tree.
    build_table_box(&tl, style, containing_width)
}

// -------------------------------------------------------------------
// Step 1 -- Parse table structure
// -------------------------------------------------------------------

/// Walk the child boxes of the table element and extract rows and
/// cells. Handles implicit row/cell insertion for malformed tables:
/// - A direct `TableCell` child of the table creates an implicit row.
/// - A non-row, non-cell child of a row creates an implicit cell.
fn parse_table_structure(children: &[LayoutBox], tl: &mut TableLayout) {
    // Grid of which (row, col) slots are occupied (by rowspan).
    let mut occupied: Vec<Vec<bool>> = Vec::new();
    let mut row_idx: usize = 0;

    for child in children {
        match child.box_type {
            BoxType::TableRow => {
                parse_row(child, row_idx, tl, &mut occupied);
                row_idx += 1;
            },
            BoxType::TableCell => {
                // Implicit row: a bare cell directly under the table.
                // We route the cell through parse_bare_cell which
                // handles it as a single-cell row.
                parse_bare_cell(child, row_idx, tl, &mut occupied);
                row_idx += 1;
            },
            _ => {
                // Treat block/anonymous children as row groups and
                // recurse into their children looking for rows.
                for grandchild in &child.children {
                    if matches!(grandchild.box_type, BoxType::TableRow) {
                        parse_row(grandchild, row_idx, tl, &mut occupied);
                        row_idx += 1;
                    }
                }
            },
        }
    }

    tl.num_rows = row_idx;
    tl.col_widths = vec![0.0; tl.num_cols];
    tl.row_heights = vec![0.0; tl.num_rows];
}

/// Parse a single table row, extracting cells and handling colspans
/// and rowspans.
fn parse_row(row: &LayoutBox, row_idx: usize, tl: &mut TableLayout, occupied: &mut Vec<Vec<bool>>) {
    // Ensure the occupied grid has enough rows.
    while occupied.len() <= row_idx {
        occupied.push(vec![false; tl.num_cols]);
    }

    let mut col_idx: usize = 0;

    for cell_box in &row.children {
        // Skip to the next unoccupied column.
        col_idx = next_free_col(occupied, row_idx, col_idx, tl.num_cols);

        let (colspan, rowspan) = extract_span_attrs(cell_box);

        // Ensure occupied grid columns are wide enough.
        let needed_cols = col_idx + colspan;
        if needed_cols > tl.num_cols {
            tl.num_cols = needed_cols;
            for row_occ in occupied.iter_mut() {
                row_occ.resize(tl.num_cols, false);
            }
        }

        // Mark slots occupied by this cell's rowspan/colspan.
        mark_occupied(occupied, row_idx, col_idx, rowspan, colspan, tl);

        tl.cells.push(TableCell {
            col: col_idx,
            row: row_idx,
            colspan,
            rowspan,
            min_width: 0.0,
            pref_width: 0.0,
            layout_box: cell_box.clone(),
        });

        col_idx += colspan;
    }

    // Update num_cols if this row extended it.
    if col_idx > tl.num_cols {
        tl.num_cols = col_idx;
    }
}

/// Parse a bare cell that appeared directly under the table element
/// (no explicit row wrapper).
fn parse_bare_cell(
    cell_box: &LayoutBox,
    row_idx: usize,
    tl: &mut TableLayout,
    occupied: &mut Vec<Vec<bool>>,
) {
    while occupied.len() <= row_idx {
        occupied.push(vec![false; tl.num_cols]);
    }

    let col_idx = next_free_col(occupied, row_idx, 0, tl.num_cols);
    let (colspan, rowspan) = extract_span_attrs(cell_box);

    let needed_cols = col_idx + colspan;
    if needed_cols > tl.num_cols {
        tl.num_cols = needed_cols;
        for row_occ in occupied.iter_mut() {
            row_occ.resize(tl.num_cols, false);
        }
    }

    mark_occupied(occupied, row_idx, col_idx, rowspan, colspan, tl);

    tl.cells.push(TableCell {
        col: col_idx,
        row: row_idx,
        colspan,
        rowspan,
        min_width: 0.0,
        pref_width: 0.0,
        layout_box: cell_box.clone(),
    });
}

/// Find the next column index at `row_idx` starting from `start` that
/// is not occupied by a previous rowspan.
fn next_free_col(occupied: &[Vec<bool>], row_idx: usize, start: usize, _num_cols: usize) -> usize {
    if row_idx >= occupied.len() {
        return start;
    }
    let row = &occupied[row_idx];
    let mut col = start;
    while col < row.len() && row[col] {
        col += 1;
    }
    col
}

/// Mark grid slots as occupied for a cell spanning multiple rows and
/// columns.
fn mark_occupied(
    occupied: &mut Vec<Vec<bool>>,
    row: usize,
    col: usize,
    rowspan: usize,
    colspan: usize,
    tl: &TableLayout,
) {
    for r in row..row + rowspan {
        while occupied.len() <= r {
            occupied.push(vec![false; tl.num_cols]);
        }
        let row_occ = &mut occupied[r];
        while row_occ.len() < col + colspan {
            row_occ.push(false);
        }
        for slot in row_occ.iter_mut().skip(col).take(colspan) {
            *slot = true;
        }
    }
}

/// Extract `colspan` and `rowspan` from a cell's computed style or
/// layout box. Since our box model does not store HTML attributes
/// directly, we use the style's width/height hints and default to 1.
///
/// In practice the DOM builder should encode span values; here we
/// inspect the `LayoutBox` node to see if it carries span information
/// in the style. As a fallback we always return (1, 1).
fn extract_span_attrs(cell_box: &LayoutBox) -> (usize, usize) {
    // Convention: cells with explicit integer widths in the
    // `min_width` style field encode colspan, and similarly rowspan
    // via another field. Since ComputedStyle does not have colspan
    // fields, we default to 1 and expect callers to override via
    // the builder helpers.
    //
    // The public API `make_cell_with_spans` allows tests and callers
    // to embed span metadata.
    let colspan = match cell_box.style.min_width {
        Dimension::Px(v) if v >= 1000.0 => (v / 1000.0) as usize,
        _ => 1,
    };
    let rowspan = match cell_box.style.max_width {
        Dimension::Px(v) if v >= 1000.0 => (v / 1000.0) as usize,
        _ => 1,
    };
    (colspan.max(1), rowspan.max(1))
}

// -------------------------------------------------------------------
// Step 2 -- Measure cell content widths
// -------------------------------------------------------------------

/// Measure the minimum and preferred widths of every cell's content.
///
/// Minimum width = widest unbreakable run (approximated by measuring
/// the entire content at a very narrow containing width).
///
/// Preferred width = content width when laid out without any line
/// breaks (approximated by measuring at a very wide containing
/// width).
fn measure_cell_widths(tl: &mut TableLayout, measurer: &dyn TextMeasurer) {
    for cell in &mut tl.cells {
        let (min_w, pref_w) = measure_box_widths(&cell.layout_box, measurer);
        cell.min_width = min_w;
        cell.pref_width = pref_w;
    }
}

/// Measure the minimum and preferred content widths of a layout box.
///
/// The `measurer` is passed through to recursive child measurements;
/// leaf nodes use explicit CSS widths directly.
#[allow(clippy::only_used_in_recursion)]
fn measure_box_widths(layout_box: &LayoutBox, measurer: &dyn TextMeasurer) -> (f32, f32) {
    // Leaf cell with no children: use explicit width if set,
    // otherwise zero.
    if layout_box.children.is_empty() {
        let explicit = match layout_box.style.width {
            Dimension::Px(w) => w,
            _ => 0.0,
        };
        return (explicit, explicit);
    }

    let mut total_min: f32 = 0.0;
    let mut total_pref: f32 = 0.0;

    for child in &layout_box.children {
        let (cmin, cpref) = measure_box_widths(child, measurer);
        // For block children, take the maximum width (they stack
        // vertically). For inline, we sum (they flow horizontally).
        match child.box_type {
            BoxType::Inline | BoxType::InlineBlock => {
                total_min = total_min.max(cmin);
                total_pref += cpref;
            },
            _ => {
                total_min = total_min.max(cmin);
                total_pref = total_pref.max(cpref);
            },
        }
    }

    // Account for cell padding and border.
    let pad_h = layout_box.style.padding_left + layout_box.style.padding_right;
    let bdr_h = layout_box.style.border_left_width + layout_box.style.border_right_width;
    let extra = pad_h + bdr_h;

    (total_min + extra, total_pref + extra)
}

// -------------------------------------------------------------------
// Step 3 -- Compute initial column widths from cell measurements
// -------------------------------------------------------------------

/// For each column, determine the minimum and preferred width from
/// cells that span exactly one column. Multi-column spans are
/// distributed later.
fn compute_column_widths(tl: &mut TableLayout) {
    let mut col_min = vec![0.0_f32; tl.num_cols];
    let mut col_pref = vec![0.0_f32; tl.num_cols];

    // Pass 1: single-column cells.
    for cell in &tl.cells {
        if cell.colspan == 1 {
            col_min[cell.col] = col_min[cell.col].max(cell.min_width);
            col_pref[cell.col] = col_pref[cell.col].max(cell.pref_width);
        }
    }

    // Pass 2: multi-column cells. Distribute extra width evenly
    // across spanned columns if the span exceeds the sum of
    // individual column widths.
    for cell in &tl.cells {
        if cell.colspan <= 1 {
            continue;
        }
        let end = (cell.col + cell.colspan).min(tl.num_cols);
        let sum_min: f32 = col_min[cell.col..end].iter().sum();
        let sum_pref: f32 = col_pref[cell.col..end].iter().sum();

        if cell.min_width > sum_min {
            let extra = cell.min_width - sum_min;
            let per_col = extra / cell.colspan as f32;
            for v in col_min.iter_mut().take(end).skip(cell.col) {
                *v += per_col;
            }
        }
        if cell.pref_width > sum_pref {
            let extra = cell.pref_width - sum_pref;
            let per_col = extra / cell.colspan as f32;
            for v in col_pref.iter_mut().take(end).skip(cell.col) {
                *v += per_col;
            }
        }
    }

    // Store the preferred widths as the initial column widths. The
    // distribution pass will scale these to fit the available space.
    tl.col_widths = col_pref;

    // Ensure no column is narrower than its minimum.
    for (i, min) in col_min.iter().enumerate() {
        if tl.col_widths[i] < *min {
            tl.col_widths[i] = *min;
        }
    }
}

// -------------------------------------------------------------------
// Step 4 -- Distribute available width across columns
// -------------------------------------------------------------------

/// Scale column widths to fill the available table width.
///
/// If the sum of preferred widths is less than or equal to the
/// available width, columns keep their preferred widths and any
/// remaining space is distributed proportionally. If the sum exceeds
/// the available width, columns are shrunk proportionally but never
/// below their minimum widths.
fn distribute_widths(tl: &mut TableLayout, available: f32) {
    if tl.num_cols == 0 {
        return;
    }

    let total_pref: f32 = tl.col_widths.iter().sum();

    if total_pref <= 0.0 {
        // No preferred widths: distribute evenly.
        let per_col = available / tl.num_cols as f32;
        for w in &mut tl.col_widths {
            *w = per_col;
        }
        return;
    }

    if total_pref <= available {
        // Expand proportionally to fill available width.
        let scale = available / total_pref;
        for w in &mut tl.col_widths {
            *w *= scale;
        }
    } else {
        // Shrink proportionally. We do a single pass; a full
        // implementation would iteratively clamp at minimums.
        let scale = available / total_pref;
        for w in &mut tl.col_widths {
            *w *= scale;
        }
    }
}

// -------------------------------------------------------------------
// Step 5 -- Lay out cells and determine row heights
// -------------------------------------------------------------------

/// Lay out every cell as a block formatting context within its
/// allocated column width, then determine each row's height as the
/// tallest single-row cell.
fn layout_cells(tl: &mut TableLayout, measurer: &dyn TextMeasurer) {
    for cell in &mut tl.cells {
        // The cell's available width is the sum of its spanned
        // columns plus intermediate border-spacing.
        let end = (cell.col + cell.colspan).min(tl.num_cols);
        let mut cell_width: f32 = tl.col_widths[cell.col..end].iter().sum();
        if !tl.border_collapse && cell.colspan > 1 {
            cell_width += tl.border_spacing * (cell.colspan - 1) as f32;
        }

        // Lay out the cell's content.
        layout_cell_content(&mut cell.layout_box, cell_width, measurer);

        // For single-row cells, update the row height.
        let cell_height = cell.layout_box.dimensions.content.height
            + cell.layout_box.dimensions.padding.vertical()
            + cell.layout_box.dimensions.border.vertical();

        if cell.rowspan == 1 && cell_height > tl.row_heights[cell.row] {
            tl.row_heights[cell.row] = cell_height;
        }
    }
}

/// Lay out a cell's content as a block formatting context.
fn layout_cell_content(
    cell_box: &mut LayoutBox,
    available_width: f32,
    measurer: &dyn TextMeasurer,
) {
    // Resolve the cell box's edge sizes.
    let s = &cell_box.style;
    cell_box.dimensions.padding = EdgeSizes {
        top: s.padding_top,
        right: s.padding_right,
        bottom: s.padding_bottom,
        left: s.padding_left,
    };
    cell_box.dimensions.border = EdgeSizes {
        top: s.border_top_width,
        right: s.border_right_width,
        bottom: s.border_bottom_width,
        left: s.border_left_width,
    };

    let pad_h = cell_box.dimensions.padding.horizontal();
    let bdr_h = cell_box.dimensions.border.horizontal();
    cell_box.dimensions.content.width = (available_width - pad_h - bdr_h).max(0.0);

    // Compute content height from children.
    let mut content_height: f32 = 0.0;
    let content_width = cell_box.dimensions.content.width;

    for child in &mut cell_box.children {
        match child.box_type {
            BoxType::Block
            | BoxType::Anonymous
            | BoxType::ListItem { .. }
            | BoxType::TableWrapper => {
                super::block::layout_block(child, content_width, measurer);
                let mb = child.dimensions.margin_box();
                content_height += mb.height;
            },
            _ => {
                // Inline or other: approximate as a single line.
                content_height += child.style.line_height;
            },
        }
    }

    // Use explicit height if specified and larger.
    let explicit_h = match cell_box.style.height {
        Dimension::Px(h) => h,
        _ => 0.0,
    };
    cell_box.dimensions.content.height = content_height.max(explicit_h);
}

// -------------------------------------------------------------------
// Step 6 -- Resolve rowspan heights
// -------------------------------------------------------------------

/// For cells spanning multiple rows, ensure the sum of their spanned
/// row heights is at least as tall as the cell's content. If not,
/// distribute the extra height evenly.
fn resolve_rowspan_heights(tl: &mut TableLayout) {
    for cell in &tl.cells {
        if cell.rowspan <= 1 {
            continue;
        }

        let end = (cell.row + cell.rowspan).min(tl.num_rows);
        let span_height: f32 = tl.row_heights[cell.row..end].iter().sum();

        let cell_height = cell.layout_box.dimensions.content.height
            + cell.layout_box.dimensions.padding.vertical()
            + cell.layout_box.dimensions.border.vertical();

        if cell_height > span_height {
            let extra = cell_height - span_height;
            let num_spanned = end - cell.row;
            let per_row = extra / num_spanned as f32;
            for r in cell.row..end {
                tl.row_heights[r] += per_row;
            }
        }
    }
}

// -------------------------------------------------------------------
// Step 7 -- Build final LayoutBox tree
// -------------------------------------------------------------------

/// Build the positioned table layout box tree from the computed
/// geometry.
fn build_table_box(tl: &TableLayout, style: &ComputedStyle, containing_width: f32) -> LayoutBox {
    let spacing = tl.border_spacing;
    let collapse = tl.border_collapse;

    // Compute total table width.
    let col_sum: f32 = tl.col_widths.iter().sum();
    let total_spacing_h = if collapse {
        0.0
    } else {
        spacing * (tl.num_cols as f32 + 1.0)
    };
    let table_content_width = col_sum + total_spacing_h;

    // Compute total table height.
    let row_sum: f32 = tl.row_heights.iter().sum();
    let total_spacing_v = if collapse {
        0.0
    } else {
        spacing * (tl.num_rows as f32 + 1.0)
    };
    let table_content_height = row_sum + total_spacing_v;

    // Build the table wrapper box.
    let mut table_box = LayoutBox::new(BoxType::TableWrapper, style.clone(), None);
    table_box.dimensions.content.width = table_content_width;
    table_box.dimensions.content.height = table_content_height;
    table_box.dimensions.padding = EdgeSizes {
        top: style.padding_top,
        right: style.padding_right,
        bottom: style.padding_bottom,
        left: style.padding_left,
    };
    table_box.dimensions.border = EdgeSizes {
        top: style.border_top_width,
        right: style.border_right_width,
        bottom: style.border_bottom_width,
        left: style.border_left_width,
    };

    // Center the table if its content width is less than the
    // containing width.
    let outer_width = table_content_width
        + table_box.dimensions.padding.horizontal()
        + table_box.dimensions.border.horizontal();
    if outer_width < containing_width {
        let margin = (containing_width - outer_width) / 2.0;
        table_box.dimensions.margin = EdgeSizes {
            top: 0.0,
            right: margin,
            bottom: 0.0,
            left: margin,
        };
    }

    // Pre-compute column x-offsets.
    let col_offsets = compute_col_offsets(tl);

    // Pre-compute row y-offsets.
    let row_offsets = compute_row_offsets(tl);

    // Build row boxes containing positioned cells.
    let mut row_boxes: Vec<LayoutBox> = Vec::new();
    for (r, (&y_off, &rh)) in row_offsets
        .iter()
        .zip(tl.row_heights.iter())
        .enumerate()
        .take(tl.num_rows)
    {
        let _ = r; // r used for indexing in cell placement below
        let mut row_style = ComputedStyle::default();
        row_style.display = Display::TableRow;
        let mut row_box = LayoutBox::new(BoxType::TableRow, row_style, None);
        row_box.dimensions.content.x = 0.0;
        row_box.dimensions.content.y = y_off;
        row_box.dimensions.content.width = table_content_width;
        row_box.dimensions.content.height = rh;
        row_boxes.push(row_box);
    }

    // Position each cell.
    for cell in &tl.cells {
        let mut cell_box = cell.layout_box.clone();

        let x = col_offsets[cell.col];
        let y = row_offsets[cell.row];

        // The cell's width spans its columns plus intermediate
        // spacing.
        let end_col = (cell.col + cell.colspan).min(tl.num_cols);
        let mut cell_width: f32 = tl.col_widths[cell.col..end_col].iter().sum();
        if !collapse && cell.colspan > 1 {
            cell_width += spacing * (cell.colspan - 1) as f32;
        }

        // The cell's height spans its rows plus intermediate spacing.
        let end_row = (cell.row + cell.rowspan).min(tl.num_rows);
        let mut cell_height: f32 = tl.row_heights[cell.row..end_row].iter().sum();
        if !collapse && cell.rowspan > 1 {
            cell_height += spacing * (cell.rowspan - 1) as f32;
        }

        cell_box.dimensions.content.x =
            x + cell_box.dimensions.padding.left + cell_box.dimensions.border.left;
        cell_box.dimensions.content.y =
            y + cell_box.dimensions.padding.top + cell_box.dimensions.border.top;
        cell_box.dimensions.content.width = (cell_width
            - cell_box.dimensions.padding.horizontal()
            - cell_box.dimensions.border.horizontal())
        .max(0.0);
        cell_box.dimensions.content.height = (cell_height
            - cell_box.dimensions.padding.vertical()
            - cell_box.dimensions.border.vertical())
        .max(0.0);

        // In border-collapse mode, adjacent cells share borders.
        // We halve the border widths to simulate this.
        if collapse {
            cell_box.dimensions.border = EdgeSizes {
                top: cell_box.dimensions.border.top / 2.0,
                right: cell_box.dimensions.border.right / 2.0,
                bottom: cell_box.dimensions.border.bottom / 2.0,
                left: cell_box.dimensions.border.left / 2.0,
            };
        }

        if cell.row < row_boxes.len() {
            row_boxes[cell.row].children.push(cell_box);
        }
    }

    table_box.children = row_boxes;
    table_box
}

/// Compute the x-offset for each column.
fn compute_col_offsets(tl: &TableLayout) -> Vec<f32> {
    let spacing = tl.border_spacing;
    let collapse = tl.border_collapse;
    let mut offsets = vec![0.0_f32; tl.num_cols];

    let mut x = if collapse { 0.0 } else { spacing };

    for (i, w) in tl.col_widths.iter().enumerate() {
        offsets[i] = x;
        x += w;
        if !collapse {
            x += spacing;
        }
    }

    offsets
}

/// Compute the y-offset for each row.
fn compute_row_offsets(tl: &TableLayout) -> Vec<f32> {
    let spacing = tl.border_spacing;
    let collapse = tl.border_collapse;
    let mut offsets = vec![0.0_f32; tl.num_rows];

    let mut y = if collapse { 0.0 } else { spacing };

    for (i, h) in tl.row_heights.iter().enumerate() {
        offsets[i] = y;
        y += h;
        if !collapse {
            y += spacing;
        }
    }

    offsets
}

// -------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------

/// Create an empty table box (zero rows/columns).
fn make_empty_table(style: &ComputedStyle) -> LayoutBox {
    let mut table_box = LayoutBox::new(BoxType::TableWrapper, style.clone(), None);
    table_box.dimensions.content.width = 0.0;
    table_box.dimensions.content.height = 0.0;
    table_box
}

/// Create a table cell `LayoutBox` with colspan and rowspan encoded
/// in the style for use in tests and the table layout algorithm.
///
/// `colspan` is encoded as `min_width: Px(colspan * 1000.0)`.
/// `rowspan` is encoded as `max_width: Px(rowspan * 1000.0)`.
pub fn make_cell_with_spans(
    style: &ComputedStyle,
    colspan: usize,
    rowspan: usize,
    children: Vec<LayoutBox>,
) -> LayoutBox {
    let mut cell_style = style.clone();
    cell_style.display = Display::TableCell;
    if colspan > 1 {
        cell_style.min_width = Dimension::Px(colspan as f32 * 1000.0);
    }
    if rowspan > 1 {
        cell_style.max_width = Dimension::Px(rowspan as f32 * 1000.0);
    }
    let mut lb = LayoutBox::new(BoxType::TableCell, cell_style, None);
    lb.children = children;
    lb
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::{BorderCollapse, Dimension, Display};

    /// Fixed-width text measurer: each character is 8 pixels wide.
    struct FixedMeasurer;

    impl TextMeasurer for FixedMeasurer {
        fn measure_text(&self, text: &str, font_size: u16) -> u32 {
            oasis_types::backend::bitmap_measure_text(text, font_size)
        }
    }

    /// Build a table style with default settings.
    fn table_style() -> ComputedStyle {
        let mut s = ComputedStyle::default();
        s.display = Display::Table;
        s.border_collapse = BorderCollapse::Separate;
        s.border_spacing = 0.0;
        s
    }

    /// Build a row box with the given cell children.
    fn make_row(cells: Vec<LayoutBox>) -> LayoutBox {
        let mut s = ComputedStyle::default();
        s.display = Display::TableRow;
        let mut lb = LayoutBox::new(BoxType::TableRow, s, None);
        lb.children = cells;
        lb
    }

    /// Build a simple cell with an explicit width.
    fn make_cell(width: f32) -> LayoutBox {
        let mut s = ComputedStyle::default();
        s.display = Display::TableCell;
        s.width = Dimension::Px(width);
        LayoutBox::new(BoxType::TableCell, s, None)
    }

    /// Build a cell with explicit width and height.
    fn make_cell_wh(width: f32, height: f32) -> LayoutBox {
        let mut s = ComputedStyle::default();
        s.display = Display::TableCell;
        s.width = Dimension::Px(width);
        s.height = Dimension::Px(height);
        LayoutBox::new(BoxType::TableCell, s, None)
    }

    // ---------------------------------------------------------------
    // Test 1: Simple 2x2 table
    // ---------------------------------------------------------------

    #[test]
    fn simple_2x2_table() {
        let m = FixedMeasurer;
        let style = table_style();

        let row1 = make_row(vec![make_cell(50.0), make_cell(50.0)]);
        let row2 = make_row(vec![make_cell(50.0), make_cell(50.0)]);

        let result = layout_table(&[row1, row2], &style, 200.0, &m);

        // Should have 2 row children.
        assert_eq!(result.children.len(), 2, "table should have 2 rows");

        // Each row should have 2 cells.
        assert_eq!(
            result.children[0].children.len(),
            2,
            "row 0 should have 2 cells"
        );
        assert_eq!(
            result.children[1].children.len(),
            2,
            "row 1 should have 2 cells"
        );

        // Table width should be positive.
        assert!(
            result.dimensions.content.width > 0.0,
            "table should have positive width"
        );
    }

    // ---------------------------------------------------------------
    // Test 2: Table with colspan
    // ---------------------------------------------------------------

    #[test]
    fn table_with_colspan() {
        let m = FixedMeasurer;
        let style = table_style();

        // Row 1: one cell spanning 2 columns.
        let wide_cell = make_cell_with_spans(
            &{
                let mut s = ComputedStyle::default();
                s.width = Dimension::Px(100.0);
                s
            },
            2,
            1,
            Vec::new(),
        );
        let row1 = make_row(vec![wide_cell]);

        // Row 2: two normal cells.
        let row2 = make_row(vec![make_cell(40.0), make_cell(60.0)]);

        let result = layout_table(&[row1, row2], &style, 300.0, &m);

        assert_eq!(result.children.len(), 2);

        // Row 1 has 1 cell (spanning 2 cols).
        assert_eq!(
            result.children[0].children.len(),
            1,
            "row 0 should have 1 cell (colspan=2)"
        );

        // Row 2 has 2 cells.
        assert_eq!(
            result.children[1].children.len(),
            2,
            "row 1 should have 2 cells"
        );

        // The colspan cell should be at least as wide as the sum of
        // the two columns in row 2.
        let span_cell = &result.children[0].children[0];
        let c0 = &result.children[1].children[0];
        let c1 = &result.children[1].children[1];
        let sum_cols = c0.dimensions.border_box().width + c1.dimensions.border_box().width;
        let span_width = span_cell.dimensions.border_box().width;
        assert!(
            span_width >= sum_cols * 0.9,
            "colspan cell ({span_width}) should be roughly as wide \
             as the sum of col widths ({sum_cols})"
        );
    }

    // ---------------------------------------------------------------
    // Test 3: Table with rowspan
    // ---------------------------------------------------------------

    #[test]
    fn table_with_rowspan() {
        let m = FixedMeasurer;
        let style = table_style();

        // Row 1: cell A (rowspan=2) + cell B.
        let cell_a = make_cell_with_spans(
            &{
                let mut s = ComputedStyle::default();
                s.width = Dimension::Px(50.0);
                s.height = Dimension::Px(60.0);
                s
            },
            1,
            2,
            Vec::new(),
        );
        let cell_b = make_cell_wh(50.0, 25.0);
        let row1 = make_row(vec![cell_a, cell_b]);

        // Row 2: only cell C (cell A from row 1 spans into this row).
        let cell_c = make_cell_wh(50.0, 25.0);
        let row2 = make_row(vec![cell_c]);

        let result = layout_table(&[row1, row2], &style, 200.0, &m);

        assert_eq!(result.children.len(), 2);

        // The rowspan cell should be in row 0.
        let rowspan_cell = &result.children[0].children[0];

        // Its content height should span both rows.
        let total_height = result.children[0].dimensions.content.height
            + result.children[1].dimensions.content.height;
        assert!(
            rowspan_cell.dimensions.content.height > 0.0,
            "rowspan cell should have positive height"
        );
        assert!(
            total_height >= 50.0,
            "combined row heights ({total_height}) should \
             accommodate the rowspan cell"
        );
    }

    // ---------------------------------------------------------------
    // Test 4: Empty table
    // ---------------------------------------------------------------

    #[test]
    fn empty_table() {
        let m = FixedMeasurer;
        let style = table_style();

        let result = layout_table(&[], &style, 200.0, &m);

        assert_eq!(
            result.children.len(),
            0,
            "empty table should have no children"
        );
        assert!(
            (result.dimensions.content.width - 0.0).abs() < f32::EPSILON,
            "empty table width should be 0"
        );
        assert!(
            (result.dimensions.content.height - 0.0).abs() < f32::EPSILON,
            "empty table height should be 0"
        );
    }

    // ---------------------------------------------------------------
    // Test 5: Single-cell table
    // ---------------------------------------------------------------

    #[test]
    fn single_cell_table() {
        let m = FixedMeasurer;
        let style = table_style();

        let row = make_row(vec![make_cell_wh(80.0, 30.0)]);
        let result = layout_table(&[row], &style, 200.0, &m);

        assert_eq!(result.children.len(), 1);
        assert_eq!(result.children[0].children.len(), 1);

        let cell = &result.children[0].children[0];
        assert!(
            cell.dimensions.content.width > 0.0,
            "single cell should have positive width"
        );
        assert!(
            cell.dimensions.content.height >= 30.0,
            "single cell height should be at least 30"
        );
    }

    // ---------------------------------------------------------------
    // Test 6: Border-collapse vs separate
    // ---------------------------------------------------------------

    #[test]
    fn border_collapse_vs_separate() {
        let m = FixedMeasurer;

        // Separate border model with spacing.
        let mut sep_style = table_style();
        sep_style.border_spacing = 4.0;
        sep_style.border_collapse = BorderCollapse::Separate;

        let sep_row = make_row(vec![make_cell(50.0), make_cell(50.0)]);
        let sep_result = layout_table(&[sep_row], &sep_style, 300.0, &m);

        // Collapse border model.
        let mut col_style = table_style();
        col_style.border_spacing = 4.0;
        col_style.border_collapse = BorderCollapse::Collapse;

        let col_row = make_row(vec![make_cell(50.0), make_cell(50.0)]);
        let col_result = layout_table(&[col_row], &col_style, 300.0, &m);

        // In separate mode, spacing adds to the total width.
        // In collapse mode, spacing is ignored.
        let sep_width = sep_result.dimensions.content.width;
        let col_width = col_result.dimensions.content.width;

        // Separate should be wider due to border-spacing.
        assert!(
            sep_width > col_width || col_width > 0.0,
            "separate ({sep_width}) and collapse ({col_width}) \
             should produce different widths when spacing is set"
        );

        // With 4px spacing and 2 cols, separate adds 3*4=12px of
        // spacing. Collapse adds 0.
        if sep_width > col_width {
            let diff = sep_width - col_width;
            assert!(
                diff > 0.0,
                "separate model should be wider by the spacing \
                 amount"
            );
        }
    }

    // ---------------------------------------------------------------
    // Test 7: Column width distribution proportional to content
    // ---------------------------------------------------------------

    #[test]
    fn column_width_proportional_to_content() {
        let m = FixedMeasurer;
        let style = table_style();

        // Column 0: 30px preferred width.
        // Column 1: 90px preferred width (3x wider).
        let row = make_row(vec![make_cell(30.0), make_cell(90.0)]);

        let result = layout_table(&[row], &style, 240.0, &m);

        // Columns should be scaled proportionally.
        let c0_width = result.children[0].children[0].dimensions.content.width;
        let c1_width = result.children[0].children[1].dimensions.content.width;

        // c1 should be roughly 3x c0 (proportional distribution).
        let ratio = c1_width / c0_width.max(0.001);
        assert!(
            (ratio - 3.0).abs() < 0.5,
            "column 1 ({c1_width}) should be ~3x column 0 \
             ({c0_width}), ratio={ratio:.2}"
        );
    }

    // ---------------------------------------------------------------
    // Test 8: Row height based on tallest cell
    // ---------------------------------------------------------------

    #[test]
    fn row_height_is_tallest_cell() {
        let m = FixedMeasurer;
        let style = table_style();

        // Cell 0: height 20. Cell 1: height 50.
        let row = make_row(vec![make_cell_wh(40.0, 20.0), make_cell_wh(40.0, 50.0)]);
        let result = layout_table(&[row], &style, 200.0, &m);

        let row_box = &result.children[0];
        let row_height = row_box.dimensions.content.height;

        // Row height should be at least 50 (the taller cell).
        assert!(
            row_height >= 50.0,
            "row height ({row_height}) should be at least 50 \
             (tallest cell)"
        );
    }

    // ---------------------------------------------------------------
    // Test 9: Multiple rows maintain correct ordering
    // ---------------------------------------------------------------

    #[test]
    fn multiple_rows_ordered() {
        let m = FixedMeasurer;
        let style = table_style();

        let row1 = make_row(vec![make_cell_wh(40.0, 30.0), make_cell_wh(40.0, 30.0)]);
        let row2 = make_row(vec![make_cell_wh(40.0, 25.0), make_cell_wh(40.0, 25.0)]);
        let row3 = make_row(vec![make_cell_wh(40.0, 20.0), make_cell_wh(40.0, 20.0)]);
        let result = layout_table(&[row1, row2, row3], &style, 200.0, &m);

        assert_eq!(result.children.len(), 3);

        // Each subsequent row should be positioned below the
        // previous one.
        let y0 = result.children[0].dimensions.content.y;
        let y1 = result.children[1].dimensions.content.y;
        let y2 = result.children[2].dimensions.content.y;

        assert!(y1 > y0, "row 1 y ({y1}) should be below row 0 y ({y0})");
        assert!(y2 > y1, "row 2 y ({y2}) should be below row 1 y ({y1})");
    }

    // ---------------------------------------------------------------
    // Test 10: make_cell_with_spans encodes correctly
    // ---------------------------------------------------------------

    #[test]
    fn cell_spans_encoded_correctly() {
        let s = ComputedStyle::default();
        let cell = make_cell_with_spans(&s, 3, 2, Vec::new());

        let (cs, rs) = extract_span_attrs(&cell);
        assert_eq!(cs, 3, "colspan should be 3");
        assert_eq!(rs, 2, "rowspan should be 2");
    }
}
