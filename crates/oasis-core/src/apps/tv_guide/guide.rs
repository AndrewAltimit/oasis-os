//! TV Guide grid state and SDI rendering.
//!
//! Manages the retro cable-TV EPG layout: header bar with current channel
//! info, time-slot column headers, channel rows with variable-width
//! program cells, selection highlight, and footer navigation hints.

use crate::active_theme::ActiveTheme;
use crate::backend::{Color, TextureId};
use crate::sdi::SdiRegistry;

use super::catalog::{ChannelCatalog, VideoEpisode};
use super::channel::{Channel, ChannelConfig};
use super::schedule::{self, CachedSchedule};

/// Number of 30-minute time columns visible in the grid.
const VISIBLE_TIME_SLOTS: usize = 5;

/// Duration of one time slot in seconds (30 minutes).
const SLOT_DURATION: u64 = 1800;

/// Number of channel rows visible at once (scrolls if more channels exist).
const VISIBLE_ROWS: usize = 5;

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

/// Maximum number of program cells per row.
const MAX_CELLS: usize = 8;

/// Pre-computed SDI object name strings (avoids per-frame `format!()` calls).
struct SdiNames {
    time_cols: [String; VISIBLE_TIME_SLOTS],
    time_bgs: [String; VISIBLE_TIME_SLOTS],
    row_bgs: [String; VISIBLE_ROWS],
    row_labels: [String; VISIBLE_ROWS],
    row_lines: [String; VISIBLE_ROWS],
    row_cells: [[String; MAX_CELLS]; VISIBLE_ROWS],
    row_cell_bgs: [[String; MAX_CELLS]; VISIBLE_ROWS],
}

