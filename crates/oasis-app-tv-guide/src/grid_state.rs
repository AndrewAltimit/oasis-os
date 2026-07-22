use oasis_skin::active_theme::ActiveTheme;
use oasis_types::backend::TextureId;

use crate::catalog::{ChannelCatalog, VideoEpisode};
use crate::channel::Channel;
use crate::grid_layout::{
    SLOT_DURATION, SdiNames, TvGuideColors, VISIBLE_ROWS, current_unix_time, truncate_title,
    volume_bar_rect,
};
use crate::schedule::{self, CachedSchedule};

/// Runtime state for the TV Guide app.
pub struct TvGuideState {
    /// Channel configuration.
    pub channels: Vec<Channel>,
    /// Per-channel catalogs (same order as `channels`). `None` = not yet loaded.
    pub catalogs: Vec<Option<ChannelCatalog>>,
    /// Selected channel index (0-based, into `channels` vec).
    pub selected_channel: usize,
    /// Time offset in 30-min slot increments (0 = current time window).
    pub time_offset: i64,
    /// Smooth selection lerp position.
    pub(crate) visual_selected: f32,
    /// Whether the guide is in "tuned" mode (watching a channel).
    pub tuned_channel: Option<usize>,
    /// Current Unix timestamp (updated each frame from system time).
    pub current_time: u64,
    /// Pre-computed shuffled playlists (rebuilt when catalogs arrive).
    pub(crate) cached_schedules: Vec<Option<CachedSchedule>>,
    /// Unix second of last schedule recompute (skip redundant updates).
    pub(crate) last_schedule_time: u64,
    /// Pre-computed SDI object name strings.
    pub(crate) sdi_names: SdiNames,
    /// Whether a catalog fetch has been attempted (prevents infinite retry).
    pub fetch_attempted: bool,
    /// Whether a catalog fetch is currently in progress (background thread/future).
    pub fetch_in_progress: bool,
    /// Error message from a failed catalog fetch.
    pub fetch_error: Option<String>,
    /// First visible channel row (for paging when channels > VISIBLE_ROWS).
    pub scroll_offset: usize,
    /// Texture for the in-app video preview (set by the video player).
    pub preview_texture: Option<TextureId>,
    /// Download progress status text (e.g. "Downloading... 42% (1234/5678KB)").
    pub download_status: Option<String>,
    /// Whether the video is expanded to fill the entire content area.
    ///
    /// When `true`, the video covers the full screen and the EPG grid is
    /// hidden. Clicking the video toggles back to PIP mode with the guide
    /// visible below.
    pub video_expanded: bool,
    /// Video volume level (0–100). Changes are communicated to the backend
    /// via the `volume_changed` dirty flag.
    pub volume: u8,
    /// Set to `true` whenever `volume` changes so the backend can sync.
    pub volume_changed: bool,
    /// Theme-derived colors for the TV Guide UI.
    pub colors: TvGuideColors,
}

impl std::fmt::Debug for TvGuideState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TvGuideState")
            .field("channels", &self.channels)
            .field("catalogs", &self.catalogs)
            .field("selected_channel", &self.selected_channel)
            .field("time_offset", &self.time_offset)
            .field("visual_selected", &self.visual_selected)
            .field("tuned_channel", &self.tuned_channel)
            .field("current_time", &self.current_time)
            .field("scroll_offset", &self.scroll_offset)
            .finish_non_exhaustive()
    }
}

impl TvGuideState {
    /// Create a new TV guide state from channel config.
    ///
    /// Colors are derived from the active theme. Skins can override
    /// individual colors via `[app_themes.tv_guide]` in theme.toml.
    pub fn new(config: &crate::channel::ChannelConfig, at: &ActiveTheme) -> Self {
        let channel_count = config.channel.len();
        Self {
            channels: config.channel.clone(),
            catalogs: vec![None; channel_count],
            selected_channel: 0,
            time_offset: 0,
            visual_selected: 0.0,
            tuned_channel: None,
            current_time: current_unix_time(),
            cached_schedules: (0..channel_count).map(|_| None).collect(),
            last_schedule_time: 0,
            sdi_names: SdiNames::new(),
            fetch_attempted: false,
            fetch_in_progress: false,
            fetch_error: None,
            scroll_offset: 0,
            preview_texture: None,
            download_status: None,
            video_expanded: false,
            volume: 50,
            volume_changed: false,
            colors: TvGuideColors::from_theme(at),
        }
    }

