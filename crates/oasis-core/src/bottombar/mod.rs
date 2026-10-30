//! PSIX-style bottom bar -- footer with media category tabs and page dots.
//!
//! Occupies the bottom 24 pixels of the 480x272 screen (y=248-272).
//! Displays URL label, USB indicator, media category tabs, page dots,
//! and shoulder button hints.
//!
//! Two styles share this module's state and chrome (background strip,
//! XP-style clock, URL plate): the classic footer (`classic`) and the
//! decorative media dock (`dock`, `bottombar_style = "media_dock"`).
//! The media category tab strip lives in `tabs`.

mod classic;
mod dock;
mod tabs;

pub use tabs::MediaTab;

use oasis_types::bitmap_font::glyph_advance_scaled;

use crate::active_theme::ActiveTheme;
use crate::platform::SystemTime;
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::{
    BezelStyle, ensure_border, ensure_chrome_bezel, ensure_text, hide_bezel, hide_objects,
};
use crate::skin::SkinFeatures;
use classic::PAGE_DOT_NAMES;
use tabs::{BPIPE_NAMES, BTAB_NAMES};

/// Top edge of the bottom bar: `screen_h - bottombar_height`, saturated
/// so a skin whose bar is taller than the screen pins it at 0 instead of
/// overflowing (a panic in debug builds, a bar at y ~ 4 billion in
/// release).
pub fn bottom_bar_y(at: &ActiveTheme) -> i32 {
    i32::try_from(at.screen_h.saturating_sub(at.bottombar_height)).unwrap_or(i32::MAX)
}

/// Translation keys of the month names for date display (`date.month_N`).
const MONTH_KEYS: [&str; 12] = [
    "date.month_1",
    "date.month_2",
    "date.month_3",
    "date.month_4",
    "date.month_5",
    "date.month_6",
    "date.month_7",
    "date.month_8",
    "date.month_9",
    "date.month_10",
    "date.month_11",
    "date.month_12",
];

/// Measure the pixel width of a text string using proportional glyph metrics.
fn text_px(s: &str, font_size: u16) -> i32 {
    s.chars()
        .map(|c| glyph_advance_scaled(c, font_size) as i32)
        .sum()
}

/// Shared vertical metrics of the bar for the active skin.
#[derive(Debug, Clone, Copy)]
struct BarGeometry {
    /// Top edge of the bar ([`bottom_bar_y`]).
    y: i32,
    /// Bar height.
    h: u32,
    /// Baseline-free top of vertically centred small text.
    text_y: i32,
    /// Top of the chrome bezels (2px inset).
    bezel_y: i32,
    /// Height of the chrome bezels.
    bezel_h: u32,
}

impl BarGeometry {
    fn new(at: &ActiveTheme) -> Self {
        let y = bottom_bar_y(at);
        let h = at.bottombar_height;
        Self {
            y,
            h,
            text_y: y + (h as i32 - at.font_small as i32) / 2,
            bezel_y: y + 2,
            bezel_h: h.saturating_sub(4),
        }
    }
}

/// Ensure a bar text label exists, then place it at (`x`, `y`) with `text`
/// and the skin's text shadow.
///
/// Position is written every frame (not only on creation) because it
/// depends on the skin and on neighbouring labels: a skin swap that
/// changes the URL text or the bar style must move the labels after it.
fn place_label(
    sdi: &mut SdiRegistry,
    at: &ActiveTheme,
    name: &str,
    (x, y): (i32, i32),
    text: &str,
    color: crate::backend::Color,
) {
    ensure_text(sdi, name, x, y, at.font_small, color);
    if let Ok(obj) = sdi.get_mut(name) {
        obj.x = x;
        obj.y = y;
        obj.set_text(text);
        obj.visible = true;
        if at.bar.text_shadow {
            obj.text_shadow_offset = Some((1, 1));
            obj.text_shadow_color = Some(at.bar.text_shadow_color);
        }
    }
}

/// Runtime state for the bottom bar.
#[derive(Debug)]
pub struct BottomBar {
    /// Currently selected media tab.
    pub active_tab: MediaTab,
    /// Current dashboard page (0-based).
    pub current_page: usize,
    /// Total number of dashboard pages.
    pub total_pages: usize,
    /// Whether L trigger is visually pressed.
    pub l_pressed: bool,
    /// Whether R trigger is visually pressed.
    pub r_pressed: bool,
    /// Smooth visual page position (lerps toward current_page).
    pub dot_visual_page: f32,
    /// Cached clock string (for XP-style bottom-right clock).
    clock_text: String,
    /// Cached date string (for XP-style bottom-right clock).
    date_text: String,
    /// Cached merged date+clock display string (built once per
    /// `update_info` instead of once per frame).
    clock_display: String,
}

impl BottomBar {
    /// Create a new bottom bar.
    pub fn new() -> Self {
        Self {
            active_tab: MediaTab::None,
            current_page: 0,
            total_pages: 1,
            l_pressed: false,
            r_pressed: false,
            dot_visual_page: 0.0,
            clock_text: "00:00".to_string(),
            date_text: String::new(),
            clock_display: "00:00".to_string(),
        }
    }

    /// Update cached clock/date strings used by the XP-style bottom-right clock.
    pub fn update_info(&mut self, time: Option<&SystemTime>) {
        if let Some(t) = time {
            self.clock_text = format!("{:02}:{:02}", t.hour, t.minute);
            let month_name = if t.month >= 1 && t.month <= 12 {
                oasis_i18n::tr!(MONTH_KEYS[(t.month - 1) as usize])
            } else {
                "???"
            };
            // Locale-specific order, e.g. "May 4, 2026" / "4. Mai 2026".
            self.date_text = oasis_i18n::tr!(
                "date.format",
                month = month_name,
                day = t.day,
                year = t.year,
            );
            self.clock_display = format!("{} {}", self.date_text, self.clock_text);
        }
    }

