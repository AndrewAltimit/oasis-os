//! PSIX-style bottom bar -- footer with media category tabs and page dots.
//!
//! Occupies the bottom 24 pixels of the 480x272 screen (y=248-272).
//! Displays URL label, USB indicator, media category tabs, page dots,
//! and shoulder button hints.

use oasis_types::bitmap_font::glyph_advance_scaled;
use oasis_types::color::lerp_color;

use crate::active_theme::ActiveTheme;
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::{
    BezelStyle, ensure_border, ensure_chrome_bezel, ensure_rounded_fill, ensure_text, hide_bezel,
    hide_objects,
};
use crate::theme;

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
                obj.text = Some(at.bar.url_text.clone());
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

        // Media category tabs (pipe-separated).
        if features.show_media_tabs {
            let tab_labels: Vec<&str> = MediaTab::TABS.iter().map(|t| t.label()).collect();
            let labels_w: i32 = tab_labels.iter().map(|l| text_px(l, font_small)).sum();
            let pipe_w = text_px("|", font_small);
            let pipes_w = (tab_labels.len() as i32 - 1) * (at.pipe_gap * 2 + pipe_w);
            let total_w = labels_w + pipes_w;
            let tabs_x = screen_w as i32 - total_w - at.r_hint_w - 8;

            // Chrome bezel around tab group.
            let tab_bx = tabs_x - 6;
            let tab_bw = (total_w + at.r_hint_w + 14) as u32;
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
                let name = format!("bar_btab_{i}");

                let color = if *tab == self.active_tab {
                    at.bar.media_tab_active
                } else {
                    at.bar.media_tab_inactive
                };
                ensure_text(sdi, &name, cx, text_y, font_small, color);
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.text = Some(label.to_string());
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
                    let pipe_name = format!("bar_bpipe_{i}");
                    ensure_text(sdi, &pipe_name, cx, text_y, font_small, at.bar.pipe_color);
                    if let Ok(obj) = sdi.get_mut(&pipe_name) {
                        obj.text = Some("|".to_string());
                    }
                    cx += pipe_w + at.pipe_gap;
                }
            }

            // "R>" shoulder button hint on far right.
            ensure_text(
                sdi,
                "bar_r_hint",
                screen_w as i32 - at.r_hint_w,
                text_y,
                font_small,
                at.bar.r_hint_color,
            );
            if let Ok(obj) = sdi.get_mut("bar_r_hint") {
                obj.text = Some("R>".to_string());
                if at.bar.text_shadow {
                    obj.text_shadow_offset = Some((1, 1));
                    obj.text_shadow_color = Some(at.bar.text_shadow_color);
                }
            }
        } else {
            // Hide media tab objects when disabled.
            for i in 0..MediaTab::TABS.len() {
                for prefix in &["bar_btab_", "bar_bpipe_"] {
                    let name = format!("{prefix}{i}");
                    if let Ok(obj) = sdi.get_mut(&name) {
                        obj.visible = false;
                    }
                }
            }
            if let Ok(obj) = sdi.get_mut("bar_r_hint") {
                obj.visible = false;
            }
            hide_bezel(sdi, "bar_tab_bezel");
        }

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
                obj.text = Some("USB".to_string());
                if at.bar.text_shadow {
                    obj.text_shadow_offset = Some((1, 1));
                    obj.text_shadow_color = Some(at.bar.text_shadow_color);
                }
            }
            usb_x + text_px("USB", font_small)
        };

        // Page dots (rounded for circular appearance, with lerp transition).
        if features.show_page_dots {
            let dots_x = usb_end + 12;
            let max_dots = theme::MAX_PAGE_DOTS;
            for i in 0..self.total_pages.min(max_dots) {
                let name = format!("bar_page_{i}");
                // Proximity: 1.0 when this dot is the visual page, 0.0 when far.
                let proximity = (1.0 - (i as f32 - self.dot_visual_page).abs()).max(0.0);
                let dot_color =
                    lerp_color(at.bar.page_dot_inactive, at.bar.page_dot_active, proximity);
                ensure_rounded_fill(
                    sdi,
                    &name,
                    dots_x + (i as i32) * 12,
                    bar_y + (bar_h as i32 - 6) / 2,
                    6,
                    6,
                    dot_color,
                    3,
                );
            }
            for i in self.total_pages.min(max_dots)..max_dots {
                let name = format!("bar_page_{i}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        } else {
            let max_dots = theme::MAX_PAGE_DOTS;
            for i in 0..max_dots {
                let name = format!("bar_page_{i}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
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
                "bar_url",
                "bar_usb",
                "bar_r_hint",
            ],
        );
        hide_bezel(sdi, "bar_url_bezel");
        hide_bezel(sdi, "bar_tab_bezel");
        for i in 0..MediaTab::TABS.len() {
            for prefix in &["bar_btab_", "bar_bpipe_", "bar_page_"] {
                let name = format!("{prefix}{i}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        }
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
        let feat = crate::skin::SkinFeatures::default();
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
        bar.update_sdi(&mut sdi, &at, &feat);

        // Now disable and verify they're hidden.
        feat.show_media_tabs = false;
        bar.update_sdi(&mut sdi, &at, &feat);

        assert!(!sdi.get("bar_btab_0").unwrap().visible);
        assert!(!sdi.get("bar_r_hint").unwrap().visible);
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
        let feat = crate::skin::SkinFeatures::default();
        bar.update_sdi(&mut sdi, &at, &feat);

        let audio_tab = sdi.get("bar_btab_0").unwrap();
        let video_tab = sdi.get("bar_btab_1").unwrap();
        assert_ne!(audio_tab.text_color, video_tab.text_color);
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