impl SdiNames {
    fn new() -> Self {
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
    /// Whether a catalog fetch is currently in progress (background thread/future).
    pub fetch_in_progress: bool,
    /// Error message from a failed catalog fetch.
    pub fetch_error: Option<String>,
    /// First visible channel row (for paging when channels > VISIBLE_ROWS).
    pub scroll_offset: usize,
    /// Texture for the in-app video preview (set by the video player).
    pub preview_texture: Option<TextureId>,
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
    pub fn new(config: &ChannelConfig, at: &ActiveTheme) -> Self {
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
        self.preview_texture = None;
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
    // SDI rendering — 1980s EPG grid
    // ---------------------------------------------------------------

    /// Render the TV guide grid to SDI objects.
    pub fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.update_time();

        // Rebuild cached schedules for any newly-loaded catalogs.
        for i in 0..self.catalogs.len() {
            if self.catalogs[i].is_some() && self.cached_schedules[i].is_none() {
                self.rebuild_cached_schedule(i);
            }
        }

        // Lerp visual selection toward visible row position.
        let vis_pos = self.selected_channel.saturating_sub(self.scroll_offset) as f32;
        self.visual_selected += (vis_pos - self.visual_selected) * at.app_selection_lerp_speed;

        let sw = at.screen_w;
        let sh = at.screen_h;
        let status_h = at.statusbar_height;
        let bottom_h = at.bottombar_height;
        let usable_h = sh.saturating_sub(status_h + bottom_h);

        // Layout proportions.
        let header_h = (usable_h * 20 / 100).max(60);
        let time_header_h = (usable_h * 4 / 100).max(20);
        let footer_h = (usable_h * 5 / 100).max(18);
        let grid_h = usable_h.saturating_sub(header_h + time_header_h + footer_h);
        let label_w = (sw * 10 / 100).max(60);
        let grid_w = sw.saturating_sub(label_w);
        let row_count = self.channels.len().clamp(1, VISIBLE_ROWS);
        let row_h = (grid_h / row_count as u32).max(20);

        let grid_y = status_h + header_h + time_header_h;
        let footer_y = sh.saturating_sub(footer_h + bottom_h);

        self.draw_background(sdi, sw, sh);
        self.draw_header(sdi, at, sw, status_h, header_h);
        self.draw_time_headers(
            sdi,
            at,
            sw,
            status_h,
            header_h,
            time_header_h,
            label_w,
            grid_w,
        );
        self.draw_channel_rows(sdi, at, grid_y, label_w, grid_w, row_h);
        self.draw_selection_highlight(sdi, grid_y, sw, row_h);
        self.draw_footer(sdi, at, sw, footer_y, footer_h);
    }

    fn draw_background(&self, sdi: &mut SdiRegistry, sw: u32, sh: u32) {
        ensure_obj(sdi, "tv_hdr_bg");
        if let Ok(obj) = sdi.get_mut("tv_hdr_bg") {
            obj.x = 0;
            obj.y = 0;
            obj.w = sw;
            obj.h = sh;
            obj.color = self.colors.bg;
            obj.visible = true;
            obj.z = 100;
        }
    }

    fn draw_header(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        sw: u32,
        status_h: u32,
        header_h: u32,
    ) {
        let y0 = status_h as i32;

        // Header background (gradient: top→bottom).
        ensure_obj(sdi, "tv_hdr_grad");
        if let Ok(obj) = sdi.get_mut("tv_hdr_grad") {
            obj.x = 0;
            obj.y = y0;
            obj.w = sw;
            obj.h = header_h;
            obj.color = self.colors.header_bg;
            obj.gradient_top = Some(self.colors.header_bg);
            obj.gradient_bottom = Some(self.colors.header_dark);
            obj.visible = true;
            obj.z = 101;
        }

        // --- Left zone (0%-40%): Channel branding ---
        let left_w = sw * 40 / 100;

        // Date (pale blue).
        let date_text = schedule::format_date(self.current_time);
        ensure_obj(sdi, "tv_hdr_date");
        if let Ok(obj) = sdi.get_mut("tv_hdr_date") {
            obj.text = Some(format!("{date_text}  |"));
            obj.x = 10;
            obj.y = y0 + 4;
            obj.font_size = at.font_small;
            obj.text_color = self.colors.date_text;
            obj.visible = true;
            obj.z = 102;
        }

        // Time (bright cyan), positioned after date.
        let time_text = schedule::format_time(self.current_time);
        let date_w = (date_text.len() as i32 + 5) * at.font_small as i32 * 5 / 8;
        ensure_obj(sdi, "tv_hdr_time");
        if let Ok(obj) = sdi.get_mut("tv_hdr_time") {
            obj.text = Some(time_text);
            obj.x = 10 + date_w;
            obj.y = y0 + 4;
            obj.font_size = at.font_small;
            obj.text_color = self.colors.time_header;
            obj.visible = true;
            obj.z = 102;
        }

        // Channel title (large, with text shadow).
        let ch_info = self.build_channel_info();
        ensure_obj(sdi, "tv_hdr_ch_info");
        if let Ok(obj) = sdi.get_mut("tv_hdr_ch_info") {
            obj.text = Some(ch_info);
            obj.x = 10;
            obj.y = y0 + 4 + at.font_small as i32 + 2;
            obj.font_size = at.font_body;
            obj.text_color = self.colors.header_title;
            obj.text_shadow_offset = Some((1, 1));
            obj.text_shadow_color = Some(Color::rgba(0, 0, 0, 128));
            obj.visible = true;
            obj.z = 102;
        }

        // Location/genre line.
        let location = self.build_channel_location();
        ensure_obj(sdi, "tv_hdr_location");
        if let Ok(obj) = sdi.get_mut("tv_hdr_location") {
            obj.text = Some(location);
            obj.x = 10;
            obj.y = y0 + 4 + at.font_small as i32 + 2 + at.font_body as i32 + 2;
            obj.font_size = at.font_hint;
            obj.text_color = self.colors.dim_text;
            obj.visible = true;
            obj.z = 102;
        }

        // --- Center zone (40%-65%): Now playing info ---
        let center_x = left_w as i32;

        // "Currently Playing:" label.
        ensure_obj(sdi, "tv_hdr_currently");
        if let Ok(obj) = sdi.get_mut("tv_hdr_currently") {
            obj.text = Some("Currently Playing:".to_string());
            obj.x = center_x + 8;
            obj.y = y0 + 4;
            obj.font_size = at.font_hint;
            obj.text_color = self.colors.dim_text;
            obj.visible = true;
            obj.z = 102;
        }

        // Show title (ALL CAPS).
        let now_title = self.build_now_playing_title();
        ensure_obj(sdi, "tv_hdr_now_title");
        if let Ok(obj) = sdi.get_mut("tv_hdr_now_title") {
            obj.text = Some(now_title);
            obj.x = center_x + 8;
            obj.y = y0 + 4 + at.font_hint as i32 + 2;
            obj.font_size = at.font_small;
            obj.text_color = self.colors.playing_text;
            obj.visible = true;
            obj.z = 102;
        }

        // Episode metadata + remaining time.
        let now_detail = self.build_now_playing_detail();
        ensure_obj(sdi, "tv_hdr_now_detail");
        if let Ok(obj) = sdi.get_mut("tv_hdr_now_detail") {
            obj.text = Some(now_detail);
            obj.x = center_x + 8;
            obj.y = y0 + 4 + at.font_hint as i32 + at.font_small as i32 + 4;
            obj.font_size = at.font_hint;
            obj.text_color = self.colors.dim_text;
            obj.visible = true;
            obj.z = 102;
        }

        // --- Right zone (65%-100%): Live preview ---
        let preview_w = (sw * 30 / 100).max(80);
        let preview_h = header_h.saturating_sub(16);
        let preview_x = sw as i32 - preview_w as i32 - 10;
        let preview_y = y0 + 8;

        // Outer glow border.
        ensure_obj(sdi, "tv_hdr_preview_outer");
        if let Ok(obj) = sdi.get_mut("tv_hdr_preview_outer") {
            obj.x = preview_x - 4;
            obj.y = preview_y - 4;
            obj.w = preview_w + 8;
            obj.h = preview_h + 8;
            obj.color = self.colors.glow_outer;
            obj.border_radius = Some(2);
            obj.visible = true;
            obj.z = 102;
        }

        // Inner border.
        ensure_obj(sdi, "tv_hdr_preview_bg");
        if let Ok(obj) = sdi.get_mut("tv_hdr_preview_bg") {
            obj.x = preview_x - 2;
            obj.y = preview_y - 2;
            obj.w = preview_w + 4;
            obj.h = preview_h + 4;
            obj.color = Color::rgba(5, 12, 25, 255);
            obj.stroke_color = Some(self.colors.glow_border);
            obj.stroke_width = Some(1);
            obj.visible = true;
            obj.z = 103;
        }

        // Video preview texture.
        ensure_obj(sdi, "tv_hdr_preview_vid");
        if let Ok(obj) = sdi.get_mut("tv_hdr_preview_vid") {
            if let Some(tex) = self.preview_texture {
                obj.x = preview_x;
                obj.y = preview_y;
                obj.w = preview_w;
                obj.h = preview_h;
                obj.texture = Some(tex);
                obj.visible = true;
                obj.z = 104;
            } else {
                obj.visible = false;
                obj.texture = None;
            }
        }

        // LIVE badge.
        let is_live = self.tuned_channel.is_some_and(|ch| {
            self.cached_schedules
                .get(ch)
                .and_then(|s| s.as_ref())
                .and_then(|c| c.at(self.current_time))
                .is_some()
        });

        ensure_obj(sdi, "tv_hdr_live_badge");
        if let Ok(obj) = sdi.get_mut("tv_hdr_live_badge") {
            obj.x = preview_x + preview_w as i32 - 42;
            obj.y = preview_y - 2;
            obj.w = 42;
            obj.h = 14;
            obj.color = self.colors.live_badge;
            obj.border_radius = Some(3);
            obj.visible = is_live;
            obj.z = 105;
        }

        ensure_obj(sdi, "tv_hdr_live_text");
        if let Ok(obj) = sdi.get_mut("tv_hdr_live_text") {
            obj.text = Some("* LIVE".to_string());
            obj.x = preview_x + preview_w as i32 - 39;
            obj.y = preview_y - 1;
            obj.font_size = at.font_hint;
            obj.text_color = self.colors.selected_text;
            obj.visible = is_live;
            obj.z = 106;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_time_headers(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        sw: u32,
        status_h: u32,
        header_h: u32,
        time_header_h: u32,
        label_w: u32,
        grid_w: u32,
    ) {
        let y0 = (status_h + header_h) as i32;
        let grid_start = self.grid_start_time();

        // Time header background.
        ensure_obj(sdi, "tv_time_bg");
        if let Ok(obj) = sdi.get_mut("tv_time_bg") {
            obj.x = 0;
            obj.y = y0;
            obj.w = sw;
            obj.h = time_header_h;
            obj.color = self.colors.time_header_bg;
            obj.visible = true;
            obj.z = 101;
        }

        // "TIME:" label (orange).
        ensure_obj(sdi, "tv_time_label_bg");
        if let Ok(obj) = sdi.get_mut("tv_time_label_bg") {
            obj.text = Some("TIME:".to_string());
            obj.x = 4;
            obj.y = y0 + 3;
            obj.font_size = at.font_hint;
            obj.text_color = self.colors.time_label;
            obj.visible = true;
            obj.z = 102;
        }

        // Time slot columns.
        let slot_w = grid_w / VISIBLE_TIME_SLOTS as u32;
        let schedule_changed = self.current_time != self.last_schedule_time;

        for col in 0..VISIBLE_TIME_SLOTS {
            let col_x = label_w + col as u32 * slot_w;

            // Column background with border.
            let bg_name = &self.sdi_names.time_bgs[col];
            ensure_obj(sdi, bg_name);
            if let Ok(obj) = sdi.get_mut(bg_name) {
                obj.x = col_x as i32;
                obj.y = y0;
                obj.w = slot_w;
                obj.h = time_header_h;
                obj.color = self.colors.time_header_bg;
                obj.stroke_color = Some(self.colors.cell_border);
                obj.stroke_width = Some(1);
                obj.visible = true;
                obj.z = 101;
            }

            // Time text (only update when second changes).
            let name = &self.sdi_names.time_cols[col];
            ensure_obj(sdi, name);
            if schedule_changed || sdi.get_mut(name).is_ok_and(|o| o.text.is_none()) {
                let slot_time = grid_start + col as u64 * SLOT_DURATION;
                if let Ok(obj) = sdi.get_mut(name) {
                    obj.text = Some(schedule::format_time(slot_time));
                    obj.x = col_x as i32 + (slot_w as i32 / 2) - 20;
                    obj.y = y0 + 3;
                    obj.font_size = at.font_hint;
                    obj.text_color = self.colors.time_header;
                    obj.visible = true;
                    obj.z = 102;
                }
            }
        }
    }

    fn draw_channel_rows(
        &mut self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        grid_y: u32,
        label_w: u32,
        grid_w: u32,
        row_h: u32,
    ) {
        let grid_start = self.grid_start_time();
        let grid_end = grid_start + VISIBLE_TIME_SLOTS as u64 * SLOT_DURATION;

        // Only recompute grid content when the current second changes.
        if self.current_time != self.last_schedule_time {
            self.last_schedule_time = self.current_time;
        }

        let total_grid_secs = (VISIBLE_TIME_SLOTS as u64 * SLOT_DURATION) as f64;
        let slot_w = grid_w / VISIBLE_TIME_SLOTS as u32;

        for vis_row in 0..VISIBLE_ROWS {
            let ch_idx = self.scroll_offset + vis_row;
            let row_y = grid_y as i32 + vis_row as i32 * row_h as i32;
            let is_selected = ch_idx == self.selected_channel;
            let has_channel = ch_idx < self.channels.len();

            // Row background.
            let bg_name = &self.sdi_names.row_bgs[vis_row];
            ensure_obj(sdi, bg_name);
            if let Ok(obj) = sdi.get_mut(bg_name) {
                obj.x = 0;
                obj.y = row_y;
                obj.w = label_w + grid_w;
                obj.h = row_h;
                obj.color = if is_selected {
                    self.colors.selected_bg
                } else {
                    self.colors.bg
                };
                if is_selected {
                    obj.gradient_top = Some(Color::rgba(255, 160, 0, 230));
                    obj.gradient_bottom = Some(Color::rgba(255, 120, 0, 200));
                } else {
                    obj.gradient_top = None;
                    obj.gradient_bottom = None;
                }
                obj.visible = has_channel;
                obj.z = 101;
            }

            // Channel label.
            let label_name = &self.sdi_names.row_labels[vis_row];
            ensure_obj(sdi, label_name);
            if let Ok(obj) = sdi.get_mut(label_name) {
                if has_channel {
                    let ch = &self.channels[ch_idx];
                    obj.text = Some(format!("[CH {}\n{}]", ch.number, ch.call_sign));
                } else {
                    obj.text = None;
                }
                obj.x = 4;
                obj.y = row_y + 4;
                obj.font_size = at.font_small;
                obj.text_color = if is_selected {
                    self.colors.selected_text
                } else {
                    self.colors.channel_label
                };
                obj.visible = has_channel;
                obj.z = 103;
            }

            // Grid line below the row.
            let line_name = &self.sdi_names.row_lines[vis_row];
            ensure_obj(sdi, line_name);
            if let Ok(obj) = sdi.get_mut(line_name) {
                obj.x = 0;
                obj.y = row_y + row_h as i32 - 1;
                obj.w = label_w + grid_w;
                obj.h = 1;
                obj.color = self.colors.grid_line;
                obj.visible = has_channel;
                obj.z = 102;
            }

            // Program cells.
            let slots = if has_channel {
                if let Some(Some(cached)) = self.cached_schedules.get(ch_idx) {
                    cached.range(grid_start, grid_end)
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            for (ci, slot) in slots.iter().enumerate().take(MAX_CELLS) {
                // Cell background rect.
                let cbg_name = &self.sdi_names.row_cell_bgs[vis_row][ci];
                ensure_obj(sdi, cbg_name);

                // Cell text.
                let cell_name = &self.sdi_names.row_cells[vis_row][ci];
                ensure_obj(sdi, cell_name);

                let ep_start = slot.start_time.max(grid_start);
                let ep_end = (slot.start_time + slot.episode.duration_secs as u64).min(grid_end);
                let x_frac = (ep_start - grid_start) as f64 / total_grid_secs;
                let w_frac = (ep_end - ep_start) as f64 / total_grid_secs;

                let cell_x = label_w as i32 + (x_frac * grid_w as f64) as i32;
                let cell_w = (w_frac * grid_w as f64) as u32;
                let visible = cell_w > 8;

                // Cell background with border.
                if let Ok(obj) = sdi.get_mut(cbg_name) {
                    obj.x = cell_x;
                    obj.y = row_y + 1;
                    obj.w = cell_w.saturating_sub(1);
                    obj.h = row_h.saturating_sub(2);
                    obj.color = if is_selected {
                        self.colors.selected_bg
                    } else {
                        self.colors.cell_bg
                    };
                    if is_selected {
                        obj.gradient_top = Some(Color::rgba(255, 160, 0, 230));
                        obj.gradient_bottom = Some(Color::rgba(255, 120, 0, 200));
                    } else {
                        obj.gradient_top = None;
                        obj.gradient_bottom = None;
                    }
                    obj.stroke_color = Some(self.colors.cell_border);
                    obj.stroke_width = Some(1);
                    obj.visible = visible;
                    obj.z = 102;
                }

                // Cell text: "H:MM\nTITLE".
                if let Ok(obj) = sdi.get_mut(cell_name) {
                    let time_str = schedule::format_time(ep_start);
                    // Scale available chars with slot width (bitmap ~6px/char).
                    let avail_cols = (slot_w as usize / 6).saturating_sub(1);
                    let max_chars = (cell_w as usize / 6).saturating_sub(1).max(avail_cols);
                    let upper_title = slot.episode.title.to_uppercase();
                    let is_now = slot.start_time <= self.current_time
                        && slot.start_time + slot.episode.duration_secs as u64 > self.current_time;
                    let title = if is_now && is_selected {
                        format!(
                            "* {}",
                            truncate_title(&upper_title, max_chars.saturating_sub(2),),
                        )
                    } else {
                        truncate_title(&upper_title, max_chars)
                    };

                    obj.text = Some(format!("{time_str}\n{title}"));
                    obj.x = cell_x + 3;
                    obj.y = row_y + 3;
                    obj.font_size = at.font_hint;
                    obj.text_color = if is_selected {
                        self.colors.selected_text
                    } else {
                        self.colors.program_text
                    };
                    obj.visible = visible;
                    obj.z = 103;
                }
            }

            // Hide excess cells from previous frames.
            let slot_count = slots.len().min(MAX_CELLS);
            for ci in slot_count..MAX_CELLS {
                let cbg_name = &self.sdi_names.row_cell_bgs[vis_row][ci];
                if let Ok(obj) = sdi.get_mut(cbg_name) {
                    obj.visible = false;
                }
                let cell_name = &self.sdi_names.row_cells[vis_row][ci];
                if let Ok(obj) = sdi.get_mut(cell_name) {
                    obj.visible = false;
                }
            }
        }
    }

    fn draw_selection_highlight(&self, sdi: &mut SdiRegistry, grid_y: u32, sw: u32, row_h: u32) {
        ensure_obj(sdi, "tv_sel_bg");
        if let Ok(obj) = sdi.get_mut("tv_sel_bg") {
            let sel_y = grid_y as f32 + self.visual_selected * row_h as f32;
            obj.x = 0;
            obj.y = sel_y as i32;
            obj.w = sw;
            obj.h = row_h;
            obj.color = self.colors.selected_bg;
            obj.gradient_top = Some(Color::rgba(255, 160, 0, 230));
            obj.gradient_bottom = Some(Color::rgba(255, 120, 0, 200));
            obj.border_radius = Some(2);
            obj.visible = !self.channels.is_empty();
            obj.z = 101;
        }
    }

    fn draw_footer(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        sw: u32,
        footer_y: u32,
        footer_h: u32,
    ) {
        ensure_obj(sdi, "tv_ftr_bg");
        if let Ok(obj) = sdi.get_mut("tv_ftr_bg") {
            obj.x = 0;
            obj.y = footer_y as i32;
            obj.w = sw;
            obj.h = footer_h;
            obj.color = self.colors.footer_bg;
            obj.visible = true;
            obj.z = 101;
        }

        // Navigation hints (left).
        ensure_obj(sdi, "tv_ftr_nav");
        if let Ok(obj) = sdi.get_mut("tv_ftr_nav") {
            obj.text = Some("[UP/DOWN SELECT]  [LEFT/RIGHT TIME]".to_string());
            obj.x = 8;
            obj.y = footer_y as i32 + 3;
            obj.font_size = at.font_hint;
            obj.text_color = self.colors.dim_text;
            obj.visible = true;
            obj.z = 102;
        }

        // Page indicator (center-right).
        let page_text = format!("[PAGE {}/{}]", self.current_page(), self.total_pages());
        ensure_obj(sdi, "tv_ftr_page");
        if let Ok(obj) = sdi.get_mut("tv_ftr_page") {
            obj.text = Some(page_text);
            obj.x = sw as i32 - 140;
            obj.y = footer_y as i32 + 3;
            obj.font_size = at.font_hint;
            obj.text_color = self.colors.dim_text;
            obj.visible = true;
            obj.z = 102;
        }

        // GUIDE label (far right).
        ensure_obj(sdi, "tv_ftr_guide");
        if let Ok(obj) = sdi.get_mut("tv_ftr_guide") {
            obj.text = Some("[GUIDE]".to_string());
            obj.x = sw as i32 - 56;
            obj.y = footer_y as i32 + 3;
            obj.font_size = at.font_hint;
            obj.text_color = self.colors.time_header;
            obj.visible = true;
            obj.z = 102;
        }
    }

    // ---------------------------------------------------------------
    // Header helpers
    // ---------------------------------------------------------------

    /// Build channel info line for the header (e.g. "RETRO 2").
    fn build_channel_info(&self) -> String {
        if let Some(ch) = self.channels.get(self.selected_channel) {
            format!("{} {}", ch.call_sign, ch.number)
        } else {
            "TV Guide".to_string()
        }
    }

    /// Build "Now Playing" title line (ALL CAPS).
    fn build_now_playing_title(&self) -> String {
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
    fn build_now_playing_detail(&self) -> String {
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
    fn build_channel_location(&self) -> String {
        if let Some(ch) = self.channels.get(self.selected_channel) {
            if let Some(ref loc) = ch.location {
                return loc.to_uppercase();
            }
            return ch.genre.to_uppercase();
        }
        String::new()
    }

    // ---------------------------------------------------------------
    // Windowed rendering (backend draw calls)
    // ---------------------------------------------------------------

    /// Draw the TV guide EPG grid using direct backend draw calls.
    ///
    /// Used by windowed mode where SDI objects aren't available.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn crate::backend::SdiBackend,
        at: &ActiveTheme,
    ) -> crate::error::Result<()> {
        // Background.
        backend.fill_rect(cx, cy, cw, ch, self.colors.bg)?;

        // Layout.
        let header_h = (ch * 20 / 100).max(40);
        let time_h = (ch * 4 / 100).max(16);
        let footer_h = (ch * 5 / 100).max(14);
        let grid_h = ch.saturating_sub(header_h + time_h + footer_h);
        let label_w = (cw * 10 / 100).max(50);
        let grid_w = cw.saturating_sub(label_w);
        let row_count = self.channels.len().clamp(1, VISIBLE_ROWS);
        let row_h = (grid_h / row_count as u32).max(16);

        // Header.
        backend.fill_rect(cx, cy, cw, header_h, self.colors.header_bg)?;
        let date_str = schedule::format_date(self.current_time);
        let time_str = schedule::format_time(self.current_time);
        backend.draw_text(
            &format!("{date_str}  |  {time_str}"),
            cx + 6,
            cy + 3,
            at.font_hint,
            self.colors.date_text,
        )?;
        let ch_info = self.build_channel_info();
        backend.draw_text(
            &ch_info,
            cx + 6,
            cy + 3 + at.font_hint as i32 + 2,
            at.font_small,
            self.colors.channel_label,
        )?;
        let now_title = self.build_now_playing_title();
        backend.draw_text(
            &now_title,
            cx + 6,
            cy + 3 + at.font_hint as i32 + at.font_small as i32 + 4,
            at.font_hint,
            self.colors.playing_text,
        )?;

        // Preview box (right side of header).
        let preview_w = (cw * 30 / 100).max(60);
        let preview_h = header_h.saturating_sub(8);
        let preview_x = cx + cw as i32 - preview_w as i32 - 4;
        let preview_y = cy + 4;
        backend.fill_rect(preview_x, preview_y, preview_w, preview_h, self.colors.bg)?;
        if let Some(tex) = self.preview_texture {
            backend.blit(
                tex,
                preview_x + 1,
                preview_y + 1,
                preview_w.saturating_sub(2),
                preview_h.saturating_sub(2),
            )?;
        }

        // Time header.
        let time_y = cy + header_h as i32;
        backend.fill_rect(cx, time_y, cw, time_h, self.colors.time_header_bg)?;
        backend.draw_text(
            "TIME:",
            cx + 4,
            time_y + 2,
            at.font_hint,
            self.colors.time_label,
        )?;

        let grid_start = self.grid_start_time();
        let slot_w = grid_w / VISIBLE_TIME_SLOTS as u32;
        for col in 0..VISIBLE_TIME_SLOTS {
            let slot_time = grid_start + col as u64 * SLOT_DURATION;
            let col_x = cx + label_w as i32 + col as i32 * slot_w as i32;
            backend.draw_text(
                &schedule::format_time(slot_time),
                col_x + 4,
                time_y + 2,
                at.font_hint,
                self.colors.time_header,
            )?;
        }

        // Channel rows.
        let grid_y = time_y + time_h as i32;
        let grid_end = grid_start + VISIBLE_TIME_SLOTS as u64 * SLOT_DURATION;
        let total_secs = (VISIBLE_TIME_SLOTS as u64 * SLOT_DURATION) as f64;

        for vis_row in 0..VISIBLE_ROWS {
            let ch_idx = self.scroll_offset + vis_row;
            if ch_idx >= self.channels.len() {
                break;
            }
            let row_y = grid_y + vis_row as i32 * row_h as i32;
            let is_sel = ch_idx == self.selected_channel;

            if is_sel {
                backend.fill_rect(cx, row_y, cw, row_h, self.colors.selected_bg)?;
            }

            let chan = &self.channels[ch_idx];
            let label = format!("[CH {}\n{}]", chan.number, chan.call_sign);
            let lbl_color = if is_sel {
                self.colors.selected_text
            } else {
                self.colors.channel_label
            };
            backend.draw_text(&label, cx + 4, row_y + 3, at.font_hint, lbl_color)?;

            // Grid line.
            backend.fill_rect(cx, row_y + row_h as i32 - 1, cw, 1, self.colors.grid_line)?;

            // Program cells.
            let slots = if let Some(Some(cached)) = self.cached_schedules.get(ch_idx) {
                cached.range(grid_start, grid_end)
            } else {
                Vec::new()
            };

            for slot in slots.iter().take(MAX_CELLS) {
                let ep_start = slot.start_time.max(grid_start);
                let ep_end = (slot.start_time + slot.episode.duration_secs as u64).min(grid_end);
                let x_frac = (ep_start - grid_start) as f64 / total_secs;
                let w_frac = (ep_end - ep_start) as f64 / total_secs;
                let cell_x = cx + label_w as i32 + (x_frac * grid_w as f64) as i32;
                let cell_w = (w_frac * grid_w as f64) as u32;

                if cell_w <= 8 {
                    continue;
                }

                let bg = if is_sel {
                    self.colors.selected_bg
                } else {
                    self.colors.cell_bg
                };
                backend.fill_rect(
                    cell_x,
                    row_y + 1,
                    cell_w.saturating_sub(1),
                    row_h.saturating_sub(2),
                    bg,
                )?;

                let txt_color = if is_sel {
                    self.colors.selected_text
                } else {
                    self.colors.program_text
                };
                let time_label = schedule::format_time(ep_start);
                let max_chars = (cell_w as usize / 6).saturating_sub(1);
                let upper_title = slot.episode.title.to_uppercase();
                let is_now = slot.start_time <= self.current_time
                    && slot.start_time + slot.episode.duration_secs as u64 > self.current_time;
                let title = if is_now && is_sel {
                    format!(
                        "* {}",
                        truncate_title(&upper_title, max_chars.saturating_sub(2),),
                    )
                } else {
                    truncate_title(&upper_title, max_chars)
                };
                backend.draw_text(
                    &format!("{time_label}\n{title}"),
                    cell_x + 3,
                    row_y + 3,
                    at.font_hint,
                    txt_color,
                )?;
            }
        }

        // Footer.
        let ftr_y = cy + ch as i32 - footer_h as i32;
        backend.fill_rect(cx, ftr_y, cw, footer_h, self.colors.footer_bg)?;
        let nav = format!(
            "[UP/DOWN SELECT]  [LEFT/RIGHT TIME]  [PAGE {}/{}]    [GUIDE]",
            self.current_page(),
            self.total_pages(),
        );
        backend.draw_text(&nav, cx + 6, ftr_y + 2, at.font_hint, self.colors.dim_text)?;

        Ok(())
    }

    // ---------------------------------------------------------------
    // Hide
    // ---------------------------------------------------------------

    /// Hide all TV guide SDI objects.
    pub fn hide_sdi(sdi: &mut SdiRegistry) {
        // New layout names.
        let fixed = [
            "tv_hdr_bg",
            "tv_hdr_grad",
            "tv_hdr_date",
            "tv_hdr_time",
            "tv_hdr_ch_info",
            "tv_hdr_location",
            "tv_hdr_currently",
            "tv_hdr_now_title",
            "tv_hdr_now_detail",
            "tv_hdr_preview_outer",
            "tv_hdr_preview_bg",
            "tv_hdr_preview_vid",
            "tv_hdr_live_badge",
            "tv_hdr_live_text",
            "tv_time_bg",
            "tv_time_label_bg",
            "tv_sel_bg",
            "tv_ftr_bg",
            "tv_ftr_nav",
            "tv_ftr_page",
            "tv_ftr_guide",
        ];
        for name in &fixed {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }

        // Legacy names (backward compat with old layout).
        let legacy = [
            "tv_bg",
            "tv_header_bg",
            "tv_header_text",
            "tv_header_playing",
            "tv_footer_bg",
            "tv_footer_text",
        ];
        for name in &legacy {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }

        // Time slot headers.
        for col in 0..VISIBLE_TIME_SLOTS {
            for prefix in &["tv_time_", "tv_timebg_"] {
                let name = format!("{prefix}{col}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        }

        // Channel rows (VISIBLE_ROWS visible + up to 10 legacy rows).
        let max_rows = VISIBLE_ROWS.max(10);
        for row in 0..max_rows {
            for suffix in &["_bg", "_label", "_line"] {
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
                let name = format!("tv_row_{row}_cbg_{ci}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        }
    }

    /// Handle a click at content-local coordinates.
    ///
    /// `cw`/`ch` are the content area dimensions (needed to recompute layout).
    /// Returns `Some(TuneRequest)` if the click tuned a channel (caller should
    /// enter fullscreen), `None` if it only selected a channel or missed.
    pub fn handle_click(
        &mut self,
        lx: i32,
        ly: i32,
        _cw: u32,
        ch: u32,
        fullscreen: bool,
    ) -> Option<TuneRequest> {
        if self.channels.is_empty() || ly < 0 || lx < 0 {
            return None;
        }

        // Recompute layout from content dimensions.
        // Fullscreen uses SDI layout minimums (update_sdi); windowed uses
        // draw_windowed minimums. Content-local: no status_h/bottom_h offset
        // since those are outside the window content area.
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
    use super::*;
    use crate::apps::tv_guide::channel::{ChannelConfig, DEFAULT_CHANNELS_TOML};

    #[test]
    fn new_guide_state() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.channels.len(), 5);
        assert_eq!(state.catalogs.len(), 5);
        assert_eq!(state.selected_channel, 0);
        assert_eq!(state.scroll_offset, 0);
        assert!(state.tuned_channel.is_none());
    }

    #[test]
    fn navigation() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());

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
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        for _ in 0..100 {
            state.select_down();
        }
        assert_eq!(state.selected_channel, 4); // 5 channels, max index 4.
    }

    #[test]
    fn navigation_auto_scroll() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        // 5 channels, VISIBLE_ROWS=5, so no scroll needed.
        for _ in 0..4 {
            state.select_down();
        }
        assert_eq!(state.selected_channel, 4);
        assert_eq!(state.scroll_offset, 0);

        // Go back up.
        state.select_up();
        assert_eq!(state.selected_channel, 3);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn paging_methods() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.current_page(), 1);
        assert_eq!(state.total_pages(), 1);
    }

    #[test]
    fn paging_empty_channels() {
        let config: ChannelConfig = toml::from_str("channel = []").unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.current_page(), 1);
        assert_eq!(state.total_pages(), 1);
    }

    #[test]
    fn time_scroll() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
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
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        let lines = state.text_content();
        assert!(lines.iter().any(|l| l.contains("TV Guide")));
        assert!(lines.iter().any(|l| l.contains("Loading")));
    }

    #[test]
    fn tune_without_catalog() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
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
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        state.fetch_attempted = true;
        state.fetch_error = Some("network timeout".to_string());
        let lines = state.text_content();
        assert!(lines.iter().any(|l| l.contains("Error: network timeout")));
        assert!(!lines.iter().any(|l| l.contains("Loading")));
    }

    #[test]
    fn text_content_after_partial_load() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
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
            format: "MPEG4".into(),
            original: None,
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
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
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
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());

        let mut catalog = super::super::catalog::ChannelCatalog::new(state.channels[0].number);
        catalog.add_episodes(vec![super::super::catalog::VideoEpisode {
            item_id: "test-item".to_string(),
            filename: "ep1.mp4".to_string(),
            title: "Test Episode".to_string(),
            duration_secs: 3600.0,
            width: 640,
            height: 480,
            size_bytes: 1000,
            format: "MPEG4".into(),
            original: None,
        }]);
        state.catalogs[0] = Some(catalog);
        state.rebuild_cached_schedule(0);

        // Tune should now work for channel 0.
        let req = state.tune();
        assert!(req.is_some());
        let req = req.unwrap();
        assert_eq!(req.episode.title, "Test Episode");
    }

    #[test]
    fn now_playing_detail_with_content() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());

        let mut catalog = super::super::catalog::ChannelCatalog::new(state.channels[0].number);
        catalog.add_episodes(vec![super::super::catalog::VideoEpisode {
            item_id: "test".to_string(),
            filename: "ep.mp4".to_string(),
            title: "Test".to_string(),
            duration_secs: 1800.0,
            width: 640,
            height: 480,
            size_bytes: 1000,
            format: "MPEG4".into(),
            original: None,
        }]);
        state.catalogs[0] = Some(catalog);
        state.rebuild_cached_schedule(0);

        let detail = state.build_now_playing_detail();
        assert!(detail.contains("640x480"));
        assert!(detail.contains("remaining"));
    }