    /// Advance page dot lerp animation by one frame.
    pub fn tick_animation(&mut self, at: &ActiveTheme) {
        self.dot_visual_page +=
            (self.current_page as f32 - self.dot_visual_page) * at.page_dot_lerp_speed;
    }

    /// Cycle to the next media tab.
    pub fn next_tab(&mut self) {
        self.active_tab = self.active_tab.next();
    }

    /// Synchronize SDI objects to reflect current bottom bar state.
    ///
    /// Accepts an `ActiveTheme` for skin-driven colors and `SkinFeatures`
    /// for content visibility toggles. Pass `&ActiveTheme::default()` and
    /// `&SkinFeatures::default()` for legacy behaviour.
    pub fn update_sdi(&self, sdi: &mut SdiRegistry, at: &ActiveTheme, features: &SkinFeatures) {
        let geo = BarGeometry::new(at);
        sync_background(sdi, at, &geo);
        // Legacy "R>" shoulder hint -- always hidden (kept for skins that may
        // still reference the SDI object by name).
        hide_objects(sdi, &["bar_r_hint"]);
        // Opt-in decorative media-dock style; the classic footer otherwise.
        if features.bottombar_style == "media_dock" {
            self.update_sdi_dock(sdi, at, features, &geo);
        } else {
            self.update_sdi_classic(sdi, at, features, &geo);
        }
    }

    /// Bottom-right clock+date (XP-style, when `clock_in_bottombar`).
    /// Returns the x the right-aligned content left of it must end before.
    fn sync_clock(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        features: &SkinFeatures,
        geo: &BarGeometry,
    ) -> i32 {
        if !features.clock_in_bottombar {
            hide_objects(sdi, &["bar_bottom_clock"]);
            return at.screen_w as i32;
        }
        let cx = at.screen_w as i32 - text_px(&self.clock_display, at.font_small) - 8;
        // Skins with shaped bar art can nudge the clock into a shelf that
        // sits below the bar's midline (clock_offset_y).
        let clock_y = geo.text_y + at.bar.clock_offset_y;
        place_label(
            sdi,
            at,
            "bar_bottom_clock",
            (cx, clock_y),
            &self.clock_display,
            at.bar.clock_color,
        );
        cx
    }

    /// Hide all bottom bar SDI objects.
    pub fn hide_sdi(sdi: &mut SdiRegistry) {
        hide_objects(
            sdi,
            &[
                "bar_bottom",
                "bar_bottom_line",
                "bar_bottom_clock",
                "bar_url",
                "bar_usb",
                "bar_r_hint",
            ],
        );
        hide_bezel(sdi, "bar_url_bezel");
        hide_bezel(sdi, "bar_tab_bezel");
        hide_objects(sdi, &BTAB_NAMES);
        hide_objects(sdi, &BPIPE_NAMES);
        hide_objects(sdi, &PAGE_DOT_NAMES);
        dock::hide_dock(sdi);
    }
}

impl Default for BottomBar {
    fn default() -> Self {
        Self::new()
    }
}

/// Background strip + the separator line along its top edge.
fn sync_background(sdi: &mut SdiRegistry, at: &ActiveTheme, geo: &BarGeometry) {
    if !sdi.contains("bar_bottom") {
        let obj = sdi.create("bar_bottom");
        obj.overlay = true;
        obj.z = 900;
    }
    if let Ok(obj) = sdi.get_mut("bar_bottom") {
        obj.x = 0;
        obj.y = geo.y;
        obj.w = at.screen_w;
        obj.h = geo.h;
        obj.color = at.bar.bg;
        obj.visible = true;
        obj.gradient_top = at.bar.gradient_top;
        obj.gradient_bottom = at.bar.gradient_bottom;
    }
    ensure_border(
        sdi,
        "bar_bottom_line",
        0,
        geo.y,
        at.screen_w,
        1,
        at.bar.separator_color,
    );
}

/// Left inset of the footer content: room for the start button when the
/// skin has a start menu.
fn url_offset(at: &ActiveTheme, features: &SkinFeatures) -> i32 {
    if features.start_menu {
        at.menu.button_width as i32 + 10
    } else {
        0
    }
}

/// URL label + chrome bezel (PSIX site plate) at the left edge.
///
/// Returns the label's right edge, or `None` (and hides the plate) when
/// the skin has no `bar_url_text`.
fn sync_url_plate(
    sdi: &mut SdiRegistry,
    at: &ActiveTheme,
    url_offset: i32,
    geo: &BarGeometry,
) -> Option<i32> {
    if at.bar.url_text.is_empty() {
        hide_objects(sdi, &["bar_url"]);
        hide_bezel(sdi, "bar_url_bezel");
        return None;
    }
    let x = 8 + url_offset;
    let end = x + text_px(&at.bar.url_text, at.font_small);
    place_label(
        sdi,
        at,
        "bar_url",
        (x, geo.text_y),
        &at.bar.url_text,
        at.bar.url_color,
    );
    // Chrome bezel around the URL area (sized to the actual text width).
    let bx = 2 + url_offset;
    let bw = (end + 6 - bx).max(60) as u32;
    ensure_chrome_bezel(
        sdi,
        "bar_url_bezel",
        bx,
        geo.bezel_y,
        bw,
        geo.bezel_h,
        &BezelStyle::chrome(),
    );
    Some(end)
}

#[cfg(test)]
mod tests;
