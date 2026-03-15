//! Lightweight flex and grid layout helpers.
//!
//! These compute pixel positions from declarative layout specs, sitting as a
//! composition layer **above** SDI. Apps describe layout declaratively; the
//! system computes pixel positions. SDI stays flat and dumb.

use crate::layout::{HAlign, Padding, VAlign, align_y};

/// Direction of flex layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    /// Horizontal layout (left to right).
    Row,
    /// Vertical layout (top to bottom).
    Column,
}

/// How a child should be sized along the main axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FlexSize {
    /// Exact pixel size.
    Fixed(u32),
    /// Flex weight (like CSS flex-grow). A child with weight 2 gets twice
    /// the remaining space of a child with weight 1.
    Flex(u32),
    /// Percentage of parent main-axis size.
    Percent(f32),
}

/// A child in a flex container.
#[derive(Debug, Clone)]
pub struct FlexChild {
    /// How this child is sized along the main axis.
    pub size: FlexSize,
    /// Cross-axis override for this child.
    pub align_self: Option<VAlign>,
    /// Margin around this child.
    pub margin: Padding,
}

impl FlexChild {
    /// Create a child with fixed pixel size.
    pub fn fixed(size: u32) -> Self {
        Self {
            size: FlexSize::Fixed(size),
            align_self: None,
            margin: Padding::ZERO,
        }
    }

    /// Create a child that fills remaining space with given weight.
    pub fn flex(weight: u32) -> Self {
        Self {
            size: FlexSize::Flex(weight),
            align_self: None,
            margin: Padding::ZERO,
        }
    }

    /// Create a child sized as percentage of parent.
    pub fn percent(pct: f32) -> Self {
        Self {
            size: FlexSize::Percent(pct),
            align_self: None,
            margin: Padding::ZERO,
        }
    }

    /// Add margin to this child.
    pub fn with_margin(mut self, margin: Padding) -> Self {
        self.margin = margin;
        self
    }

    /// Set cross-axis alignment for this child.
    pub fn with_align(mut self, align: VAlign) -> Self {
        self.align_self = Some(align);
        self
    }
}

/// Computed position for a child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputedRect {
    /// X coordinate.
    pub x: i32,
    /// Y coordinate.
    pub y: i32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
}

/// A flex container that computes child positions along a main axis.
#[derive(Debug, Clone)]
pub struct FlexLayout {
    /// Layout direction (row or column).
    pub direction: FlexDirection,
    /// Gap between items in pixels.
    pub gap: u32,
    /// Cross-axis default alignment.
    pub align_items: VAlign,
    /// Main-axis alignment (distributes remaining space).
    pub justify: HAlign,
}

impl Default for FlexLayout {
    fn default() -> Self {
        Self {
            direction: FlexDirection::Row,
            gap: 0,
            align_items: VAlign::Top,
            justify: HAlign::Left,
        }
    }
}

impl FlexLayout {
    /// Create a horizontal flex layout.
    pub fn row() -> Self {
        Self::default()
    }

    /// Create a vertical flex layout.
    pub fn column() -> Self {
        Self {
            direction: FlexDirection::Column,
            ..Self::default()
        }
    }

    /// Set gap between items.
    pub fn with_gap(mut self, gap: u32) -> Self {
        self.gap = gap;
        self
    }

    /// Set cross-axis alignment for all items.
    pub fn with_align_items(mut self, align: VAlign) -> Self {
        self.align_items = align;
        self
    }

    /// Set main-axis justification.
    pub fn with_justify(mut self, justify: HAlign) -> Self {
        self.justify = justify;
        self
    }

