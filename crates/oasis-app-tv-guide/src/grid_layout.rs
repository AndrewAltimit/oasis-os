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

/// TV Guide color palette, populated from the active theme.
///
/// Defaults match the original retro CRT aesthetic. Skins can override
/// any color via `[app_themes.tv_guide]` in theme.toml.
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
    pub dim_text: Color,
    pub playing_text: Color,
    pub cell_bg: Color,
    pub cell_border: Color,
    pub live_badge: Color,
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
        Self {
            bg: c("bg", Color::rgba(10, 22, 40, 255)),
            grid_line: c("grid_line", Color::rgba(26, 58, 92, 255)),
            header_bg: c("header_bg", Color::rgba(12, 25, 50, 255)),
            header_dark: c("header_dark", Color::rgba(8, 18, 38, 255)),
            time_header_bg: c("time_header_bg", Color::rgba(15, 35, 65, 255)),
            time_header: c("time_header", Color::rgba(0, 204, 255, 255)),
            channel_label: c("channel_label", Color::rgba(200, 220, 240, 255)),
            program_text: c("program_text", Color::rgba(192, 216, 232, 255)),
            selected_bg: c("selected_bg", Color::rgba(255, 140, 0, 220)),
            selected_text: c("selected_text", Color::rgba(255, 255, 255, 255)),
            dim_text: c("dim_text", Color::rgba(100, 130, 160, 255)),
            playing_text: c("playing_text", Color::rgba(0, 221, 255, 255)),
            cell_bg: c("cell_bg", Color::rgba(15, 30, 55, 255)),
            cell_border: c("cell_border", Color::rgba(26, 58, 92, 255)),
            live_badge: c("live_badge", Color::rgba(220, 40, 40, 255)),
            date_text: c("date_text", Color::rgba(180, 200, 220, 255)),
            footer_bg: c("footer_bg", Color::rgba(12, 25, 45, 255)),
            time_label: c("time_label", Color::rgba(255, 160, 0, 255)),
            glow_border: c("glow_border", Color::rgba(60, 130, 200, 255)),
            glow_outer: c("glow_outer", Color::rgba(30, 70, 130, 180)),
            header_title: c("header_title", Color::rgba(220, 240, 255, 255)),
        }
    }

    /// Build colors with hardcoded defaults (no theme overrides).
    pub fn defaults() -> Self {
        Self {
            bg: Color::rgba(10, 22, 40, 255),
            grid_line: Color::rgba(26, 58, 92, 255),
            header_bg: Color::rgba(12, 25, 50, 255),
            header_dark: Color::rgba(8, 18, 38, 255),
            time_header_bg: Color::rgba(15, 35, 65, 255),
            time_header: Color::rgba(0, 204, 255, 255),
            channel_label: Color::rgba(200, 220, 240, 255),
            program_text: Color::rgba(192, 216, 232, 255),
            selected_bg: Color::rgba(255, 140, 0, 220),
            selected_text: Color::rgba(255, 255, 255, 255),
            dim_text: Color::rgba(100, 130, 160, 255),
            playing_text: Color::rgba(0, 221, 255, 255),
            cell_bg: Color::rgba(15, 30, 55, 255),
            cell_border: Color::rgba(26, 58, 92, 255),
            live_badge: Color::rgba(220, 40, 40, 255),
            date_text: Color::rgba(180, 200, 220, 255),
            footer_bg: Color::rgba(12, 25, 45, 255),
            time_label: Color::rgba(255, 160, 0, 255),
            glow_border: Color::rgba(60, 130, 200, 255),
            glow_outer: Color::rgba(30, 70, 130, 180),
            header_title: Color::rgba(220, 240, 255, 255),
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
pub(crate) fn volume_bar_rect(cw: u32, ch: u32, expanded: bool) -> VolumeBarRect {
    let bar_w = (cw / 4).clamp(60, 160);
    let bar_h = 10u32;
    let overlay_h = 20u32;
    let overlay_y = ch as i32 - overlay_h as i32;
    if expanded {
        VolumeBarRect {
            x: cw as i32 - bar_w as i32 - 8,
            y: overlay_y + (overlay_h as i32 - bar_h as i32) / 2,
            w: bar_w,
            h: bar_h,
        }
    } else {
        // PIP mode: put in the footer bar area.
        let footer_h = (ch * 5 / 100).max(14);
        let ftr_y = ch as i32 - footer_h as i32;
        VolumeBarRect {
            x: cw as i32 - bar_w as i32 - 8,
            y: ftr_y + (footer_h as i32 - bar_h as i32) / 2,
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
pub(crate) fn current_unix_time() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

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
        // bar_w = (800/4).clamp(60,160) = 160
        assert_eq!(r.w, 160);
        assert_eq!(r.h, 10);
        // x = 800 - 160 - 8 = 632
        assert_eq!(r.x, 632);
        // overlay_y = 600 - 20 = 580, y = 580 + (20 - 10)/2 = 585
        assert_eq!(r.y, 585);
    }

    #[test]
    fn volume_bar_rect_collapsed_basic() {
        let r = volume_bar_rect(800, 600, false);
        assert_eq!(r.w, 160);
        assert_eq!(r.h, 10);
        // footer_h = (600*5/100).max(14) = 30
        // ftr_y = 600 - 30 = 570
        // y = 570 + (30-10)/2 = 580
        assert_eq!(r.y, 580);
    }

    #[test]
    fn volume_bar_rect_small_window() {
        let r = volume_bar_rect(200, 100, true);
        // bar_w = (200/4).clamp(60,160) = 60
        assert_eq!(r.w, 60);
        // x = 200 - 60 - 8 = 132
        assert_eq!(r.x, 132);
    }

    #[test]
    fn volume_bar_rect_tiny_height() {
        // Even with tiny height, should not panic.
        let r = volume_bar_rect(100, 10, false);
        assert!(r.w >= 60);
        assert_eq!(r.h, 10);
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
