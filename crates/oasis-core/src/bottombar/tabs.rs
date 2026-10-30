//! Media category tabs (AUDIO / VIDEO / IMAGE / FILE) and their strip.

use super::{BarGeometry, BottomBar, place_label, text_px};
use crate::active_theme::ActiveTheme;
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::{BezelStyle, ensure_chrome_bezel, ensure_text};

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

    /// Display label for the tab, in the active locale.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Audio => oasis_i18n::tr!("shell.tab_audio"),
            Self::Video => oasis_i18n::tr!("shell.tab_video"),
            Self::Image => oasis_i18n::tr!("shell.tab_image"),
            Self::File => oasis_i18n::tr!("shell.tab_file"),
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

/// Pre-built SDI object names (avoids per-frame `format!` allocations).
/// Indices match `MediaTab::TABS`; pipes sit between adjacent tabs.
pub(super) const BTAB_NAMES: [&str; 4] = ["bar_btab_0", "bar_btab_1", "bar_btab_2", "bar_btab_3"];
pub(super) const BPIPE_NAMES: [&str; 3] = ["bar_bpipe_0", "bar_bpipe_1", "bar_bpipe_2"];
const _: () = assert!(BTAB_NAMES.len() == MediaTab::TABS.len());
const _: () = assert!(BPIPE_NAMES.len() + 1 == MediaTab::TABS.len());

impl BottomBar {
    /// Draw the right-aligned media category tab strip (AUDIO/VIDEO/IMAGE/
    /// FILE) ending just left of `right_edge`, and return the left x of the
    /// tab group. Shared by the classic footer and the media dock.
    pub(super) fn draw_media_tabs(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        geo: &BarGeometry,
        right_edge: i32,
    ) -> i32 {
        let font_small = at.font_small;
        let text_y = geo.text_y;

        let labels_w: i32 = MediaTab::TABS
            .iter()
            .map(|t| text_px(t.label(), font_small))
            .sum();
        let pipe_w = text_px("|", font_small);
        let pipes_w = (MediaTab::TABS.len() as i32 - 1) * (at.pipe_gap * 2 + pipe_w);
        let total_w = labels_w + pipes_w;
        let tabs_x = right_edge - total_w - 8;

        // Chrome bezel around tab group.
        ensure_chrome_bezel(
            sdi,
            "bar_tab_bezel",
            tabs_x - 6,
            geo.bezel_y,
            (total_w + 14) as u32,
            geo.bezel_h,
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
            place_label(sdi, at, name, (cx, text_y), label, color);
            if let Ok(obj) = sdi.get_mut(name) {
                obj.text_color = color;
            }
            cx += text_px(label, font_small);

            // Pipe separator (except after last tab).
            if let Some(&pipe_name) = BPIPE_NAMES.get(i) {
                cx += at.pipe_gap;
                ensure_text(sdi, pipe_name, cx, text_y, font_small, at.bar.pipe_color);
                if let Ok(obj) = sdi.get_mut(pipe_name) {
                    obj.x = cx;
                    obj.y = text_y;
                    obj.set_text("|");
                }
                cx += pipe_w + at.pipe_gap;
            }
        }
        tabs_x
    }
}