    /// Reset fetch state so catalogs can be re-fetched from scratch.
    ///
    /// Clears existing catalogs and cached schedules so the fetch guard
    /// (`catalogs.iter().all(|c| c.is_none())`) will pass on retry.
    pub fn reset_for_retry(&mut self) {
        self.fetch_attempted = false;
        self.fetch_in_progress = false;
        self.fetch_error = None;
        self.catalogs.fill(None);
        // `CachedSchedule` is not `Clone`, so `fill(None)` can't be used.
        self.cached_schedules.fill_with(|| None);
    }

    /// Rebuild the cached schedule for a channel after its catalog changes.
    pub fn rebuild_cached_schedule(&mut self, index: usize) {
        if let Some(Some(catalog)) = self.catalogs.get(index) {
            let cached = CachedSchedule::new(catalog);
            if index < self.cached_schedules.len() {
                self.cached_schedules[index] = cached;
            }
        }
    }

    /// Update current time from system clock.
    pub fn update_time(&mut self) {
        self.current_time = current_unix_time();
    }

    /// Navigate channel selection up (auto-scrolls).
    pub fn select_up(&mut self) {
        if self.selected_channel > 0 {
            self.selected_channel -= 1;
            if self.selected_channel < self.scroll_offset {
                self.scroll_offset = self.selected_channel;
            }
        }
    }

    /// Navigate channel selection down (auto-scrolls).
    pub fn select_down(&mut self) {
        if self.selected_channel + 1 < self.channels.len() {
            self.selected_channel += 1;
            if self.selected_channel >= self.scroll_offset + VISIBLE_ROWS {
                self.scroll_offset = self.selected_channel + 1 - VISIBLE_ROWS;
            }
        }
    }

    /// Scroll time window left (earlier).
    pub fn scroll_left(&mut self) {
        self.time_offset -= 1;
    }

    /// Scroll time window right (later).
    pub fn scroll_right(&mut self) {
        self.time_offset += 1;
    }

    /// Current page number (1-based).
    pub fn current_page(&self) -> usize {
        if self.channels.is_empty() {
            return 1;
        }
        self.scroll_offset / VISIBLE_ROWS + 1
    }

    /// Total number of pages.
    pub fn total_pages(&self) -> usize {
        if self.channels.is_empty() {
            return 1;
        }
        self.channels.len().div_ceil(VISIBLE_ROWS)
    }

    /// Tune to the currently selected channel.
    ///
    /// Returns `None` (no-op) if the selected channel is already playing.
    pub fn tune(&mut self) -> Option<TuneRequest> {
        if self.selected_channel >= self.channels.len() {
            return None;
        }
        // Ignore if already tuned to this channel.
        if self.tuned_channel == Some(self.selected_channel) {
            return None;
        }
        // Refresh time so the schedule reflects *now*, not the last render frame.
        self.update_time();
        let catalog = self.catalogs.get(self.selected_channel)?.as_ref()?;
        let slot = schedule::schedule_at(catalog, self.current_time)?;
        self.tuned_channel = Some(self.selected_channel);
        Some(TuneRequest {
            channel_index: self.selected_channel,
            episode: slot.episode,
            seek_secs: slot.elapsed_secs,
        })
    }

    /// Un-tune (return to guide view).
    pub fn untune(&mut self) {
        self.tuned_channel = None;
        self.preview_texture = None;
        self.download_status = None;
        self.video_expanded = false;
    }

