//! `OasisWasm::tick` phases: one method per stage of the frame, in the
//! order `tick` runs them.

use oasis_core::backend::SdiCore;
use oasis_core::cursor::CursorState;

use super::*;

impl OasisWasm {
    /// App requests, `wm` IPC, queued VFS operations and app ticks.
    pub(crate) fn tick_app_requests(&mut self, delta_seconds: f32) {
        // Carry out every request the apps queued (radio tune, settings,
        // ...), those of apps that closed this frame first. TV Guide tunes
        // go to the video overlay section below, Video Embed commands to
        // the YouTube overlay; the line-queued request files are appended.
        let requests = app_requests::gather(&mut self.closed_app_requests, &mut self.open_runners);
        for req in requests {
            if req.path == oasis_core::ipc::TV_REQUEST_PATH
                && let Some(oasis_core::ipc::TvRequest::TuneUrl(tune)) =
                    oasis_core::ipc::TvRequest::parse(&req.data)
            {
                self.pending_tv_tune = Some(tune);
                continue;
            }
            #[cfg(feature = "wasm-youtube")]
            if req.path == oasis_apps::video_embed::VIDEO_EMBED_REQUEST_PATH {
                self.handle_video_embed_request(&req.data);
                continue;
            }
            if let Err(e) = app_requests::write(&mut self.vfs, &req) {
                log::warn!("app VFS request write failed ({}): {e}", req.path);
            }
        }
        // `wm` / `open` commands (terminal, scripts).
        self.poll_wm_ipc();

        // Let open apps apply queued VFS mutations (file manager
        // deletes / renames / copies, paint's binary BMP saves).
        for (_, runner) in &mut self.open_runners {
            runner.apply_vfs_ops(&mut self.vfs);
        }

        // Advance time-driven app state (game loops, slideshows) by
        // the frame's wall time so it runs at a fixed rate
        // independent of the display's refresh rate.
        let dt_ms = (delta_seconds.max(0.0) * 1000.0).min(u32::MAX as f32) as u32;
        for (_, runner) in &mut self.open_runners {
            runner.tick(dt_ms, &self.vfs);
        }
        // Apps that decided to close outside an input handler (Text
        // Editor "Save & close" once the save above is written).
        self.apply_app_close_requests();

        // Drive the YouTube search fetcher; the embed app picks up
        // freshly-published results from VFS in its own `tick`.
        #[cfg(feature = "wasm-youtube")]
        self.poll_youtube_search();
    }

