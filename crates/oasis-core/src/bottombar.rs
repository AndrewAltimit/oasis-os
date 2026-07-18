//! PSIX-style bottom bar -- footer with media category tabs and page dots.
//!
//! Occupies the bottom 24 pixels of the 480x272 screen (y=248-272).
//! Displays URL label, USB indicator, media category tabs, page dots,
//! and shoulder button hints.

use oasis_types::bitmap_font::glyph_advance_scaled;
use oasis_types::color::lerp_color;

use crate::active_theme::ActiveTheme;
use crate::platform::SystemTime;
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::{
    BezelStyle, ensure_border, ensure_chrome_bezel, ensure_pill, ensure_rounded_fill, ensure_text,
    hide_bezel, hide_indexed, hide_objects,
};
use crate::theme;

/// Month names for date display (matches statusbar formatting).
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Pre-built SDI object names (D7: avoids per-frame `format!` allocations).
/// Indices match `MediaTab::TABS`; pipes sit between adjacent tabs.
const BTAB_NAMES: [&str; 4] = ["bar_btab_0", "bar_btab_1", "bar_btab_2", "bar_btab_3"];
const BPIPE_NAMES: [&str; 3] = ["bar_bpipe_0", "bar_bpipe_1", "bar_bpipe_2"];
/// Page dot names; length matches `theme::MAX_PAGE_DOTS`.
const PAGE_DOT_NAMES: [&str; 4] = ["bar_page_0", "bar_page_1", "bar_page_2", "bar_page_3"];
const _: () = assert!(PAGE_DOT_NAMES.len() == theme::MAX_PAGE_DOTS);
const _: () = assert!(BTAB_NAMES.len() == MediaTab::TABS.len());

/// Media-dock transport pill names (`bottombar_style = "media_dock"`).
const DOCK_BTN_NAMES: [&str; 3] = ["bar_dock_prev", "bar_dock_play", "bar_dock_next"];
/// Media-dock progress/volume track + fill names.
const DOCK_TRACK_NAMES: [&str; 4] = [
    "bar_dock_progress_track",
    "bar_dock_progress_fill",
    "bar_dock_vol_track",
    "bar_dock_vol_fill",
];
/// Scanline-rect prefixes for the transport-glyph triangles. Each hosts up
/// to `DOCK_GLYPH_ROWS` 1px scanlines (`{prefix}0`, `{prefix}1`, …): prev is
/// two left triangles, play one right triangle, next two right triangles.
const DOCK_GLYPH_PREFIXES: [&str; 5] = [
    "bar_dock_prev_a_",
    "bar_dock_prev_b_",
    "bar_dock_play_g_",
    "bar_dock_next_a_",
    "bar_dock_next_b_",
];
/// Max scanlines reserved per transport-glyph triangle.
const DOCK_GLYPH_ROWS: usize = 9;

/// Hide every media-dock SDI object (transport pills, glyph scanlines, and
/// progress/volume tracks). A no-op when the objects don't exist, so it is
/// safe to call from the classic footer path.
fn hide_dock(sdi: &mut SdiRegistry) {
    hide_objects(sdi, &DOCK_BTN_NAMES);
    hide_objects(sdi, &DOCK_TRACK_NAMES);
    for prefix in DOCK_GLYPH_PREFIXES {
        hide_indexed(sdi, prefix, DOCK_GLYPH_ROWS);
    }
}

/// Draw one transport-glyph triangle as a stack of 1px scanline rects.
///
/// `rows` scanlines form a triangle whose apex points right (`point_right`)
/// or left. The widest scanline sits at the vertical center. Unused
/// scanlines up to `DOCK_GLYPH_ROWS` are left hidden by the caller.
fn dock_triangle(
    sdi: &mut SdiRegistry,
    prefix: &str,
    gx: i32,
    gy: i32,
    rows: usize,
    point_right: bool,
    color: oasis_types::backend::Color,
) {
    let half = (rows as i32 - 1) / 2;
    let max_w = half + 1;
    for r in 0..rows {
        let dc = (r as i32 - half).abs();
        let w = (max_w - dc).max(1);
        let x = if point_right { gx } else { gx + (max_w - w) };
        let name = format!("{prefix}{r}");
        ensure_border(sdi, &name, x, gy + r as i32, w as u32, 1, color);
        if let Ok(obj) = sdi.get_mut(&name) {
            obj.z = 902; // above the pill fill (901)
        }
    }
}

