use oasis_sdi::SdiRegistry;
use oasis_skin::active_theme::ActiveTheme;
use oasis_types::backend::Color;

/// Number of 30-minute time columns visible in the grid.
pub(crate) const VISIBLE_TIME_SLOTS: usize = 5;

/// Duration of one time slot in seconds (30 minutes).
pub(crate) const SLOT_DURATION: u64 = 1800;

/// Number of channel rows visible at once (scrolls if more channels exist).
pub(crate) const VISIBLE_ROWS: usize = 5;

/// Maximum number of program cells per row.
pub(crate) const MAX_CELLS: usize = 8;

/// Tick mark positions for the volume bar (percentages).
pub(crate) const VOLUME_TICK_POSITIONS: &[u32] = &[0, 25, 50, 75, 100];

/// Width threshold (in pixels) at which the volume bar shows the long
/// "VOLUME: X%" label and the per-25%-tick marks. Below this threshold
/// the short "VOL X%" label is used and ticks are hidden.
pub(crate) const VOLUME_BAR_WIDE_THRESHOLD: u32 = 140;

/// Compute the volume bar's text label and the x-offset (in pixels)
/// from the bar's left edge at which the label should be drawn.
///
/// Returns the long form ("VOLUME: X%", offset 75) when the bar is at
/// least `VOLUME_BAR_WIDE_THRESHOLD` wide, otherwise the short form
/// ("VOL X%", offset 45). Shared between SDI and windowed render paths.
pub(crate) fn volume_bar_label(bar_w: u32, volume: u8) -> (String, i32) {
    if bar_w >= VOLUME_BAR_WIDE_THRESHOLD {
        (format!("VOLUME: {volume}%"), 75)
    } else {
        (format!("VOL {volume}%"), 45)
    }
}

/// TV Guide color palette, populated from the active theme.
///
/// Defaults match the retro cable-TV blue/amber EPG aesthetic. Skins can
/// override any color via `[app_themes.tv_guide]` in theme.toml.
#[derive(Debug, Clone)]
pub struct TvGuideColors {
    pub bg: Color,
    pub grid_line: Color,
    pub header_bg: Color,
    pub header_dark: Color,
    pub time_header_bg: Color,
    pub time_header: Color,
    pub channel_label: Color,
    pub program_text: Color,
    pub selected_bg: Color,
    pub selected_text: Color,
    /// Bright amber highlight drawn on the top/bottom edges of the
    /// selected row to create a glowing-bar effect.
    pub selected_glow: Color,
    pub dim_text: Color,
    pub playing_text: Color,
    pub cell_bg: Color,
    pub cell_border: Color,
    pub live_badge: Color,
    /// Text color drawn on top of the LIVE badge. Defaults to white;
    /// kept independent of `selected_text` (which is dark warm for the
    /// amber-on-row context).
    pub live_badge_text: Color,
    pub date_text: Color,
    pub footer_bg: Color,
    pub time_label: Color,
    pub glow_border: Color,
    pub glow_outer: Color,
    pub header_title: Color,
}

impl TvGuideColors {
    /// Build colors from the active theme, using app_color overrides.
    pub fn from_theme(at: &ActiveTheme) -> Self {
        let c = |key: &str, default: Color| -> Color {
            at.app_color("tv_guide", key).unwrap_or(default)
        };
        let d = Self::defaults();
        Self {
            bg: c("bg", d.bg),
            grid_line: c("grid_line", d.grid_line),
            header_bg: c("header_bg", d.header_bg),
            header_dark: c("header_dark", d.header_dark),
            time_header_bg: c("time_header_bg", d.time_header_bg),
            time_header: c("time_header", d.time_header),
            channel_label: c("channel_label", d.channel_label),
            program_text: c("program_text", d.program_text),
            selected_bg: c("selected_bg", d.selected_bg),
            selected_text: c("selected_text", d.selected_text),
            selected_glow: c("selected_glow", d.selected_glow),
            dim_text: c("dim_text", d.dim_text),
            playing_text: c("playing_text", d.playing_text),
            cell_bg: c("cell_bg", d.cell_bg),
            cell_border: c("cell_border", d.cell_border),
            live_badge: c("live_badge", d.live_badge),
            live_badge_text: c("live_badge_text", d.live_badge_text),
            date_text: c("date_text", d.date_text),
            footer_bg: c("footer_bg", d.footer_bg),
            time_label: c("time_label", d.time_label),
            glow_border: c("glow_border", d.glow_border),
            glow_outer: c("glow_outer", d.glow_outer),
            header_title: c("header_title", d.header_title),
        }
    }

