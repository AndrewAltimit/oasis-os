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
//! - `frame` -- boot helpers and the per-frame stages of the main loop
//! - `input_dispatch` -- unified input event routing
//! - `io_poll` -- async I/O response polling
//! - `render_classic` -- kiosk fullscreen view rendering
//! - `render_desktop` -- windowed desktop mode rendering

#![feature(restricted_std)]
#![no_main]

use oasis_backend_psp::{
    CURSOR_H, CURSOR_W, Color, PspBackend, SCREEN_HEIGHT, SCREEN_WIDTH, SdiRegistry, StatusBarInfo,
    SystemInfo, TextureId, WindowManager,
};

// oasis-core SDI integration types.
use oasis_core::bottombar::BottomBar;
use oasis_core::statusbar::StatusBar;
use oasis_core::taskbar::Taskbar;
use oasis_core::terminal_sdi;

mod app_states;
#[cfg(feature = "autorun-script")]
mod autorun;
mod boot;
mod chrome;
mod commands;
mod dashboard;
mod desktop;
mod frame;
mod input_dispatch;
mod io_poll;
mod js_selftest;
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

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn psp_main() {
    let _ = psp::callback::setup_exit_callback();

    dbg_log("[EBOOT] psp_main entered");

    frame::install_browser_hooks();

    let mut backend = PspBackend::new();
    backend.init();
    boot::show_boot_screen(&mut backend, "Initializing...", 10);
    dbg_log("[EBOOT] backend init OK");

    // Boot-time JS self-test: run BEFORE any other init so a crash
    // during the rest of the boot can't mask the JSTEST result.
    // Sentinel deleted on success; exits cleanly so the next launch
    // from XMB boots normally. See `js_selftest`.
    if psp::io::stat("ms0:/JSTEST").is_ok() {
        boot::show_boot_screen(&mut backend, "Running JS self-test...", 20);
        oasis_backend_psp::vlog_force("[JSTEST] sentinel found, running JS probes");
        js_selftest::run();
        let _ = psp::io::remove_file("ms0:/JSTEST");
        oasis_backend_psp::vlog_force("[JSTEST] done, exiting cleanly");
        // SAFETY: terminates the application; nothing runs after it.
        unsafe { psp::sys::sceKernelExitGame() };
    }

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

    let mut wallpaper = frame::Wallpaper::new(&mut backend, current_preset, &active_theme);

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
    let mut term = frame::initial_terminal(&backend, &sysinfo);

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
    let mut settings = SettingsState::new();

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

    // Load AUTORUN.txt if present (feature-gated test scaffolding).
    #[cfg(feature = "autorun-script")]
    let mut autorun_runner = autorun::AutorunRunner::load();
    #[cfg(feature = "autorun-script")]
    if autorun_runner.is_some() {
        dbg_log("[EBOOT] autorun script loaded");
    }

    dbg_log("[EBOOT] entering main loop");
    psp::thread::sleep_ms(400);

    // Cached values for expensive kernel queries (throttled to ~4Hz).
    let mut cached_status = StatusBarInfo::poll();
    let mut cached_free_kb: i32 = 0;
    let mut cached_max_blk_kb: i32 = 0;

    // PPSSPP-driven test loop: auto-trigger a wikipedia browse on
    // boot using a hardcoded VFS resource so we can iterate on the
    // browser pipeline without needing the full deploy / reboot /
    // wifi cycle on real hardware. Gated behind the
    // `auto-browse-wiki` cargo feature so the 92 KB `test_wiki.html`
    // fixture only lands in the EBOOT data segment when explicitly
    // enabled (default builds don't pay the size cost).
    #[cfg(feature = "auto-browse-wiki")]
    const WIKI_TRUNC_BYTES: usize = 92443;
    #[cfg(feature = "auto-browse-wiki")]
    static WIKI_HTML: &[u8] = include_bytes!("../test_wiki.html");
    #[cfg(feature = "auto-browse-wiki")]
    let mut auto_browse_fired = false;
    #[cfg(feature = "auto-browse-wiki")]
    {
        use oasis_core::vfs::Vfs;
        let truncated = &WIKI_HTML[..WIKI_TRUNC_BYTES.min(WIKI_HTML.len())];
        let _ = br.vfs.write("/test_wiki.html", truncated);
        dbg_log(&format!(
            "[EBOOT] auto-browse-wiki armed: {} bytes in vfs",
            truncated.len()
        ));
    }

    loop {
        let _dt = frame_timer.tick();
        // Log first frame only.
        if viz_frame == 0 {
            dbg_log("[EBOOT] first frame tick");
        }
        // Auto-browse trigger after a brief warm-up so the boot
        // splash + first frame complete before we monopolise the
        // main thread inside navigate_vfs.
        #[cfg(feature = "auto-browse-wiki")]
        if !auto_browse_fired && viz_frame == 30 {
            auto_browse_fired = true;
            dbg_log("[EBOOT] auto-browse-wiki firing");
            kiosk_app = KioskApp::Browser;
            let _ = br.ensure_widget();
            let BrowserState { widget, vfs, .. } = &mut br;
            if let Some(w) = widget.as_mut() {
                oasis_backend_psp::vlog_force("[BR/MAIN] auto navigate_vfs start");
                w.navigate_vfs("vfs:///test_wiki.html", vfs);
                oasis_backend_psp::vlog_force("[BR/MAIN] auto navigate_vfs end");
            }
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

        // Browser tick is currently still gated off on PSP. The
        // historical reason ("std::time::Instant crashes on Allegrex")
        // turned out to be wrong — the rust-psp std overlay was
        // missing a target_os = "psp" arm in sys/time/mod.rs, so PSP
        // fell through to unsupported::Instant::now which panic!()s.
        // Fixed in rust-psp branch
        // fix/psp-hardware-std-overlay-alignment-and-time. Image
        // loading still happens synchronously during navigate_vfs;
        // the gate above can be removed whenever browser perf needs
        // progressive image loading on PSP.

        // Poll radio streaming state from audio thread atomics.
        frame::poll_radio_status(&audio, &mut radio);

        // -- Autorun script tick (one command per frame; injects events
        // into cmd_server's queue, drained by poll_events_inner below). --
        #[cfg(feature = "autorun-script")]
        if let Some(ar) = autorun_runner.as_mut() {
            ar.tick();
            if ar.is_done() {
                autorun_runner = None;
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
                &mut settings,
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
                DispatchResult::Continue => {},
            }
        }
        if should_quit {
            return;
        }

        // -- Poll video decode frames + decode-hang watchdog --
        frame::poll_video(&mut backend, &mut tv, kiosk_app, viz_frame);

        // -- Poll remote browse requests --
        frame::poll_remote_browse(&mut backend, &mut kiosk_app, &mut br);

        // -- Poll remote skin change requests --
        if let Some(key) = oasis_backend_psp::cmd_server::take_pending_skin() {
            let preset = skins::PspSkinPreset::from_key(&key);
            if skins::apply_skin_preset(
                preset,
                &mut current_preset,
                &mut active_theme,
                &skin_features,
                &mut dashboard_state,
                &mut config,
            ) {
                dbg_log(&format!("[SKIN] changed to '{}' via TCP", preset.name()));
            }
        }

        // -- Render --
        // Skip clear + wallpaper when video covers the entire screen.
        let video_fullscreen =
            kiosk_app == KioskApp::TvGuide && tv.tuned.is_some() && tv.preview_tex.is_some();

        // During fullscreen video, skip all non-video rendering:
        // status bar polling (6+ kernel calls), bar updates, wallpaper,
        // SDI updates, WM operations. Just blit video + swap.
        if !video_fullscreen {
            // Throttle expensive kernel syscalls (~4Hz at 60fps).
            if viz_frame % 15 == 0 {
                cached_status = StatusBarInfo::poll();
            }

            // Feed PSP status info into oasis-core's StatusBar.
            frame::feed_status_bars(&cached_status, &sysinfo, &mut status_bar, &mut bottom_bar);

            // Update bottom bar page tracking.
            bottom_bar.current_page = dashboard_state.page;
            bottom_bar.total_pages = dashboard_state.page_count();
            bottom_bar.tick_animation(&active_theme);

            backend.clear_inner(Color::BLACK);

            wallpaper.draw(&mut backend, current_preset, &active_theme, viz_frame);
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
                &mut fm,
                &mut pv,
                &mut mp,
                &mut br,
                &mut radio,
                &mut tv,
                &settings,
                current_preset,
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
                chrome::draw_dashboard(&mut backend, dashboard_state.page, &active_theme);
            }

            // Kiosk transition: hide old SDI objects, restore window decorations.
            if prev_kiosk_app != KioskApp::None {
                views_sdi::hide_all(&mut sdi);
                wm.show_all_window_sdi(&mut sdi);
                prev_kiosk_app = KioskApp::None;
            }

            // Throttle kernel heap queries (~4Hz).
            if viz_frame % 15 == 0 {
                (cached_free_kb, cached_max_blk_kb) = oasis_backend_psp::heap_kb();
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
            // The bar lays out first and leaves the taskbar its free span
            // (clear of the URL plate, USB label and media dock), as on
            // the desktop shell.
            bottom_bar.reserve_taskbar = !wm.windows().is_empty();
            bottom_bar.update_sdi(&mut sdi, &active_theme, &skin_features);
            taskbar.update_sdi_in(
                &mut sdi,
                &active_theme,
                wm.windows(),
                wm.active_window(),
                bottom_bar.taskbar_span(),
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
            // Status bar battery / AC glyphs (vector art over the bar).
            let _ = oasis_core::statusbar::render_status_glyphs(&mut backend, &sdi);
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

/// Append a line to `eboot.log`. Passed down as the `dbg_log` callback.
fn dbg_log(msg: &str) {
    oasis_backend_psp::vlog_force(msg);
}

/// Low 32 bits of the microsecond system clock (for short interval timing).
fn now_us() -> u32 {
    // SAFETY: argument-less clock read.
    unsafe { psp::sys::sceKernelGetSystemTimeLow() }
}