    /// Given parent bounds and child specs, compute rects for each child.
    pub fn compute(
        &self,
        parent_x: i32,
        parent_y: i32,
        parent_w: u32,
        parent_h: u32,
        children: &[FlexChild],
    ) -> Vec<ComputedRect> {
        if children.is_empty() {
            return Vec::new();
        }

        let is_row = self.direction == FlexDirection::Row;
        let main_total = if is_row { parent_w } else { parent_h };
        let cross_total = if is_row { parent_h } else { parent_w };

        let total_gap = self.gap * children.len().saturating_sub(1) as u32;

        // First pass: compute fixed/percent sizes and total flex weight.
        let mut sizes: Vec<u32> = Vec::with_capacity(children.len());
        let mut flex_total_weight = 0u32;
        let mut consumed = total_gap;

        for child in children {
            let margin_main = if is_row {
                child.margin.horizontal()
            } else {
                child.margin.vertical()
            };
            match child.size {
                FlexSize::Fixed(px) => {
                    sizes.push(px);
                    consumed += px + margin_main;
                },
                FlexSize::Percent(pct) => {
                    let px = (main_total as f32 * pct / 100.0) as u32;
                    sizes.push(px);
                    consumed += px + margin_main;
                },
                FlexSize::Flex(weight) => {
                    sizes.push(0); // placeholder
                    flex_total_weight += weight;
                    consumed += margin_main;
                },
            }
        }

        // Second pass: distribute remaining space to flex children.
        let remaining = main_total.saturating_sub(consumed);
        if let Some(ftw) = std::num::NonZero::new(flex_total_weight) {
            for (i, child) in children.iter().enumerate() {
                if let FlexSize::Flex(weight) = child.size {
                    sizes[i] = remaining * weight / ftw;
                }
            }
        }

        // Compute main-axis starting offset for justify alignment.
        let total_used: u32 = sizes.iter().sum::<u32>()
            + total_gap
            + children
                .iter()
                .map(|c| {
                    if is_row {
                        c.margin.horizontal()
                    } else {
                        c.margin.vertical()
                    }
                })
                .sum::<u32>();
        let justify_offset = match self.justify {
            HAlign::Left => 0,
            HAlign::Center => main_total.saturating_sub(total_used) / 2,
            HAlign::Right => main_total.saturating_sub(total_used),
        };

        // Third pass: compute final rects.
        let mut results = Vec::with_capacity(children.len());
        let mut cursor = justify_offset as i32;

        for (i, child) in children.iter().enumerate() {
            let main_size = sizes[i];
            let align = child.align_self.unwrap_or(self.align_items);

            let margin_before = if is_row {
                child.margin.left as i32
            } else {
                child.margin.top as i32
            };
            let margin_after = if is_row {
                child.margin.right as i32
            } else {
                child.margin.bottom as i32
            };

            let cross_margin = if is_row {
                child.margin.vertical()
            } else {
                child.margin.horizontal()
            };
            let cross_avail = cross_total.saturating_sub(cross_margin);
            let cross_offset = align_y(cross_avail, cross_avail, align)
                + if is_row {
                    child.margin.top as i32
                } else {
                    child.margin.left as i32
                };

            cursor += margin_before;

            let rect = if is_row {
                ComputedRect {
                    x: parent_x + cursor,
                    y: parent_y + cross_offset,
                    w: main_size,
                    h: cross_avail,
                }
            } else {
                ComputedRect {
                    x: parent_x + cross_offset,
                    y: parent_y + cursor,
                    w: cross_avail,
                    h: main_size,
                }
            };
            results.push(rect);

            cursor += main_size as i32 + margin_after + self.gap as i32;
        }

        results
    }
}

/// A grid layout helper that computes cell positions for a uniform grid.
#[derive(Debug, Clone)]
pub struct GridLayout {
    /// Number of columns.
    pub cols: u32,
    /// Gap between rows in pixels.
    pub row_gap: u32,
    /// Gap between columns in pixels.
    pub col_gap: u32,
    /// Padding around the grid.
    pub padding: Padding,
}

impl GridLayout {
    /// Create a new grid layout with specified column count.
    pub fn new(cols: u32) -> Self {
        Self {
            cols,
            row_gap: 0,
            col_gap: 0,
            padding: Padding::ZERO,
        }
    }

    /// Set gaps between cells.
    pub fn with_gap(mut self, row_gap: u32, col_gap: u32) -> Self {
        self.row_gap = row_gap;
        self.col_gap = col_gap;
        self
    }

    /// Set padding around the grid.
    pub fn with_padding(mut self, padding: Padding) -> Self {
        self.padding = padding;
        self
    }