    /// Build colors with hardcoded defaults (no theme overrides).
    pub fn defaults() -> Self {
        Self {
            bg: Color::rgba(8, 14, 30, 255),
            grid_line: Color::rgba(45, 90, 160, 255),
            header_bg: Color::rgba(8, 14, 30, 255),
            header_dark: Color::rgba(8, 14, 30, 255),
            time_header_bg: Color::rgba(30, 50, 90, 255),
            time_header: Color::rgba(255, 176, 50, 255),
            channel_label: Color::rgba(80, 190, 255, 255),
            program_text: Color::rgba(220, 230, 255, 255),
            selected_bg: Color::rgba(240, 165, 40, 255),
            selected_text: Color::rgba(30, 15, 0, 255),
            selected_glow: Color::rgba(255, 200, 80, 255),
            dim_text: Color::rgba(100, 130, 170, 255),
            playing_text: Color::rgba(80, 190, 255, 255),
            cell_bg: Color::rgba(16, 28, 55, 255),
            cell_border: Color::rgba(45, 90, 160, 255),
            live_badge: Color::rgba(255, 40, 40, 255),
            live_badge_text: Color::rgba(255, 255, 255, 255),
            date_text: Color::rgba(100, 130, 170, 255),
            footer_bg: Color::rgba(8, 14, 30, 255),
            time_label: Color::rgba(255, 176, 50, 255),
            glow_border: Color::rgba(45, 90, 160, 255),
            glow_outer: Color::rgba(30, 70, 130, 255),
            header_title: Color::rgba(255, 255, 255, 255),
        }
    }
}

/// Pre-computed SDI object name strings (avoids per-frame `format!()` calls).
pub(crate) struct SdiNames {
    pub(crate) time_cols: [String; VISIBLE_TIME_SLOTS],
    pub(crate) time_bgs: [String; VISIBLE_TIME_SLOTS],
    pub(crate) row_bgs: [String; VISIBLE_ROWS],
    pub(crate) row_labels: [String; VISIBLE_ROWS],
    pub(crate) row_lines: [String; VISIBLE_ROWS],
    pub(crate) row_cells: [[String; MAX_CELLS]; VISIBLE_ROWS],
    pub(crate) row_cell_bgs: [[String; MAX_CELLS]; VISIBLE_ROWS],
}

impl SdiNames {
    pub(crate) fn new() -> Self {
        Self {
            time_cols: std::array::from_fn(|col| format!("tv_time_{col}")),
            time_bgs: std::array::from_fn(|col| format!("tv_timebg_{col}")),
            row_bgs: std::array::from_fn(|row| format!("tv_row_{row}_bg")),
            row_labels: std::array::from_fn(|row| format!("tv_row_{row}_label")),
            row_lines: std::array::from_fn(|row| format!("tv_row_{row}_line")),
            row_cells: std::array::from_fn(|row| {
                std::array::from_fn(|ci| format!("tv_row_{row}_cell_{ci}"))
            }),
            row_cell_bgs: std::array::from_fn(|row| {
                std::array::from_fn(|ci| format!("tv_row_{row}_cbg_{ci}"))
            }),
        }
    }
}

/// Volume bar layout rectangle (x, y, w, h) relative to content origin.
pub(crate) struct VolumeBarRect {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: u32,
    pub(crate) h: u32,
}

