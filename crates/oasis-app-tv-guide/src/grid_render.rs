use oasis_sdi::SdiRegistry;
use oasis_skin::active_theme::ActiveTheme;
use oasis_types::backend::Color;

use crate::grid_layout::{
    MAX_CELLS, SLOT_DURATION, VISIBLE_ROWS, VISIBLE_TIME_SLOTS, VOLUME_BAR_WIDE_THRESHOLD,
    VOLUME_TICK_POSITIONS, ensure_obj, truncate_title, volume_bar_label, volume_bar_rect,
};
use crate::grid_state::TvGuideState;
use crate::schedule;

impl TvGuideState {
    // ---------------------------------------------------------------
    // SDI rendering — 1980s EPG grid
    // ---------------------------------------------------------------

    /// Render the TV guide grid to SDI objects.
    pub fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.update_time();

        // Re-derive theme colors so a skin switch while the guide is open
        // takes effect immediately (they were previously captured once at
        // construction and went stale after a theme swap).
        self.colors = crate::grid_layout::TvGuideColors::from_theme(at);

        // Rebuild cached schedules for any newly-loaded catalogs.
        for i in 0..self.catalogs.len() {
            if self.catalogs[i].is_some() && self.cached_schedules[i].is_none() {
                self.rebuild_cached_schedule(i);
            }
        }

