//! Classic PSIX footer: URL plate, USB indicator, page dots and the
//! optional media tab strip.

use oasis_skin::legacy_theme as theme;
use oasis_types::color::lerp_color;

use super::tabs::{BPIPE_NAMES, BTAB_NAMES};
use super::{BarGeometry, BottomBar, dock, place_label, sync_url_plate, text_px, url_offset};
use crate::active_theme::ActiveTheme;
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::{ensure_rounded_fill, hide_bezel, hide_objects};
use crate::skin::SkinFeatures;

/// Page dot names; length matches `theme::MAX_PAGE_DOTS`.
pub(super) const PAGE_DOT_NAMES: [&str; 4] =
    ["bar_page_0", "bar_page_1", "bar_page_2", "bar_page_3"];
const _: () = assert!(PAGE_DOT_NAMES.len() == theme::MAX_PAGE_DOTS);

impl BottomBar {
    /// Classic footer (any `bottombar_style` other than `"media_dock"`).
    pub(super) fn update_sdi_classic(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        features: &SkinFeatures,
        geo: &BarGeometry,
    ) {
        let url_offset = url_offset(at, features);
        let url_end = sync_url_plate(sdi, at, url_offset, geo);

        let right_edge = self.sync_clock(sdi, at, features, geo);

        // Media category tabs (pipe-separated). Disabled by default; opt in
        // per-skin via `show_media_tabs = true` for PSP-style layouts.
        let right_content_x = if features.show_media_tabs {
            self.draw_media_tabs(sdi, at, geo, right_edge)
        } else {
            hide_objects(sdi, &BTAB_NAMES);
            hide_objects(sdi, &BPIPE_NAMES);
            hide_bezel(sdi, "bar_tab_bezel");
            right_edge
        };

        // Classic footer never shows the media dock; hide any dock objects
        // left over from a prior dock skin (no-op when they don't exist).
        dock::hide_dock(sdi);

        // USB indicator after the URL plate (hidden without one), with a
        // gap from the bezel edge.
        let usb_end = match url_end {
            None => {
                hide_objects(sdi, &["bar_usb"]);
                url_offset
            },
            Some(end) => {
                let usb_x = end + 8;
                place_label(
                    sdi,
                    at,
                    "bar_usb",
                    (usb_x, geo.text_y),
                    "USB",
                    at.bar.usb_color,
                );
                usb_x + text_px("USB", at.font_small)
            },
        };

        let dots_x = usb_end + 12;
        let left_end = self.sync_page_dots(sdi, at, features, geo, dots_x);

        // Taskbar buttons go in the gap between the left and right content
        // (they used to start right after the start button, on top of the
        // URL plate and USB label). With neither a plate nor page dots the
        // left content is just the start button.
        let start = if url_end.is_some() || left_end != dots_x {
            left_end + 8
        } else {
            crate::taskbar::Taskbar::taskbar_start_x(at, features.start_menu)
        };
        let end = (right_content_x - 8).min(at.screen_w as i32 * 2 / 3);
        self.taskbar_span.set((start, end.max(start)));
    }

    /// Page dots (rounded for circular appearance, with lerp transition).
    ///
    /// Only rendered when there's actual pagination to indicate — a single
    /// lone "active" dot otherwise shows up next to the START button as an
    /// unexplained white speck on skins that inherit the default
    /// `page_dot_active` color.
    ///
    /// Skins without a dashboard (`dashboard = false`) have no pages to
    /// indicate either.
    ///
    /// Returns the right edge of the dots, or `dots_x` when none are shown.
    fn sync_page_dots(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        features: &SkinFeatures,
        geo: &BarGeometry,
        dots_x: i32,
    ) -> i32 {
        if !features.show_page_dots || !features.dashboard || self.total_pages <= 1 {
            hide_objects(sdi, &PAGE_DOT_NAMES);
            return dots_x;
        }
        let shown = self.total_pages.min(theme::MAX_PAGE_DOTS);
        for (i, name) in PAGE_DOT_NAMES.iter().enumerate().take(shown) {
            // Proximity: 1.0 when this dot is the visual page, 0.0 when far.
            let proximity = (1.0 - (i as f32 - self.dot_visual_page).abs()).max(0.0);
            let dot_color = lerp_color(at.bar.page_dot_inactive, at.bar.page_dot_active, proximity);
            ensure_rounded_fill(
                sdi,
                name,
                dots_x + (i as i32) * 12,
                geo.y + (geo.h as i32 - 6) / 2,
                6,
                6,
                dot_color,
                3,
            );
        }
        hide_objects(sdi, &PAGE_DOT_NAMES[shown..]);
        dots_x + (shown as i32 - 1) * 12 + 6
    }
}
