//! Table widget: multi-column data grid with optional header and selection.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// Row height constant.
const ROW_HEIGHT: u32 = 18;

/// Header row height.
const HEADER_HEIGHT: u32 = 20;

/// Sort direction for a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

/// A column definition for the table.
#[derive(Debug, Clone)]
pub struct Column {
    /// Column header text.
    pub label: String,
    /// Fixed width in pixels (0 = auto-fill remaining space).
    pub width: u32,
}

impl Column {
    /// Create a new column with the given label and width.
    pub fn new(label: impl Into<String>, width: u32) -> Self {
        Self {
            label: label.into(),
            width,
        }
    }
}

/// A multi-column data grid.
pub struct Table {
    /// Column definitions.
    pub columns: Vec<Column>,
    /// Row data (each row is a Vec of cell strings, one per column).
    pub rows: Vec<Vec<String>>,
    /// Index of the currently selected row (if any).
    pub selected_row: Option<usize>,
    /// Scroll offset (first visible row index).
    pub scroll_offset: usize,
    /// Whether to show the header row.
    pub show_header: bool,
    /// Optional sort state: (column index, direction).
    pub sort: Option<(usize, SortDirection)>,
    /// Whether the table is disabled.
    pub disabled: bool,
}

impl Table {
    /// Create a new table with the given columns.
    pub fn new(columns: Vec<Column>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            selected_row: None,
            scroll_offset: 0,
            show_header: true,
            sort: None,
            disabled: false,
        }
    }

    /// Add a row of data.
    pub fn add_row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    /// Select the next row.
    pub fn select_next(&mut self) {
        if self.disabled || self.rows.is_empty() {
            return;
        }
        self.selected_row = Some(
            self.selected_row
                .map(|i| (i + 1).min(self.rows.len() - 1))
                .unwrap_or(0),
        );
    }

    /// Select the previous row.
    pub fn select_prev(&mut self) {
        if self.disabled || self.rows.is_empty() {
            return;
        }
        self.selected_row = Some(self.selected_row.map(|i| i.saturating_sub(1)).unwrap_or(0));
    }

    /// Sort the rows by a column.
    pub fn sort_by_column(&mut self, col: usize) {
        if col >= self.columns.len() {
            return;
        }
        let dir = match self.sort {
            Some((c, SortDirection::Ascending)) if c == col => SortDirection::Descending,
            _ => SortDirection::Ascending,
        };
        self.sort = Some((col, dir));
        self.rows.sort_by(|a, b| {
            let ca = a.get(col).map(String::as_str).unwrap_or("");
            let cb = b.get(col).map(String::as_str).unwrap_or("");
            match dir {
                SortDirection::Ascending => ca.cmp(cb),
                SortDirection::Descending => cb.cmp(ca),
            }
        });
    }

    /// Number of visible rows that fit in the given height.
    fn visible_rows(&self, h: u32) -> usize {
        let header_h = if self.show_header { HEADER_HEIGHT } else { 0 };
        let body_h = h.saturating_sub(header_h);
        (body_h / ROW_HEIGHT) as usize
    }

    /// Ensure the selected row is visible by adjusting scroll offset.
    pub fn ensure_visible(&mut self, h: u32) {
        if let Some(sel) = self.selected_row {
            let visible = self.visible_rows(h);
            if visible == 0 {
                return;
            }
            if sel < self.scroll_offset {
                self.scroll_offset = sel;
            } else if sel >= self.scroll_offset + visible {
                self.scroll_offset = sel + 1 - visible;
            }
        }
    }
}