/// Compute the volume bar position for the bottom-right overlay area.
///
/// The bar sits at the right side of the overlay strip shown when tuned.
/// Sized to match the retro cable-TV look: wide enough for tick marks
/// when the screen is large, but clamped down on small (PSP) layouts.
/// `x`/`y` are clamped to ≥ 0 so very small canvases (e.g. cw < 92) do
/// not produce a negative origin that pushes the bar off-screen.
pub(crate) fn volume_bar_rect(cw: u32, ch: u32, expanded: bool) -> VolumeBarRect {
    let bar_w = (cw / 3).clamp(80, 220);
    let bar_h = 12u32;
    let overlay_h = 20u32;
    let overlay_y = (ch as i32 - overlay_h as i32).max(0);
    let x = (cw as i32 - bar_w as i32 - 12).max(0);
    if expanded {
        VolumeBarRect {
            x,
            y: (overlay_y + (overlay_h as i32 - bar_h as i32) / 2).max(0),
            w: bar_w,
            h: bar_h,
        }
    } else {
        // PIP mode: put in the footer bar area.
        let footer_h = (ch * 5 / 100).max(14);
        let ftr_y = (ch as i32 - footer_h as i32).max(0);
        VolumeBarRect {
            x,
            y: (ftr_y + (footer_h as i32 - bar_h as i32) / 2).max(0),
            w: bar_w,
            h: bar_h,
        }
    }
}

/// Ensure an SDI object exists.
pub(crate) fn ensure_obj(sdi: &mut SdiRegistry, name: &str) {
    if !sdi.contains(name) {
        sdi.create(name);
    }
}

/// Truncate a title to fit within max characters.
pub(crate) fn truncate_title(title: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if title.len() <= max_chars {
        title.to_string()
    } else if max_chars <= 3 {
        title[..title.floor_char_boundary(max_chars)].to_string()
    } else {
        let boundary = title.floor_char_boundary(max_chars - 2);
        format!("{}..", &title[..boundary])
    }
}

/// Get the current Unix timestamp in seconds.
///
/// Honors `OASIS_FIXED_TIME` (`"YYYY-MM-DD HH:MM:SS"`), the same clock freeze
/// the platform `TimeService` uses. The guide's date header and schedule grid
/// are laid out from this timestamp, so without it a captured screenshot
/// changes every minute — and every scheduled slot changes at midnight.
pub(crate) fn current_unix_time() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(fixed) = fixed_unix_time() {
            return fixed;
        }
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Parse `OASIS_FIXED_TIME` into a Unix timestamp, if it is set and valid.
#[cfg(not(target_arch = "wasm32"))]
fn fixed_unix_time() -> Option<u64> {
    let raw = std::env::var("OASIS_FIXED_TIME").ok()?;
    parse_fixed_time(&raw)
}

/// Parse `"YYYY-MM-DD HH:MM:SS"` (or with a `T` separator) into Unix seconds.
#[cfg(not(target_arch = "wasm32"))]
fn parse_fixed_time(raw: &str) -> Option<u64> {
    let normalized = raw.replace('T', " ");
    let (date, time) = normalized.split_once(' ')?;
    let mut d = date.split('-');
    let mut t = time.split(':');
    let year: i64 = d.next()?.trim().parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    let hour: i64 = t.next()?.parse().ok()?;
    let minute: i64 = t.next()?.parse().ok()?;
    let second: i64 = t.next()?.trim().parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let secs = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second;
    u64::try_from(secs).ok()
}

