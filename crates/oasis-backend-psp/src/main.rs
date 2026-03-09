//! PSP entry point for OASIS_OS.
//!
//! PSIX-style dashboard with document icons, tabbed status bar, chrome bezel
//! bottom bar, terminal mode, and windowed desktop mode with floating windows
//! managed by the oasis-core WindowManager.
//!
//! Audio playback and file I/O run on background threads to prevent frame drops.
//!
//! Major subsystems are split into modules:
//! - `app_states` -- per-app mutable state structs
//! - `dashboard` -- dashboard init and SDI helpers
//! - `input_dispatch` -- input event routing for Classic and Desktop modes

#![feature(restricted_std)]
#![feature(asm_experimental_arch)]
#![no_main]

use oasis_backend_psp::{
    AudioCmd, CURSOR_H, CURSOR_W, Color, InputEvent, IoResponse, PspBackend, SCREEN_HEIGHT,
    SCREEN_WIDTH, SdiRegistry, StatusBarInfo, SystemInfo, TextureId, WindowManager,
};

// oasis-core SDI integration types.
use oasis_core::bottombar::BottomBar;
use oasis_core::platform::{BatteryState, CpuClock, PowerInfo, SystemTime};
use oasis_core::statusbar::StatusBar;
use oasis_core::terminal_sdi;

mod app_states;
mod boot;
mod chrome;
mod commands;
mod dashboard;
mod desktop;
mod input_dispatch;
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
// Custom getrandom backends for PSP (no native OS entropy source).
// Uses the PSP's hardware MT19937 PRNG via sceKernelUtils.
// ---------------------------------------------------------------------------

/// getrandom 0.2 custom backend (used by transitive deps like webpki).
mod psp_getrandom_v02 {
    use psp::sys::{
        sceKernelGetSystemTimeLow, sceKernelUtilsMt19937Init, sceKernelUtilsMt19937UInt,
    };

    fn psp_fill_random(buf: &mut [u8]) -> Result<(), getrandom_02::Error> {
        // SAFETY: MT19937 context is initialized by sceKernelUtilsMt19937Init
        // before any reads. Seed from system timer (user-mode safe).
        // mfc0 $9 (COP0 Count) is privileged on PSP Allegrex.
        unsafe {
            let mut ctx = core::mem::MaybeUninit::uninit();
            let seed = sceKernelGetSystemTimeLow() as u32;
            sceKernelUtilsMt19937Init(ctx.as_mut_ptr(), seed);
            let mut ctx = ctx.assume_init();
            for byte in buf.iter_mut() {
                *byte = (sceKernelUtilsMt19937UInt(&mut ctx) & 0xFF) as u8;
            }
        }
        Ok(())
    }

    getrandom_02::register_custom_getrandom!(psp_fill_random);
}