impl Widget for Table {
    fn measure(&self, _ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        let w: u32 = self.columns.iter().map(|c| c.width).sum();
        let w = w.max(available_w);
        let header_h = if self.show_header { HEADER_HEIGHT } else { 0 };
        let body_h = ROW_HEIGHT * self.rows.len() as u32;
        (w, header_h + body_h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let fs = ctx.theme.font_size_sm;
        let text_h = ctx.backend.measure_text_height(fs);
        let text_color = ctx.theme.interactive_text(self.disabled);
        let border = ctx.theme.border;

        // Compute column widths, distributing remaining space to zero-width columns.
        let fixed_total: u32 = self.columns.iter().map(|c| c.width).sum();
        let auto_count = self.columns.iter().filter(|c| c.width == 0).count() as u32;
        let auto_width = if auto_count > 0 && w > fixed_total {
            (w - fixed_total) / auto_count
        } else {
            0
        };

        let col_widths: Vec<u32> = self
            .columns
            .iter()
            .map(|c| if c.width == 0 { auto_width } else { c.width })
            .collect();

        let mut cy = y;

        // Header row.
        if self.show_header {
            let header_bg = ctx.theme.surface;
            ctx.backend.fill_rect(x, cy, w, HEADER_HEIGHT, header_bg)?;
            ctx.backend
                .stroke_rect(x, cy, w, HEADER_HEIGHT, 1, border)?;

            let mut cx = x;
            for (i, col) in self.columns.iter().enumerate() {
                let cw = col_widths[i];
                let sort_indicator = match self.sort {
                    Some((c, SortDirection::Ascending)) if c == i => " ^",
                    Some((c, SortDirection::Descending)) if c == i => " v",
                    _ => "",
                };
                let label = format!("{}{}", col.label, sort_indicator);
                let tx = cx + 4;
                let ty = cy + layout::center(HEADER_HEIGHT, text_h);
                ctx.backend.draw_text(&label, tx, ty, fs, text_color)?;
                cx += cw as i32;
            }
            cy += HEADER_HEIGHT as i32;
        }

        // Data rows.
        let visible = self.visible_rows(h);
        let end = (self.scroll_offset + visible).min(self.rows.len());
        for row_idx in self.scroll_offset..end {
            let row = &self.rows[row_idx];
            let is_selected = self.selected_row == Some(row_idx);

            if is_selected {
                let sel_bg = ctx.theme.interactive_accent(self.disabled);
                ctx.backend.fill_rect(x, cy, w, ROW_HEIGHT, sel_bg)?;
            }

            let mut cx = x;
            for (col_idx, cell) in row.iter().enumerate() {
                if col_idx >= col_widths.len() {
                    break;
                }
                let cw = col_widths[col_idx];
                let tx = cx + 4;
                let ty = cy + layout::center(ROW_HEIGHT, text_h);
                let fg = if is_selected {
                    ctx.theme.background
                } else {
                    text_color
                };
                ctx.backend.draw_text(cell, tx, ty, fs, fg)?;
                cx += cw as i32;
            }

            // Row separator line.
            ctx.backend
                .fill_rect(x, cy + ROW_HEIGHT as i32 - 1, w, 1, border)?;
            cy += ROW_HEIGHT as i32;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_table() -> Table {
        let cols = vec![
            Column::new("Name", 80),
            Column::new("Size", 60),
            Column::new("Type", 60),
        ];
        let mut t = Table::new(cols);
        t.add_row(vec!["file.txt".into(), "1024".into(), "text".into()]);
        t.add_row(vec!["img.png".into(), "2048".into(), "image".into()]);
        t.add_row(vec!["doc.pdf".into(), "512".into(), "pdf".into()]);
        t
    }

    #[test]
    fn new_defaults() {
        let t = sample_table();
        assert_eq!(t.columns.len(), 3);
        assert_eq!(t.rows.len(), 3);
        assert_eq!(t.selected_row, None);
        assert!(!t.disabled);
    }

    #[test]
    fn select_next_prev() {
        let mut t = sample_table();
        t.select_next();
        assert_eq!(t.selected_row, Some(0));
        t.select_next();
        assert_eq!(t.selected_row, Some(1));
        t.select_prev();
        assert_eq!(t.selected_row, Some(0));
        t.select_prev();
        assert_eq!(t.selected_row, Some(0)); // clamp at 0
    }

    #[test]
    fn select_next_clamps_at_end() {
        let mut t = sample_table();
        for _ in 0..10 {
            t.select_next();
        }
        assert_eq!(t.selected_row, Some(2));
    }

    #[test]
    fn select_disabled_noop() {
        let mut t = sample_table();
        t.disabled = true;
        t.select_next();
        assert_eq!(t.selected_row, None);
    }

    #[test]
    fn sort_ascending_then_descending() {
        let mut t = sample_table();
        t.sort_by_column(0);
        assert_eq!(t.sort, Some((0, SortDirection::Ascending)));
        assert_eq!(t.rows[0][0], "doc.pdf");

        t.sort_by_column(0);
        assert_eq!(t.sort, Some((0, SortDirection::Descending)));
        assert_eq!(t.rows[0][0], "img.png");
    }

    #[test]
    fn sort_out_of_bounds_noop() {
        let mut t = sample_table();
        t.sort_by_column(10);
        assert_eq!(t.sort, None);
    }

    #[test]
    fn ensure_visible_scrolls() {
        let mut t = sample_table();
        // Add more rows to trigger scrolling.
        for i in 0..20 {
            t.add_row(vec![format!("f{i}"), "0".into(), "x".into()]);
        }
        t.selected_row = Some(20);
        t.ensure_visible(60); // ~3 rows visible
        assert!(t.scroll_offset > 0);
    }

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn draw_shows_headers_and_data() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let t = sample_table();
            t.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert!(backend.has_text("Name"));
        assert!(backend.has_text("Size"));
        assert!(backend.has_text("file.txt"));
    }

    #[test]
    fn draw_no_header() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut t = sample_table();
            t.show_header = false;
            t.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        // Data should still render.
        assert!(backend.has_text("file.txt"));
    }

    #[test]
    fn draw_empty_no_panic() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let t = Table::new(vec![Column::new("A", 100)]);
            t.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
    }

    #[test]
    fn draw_all_themes_no_panic() {
        crate::test_utils::test_draw_all_themes(|ctx| {
            let t = sample_table();
            t.draw(ctx, 0, 0, 200, 100).unwrap();
        });
    }
}
