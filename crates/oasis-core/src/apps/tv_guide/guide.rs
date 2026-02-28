//! TV Guide grid state and SDI rendering.
//!
//! Manages the retro cable-TV EPG layout: header bar with current channel
//! info, time-slot column headers, channel rows with variable-width
//! program cells, selection highlight, and footer navigation hints.

use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::sdi::SdiRegistry;

use super::catalog::{ChannelCatalog, VideoEpisode};
use super::channel::{Channel, ChannelConfig};
use super::schedule::{self, CachedSchedule};

/// Number of 30-minute time columns visible in the grid.
const VISIBLE_TIME_SLOTS: usize = 5;

/// Duration of one time slot in seconds (30 minutes).
const SLOT_DURATION: u64 = 1800;

// --- Retro CRT color palette ---
const COLOR_BG: Color = Color::rgba(10, 22, 40, 255);
const COLOR_GRID_LINE: Color = Color::rgba(26, 58, 92, 255);
const COLOR_HEADER_BG: Color = Color::rgba(15, 30, 55, 255);
const COLOR_TIME_HEADER: Color = Color::rgba(0, 204, 255, 255);
const COLOR_CHANNEL_LABEL: Color = Color::rgba(200, 220, 240, 255);
const COLOR_PROGRAM_TEXT: Color = Color::rgba(192, 216, 232, 255);
const COLOR_SELECTED_BG: Color = Color::rgba(255, 140, 0, 180);
const COLOR_SELECTED_TEXT: Color = Color::rgba(255, 255, 255, 255);
const COLOR_DIM_TEXT: Color = Color::rgba(100, 130, 160, 255);
const COLOR_PLAYING_TEXT: Color = Color::rgba(0, 221, 255, 255);
const COLOR_FOOTER_BG: Color = Color::rgba(12, 25, 45, 255);

/// Maximum number of channels supported for pre-computed SDI names.
const MAX_CHANNELS: usize = 10;

/// Maximum number of program cells per row.
const MAX_CELLS: usize = 8;

/// Pre-computed SDI object name strings (avoids per-frame `format!()` calls).
struct SdiNames {
    time_cols: [String; VISIBLE_TIME_SLOTS],
    row_labels: Vec<String>,
    row_lines: Vec<String>,
    row_cells: Vec<[String; MAX_CELLS]>,
}

impl SdiNames {
    fn new(channel_count: usize) -> Self {
        let n = channel_count.min(MAX_CHANNELS);
        let time_cols = std::array::from_fn(|col| format!("tv_time_{col}"));
        let row_labels = (0..n).map(|row| format!("tv_row_{row}_label")).collect();
        let row_lines = (0..n).map(|row| format!("tv_row_{row}_line")).collect();
        let row_cells = (0..n)
            .map(|row| std::array::from_fn(|ci| format!("tv_row_{row}_cell_{ci}")))
            .collect();
        Self {
            time_cols,
            row_labels,
            row_lines,
            row_cells,
        }
    }
}

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
    visual_selected: f32,
    /// Whether the guide is in "tuned" mode (watching a channel).
    pub tuned_channel: Option<usize>,
    /// Current Unix timestamp (updated each frame from system time).
    pub current_time: u64,
    /// Pre-computed shuffled playlists (rebuilt when catalogs arrive).
    cached_schedules: Vec<Option<CachedSchedule>>,
    /// Unix second of last schedule recompute (skip redundant updates).
    last_schedule_time: u64,
    /// Pre-computed SDI object name strings.
    sdi_names: SdiNames,
    /// Whether a catalog fetch has been attempted (prevents infinite retry).
    pub fetch_attempted: bool,
    /// Error message from a failed catalog fetch.
    pub fetch_error: Option<String>,
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
            .finish_non_exhaustive()
    }
}

