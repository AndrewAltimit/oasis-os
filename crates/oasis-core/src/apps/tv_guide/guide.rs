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

// --- Retro CRT color palette ---
const COLOR_BG: Color = Color::rgba(10, 22, 40, 255);
const COLOR_GRID_LINE: Color = Color::rgba(26, 58, 92, 255);
const COLOR_HEADER_BG: Color = Color::rgba(12, 25, 50, 255);
const COLOR_HEADER_DARK: Color = Color::rgba(8, 18, 38, 255);
const COLOR_TIME_HEADER_BG: Color = Color::rgba(15, 35, 65, 255);
const COLOR_TIME_HEADER: Color = Color::rgba(0, 204, 255, 255);
const COLOR_CHANNEL_LABEL: Color = Color::rgba(200, 220, 240, 255);
const COLOR_PROGRAM_TEXT: Color = Color::rgba(192, 216, 232, 255);
const COLOR_SELECTED_BG: Color = Color::rgba(255, 140, 0, 180);
const COLOR_SELECTED_TEXT: Color = Color::rgba(255, 255, 255, 255);
const COLOR_DIM_TEXT: Color = Color::rgba(100, 130, 160, 255);
const COLOR_PLAYING_TEXT: Color = Color::rgba(0, 221, 255, 255);
const COLOR_CELL_BG: Color = Color::rgba(15, 30, 55, 255);
const COLOR_CELL_BORDER: Color = Color::rgba(26, 58, 92, 255);
const COLOR_LIVE_BADGE: Color = Color::rgba(220, 40, 40, 255);
const COLOR_DATE_TEXT: Color = Color::rgba(180, 200, 220, 255);
const COLOR_FOOTER_BG: Color = Color::rgba(12, 25, 45, 255);

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
    /// Error message from a failed catalog fetch.
    pub fetch_error: Option<String>,
    /// First visible channel row (for paging when channels > VISIBLE_ROWS).
    pub scroll_offset: usize,
    /// Texture for the in-app video preview (set by the video player).
    pub preview_texture: Option<TextureId>,
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
            sdi_names: SdiNames::new(),
            fetch_attempted: false,
            fetch_error: None,
            scroll_offset: 0,
            preview_texture: None,
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
            } else if self.fetch_attempted {
                lines.push("No content available from any channel.".to_string());
            } else {
                let dots = ".".repeat((self.current_time % 4) as usize + 1);
                lines.push(format!("Loading channel catalogs{dots}"));
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
            obj.color = COLOR_BG;
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
            obj.color = COLOR_HEADER_BG;
            obj.gradient_top = Some(COLOR_HEADER_BG);
            obj.gradient_bottom = Some(COLOR_HEADER_DARK);
            obj.visible = true;
            obj.z = 101;
        }

        // Date line.
        let date_text = schedule::format_date(self.current_time);
        let time_text = schedule::format_time(self.current_time);
        ensure_obj(sdi, "tv_hdr_date");
        if let Ok(obj) = sdi.get_mut("tv_hdr_date") {
            obj.text = Some(format!("{date_text}  |  {time_text}"));
            obj.x = 10;
            obj.y = y0 + 4;
            obj.font_size = at.font_small;
            obj.text_color = COLOR_DATE_TEXT;
            obj.visible = true;
            obj.z = 102;
        }

        // Channel info line.
        let ch_info = self.build_channel_info();
        ensure_obj(sdi, "tv_hdr_ch_info");
        if let Ok(obj) = sdi.get_mut("tv_hdr_ch_info") {
            obj.text = Some(ch_info);
            obj.x = 10;
            obj.y = y0 + 4 + at.font_small as i32 + 2;
            obj.font_size = at.font_body;
            obj.text_color = COLOR_CHANNEL_LABEL;
            obj.visible = true;
            obj.z = 102;
        }

        // Now playing title.
        let now_title = self.build_now_playing_title();
        ensure_obj(sdi, "tv_hdr_now_title");
        if let Ok(obj) = sdi.get_mut("tv_hdr_now_title") {
            obj.text = Some(now_title);
            obj.x = 10;
            obj.y = y0 + 4 + at.font_small as i32 + 2 + at.font_body as i32 + 2;
            obj.font_size = at.font_small;
            obj.text_color = COLOR_PLAYING_TEXT;
            obj.visible = true;
            obj.z = 102;
        }

        // Now playing detail (duration + resolution).
        let now_detail = self.build_now_playing_detail();
        ensure_obj(sdi, "tv_hdr_now_detail");
        if let Ok(obj) = sdi.get_mut("tv_hdr_now_detail") {
            obj.text = Some(now_detail);
            obj.x = 10;
            obj.y = y0 + 4 + at.font_small as i32 * 2 + at.font_body as i32 + 6;
            obj.font_size = at.font_hint;
            obj.text_color = COLOR_DIM_TEXT;
            obj.visible = true;
            obj.z = 102;
        }

        // Preview box outline (right side of header).
        let preview_w = (sw / 5).max(80);
        let preview_h = header_h.saturating_sub(16);
        let preview_x = sw as i32 - preview_w as i32 - 10;
        let preview_y = y0 + 8;

        ensure_obj(sdi, "tv_hdr_preview_bg");
        if let Ok(obj) = sdi.get_mut("tv_hdr_preview_bg") {
            obj.x = preview_x;
            obj.y = preview_y;
            obj.w = preview_w;
            obj.h = preview_h;
            obj.color = Color::rgba(5, 12, 25, 255);
            obj.stroke_color = Some(COLOR_CELL_BORDER);
            obj.stroke_width = Some(1);
            obj.visible = true;
            obj.z = 102;
        }

        // Video preview texture (rendered inset 1px inside the preview box).
        ensure_obj(sdi, "tv_hdr_preview_vid");
        if let Ok(obj) = sdi.get_mut("tv_hdr_preview_vid") {
            if let Some(tex) = self.preview_texture {
                obj.x = preview_x + 1;
                obj.y = preview_y + 1;
                obj.w = preview_w.saturating_sub(2);
                obj.h = preview_h.saturating_sub(2);
                obj.texture = Some(tex);
                obj.visible = true;
                obj.z = 103;
            } else {
                obj.visible = false;
                obj.texture = None;
            }
        }

        // LIVE badge (red rounded rect, shown when tuned or has content).
        let has_content = self
            .cached_schedules
            .get(self.selected_channel)
            .and_then(|s| s.as_ref())
            .and_then(|c| c.at(self.current_time))
            .is_some();

        ensure_obj(sdi, "tv_hdr_live_badge");
        if let Ok(obj) = sdi.get_mut("tv_hdr_live_badge") {
            obj.x = preview_x + preview_w as i32 - 38;
            obj.y = preview_y + 4;
            obj.w = 34;
            obj.h = 14;
            obj.color = COLOR_LIVE_BADGE;
            obj.border_radius = Some(3);
            obj.visible = has_content;
            obj.z = 103;
        }

        ensure_obj(sdi, "tv_hdr_live_text");
        if let Ok(obj) = sdi.get_mut("tv_hdr_live_text") {
            obj.text = Some("LIVE".to_string());
            obj.x = preview_x + preview_w as i32 - 34;
            obj.y = preview_y + 5;
            obj.font_size = at.font_hint;
            obj.text_color = COLOR_SELECTED_TEXT;
            obj.visible = has_content;
            obj.z = 104;
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
            obj.color = COLOR_TIME_HEADER_BG;
            obj.visible = true;
            obj.z = 101;
        }

        // "TIME" label.
        ensure_obj(sdi, "tv_time_label_bg");
        if let Ok(obj) = sdi.get_mut("tv_time_label_bg") {
            obj.text = Some("TIME".to_string());
            obj.x = 4;
            obj.y = y0 + 3;
            obj.font_size = at.font_hint;
            obj.text_color = COLOR_TIME_HEADER;
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
                obj.color = COLOR_TIME_HEADER_BG;
                obj.stroke_color = Some(COLOR_CELL_BORDER);
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
                    obj.text_color = COLOR_TIME_HEADER;
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
                    COLOR_SELECTED_BG
                } else {
                    COLOR_BG
                };
                obj.visible = has_channel;
                obj.z = 101;
            }

            // Channel label.
            let label_name = &self.sdi_names.row_labels[vis_row];
            ensure_obj(sdi, label_name);
            if let Ok(obj) = sdi.get_mut(label_name) {
                if has_channel {
                    let ch = &self.channels[ch_idx];
                    obj.text = Some(format!("CH {:>2}\n{}", ch.number, ch.call_sign));
                } else {
                    obj.text = None;
                }
                obj.x = 4;
                obj.y = row_y + 4;
                obj.font_size = at.font_small;
                obj.text_color = if is_selected {
                    COLOR_SELECTED_TEXT
                } else {
                    COLOR_CHANNEL_LABEL
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
                obj.color = COLOR_GRID_LINE;
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
                        COLOR_SELECTED_BG
                    } else {
                        COLOR_CELL_BG
                    };
                    obj.stroke_color = Some(COLOR_CELL_BORDER);
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
                    let title = truncate_title(&slot.episode.title, max_chars);

                    obj.text = Some(format!("{time_str}\n{title}"));
                    obj.x = cell_x + 3;
                    obj.y = row_y + 3;
                    obj.font_size = at.font_hint;
                    obj.text_color = if is_selected {
                        COLOR_SELECTED_TEXT
                    } else {
                        COLOR_PROGRAM_TEXT
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
            obj.color = COLOR_SELECTED_BG;
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
            obj.color = COLOR_FOOTER_BG;
            obj.visible = true;
            obj.z = 101;
        }

        // Navigation hints (left).
        ensure_obj(sdi, "tv_ftr_nav");
        if let Ok(obj) = sdi.get_mut("tv_ftr_nav") {
            obj.text = Some("Up/Down=SELECT  L/R=TIME  Confirm=TUNE  Select=RETRY".to_string());
            obj.x = 8;
            obj.y = footer_y as i32 + 3;
            obj.font_size = at.font_hint;
            obj.text_color = COLOR_DIM_TEXT;
            obj.visible = true;
            obj.z = 102;
        }

        // Page indicator (center-right).
        let page_text = format!("PAGE {}/{}", self.current_page(), self.total_pages(),);
        ensure_obj(sdi, "tv_ftr_page");
        if let Ok(obj) = sdi.get_mut("tv_ftr_page") {
            obj.text = Some(page_text);
            obj.x = sw as i32 - 140;
            obj.y = footer_y as i32 + 3;
            obj.font_size = at.font_hint;
            obj.text_color = COLOR_DIM_TEXT;
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
            obj.text_color = COLOR_TIME_HEADER;
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

    /// Build "Now Playing" title line (e.g. "Now: Title (12:45 left)").
    fn build_now_playing_title(&self) -> String {
        let idx = self.selected_channel;
        if let Some(Some(cached)) = self.cached_schedules.get(idx)
            && let Some(slot) = cached.at(self.current_time)
        {
            let remaining = schedule::format_duration(slot.remaining_secs as f64);
            return format!(
                "Now: {} ({remaining} left)",
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
            let dots = ".".repeat((self.current_time % 4) as usize + 1);
            return format!("Loading catalog{dots}");
        }
        "No content available".to_string()
    }

    /// Build detail line for now-playing (e.g. "Duration: 30:00 | 640x480").
    fn build_now_playing_detail(&self) -> String {
        let idx = self.selected_channel;
        if let Some(Some(cached)) = self.cached_schedules.get(idx)
            && let Some(slot) = cached.at(self.current_time)
        {
            let dur = schedule::format_duration(slot.episode.duration_secs);
            return format!(
                "Duration: {dur} | {}x{}",
                slot.episode.width, slot.episode.height,
            );
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
        backend.fill_rect(cx, cy, cw, ch, COLOR_BG)?;

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
        backend.fill_rect(cx, cy, cw, header_h, COLOR_HEADER_BG)?;
        let date_str = schedule::format_date(self.current_time);
        let time_str = schedule::format_time(self.current_time);
        backend.draw_text(
            &format!("{date_str}  |  {time_str}"),
            cx + 6,
            cy + 3,
            at.font_hint,
            COLOR_DATE_TEXT,
        )?;
        let ch_info = self.build_channel_info();
        backend.draw_text(
            &ch_info,
            cx + 6,
            cy + 3 + at.font_hint as i32 + 2,
            at.font_small,
            COLOR_CHANNEL_LABEL,
        )?;
        let now_title = self.build_now_playing_title();
        backend.draw_text(
            &now_title,
            cx + 6,
            cy + 3 + at.font_hint as i32 + at.font_small as i32 + 4,
            at.font_hint,
            COLOR_PLAYING_TEXT,
        )?;

        // Time header.
        let time_y = cy + header_h as i32;
        backend.fill_rect(cx, time_y, cw, time_h, COLOR_TIME_HEADER_BG)?;
        backend.draw_text("TIME", cx + 4, time_y + 2, at.font_hint, COLOR_TIME_HEADER)?;

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
                COLOR_TIME_HEADER,
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
                backend.fill_rect(cx, row_y, cw, row_h, COLOR_SELECTED_BG)?;
            }

            let chan = &self.channels[ch_idx];
            let label = format!("CH{:>2}\n{}", chan.number, chan.call_sign);
            let lbl_color = if is_sel {
                COLOR_SELECTED_TEXT
            } else {
                COLOR_CHANNEL_LABEL
            };
            backend.draw_text(&label, cx + 4, row_y + 3, at.font_hint, lbl_color)?;

            // Grid line.
            backend.fill_rect(cx, row_y + row_h as i32 - 1, cw, 1, COLOR_GRID_LINE)?;

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
                    COLOR_SELECTED_BG
                } else {
                    COLOR_CELL_BG
                };
                backend.fill_rect(
                    cell_x,
                    row_y + 1,
                    cell_w.saturating_sub(1),
                    row_h.saturating_sub(2),
                    bg,
                )?;

                let txt_color = if is_sel {
                    COLOR_SELECTED_TEXT
                } else {
                    COLOR_PROGRAM_TEXT
                };
                let time_label = schedule::format_time(ep_start);
                let max_chars = (cell_w as usize / 6).saturating_sub(1);
                let title = truncate_title(&slot.episode.title, max_chars);
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
        backend.fill_rect(cx, ftr_y, cw, footer_h, COLOR_FOOTER_BG)?;
        let nav = format!(
            "Up/Down=SELECT  L/R=TIME          PAGE {}/{}    [GUIDE]",
            self.current_page(),
            self.total_pages(),
        );
        backend.draw_text(&nav, cx + 6, ftr_y + 2, at.font_hint, COLOR_DIM_TEXT)?;

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
            "tv_hdr_ch_info",
            "tv_hdr_now_title",
            "tv_hdr_now_detail",
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
        assert_eq!(state.scroll_offset, 0);
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
    fn navigation_auto_scroll() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config);
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
        let state = TvGuideState::new(&config);
        assert_eq!(state.current_page(), 1);
        assert_eq!(state.total_pages(), 1);
    }

    #[test]
    fn paging_empty_channels() {
        let config: ChannelConfig = toml::from_str("channel = []").unwrap();
        let state = TvGuideState::new(&config);
        assert_eq!(state.current_page(), 1);
        assert_eq!(state.total_pages(), 1);
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
        assert!(lines.iter().any(|l| l.contains("Loading")));
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

    #[test]
    fn now_playing_detail_with_content() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config);

        let mut catalog = super::super::catalog::ChannelCatalog::new(state.channels[0].number);
        catalog.add_episodes(vec![super::super::catalog::VideoEpisode {
            item_id: "test".to_string(),
            filename: "ep.mp4".to_string(),
            title: "Test".to_string(),
            duration_secs: 1800.0,
            width: 640,
            height: 480,
            size_bytes: 1000,
        }]);
        state.catalogs[0] = Some(catalog);
        state.rebuild_cached_schedule(0);

        let detail = state.build_now_playing_detail();
        assert!(detail.contains("Duration:"));
        assert!(detail.contains("640x480"));
    }

    #[test]
    fn now_playing_detail_empty() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let state = TvGuideState::new(&config);
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
        let mut state = TvGuideState::new(&config);
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
        let state = TvGuideState::new(&config);
        assert_eq!(state.total_pages(), 3); // ceil(12/5) = 3
        assert_eq!(state.current_page(), 1);
    }
}