    /// Get the grid's start time (aligned to 30-min boundary, with offset).
    pub(crate) fn grid_start_time(&self) -> u64 {
        let base = schedule::align_to_slot(self.current_time);
        if self.time_offset >= 0 {
            base + (self.time_offset as u64) * SLOT_DURATION
        } else {
            base.saturating_sub((-self.time_offset) as u64 * SLOT_DURATION)
        }
    }

    /// Generate content lines for text-mode display (fallback rendering).
    pub fn text_content(&self) -> Vec<String> {
        let loaded = self.catalogs.iter().filter(|c| c.is_some()).count();
        log::debug!(
            "TV: text_content() channels={} loaded={} fetch_attempted={} error={:?}",
            self.channels.len(),
            loaded,
            self.fetch_attempted,
            self.fetch_error,
        );

        let mut lines = Vec::new();
        lines.push("=== TV Guide ===".to_string());
        lines.push(String::new());

        if self.channels.is_empty() {
            lines.push("(No channels configured)".to_string());
            return lines;
        }

        let any_loaded = self.catalogs.iter().any(|c| c.is_some());
        if !any_loaded {
            if let Some(ref err) = self.fetch_error {
                lines.push(format!("Error: {err}"));
            } else if self.fetch_in_progress || !self.fetch_attempted {
                let dots = ".".repeat((self.current_time % 4) as usize + 1);
                lines.push(format!("Loading channel catalogs{dots}"));
            } else {
                lines.push("No content available from any channel.".to_string());
            }
            lines.push(String::new());
        }

        // Show what's on now for each channel.
        lines.push("--- Now Playing ---".to_string());
        for (i, ch) in self.channels.iter().enumerate() {
            let now_text = if let Some(catalog) = self.catalogs.get(i).and_then(|c| c.as_ref()) {
                if let Some(slot) = schedule::schedule_at(catalog, self.current_time) {
                    let remaining = schedule::format_duration(slot.remaining_secs as f64);
                    format!(
                        "{} ({remaining} left)",
                        truncate_title(&slot.episode.title, 28),
                    )
                } else {
                    "(empty schedule)".to_string()
                }
            } else {
                "(loading...)".to_string()
            };
            let marker = if self.tuned_channel == Some(i) {
                ">"
            } else {
                " "
            };
            lines.push(format!(
                " {marker} CH {:>2} {:<5} {}",
                ch.number, ch.call_sign, now_text,
            ));
        }

        lines.push(String::new());
        lines.push("Up/Down=Channel  Confirm=Tune  Select=Retry  Cancel=Exit".to_string());

        lines
    }

    // ---------------------------------------------------------------
    // Header helpers
    // ---------------------------------------------------------------

    /// Build channel info line for the header (e.g. "RETRO 2").
    pub(crate) fn build_channel_info(&self) -> String {
        if let Some(ch) = self.channels.get(self.selected_channel) {
            format!("{} {}", ch.call_sign, ch.number)
        } else {
            "TV Guide".to_string()
        }
    }

    /// Build "Now Playing" title line (ALL CAPS).
    pub(crate) fn build_now_playing_title(&self) -> String {
        let idx = self.selected_channel;
        if let Some(Some(cached)) = self.cached_schedules.get(idx)
            && let Some(slot) = cached.at(self.current_time)
        {
            return truncate_title(&slot.episode.title.to_uppercase(), 35);
        }
        if self.catalogs.get(idx).and_then(|c| c.as_ref()).is_none() {
            if let Some(ref err) = self.fetch_error {
                return format!("Error: {err}");
            }
            if self.fetch_in_progress || !self.fetch_attempted {
                let dots = ".".repeat((self.current_time % 4) as usize + 1);
                return format!("Loading catalog{dots}");
            }
            return "No content available".to_string();
        }
        "No content available".to_string()
    }