/// Measure the pixel width of a text string using proportional glyph metrics.
fn text_px(s: &str, font_size: u16) -> i32 {
    s.chars()
        .map(|c| glyph_advance_scaled(c, font_size) as i32)
        .sum()
}

/// Media category tabs (cycled with R trigger).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaTab {
    /// No media tab selected -- dashboard is visible.
    None,
    /// Audio player page.
    Audio,
    /// Video player page.
    Video,
    /// Image viewer page.
    Image,
    /// File browser page.
    File,
}

impl MediaTab {
    /// Cycle to the next tab.
    pub fn next(self) -> Self {
        match self {
            Self::None => Self::Audio,
            Self::Audio => Self::Video,
            Self::Video => Self::Image,
            Self::Image => Self::File,
            Self::File => Self::None,
        }
    }

    /// Display label for the tab.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Audio => "AUDIO",
            Self::Video => "VIDEO",
            Self::Image => "IMAGE",
            Self::File => "FILE",
        }
    }

    /// All selectable tabs in order (excluding None).
    pub const TABS: &[MediaTab] = &[
        MediaTab::Audio,
        MediaTab::Video,
        MediaTab::Image,
        MediaTab::File,
    ];
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
    /// Cached merged date+clock display string (D7: built once per
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
                MONTHS[(t.month - 1) as usize]
            } else {
                "???"
            };
            self.date_text = format!("{month_name} {}, {}", t.day, t.year);
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
    pub fn update_sdi(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        features: &crate::skin::SkinFeatures,
    ) {
        // Opt-in decorative media-dock style. Branch early so the classic
        // footer path below is untouched when the style is unset.
        if features.bottombar_style == "media_dock" {
            self.update_sdi_dock(sdi, at, features);
            return;
        }
        let bar_y = (at.screen_h - at.bottombar_height) as i32;
        let bar_h = at.bottombar_height;
        let font_small = at.font_small;
        let screen_w = at.screen_w;
        // Vertically center text within the bar.
        let text_y = bar_y + (bar_h as i32 - font_small as i32) / 2;

        // Semi-transparent background bar.
        if !sdi.contains("bar_bottom") {
            let obj = sdi.create("bar_bottom");
            obj.x = 0;
            obj.y = bar_y;
            obj.w = screen_w;
            obj.h = bar_h;
            obj.color = at.bar.bg;
            obj.overlay = true;
            obj.z = 900;
        }
        if let Ok(obj) = sdi.get_mut("bar_bottom") {
            obj.color = at.bar.bg;
            obj.y = bar_y;
            obj.h = bar_h;
            obj.visible = true;
            obj.gradient_top = at.bar.gradient_top;
            obj.gradient_bottom = at.bar.gradient_bottom;
        }

        // Thin separator line at top of bottom bar.
        ensure_border(
            sdi,
            "bar_bottom_line",
            0,
            bar_y,
            screen_w,
            1,
            at.bar.separator_color,
        );

        // URL label + chrome bezel (only shown when bar_url_text is non-empty).
        let url_offset = if features.start_menu {
            at.menu.button_width as i32 + 10
        } else {
            0
        };
        let bz_y = bar_y + 2;
        let bz_h = bar_h.saturating_sub(4);
        let url_text_end = if at.bar.url_text.is_empty() {
            // No URL text -- hide URL label and bezel.
            if let Ok(obj) = sdi.get_mut("bar_url") {
                obj.visible = false;
            }
            hide_bezel(sdi, "bar_url_bezel");
            url_offset
        } else {
            let end = 8 + url_offset + text_px(&at.bar.url_text, font_small);
            ensure_text(
                sdi,
                "bar_url",
                8 + url_offset,
                text_y,
                font_small,
                at.bar.url_color,
            );
            if let Ok(obj) = sdi.get_mut("bar_url") {
                obj.set_text(&at.bar.url_text);
                if at.bar.text_shadow {
                    obj.text_shadow_offset = Some((1, 1));
                    obj.text_shadow_color = Some(at.bar.text_shadow_color);
                }
            }

            // Chrome bezel around URL area (sized to actual text width).
            let url_bx = 2i32 + url_offset;
            let url_bw = (end + 6 - url_bx).max(60) as u32;
            ensure_chrome_bezel(
                sdi,
                "bar_url_bezel",
                url_bx,
                bz_y,
                url_bw,
                bz_h,
                &BezelStyle::chrome(),
            );
            end
        };

        // Bottom-right clock+date (XP-style, when enabled).
        let right_edge = if features.clock_in_bottombar {
            let clock_w = text_px(&self.clock_display, font_small);
            let cx = screen_w as i32 - clock_w - 8;
            ensure_text(
                sdi,
                "bar_bottom_clock",
                cx,
                text_y,
                font_small,
                at.bar.clock_color,
            );
            if let Ok(obj) = sdi.get_mut("bar_bottom_clock") {
                obj.set_text(&self.clock_display);
                obj.visible = true;
                if at.bar.text_shadow {
                    obj.text_shadow_offset = Some((1, 1));
                    obj.text_shadow_color = Some(at.bar.text_shadow_color);
                }
            }
            cx
        } else {
            if let Ok(obj) = sdi.get_mut("bar_bottom_clock") {
                obj.visible = false;
            }
            screen_w as i32
        };

        // Media category tabs (pipe-separated). Disabled by default; opt in
        // per-skin via `show_media_tabs = true` for PSP-style layouts.
        if features.show_media_tabs {
            self.draw_media_tabs(sdi, at, right_edge);
        } else {
            // Hide media tab objects when disabled.
            hide_objects(sdi, &BTAB_NAMES);
            hide_objects(sdi, &BPIPE_NAMES);
            hide_bezel(sdi, "bar_tab_bezel");
        }

        // Legacy "R>" shoulder hint -- always hidden (kept for skins that may
        // still reference the SDI object by name).
        if let Ok(obj) = sdi.get_mut("bar_r_hint") {
            obj.visible = false;
        }

        // Classic footer never shows the media dock; hide any dock objects
        // left over from a prior dock skin (no-op when they don't exist).
        hide_dock(sdi);

        // USB indicator (after URL text -- hidden when URL is empty).
        let usb_end = if at.bar.url_text.is_empty() {
            if let Ok(obj) = sdi.get_mut("bar_usb") {
                obj.visible = false;
            }
            url_offset
        } else {
            let usb_x = url_text_end + 6;
            ensure_text(sdi, "bar_usb", usb_x, text_y, font_small, at.bar.usb_color);
            if let Ok(obj) = sdi.get_mut("bar_usb") {
                obj.set_text("USB");
                if at.bar.text_shadow {
                    obj.text_shadow_offset = Some((1, 1));
                    obj.text_shadow_color = Some(at.bar.text_shadow_color);
                }
            }
            usb_x + text_px("USB", font_small)
        };

        // Page dots (rounded for circular appearance, with lerp transition).
        // Only render when there's actual pagination to indicate — a single
        // lone "active" dot otherwise shows up next to the START button as
        // an unexplained white speck on skins that inherit the default
        // `page_dot_active` color.
        let show_dots = features.show_page_dots && self.total_pages > 1;
        if show_dots {
            let dots_x = usb_end + 12;
            let max_dots = theme::MAX_PAGE_DOTS;
            for (i, name) in PAGE_DOT_NAMES
                .iter()
                .enumerate()
                .take(self.total_pages.min(max_dots))
            {
                // Proximity: 1.0 when this dot is the visual page, 0.0 when far.
                let proximity = (1.0 - (i as f32 - self.dot_visual_page).abs()).max(0.0);
                let dot_color =
                    lerp_color(at.bar.page_dot_inactive, at.bar.page_dot_active, proximity);
                ensure_rounded_fill(
                    sdi,
                    name,
                    dots_x + (i as i32) * 12,
                    bar_y + (bar_h as i32 - 6) / 2,
                    6,
                    6,
                    dot_color,
                    3,
                );
            }
            hide_objects(sdi, &PAGE_DOT_NAMES[self.total_pages.min(max_dots)..]);
        } else {
            hide_objects(sdi, &PAGE_DOT_NAMES);
        }
    }

    /// Draw the right-aligned media category tab strip (AUDIO/VIDEO/IMAGE/
    /// FILE) ending just left of `right_edge`, and return the left x of the
    /// tab group. Shared by the classic footer and the media dock.
    fn draw_media_tabs(&self, sdi: &mut SdiRegistry, at: &ActiveTheme, right_edge: i32) -> i32 {
        let font_small = at.font_small;
        let bar_y = (at.screen_h - at.bottombar_height) as i32;
        let bar_h = at.bottombar_height;
        let text_y = bar_y + (bar_h as i32 - font_small as i32) / 2;
        let bz_y = bar_y + 2;
        let bz_h = bar_h.saturating_sub(4);

        let tab_labels: Vec<&str> = MediaTab::TABS.iter().map(|t| t.label()).collect();
        let labels_w: i32 = tab_labels.iter().map(|l| text_px(l, font_small)).sum();
        let pipe_w = text_px("|", font_small);
        let pipes_w = (tab_labels.len() as i32 - 1) * (at.pipe_gap * 2 + pipe_w);
        let total_w = labels_w + pipes_w;
        let tabs_x = right_edge - total_w - 8;

        // Chrome bezel around tab group.
        let tab_bx = tabs_x - 6;
        let tab_bw = (total_w + 14) as u32;
        ensure_chrome_bezel(
            sdi,
            "bar_tab_bezel",
            tab_bx,
            bz_y,
            tab_bw,
            bz_h,
            &BezelStyle::chrome(),
        );

        let mut cx = tabs_x;
        for (i, tab) in MediaTab::TABS.iter().enumerate() {
            let label = tab.label();
            let name = BTAB_NAMES[i];

            let color = if *tab == self.active_tab {
                at.bar.media_tab_active
            } else {
                at.bar.media_tab_inactive
            };
            ensure_text(sdi, name, cx, text_y, font_small, color);
            if let Ok(obj) = sdi.get_mut(name) {
                obj.set_text(label);
                obj.text_color = color;
                obj.visible = true;
                if at.bar.text_shadow {
                    obj.text_shadow_offset = Some((1, 1));
                    obj.text_shadow_color = Some(at.bar.text_shadow_color);
                }
            }
            cx += text_px(label, font_small);

            // Pipe separator (except after last tab).
            if i < MediaTab::TABS.len() - 1 {
                cx += at.pipe_gap;
                let pipe_name = BPIPE_NAMES[i];
                ensure_text(sdi, pipe_name, cx, text_y, font_small, at.bar.pipe_color);
                if let Ok(obj) = sdi.get_mut(pipe_name) {
                    obj.set_text("|");
                }
                cx += pipe_w + at.pipe_gap;
            }
        }
        tabs_x
    }

    /// Decorative PSIX-style media dock (`bottombar_style = "media_dock"`).
    ///
    /// Draws transport pills (prev / play / next) with primitive triangle
    /// glyphs, a progress track + fill, a volume track + fill, and keeps the
    /// right-aligned media tab strip + USB indicator + clock. Progress and
    /// volume use static placeholder levels — the dock carries no audio
    /// binding.
    fn update_sdi_dock(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        features: &crate::skin::SkinFeatures,
    ) {
        let bar_y = (at.screen_h - at.bottombar_height) as i32;
        let bar_h = at.bottombar_height;
        let font_small = at.font_small;
        let screen_w = at.screen_w;
        let text_y = bar_y + (bar_h as i32 - font_small as i32) / 2;

        // Background bar + separator (same chrome as the classic footer).
        if !sdi.contains("bar_bottom") {
            let obj = sdi.create("bar_bottom");
            obj.overlay = true;
            obj.z = 900;
        }
        if let Ok(obj) = sdi.get_mut("bar_bottom") {
            obj.x = 0;
            obj.y = bar_y;
            obj.w = screen_w;
            obj.h = bar_h;
            obj.color = at.bar.bg;
            obj.visible = true;
            obj.gradient_top = at.bar.gradient_top;
            obj.gradient_bottom = at.bar.gradient_bottom;
        }
        ensure_border(
            sdi,
            "bar_bottom_line",
            0,
            bar_y,
            screen_w,
            1,
            at.bar.separator_color,
        );

        // Classic-only elements the dock omits (the URL plate is kept:
        // PSIX's footer leads with its site plate at the left edge).
        hide_objects(sdi, &["bar_r_hint"]);
        hide_objects(sdi, &PAGE_DOT_NAMES);

        // Right side: clock (optional), then media tabs, then USB.
        let right_edge = if features.clock_in_bottombar {
            let clock_w = text_px(&self.clock_display, font_small);
            let cx = screen_w as i32 - clock_w - 8;
            ensure_text(
                sdi,
                "bar_bottom_clock",
                cx,
                text_y,
                font_small,
                at.bar.clock_color,
            );
            if let Ok(obj) = sdi.get_mut("bar_bottom_clock") {
                obj.set_text(&self.clock_display);
                obj.visible = true;
                if at.bar.text_shadow {
                    obj.text_shadow_offset = Some((1, 1));
                    obj.text_shadow_color = Some(at.bar.text_shadow_color);
                }
            }
            cx
        } else {
            if let Ok(obj) = sdi.get_mut("bar_bottom_clock") {
                obj.visible = false;
            }
            screen_w as i32
        };

        // Media tab strip is forced on in dock mode (regardless of
        // `show_media_tabs`) so the dock always carries the AUDIO/VIDEO/
        // IMAGE/FILE selector. The classic chrome bezel is hidden: dock
        // skins bake their own tab-shelf chrome into the bar texture.
        let tabs_x = self.draw_media_tabs(sdi, at, right_edge);
        hide_bezel(sdi, "bar_tab_bezel");

        // USB indicator to the left of the tab strip.
        let usb_w = text_px("USB", font_small);
        let usb_x = tabs_x - 8 - usb_w;
        ensure_text(sdi, "bar_usb", usb_x, text_y, font_small, at.bar.usb_color);
        if let Ok(obj) = sdi.get_mut("bar_usb") {
            obj.set_text("USB");
            obj.visible = true;
            if at.bar.text_shadow {
                obj.text_shadow_offset = Some((1, 1));
                obj.text_shadow_color = Some(at.bar.text_shadow_color);
            }
        }

        // Left side: URL plate (PSIX site plate) followed by transport
        // pills. The plate reuses the classic footer's objects so switching
        // bottombar styles never leaks a stale copy.
        let url_offset = if features.start_menu {
            at.menu.button_width as i32 + 10
        } else {
            0
        };
        let bz_y = bar_y + 2;
        let bz_h = bar_h.saturating_sub(4);
        let left = if at.bar.url_text.is_empty() {
            if let Ok(obj) = sdi.get_mut("bar_url") {
                obj.visible = false;
            }
            hide_bezel(sdi, "bar_url_bezel");
            url_offset + 8
        } else {
            let end = 8 + url_offset + text_px(&at.bar.url_text, font_small);
            ensure_text(
                sdi,
                "bar_url",
                8 + url_offset,
                text_y,
                font_small,
                at.bar.url_color,
            );
            if let Ok(obj) = sdi.get_mut("bar_url") {
                obj.set_text(&at.bar.url_text);
                obj.visible = true;
                if at.bar.text_shadow {
                    obj.text_shadow_offset = Some((1, 1));
                    obj.text_shadow_color = Some(at.bar.text_shadow_color);
                }
            }
            let url_bx = 2i32 + url_offset;
            let url_bw = (end + 6 - url_bx).max(60) as u32;
            ensure_chrome_bezel(
                sdi,
                "bar_url_bezel",
                url_bx,
                bz_y,
                url_bw,
                bz_h,
                &BezelStyle::chrome(),
            );
            end + 6
        };
        // Compact transport pills (~1/3 bar height): PSIX's transports are
        // small chrome squares, not full-height buttons.
        let pill_h = (bar_h as i32 / 3).max(10);
        let pill_w = pill_h + 6;
        let gap = 4;
        let by = bar_y + (bar_h as i32 - pill_h) / 2;
        // Glyph triangle geometry (odd row count for a symmetric apex).
        let rows = ((pill_h / 2) | 1).clamp(3, DOCK_GLYPH_ROWS as i32) as usize;
        let max_w = (rows as i32 - 1) / 2 + 1;
        let gy = by + (pill_h - rows as i32) / 2;
        let glyph = at.bar.dock_button_glyph;

        let mut bx = left + 4;
        for (i, name) in DOCK_BTN_NAMES.iter().enumerate() {
            ensure_pill(
                sdi,
                name,
                bx,
                by,
                pill_w as u32,
                pill_h as u32,
                at.bar.dock_button_fill,
                glyph,
            );
            match i {
                // prev: two left-pointing triangles.
                0 => {
                    let gw = 2 * max_w + 1;
                    let gx = bx + (pill_w - gw) / 2;
                    dock_triangle(sdi, DOCK_GLYPH_PREFIXES[0], gx, gy, rows, false, glyph);
                    dock_triangle(
                        sdi,
                        DOCK_GLYPH_PREFIXES[1],
                        gx + max_w + 1,
                        gy,
                        rows,
                        false,
                        glyph,
                    );
                },
                // play: one right-pointing triangle.
                1 => {
                    let gx = bx + (pill_w - max_w) / 2;
                    dock_triangle(sdi, DOCK_GLYPH_PREFIXES[2], gx, gy, rows, true, glyph);
                },
                // next: two right-pointing triangles.
                _ => {
                    let gw = 2 * max_w + 1;
                    let gx = bx + (pill_w - gw) / 2;
                    dock_triangle(sdi, DOCK_GLYPH_PREFIXES[3], gx, gy, rows, true, glyph);
                    dock_triangle(
                        sdi,
                        DOCK_GLYPH_PREFIXES[4],
                        gx + max_w + 1,
                        gy,
                        rows,
                        true,
                        glyph,
                    );
                },
            }
            bx += pill_w + gap;
        }
        // Hide any glyph scanlines beyond the active row count.
        for prefix in DOCK_GLYPH_PREFIXES {
            for r in rows..DOCK_GLYPH_ROWS {
                if let Ok(obj) = sdi.get_mut(&format!("{prefix}{r}")) {
                    obj.visible = false;
                }
            }
        }

        // Progress + volume tracks fill the middle between the transport
        // pills and the USB indicator. Static placeholder levels.
        let track_h = 4u32;
        let track_y = bar_y + (bar_h as i32 - track_h as i32) / 2;
        let track_x = bx + 6;
        let region_end = usb_x - 12;
        let avail = (region_end - track_x).max(24);
        let prog_w = (avail * 3 / 5).max(16) as u32;
        let vol_x = track_x + prog_w as i32 + 10;
        let vol_w = (region_end - vol_x).clamp(12, avail) as u32;

        ensure_rounded_fill(
            sdi,
            "bar_dock_progress_track",
            track_x,
            track_y,
            prog_w,
            track_h,
            at.bar.dock_progress_track,
            2,
        );
        // 40% placeholder progress.
        ensure_rounded_fill(
            sdi,
            "bar_dock_progress_fill",
            track_x,
            track_y,
            (prog_w * 2 / 5).max(1),
            track_h,
            at.bar.dock_progress_fill,
            2,
        );
        ensure_rounded_fill(
            sdi,
            "bar_dock_vol_track",
            vol_x,
            track_y,
            vol_w,
            track_h,
            at.bar.dock_vol_track,
            2,
        );
        // 70% placeholder volume.
        ensure_rounded_fill(
            sdi,
            "bar_dock_vol_fill",
            vol_x,
            track_y,
            (vol_w * 7 / 10).max(1),
            track_h,
            at.bar.dock_vol_fill,
            2,
        );
        // Draw fills above their tracks.
        for name in ["bar_dock_progress_fill", "bar_dock_vol_fill"] {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.z = 902;
            }
        }
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
        hide_dock(sdi);
    }
}

