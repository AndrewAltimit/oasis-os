//! DatePicker widget: calendar grid with month/year navigation.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// Cell size for each day in the calendar grid.
const CELL_SIZE: u32 = 20;

/// Header height for month/year and day-of-week labels.
const HEADER_HEIGHT: u32 = 20;

/// Day-of-week label row height.
const DOW_HEIGHT: u32 = 16;

/// Day-of-week labels.
const DOW_LABELS: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

/// A calendar date picker.
pub struct DatePicker {
    /// Currently displayed year.
    pub year: u16,
    /// Currently displayed month (1-12).
    pub month: u8,
    /// Selected day of the month (1-based, 0 = none).
    pub selected_day: u8,
    /// Whether the picker is disabled.
    pub disabled: bool,
}

impl DatePicker {
    /// Create a new date picker for the given year/month.
    pub fn new(year: u16, month: u8) -> Self {
        let month = month.clamp(1, 12);
        Self {
            year,
            month,
            selected_day: 0,
            disabled: false,
        }
    }

    /// Select a day (clamped to valid range).
    pub fn select_day(&mut self, day: u8) {
        if !self.disabled {
            let max = days_in_month(self.year, self.month);
            self.selected_day = day.clamp(1, max);
        }
    }

    /// Navigate to the previous month.
    pub fn prev_month(&mut self) {
        if !self.disabled {
            if self.month == 1 {
                self.month = 12;
                self.year = self.year.saturating_sub(1);
            } else {
                self.month -= 1;
            }
            self.clamp_selected();
        }
    }

    /// Navigate to the next month.
    pub fn next_month(&mut self) {
        if !self.disabled {
            if self.month == 12 {
                self.month = 1;
                self.year = self.year.saturating_add(1);
            } else {
                self.month += 1;
            }
            self.clamp_selected();
        }
    }

    /// Clamp selected_day to valid range for current month.
    fn clamp_selected(&mut self) {
        if self.selected_day > 0 {
            let max = days_in_month(self.year, self.month);
            self.selected_day = self.selected_day.min(max);
        }
    }

    /// Header text like "January 2025".
    pub fn header_text(&self) -> String {
        let month_name = month_name(self.month);
        format!("{} {}", month_name, self.year)
    }
}

/// Return the number of days in a month.
fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        },
        _ => 30,
    }
}

/// Check if a year is a leap year.
fn is_leap_year(year: u16) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Day of week for the first day of a month (0=Sunday).
/// Uses Zeller's formula simplified for Gregorian calendar.
fn first_dow(year: u16, month: u8) -> u8 {
    // Tomohiko Sakamoto's algorithm.
    let t = [0i32, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 {
        year as i32 - 1
    } else {
        year as i32
    };
    let m = month as usize;
    ((y + y / 4 - y / 100 + y / 400 + t[m - 1] + 1) % 7) as u8
}

/// Month name from number.
fn month_name(m: u8) -> &'static str {
    match m {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "???",
    }
}