    /// Radio requests, catalog fetches, the radio state machine and its
    /// auto-advance / circuit breaker, and the status publish.
    pub(crate) fn tick_radio(&mut self) {
        use oasis_audio::RADIO_REQUEST_PATH;

        // Read radio requests from VFS.
        if self.vfs.exists(RADIO_REQUEST_PATH)
            && let Ok(data) = self.vfs.read(RADIO_REQUEST_PATH)
        {
            let request = String::from_utf8_lossy(&data).to_string();
            if !request.is_empty() {
                let _ = self.vfs.write(RADIO_REQUEST_PATH, b"");

                use oasis_audio::radio::RadioRequest;
                let parsed = RadioRequest::parse(&request);
                if let Some(RadioRequest::Tune(target)) = &parsed {
                    let station = self.radio_manager.registry.find(target).cloned();
                    if let Some(station) = station {
                        // Reset the failure counter on every
                        // user-initiated tune — they might be
                        // explicitly retrying after a previous
                        // failure tripped the circuit breaker.
                        self.radio_consecutive_errors = 0;
                        // Radio and music share the audio backend:
                        // the tune below stops the backend, so unload
                        // the music track and mark it stopped too.
                        self.stop_music();
                        let _ = self.radio_manager.tune(
                            &station.name,
                            station.bitrate,
                            &mut self.audio,
                        );

                        // Clear stale catalog/pending fetches on station change.
                        self.archive_catalog = None;
                        self.pending_catalog = None;
                        // Disconnect old source so tick() doesn't keep
                        // polling the previous station while the new
                        // catalog is being fetched.
                        if let Some(mut old) = self.radio_source.take() {
                            old.disconnect();
                        }

                        self.radio_manager
                            .set_source_info(&station.source_type, &station.collection);

                        if station.source_type == "archive" && !station.collection.is_empty() {
                            self.pending_catalog = Some(archive::WasmArchiveCatalogFetcher::new(
                                &station.collection,
                                self.frame_counter,
                            ));
                        }
                    } else {
                        console_log!("radio: station not found: {target}");
                        self.radio_manager
                            .set_error(&format!("station not found: {target}"));
                    }
                } else if let Some(req) = &parsed {
                    let _ = self.radio_manager.handle_request(req, &mut self.audio);
                    // Clear catalog on stop.
                    if self.radio_manager.state() == oasis_audio::radio::RadioState::Stopped {
                        self.archive_catalog = None;
                        self.pending_catalog = None;
                    }
                }
            }
        }

        // Poll pending catalog fetch.
        if self.pending_catalog.as_ref().is_some_and(|f| f.is_ready())
            && let Some(fetcher) = self.pending_catalog.take()
        {
            match fetcher.take_results() {
                Ok((catalog, source)) => {
                    if let Some(mut old) = self.radio_source.take() {
                        old.disconnect();
                    }
                    // Hand the first track's URL to the audio
                    // backend before the radio state machine asks
                    // it to load streaming. The backend's
                    // `<audio>` element streams the URL natively
                    // (works around Firefox's lack of `audio/mpeg`
                    // MSE support).
                    if let Some(url) = source.streaming_url() {
                        self.audio.set_streaming_url(url);
                    }
                    self.archive_catalog = Some(catalog);
                    self.radio_source = Some(source);
                },
                Err(e) => {
                    console_log!("radio: catalog fetch failed: {e}");
                    self.radio_manager.set_error(&e);
                },
            }
        }

        // Promote `<audio>` element errors to radio errors so the
        // failure counter advances. Without this the source/buffer
        // state machine doesn't see the decode failure and just
        // stays in Buffering forever.
        if let Some(err) = self.audio.take_streaming_error() {
            console_log!("radio: audio element error: {err}");
            self.radio_manager.set_error(&err);
        }

        // Drive the radio state machine. Capture errors to console
        // so users can diagnose tune failures (autoplay-blocked
        // playback, MSE not supporting `audio/mpeg`, network
        // hiccups, etc.) — `radio_manager.tick` itself swallows the
        // error into `set_error` but doesn't log it.
        let prev_state = self.radio_manager.state();
        let _ = self
            .radio_manager
            .tick(&mut self.radio_source, &mut self.audio);
        if prev_state != oasis_audio::radio::RadioState::Error
            && self.radio_manager.state() == oasis_audio::radio::RadioState::Error
        {
            self.radio_consecutive_errors += 1;
            console_log!(
                "radio: error #{} of {RADIO_FAILURE_LIMIT}: {}",
                self.radio_consecutive_errors,
                self.radio_manager.error_msg(),
            );
        }

        // The `<audio>` element fires `ended` when a track
        // finishes; surface that to the radio source so the state
        // machine knows to auto-advance. (RadioManager's normal
        // path watches the source's `Ended` state.)
        if self.audio.take_streaming_ended()
            && let Some(ref mut src) = self.radio_source
        {
            src.disconnect();
        }

        // Auto-advance to next track for archive stations — but
        // only while we're under the failure budget. Once the
        // circuit breaker trips, sit in error state until the
        // user picks a station again.
        if self.radio_consecutive_errors < RADIO_FAILURE_LIMIT
            && self.radio_manager.needs_next_track()
            && let Some(ref mut catalog) = self.archive_catalog
            && let Some(track) = catalog.next_track().cloned()
        {
            if let Some(mut old) = self.radio_source.take() {
                old.disconnect();
            }
            let url = oasis_audio::radio::ArchiveCatalog::download_url(&track);
            let source = archive::WasmArchiveSource::new(&url, &track.title, &track.creator);
            self.audio.set_streaming_url(&url);
            self.radio_source = Some(Box::new(source));
            self.radio_manager.continue_playing();
        } else if self.radio_consecutive_errors >= RADIO_FAILURE_LIMIT
            && self.radio_manager.needs_next_track()
        {
            // Stop the cascade: drop the source and force-set a
            // sticky error so the radio app surfaces it. Drop the
            // catalog too so a stale next-track pointer doesn't
            // immediately re-fire on the next tick.
            if let Some(mut old) = self.radio_source.take() {
                old.disconnect();
            }
            self.archive_catalog = None;
            self.radio_manager.set_error(&format!(
                "stopped after {RADIO_FAILURE_LIMIT} failed tracks — pick a station to retry",
            ));
        }

        // Publish radio status periodically.
        if self.frame_counter.is_multiple_of(15) {
            let _ = self.radio_manager.publish_status(&mut self.vfs);
        }
        // (The Internet Radio app polls the status in its `tick`.)
    }

