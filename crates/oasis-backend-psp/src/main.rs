//! PSP entry point for OASIS_OS.
//!
//! Unified desktop with dashboard icons, windowed WM, kiosk fullscreen apps,
//! and a taskbar showing running windows.
//!
//! Audio playback and file I/O run on background threads to prevent frame drops.
//!
//! Major subsystems are split into modules:
//! - `app_states` -- per-app mutable state structs
//! - `dashboard` -- dashboard init and SDI helpers
//! - `input_dispatch` -- unified input event routing
//! - `getrandom` -- custom getrandom backends for PSP entropy
//! - `io_poll` -- async I/O response polling
//! - `render_classic` -- kiosk fullscreen view rendering
//! - `render_desktop` -- windowed desktop mode rendering

#![feature(restricted_std)]
#![no_main]

use oasis_backend_psp::{
    CURSOR_H, CURSOR_W, Color, PspBackend, SCREEN_HEIGHT, SCREEN_WIDTH, SdiRegistry,
    StatusBarInfo, SystemInfo, TextureId, WindowManager,
};

// oasis-core SDI integration types.
use oasis_core::bottombar::BottomBar;
use oasis_core::platform::{BatteryState, CpuClock, PowerInfo, SystemTime};
use oasis_core::statusbar::StatusBar;
use oasis_core::taskbar::Taskbar;
use oasis_core::terminal_sdi;

mod app_states;
mod boot;
mod chrome;
mod commands;
mod dashboard;
mod desktop;
mod getrandom;
mod input_dispatch;
mod io_poll;
mod render_classic;
mod render_desktop;
mod skins;
mod theme;
mod types;
mod views;
mod views_sdi;

use app_states::*;
use input_dispatch::DispatchResult;
use theme::*;
use types::*;

// Always use user-mode module flag (0x0000). PRO-C CFW allows user-mode
// modules to call kernel syscalls, and module_kernel! (0x1000) fails to
// load on PSP-3000 + 6.20 PRO-C.
psp::module!("OASIS_OS", 1, 0);