/// Days since the Unix epoch for a proleptic-Gregorian date (Hinnant's
/// `days_from_civil`).
#[cfg(not(target_arch = "wasm32"))]
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    // -- fixed clock --

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn parse_fixed_time_matches_known_epochs() {
        assert_eq!(parse_fixed_time("1970-01-01 00:00:00"), Some(0));
        assert_eq!(parse_fixed_time("2000-01-01 00:00:00"), Some(946_684_800));
        // The screenshot harness's frozen timestamp.
        assert_eq!(parse_fixed_time("2025-06-15 12:00:00"), Some(1_749_988_800));
        // The 'T' separator is accepted too.
        assert_eq!(parse_fixed_time("2025-06-15T12:00:00"), Some(1_749_988_800));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn parse_fixed_time_rejects_garbage() {
        assert_eq!(parse_fixed_time(""), None);
        assert_eq!(parse_fixed_time("not-a-time"), None);
        assert_eq!(parse_fixed_time("2025-13-01 00:00:00"), None);
        assert_eq!(parse_fixed_time("2025-06-15 12:00"), None);
    }

    // -- truncate_title --

    #[test]
    fn truncate_title_empty_input() {
        assert_eq!(truncate_title("", 10), "");
    }

    #[test]
    fn truncate_title_exactly_three() {
        // max_chars == 3: still uses the truncation path (3 <= 3)
        assert_eq!(truncate_title("ABCDEF", 3), "ABC");
    }

    #[test]
    fn truncate_title_four_chars_with_dots() {
        // max_chars == 4 and title is longer: should show 2 chars + ".."
        assert_eq!(truncate_title("Hello World", 4), "He..");
    }

    #[test]
    fn truncate_title_unicode_boundary() {
        // Multi-byte chars: floor_char_boundary should not panic.
        let title = "\u{00e9}\u{00e9}\u{00e9}\u{00e9}\u{00e9}"; // "eeeee" (5 x 2-byte)
        let result = truncate_title(title, 4);
        // Should truncate safely without panicking.
        assert!(result.len() <= 6); // at most 2 chars (4 bytes) + ".." (2 bytes)
    }

    #[test]
    fn truncate_title_one_char() {
        assert_eq!(truncate_title("Hello", 1), "H");
    }

    // -- volume_bar_rect --

    #[test]
    fn volume_bar_rect_expanded_basic() {
        let r = volume_bar_rect(800, 600, true);
        // bar_w = (800/3).clamp(80,220) = 220
        assert_eq!(r.w, 220);
        assert_eq!(r.h, 12);
        // x = 800 - 220 - 12 = 568
        assert_eq!(r.x, 568);
        // overlay_y = 600 - 20 = 580, y = 580 + (20 - 12)/2 = 584
        assert_eq!(r.y, 584);
    }

    #[test]
    fn volume_bar_rect_collapsed_basic() {
        let r = volume_bar_rect(800, 600, false);
        assert_eq!(r.w, 220);
        assert_eq!(r.h, 12);
        // footer_h = (600*5/100).max(14) = 30
        // ftr_y = 600 - 30 = 570
        // y = 570 + (30-12)/2 = 579
        assert_eq!(r.y, 579);
    }

    #[test]
    fn volume_bar_rect_small_window() {
        let r = volume_bar_rect(200, 100, true);
        // bar_w = (200/3).clamp(80,220) = 80
        assert_eq!(r.w, 80);
        // x = 200 - 80 - 12 = 108
        assert_eq!(r.x, 108);
    }

    #[test]
    fn volume_bar_rect_tiny_height() {
        // Even with tiny height, should not panic.
        let r = volume_bar_rect(100, 10, false);
        assert!(r.w >= 80);
        assert_eq!(r.h, 12);
    }

    #[test]
    fn volume_bar_rect_narrow_canvas_clamps_x() {
        // cw < 92 would otherwise produce a negative x; the rect must
        // clamp x ≥ 0 so the bar stays on-screen.
        let r = volume_bar_rect(80, 100, true);
        assert_eq!(r.w, 80);
        assert!(r.x >= 0, "x must be non-negative on tiny canvases");
        assert!(r.y >= 0, "y must be non-negative on tiny canvases");
    }

    #[test]
    fn volume_bar_rect_tiny_canvas_clamps_y() {
        // ch smaller than overlay/footer height would produce negative y.
        let r = volume_bar_rect(200, 5, true);
        assert!(r.y >= 0, "y must be non-negative");
    }

    // -- volume_bar_label --

    #[test]
    fn volume_bar_label_long_form_at_threshold() {
        let (label, offset) = volume_bar_label(VOLUME_BAR_WIDE_THRESHOLD, 50);
        assert_eq!(label, "VOLUME: 50%");
        assert_eq!(offset, 75);
    }

    #[test]
    fn volume_bar_label_short_form_below_threshold() {
        let (label, offset) = volume_bar_label(VOLUME_BAR_WIDE_THRESHOLD - 1, 75);
        assert_eq!(label, "VOL 75%");
        assert_eq!(offset, 45);
    }

    #[test]
    fn volume_tick_positions_endpoints() {
        // The constant must start at 0 and end at 100 — the tick rendering
        // logic relies on this for the off-by-one fix on the 100% tick.
        assert_eq!(VOLUME_TICK_POSITIONS.first(), Some(&0));
        assert_eq!(VOLUME_TICK_POSITIONS.last(), Some(&100));
    }

    // -- TvGuideColors --

    #[test]
    fn tv_guide_colors_defaults_consistent() {
        let a = TvGuideColors::defaults();
        let b = TvGuideColors::defaults();
        assert_eq!(a.bg, b.bg);
        assert_eq!(a.selected_bg, b.selected_bg);
        assert_eq!(a.live_badge, b.live_badge);
    }

    #[test]
    fn tv_guide_colors_from_default_theme() {
        let at = ActiveTheme::default();
        let colors = TvGuideColors::from_theme(&at);
        let defaults = TvGuideColors::defaults();
        // With no app_color overrides, from_theme should match defaults.
        assert_eq!(colors.bg, defaults.bg);
        assert_eq!(colors.header_bg, defaults.header_bg);
        assert_eq!(colors.selected_bg, defaults.selected_bg);
    }

    // -- SdiNames --

    #[test]
    fn sdi_names_correct_format() {
        let names = SdiNames::new();
        assert_eq!(names.time_cols[0], "tv_time_0");
        assert_eq!(
            names.time_cols[VISIBLE_TIME_SLOTS - 1],
            format!("tv_time_{}", VISIBLE_TIME_SLOTS - 1)
        );
        assert_eq!(names.row_bgs[0], "tv_row_0_bg");
        assert_eq!(names.row_labels[2], "tv_row_2_label");
        assert_eq!(names.row_cells[1][3], "tv_row_1_cell_3");
        assert_eq!(names.row_cell_bgs[0][0], "tv_row_0_cbg_0");
    }

    #[test]
    fn sdi_names_array_lengths() {
        let names = SdiNames::new();
        assert_eq!(names.time_cols.len(), VISIBLE_TIME_SLOTS);
        assert_eq!(names.time_bgs.len(), VISIBLE_TIME_SLOTS);
        assert_eq!(names.row_bgs.len(), VISIBLE_ROWS);
        assert_eq!(names.row_labels.len(), VISIBLE_ROWS);
        assert_eq!(names.row_lines.len(), VISIBLE_ROWS);
        assert_eq!(names.row_cells.len(), VISIBLE_ROWS);
        assert_eq!(names.row_cells[0].len(), MAX_CELLS);
    }

    // -- ensure_obj --

    #[test]
    fn ensure_obj_creates_if_missing() {
        let mut sdi = SdiRegistry::new();
        assert!(!sdi.contains("test_obj"));
        ensure_obj(&mut sdi, "test_obj");
        assert!(sdi.contains("test_obj"));
    }

    #[test]
    fn ensure_obj_idempotent() {
        let mut sdi = SdiRegistry::new();
        ensure_obj(&mut sdi, "test_obj");
        ensure_obj(&mut sdi, "test_obj"); // should not panic
        assert!(sdi.contains("test_obj"));
    }

    // -- constants --

    #[test]
    fn constants_sensible() {
        assert_eq!(SLOT_DURATION, 1800); // 30 minutes
        assert!(VISIBLE_TIME_SLOTS >= 3);
        assert!(VISIBLE_ROWS >= 3);
        assert!(MAX_CELLS >= 4);
    }
}