impl TvGuideState {
    /// Create a new TV guide state from channel config.
    pub fn new(config: &ChannelConfig) -> Self {
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
            sdi_names: SdiNames::new(channel_count),
            fetch_attempted: false,
            fetch_error: None,
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

    /// Navigate channel selection up.
    pub fn select_up(&mut self) {
        if self.selected_channel > 0 {
            self.selected_channel -= 1;
        }
    }

    /// Navigate channel selection down.
    pub fn select_down(&mut self) {
        if self.selected_channel + 1 < self.channels.len() {
            self.selected_channel += 1;
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

    /// Tune to the currently selected channel.
    pub fn tune(&mut self) -> Option<TuneRequest> {
        if self.selected_channel >= self.channels.len() {
            return None;
        }
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
    }

    /// Get the grid's start time (aligned to 30-min boundary, with offset).
    fn grid_start_time(&self) -> u64 {
        let base = schedule::align_to_slot(self.current_time);
        if self.time_offset >= 0 {
            base + (self.time_offset as u64) * SLOT_DURATION
        } else {
            base.saturating_sub((-self.time_offset) as u64 * SLOT_DURATION)
        }
    }

    /// Generate content lines for text-mode display (fallback rendering).
    pub fn text_content(&self) -> Vec<String> {
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
            } else if self.fetch_attempted {
                lines.push("No content available from any channel.".to_string());
            } else {
                lines.push("Loading channel catalogs...".to_string());
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
        lines.push("Up/Down=Channel  Confirm=Tune  Cancel=Exit".to_string());

        lines
    }

    /// Render the TV guide grid to SDI objects.
    pub fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.update_time();

        // Rebuild cached schedules for any newly-loaded catalogs.
        for i in 0..self.catalogs.len() {
            if self.catalogs[i].is_some() && self.cached_schedules[i].is_none() {
                self.rebuild_cached_schedule(i);
            }
        }

        // Lerp visual selection (always runs — drives smooth animation).
        self.visual_selected +=
            (self.selected_channel as f32 - self.visual_selected) * at.app_selection_lerp_speed;

        let sw = at.screen_w;
        let sh = at.screen_h;
        let status_h = at.statusbar_height;
        let bottom_h = at.bottombar_height;

        // Layout geometry.
        let header_h = 40u32.min(sh / 6);
        let footer_h = 16u32.min(sh / 15);
        let time_header_h = 16u32.min(sh / 15);
        let grid_y = status_h + header_h + time_header_h;
        let grid_h = sh.saturating_sub(status_h + header_h + time_header_h + footer_h + bottom_h);
        let label_w = 52u32.min(sw / 6);
        let grid_w = sw.saturating_sub(label_w);
        let channel_count = self.channels.len().max(1);
        let row_h = (grid_h / channel_count as u32).clamp(12, 32);

        // --- Background ---
        ensure_obj(sdi, "tv_bg");
        if let Ok(obj) = sdi.get_mut("tv_bg") {
            obj.x = 0;
            obj.y = 0;
            obj.w = sw;
            obj.h = sh;
            obj.color = COLOR_BG;
            obj.visible = true;
            obj.z = 100;
        }

        // --- Header bar ---
        ensure_obj(sdi, "tv_header_bg");
        if let Ok(obj) = sdi.get_mut("tv_header_bg") {
            obj.x = 0;
            obj.y = status_h as i32;
            obj.w = sw;
            obj.h = header_h;
            obj.color = COLOR_HEADER_BG;
            obj.visible = true;
            obj.z = 101;
        }

        // Header: currently playing info.
        let header_text = self.build_header_text();
        ensure_obj(sdi, "tv_header_text");
        if let Ok(obj) = sdi.get_mut("tv_header_text") {
            obj.text = Some(header_text);
            obj.x = 8;
            obj.y = status_h as i32 + 4;
            obj.font_size = at.font_body;
            obj.text_color = COLOR_CHANNEL_LABEL;
            obj.visible = true;
            obj.z = 102;
        }

        // Header: playing title.
        let playing_text = self.build_playing_text();
        ensure_obj(sdi, "tv_header_playing");
        if let Ok(obj) = sdi.get_mut("tv_header_playing") {
            obj.text = Some(playing_text);
            obj.x = 8;
            obj.y = status_h as i32 + 4 + at.font_body as i32 + 2;
            obj.font_size = at.font_small;
            obj.text_color = COLOR_PLAYING_TEXT;
            obj.visible = true;
            obj.z = 102;
        }

        // --- Time header row ---
        let grid_start = self.grid_start_time();
        ensure_obj(sdi, "tv_time_bg");
        if let Ok(obj) = sdi.get_mut("tv_time_bg") {
            obj.x = 0;
            obj.y = (status_h + header_h) as i32;
            obj.w = sw;
            obj.h = time_header_h;
            obj.color = COLOR_GRID_LINE;
            obj.visible = true;
            obj.z = 101;
        }

        // Only recompute grid content when the current second changes.
        let schedule_changed = self.current_time != self.last_schedule_time;
        if schedule_changed {
            self.last_schedule_time = self.current_time;
        }

        let slot_w = grid_w / VISIBLE_TIME_SLOTS as u32;
        for col in 0..VISIBLE_TIME_SLOTS {
            let name = &self.sdi_names.time_cols[col];
            ensure_obj(sdi, name);
            if schedule_changed {
                let slot_time = grid_start + col as u64 * SLOT_DURATION;
                if let Ok(obj) = sdi.get_mut(name) {
                    obj.text = Some(schedule::format_time(slot_time));
                    obj.x = (label_w + col as u32 * slot_w) as i32 + 4;
                    obj.y = (status_h + header_h) as i32 + 2;
                    obj.font_size = at.font_small;
                    obj.text_color = COLOR_TIME_HEADER;
                    obj.visible = true;
                    obj.z = 102;
                }
            }
        }

        // --- Channel rows ---
        let grid_end = grid_start + VISIBLE_TIME_SLOTS as u64 * SLOT_DURATION;

        for (row, ch) in self.channels.iter().enumerate() {
            let row_y = grid_y as i32 + row as i32 * row_h as i32;

            // Channel label.
            let label_name = &self.sdi_names.row_labels[row];
            ensure_obj(sdi, label_name);
            if let Ok(obj) = sdi.get_mut(label_name) {
                obj.text = Some(format!("CH{:>2}\n{}", ch.number, ch.call_sign));
                obj.x = 4;
                obj.y = row_y + 2;
                obj.font_size = at.font_small;
                obj.text_color = COLOR_CHANNEL_LABEL;
                obj.visible = true;
                obj.z = 102;
            }

            // Grid line below the row.
            let line_name = &self.sdi_names.row_lines[row];
            ensure_obj(sdi, line_name);
            if let Ok(obj) = sdi.get_mut(line_name) {
                obj.x = 0;
                obj.y = row_y + row_h as i32 - 1;
                obj.w = sw;
                obj.h = 1;
                obj.color = COLOR_GRID_LINE;
                obj.visible = true;
                obj.z = 101;
            }

            // Program cells — use cached schedule if available.
            let slots = if let Some(Some(cached)) = self.cached_schedules.get(row) {
                cached.range(grid_start, grid_end)
            } else {
                Vec::new()
            };

            let total_grid_secs = (VISIBLE_TIME_SLOTS as u64 * SLOT_DURATION) as f64;
            for (ci, slot) in slots.iter().enumerate().take(MAX_CELLS) {
                let cell_name = &self.sdi_names.row_cells[row][ci];
                ensure_obj(sdi, cell_name);
                if let Ok(obj) = sdi.get_mut(cell_name) {
                    // Calculate cell position based on episode timing.
                    let ep_start = slot.start_time.max(grid_start);
                    let ep_end =
                        (slot.start_time + slot.episode.duration_secs as u64).min(grid_end);
                    let x_frac = (ep_start - grid_start) as f64 / total_grid_secs;
                    let w_frac = (ep_end - ep_start) as f64 / total_grid_secs;

                    let cell_x = label_w as i32 + (x_frac * grid_w as f64) as i32;
                    let cell_w = (w_frac * grid_w as f64) as u32;

                    let duration_str = schedule::format_duration(slot.episode.duration_secs);
                    let max_chars = (cell_w as usize / 7).saturating_sub(2);
                    let title = truncate_title(&slot.episode.title, max_chars);

                    obj.text = Some(format!("{title} ({duration_str})"));
                    obj.x = cell_x + 2;
                    obj.y = row_y + 2;
                    obj.font_size = at.font_small;
                    obj.text_color = if row == self.selected_channel {
                        COLOR_SELECTED_TEXT
                    } else {
                        COLOR_PROGRAM_TEXT
                    };
                    obj.visible = cell_w > 8;
                    obj.z = 102;
                }
            }

            // Hide excess cells from previous frames.
            let slot_count = slots.len().min(MAX_CELLS);
            for ci in slot_count..MAX_CELLS {
                let cell_name = &self.sdi_names.row_cells[row][ci];
                if let Ok(obj) = sdi.get_mut(cell_name) {
                    obj.visible = false;
                }
            }
        }

        // --- Selection highlight ---
        ensure_obj(sdi, "tv_sel_bg");
        if let Ok(obj) = sdi.get_mut("tv_sel_bg") {
            let sel_y = grid_y as f32 + self.visual_selected * row_h as f32;
            obj.x = 0;
            obj.y = sel_y as i32;
            obj.w = sw;
            obj.h = row_h;
            obj.color = COLOR_SELECTED_BG;
            obj.border_radius = Some(2);
            obj.visible = !self.channels.is_empty();
            obj.z = 101;
        }

        // --- Footer ---
        let footer_y = sh.saturating_sub(footer_h + bottom_h);
        ensure_obj(sdi, "tv_footer_bg");
        if let Ok(obj) = sdi.get_mut("tv_footer_bg") {
            obj.x = 0;
            obj.y = footer_y as i32;
            obj.w = sw;
            obj.h = footer_h;
            obj.color = COLOR_FOOTER_BG;
            obj.visible = true;
            obj.z = 101;
        }

        ensure_obj(sdi, "tv_footer_text");
        if let Ok(obj) = sdi.get_mut("tv_footer_text") {
            obj.text = Some("Up/Down=Channel  L/R=Time  Confirm=Tune  Cancel=Exit".to_string());
            obj.x = 8;
            obj.y = footer_y as i32 + 2;
            obj.font_size = at.font_hint;
            obj.text_color = COLOR_DIM_TEXT;
            obj.visible = true;
            obj.z = 102;
        }
    }

    /// Build header channel info text.
    fn build_header_text(&self) -> String {
        if let Some(ch) = self.channels.get(self.selected_channel) {
            format!("CH {} {} - {}", ch.number, ch.call_sign, ch.name)
        } else {
            "TV Guide".to_string()
        }
    }

    /// Build "Currently Playing" text for the header.
    fn build_playing_text(&self) -> String {
        let idx = self.selected_channel;
        if let Some(Some(cached)) = self.cached_schedules.get(idx)
            && let Some(slot) = cached.at(self.current_time)
        {
            let remaining = schedule::format_duration(slot.remaining_secs as f64);
            return format!(
                "Now: {} ({remaining} remaining)",
                truncate_title(&slot.episode.title, 35),
            );
        }
        if self.catalogs.get(idx).and_then(|c| c.as_ref()).is_none() {
            if let Some(ref err) = self.fetch_error {
                return format!("Error: {err}");
            }
            if self.fetch_attempted {
                return "No content available".to_string();
            }
            return "Loading catalog...".to_string();
        }
        "No content available".to_string()
    }

    /// Hide all TV guide SDI objects.
    pub fn hide_sdi(sdi: &mut SdiRegistry) {
        let fixed = [
            "tv_bg",
            "tv_header_bg",
            "tv_header_text",
            "tv_header_playing",
            "tv_time_bg",
            "tv_sel_bg",
            "tv_footer_bg",
            "tv_footer_text",
        ];
        for name in &fixed {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
        // Time slot headers.
        for col in 0..VISIBLE_TIME_SLOTS {
            let name = format!("tv_time_{col}");
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = false;
            }
        }
        // Channel rows (up to MAX_CHANNELS channels, MAX_CELLS cells each).
        for row in 0..MAX_CHANNELS {
            for suffix in &["_label", "_line"] {
                let name = format!("tv_row_{row}{suffix}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
            for ci in 0..MAX_CELLS {
                let name = format!("tv_row_{row}_cell_{ci}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
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

/// Ensure an SDI object exists.
fn ensure_obj(sdi: &mut SdiRegistry, name: &str) {
    if !sdi.contains(name) {
        sdi.create(name);
    }
}

/// Truncate a title to fit within max characters.
fn truncate_title(title: &str, max_chars: usize) -> String {
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
fn current_unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tv_guide::channel::{ChannelConfig, DEFAULT_CHANNELS_TOML};

    #[test]
    fn new_guide_state() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let state = TvGuideState::new(&config);
        assert_eq!(state.channels.len(), 5);
        assert_eq!(state.catalogs.len(), 5);
        assert_eq!(state.selected_channel, 0);
        assert!(state.tuned_channel.is_none());
    }

    #[test]
    fn navigation() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config);

        state.select_down();
        assert_eq!(state.selected_channel, 1);
        state.select_down();
        assert_eq!(state.selected_channel, 2);
        state.select_up();
        assert_eq!(state.selected_channel, 1);
        state.select_up();
        assert_eq!(state.selected_channel, 0);
        state.select_up(); // Should not go below 0.
        assert_eq!(state.selected_channel, 0);
    }

    #[test]
    fn navigation_bounds() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config);
        for _ in 0..100 {
            state.select_down();
        }
        assert_eq!(state.selected_channel, 4); // 5 channels, max index 4.
    }

    #[test]
    fn time_scroll() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config);
        let start = state.grid_start_time();
        state.scroll_right();
        let after_right = state.grid_start_time();
        assert_eq!(after_right, start + 1800);
        state.scroll_left();
        let after_left = state.grid_start_time();
        assert_eq!(after_left, start);
    }

    #[test]
    fn text_content_no_catalogs() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let state = TvGuideState::new(&config);
        let lines = state.text_content();
        assert!(lines.iter().any(|l| l.contains("TV Guide")));
        assert!(lines.iter().any(|l| l.contains("loading")));
    }

    #[test]
    fn tune_without_catalog() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config);
        assert!(state.tune().is_none());
    }

    #[test]
    fn truncate_title_short() {
        assert_eq!(truncate_title("Hello", 10), "Hello");
    }

    #[test]
    fn truncate_title_exact() {
        assert_eq!(truncate_title("Hello", 5), "Hello");
    }

    #[test]
    fn truncate_title_overflow() {
        assert_eq!(truncate_title("Hello World", 7), "Hello..");
    }

    #[test]
    fn truncate_title_tiny() {
        assert_eq!(truncate_title("Hello", 2), "He");
    }

    #[test]
    fn truncate_title_zero() {
        assert_eq!(truncate_title("Hello", 0), "");
    }

    #[test]
    fn text_content_with_fetch_error() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config);
        state.fetch_attempted = true;
        state.fetch_error = Some("network timeout".to_string());
        let lines = state.text_content();
        assert!(lines.iter().any(|l| l.contains("Error: network timeout")));
        assert!(!lines.iter().any(|l| l.contains("Loading")));
    }

    #[test]
    fn text_content_after_partial_load() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config);
        state.fetch_attempted = true;

        // Inject a catalog for channel 0.
        let mut catalog = super::super::catalog::ChannelCatalog::new(state.channels[0].number);
        catalog.add_episodes(vec![super::super::catalog::VideoEpisode {
            item_id: "test-item".to_string(),
            filename: "ep1.mp4".to_string(),
            title: "Test Episode".to_string(),
            duration_secs: 1800.0,
            width: 640,
            height: 480,
            size_bytes: 1000,
        }]);
        state.catalogs[0] = Some(catalog);
        state.rebuild_cached_schedule(0);

        let lines = state.text_content();
        // Should not show "Loading" — at least one catalog loaded.
        assert!(!lines.iter().any(|l| l.contains("Loading")));
        // Channel 0 should show the episode title.
        assert!(lines.iter().any(|l| l.contains("Test Episode")));
    }

    #[test]
    fn fetch_attempted_prevents_refetch_text() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config);
        // Before fetch_attempted: shows loading.
        let lines = state.text_content();
        assert!(lines.iter().any(|l| l.contains("Loading")));

        // After fetch_attempted with all None: shows no content.
        state.fetch_attempted = true;
        let lines = state.text_content();
        assert!(lines.iter().any(|l| l.contains("No content")));
        assert!(!lines.iter().any(|l| l.contains("Loading")));
    }

    #[test]
    fn rebuild_cached_schedule_after_catalog() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config);

        let mut catalog = super::super::catalog::ChannelCatalog::new(state.channels[0].number);
        catalog.add_episodes(vec![super::super::catalog::VideoEpisode {
            item_id: "test-item".to_string(),
            filename: "ep1.mp4".to_string(),
            title: "Test Episode".to_string(),
            duration_secs: 3600.0,
            width: 640,
            height: 480,
            size_bytes: 1000,
        }]);
        state.catalogs[0] = Some(catalog);
        state.rebuild_cached_schedule(0);

        // Tune should now work for channel 0.
        let req = state.tune();
        assert!(req.is_some());
        let req = req.unwrap();
        assert_eq!(req.episode.title, "Test Episode");
    }
}