    /// Build detail line for now-playing (duration, resolution, remaining).
    pub(crate) fn build_now_playing_detail(&self) -> String {
        let idx = self.selected_channel;
        if let Some(Some(cached)) = self.cached_schedules.get(idx)
            && let Some(slot) = cached.at(self.current_time)
        {
            let dur = schedule::format_duration(slot.episode.duration_secs);
            let remaining = schedule::format_duration(slot.remaining_secs as f64);
            return format!(
                "{dur} | {}x{}  ({remaining} remaining)",
                slot.episode.width, slot.episode.height,
            );
        }
        String::new()
    }

    /// Build location/genre line for the header.
    pub(crate) fn build_channel_location(&self) -> String {
        if let Some(ch) = self.channels.get(self.selected_channel) {
            if let Some(ref loc) = ch.location {
                return loc.to_uppercase();
            }
            return ch.genre.to_uppercase();
        }
        String::new()
    }

    /// Handle a click at content-local coordinates.
    ///
    /// `cw`/`ch` are the content area dimensions (needed to recompute layout).
    /// Returns `Some(TuneRequest)` if the click tuned a channel (caller should
    /// enter fullscreen), `None` if it only selected a channel or missed.
    ///
    /// When the video is expanded (fullscreen), any click collapses back to
    /// PIP mode. When in PIP mode, clicking the video preview area expands
    /// it to fullscreen.
    pub fn handle_click(
        &mut self,
        lx: i32,
        ly: i32,
        cw: u32,
        ch: u32,
        fullscreen: bool,
    ) -> Option<TuneRequest> {
        // Volume bar click (available in both expanded and PIP modes).
        if self.tuned_channel.is_some() {
            let vr = volume_bar_rect(cw, ch, self.video_expanded);
            if lx >= vr.x && lx < vr.x + vr.w as i32 && ly >= vr.y && ly < vr.y + vr.h as i32 {
                let frac = ((lx - vr.x) as f32 / vr.w as f32).clamp(0.0, 1.0);
                self.volume = (frac * 100.0) as u8;
                self.volume_changed = true;
                return None;
            }
        }

        // Expanded video: click anywhere to collapse back to PIP.
        if self.video_expanded {
            self.video_expanded = false;
            return None;
        }

        // Check if clicking the video preview area (PIP) to expand.
        if self.tuned_channel.is_some() && self.preview_texture.is_some() {
            let usable_h = ch;
            let min_header = if fullscreen { 60 } else { 40 };
            let header_h = (usable_h * 20 / 100).max(min_header);
            let preview_w = (cw * 30 / 100).max(if fullscreen { 80 } else { 60 });
            let preview_h = header_h.saturating_sub(if fullscreen { 16 } else { 8 });
            let preview_x = cw as i32 - preview_w as i32 - if fullscreen { 10 } else { 4 };
            let preview_y = if fullscreen { 8i32 } else { 4i32 };

            if lx >= preview_x
                && lx < preview_x + preview_w as i32
                && ly >= preview_y
                && ly < preview_y + preview_h as i32
            {
                self.video_expanded = true;
                return None;
            }
        }

        if self.channels.is_empty() || ly < 0 || lx < 0 {
            return None;
        }

        // Recompute layout from content dimensions.
        let usable_h = ch;
        let (min_header, min_time, min_footer) = if fullscreen {
            (60, 20, 18) // matches update_sdi
        } else {
            (40, 16, 14) // matches draw_windowed
        };
        let header_h = (usable_h * 20 / 100).max(min_header);
        let time_header_h = (usable_h * 4 / 100).max(min_time);
        let footer_h = (usable_h * 5 / 100).max(min_footer);
        let grid_h = usable_h.saturating_sub(header_h + time_header_h + footer_h);

        let row_count = self.channels.len().clamp(1, VISIBLE_ROWS) as u32;
        let min_row = if fullscreen { 20 } else { 16 };
        let row_h = (grid_h / row_count).max(min_row);

        let grid_y = header_h + time_header_h;

        let ly_u = ly as u32;
        if ly_u < grid_y || ly_u >= grid_y + row_count * row_h {
            return None;
        }

        let vis_row = ((ly_u - grid_y) / row_h) as usize;
        let channel_idx = self.scroll_offset + vis_row;
        if channel_idx >= self.channels.len() {
            return None;
        }

        if self.selected_channel == channel_idx {
            // Already selected -- tune (same as Confirm).
            self.tune()
        } else {
            // Select this channel.
            self.selected_channel = channel_idx;
            None
        }
    }
}