    #[test]
    fn now_playing_detail_empty() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        let detail = state.build_now_playing_detail();
        assert!(detail.is_empty());
    }

    #[test]
    fn scroll_offset_with_many_channels() {
        // Build a config with 12 channels.
        let mut toml_str = String::new();
        for i in 0..12 {
            toml_str.push_str(&format!(
                "[[channel]]\nnumber = {}\ncall_sign = \"C{i}\"\n\
                 name = \"Channel {i}\"\ngenre = \"test\"\n\
                 [[channel.source]]\nitem_id = \"test-{i}\"\n\n",
                i + 1,
            ));
        }
        let config: ChannelConfig = toml::from_str(&toml_str).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.channels.len(), 12);

        // Navigate down past VISIBLE_ROWS.
        for _ in 0..7 {
            state.select_down();
        }
        assert_eq!(state.selected_channel, 7);
        // scroll_offset should have adjusted so selected is visible.
        assert!(state.selected_channel < state.scroll_offset + VISIBLE_ROWS);
        assert!(state.selected_channel >= state.scroll_offset);

        // Navigate back up.
        for _ in 0..7 {
            state.select_up();
        }
        assert_eq!(state.selected_channel, 0);
        assert_eq!(state.scroll_offset, 0);
    }

    // -- handle_click tests --

    /// Compute grid_y for a given content height (mirrors handle_click layout).
    fn grid_y_for(ch: u32) -> u32 {
        let usable_h = ch;
        let header_h = (usable_h * 20 / 100).max(60);
        let time_header_h = (usable_h * 4 / 100).max(20);
        header_h + time_header_h
    }

    /// Compute row height for a given content height and channel count.
    fn row_h_for(ch: u32, num_channels: usize) -> u32 {
        let usable_h = ch;
        let header_h = (usable_h * 20 / 100).max(60);
        let time_header_h = (usable_h * 4 / 100).max(20);
        let footer_h = (usable_h * 5 / 100).max(18);
        let grid_h = usable_h.saturating_sub(header_h + time_header_h + footer_h);
        let row_count = num_channels.clamp(1, VISIBLE_ROWS) as u32;
        (grid_h / row_count).max(20)
    }

    #[test]
    fn click_selects_channel() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.selected_channel, 0);

        let ch = 600u32;
        let cw = 800u32;
        let gy = grid_y_for(ch);
        let rh = row_h_for(ch, state.channels.len());

        // Click on row 2 (third channel).
        let ly = (gy + rh * 2 + rh / 2) as i32;
        let result = state.handle_click(100, ly, cw, ch, true);
        assert!(result.is_none(), "first click should select, not tune");
        assert_eq!(state.selected_channel, 2);
    }

    #[test]
    fn click_already_selected_tunes() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());

        // Inject a catalog so tune() can succeed.
        let mut catalog = super::super::catalog::ChannelCatalog::new(state.channels[0].number);
        catalog.add_episodes(vec![super::super::catalog::VideoEpisode {
            item_id: "click-test".to_string(),
            filename: "ep.mp4".to_string(),
            title: "Click Test".to_string(),
            duration_secs: 1800.0,
            width: 640,
            height: 480,
            size_bytes: 1000,
            format: "MPEG4".into(),
            original: None,
        }]);
        state.catalogs[0] = Some(catalog);
        state.rebuild_cached_schedule(0);

        let ch = 600u32;
        let cw = 800u32;
        let gy = grid_y_for(ch);
        let rh = row_h_for(ch, state.channels.len());

        // Channel 0 is already selected. Click on row 0.
        let ly = (gy + rh / 2) as i32;
        let result = state.handle_click(100, ly, cw, ch, true);
        assert!(result.is_some(), "second click on selected should tune");
        assert_eq!(result.unwrap().episode.title, "Click Test");
    }

    #[test]
    fn click_outside_grid_ignored() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());

        let ch = 600u32;
        let cw = 800u32;

        // Click in header area (y=10, well above grid).
        let result = state.handle_click(100, 10, cw, ch, true);
        assert!(result.is_none());
        assert_eq!(state.selected_channel, 0);

        // Click below grid area (y=ch-1, in footer).
        let result = state.handle_click(100, ch as i32 - 1, cw, ch, true);
        assert!(result.is_none());
        assert_eq!(state.selected_channel, 0);
    }

    #[test]
    fn click_negative_coords_ignored() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        let result = state.handle_click(-10, -5, 800, 600, true);
        assert!(result.is_none());
        assert_eq!(state.selected_channel, 0);
    }

    #[test]
    fn click_empty_channels_ignored() {
        let config: ChannelConfig = toml::from_str("channel = []").unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        let result = state.handle_click(100, 200, 800, 600, true);
        assert!(result.is_none());
    }

    #[test]
    fn paging_with_many_channels() {
        let mut toml_str = String::new();
        for i in 0..12 {
            toml_str.push_str(&format!(
                "[[channel]]\nnumber = {}\ncall_sign = \"C{i}\"\n\
                 name = \"Ch {i}\"\ngenre = \"t\"\n\
                 [[channel.source]]\nitem_id = \"t-{i}\"\n\n",
                i + 1,
            ));
        }
        let config: ChannelConfig = toml::from_str(&toml_str).unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.total_pages(), 3); // ceil(12/5) = 3
        assert_eq!(state.current_page(), 1);
    }
}