// Force the getrandom module to be linked (contains registration macros).
use getrandom as _;

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn psp_main() {
    let _ = psp::callback::setup_exit_callback();

    // Debug log helper -- appends a line using raw PSP I/O.
    fn dbg_log(msg: &str) {
        // SAFETY: sceIo calls with valid path and buffer pointers.
        unsafe {
            let fd = psp::sys::sceIoOpen(
                b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
                psp::sys::IoOpenFlags::APPEND
                    | psp::sys::IoOpenFlags::CREAT
                    | psp::sys::IoOpenFlags::WR_ONLY,
                0o777,
            );
            if fd >= psp::sys::SceUid(0) {
                psp::sys::sceIoWrite(fd, msg.as_ptr() as *const _, msg.len());
                psp::sys::sceIoWrite(fd, b"\n".as_ptr() as *const _, 1);
                psp::sys::sceIoClose(fd);
            }
        }
    }

    dbg_log("[EBOOT] psp_main entered");

    let mut backend = PspBackend::new();
    backend.init();
    boot::show_boot_screen(&mut backend, "Initializing...", 10);
    dbg_log("[EBOOT] backend init OK");

    // Register exception handler (kernel mode only) for crash diagnostics.
    #[cfg(feature = "kernel-exception")]
    oasis_backend_psp::register_exception_handler();
    boot::show_boot_screen(&mut backend, "Loading config...", 25);

    // Load persistent configuration.
    let mut config =
        psp::config::Config::load(CONFIG_PATH).unwrap_or_else(|_| psp::config::Config::new());
    dbg_log("[EBOOT] config loaded");

    // Set clock speed from config (default: max 333MHz).
    let clock_mhz = config.get_i32("clock_mhz").unwrap_or(333);
    let bus_mhz = config.get_i32("bus_mhz").unwrap_or(166);
    oasis_backend_psp::set_clock(clock_mhz, bus_mhz);

    // Query static hardware info.
    let sysinfo = SystemInfo::query();
    dbg_log("[EBOOT] sysinfo queried");

    // -- ActiveTheme (SDI integration) --
    // Derive theme from a lightweight preset (9 base colors) instead of
    // pulling in the TOML parser + 18 embedded skin strings (~850KB).
    dbg_log("[EBOOT] creating theme...");
    let skin_key = config.get_str("skin").unwrap_or("psix");
    let mut current_preset = skins::PspSkinPreset::from_key(skin_key);
    let mut active_theme = current_preset.to_active_theme();
    let skin_features = skins::PspSkinPreset::skin_features();
    dbg_log("[EBOOT] active_theme created");

    // Create dashboard from PSP app list.
    let mut dashboard_state = dashboard::create_dashboard(&skin_features, &active_theme);
    dbg_log("[EBOOT] dashboard created");

    let mut status_bar = StatusBar::new();
    let mut bottom_bar = BottomBar::new();
    bottom_bar.total_pages = dashboard_state.page_count();
    let mut taskbar = Taskbar::new();

    boot::show_boot_screen(&mut backend, "Generating textures...", 40);
    dbg_log("[EBOOT] textures phase");

    // Load wallpaper texture at reduced resolution (64x64 = 16KB vs 1MB).
    // The GE scales it up to 480x272 with bilinear filtering during blit.
    use oasis_backend_psp::{WALLPAPER_TEX_H, WALLPAPER_TEX_W};
    let wallpaper_data = oasis_backend_psp::generate_gradient(WALLPAPER_TEX_W, WALLPAPER_TEX_H);
    let wallpaper_tex = backend
        .load_texture_inner(WALLPAPER_TEX_W, WALLPAPER_TEX_H, &wallpaper_data)
        .unwrap_or(TextureId(0));

    // Load cursor texture.
    let cursor_data = oasis_backend_psp::generate_cursor_pixels();
    let cursor_tex = backend
        .load_texture_inner(CURSOR_W, CURSOR_H, &cursor_data)
        .unwrap_or(TextureId(0));
    boot::show_boot_screen(&mut backend, "Setting up UI...", 60);

    // -- Window Manager (Desktop mode) --
    let psp_theme = oasis_backend_psp::psp_wm_theme();
    let mut wm = WindowManager::with_theme(SCREEN_WIDTH, SCREEN_HEIGHT, psp_theme);
    let mut sdi = SdiRegistry::new();
    dbg_log("[EBOOT] SDI registry created");

    // -- Kiosk app tracking (unified desktop) --
    let mut kiosk_app = KioskApp::None;
    let mut prev_kiosk_app = KioskApp::None;

    let mut icons_hidden: bool = false;
    let mut viz_frame: u32 = 0;

    // -- Per-app state --
    let vol_info = backend.volatile_mem_info();
    let mode_label = if cfg!(feature = "kernel-mode") {
        "kernel"
    } else {
        "user"
    };
    let initial_lines = vec![
        format!("OASIS_OS v0.1.0 [PSP] ({mode_label} mode)"),
        format!(
            "CPU: {}MHz  Bus: {}MHz  ME: {}MHz",
            sysinfo.cpu_mhz, sysinfo.bus_mhz, sysinfo.me_mhz,
        ),
        if let Some((total, _)) = vol_info {
            format!("Texture cache: {} KB volatile RAM claimed", total / 1024)
        } else {
            String::from("Texture cache: main heap only (PSP-1000)")
        },
        String::from("Type 'help' for commands. []=OSK, Up/Down=scroll."),
        String::new(),
    ];
    let mut term = TerminalState::new(initial_lines);

    // Try to restore previous terminal history from save data (silent).
    if let Ok(saved) = commands::load_terminal_history() {
        if !saved.is_empty() {
            term.lines.push(String::from("(restored previous session)"));
            term.lines.extend(saved);
            term.lines.push(String::new());
        }
    }

    // Boot-time self-test: if sentinel file exists, run test suite,
    // write results to selftest.log, delete sentinel, then exit.
    if psp::io::stat(commands::SELFTEST_SENTINEL).is_ok() {
        boot::show_boot_screen(&mut backend, "Running self-test...", 90);
        let results = commands::run_selftest(&mut config);
        let _ = psp::io::remove_file(commands::SELFTEST_SENTINEL);
        for line in &results {
            term.lines.push(line.clone());
        }
        backend.clear_inner(Color::rgb(0, 0, 0));
        let y_start = 4i32;
        for (i, line) in results.iter().enumerate().take(30) {
            backend.draw_text_inner(line, 4, y_start + (i as i32 * 9), 8, Color::WHITE);
        }
        backend.swap_buffers_inner();
        psp::thread::sleep_ms(2000);
        // SAFETY: sceKernelExitGame terminates the running application.
        unsafe { psp::sys::sceKernelExitGame() };
    }

    let mut fm = FileManagerState::new();
    let mut pv = PhotoViewerState::new();
    let mut mp = MusicPlayerState::new();
    let mut br = BrowserState::new();
    // BrowserWidget is lazily initialized on first use (saves ~500KB RAM).
    let mut radio = RadioState::new();
    let mut tv = TvGuideState::new();

    // USB storage mode handle (RAII: drop exits storage mode).
    let mut usb_storage: Option<psp::usb::UsbStorageMode> = None;

    // AV codec modules (AvCodec, AvMpegBase, AvMp3) are loaded lazily
    // by the audio thread on first play. Loading them here at startup
    // would conflict with the PRX overlay's sceAudiocodec if the PRX
    // initialized before the EBOOT was launched.

    // Pre-init MPEG subsystem before spawning workers (audio thread would
    // load AvMpegBase first, making sceMpegInit fail with 0x8002013a).
    oasis_backend_psp::video::preinit_mpeg();

    // PMF test disabled — even real Persona 3 PMSF crashes.
    // Root cause (confirmed by Ghidra): sceMpeg internally calls
    // sceVideocodec (avcodec.prx) for ME decode. The ME submission
    // Background worker threads: audio, file I/O, and video decode.
    let (audio, io) = oasis_backend_psp::spawn_workers();
    oasis_backend_psp::video::spawn_video_thread();
    boot::show_boot_screen(&mut backend, "Starting workers...", 80);

    // Confirm button held state for pointer simulation.
    let mut _confirm_held = false;

    // Register power callback for sleep/wake handling (keep handle alive).
    let _power_cb = oasis_backend_psp::register_power_callback();

    // Frame timing via hardware tick counter.
    let mut frame_timer = psp::time::FrameTimer::new();
    boot::show_boot_screen(&mut backend, "Ready", 100);
    // Start TCP command server for remote dev automation.
    oasis_backend_psp::cmd_server::spawn();

    dbg_log("[EBOOT] entering main loop");
    psp::thread::sleep_ms(400);

    // Cached values for expensive kernel queries (throttled to ~4Hz).
    let mut cached_status = StatusBarInfo::poll();
    let mut cached_free_kb: i32 = 0;
    let mut cached_max_blk_kb: i32 = 0;

    loop {
        let _dt = frame_timer.tick();
        // Log first frame only.
        if viz_frame == 0 {
            dbg_log("[EBOOT] first frame tick");
        }
        // Prevent idle auto-suspend while running.
        oasis_backend_psp::power_tick();

        // Check if we resumed from sleep.
        if oasis_backend_psp::check_power_resumed() {
            term.lines.push(String::from("[Power] Resumed from sleep"));
        }

        // -- Poll async I/O responses --
        io_poll::poll_io_responses(
            &io,
            &audio,
            &mut backend,
            &mut term,
            &mut pv,
            &mut br,
            &mut radio,
            &mut tv,
            &dbg_log,
        );

        // Browser tick disabled on PSP -- std::time::Instant crashes
        // on Allegrex (confirmed by testing). Image loading happens
        // synchronously during navigate_vfs instead.

        // Poll radio streaming state from audio thread atomics.
        if radio.status == RadioStatus::Buffering || radio.status == RadioStatus::Playing {
            if !audio.is_radio_streaming() {
                radio.status = RadioStatus::Stopped;
                radio.now_playing.clear();
            } else if audio.is_radio_buffering() {
                radio.status = RadioStatus::Buffering;
            } else {
                radio.status = RadioStatus::Playing;
            }
            if let Some(meta) = audio.poll_radio_meta() {
                radio.now_playing = meta;
            }
        }

        // -- Input dispatch (unified) --
        let events = backend.poll_events_inner();
        let mut should_quit = false;

        for event in &events {
            match input_dispatch::dispatch_unified(
                event,
                &mut backend,
                &mut kiosk_app,
                &mut dashboard_state,
                &mut wm,
                &mut sdi,
                &audio,
                &io,
                &mut term,
                &mut fm,
                &mut pv,
                &mut mp,
                &mut br,
                &mut radio,
                &mut tv,
                &mut icons_hidden,
                &mut usb_storage,
                &mut config,
                &mut current_preset,
                &mut active_theme,
                &skin_features,
                &dbg_log,
            ) {
                DispatchResult::Quit => {
                    should_quit = true;
                    break;
                },
                DispatchResult::Continue | DispatchResult::SkipRest => {},
            }
        }
        if should_quit {
            return;
        }

        // -- Poll video decode frames --
        // Poll regardless of download state — video decode starts as soon
        // as the first keyframe arrives, even during HTTP streaming.
        // Always check for video frames — log state every 5 seconds.
        if viz_frame % 300 == 1 {
            let vp = oasis_backend_psp::video::is_video_playing();
            oasis_backend_psp::video::vlog_force(&format!(
                "[MAIN] tuned={} vp={} tex={:?} kiosk={:?} frame={}",
                tv.tuned.is_some(), vp, tv.preview_tex, kiosk_app as u8, viz_frame,
            ));
        }
        if tv.tuned.is_some() {
            // Drain all queued frames to keep the queue clear.
            let mut latest = oasis_backend_psp::video::poll_video_frame();
            while let Some(newer) = oasis_backend_psp::video::poll_video_frame()
            {
                latest = Some(newer);
            }
            // Upload every frame — direct CSC is fast enough (~15ms).
            let do_upload = latest.is_some();
            if let Some(frame) = latest {
                if do_upload {
                    let t0 = unsafe {
                        psp::sys::sceKernelGetSystemTimeWide()
                    } as u32;
                    let pixels =
                        oasis_backend_psp::video::frame_pixels_raw(&frame);
                    let old_tex = tv.preview_tex;
                    // Pass stride as width so texture buf_w matches CSC
                    // stride (both 512px). This eliminates row-by-row copy.
                    tv.preview_tex = backend.update_video_texture(
                        frame.stride, frame.height, pixels,
                    );
                    // Set actual content dimensions for correct UV mapping.
                    backend.set_video_content_size(
                        frame.width, frame.height,
                    );
                    let dt = (unsafe {
                        psp::sys::sceKernelGetSystemTimeWide()
                    } as u32)
                        .wrapping_sub(t0);
                    oasis_backend_psp::video::record_upload_time(dt);
                    if old_tex.is_none() && tv.preview_tex.is_some() {
                        oasis_backend_psp::video::vlog_force(&format!(
                            "[MAIN] first video tex: {}x{} → {:?}",
                            frame.width, frame.height, tv.preview_tex,
                        ));
                    }
                }
            }
            // Only clear tuned state when video WAS playing and stopped
            // (not during the startup gap before streaming begins).
            if tv.preview_tex.is_some()
                && !oasis_backend_psp::video::is_video_playing()
            {
                backend.free_video_texture();
                tv.preview_tex = None;
                tv.tuned = None;
                tv.now_playing.clear();
            }
        }

        // -- Decode hang watchdog (main thread, every ~0.5s) --
        // If DECODE_STEP == 2 (stuck inside sceMpegAvcDecode), signal
        // the internal semaphore to unblock the ME and force an error
        // return, allowing the video thread to fall back to audio-only.
        if tv.tuned.is_some() && viz_frame % 30 == 0 {
            let step = psp::mpeg::DECODE_STEP.load(
                core::sync::atomic::Ordering::Relaxed,
            );
            if step == 2 {
                oasis_backend_psp::video::vlog_force(
                    "[WATCHDOG] ME stuck in AvcDecode — unblocking"
                );
                oasis_backend_psp::video::unblock_stuck_decode();
            }
        }

        // -- Render --
        // Skip clear + wallpaper when video covers the entire screen.
        let video_fullscreen = kiosk_app == KioskApp::TvGuide
            && tv.tuned.is_some()
            && tv.preview_tex.is_some();

        // During fullscreen video, skip all non-video rendering:
        // status bar polling (6+ kernel calls), bar updates, wallpaper,
        // SDI updates, WM operations. Just blit video + swap.
        if !video_fullscreen {
            // Throttle expensive kernel syscalls (~4Hz at 60fps).
            if viz_frame % 15 == 0 {
                cached_status = StatusBarInfo::poll();
            }

            // Feed PSP status info into oasis-core's StatusBar.
            {
                let st = cached_status;
                let sys_time = SystemTime {
                    year: st.year,
                    month: st.month as u8,
                    day: st.day as u8,
                    hour: st.hour as u8,
                    minute: st.minute as u8,
                    second: 0,
                };
                let bat_state = if st.ac_power && !st.battery_charging {
                    BatteryState::Full
                } else if st.battery_charging {
                    BatteryState::Charging
                } else if st.battery_percent < 0 {
                    BatteryState::NoBattery
                } else {
                    BatteryState::Discharging
                };
                let power = PowerInfo {
                    battery_percent: if st.battery_percent >= 0 {
                        Some(st.battery_percent as u8)
                    } else {
                        None
                    },
                    battery_minutes: None,
                    state: bat_state,
                    cpu: CpuClock {
                        current_mhz: sysinfo.cpu_mhz as u32,
                        max_mhz: 333,
                    },
                };
                status_bar.update_info(Some(&sys_time), Some(&power));
            }

            // Update bottom bar page tracking.
            bottom_bar.current_page = dashboard_state.page;
            bottom_bar.total_pages = dashboard_state.page_count();
            bottom_bar.tick_animation(&active_theme);

            backend.clear_inner(Color::BLACK);
            // Wallpaper: 64x64 texture scaled to fullscreen by GE (bilinear).
            backend.blit_scaled(wallpaper_tex, 0, 0, SCREEN_WIDTH, SCREEN_HEIGHT);
        }
        let status = cached_status;
        let fps = frame_timer.fps();
        let usb_active = usb_storage.is_some();

        // -- Unified render path --
        if kiosk_app != KioskApp::None {
            // Hide all floating window decorations during fullscreen.
            if !video_fullscreen {
                wm.hide_all_window_sdi(&mut sdi);
            }

            // Kiosk app active: render it fullscreen using Classic renderers.
            render_classic::render_classic(
                &mut backend,
                &mut sdi,
                &mut dashboard_state,
                &active_theme,
                kiosk_app,
                &mut prev_kiosk_app,
                icons_hidden,
                &mut fm,
                &mut pv,
                &mut mp,
                &mut br,
                &mut radio,
                &mut tv,
                &mut term,
                &audio,
                viz_frame,
                &dbg_log,
            );
        } else {
            // No kiosk app: show dashboard + any windowed WM windows.
            dashboard::hide_dashboard_sdi(&mut dashboard_state, &mut sdi);
            terminal_sdi::set_terminal_visible(&mut sdi, false);

            // Draw desktop icons directly (bypasses SDI, cheaper on GU).
            if !icons_hidden {
                chrome::draw_dashboard(
                    &mut backend,
                    dashboard_state.selected,
                    dashboard_state.page,
                    viz_frame,
                );
            }

            // Kiosk transition: hide old SDI objects, restore window decorations.
            if prev_kiosk_app != KioskApp::None {
                views_sdi::hide_all(&mut sdi);
                wm.show_all_window_sdi(&mut sdi);
                prev_kiosk_app = KioskApp::None;
            }

            // Throttle kernel heap queries (~4Hz).
            if viz_frame % 15 == 0 {
                // SAFETY: scalar FFI returning available memory stats.
                unsafe {
                    cached_free_kb = psp::sys::sceKernelTotalFreeMemSize() as i32 / 1024;
                    cached_max_blk_kb = psp::sys::sceKernelMaxFreeMemSize() as i32 / 1024;
                }
                // Update TCP command server with current state.
                oasis_backend_psp::cmd_server::update_status(
                    kiosk_app as u8,
                    cached_free_kb,
                    cached_max_blk_kb,
                    viz_frame as i32,
                );
            }

            // Render windowed WM windows (if any are open).
            render_desktop::render_desktop(
                &mut backend,
                &mut wm,
                &mut sdi,
                &config,
                &status,
                &sysinfo,
                fps,
                usb_active,
                cached_free_kb,
                cached_max_blk_kb,
                &term,
                &fm,
                &pv,
                &mp,
                &audio,
                &mut br,
                &tv,
            );
        }

        // Status bar + bottom bar + taskbar — skip during fullscreen video.
        if !video_fullscreen {
            active_theme.bar.url_text.clear();
            status_bar.update_sdi(&mut sdi, &active_theme, &skin_features);
            bottom_bar.update_sdi(&mut sdi, &active_theme, &skin_features);
            taskbar.update_sdi(
                &mut sdi,
                &active_theme,
                wm.windows(),
                wm.active_window(),
                false,
            );
        }

        // SDI two-pass rendering for PSP performance:
        // - Base layer for kiosk apps that use SDI (file manager, terminal, etc.)
        // - Overlay layer always (status bar, bottom bar, taskbar at z=900+)
        let needs_base = if kiosk_app != KioskApp::None {
            let is_direct_only = (kiosk_app == KioskApp::MusicPlayer && audio.is_playing())
                || (kiosk_app == KioskApp::Radio && radio.status != RadioStatus::Stopped)
                || (kiosk_app == KioskApp::TvGuide
                    && (tv.tuned.is_some() || !tv.error_msg.is_empty()));
            !is_direct_only
        } else {
            false
        };
        if needs_base {
            let _ = sdi.draw_base_layer(&mut backend);
        }
        // Skip overlay (status/bottom bars) during TV Guide video playback
        // to prevent Z-order flickering over the video frame.
        if !video_fullscreen {
            let _ = sdi.draw_overlay_layer(&mut backend);
        }

        // Post-SDI overlays drawn directly on the backend.
        if kiosk_app == KioskApp::None {
            if !icons_hidden
                && (active_theme.icon.style == "vector"
                    || !active_theme.background_layers.is_empty())
            {
                dashboard::render_vector_overlays(
                    &mut backend,
                    &mut dashboard_state,
                    &active_theme,
                    viz_frame,
                );
            }
        } else if kiosk_app == KioskApp::Terminal {
            let _ = terminal_sdi::paint_terminal_scrollbar(
                &mut backend,
                term.lines.len(),
                term.scroll,
                &active_theme,
            );
        }

        viz_frame = viz_frame.wrapping_add(1);

        // Cursor (always on top).
        let (cx, cy) = backend.cursor_pos();
        backend.blit_inner(cursor_tex, cx, cy, CURSOR_W, CURSOR_H);

        backend.swap_buffers_inner();
    }
}

// Remaining extracted modules: desktop, chrome, views, boot, theme, types.
// Command interpreter and utilities are in commands.rs module.