    /// Finish or start the TV Guide catalog fetch.
    pub(crate) fn tick_tv_catalog(&mut self) {
        // Poll pending TV catalog fetch.
        if self
            .pending_tv_catalog
            .as_ref()
            .is_some_and(|f| f.is_ready())
            && let Some(fetcher) = self.pending_tv_catalog.take()
        {
            match fetcher.take_results() {
                Ok(catalogs) => {
                    let runner = vfs_content::find_tv_guide_runner_wasm(&mut self.open_runners);
                    if let Some(runner) = runner {
                        if let Some(guide) = runner.tv_guide_state() {
                            guide.fetch_in_progress = false;
                            let all_none = catalogs.iter().all(|c| c.is_none());
                            for (i, cat) in catalogs.into_iter().enumerate() {
                                if let Some(c) = cat
                                    && i < guide.catalogs.len()
                                {
                                    guide.catalogs[i] = Some(c);
                                    guide.rebuild_cached_schedule(i);
                                }
                            }
                            if all_none {
                                guide.fetch_error =
                                    Some("No episodes found for any channel".into());
                            }
                        }
                        runner.refresh_tv_text();
                    }
                },
                Err(e) => {
                    console_log!("TV catalog fetch failed: {e}");
                    let runner = vfs_content::find_tv_guide_runner_wasm(&mut self.open_runners);
                    if let Some(runner) = runner {
                        if let Some(guide) = runner.tv_guide_state() {
                            guide.fetch_in_progress = false;
                            guide.fetch_error = Some(e);
                        }
                        runner.refresh_tv_text();
                    }
                },
            }
        }

        // Start TV catalog fetch if a TV Guide app needs it.
        if self.pending_tv_catalog.is_none() {
            let runner = vfs_content::find_tv_guide_runner_wasm(&mut self.open_runners);
            if let Some(runner) = runner
                && let Some(guide) = runner.tv_guide_state()
                && !guide.fetch_attempted
                && guide.catalogs.iter().all(|c| c.is_none())
            {
                guide.fetch_attempted = true;
                guide.fetch_in_progress = true;
                self.pending_tv_catalog =
                    Some(tv_catalog::WasmTvCatalogFetcher::new(&guide.channels));
            }
        }
    }

    /// Start the queued TV tune, if the guide is still open.
    pub(crate) fn start_pending_tv_tune(&mut self) {
        // Dropped when the guide is closed.
        let Some(tune) = self.pending_tv_tune.take() else {
            return;
        };
        let Some(runner) = vfs_content::find_tv_guide_runner_wasm(&mut self.open_runners) else {
            return;
        };
        let (url, seek_secs) = (tune.url.as_str(), tune.seek_secs);
        if url.is_empty() {
            return;
        }
        console_log!(
            "TV tune: {} seek={}s",
            &url[..url.floor_char_boundary(80)],
            seek_secs,
        );
        let (_, _, pw, ph) = vfs_content::tv_preview_rect(&self.active_theme);
        let tex_id = self
            .video_player
            .start(url, seek_secs, pw, ph, &mut self.backend);
        // Assign the texture to the guide so its existing rendering code
        // displays it.
        if let Some(guide) = runner.tv_guide_state() {
            guide.preview_texture = tex_id;
            console_log!("TV tune: texture={:?} preview={}x{}", tex_id, pw, ph,);
        }
    }