        // Expanded video mode: show only the video fullscreen.
        if self.video_expanded {
            self.draw_expanded_video(sdi, at);
            return;
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

        // Volume bar in footer (only visible when tuned).
        if self.tuned_channel.is_some() {
            self.draw_volume_bar_sdi(sdi, at, sw, usable_h, false);
        } else {
            // Hide volume bar when not tuned.
            for name in &["tv_vol_bg", "tv_vol_fill", "tv_vol_label"] {
                if let Ok(obj) = sdi.get_mut(name) {
                    obj.visible = false;
                }
            }
            for i in 0..VOLUME_TICK_POSITIONS.len() {
                let name = format!("tv_vol_tick_{i}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        }
    }

    /// Render expanded (fullscreen) video mode via SDI.
    ///
    /// Hides the EPG grid and shows only the video texture filling the
    /// content area, with a small channel info overlay at the bottom.
    fn draw_expanded_video(&self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        let sw = at.screen_w;
        let sh = at.screen_h;
        let status_h = at.statusbar_height;
        let bottom_h = at.bottombar_height;
        let usable_h = sh.saturating_sub(status_h + bottom_h);

        // Black background.
        ensure_obj(sdi, "tv_hdr_bg");
        if let Ok(obj) = sdi.get_mut("tv_hdr_bg") {
            obj.x = 0;
            obj.y = 0;
            obj.w = sw;
            obj.h = sh;
            obj.color = Color::rgba(0, 0, 0, 255);
            obj.visible = true;
            obj.z = 100;
        }

        // Video texture fills the content area.
        ensure_obj(sdi, "tv_hdr_preview_vid");
        if let Ok(obj) = sdi.get_mut("tv_hdr_preview_vid") {
            if let Some(tex) = self.preview_texture {
                obj.x = 0;
                obj.y = status_h as i32;
                obj.w = sw;
                obj.h = usable_h;
                obj.texture = Some(tex);
                obj.visible = true;
                obj.z = 110;
            } else {
                obj.visible = false;
                obj.texture = None;
            }
        }

        // Loading text (centered) when no texture yet.
        let is_loading = self.tuned_channel.is_some() && self.preview_texture.is_none();
        let dots = match self.current_time % 4 {
            0 => "",
            1 => ".",
            2 => "..",
            _ => "...",
        };
        let loading_text = if let Some(ref status) = self.download_status {
            status.clone()
        } else {
            format!("Loading{dots}")
        };
        ensure_obj(sdi, "tv_hdr_loading_text");
        if let Ok(obj) = sdi.get_mut("tv_hdr_loading_text") {
            obj.text = Some(loading_text);
            obj.x = sw as i32 / 2 - 30;
            obj.y = (status_h + usable_h / 2) as i32;
            obj.font_size = at.font_body;
            obj.text_color = self.colors.dim_text;
            obj.visible = is_loading;
            obj.z = 111;
        }

        // Small channel overlay at the bottom.
        let overlay_h = 20u32;
        let overlay_y = (sh - bottom_h - overlay_h) as i32;
        ensure_obj(sdi, "tv_expanded_overlay_bg");
        if let Ok(obj) = sdi.get_mut("tv_expanded_overlay_bg") {
            obj.x = 0;
            obj.y = overlay_y;
            obj.w = sw;
            obj.h = overlay_h;
            obj.color = Color::rgba(0, 0, 0, 160);
            obj.visible = true;
            obj.z = 112;
        }

        let ch_info = self.build_channel_info();
        let now_title = self.build_now_playing_title();
        let overlay_text = format!("{ch_info}  |  {now_title}");
        ensure_obj(sdi, "tv_expanded_overlay_text");
        if let Ok(obj) = sdi.get_mut("tv_expanded_overlay_text") {
            obj.text = Some(overlay_text);
            obj.x = 8;
            obj.y = overlay_y + 3;
            obj.font_size = at.font_hint;
            obj.text_color = self.colors.channel_label;
            obj.visible = true;
            obj.z = 113;
        }

        // Volume bar in expanded overlay.
        self.draw_volume_bar_sdi(sdi, at, sw, usable_h, true);

        // Hide all EPG-specific SDI objects that are not reused above.
        for name in &[
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
            "tv_hdr_live_badge",
            "tv_hdr_live_text",
            "tv_time_bg",
            "tv_time_label_bg",
            "tv_sel_bg",
            "tv_sel_glow_top",
            "tv_sel_glow_bot",
            "tv_ftr_bg",
            "tv_ftr_nav",
            "tv_ftr_page",
            "tv_ftr_guide",
        ] {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
        for col in 0..VISIBLE_TIME_SLOTS {
            for prefix in &["tv_time_", "tv_timebg_"] {
                let name = format!("{prefix}{col}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        }
        for row in 0..VISIBLE_ROWS {
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

    /// Render the volume bar as SDI objects.
    ///
    /// Retro cable-TV remote display: dark blue track, amber fill,
    /// cobalt grid_line border, and tick marks at 0/25/50/75/100 when
    /// the bar is wide enough.
    fn draw_volume_bar_sdi(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        cw: u32,
        ch: u32,
        expanded: bool,
    ) {
        let vr = volume_bar_rect(cw, ch, expanded);
        let fill_w = (vr.w as f32 * self.volume as f32 / 100.0) as u32;
        let show_ticks = vr.w >= VOLUME_BAR_WIDE_THRESHOLD;

        // Track (dark blue) with cobalt border via stroke.
        ensure_obj(sdi, "tv_vol_bg");
        if let Ok(obj) = sdi.get_mut("tv_vol_bg") {
            obj.x = vr.x;
            obj.y = vr.y;
            obj.w = vr.w;
            obj.h = vr.h;
            obj.color = self.colors.time_header_bg;
            obj.stroke_color = Some(self.colors.grid_line);
            obj.stroke_width = Some(1);
            obj.border_radius = None;
            obj.visible = true;
            obj.z = 114;
        }

        // Filled portion (amber).
        ensure_obj(sdi, "tv_vol_fill");
        if let Ok(obj) = sdi.get_mut("tv_vol_fill") {
            obj.x = vr.x;
            obj.y = vr.y;
            obj.w = fill_w;
            obj.h = vr.h;
            obj.color = self.colors.time_label;
            obj.stroke_color = None;
            obj.border_radius = None;
            obj.visible = fill_w > 0;
            obj.z = 115;
        }

        // Label "VOLUME: X%" left of the bar (or short "VOL X%" on tight layouts).
        let (label, label_offset) = volume_bar_label(vr.w, self.volume);
        ensure_obj(sdi, "tv_vol_label");
        if let Ok(obj) = sdi.get_mut("tv_vol_label") {
            obj.text = Some(label);
            // Clamp to ≥ 0 so the label stays on-screen on very narrow canvases.
            obj.x = (vr.x - label_offset).max(0);
            obj.y = vr.y + (vr.h as i32 - at.font_hint as i32) / 2;
            obj.font_size = at.font_hint;
            obj.text_color = self.colors.time_label;
            obj.visible = true;
            obj.z = 116;
        }

        // Tick marks at 0/25/50/75/100 — only when bar is wide enough.
        // The 100% tick is drawn at `vr.w - 1` so it stays inside the bar's
        // right edge instead of one pixel past it.
        let last_tick_idx = VOLUME_TICK_POSITIONS.len().saturating_sub(1);
        for (i, pct) in VOLUME_TICK_POSITIONS.iter().enumerate() {
            let tick_name = format!("tv_vol_tick_{i}");
            ensure_obj(sdi, &tick_name);
            if let Ok(obj) = sdi.get_mut(&tick_name) {
                let tx = if i == last_tick_idx {
                    vr.x + vr.w as i32 - 1
                } else {
                    vr.x + ((*pct as i32) * vr.w as i32 / 100)
                };
                obj.x = tx;
                obj.y = vr.y + vr.h as i32 + 1;
                obj.w = 1;
                obj.h = 3;
                obj.color = self.colors.dim_text;
                obj.visible = show_ticks;
                obj.z = 116;
            }
        }
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
        let is_loading = self.tuned_channel.is_some() && self.preview_texture.is_none();
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

        // Loading indicator (visible while tuned but no video frame yet).
        let dots = match self.current_time % 4 {
            0 => "",
            1 => ".",
            2 => "..",
            _ => "...",
        };
        let loading_text = if let Some(ref status) = self.download_status {
            status.clone()
        } else {
            format!("Loading{dots}")
        };
        ensure_obj(sdi, "tv_hdr_loading_text");
        if let Ok(obj) = sdi.get_mut("tv_hdr_loading_text") {
            obj.text = Some(loading_text);
            obj.x = preview_x + preview_w as i32 / 2 - 20;
            obj.y = preview_y + preview_h as i32 / 2 - 4;
            obj.font_size = at.font_hint;
            obj.text_color = self.colors.dim_text;
            obj.visible = is_loading;
            obj.z = 105;
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
            obj.text_color = self.colors.live_badge_text;
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

            // Row background — solid amber for the selected row, deep
            // navy otherwise. No gradients (matches retro EPG flat fills).
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
                obj.gradient_top = None;
                obj.gradient_bottom = None;
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

                // Cell background — flat fills, with a single bright
                // grid-line on the left edge (drawn implicitly by the
                // adjacent cell or the channel-label column border).
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
                    obj.gradient_top = None;
                    obj.gradient_bottom = None;
                    obj.stroke_color = Some(self.colors.grid_line);
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
        // Solid amber bar that lerps between rows. Glow lines on top/bottom
        // are drawn separately so the highlight reads as a CRT-lit cell.
        ensure_obj(sdi, "tv_sel_bg");
        if let Ok(obj) = sdi.get_mut("tv_sel_bg") {
            let sel_y = grid_y as f32 + self.visual_selected * row_h as f32;
            obj.x = 0;
            obj.y = sel_y as i32;
            obj.w = sw;
            obj.h = row_h;
            obj.color = self.colors.selected_bg;
            obj.gradient_top = None;
            obj.gradient_bottom = None;
            obj.border_radius = None;
            obj.visible = !self.channels.is_empty();
            obj.z = 101;
        }

        // Top/bottom glow stripes (light amber).
        ensure_obj(sdi, "tv_sel_glow_top");
        if let Ok(obj) = sdi.get_mut("tv_sel_glow_top") {
            let sel_y = grid_y as f32 + self.visual_selected * row_h as f32;
            obj.x = 0;
            obj.y = sel_y as i32;
            obj.w = sw;
            obj.h = 2;
            obj.color = self.colors.selected_glow;
            obj.gradient_top = None;
            obj.gradient_bottom = None;
            obj.visible = !self.channels.is_empty();
            obj.z = 103;
        }
        ensure_obj(sdi, "tv_sel_glow_bot");
        if let Ok(obj) = sdi.get_mut("tv_sel_glow_bot") {
            let sel_y = grid_y as f32 + self.visual_selected * row_h as f32;
            obj.x = 0;
            obj.y = sel_y as i32 + row_h as i32 - 2;
            obj.w = sw;
            obj.h = 2;
            obj.color = self.colors.selected_glow;
            obj.gradient_top = None;
            obj.gradient_bottom = None;
            obj.visible = !self.channels.is_empty();
            obj.z = 103;
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
        backend: &mut dyn oasis_types::backend::SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        // Expanded video mode: fill the entire content area with video.
        if self.video_expanded {
            return self.draw_windowed_expanded(cx, cy, cw, ch, backend, at);
        }

        // Derive theme colors fresh each draw: `draw_windowed` takes
        // `&self`, so the cached `self.colors` (refreshed in `update_sdi`)
        // can't be updated here and would go stale after a skin switch
        // while the guide runs in a window.
        let colors = crate::grid_layout::TvGuideColors::from_theme(at);

        // Background.
        backend.fill_rect(cx, cy, cw, ch, colors.bg)?;

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
        backend.fill_rect(cx, cy, cw, header_h, colors.header_bg)?;
        let date_str = schedule::format_date(self.current_time);
        let time_str = schedule::format_time(self.current_time);
        backend.draw_text(
            &format!("{date_str}  |  {time_str}"),
            cx + 6,
            cy + 3,
            at.font_hint,
            colors.date_text,
        )?;
        let ch_info = self.build_channel_info();
        backend.draw_text(
            &ch_info,
            cx + 6,
            cy + 3 + at.font_hint as i32 + 2,
            at.font_small,
            colors.channel_label,
        )?;
        let now_title = self.build_now_playing_title();
        backend.draw_text(
            &now_title,
            cx + 6,
            cy + 3 + at.font_hint as i32 + at.font_small as i32 + 4,
            at.font_hint,
            colors.playing_text,
        )?;

        // Preview box (right side of header).
        let preview_w = (cw * 30 / 100).max(60);
        let preview_h = header_h.saturating_sub(8);
        let preview_x = cx + cw as i32 - preview_w as i32 - 4;
        let preview_y = cy + 4;
        backend.fill_rect(preview_x, preview_y, preview_w, preview_h, colors.bg)?;
        if let Some(tex) = self.preview_texture {
            backend.blit(
                tex,
                preview_x + 1,
                preview_y + 1,
                preview_w.saturating_sub(2),
                preview_h.saturating_sub(2),
            )?;
        } else if let Some(ref status) = self.download_status {
            // Show download progress text centered in the preview area.
            backend.draw_text(
                status,
                preview_x + 4,
                preview_y + preview_h as i32 / 2 - 4,
                at.font_hint,
                colors.program_text,
            )?;
        }

        // Time header.
        let time_y = cy + header_h as i32;
        backend.fill_rect(cx, time_y, cw, time_h, colors.time_header_bg)?;
        backend.draw_text("TIME:", cx + 4, time_y + 2, at.font_hint, colors.time_label)?;

        let grid_start = self.grid_start_time();
        let slot_w = grid_w / VISIBLE_TIME_SLOTS as u32;
        for col in 0..VISIBLE_TIME_SLOTS {
            let slot_time = grid_start + col as u64 * SLOT_DURATION;
            let col_x = cx + label_w as i32 + col as i32 * slot_w as i32;
            // Vertical separator between time-header columns.
            backend.fill_rect(col_x, time_y, 1, time_h, colors.grid_line)?;
            backend.draw_text(
                &schedule::format_time(slot_time),
                col_x + 4,
                time_y + 2,
                at.font_hint,
                colors.time_header,
            )?;
        }
        // Horizontal grid line below the time header.
        backend.fill_rect(cx, time_y + time_h as i32 - 1, cw, 1, colors.grid_line)?;

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
                backend.fill_rect(cx, row_y, cw, row_h, colors.selected_bg)?;
                // Top/bottom amber glow stripes on selected row.
                backend.fill_rect(cx, row_y, cw, 2, colors.selected_glow)?;
                backend.fill_rect(cx, row_y + row_h as i32 - 2, cw, 2, colors.selected_glow)?;
            }

            let chan = &self.channels[ch_idx];
            let label = format!("[CH {}\n{}]", chan.number, chan.call_sign);
            let lbl_color = if is_sel {
                colors.selected_text
            } else {
                colors.channel_label
            };
            backend.draw_text(&label, cx + 4, row_y + 3, at.font_hint, lbl_color)?;

            // Grid line.
            backend.fill_rect(cx, row_y + row_h as i32 - 1, cw, 1, colors.grid_line)?;

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
                    colors.selected_bg
                } else {
                    colors.cell_bg
                };
                backend.fill_rect(
                    cell_x,
                    row_y + 1,
                    cell_w.saturating_sub(1),
                    row_h.saturating_sub(2),
                    bg,
                )?;
                // Left grid-line for the cell — gives the EPG its grid look.
                backend.fill_rect(
                    cell_x,
                    row_y + 1,
                    1,
                    row_h.saturating_sub(2),
                    colors.grid_line,
                )?;

                let txt_color = if is_sel {
                    colors.selected_text
                } else {
                    colors.program_text
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
        backend.fill_rect(cx, ftr_y, cw, footer_h, colors.footer_bg)?;
        let nav = format!(
            "[UP/DOWN SELECT]  [LEFT/RIGHT TIME]  [PAGE {}/{}]    [GUIDE]",
            self.current_page(),
            self.total_pages(),
        );
        backend.draw_text(&nav, cx + 6, ftr_y + 2, at.font_hint, colors.dim_text)?;

        // Volume bar in footer when tuned.
        if self.tuned_channel.is_some() {
            self.draw_volume_bar_windowed(cx, cy, cw, ch, backend, at, false)?;
        }

        Ok(())
    }

    /// Draw expanded (fullscreen) video in windowed rendering mode.
    fn draw_windowed_expanded(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn oasis_types::backend::SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        // Fresh theme colors — see `draw_windowed` for why this can't use
        // the cached `self.colors`.
        let colors = crate::grid_layout::TvGuideColors::from_theme(at);

        // Black background.
        backend.fill_rect(cx, cy, cw, ch, Color::rgba(0, 0, 0, 255))?;

        // Video fills the content area.
        if let Some(tex) = self.preview_texture {
            backend.blit(tex, cx, cy, cw, ch)?;
        } else if self.tuned_channel.is_some() {
            let dots = match self.current_time % 4 {
                0 => "",
                1 => ".",
                2 => "..",
                _ => "...",
            };
            let loading_text = if let Some(ref status) = self.download_status {
                status.clone()
            } else {
                format!("Loading{dots}")
            };
            backend.draw_text(
                &loading_text,
                cx + cw as i32 / 2 - 30,
                cy + ch as i32 / 2 - 4,
                at.font_body,
                colors.dim_text,
            )?;
        }

        // Small channel overlay at the bottom.
        let overlay_h = 20u32;
        let overlay_y = cy + ch as i32 - overlay_h as i32;
        backend.fill_rect(cx, overlay_y, cw, overlay_h, Color::rgba(0, 0, 0, 160))?;
        let ch_info = self.build_channel_info();
        let now_title = self.build_now_playing_title();
        backend.draw_text(
            &format!("{ch_info}  |  {now_title}"),
            cx + 8,
            overlay_y + 3,
            at.font_hint,
            colors.channel_label,
        )?;

        // Volume bar in expanded overlay.
        self.draw_volume_bar_windowed(cx, cy, cw, ch, backend, at, true)?;

        Ok(())
    }

    /// Draw the volume bar using the backend's direct rendering.
    ///
    /// Retro cable-TV remote display: dark blue track, amber fill,
    /// cobalt grid_line border, and tick marks at 0/25/50/75/100 when
    /// the bar is wide enough.
    #[allow(clippy::too_many_arguments)]
    fn draw_volume_bar_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn oasis_types::backend::SdiBackend,
        at: &ActiveTheme,
        expanded: bool,
    ) -> oasis_types::error::Result<()> {
        // Fresh theme colors — see `draw_windowed` for why this can't use
        // the cached `self.colors`.
        let colors = crate::grid_layout::TvGuideColors::from_theme(at);
        let vr = volume_bar_rect(cw, ch, expanded);
        let fill_w = (vr.w as f32 * self.volume as f32 / 100.0) as u32;
        let bx = cx + vr.x;
        let by = cy + vr.y;

        // Track (dark blue).
        backend.fill_rect(bx, by, vr.w, vr.h, colors.time_header_bg)?;
        // Filled portion (amber).
        if fill_w > 0 {
            backend.fill_rect(bx, by, fill_w, vr.h, colors.time_label)?;
        }
        // Cobalt border around the track.
        backend.fill_rect(bx, by, vr.w, 1, colors.grid_line)?;
        backend.fill_rect(bx, by + vr.h as i32 - 1, vr.w, 1, colors.grid_line)?;
        backend.fill_rect(bx, by, 1, vr.h, colors.grid_line)?;
        backend.fill_rect(bx + vr.w as i32 - 1, by, 1, vr.h, colors.grid_line)?;

        // Label "VOLUME: X%" (or short "VOL X%" on tight layouts).
        let (label, label_offset) = volume_bar_label(vr.w, self.volume);
        backend.draw_text(
            &label,
            (bx - label_offset).max(0),
            by + (vr.h as i32 - at.font_hint as i32) / 2,
            at.font_hint,
            colors.time_label,
        )?;

        // Tick marks at 0/25/50/75/100 — only when bar is wide enough.
        // The 100% tick uses `vr.w - 1` so it stays inside the right edge.
        if vr.w >= VOLUME_BAR_WIDE_THRESHOLD {
            let last_tick_idx = VOLUME_TICK_POSITIONS.len().saturating_sub(1);
            for (i, pct) in VOLUME_TICK_POSITIONS.iter().enumerate() {
                let tx = if i == last_tick_idx {
                    bx + vr.w as i32 - 1
                } else {
                    bx + ((*pct as i32) * vr.w as i32 / 100)
                };
                backend.fill_rect(tx, by + vr.h as i32 + 1, 1, 3, colors.dim_text)?;
            }
        }
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
            "tv_sel_glow_top",
            "tv_sel_glow_bot",
            "tv_ftr_bg",
            "tv_ftr_nav",
            "tv_ftr_page",
            "tv_ftr_guide",
            "tv_expanded_overlay_bg",
            "tv_expanded_overlay_text",
            "tv_vol_bg",
            "tv_vol_fill",
            "tv_vol_label",
        ];
        for name in &fixed {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
        for i in 0..VOLUME_TICK_POSITIONS.len() {
            let name = format!("tv_vol_tick_{i}");
            if let Ok(obj) = sdi.get_mut(&name) {
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