    /// Compute the cell rect for a given linear index.
    ///
    /// Cells are sized uniformly to fill the parent, accounting for gaps
    /// and padding. Returns `None` if `cols` is 0.
    pub fn cell_rect(
        &self,
        index: usize,
        parent_x: i32,
        parent_y: i32,
        parent_w: u32,
        parent_h: u32,
        total: usize,
    ) -> Option<ComputedRect> {
        if self.cols == 0 {
            return None;
        }
        let rows = (total as u32).div_ceil(self.cols);
        if rows == 0 {
            return None;
        }

        let (px, py, pw, ph) = self
            .padding
            .inner_rect(parent_x, parent_y, parent_w, parent_h);

        let total_col_gap = self.col_gap * self.cols.saturating_sub(1);
        let total_row_gap = self.row_gap * rows.saturating_sub(1);
        let cell_w = pw.saturating_sub(total_col_gap) / self.cols;
        let cell_h = ph.saturating_sub(total_row_gap) / rows;

        let col = (index as u32) % self.cols;
        let row = (index as u32) / self.cols;

        Some(ComputedRect {
            x: px + (col * (cell_w + self.col_gap)) as i32,
            y: py + (row * (cell_h + self.row_gap)) as i32,
            w: cell_w,
            h: cell_h,
        })
    }

    /// Convenience: compute all cell rects for `total` items.
    pub fn all_cells(
        &self,
        parent_x: i32,
        parent_y: i32,
        parent_w: u32,
        parent_h: u32,
        total: usize,
    ) -> Vec<ComputedRect> {
        (0..total)
            .filter_map(|i| self.cell_rect(i, parent_x, parent_y, parent_w, parent_h, total))
            .collect()
    }
}