/// getrandom 0.3 custom backend (enabled via `--cfg getrandom_backend="custom"`
/// in `.cargo/config.toml`).
#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    use psp::sys::{
        sceKernelGetSystemTimeLow, sceKernelUtilsMt19937Init, sceKernelUtilsMt19937UInt,
    };
    // SAFETY: MT19937 context is initialized by sceKernelUtilsMt19937Init
    // before any reads. Seed from system timer (user-mode safe).
    // mfc0 $9 (COP0 Count) is privileged on PSP Allegrex.
    unsafe {
        let mut ctx = core::mem::MaybeUninit::uninit();
        let seed = sceKernelGetSystemTimeLow() as u32;
        sceKernelUtilsMt19937Init(ctx.as_mut_ptr(), seed);
        let mut ctx = ctx.assume_init();
        for i in 0..len {
            *dest.add(i) = (sceKernelUtilsMt19937UInt(&mut ctx) & 0xFF) as u8;
        }
    }
    Ok(())
}

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

    // -- App mode --
    let mut app_mode = AppMode::Classic;
    let mut classic_view = ClassicView::Dashboard;
    let mut prev_classic_view = ClassicView::Dashboard;

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
    let mut radio = RadioState::new();
    let mut tv = TvGuideState::new();

    // USB storage mode handle (RAII: drop exits storage mode).
    let mut usb_storage: Option<psp::usb::UsbStorageMode> = None;

    // AV codec modules (AvCodec, AvMpegBase, AvMp3) are loaded lazily
    // by the audio thread on first play. Loading them here at startup
    // would conflict with the PRX overlay's sceAudiocodec if the PRX
    // initialized before the EBOOT was launched.

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
    dbg_log("[EBOOT] entering main loop");
    psp::thread::sleep_ms(400);

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
        poll_io_responses(
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

        // -- Input dispatch --
        let events = backend.poll_events_inner();
        let mut should_quit = false;

        for event in &events {
            if app_mode == AppMode::Desktop {
                match event {
                    InputEvent::ButtonRelease(oasis_backend_psp::Button::Confirm) => {
                        _confirm_held = false;
                    },
                    InputEvent::ButtonPress(oasis_backend_psp::Button::Confirm) => {
                        _confirm_held = true;
                    },
                    _ => {},
                }
                match input_dispatch::dispatch_desktop(
                    event,
                    &mut backend,
                    &mut app_mode,
                    &mut classic_view,
                    &mut dashboard_state,
                    &mut wm,
                    &mut sdi,
                    &mut term,
                    &audio,
                ) {
                    DispatchResult::Quit => {
                        should_quit = true;
                        break;
                    },
                    DispatchResult::SkipRest | DispatchResult::Continue => continue,
                }
            }

            // Classic mode input.
            match input_dispatch::dispatch_classic(
                event,
                &mut backend,
                &mut app_mode,
                &mut classic_view,
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
        if tv.tuned.is_some() && !tv.downloading {
            if let Some(frame) = oasis_backend_psp::video::poll_video_frame() {
                if let Some(old) = tv.preview_tex.take() {
                    backend.destroy_texture_inner(old);
                }
                tv.preview_tex = backend.load_texture_inner(frame.width, frame.height, &frame.rgba);
            }
            if !oasis_backend_psp::video::is_video_playing() {
                if let Some(old) = tv.preview_tex.take() {
                    backend.destroy_texture_inner(old);
                }
                tv.tuned = None;
                tv.now_playing.clear();
            }
        }

        // -- Render --
        let status = StatusBarInfo::poll();
        let fps = frame_timer.fps();
        let usb_active = usb_storage.is_some();

        // Feed PSP status info into oasis-core's StatusBar for SDI rendering.
        {
            let sys_time = SystemTime {
                year: status.year,
                month: status.month as u8,
                day: status.day as u8,
                hour: status.hour as u8,
                minute: status.minute as u8,
                second: 0,
            };
            let bat_state = if status.ac_power && !status.battery_charging {
                BatteryState::Full
            } else if status.battery_charging {
                BatteryState::Charging
            } else if status.battery_percent < 0 {
                BatteryState::NoBattery
            } else {
                BatteryState::Discharging
            };
            let power = PowerInfo {
                battery_percent: if status.battery_percent >= 0 {
                    Some(status.battery_percent as u8)
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

        match app_mode {
            AppMode::Classic => {
                render_classic(
                    &mut backend,
                    &mut sdi,
                    &mut dashboard_state,
                    &active_theme,
                    classic_view,
                    &mut prev_classic_view,
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
            },

            AppMode::Desktop => {
                // Show dashboard icons behind windows in Desktop mode.
                if !icons_hidden {
                    dashboard::show_dashboard_sdi(&mut dashboard_state, &mut sdi, &active_theme);
                } else {
                    dashboard::hide_dashboard_sdi(&mut dashboard_state, &mut sdi);
                }
                terminal_sdi::set_terminal_visible(&mut sdi, false);

                render_desktop(
                    &mut backend,
                    &mut wm,
                    &mut sdi,
                    &config,
                    &status,
                    &sysinfo,
                    fps,
                    usb_active,
                    &term,
                    &fm,
                    &pv,
                    &mp,
                    &audio,
                    &br,
                );
            },
        }

        // Status bar + bottom bar (always visible, drawn on top via SDI).
        active_theme.bar.url_text =
            compute_url_text(app_mode, classic_view, &fm, fm.umd_activated, &audio, &tv);
        status_bar.update_sdi(&mut sdi, &active_theme, &skin_features);
        bottom_bar.update_sdi(&mut sdi, &active_theme, &skin_features);

        // On PSP, draw SDI in two passes to control cost:
        // - Base layer only when dashboard/terminal are active (icons, term lines)
        // - Overlay layer always (status bar, bottom bar at z=900)
        let needs_base = match app_mode {
            AppMode::Classic => {
                let is_direct_only = (classic_view == ClassicView::MusicPlayer
                    && audio.is_playing())
                    || (classic_view == ClassicView::Radio && radio.status != RadioStatus::Stopped)
                    || (classic_view == ClassicView::TvGuide
                        && (tv.tuned.is_some() || !tv.error_msg.is_empty()));
                !is_direct_only
            },
            AppMode::Desktop => false,
        };
        if needs_base {
            let _ = sdi.draw_base_layer(&mut backend);
        }
        let _ = sdi.draw_overlay_layer(&mut backend);

        // Post-SDI overlays drawn directly on the backend.
        if app_mode == AppMode::Classic {
            match classic_view {
                ClassicView::Dashboard if !icons_hidden && active_theme.icon.style == "vector" => {
                    dashboard::render_vector_overlays(
                        &mut backend,
                        &mut dashboard_state,
                        &active_theme,
                        viz_frame,
                    );
                },
                ClassicView::Terminal => {
                    let _ = terminal_sdi::paint_terminal_scrollbar(
                        &mut backend,
                        term.lines.len(),
                        term.scroll,
                        &active_theme,
                    );
                },
                _ => {},
            }
        }

        viz_frame = viz_frame.wrapping_add(1);

        // Cursor (always on top).
        let (cx, cy) = backend.cursor_pos();
        backend.blit_inner(cursor_tex, cx, cy, CURSOR_W, CURSOR_H);

        backend.swap_buffers_inner();
    }
}

// ---------------------------------------------------------------------------
// I/O response polling (extracted from main loop body)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn poll_io_responses(
    io: &oasis_backend_psp::threading::IoHandle,
    audio: &oasis_backend_psp::AudioHandle,
    backend: &mut PspBackend,
    term: &mut TerminalState,
    pv: &mut PhotoViewerState,
    br: &mut BrowserState,
    radio: &mut RadioState,
    tv: &mut TvGuideState,
    dbg_log: &dyn Fn(&str),
) {
    while let Some(resp) = io.try_recv() {
        match resp {
            IoResponse::TextureReady {
                path: _,
                width,
                height,
                rgba,
            } => {
                if pv.loading {
                    if let Some(old) = pv.tex.take() {
                        backend.destroy_texture_inner(old);
                    }
                    pv.tex = backend.load_texture_inner(width, height, &rgba);
                    pv.img_w = width;
                    pv.img_h = height;
                    pv.viewing = true;
                    pv.loading = false;
                }
            },
            IoResponse::Error { path, msg } => {
                dbg_log(&format!("[IO] error: {} - {}", path, msg));
                term.lines.push(format!("I/O error: {} - {}", path, msg));
                pv.loading = false;
                if br.loading {
                    br.loading = false;
                    br.status_msg = format!("Error: {}", msg);
                }
            },
            IoResponse::FileReady { .. } => {},
            IoResponse::HttpDone {
                tag,
                status_code,
                body,
            } => {
                if tag == 0xBEEF {
                    let html = String::from_utf8_lossy(&body);
                    let text = views::strip_html(&html);
                    br.content_lines = views::wrap_text(&text, 58);
                    br.scroll = 0;
                    br.loading = false;
                    br.status_msg = format!("HTTP {} - {} bytes", status_code, body.len());
                } else if (tag & 0xFF00) == 0xAA00 {
                    // Legacy TV Guide tag -- no longer used.
                    let _ = (tag, body);
                } else {
                    let preview = String::from_utf8_lossy(&body[..body.len().min(256)]);
                    term.lines.push(format!(
                        "HTTP {status_code} ({} bytes): {preview}",
                        body.len(),
                    ));
                }
            },
            IoResponse::TvCatalogReady { ch_idx, episodes } => {
                dbg_log(&format!(
                    "[TV] catalog ready ch={ch_idx} episodes={}",
                    episodes.len()
                ));
                if ch_idx < tv.channels.len() {
                    let ch = &tv.channels[ch_idx];
                    let catalog = tv.catalogs[ch_idx].get_or_insert_with(|| {
                        oasis_core::apps::tv_guide::ChannelCatalog::new(ch.number)
                    });
                    if !episodes.is_empty() {
                        catalog.add_episodes(episodes);
                    }
                }
            },
            IoResponse::RadioConnected {
                fd,
                icy_metaint,
                initial_data,
            } => {
                radio.status = RadioStatus::Buffering;
                audio.send(AudioCmd::RadioStreamFromFd {
                    fd,
                    icy_metaint,
                    initial_data,
                });
            },
            IoResponse::RadioError { msg } => {
                radio.status = RadioStatus::Error;
                radio.error_msg = msg;
            },
            IoResponse::VideoProgress {
                tag: _,
                bytes,
                total,
            } => {
                if let Some(t) = total {
                    if t > 0 {
                        tv.download_progress = bytes as f32 / t as f32;
                    }
                }
            },
            IoResponse::VideoReady { tag: _, path } => {
                tv.downloading = false;
                tv.download_progress = 1.0;
                oasis_backend_psp::video::send_video_cmd(
                    oasis_backend_psp::video::VideoCmd::Play { path, seek_secs: 0 },
                );
            },
            IoResponse::VideoStreamReady { tag: _, .. } => {
                tv.downloading = false;
                tv.download_progress = 1.0;
            },
            IoResponse::VideoError { tag: _, msg } => {
                tv.downloading = false;
                tv.error_msg = format!("Download: {msg}");
                tv.tuned = None;
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Classic mode rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_classic(
    backend: &mut PspBackend,
    sdi: &mut SdiRegistry,
    dashboard_state: &mut oasis_core::dashboard::DashboardState,
    active_theme: &oasis_core::active_theme::ActiveTheme,
    classic_view: ClassicView,
    prev_classic_view: &mut ClassicView,
    icons_hidden: bool,
    fm: &mut FileManagerState,
    pv: &mut PhotoViewerState,
    mp: &mut MusicPlayerState,
    br: &mut BrowserState,
    radio: &mut RadioState,
    tv: &mut TvGuideState,
    term: &mut TerminalState,
    audio: &oasis_backend_psp::AudioHandle,
    viz_frame: u32,
    dbg_log: &dyn Fn(&str),
) {
    // Lazy-load directory entries for browser modes.
    if classic_view == ClassicView::FileManager && !fm.left.loaded {
        fm.left.entries = oasis_backend_psp::list_directory(&fm.left.path);
        fm.left.selected = 0;
        fm.left.scroll = 0;
        fm.left.loaded = true;
    }
    if classic_view == ClassicView::FileManager && !fm.right.loaded {
        fm.right.entries = oasis_backend_psp::list_directory(&fm.right.path);
        fm.right.selected = 0;
        fm.right.scroll = 0;
        fm.right.loaded = true;
    }
    if classic_view == ClassicView::PhotoViewer && !pv.loaded && !pv.viewing {
        let all = oasis_backend_psp::list_directory(&pv.path);
        pv.entries = all
            .into_iter()
            .filter(|e| {
                e.is_dir || {
                    let lower: String = e.name.chars().map(|c| c.to_ascii_lowercase()).collect();
                    lower.ends_with(".jpg") || lower.ends_with(".jpeg")
                }
            })
            .collect();
        pv.selected = 0;
        pv.scroll = 0;
        pv.loaded = true;
    }
    if classic_view == ClassicView::MusicPlayer && !mp.loaded && !audio.is_playing() {
        let all = oasis_backend_psp::list_directory(&mp.path);
        mp.entries = all
            .into_iter()
            .filter(|e| {
                e.is_dir || {
                    let lower: String = e.name.chars().map(|c| c.to_ascii_lowercase()).collect();
                    lower.ends_with(".mp3")
                }
            })
            .collect();
        mp.selected = 0;
        mp.scroll = 0;
        mp.loaded = true;
    }

    // Show or hide dashboard icons based on current view.
    let show_dashboard = classic_view == ClassicView::Dashboard && !icons_hidden;
    if show_dashboard {
        dashboard::show_dashboard_sdi(dashboard_state, sdi, active_theme);
    } else {
        dashboard::hide_dashboard_sdi(dashboard_state, sdi);
    }

    // Show or hide terminal SDI objects.
    if classic_view != ClassicView::Terminal {
        terminal_sdi::set_terminal_visible(sdi, false);
    }

    // View transition: hide old SDI objects, set up new ones.
    if classic_view != *prev_classic_view {
        views_sdi::hide_all(sdi);
        views_sdi::setup_view(sdi, classic_view);
        *prev_classic_view = classic_view;
    }

    match classic_view {
        ClassicView::Dashboard => {
            backend.force_bitmap_font = true;
            chrome::draw_button_hints(
                backend,
                &[
                    ("X", "Open"),
                    ("L/R", "Window"),
                    ("Start", "Term"),
                    ("Sel", "Desktop"),
                ],
            );
            backend.force_bitmap_font = false;
        },
        ClassicView::Terminal => {
            terminal_sdi::setup_terminal_objects(
                sdi,
                &term.lines,
                "/",
                &term.input,
                term.scroll,
                active_theme,
                viz_frame % 30 < 15,
            );
            backend.force_bitmap_font = true;
            chrome::draw_button_hints(
                backend,
                &[
                    ("X", "Run"),
                    ("[]", "OSK"),
                    ("Up/Dn", "Scroll"),
                    ("Start", "Back"),
                ],
            );
            backend.force_bitmap_font = false;
        },
        ClassicView::FileManager => {
            views_sdi::update_file_manager(
                sdi,
                &fm.left.path,
                &fm.left.entries,
                fm.left.selected,
                fm.left.scroll,
                &fm.right.path,
                &fm.right.entries,
                fm.right.selected,
                fm.right.scroll,
                fm.active_panel,
                active_theme,
            );
            backend.force_bitmap_font = true;
            chrome::draw_button_hints(
                backend,
                &[("X", "Open"), ("O", "Back"), ("<>", "Panel"), ("^v", "Nav")],
            );
            backend.force_bitmap_font = false;
        },
        ClassicView::PhotoViewer => {
            if pv.viewing {
                views_sdi::update_photo_view(sdi, pv.tex, pv.img_w, pv.img_h);
                backend.force_bitmap_font = true;
                chrome::draw_button_hints(backend, &[("O", "Back")]);
                backend.force_bitmap_font = false;
            } else if pv.loading {
                desktop::draw_loading_indicator(backend, "Decoding image...");
            } else {
                views_sdi::update_photo_browser(
                    sdi,
                    &pv.path,
                    &pv.entries,
                    pv.selected,
                    pv.scroll,
                    active_theme,
                );
                backend.force_bitmap_font = true;
                chrome::draw_button_hints(backend, &[("X", "View"), ("O", "Back"), ("^v", "Nav")]);
                backend.force_bitmap_font = false;
            }
        },
        ClassicView::MusicPlayer => {
            if audio.is_playing() {
                backend.force_bitmap_font = true;
                views::draw_music_player_threaded(backend, &mp.file_name, audio, viz_frame);
                chrome::draw_button_hints(
                    backend,
                    &[("X", "Pause"), ("[]", "Stop"), ("^v", "Back")],
                );
                backend.force_bitmap_font = false;
            } else {
                views_sdi::update_music_browser(
                    sdi,
                    &mp.path,
                    &mp.entries,
                    mp.selected,
                    mp.scroll,
                    active_theme,
                );
                backend.force_bitmap_font = true;
                chrome::draw_button_hints(backend, &[("X", "Play"), ("O", "Back"), ("^v", "Nav")]);
                backend.force_bitmap_font = false;
            }
        },
        ClassicView::Browser => {
            if br.loading {
                desktop::draw_loading_indicator(backend, "Loading page...");
            } else {
                views_sdi::update_browser(
                    sdi,
                    &br.url,
                    &br.content_lines,
                    br.scroll,
                    &br.status_msg,
                    active_theme,
                );
            }
            backend.force_bitmap_font = true;
            chrome::draw_button_hints(
                backend,
                &[
                    ("[]", "URL"),
                    ("X", "Load"),
                    ("^v", "Scroll"),
                    ("O", "Back"),
                ],
            );
            backend.force_bitmap_font = false;
        },
        ClassicView::Radio => {
            backend.force_bitmap_font = true;
            match radio.status {
                RadioStatus::Stopped => {
                    views_sdi::update_radio(sdi, radio.selected, radio.scroll, active_theme);
                    chrome::draw_button_hints(
                        backend,
                        &[("X", "Tune"), ("^v", "Nav"), ("O", "Back")],
                    );
                },
                RadioStatus::Connecting => {
                    desktop::draw_loading_indicator(backend, "Connecting...");
                },
                RadioStatus::Buffering | RadioStatus::Playing => {
                    views::draw_radio_playing(
                        backend,
                        &radio.station_name,
                        &radio.now_playing,
                        radio.status == RadioStatus::Buffering,
                        audio,
                        viz_frame,
                    );
                    chrome::draw_button_hints(
                        backend,
                        &[("[]", "Stop"), ("^", "Back"), ("O", "Stop+Back")],
                    );
                },
                RadioStatus::Error => {
                    views::draw_radio_error(backend, &radio.error_msg);
                    chrome::draw_button_hints(backend, &[("X", "Retry"), ("O", "Back")]);
                },
            }
            backend.force_bitmap_font = false;
        },
        ClassicView::TvGuide => {
            if viz_frame < 3 || viz_frame % 60 == 0 {
                dbg_log(&format!("[TV] render frame {}", viz_frame));
            }
            backend.force_bitmap_font = true;
            if tv.tuned.is_some() {
                views::draw_tv_playing(
                    backend,
                    &tv.now_playing,
                    tv.downloading,
                    tv.download_progress,
                    tv.preview_tex,
                    &tv.error_msg,
                );
                chrome::draw_button_hints(backend, &[("O", "Untune"), ("^", "Back")]);
            } else if !tv.error_msg.is_empty() {
                views::draw_tv_error(backend, &tv.error_msg);
                chrome::draw_button_hints(backend, &[("X", "Retry"), ("O", "Back")]);
            } else {
                views_sdi::update_tv_channels(
                    sdi,
                    &tv.channels,
                    &tv.catalogs,
                    tv.selected,
                    tv.scroll,
                    active_theme,
                );
                chrome::draw_button_hints(backend, &[("X", "Tune"), ("^v", "Nav"), ("O", "Back")]);
            }
            backend.force_bitmap_font = false;
        },
    }
}

// ---------------------------------------------------------------------------
// Desktop mode rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_desktop(
    backend: &mut PspBackend,
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    config: &psp::config::Config,
    status: &StatusBarInfo,
    sysinfo: &SystemInfo,
    fps: f32,
    usb_active: bool,
    term: &TerminalState,
    fm: &FileManagerState,
    pv: &PhotoViewerState,
    mp: &MusicPlayerState,
    audio: &oasis_backend_psp::AudioHandle,
    br: &BrowserState,
) {
    let settings_clock = config.get_i32("clock_mhz").unwrap_or(333);
    let settings_bus = config.get_i32("bus_mhz").unwrap_or(166);
    let current_vol = backend.volatile_mem_info();
    // SAFETY: scalar FFI returning available memory stats.
    let (free_kb, max_blk_kb) = unsafe {
        (
            psp::sys::sceKernelTotalFreeMemSize() as i32 / 1024,
            psp::sys::sceKernelMaxFreeMemSize() as i32 / 1024,
        )
    };

    backend.force_bitmap_font = true;
    let _ = wm.draw_with_clips(
        sdi,
        backend,
        |window_id, cx, cy, cw, ch, be| match window_id {
            "terminal" => {
                desktop::draw_terminal_windowed(&term.lines, &term.input, cx, cy, cw, ch, be)
            },
            "filemgr" => desktop::draw_filemgr_windowed(
                &fm.left.path,
                &fm.left.entries,
                fm.left.selected,
                fm.left.scroll,
                &fm.right.path,
                &fm.right.entries,
                fm.right.selected,
                fm.right.scroll,
                fm.active_panel,
                cx,
                cy,
                cw,
                ch,
                be,
            ),
            "photos" => desktop::draw_photos_windowed(
                pv.tex, pv.img_w, pv.img_h, pv.viewing, cx, cy, cw, ch, be,
            ),
            "music" => desktop::draw_music_windowed(&mp.file_name, audio, cx, cy, cw, ch, be),
            "settings" => desktop::draw_settings_windowed(
                settings_clock,
                settings_bus,
                current_vol,
                cx,
                cy,
                cw,
                ch,
                be,
            ),
            "network" => desktop::draw_network_windowed(status, cx, cy, cw, ch, be),
            "sysmon" => desktop::draw_sysmon_windowed(
                status,
                sysinfo,
                fps,
                free_kb,
                max_blk_kb,
                current_vol,
                usb_active,
                cx,
                cy,
                cw,
                ch,
                be,
            ),
            "browser" => desktop::draw_browser_windowed(cx, cy, cw, ch, be),
            "packages" => desktop::draw_packages_windowed(cx, cy, cw, ch, be),
            "radio" => desktop::draw_radio_windowed(audio, cx, cy, cw, ch, be),
            _ => Ok(()),
        },
    );
    backend.force_bitmap_font = false;
}

// ---------------------------------------------------------------------------
// URL text computation
// ---------------------------------------------------------------------------

fn compute_url_text(
    app_mode: AppMode,
    classic_view: ClassicView,
    fm: &FileManagerState,
    umd_activated: bool,
    audio: &oasis_backend_psp::AudioHandle,
    tv: &TvGuideState,
) -> String {
    match (app_mode, classic_view) {
        (AppMode::Desktop, _) => "SYS://DESKTOP".to_string(),
        (_, ClassicView::Dashboard) => "SYS://DASHBOARD".to_string(),
        (_, ClassicView::Terminal) => "SYS://TERMINAL".to_string(),
        (_, ClassicView::FileManager) => {
            let active_path = if fm.active_panel == 0 {
                &fm.left.path
            } else {
                &fm.right.path
            };
            let path_part = if active_path.len() > 14 {
                let start = active_path.ceil_char_boundary(active_path.len() - 14);
                &active_path[start..]
            } else {
                active_path.as_str()
            };
            if umd_activated {
                format!("UMD:{}", path_part)
            } else {
                format!("MSO:/{}", path_part)
            }
        },
        (_, ClassicView::PhotoViewer) => "SYS://PHOTOS".to_string(),
        (_, ClassicView::MusicPlayer) => {
            if audio.is_playing() {
                "SYS://NOW_PLAY".to_string()
            } else {
                "SYS://MUSIC".to_string()
            }
        },
        (_, ClassicView::Browser) => "SYS://BROWSER".to_string(),
        (_, ClassicView::Radio) => {
            if audio.is_radio_streaming() {
                "SYS://RADIO_ON".to_string()
            } else {
                "SYS://RADIO".to_string()
            }
        },
        (_, ClassicView::TvGuide) => {
            if tv.tuned.is_some() {
                "SYS://TV_LIVE".to_string()
            } else {
                "SYS://TV_GUIDE".to_string()
            }
        },
    }
}

// Remaining extracted modules: desktop, chrome, views, boot, theme, types.
// Command interpreter and utilities are in commands.rs module.