impl Widget for DatePicker {
    fn measure(&self, _ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        let w = CELL_SIZE * 7;
        // header + dow labels + max 6 rows of days
        let h = HEADER_HEIGHT + DOW_HEIGHT + CELL_SIZE * 6;
        (w, h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, _h: u32) -> Result<()> {
        let fs = ctx.theme.font_size_sm;
        let text_h = ctx.backend.measure_text_height(fs);
        let text_color = ctx.theme.interactive_text(self.disabled);
        let dim_color = ctx.theme.text_secondary;
        let border = ctx.theme.border;

        // Header: "< January 2025 >"
        let header = self.header_text();
        let header_w = ctx.backend.measure_text(&header, fs);
        let hx = x + layout::center(w, header_w);
        let hy = y + layout::center(HEADER_HEIGHT, text_h);
        ctx.backend.draw_text(&header, hx, hy, fs, text_color)?;

        // Navigation arrows.
        let arrow_y = y + layout::center(HEADER_HEIGHT, text_h);
        ctx.backend.draw_text("<", x + 4, arrow_y, fs, text_color)?;
        let right_arrow_x = x + w as i32 - 4 - ctx.backend.measure_text(">", fs) as i32;
        ctx.backend
            .draw_text(">", right_arrow_x, arrow_y, fs, text_color)?;

        // Day-of-week labels.
        let dow_y = y + HEADER_HEIGHT as i32;
        for (i, label) in DOW_LABELS.iter().enumerate() {
            let dx = x + (i as u32 * CELL_SIZE) as i32;
            let tx = dx + layout::center(CELL_SIZE, ctx.backend.measure_text(label, fs));
            let ty = dow_y + layout::center(DOW_HEIGHT, text_h);
            ctx.backend.draw_text(label, tx, ty, fs, dim_color)?;
        }

        // Separator.
        let sep_y = dow_y + DOW_HEIGHT as i32 - 1;
        ctx.backend.fill_rect(x, sep_y, w, 1, border)?;

        // Day grid.
        let days = days_in_month(self.year, self.month);
        let start_dow = first_dow(self.year, self.month) as u32;
        let grid_y = dow_y + DOW_HEIGHT as i32;

        for day in 1..=days {
            let cell_index = start_dow + (day - 1) as u32;
            let col = cell_index % 7;
            let row = cell_index / 7;
            let cx = x + (col * CELL_SIZE) as i32;
            let cy = grid_y + (row * CELL_SIZE) as i32;

            let is_selected = self.selected_day == day;
            if is_selected {
                let accent = ctx.theme.interactive_accent(self.disabled);
                ctx.backend
                    .fill_rect(cx, cy, CELL_SIZE, CELL_SIZE, accent)?;
            }

            let label = format!("{day}");
            let tx = cx + layout::center(CELL_SIZE, ctx.backend.measure_text(&label, fs));
            let ty = cy + layout::center(CELL_SIZE, text_h);
            let fg = if is_selected {
                ctx.theme.background
            } else {
                text_color
            };
            ctx.backend.draw_text(&label, tx, ty, fs, fg)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let dp = DatePicker::new(2025, 3);
        assert_eq!(dp.year, 2025);
        assert_eq!(dp.month, 3);
        assert_eq!(dp.selected_day, 0);
        assert!(!dp.disabled);
    }

    #[test]
    fn month_clamped() {
        let dp = DatePicker::new(2025, 15);
        assert_eq!(dp.month, 12);
    }

    #[test]
    fn select_day_clamps() {
        let mut dp = DatePicker::new(2025, 2);
        dp.select_day(30);
        assert_eq!(dp.selected_day, 28); // Feb 2025 non-leap
    }

    #[test]
    fn select_day_disabled_noop() {
        let mut dp = DatePicker::new(2025, 1);
        dp.disabled = true;
        dp.select_day(15);
        assert_eq!(dp.selected_day, 0);
    }

    #[test]
    fn next_month_wraps() {
        let mut dp = DatePicker::new(2025, 12);
        dp.next_month();
        assert_eq!(dp.month, 1);
        assert_eq!(dp.year, 2026);
    }

    #[test]
    fn prev_month_wraps() {
        let mut dp = DatePicker::new(2025, 1);
        dp.prev_month();
        assert_eq!(dp.month, 12);
        assert_eq!(dp.year, 2024);
    }

    #[test]
    fn prev_month_clamps_day() {
        let mut dp = DatePicker::new(2025, 3);
        dp.select_day(31);
        dp.prev_month(); // Feb has 28 days
        assert_eq!(dp.selected_day, 28);
    }

    #[test]
    fn leap_year_feb() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2025, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
    }

    #[test]
    fn header_text() {
        let dp = DatePicker::new(2025, 7);
        assert_eq!(dp.header_text(), "July 2025");
    }

    #[test]
    fn first_dow_known_dates() {
        // Jan 1, 2025 is a Wednesday (3).
        assert_eq!(first_dow(2025, 1), 3);
        // Mar 1, 2025 is a Saturday (6).
        assert_eq!(first_dow(2025, 3), 6);
    }

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn draw_shows_header_and_days() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let dp = DatePicker::new(2025, 3);
            dp.draw(&mut ctx, 0, 0, 140, 160).unwrap();
        }
        assert!(backend.has_text("March 2025"));
        assert!(backend.has_text("1"));
        assert!(backend.has_text("31"));
        assert!(backend.has_text("Su"));
    }

    #[test]
    fn draw_all_themes_no_panic() {
        crate::test_utils::test_draw_all_themes(|ctx| {
            let dp = DatePicker::new(2025, 6);
            dp.draw(ctx, 0, 0, 140, 160).unwrap();
        });
    }
}