/// A request to tune to a specific channel and episode.
#[derive(Debug, Clone)]
pub struct TuneRequest {
    /// Index into the channels vec.
    pub channel_index: usize,
    /// The episode that should be playing.
    pub episode: VideoEpisode,
    /// How many seconds into the episode to seek.
    pub seek_secs: u64,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::catalog::{ChannelCatalog, VideoEpisode};
    use crate::channel::{ChannelConfig, DEFAULT_CHANNELS_TOML};
    use oasis_skin::active_theme::ActiveTheme;

    fn default_state() -> TvGuideState {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        TvGuideState::new(&config, &ActiveTheme::default())
    }

    fn make_episode(title: &str, duration: f64) -> VideoEpisode {
        VideoEpisode {
            item_id: "test-item".to_string(),
            filename: format!("{title}.mp4"),
            title: title.to_string(),
            duration_secs: duration,
            width: 640,
            height: 480,
            size_bytes: 50_000_000,
            format: "MPEG4".into(),
            original: None,
        }
    }

    fn state_with_catalog() -> TvGuideState {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        let mut catalog = ChannelCatalog::new(state.channels[0].number);
        catalog.add_episodes(vec![
            make_episode("Alpha", 1800.0),
            make_episode("Beta", 2400.0),
            make_episode("Gamma", 3600.0),
        ]);
        state.catalogs[0] = Some(catalog);
        state.rebuild_cached_schedule(0);
        state
    }

    // -- build_channel_info --

    #[test]
    fn build_channel_info_first_channel() {
        let state = default_state();
        let info = state.build_channel_info();
        assert_eq!(info, "RETRO 2");
    }

    #[test]
    fn build_channel_info_navigated() {
        let mut state = default_state();
        state.select_down(); // channel index 1
        let info = state.build_channel_info();
        assert_eq!(info, "TECH 5");
    }