    /// Untune / volume sync / auto-advance / frame capture for the TV
    /// video player.
    pub(crate) fn tick_video(&mut self) {
        // Detect untune: video is playing but guide has no tuned channel.
        if self.video_player.is_active() {
            let should_stop = {
                let runner = vfs_content::find_tv_guide_runner_wasm(&mut self.open_runners);
                match runner {
                    Some(runner) => runner
                        .tv_guide_state()
                        .is_none_or(|g| g.tuned_channel.is_none()),
                    None => true, // TV Guide closed.
                }
            };
            if should_stop {
                self.video_player.stop(&mut self.backend);
            }
        }

        // Sync volume from guide state to the video element. Synced every
        // frame (set_volume skips no-op writes) so a freshly created element
        // picks up the guide's volume instead of the browser default of 100%.
        if self.video_player.is_active() {
            let runner = vfs_content::find_tv_guide_runner_wasm(&mut self.open_runners);
            if let Some(runner) = runner
                && let Some(guide) = runner.tv_guide_state()
            {
                self.video_player.set_volume(guide.volume as f64 / 100.0);
                guide.volume_changed = false;
            }
        }

        // Auto-advance to next episode when the current video ends.
        if self.video_player.is_ended() {
            self.auto_advance_episode();
        }

        // Capture the latest video frame onto the texture canvas.
        self.video_player.tick();
    }