/// Convenience: compute a vertical list of equally-spaced items.
///
/// Returns `(item_height, rects)`.
pub fn vertical_list(
    x: i32,
    y: i32,
    w: u32,
    item_height: u32,
    gap: u32,
    count: usize,
) -> Vec<ComputedRect> {
    (0..count)
        .map(|i| ComputedRect {
            x,
            y: y + (i as u32 * (item_height + gap)) as i32,
            w,
            h: item_height,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- FlexLayout -------------------------------------------------------

    #[test]
    fn row_fixed_children() {
        let layout = FlexLayout::row().with_gap(4);
        let children = vec![
            FlexChild::fixed(50),
            FlexChild::fixed(30),
            FlexChild::fixed(20),
        ];
        let rects = layout.compute(10, 20, 200, 40, &children);
        assert_eq!(rects.len(), 3);
        assert_eq!(
            rects[0],
            ComputedRect {
                x: 10,
                y: 20,
                w: 50,
                h: 40
            }
        );
        assert_eq!(
            rects[1],
            ComputedRect {
                x: 64,
                y: 20,
                w: 30,
                h: 40
            }
        );
        assert_eq!(
            rects[2],
            ComputedRect {
                x: 98,
                y: 20,
                w: 20,
                h: 40
            }
        );
    }

    #[test]
    fn column_fixed_children() {
        let layout = FlexLayout::column().with_gap(2);
        let children = vec![FlexChild::fixed(10), FlexChild::fixed(10)];
        let rects = layout.compute(0, 0, 100, 50, &children);
        assert_eq!(rects.len(), 2);
        assert_eq!(
            rects[0],
            ComputedRect {
                x: 0,
                y: 0,
                w: 100,
                h: 10
            }
        );
        assert_eq!(
            rects[1],
            ComputedRect {
                x: 0,
                y: 12,
                w: 100,
                h: 10
            }
        );
    }

    #[test]
    fn flex_weights_distribute_space() {
        let layout = FlexLayout::row();
        let children = vec![FlexChild::flex(1), FlexChild::flex(2)];
        let rects = layout.compute(0, 0, 300, 50, &children);
        assert_eq!(rects[0].w, 100);
        assert_eq!(rects[1].w, 200);
    }

    #[test]
    fn percent_sizing() {
        let layout = FlexLayout::row();
        let children = vec![FlexChild::percent(25.0), FlexChild::percent(75.0)];
        let rects = layout.compute(0, 0, 200, 50, &children);
        assert_eq!(rects[0].w, 50);
        assert_eq!(rects[1].w, 150);
    }

    #[test]
    fn mixed_fixed_and_flex() {
        let layout = FlexLayout::row();
        let children = vec![FlexChild::fixed(40), FlexChild::flex(1)];
        let rects = layout.compute(0, 0, 100, 20, &children);
        assert_eq!(rects[0].w, 40);
        assert_eq!(rects[1].w, 60);
    }

    #[test]
    fn justify_center() {
        let layout = FlexLayout::row().with_justify(HAlign::Center);
        let children = vec![FlexChild::fixed(20)];
        let rects = layout.compute(0, 0, 100, 40, &children);
        assert_eq!(rects[0].x, 40);
    }

    #[test]
    fn justify_right() {
        let layout = FlexLayout::row().with_justify(HAlign::Right);
        let children = vec![FlexChild::fixed(20)];
        let rects = layout.compute(0, 0, 100, 40, &children);
        assert_eq!(rects[0].x, 80);
    }

    #[test]
    fn empty_children() {
        let layout = FlexLayout::row();
        let rects = layout.compute(0, 0, 100, 50, &[]);
        assert!(rects.is_empty());
    }

    // -- GridLayout -------------------------------------------------------

    #[test]
    fn grid_basic() {
        let grid = GridLayout::new(3);
        let rect = grid.cell_rect(0, 0, 0, 90, 60, 6).unwrap();
        assert_eq!(
            rect,
            ComputedRect {
                x: 0,
                y: 0,
                w: 30,
                h: 30
            }
        );
        let rect = grid.cell_rect(1, 0, 0, 90, 60, 6).unwrap();
        assert_eq!(
            rect,
            ComputedRect {
                x: 30,
                y: 0,
                w: 30,
                h: 30
            }
        );
        let rect = grid.cell_rect(3, 0, 0, 90, 60, 6).unwrap();
        assert_eq!(
            rect,
            ComputedRect {
                x: 0,
                y: 30,
                w: 30,
                h: 30
            }
        );
    }

    #[test]
    fn grid_with_gap() {
        let grid = GridLayout::new(2).with_gap(4, 4);
        let rect = grid.cell_rect(0, 10, 20, 104, 44, 4).unwrap();
        // cols=2, rows=2, pw=104, col_gap=4*1=4, cell_w=(104-4)/2=50
        // ph=44, row_gap=4*1=4, cell_h=(44-4)/2=20
        assert_eq!(
            rect,
            ComputedRect {
                x: 10,
                y: 20,
                w: 50,
                h: 20
            }
        );
        let rect = grid.cell_rect(1, 10, 20, 104, 44, 4).unwrap();
        assert_eq!(
            rect,
            ComputedRect {
                x: 64,
                y: 20,
                w: 50,
                h: 20
            }
        );
        let rect = grid.cell_rect(2, 10, 20, 104, 44, 4).unwrap();
        assert_eq!(
            rect,
            ComputedRect {
                x: 10,
                y: 44,
                w: 50,
                h: 20
            }
        );
    }

    #[test]
    fn grid_with_padding() {
        let grid = GridLayout::new(2).with_padding(Padding::uniform(5));
        let rect = grid.cell_rect(0, 0, 0, 50, 30, 2).unwrap();
        // inner: x=5, y=5, w=40, h=20. cols=2 rows=1. cell_w=20, cell_h=20
        assert_eq!(
            rect,
            ComputedRect {
                x: 5,
                y: 5,
                w: 20,
                h: 20
            }
        );
    }

    #[test]
    fn grid_zero_cols_returns_none() {
        let grid = GridLayout::new(0);
        assert!(grid.cell_rect(0, 0, 0, 100, 100, 1).is_none());
    }

    #[test]
    fn grid_all_cells() {
        let grid = GridLayout::new(2);
        let cells = grid.all_cells(0, 0, 40, 40, 4);
        assert_eq!(cells.len(), 4);
        assert_eq!(
            cells[0],
            ComputedRect {
                x: 0,
                y: 0,
                w: 20,
                h: 20
            }
        );
        assert_eq!(
            cells[3],
            ComputedRect {
                x: 20,
                y: 20,
                w: 20,
                h: 20
            }
        );
    }

    // -- vertical_list ----------------------------------------------------

    #[test]
    fn vertical_list_basic() {
        let rects = vertical_list(8, 26, 464, 18, 0, 3);
        assert_eq!(rects.len(), 3);
        assert_eq!(
            rects[0],
            ComputedRect {
                x: 8,
                y: 26,
                w: 464,
                h: 18
            }
        );
        assert_eq!(
            rects[1],
            ComputedRect {
                x: 8,
                y: 44,
                w: 464,
                h: 18
            }
        );
        assert_eq!(
            rects[2],
            ComputedRect {
                x: 8,
                y: 62,
                w: 464,
                h: 18
            }
        );
    }

    #[test]
    fn vertical_list_with_gap() {
        let rects = vertical_list(0, 0, 100, 10, 5, 2);
        assert_eq!(
            rects[0],
            ComputedRect {
                x: 0,
                y: 0,
                w: 100,
                h: 10
            }
        );
        assert_eq!(
            rects[1],
            ComputedRect {
                x: 0,
                y: 15,
                w: 100,
                h: 10
            }
        );
    }

    // -- Additional flex layout tests --

    #[test]
    fn column_flex_distributes_vertically() {
        let layout = FlexLayout::column();
        let children = vec![FlexChild::flex(1), FlexChild::flex(3)];
        let rects = layout.compute(0, 0, 100, 200, &children);
        assert_eq!(rects[0].h, 50);
        assert_eq!(rects[1].h, 150);
        assert_eq!(rects[0].w, 100); // cross-axis full
        assert_eq!(rects[1].w, 100);
    }

    #[test]
    fn column_percent_sizing() {
        let layout = FlexLayout::column();
        let children = vec![FlexChild::percent(30.0), FlexChild::percent(70.0)];
        let rects = layout.compute(0, 0, 100, 200, &children);
        assert_eq!(rects[0].h, 60); // 30% of 200
        assert_eq!(rects[1].h, 140); // 70% of 200
    }

    #[test]
    fn column_mixed_fixed_and_flex() {
        let layout = FlexLayout::column();
        let children = vec![FlexChild::fixed(20), FlexChild::flex(1)];
        let rects = layout.compute(0, 0, 100, 100, &children);
        assert_eq!(rects[0].h, 20);
        assert_eq!(rects[1].h, 80);
    }

    #[test]
    fn column_justify_center() {
        let layout = FlexLayout::column().with_justify(HAlign::Center);
        let children = vec![FlexChild::fixed(20)];
        let rects = layout.compute(0, 0, 100, 100, &children);
        assert_eq!(rects[0].y, 40);
    }

    #[test]
    fn column_justify_right() {
        let layout = FlexLayout::column().with_justify(HAlign::Right);
        let children = vec![FlexChild::fixed(20)];
        let rects = layout.compute(0, 0, 100, 100, &children);
        assert_eq!(rects[0].y, 80);
    }

    #[test]
    fn row_with_margin() {
        let layout = FlexLayout::row();
        let children = vec![FlexChild::fixed(40).with_margin(Padding::uniform(5))];
        let rects = layout.compute(0, 0, 200, 50, &children);
        assert_eq!(rects[0].x, 5); // left margin
        assert_eq!(rects[0].w, 40);
    }

    #[test]
    fn column_with_margin() {
        let layout = FlexLayout::column();
        let children = vec![FlexChild::fixed(30).with_margin(Padding::new(10, 0, 0, 0))];
        let rects = layout.compute(0, 0, 100, 200, &children);
        assert_eq!(rects[0].y, 10); // top margin
        assert_eq!(rects[0].h, 30);
    }

    #[test]
    fn row_three_flex_equal_weight() {
        let layout = FlexLayout::row();
        let children = vec![FlexChild::flex(1), FlexChild::flex(1), FlexChild::flex(1)];
        let rects = layout.compute(0, 0, 300, 50, &children);
        assert_eq!(rects[0].w, 100);
        assert_eq!(rects[1].w, 100);
        assert_eq!(rects[2].w, 100);
    }

    #[test]
    fn row_with_gap_positions() {
        let layout = FlexLayout::row().with_gap(10);
        let children = vec![FlexChild::fixed(50), FlexChild::fixed(50)];
        let rects = layout.compute(0, 0, 200, 40, &children);
        assert_eq!(rects[0].x, 0);
        assert_eq!(rects[1].x, 60); // 50 + 10 gap
    }

    #[test]
    fn column_with_gap_positions() {
        let layout = FlexLayout::column().with_gap(5);
        let children = vec![
            FlexChild::fixed(20),
            FlexChild::fixed(20),
            FlexChild::fixed(20),
        ];
        let rects = layout.compute(0, 0, 100, 200, &children);
        assert_eq!(rects[0].y, 0);
        assert_eq!(rects[1].y, 25); // 20 + 5
        assert_eq!(rects[2].y, 50); // 20 + 5 + 20 + 5
    }

    #[test]
    fn parent_offset_applied() {
        let layout = FlexLayout::row();
        let children = vec![FlexChild::fixed(30)];
        let rects = layout.compute(100, 200, 300, 50, &children);
        assert_eq!(rects[0].x, 100);
        assert_eq!(rects[0].y, 200);
    }

    #[test]
    fn zero_size_parent() {
        let layout = FlexLayout::row();
        let children = vec![FlexChild::flex(1)];
        let rects = layout.compute(0, 0, 0, 0, &children);
        assert_eq!(rects[0].w, 0);
        assert_eq!(rects[0].h, 0);
    }

    #[test]
    fn flex_child_with_align_self() {
        let child = FlexChild::fixed(20).with_align(VAlign::Bottom);
        assert_eq!(child.align_self, Some(VAlign::Bottom));
    }

    #[test]
    fn flex_child_default_margin() {
        let child = FlexChild::fixed(50);
        assert_eq!(child.margin, Padding::ZERO);
    }

    #[test]
    fn flex_direction_debug() {
        assert_eq!(format!("{:?}", FlexDirection::Row), "Row");
        assert_eq!(format!("{:?}", FlexDirection::Column), "Column");
    }

    #[test]
    fn grid_large_index() {
        let grid = GridLayout::new(3);
        let rect = grid.cell_rect(5, 0, 0, 90, 60, 6).unwrap();
        // Index 5: col=2, row=1
        assert_eq!(rect.x, 60); // col 2 * 30
        assert_eq!(rect.y, 30); // row 1 * 30
    }

    #[test]
    fn grid_one_col() {
        let grid = GridLayout::new(1);
        let cells = grid.all_cells(0, 0, 100, 100, 3);
        assert_eq!(cells.len(), 3);
        // Each cell should be full width.
        for cell in &cells {
            assert_eq!(cell.w, 100);
        }
    }

    #[test]
    fn grid_with_both_gaps_and_padding() {
        let grid = GridLayout::new(2)
            .with_gap(4, 4)
            .with_padding(Padding::uniform(5));
        let rect = grid.cell_rect(0, 0, 0, 100, 100, 4).unwrap();
        assert_eq!(rect.x, 5); // padding left
        assert_eq!(rect.y, 5); // padding top
    }

    #[test]
    fn grid_zero_total_returns_none() {
        let grid = GridLayout::new(3);
        // zero total items => rows=0 => returns None
        assert!(grid.cell_rect(0, 0, 0, 90, 60, 0).is_none());
    }

    #[test]
    fn grid_all_cells_consistent_with_cell_rect() {
        let grid = GridLayout::new(3).with_gap(2, 2);
        let cells = grid.all_cells(10, 20, 200, 100, 6);
        for i in 0..6 {
            let single = grid.cell_rect(i, 10, 20, 200, 100, 6).unwrap();
            assert_eq!(cells[i], single);
        }
    }

    #[test]
    fn vertical_list_empty() {
        let rects = vertical_list(0, 0, 100, 20, 0, 0);
        assert!(rects.is_empty());
    }

    #[test]
    fn vertical_list_single_item() {
        let rects = vertical_list(5, 10, 80, 20, 0, 1);
        assert_eq!(rects.len(), 1);
        assert_eq!(
            rects[0],
            ComputedRect {
                x: 5,
                y: 10,
                w: 80,
                h: 20,
            }
        );
    }

    #[test]
    fn computed_rect_debug() {
        let r = ComputedRect {
            x: 1,
            y: 2,
            w: 3,
            h: 4,
        };
        let dbg = format!("{r:?}");
        assert!(dbg.contains("ComputedRect"));
    }

    #[test]
    fn computed_rect_clone_and_eq() {
        let a = ComputedRect {
            x: 10,
            y: 20,
            w: 30,
            h: 40,
        };
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn flex_layout_default_is_row() {
        let layout = FlexLayout::default();
        assert_eq!(layout.direction, FlexDirection::Row);
        assert_eq!(layout.gap, 0);
        assert_eq!(layout.align_items, VAlign::Top);
        assert_eq!(layout.justify, HAlign::Left);
    }
}