    #[test]
    fn build_channel_info_empty_channels() {
        let config: ChannelConfig = toml::from_str("channel = []").unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.build_channel_info(), "TV Guide");
    }

    // -- build_channel_location --

    #[test]
    fn build_channel_location_with_location() {
        let state = default_state();
        let loc = state.build_channel_location();
        assert_eq!(loc, "LOS ANGELES, CA");
    }

    #[test]
    fn build_channel_location_without_location() {
        let toml = r#"
            [[channel]]
            number = 1
            call_sign = "TEST"
            name = "Test"
            genre = "comedy"
            [[channel.source]]
            item_id = "test-item"
        "#;
        let config = ChannelConfig::from_toml(toml).unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.build_channel_location(), "COMEDY");
    }

    #[test]
    fn build_channel_location_empty() {
        let config: ChannelConfig = toml::from_str("channel = []").unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.build_channel_location(), "");
    }

    // -- build_now_playing_title --

    #[test]
    fn build_now_playing_title_no_catalog() {
        let state = default_state();
        // No catalogs loaded, not fetched yet.
        let title = state.build_now_playing_title();
        assert!(title.starts_with("Loading catalog"));
    }

    #[test]
    fn build_now_playing_title_with_error() {
        let mut state = default_state();
        state.fetch_attempted = true;
        state.fetch_error = Some("connection refused".into());
        let title = state.build_now_playing_title();
        assert_eq!(title, "Error: connection refused");
    }

    #[test]
    fn build_now_playing_title_fetch_done_no_content() {
        let mut state = default_state();
        state.fetch_attempted = true;
        let title = state.build_now_playing_title();
        assert_eq!(title, "No content available");
    }

    #[test]
    fn build_now_playing_title_with_catalog() {
        let state = state_with_catalog();
        let title = state.build_now_playing_title();
        // Should be uppercase and non-empty.
        assert!(!title.is_empty());
        assert_eq!(title, title.to_uppercase());
    }

    // -- build_now_playing_detail --

    #[test]
    fn build_now_playing_detail_with_catalog() {
        let state = state_with_catalog();
        let detail = state.build_now_playing_detail();
        assert!(detail.contains("640x480"));
        assert!(detail.contains("remaining"));
        assert!(detail.contains('|'));
    }

    #[test]
    fn build_now_playing_detail_no_catalog() {
        let state = default_state();
        assert!(state.build_now_playing_detail().is_empty());
    }

    // -- reset_for_retry --

    #[test]
    fn reset_for_retry_clears_state() {
        let mut state = state_with_catalog();
        state.fetch_attempted = true;
        state.fetch_in_progress = true;
        state.fetch_error = Some("test error".into());

        state.reset_for_retry();

        assert!(!state.fetch_attempted);
        assert!(!state.fetch_in_progress);
        assert!(state.fetch_error.is_none());
        assert!(state.catalogs.iter().all(|c| c.is_none()));
        assert!(state.cached_schedules.iter().all(|c| c.is_none()));
    }

    // -- untune --

    #[test]
    fn untune_clears_playback_state() {
        let mut state = state_with_catalog();
        state.tuned_channel = Some(0);
        state.preview_texture = Some(oasis_types::backend::TextureId(42));
        state.download_status = Some("Downloading...".into());
        state.video_expanded = true;

        state.untune();

        assert!(state.tuned_channel.is_none());
        assert!(state.preview_texture.is_none());
        assert!(state.download_status.is_none());
        assert!(!state.video_expanded);
    }

    // -- tune --

    #[test]
    fn tune_returns_request_with_catalog() {
        let mut state = state_with_catalog();
        let req = state.tune();
        assert!(req.is_some());
        let req = req.unwrap();
        assert_eq!(req.channel_index, 0);
        assert!(req.episode.duration_secs > 0.0);
    }

    #[test]
    fn tune_noop_if_already_tuned() {
        let mut state = state_with_catalog();
        state.tune(); // first tune
        let req = state.tune(); // should be None (already tuned to same channel)
        assert!(req.is_none());
    }

    #[test]
    fn tune_different_channel() {
        let mut state = state_with_catalog();
        // Also add catalog for channel 1.
        let mut catalog = ChannelCatalog::new(state.channels[1].number);
        catalog.add_episodes(vec![make_episode("Other Show", 900.0)]);
        state.catalogs[1] = Some(catalog);
        state.rebuild_cached_schedule(1);

        state.tune(); // tune to channel 0
        state.select_down(); // select channel 1
        let req = state.tune(); // tune to channel 1
        assert!(req.is_some());
        assert_eq!(req.unwrap().channel_index, 1);
    }

    #[test]
    fn tune_without_catalog_returns_none() {
        let mut state = default_state();
        assert!(state.tune().is_none());
    }

    // -- grid_start_time --

    #[test]
    fn grid_start_time_no_offset() {
        let state = default_state();
        let start = state.grid_start_time();
        // Should be aligned to 30-minute boundary.
        assert_eq!(start % 1800, 0);
    }

    #[test]
    fn grid_start_time_positive_offset() {
        let mut state = default_state();
        let base = state.grid_start_time();
        state.scroll_right();
        assert_eq!(state.grid_start_time(), base + 1800);
        state.scroll_right();
        assert_eq!(state.grid_start_time(), base + 3600);
    }

    #[test]
    fn grid_start_time_negative_offset() {
        let mut state = default_state();
        let base = state.grid_start_time();
        state.scroll_left();
        assert_eq!(state.grid_start_time(), base - 1800);
    }

    // -- volume in handle_click --

    #[test]
    fn handle_click_volume_bar_expanded() {
        let mut state = state_with_catalog();
        state.tuned_channel = Some(0);
        state.video_expanded = true;

        let cw = 800u32;
        let ch = 600u32;
        let vr = crate::grid_layout::volume_bar_rect(cw, ch, true);
        // Click in the middle of the volume bar.
        let click_x = vr.x + (vr.w as i32 / 2);
        let click_y = vr.y + (vr.h as i32 / 2);

        let result = state.handle_click(click_x, click_y, cw, ch, true);
        assert!(result.is_none()); // volume click doesn't tune
        assert!(state.volume_changed);
        // Should be roughly 50%.
        assert!((45..=55).contains(&state.volume));
    }

    #[test]
    fn handle_click_volume_bar_left_edge() {
        let mut state = state_with_catalog();
        state.tuned_channel = Some(0);
        state.video_expanded = true;

        let cw = 800u32;
        let ch = 600u32;
        let vr = crate::grid_layout::volume_bar_rect(cw, ch, true);
        // Click at left edge of volume bar.
        let result = state.handle_click(vr.x, vr.y, cw, ch, true);
        assert!(result.is_none());
        assert!(state.volume_changed);
        assert!(state.volume <= 5); // should be near 0%
    }

    // -- video expand/collapse --

    #[test]
    fn handle_click_expanded_collapses() {
        let mut state = state_with_catalog();
        state.tuned_channel = Some(0);
        state.video_expanded = true;

        // Click outside volume bar should collapse.
        let result = state.handle_click(10, 10, 800, 600, true);
        assert!(result.is_none());
        assert!(!state.video_expanded);
    }

    // -- rebuild_cached_schedule --

    #[test]
    fn rebuild_cached_schedule_out_of_bounds() {
        let mut state = default_state();
        // Should not panic for out-of-bounds index.
        state.rebuild_cached_schedule(100);
    }

    #[test]
    fn rebuild_cached_schedule_no_catalog() {
        let mut state = default_state();
        // No catalog at index 0 -> cached_schedule should remain None.
        state.rebuild_cached_schedule(0);
        assert!(state.cached_schedules[0].is_none());
    }

    // -- text_content with tuned channel --

    #[test]
    fn text_content_shows_tuned_marker() {
        let mut state = state_with_catalog();
        state.tuned_channel = Some(0);
        state.fetch_attempted = true;
        let lines = state.text_content();
        // The tuned channel should have a ">" marker.
        assert!(lines.iter().any(|l| l.contains('>')));
    }

    #[test]
    fn text_content_no_channels() {
        let config: ChannelConfig = toml::from_str("channel = []").unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        let lines = state.text_content();
        assert!(lines.iter().any(|l| l.contains("No channels configured")));
    }

    // -- select_up / select_down edge cases --

    #[test]
    fn select_down_single_channel() {
        let toml = r#"
            [[channel]]
            number = 1
            call_sign = "ONE"
            name = "Only"
            genre = "test"
            [[channel.source]]
            item_id = "test"
        "#;
        let config = ChannelConfig::from_toml(toml).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        state.select_down();
        assert_eq!(state.selected_channel, 0);
    }

    // -- time_offset --

    #[test]
    fn scroll_left_right_returns_to_origin() {
        let mut state = default_state();
        assert_eq!(state.time_offset, 0);
        state.scroll_right();
        state.scroll_right();
        state.scroll_left();
        state.scroll_left();
        assert_eq!(state.time_offset, 0);
    }

    #[test]
    fn scroll_left_goes_negative() {
        let mut state = default_state();
        state.scroll_left();
        assert_eq!(state.time_offset, -1);
        state.scroll_left();
        assert_eq!(state.time_offset, -2);
    }

    // -- initial state --

    #[test]
    fn new_state_defaults() {
        let state = default_state();
        assert_eq!(state.volume, 50);
        assert!(!state.volume_changed);
        assert!(!state.video_expanded);
        assert!(state.preview_texture.is_none());
        assert!(state.download_status.is_none());
        assert!(!state.fetch_attempted);
        assert!(!state.fetch_in_progress);
        assert!(state.fetch_error.is_none());
        assert_eq!(state.time_offset, 0);
    }
}