    /// Draw the frame: wallpaper, windows or the SDI scene, overlays and
    /// the cursor, then present.
    pub(crate) fn render_frame(&mut self) {
        // Hide cursor from SDI so window content doesn't draw under it;
        // we blit the cursor manually after all rendering.
        CursorState::hide_sdi(&mut self.sdi);

        if let Err(e) = self.backend.clear(self.bg_color) {
            console_log!("clear error: {e}");
        }

        // Render shader wallpaper FIRST (replaces bg_color clear).
        // Runs every frame so the animation stays live in all modes.
        if let Some(ref mut bridge) = self.shader_bridge
            && let Some(info) = oasis_core::vector_overlay::get_shader_layer(&self.active_theme)
        {
            bridge.render_frame(
                &info.name,
                self.frame_counter as f32 / 60.0,
                &info.params,
                self.backend.ctx(),
            );
        }

        // Pre-draw: hide the YouTube iframe if its window is minimized or
        // gone. Windows in `Minimized` state are not visited by
        // `draw_with_clips_overlay`, so without this the iframe would
        // stay glued to its last position even though the canvas has
        // already cleared the area. Use `soft_hide` for the minimize
        // case so playback state survives until the user restores the
        // window; full `hide` is reserved for stop/close.
        #[cfg(feature = "wasm-youtube")]
        if self.youtube_active_id.is_some() {
            match self.wm.get_window("video_embed") {
                Some(w) if w.state == oasis_core::wm::window::WindowState::Minimized => {
                    self.iframe.soft_hide();
                },
                None => {
                    self.iframe.hide();
                },
                _ => {},
            }
        }

        if self.mode == Mode::Desktop && self.wm.window_count() > 0 {
            let browser = &mut self.browser;
            let iframe_ref = &mut self.iframe;
            let open_runners = &mut self.open_runners;
            let active_theme = &self.active_theme;
            let dashboard = &self.dashboard;
            let frame = self.frame_counter as u32;
            // The DOM iframe overlay sits above the canvas at a fixed
            // z-index, so it can't be composed under other OASIS windows
            // that are stacked above the browser. Only show it when the
            // browser/video_embed is the topmost (active) window;
            // otherwise soft-hide so its document state survives until
            // it returns to the front.
            let active_window_id: Option<String> = self.wm.active_window().map(|s| s.to_string());
            #[cfg(feature = "wasm-youtube")]
            let youtube_active_url = self.youtube_active_url.clone();
            let wants_vector_icons =
                self.skin.features.dashboard && active_theme.icon.style == "vector";
            let overlay =
                |be: &mut dyn oasis_core::backend::SdiBackend| -> oasis_core::error::Result<()> {
                    if wants_vector_icons {
                        dashboard.render_vector_icons(be, active_theme, frame)?;
                    }
                    Ok(())
                };
            if let Err(e) = self.wm.draw_with_clips_overlay(
                &mut self.sdi,
                &mut self.backend,
                overlay,
                |window_id, cx, cy, cw, ch, be| {
                    let is_topmost = active_window_id.as_deref() == Some(window_id);
                    let result = if window_id == "browser" {
                        if let Some(ref mut bw) = *browser {
                            bw.set_window(cx, cy, cw, ch);
                            let url = bw.current_url().map(|s| s.to_string());
                            let is_http = url.as_ref().is_some_and(|u| {
                                u.starts_with("http://") || u.starts_with("https://")
                            });
                            if is_http && let Some(ref u) = url {
                                let url_bar_h = bw.config.url_bar_height;
                                let status_bar_h = bw.config.status_bar_height;
                                let content_y = cy + url_bar_h as i32;
                                let content_h = ch.saturating_sub(url_bar_h + status_bar_h);
                                if is_topmost {
                                    iframe_ref.show(u, cx, content_y, cw, content_h);
                                } else {
                                    iframe_ref.soft_hide();
                                }
                                bw.paint_chrome_only(be)
                            } else {
                                iframe_ref.hide();
                                bw.paint(be)
                            }
                        } else {
                            iframe_ref.hide();
                            Ok(())
                        }
                    } else if let Some((_, runner)) =
                        open_runners.iter_mut().find(|(id, _)| id == window_id)
                    {
                        let r = runner.draw_windowed(cx, cy, cw, ch, be, active_theme);
                        // YouTube iframe: glue to the Video Embed window's
                        // content rect every frame so drag/resize tracks.
                        #[cfg(feature = "wasm-youtube")]
                        if window_id == "video_embed"
                            && let Some(ref url) = youtube_active_url
                        {
                            let title_h = active_theme.app.title_bar_height as i32;
                            let inner_y = cy + title_h;
                            let inner_h = ch.saturating_sub(active_theme.app.title_bar_height);
                            if is_topmost {
                                iframe_ref.show(url, cx, inner_y, cw, inner_h);
                            } else {
                                iframe_ref.soft_hide();
                            }
                        }
                        r
                    } else {
                        Ok(())
                    };
                    if let Err(ref e) = result {
                        web_sys::console::error_1(
                            &format!("window '{window_id}' content error: {e}").into(),
                        );
                    }
                    result
                },
            ) {
                console_log!("draw_with_clips error: {e}");
            }
        } else {
            // Not in desktop mode or no windows — hide iframe.
            self.iframe.hide();
            if self.mode == Mode::Dashboard
                && (self.active_theme.icon.style == "vector"
                    || !self.active_theme.background_layers.is_empty())
            {
                // Shader already rendered above as wallpaper.
                if let Err(e) = self.sdi.draw_base_layer(&mut self.backend) {
                    console_log!("sdi draw_base error: {e}");
                }

                let _ = oasis_core::vector_overlay::render_vector_background(
                    &mut self.backend,
                    &self.active_theme,
                    self.frame_counter as u32,
                );
                let _ = self.dashboard.render_vector_icons(
                    &mut self.backend,
                    &self.active_theme,
                    self.frame_counter as u32,
                );
                if let Err(e) = self.sdi.draw_overlay_layer(&mut self.backend) {
                    console_log!("sdi draw_overlay error: {e}");
                }
            } else if let Err(e) = self.sdi.draw(&mut self.backend) {
                console_log!("sdi draw error: {e}");
            }
        }

        // Paint terminal scrollbar when in terminal mode.
        if self.mode == Mode::Terminal
            && let Err(e) = terminal_sdi::paint_terminal_scrollbar(
                &mut self.backend,
                self.output_lines.len(),
                self.terminal_scroll_offset,
                &self.active_theme,
            )
        {
            console_log!("terminal scrollbar error: {e}");
        }

        // Draw transition overlay if active.
        if let Some(ref mut trans) = self.active_transition {
            if let Err(e) = trans.draw_overlay(&mut self.backend) {
                console_log!("transition overlay error: {e}");
            }
            trans.tick();
            if trans.is_done() {
                self.active_transition = None;
            }
        }

        // Draw cursor on top of everything (after windows, scrollbar,
        // transition overlay). Hide it while the pointer is over the
        // iframe overlay or off the page — the canvas stops receiving
        // mousemove events there so the cursor would otherwise freeze
        // at its last tracked position, looking trapped.
        if self.mouse_cursor.visible
            && self.input.pointer_on_canvas()
            && let Some(tex) = self.cursor_texture
            && let Err(e) = self.backend.blit(
                tex,
                self.mouse_cursor.x,
                self.mouse_cursor.y,
                12 * self.mouse_cursor.scale,
                18 * self.mouse_cursor.scale,
            )
        {
            console_log!("cursor blit error: {e}");
        }

        if let Err(e) = self.backend.swap_buffers() {
            console_log!("swap_buffers error: {e}");
        }
    }
}