impl Default for BottomBar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_tab_cycle() {
        let mut bar = BottomBar::new();
        assert_eq!(bar.active_tab, MediaTab::None);
        bar.next_tab();
        assert_eq!(bar.active_tab, MediaTab::Audio);
        bar.next_tab();
        assert_eq!(bar.active_tab, MediaTab::Video);
        bar.next_tab();
        assert_eq!(bar.active_tab, MediaTab::Image);
        bar.next_tab();
        assert_eq!(bar.active_tab, MediaTab::File);
        bar.next_tab();
        assert_eq!(bar.active_tab, MediaTab::None);
    }

    #[test]
    fn update_sdi_creates_objects() {
        let bar = BottomBar::new();
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();
        let mut feat = crate::skin::SkinFeatures::default();
        feat.show_media_tabs = true;
        bar.update_sdi(&mut sdi, &at, &feat);
        assert!(sdi.contains("bar_bottom"));
        // bar_url is not created when bar_url_text is empty (default).
        assert!(sdi.contains("bar_btab_0"));
        assert!(sdi.contains("bar_btab_1"));
        assert!(sdi.contains("bar_btab_2"));
        assert!(sdi.contains("bar_btab_3"));
        assert!(sdi.contains("bar_bpipe_0"));
        assert!(sdi.contains("bar_bpipe_1"));
        assert!(sdi.contains("bar_bpipe_2"));
    }

    #[test]
    fn page_dots_visibility() {
        let mut bar = BottomBar::new();
        bar.total_pages = 3;
        bar.current_page = 1;
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();
        let feat = crate::skin::SkinFeatures::default();
        bar.update_sdi(&mut sdi, &at, &feat);

        assert!(sdi.get("bar_page_0").unwrap().visible);
        assert!(sdi.get("bar_page_1").unwrap().visible);
        assert!(sdi.get("bar_page_2").unwrap().visible);
    }

    #[test]
    fn bar_is_overlay() {
        let bar = BottomBar::new();
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();
        let feat = crate::skin::SkinFeatures::default();
        bar.update_sdi(&mut sdi, &at, &feat);
        assert!(sdi.get("bar_bottom").unwrap().overlay);
    }

    #[test]
    fn media_tab_labels() {
        assert_eq!(MediaTab::None.label(), "");
        assert_eq!(MediaTab::Audio.label(), "AUDIO");
        assert_eq!(MediaTab::Video.label(), "VIDEO");
        assert_eq!(MediaTab::Image.label(), "IMAGE");
        assert_eq!(MediaTab::File.label(), "FILE");
    }

    #[test]
    fn media_tab_next_from_none() {
        assert_eq!(MediaTab::None.next(), MediaTab::Audio);
    }

    #[test]
    fn media_tab_next_from_file_wraps() {
        assert_eq!(MediaTab::File.next(), MediaTab::None);
    }

    #[test]
    fn bottombar_default_state() {
        let bar = BottomBar::new();
        assert_eq!(bar.active_tab, MediaTab::None);
        assert_eq!(bar.current_page, 0);
        assert_eq!(bar.total_pages, 1);
        assert!(!bar.l_pressed);
        assert!(!bar.r_pressed);
    }

    #[test]
    fn bottombar_default_trait() {
        let bar = BottomBar::default();
        assert_eq!(bar.active_tab, MediaTab::None);
    }

    #[test]
    fn next_tab_cycles_correctly() {
        let mut bar = BottomBar::new();
        bar.next_tab();
        assert_eq!(bar.active_tab, MediaTab::Audio);
        bar.next_tab();
        assert_eq!(bar.active_tab, MediaTab::Video);
        bar.next_tab();
        assert_eq!(bar.active_tab, MediaTab::Image);
        bar.next_tab();
        assert_eq!(bar.active_tab, MediaTab::File);
        bar.next_tab();
        assert_eq!(bar.active_tab, MediaTab::None);
    }

    #[test]
    fn page_dots_hidden_when_disabled() {
        let mut bar = BottomBar::new();
        bar.total_pages = 3;
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();

        // First enable to create objects.
        let mut feat = crate::skin::SkinFeatures::default();
        bar.update_sdi(&mut sdi, &at, &feat);

        // Now disable and verify they're hidden.
        feat.show_page_dots = false;
        bar.update_sdi(&mut sdi, &at, &feat);

        assert!(!sdi.get("bar_page_0").unwrap().visible);
        assert!(!sdi.get("bar_page_1").unwrap().visible);
        assert!(!sdi.get("bar_page_2").unwrap().visible);
    }

    #[test]
    fn media_tabs_hidden_when_disabled() {
        let bar = BottomBar::new();
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();

        // First enable to create objects.
        let mut feat = crate::skin::SkinFeatures::default();
        feat.show_media_tabs = true;
        bar.update_sdi(&mut sdi, &at, &feat);

        // Now disable and verify they're hidden.
        feat.show_media_tabs = false;
        bar.update_sdi(&mut sdi, &at, &feat);

        assert!(!sdi.get("bar_btab_0").unwrap().visible);
    }

    #[test]
    fn hide_sdi_hides_all_objects() {
        let bar = BottomBar::new();
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();
        let feat = crate::skin::SkinFeatures::default();
        bar.update_sdi(&mut sdi, &at, &feat);

        BottomBar::hide_sdi(&mut sdi);

        assert!(!sdi.get("bar_bottom").unwrap().visible);
        // bar_url and bar_usb are not created when URL text is empty.
    }

    #[test]
    fn active_tab_color_differs_from_inactive() {
        let mut bar = BottomBar::new();
        bar.active_tab = MediaTab::Audio;
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();
        let mut feat = crate::skin::SkinFeatures::default();
        feat.show_media_tabs = true;
        bar.update_sdi(&mut sdi, &at, &feat);

        let audio_tab = sdi.get("bar_btab_0").unwrap();
        let video_tab = sdi.get("bar_btab_1").unwrap();
        assert_ne!(audio_tab.text_color, video_tab.text_color);
    }

    #[test]
    fn bottom_clock_renders_when_enabled() {
        let mut bar = BottomBar::new();
        bar.update_info(Some(&crate::platform::SystemTime {
            year: 2026,
            month: 5,
            day: 1,
            hour: 14,
            minute: 32,
            second: 0,
        }));
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();
        let mut feat = crate::skin::SkinFeatures::default();
        feat.clock_in_bottombar = true;
        bar.update_sdi(&mut sdi, &at, &feat);

        let clock = sdi.get("bar_bottom_clock").unwrap();
        let text = clock.text.as_ref().unwrap();
        assert!(text.contains("14:32"));
        assert!(text.contains("May"));
        assert!(clock.visible);
    }

    #[test]
    fn bottom_clock_hidden_when_opted_out() {
        let bar = BottomBar::new();
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();
        let mut feat = crate::skin::SkinFeatures::default();
        feat.clock_in_bottombar = false;
        bar.update_sdi(&mut sdi, &at, &feat);
        // Object is not created when the feature is opted out.
        assert!(!sdi.contains("bar_bottom_clock"));
    }

    #[test]
    fn url_label_hidden_when_empty() {
        let bar = BottomBar::new();
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();
        let feat = crate::skin::SkinFeatures::default();
        bar.update_sdi(&mut sdi, &at, &feat);

        // Default theme has empty bar_url_text, so URL label is not created.
        assert!(!sdi.contains("bar_url"));
    }

    #[test]
    fn url_label_shown_when_set() {
        let bar = BottomBar::new();
        let mut sdi = SdiRegistry::new();
        let mut at = crate::active_theme::ActiveTheme::default();
        at.bar.url_text = "HTTP://EXAMPLE".to_string();
        let feat = crate::skin::SkinFeatures::default();
        bar.update_sdi(&mut sdi, &at, &feat);

        let url = sdi.get("bar_url").unwrap();
        assert_eq!(url.text, Some("HTTP://EXAMPLE".to_string()));
        assert!(url.visible);
    }

    #[test]
    fn usb_hidden_when_url_empty() {
        let bar = BottomBar::new();
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();
        let feat = crate::skin::SkinFeatures::default();
        bar.update_sdi(&mut sdi, &at, &feat);

        // USB indicator is hidden when URL text is empty.
        assert!(!sdi.contains("bar_usb"));
    }

    #[test]
    fn page_dot_count_limited_to_max() {
        let mut bar = BottomBar::new();
        bar.total_pages = 20; // More than MAX_PAGE_DOTS.
        let mut sdi = SdiRegistry::new();
        let at = crate::active_theme::ActiveTheme::default();
        let feat = crate::skin::SkinFeatures::default();
        bar.update_sdi(&mut sdi, &at, &feat);

        // Only MAX_PAGE_DOTS (typically 8) should be visible.
        let max_dots = theme::MAX_PAGE_DOTS;
        assert!(
            sdi.get(&format!("bar_page_{}", max_dots - 1))
                .unwrap()
                .visible
        );
    }
}
