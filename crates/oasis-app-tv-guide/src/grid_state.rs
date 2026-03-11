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
        for cat in &mut self.catalogs {
            *cat = None;
        }
        for sched in &mut self.cached_schedules {
            *sched = None;
        }
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
