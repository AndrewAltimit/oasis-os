//! TV Guide EPG grid rendering (direct backend draw calls).
//!
//! `draw_windowed` is the only render path: the App trait draws every app
//! into its window's content rect, so there is no SDI-object variant.

use oasis_skin::active_theme::ActiveTheme;
use oasis_types::backend::Color;

use crate::grid_layout::{
    MAX_CELLS, SLOT_DURATION, VISIBLE_ROWS, VISIBLE_TIME_SLOTS, VOLUME_BAR_WIDE_THRESHOLD,
    VOLUME_TICK_POSITIONS, truncate_title, volume_bar_label, volume_bar_rect,
};
use crate::grid_state::TvGuideState;
use crate::schedule;

impl TvGuideState {
    /// Draw the TV guide EPG grid into the content rect `(cx, cy, cw, ch)`.
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

        // Derive theme colors fresh each draw so a skin switch while the
        // guide is open takes effect on the next frame.
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
}
