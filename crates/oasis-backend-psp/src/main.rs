//! PSP entry point for OASIS_OS.
//!
//! PSIX-style dashboard with document icons, tabbed status bar, chrome bezel
//! bottom bar, terminal mode, and windowed desktop mode with floating windows
//! managed by the oasis-core WindowManager.
//!
//! Audio playback and file I/O run on background threads to prevent frame drops.

#![feature(restricted_std)]
#![feature(asm_experimental_arch)]
#![no_main]

use psp::sys::CtrlButtons;

use oasis_backend_psp::{
    AudioCmd, Button, CURSOR_H, CURSOR_W, Color, FileEntry, InputEvent, IoCmd, IoResponse,
    PspBackend, SCREEN_HEIGHT, SCREEN_WIDTH, SdiRegistry, SfxId, StatusBarInfo, SystemInfo,
    TextureId, Trigger, WindowManager,
};

// oasis-core SDI integration types.
use oasis_core::active_theme::ActiveTheme;
use oasis_core::bottombar::BottomBar;
use oasis_core::dashboard::{AppEntry as CoreAppEntry, DashboardConfig, DashboardState};
use oasis_core::platform::{BatteryState, CpuClock, PowerInfo, SystemTime};
use oasis_core::skin::SkinFeatures;
use oasis_core::statusbar::StatusBar;
use oasis_core::terminal_sdi;

mod boot;
mod chrome;
mod commands;
mod desktop;
mod theme;
mod types;
mod views;

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
    use psp::sys::{sceKernelUtilsMt19937Init, sceKernelUtilsMt19937UInt};

    fn psp_fill_random(buf: &mut [u8]) -> Result<(), getrandom_02::Error> {
        // SAFETY: MT19937 context is initialized by sceKernelUtilsMt19937Init
        // before any reads. MaybeUninit avoids potential UB from zeroing a
        // struct with padding or invariant fields. Seed from CPU cycle counter.
        unsafe {
            let mut ctx = core::mem::MaybeUninit::uninit();
            let seed: u32;
            core::arch::asm!("mfc0 {}, $9", out(reg) seed);
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
    use psp::sys::{sceKernelUtilsMt19937Init, sceKernelUtilsMt19937UInt};
    // SAFETY: MT19937 context is initialized by sceKernelUtilsMt19937Init
    // before any reads. MaybeUninit avoids potential UB from zeroing a
    // struct with padding or invariant fields. Seed from CPU cycle counter.
    unsafe {
        let mut ctx = core::mem::MaybeUninit::uninit();
        let seed: u32;
        core::arch::asm!("mfc0 {}, $9", out(reg) seed);
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
    // Use default theme directly to avoid pulling in the TOML parser and
    // 17 embedded skin files (~850KB code). ActiveTheme::default() provides
    // PSIX-style layout already matched to 480x272.
    dbg_log("[EBOOT] creating theme...");
    let mut active_theme = ActiveTheme::default()
        .with_screen_size(SCREEN_WIDTH, SCREEN_HEIGHT);
    // PSP: make bar backgrounds opaque to prevent darkening window content.
    // Default is semi-transparent (alpha=80) which looks muddy on 480x272.
    active_theme.bar.statusbar_bg = Color::rgba(10, 10, 20, 255);
    active_theme.bar.bg = Color::rgba(10, 10, 20, 255);
    let skin_features = SkinFeatures::default();
    dbg_log("[EBOOT] active_theme created");
    let dash_config = DashboardConfig::from_features(&skin_features, &active_theme);

    // Convert PSP app list to oasis-core AppEntry for DashboardState.
    let core_apps: Vec<CoreAppEntry> = APPS
        .iter()
        .map(|a| CoreAppEntry {
            title: a.title.to_string(),
            path: format!("/apps/{}", a.id),
            icon_png: Vec::new(),
            color: a.color,
        })
        .collect();
    let mut dashboard = DashboardState::new(dash_config, core_apps);
    dbg_log("[EBOOT] dashboard created");

    let mut status_bar = StatusBar::new();
    let mut bottom_bar = BottomBar::new();
    bottom_bar.total_pages = dashboard.page_count();

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

    let mut icons_hidden: bool = false;
    let mut viz_frame: u32 = 0;

    // Terminal state.
    let vol_info = backend.volatile_mem_info();
    let mode_label = if cfg!(feature = "kernel-mode") {
        "kernel"
    } else {
        "user"
    };
    let mut term_lines: Vec<String> = vec![
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
    let mut term_input = String::new();
    // Scroll offset: 0 means "show latest lines" (auto-scroll).
    // Positive values scroll back into history.
    let mut term_scroll: usize = 0;

    // Try to restore previous terminal history from save data (silent).
    if let Ok(saved) = commands::load_terminal_history() {
        if !saved.is_empty() {
            term_lines.push(String::from("(restored previous session)"));
            term_lines.extend(saved);
            term_lines.push(String::new());
        }
    }

    // Boot-time self-test: if sentinel file exists, run test suite,
    // write results to selftest.log, delete sentinel, then exit.
    if psp::io::stat(commands::SELFTEST_SENTINEL).is_ok() {
        boot::show_boot_screen(&mut backend, "Running self-test...", 90);
        let results = commands::run_selftest(&mut config);
        // Delete the sentinel so next boot is normal.
        let _ = psp::io::remove_file(commands::SELFTEST_SENTINEL);
        // Show results briefly on screen, then exit.
        for line in &results {
            term_lines.push(line.clone());
        }
        // Render one frame so the results are visible in screenshots.
        backend.clear_inner(Color::rgb(0, 0, 0));
        let y_start = 4i32;
        for (i, line) in results.iter().enumerate().take(30) {
            backend.draw_text_inner(line, 4, y_start + (i as i32 * 9), 8, Color::WHITE);
        }
        backend.swap_buffers_inner();
        // Wait a moment for screenshot capture, then exit.
        psp::thread::sleep_ms(2000);
        // SAFETY: sceKernelExitGame terminates the running application.
        unsafe { psp::sys::sceKernelExitGame() };
    }

    // File manager dual-panel state.
    let mut fm_path = String::from("ms0:/");
    let mut fm_entries: Vec<FileEntry> = Vec::new();
    let mut fm_selected: usize = 0;
    let mut fm_scroll: usize = 0;
    let mut fm_loaded = false;

    let mut fm2_path = String::from("ms0:/");
    let mut fm2_entries: Vec<FileEntry> = Vec::new();
    let mut fm2_selected: usize = 0;
    let mut fm2_scroll: usize = 0;
    let mut fm2_loaded = false;

    // 0 = left panel, 1 = right panel.
    let mut fm_active_panel: usize = 0;

    // UMD drive state.
    let mut umd_activated = false;

    // USB storage mode handle (RAII: drop exits storage mode).
    let mut usb_storage: Option<psp::usb::UsbStorageMode> = None;

    // Photo viewer state.
    let mut pv_path = String::from("ms0:/");
    let mut pv_entries: Vec<FileEntry> = Vec::new();
    let mut pv_selected: usize = 0;
    let mut pv_scroll: usize = 0;
    let mut pv_loaded = false;
    let mut pv_viewing = false;
    let mut pv_tex: Option<TextureId> = None;
    let mut pv_img_w: u32 = 0;
    let mut pv_img_h: u32 = 0;

    // Music player state (background thread).
    let mut mp_path = String::from("ms0:/");
    let mut mp_entries: Vec<FileEntry> = Vec::new();
    let mut mp_selected: usize = 0;
    let mut mp_scroll: usize = 0;
    let mut mp_loaded = false;
    let mut mp_file_name = String::new();

    // Browser state.
    let mut br_url = String::from("http://info.cern.ch");
    let mut br_content_lines: Vec<String> = Vec::new();
    let mut br_scroll: usize = 0;
    let mut br_loading = false;
    let mut br_status_msg = String::from("Press [] to enter URL");

    // Radio state.
    let mut radio_selected: usize = 0;
    let mut radio_scroll: usize = 0;
    let mut radio_status = RadioStatus::Stopped;
    let mut radio_station_name = String::new();
    let mut radio_now_playing = String::new();
    let mut radio_error_msg = String::new();

    // TV Guide state.
    let mut tv_channels: Vec<oasis_core::apps::tv_guide::Channel> = Vec::new();
    let mut tv_catalogs: Vec<Option<oasis_core::apps::tv_guide::ChannelCatalog>> = Vec::new();
    let mut tv_selected: usize = 0;
    let mut tv_scroll: usize = 0;
    let mut tv_tuned: Option<usize> = None;
    let mut tv_downloading = false;
    let mut tv_download_progress: f32 = 0.0;
    let mut tv_preview_tex: Option<TextureId> = None;
    let mut tv_error_msg = String::new();
    let mut tv_now_playing = String::new();

    // AV codec modules (AvCodec, AvMpegBase, AvMp3) are loaded lazily
    // by the audio thread on first play. Loading them here at startup
    // would conflict with the PRX overlay's sceAudiocodec if the PRX
    // initialized before the EBOOT was launched.

    // Background worker threads: audio, file I/O, and video decode.
    let (audio, io) = oasis_backend_psp::spawn_workers();
    oasis_backend_psp::video::spawn_video_thread();
    let mut pv_loading = false; // true while waiting for async texture load
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
            term_lines.push(String::from("[Power] Resumed from sleep"));
        }

        // -- Poll async I/O responses --
        while let Some(resp) = io.try_recv() {
            match resp {
                IoResponse::TextureReady {
                    path: _,
                    width,
                    height,
                    rgba,
                } => {
                    if pv_loading {
                        if let Some(old) = pv_tex.take() {
                            backend.destroy_texture_inner(old);
                        }
                        pv_tex = backend.load_texture_inner(width, height, &rgba);
                        pv_img_w = width;
                        pv_img_h = height;
                        pv_viewing = true;
                        pv_loading = false;
                    }
                },
                IoResponse::Error { path, msg } => {
                    term_lines.push(format!("I/O error: {} - {}", path, msg));
                    pv_loading = false;
                    if br_loading {
                        br_loading = false;
                        br_status_msg = format!("Error: {}", msg);
                    }
                },
                IoResponse::FileReady { .. } => {},
                IoResponse::HttpDone {
                    tag,
                    status_code,
                    body,
                } => {
                    if tag == 0xBEEF {
                        // Browser response.
                        let html = String::from_utf8_lossy(&body);
                        let text = views::strip_html(&html);
                        br_content_lines = views::wrap_text(&text, 58);
                        br_scroll = 0;
                        br_loading = false;
                        br_status_msg = format!("HTTP {} - {} bytes", status_code, body.len(),);
                    } else if (tag & 0xFF00) == 0xAA00 {
                        // TV Guide catalog response.
                        let ch_idx = (tag & 0xFF) as usize;
                        let src_idx = ((tag >> 16) & 0xF) as usize;
                        if ch_idx < tv_channels.len() && status_code >= 200 && status_code < 300 {
                            let json = String::from_utf8_lossy(&body);
                            let ch = &tv_channels[ch_idx];
                            let subfolder = ch
                                .source
                                .get(src_idx)
                                .and_then(|s| s.subfolder.as_deref());
                            let item_id = ch
                                .source
                                .get(src_idx)
                                .map(|s| s.item_id.as_str())
                                .unwrap_or("");
                            let episodes =
                                oasis_core::apps::tv_guide::ChannelCatalog
                                    ::parse_files_response(&json, item_id, subfolder);
                            if !episodes.is_empty() {
                                let catalog = tv_catalogs[ch_idx]
                                    .get_or_insert_with(|| {
                                        oasis_core::apps::tv_guide::ChannelCatalog
                                            ::new(ch.number)
                                    });
                                catalog.add_episodes(episodes);
                            }
                        }
                    } else {
                        let preview = String::from_utf8_lossy(&body[..body.len().min(256)]);
                        term_lines.push(format!(
                            "HTTP {status_code} ({} bytes): {preview}",
                            body.len(),
                        ));
                    }
                },
                IoResponse::RadioConnected {
                    fd,
                    icy_metaint,
                    initial_data,
                } => {
                    radio_status = RadioStatus::Buffering;
                    audio.send(AudioCmd::RadioStreamFromFd {
                        fd,
                        icy_metaint,
                        initial_data,
                    });
                },
                IoResponse::RadioError { msg } => {
                    radio_status = RadioStatus::Error;
                    radio_error_msg = msg;
                },
                IoResponse::VideoProgress { tag: _, bytes, total } => {
                    if let Some(t) = total {
                        if t > 0 {
                            tv_download_progress = bytes as f32 / t as f32;
                        }
                    }
                },
                IoResponse::VideoReady { tag: _, path } => {
                    tv_downloading = false;
                    tv_download_progress = 1.0;
                    // Start video decode thread.
                    oasis_backend_psp::video::send_video_cmd(
                        oasis_backend_psp::video::VideoCmd::Play {
                            path,
                            seek_secs: 0,
                        },
                    );
                },
                IoResponse::VideoError { tag: _, msg } => {
                    tv_downloading = false;
                    tv_error_msg = format!("Download: {msg}");
                    tv_tuned = None;
                },
            }
        }


        // Poll radio streaming state from audio thread atomics.
        if radio_status == RadioStatus::Buffering || radio_status == RadioStatus::Playing {
            if !audio.is_radio_streaming() {
                radio_status = RadioStatus::Stopped;
                radio_now_playing.clear();
            } else if audio.is_radio_buffering() {
                radio_status = RadioStatus::Buffering;
            } else {
                radio_status = RadioStatus::Playing;
            }
            if let Some(meta) = audio.poll_radio_meta() {
                radio_now_playing = meta;
            }
        }

        let events = backend.poll_events_inner();

        for event in &events {
            // -- Desktop mode: bridge analog stick + Confirm to pointer events --
            if app_mode == AppMode::Desktop {
                match event {
                    InputEvent::ButtonPress(Button::Confirm) => {
                        _confirm_held = true;
                        let (cx, cy) = backend.cursor_pos();
                        let ptr_event = InputEvent::PointerClick { x: cx, y: cy };
                        let wm_event = wm.handle_input(&ptr_event, &mut sdi);
                        desktop::handle_wm_event(
                            &wm_event,
                            &mut term_lines,
                            &mut classic_view,
                            &mut app_mode,
                            &mut wm,
                            &mut sdi,
                            dashboard.page,
                        );
                    },
                    InputEvent::ButtonRelease(Button::Confirm) => {
                        _confirm_held = false;
                        let (cx, cy) = backend.cursor_pos();
                        let ptr_event = InputEvent::PointerRelease { x: cx, y: cy };
                        wm.handle_input(&ptr_event, &mut sdi);
                    },
                    InputEvent::CursorMove { x, y } => {
                        // Always forward cursor moves when in Desktop mode.
                        let move_event = InputEvent::CursorMove { x: *x, y: *y };
                        wm.handle_input(&move_event, &mut sdi);
                    },
                    InputEvent::ButtonPress(Button::Select) => {
                        // Toggle back to Classic mode.
                        app_mode = AppMode::Classic;
                        classic_view = ClassicView::Dashboard;
                    },
                    InputEvent::ButtonPress(Button::Triangle) => {
                        // Open app launcher: open selected app as window.
                        if let Some(app) = dashboard.selected_app() {
                            let title = app.title.clone();
                            if let Some(psp_app) = APPS.iter().find(|a| a.title == title.as_str()) {
                                desktop::open_app_window(
                                    &mut wm, &mut sdi, psp_app.id, psp_app.title,
                                );
                            }
                        }
                    },
                    InputEvent::ButtonPress(Button::Start) => {
                        desktop::open_app_window(&mut wm, &mut sdi, "terminal", "Terminal");
                    },
                    // Dashboard navigation works in Desktop mode too.
                    InputEvent::ButtonPress(
                        btn @ (Button::Up | Button::Down | Button::Left | Button::Right),
                    ) => {
                        let old_sel = dashboard.selected;
                        dashboard.handle_input(btn);
                        if dashboard.selected != old_sel {
                            audio.send(AudioCmd::PlaySfx(SfxId::Click));
                        }
                    },
                    InputEvent::TriggerPress(Trigger::Left) => {
                        // Both triggers held = close all windows.
                        if backend.is_button_held(CtrlButtons::RTRIGGER) {
                            wm.close_all(&mut sdi);
                        } else {
                            // Cycle window focus backward (send top to bottom).
                            wm.cycle_focus(false, &mut sdi);
                            audio.send(AudioCmd::PlaySfx(SfxId::Click));
                        }
                    },
                    InputEvent::TriggerPress(Trigger::Right) => {
                        // Both triggers held = close all windows.
                        if backend.is_button_held(CtrlButtons::LTRIGGER) {
                            wm.close_all(&mut sdi);
                        } else {
                            // Cycle window focus forward (bring bottom to top).
                            wm.cycle_focus(true, &mut sdi);
                            audio.send(AudioCmd::PlaySfx(SfxId::Click));
                        }
                    },
                    InputEvent::Quit => return,
                    _ => {},
                }
                continue; // Skip classic input handling.
            }

            // -- Classic mode input --
            match event {
                InputEvent::Quit => return,

                InputEvent::ButtonPress(Button::Start) => {
                    if classic_view == ClassicView::FileManager && umd_activated {
                        // SAFETY: deactivate UMD drive on exit.
                        unsafe {
                            psp::sys::sceUmdDeactivate(1, b"disc0:\0".as_ptr());
                        }
                        umd_activated = false;
                    }
                    classic_view = match classic_view {
                        ClassicView::Dashboard => ClassicView::Terminal,
                        ClassicView::Terminal => ClassicView::Dashboard,
                        ClassicView::FileManager => ClassicView::Dashboard,
                        ClassicView::PhotoViewer => ClassicView::Dashboard,
                        ClassicView::MusicPlayer => ClassicView::Dashboard,
                        ClassicView::Browser => ClassicView::Dashboard,
                        ClassicView::Radio => ClassicView::Dashboard,
                        ClassicView::TvGuide => ClassicView::Dashboard,
                    };
                },

                InputEvent::ButtonPress(Button::Select)
                    if classic_view == ClassicView::Dashboard =>
                {
                    // Toggle to Desktop mode.
                    app_mode = AppMode::Desktop;
                },

                // -- Dashboard input (via DashboardState) --
                InputEvent::ButtonPress(btn @ (Button::Up | Button::Down | Button::Left | Button::Right))
                    if classic_view == ClassicView::Dashboard =>
                {
                    let old_sel = dashboard.selected;
                    dashboard.handle_input(btn);
                    if dashboard.selected != old_sel {
                        audio.send(AudioCmd::PlaySfx(SfxId::Click));
                    }
                },
                InputEvent::ButtonPress(Button::Confirm)
                    if classic_view == ClassicView::Dashboard =>
                {
                    audio.send(AudioCmd::PlaySfx(SfxId::Navigate));
                    dashboard.trigger_press_flash();
                    let app_title = dashboard.selected_app().map(|a| a.title.clone());
                    if let Some(ref title) = app_title {
                        match title.as_str() {
                            "Terminal" => {
                                classic_view = ClassicView::Terminal;
                            },
                            "File Manager" => {
                                classic_view = ClassicView::FileManager;
                                fm_path = String::from("ms0:/");
                                fm_loaded = false;
                                fm2_path = fm_path.clone();
                                fm2_loaded = false;
                                fm_active_panel = 0;
                            },
                            "Photo Viewer" => {
                                classic_view = ClassicView::PhotoViewer;
                                pv_viewing = false;
                                pv_loaded = false;
                            },
                            "Music Player" => {
                                classic_view = ClassicView::MusicPlayer;
                                mp_loaded = false;
                            },
                            "Browser" => {
                                classic_view = ClassicView::Browser;
                                br_content_lines.clear();
                                br_scroll = 0;
                                br_loading = false;
                                br_status_msg = String::from("Press [] to enter URL");
                            },
                            "Radio" => {
                                classic_view = ClassicView::Radio;
                                radio_selected = 0;
                                radio_scroll = 0;
                            },
                            "TV Guide" => {
                                classic_view = ClassicView::TvGuide;
                                if tv_channels.is_empty() {
                                    if let Ok(config) =
                                        oasis_core::apps::tv_guide::ChannelConfig::from_toml(
                                            oasis_core::apps::tv_guide::channel
                                                ::DEFAULT_CHANNELS_TOML,
                                        )
                                    {
                                        tv_channels = config.channel;
                                        tv_catalogs = vec![None; tv_channels.len()];
                                        for (i, ch) in tv_channels.iter().enumerate() {
                                            for (si, src) in ch.source.iter().enumerate() {
                                                let api_path =
                                                    oasis_core::apps::tv_guide::ChannelCatalog
                                                        ::files_api_path(&src.item_id);
                                                let url = format!(
                                                    "https://archive.org{}",
                                                    api_path,
                                                );
                                                let tag = 0xAA00
                                                    | (i as u32 & 0xFF)
                                                    | ((si as u32 & 0xF) << 16);
                                                io.send(IoCmd::HttpGet { url, tag });
                                            }
                                        }
                                    }
                                }
                                tv_selected = 0;
                                tv_scroll = 0;
                            },
                            _ => {
                                // Apps without a Classic view: open in Desktop mode.
                                if let Some(app) = APPS.iter().find(|a| a.title == title.as_str()) {
                                    app_mode = AppMode::Desktop;
                                    desktop::open_app_window(
                                        &mut wm, &mut sdi, app.id, app.title,
                                    );
                                }
                            },
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Cancel)
                    if classic_view == ClassicView::Dashboard =>
                {
                    icons_hidden = !icons_hidden;
                },

                // Trigger cycling through open windows (z-order).
                InputEvent::TriggerPress(Trigger::Left)
                    if classic_view == ClassicView::Dashboard =>
                {
                    wm.cycle_focus(false, &mut sdi);
                    audio.send(AudioCmd::PlaySfx(SfxId::Click));
                },
                InputEvent::TriggerPress(Trigger::Right)
                    if classic_view == ClassicView::Dashboard =>
                {
                    wm.cycle_focus(true, &mut sdi);
                    audio.send(AudioCmd::PlaySfx(SfxId::Click));
                },

                // -- Terminal input --
                InputEvent::ButtonPress(Button::Confirm)
                    if classic_view == ClassicView::Terminal =>
                {
                    let cmd = term_input.clone();
                    term_lines.push(format!("> {}", cmd));
                    // Handle commands that need main-loop state first;
                    // fall through to execute_command for everything else.
                    let (output, used_dialog) = match cmd.trim() {
                        "sfx click" => {
                            audio.send(AudioCmd::PlaySfx(SfxId::Click));
                            (vec!["SFX: click".into()], false)
                        },
                        "sfx nav" => {
                            audio.send(AudioCmd::PlaySfx(SfxId::Navigate));
                            (vec!["SFX: navigate".into()], false)
                        },
                        "sfx error" => {
                            audio.send(AudioCmd::PlaySfx(SfxId::Error));
                            (vec!["SFX: error".into()], false)
                        },
                        "save" => match commands::save_terminal_history(&term_lines) {
                            Ok(()) => (vec!["State saved.".into()], true),
                            Err(e) => (vec![format!("Save failed: {e}")], true),
                        },
                        "load" => match commands::load_terminal_history() {
                            Ok(lines) => {
                                term_lines.clear();
                                term_lines.extend(lines);
                                (vec!["State restored.".into()], true)
                            },
                            Err(e) => (vec![format!("Load failed: {e}")], true),
                        },
                        "usb mount" => {
                            if usb_storage.is_some() {
                                (vec!["USB storage already active.".into()], false)
                            } else {
                                match psp::usb::start_bus() {
                                    Ok(()) => match psp::usb::UsbStorageMode::activate() {
                                        Ok(handle) => {
                                            usb_storage = Some(handle);
                                            (
                                                vec![
                                                    "USB storage mode active. Connect cable to PC."
                                                        .into(),
                                                ],
                                                false,
                                            )
                                        },
                                        Err(e) => {
                                            (vec![format!("USB activate failed: {e}")], false)
                                        },
                                    },
                                    Err(e) => (vec![format!("USB bus start failed: {e}")], false),
                                }
                            }
                        },
                        "usb unmount" | "usb eject" => {
                            if usb_storage.take().is_some() {
                                (vec!["USB storage mode deactivated.".into()], false)
                            } else {
                                (vec!["USB storage not active.".into()], false)
                            }
                        },
                        "usb" | "usb status" => {
                            let connected = psp::usb::is_connected();
                            let established = psp::usb::is_established();
                            let active = usb_storage.is_some();
                            (
                                vec![
                                    format!(
                                        "USB cable: {}",
                                        if connected {
                                            "connected"
                                        } else {
                                            "disconnected"
                                        }
                                    ),
                                    format!(
                                        "Storage mode: {}",
                                        if active { "ACTIVE" } else { "inactive" }
                                    ),
                                    format!(
                                        "Host mounted: {}",
                                        if established { "yes" } else { "no" },
                                    ),
                                ],
                                false,
                            )
                        },
                        _ if cmd.trim().starts_with("play ") => {
                            let path = cmd.trim().strip_prefix("play ").unwrap().trim();
                            audio.send(AudioCmd::LoadAndPlay(path.to_string()));
                            mp_file_name = path.to_string();
                            (vec![format!("Playing: {}", path)], false)
                        },
                        "pause" => {
                            audio.send(AudioCmd::Pause);
                            (vec!["Paused.".into()], false)
                        },
                        "resume" => {
                            audio.send(AudioCmd::Resume);
                            (vec!["Resumed.".into()], false)
                        },
                        "stop" => {
                            audio.send(AudioCmd::Stop);
                            (vec!["Stopped.".into()], false)
                        },
                        _ => {
                            let r = commands::execute_command(&cmd, &mut config);
                            (r.lines, r.used_dialog)
                        },
                    };
                    // Only reinit GU when a dialog was shown (e.g. `rm`,
                    // `save`, `load`). Calling reinit unconditionally
                    // freezes real PSP hardware because sceGuStart is
                    // issued on an already-open display list.
                    if used_dialog {
                        backend.reinit_gu_frame();
                    }
                    for line in output {
                        term_lines.push(line);
                    }
                    term_input.clear();
                    term_scroll = 0; // Auto-scroll to bottom on new output.
                    while term_lines.len() > 200 {
                        term_lines.remove(0);
                    }
                },
                InputEvent::ButtonPress(Button::Square)
                    if classic_view == ClassicView::Terminal =>
                {
                    // Open PSP on-screen keyboard for command input.
                    match psp::osk::OskBuilder::new("Enter command")
                        .max_chars(256)
                        .initial_text(&term_input)
                        .show()
                    {
                        Ok(Some(text)) => {
                            term_input = text;
                        },
                        Ok(None) | Err(_) => {}, // Cancelled or unsupported (PPSSPP)
                    }
                    // OSK closes the GU display list. Re-open for rendering.
                    backend.reinit_gu_frame();
                },
                InputEvent::ButtonPress(Button::Up) if classic_view == ClassicView::Terminal => {
                    // Scroll up through terminal history.
                    let max_scroll = term_lines.len().saturating_sub(MAX_OUTPUT_LINES);
                    if term_scroll < max_scroll {
                        term_scroll += 3;
                        if term_scroll > max_scroll {
                            term_scroll = max_scroll;
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Down) if classic_view == ClassicView::Terminal => {
                    // Scroll down (towards latest output).
                    term_scroll = term_scroll.saturating_sub(3);
                },

                // -- File manager input (dual-panel) --
                InputEvent::ButtonPress(Button::Left)
                    if classic_view == ClassicView::FileManager =>
                {
                    fm_active_panel = 0;
                    audio.send(AudioCmd::PlaySfx(SfxId::Click));
                },
                InputEvent::ButtonPress(Button::Right)
                    if classic_view == ClassicView::FileManager =>
                {
                    fm_active_panel = 1;
                    audio.send(AudioCmd::PlaySfx(SfxId::Click));
                },
                InputEvent::ButtonPress(Button::Up) if classic_view == ClassicView::FileManager => {
                    let (sel, scr) = if fm_active_panel == 0 {
                        (&mut fm_selected, &mut fm_scroll)
                    } else {
                        (&mut fm2_selected, &mut fm2_scroll)
                    };
                    if *sel > 0 {
                        *sel -= 1;
                        if *sel < *scr {
                            *scr = *sel;
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Down)
                    if classic_view == ClassicView::FileManager =>
                {
                    let (sel, scr, entries) = if fm_active_panel == 0 {
                        (&mut fm_selected, &mut fm_scroll, &fm_entries)
                    } else {
                        (&mut fm2_selected, &mut fm2_scroll, &fm2_entries)
                    };
                    if *sel + 1 < entries.len() {
                        *sel += 1;
                        if *sel >= *scr + FM_VISIBLE_ROWS {
                            *scr = *sel - FM_VISIBLE_ROWS + 1;
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Confirm)
                    if classic_view == ClassicView::FileManager =>
                {
                    let (path, entries, sel, loaded) = if fm_active_panel == 0 {
                        (&mut fm_path, &fm_entries, fm_selected, &mut fm_loaded)
                    } else {
                        (&mut fm2_path, &fm2_entries, fm2_selected, &mut fm2_loaded)
                    };
                    if sel < entries.len() && entries[sel].is_dir {
                        let dir_name = entries[sel].name.clone();
                        if path.ends_with('/') {
                            *path = format!("{}{}", path, dir_name);
                        } else {
                            *path = format!("{}/{}", path, dir_name);
                        }
                        *loaded = false;
                    }
                },
                InputEvent::ButtonPress(Button::Cancel)
                    if classic_view == ClassicView::FileManager =>
                {
                    let (path, loaded) = if fm_active_panel == 0 {
                        (&mut fm_path, &mut fm_loaded)
                    } else {
                        (&mut fm2_path, &mut fm2_loaded)
                    };
                    if let Some(pos) = path.rfind('/') {
                        if pos > 0 && !path[..pos].ends_with(':') {
                            path.truncate(pos);
                        } else if path.len() > pos + 1 {
                            path.truncate(pos + 1);
                        } else {
                            if umd_activated {
                                // SAFETY: deactivate UMD drive on exit.
                                unsafe {
                                    psp::sys::sceUmdDeactivate(1, b"disc0:\0".as_ptr());
                                }
                                umd_activated = false;
                            }
                            classic_view = ClassicView::Dashboard;
                        }
                        *loaded = false;
                    } else {
                        if umd_activated {
                            // SAFETY: deactivate UMD drive on exit.
                            unsafe {
                                psp::sys::sceUmdDeactivate(1, b"disc0:\0".as_ptr());
                            }
                            umd_activated = false;
                        }
                        classic_view = ClassicView::Dashboard;
                    }
                },
                InputEvent::ButtonPress(Button::Square)
                    if classic_view == ClassicView::FileManager =>
                {
                    let (path, entries, sel, loaded) = if fm_active_panel == 0 {
                        (&fm_path, &fm_entries, fm_selected, &mut fm_loaded)
                    } else {
                        (&fm2_path, &fm2_entries, fm2_selected, &mut fm2_loaded)
                    };
                    // UMD is read-only, skip delete.
                    if path.starts_with("disc0:") {
                        term_lines.push("UMD is read-only.".into());
                    } else if sel < entries.len() && !entries[sel].is_dir {
                        let name = &entries[sel].name;
                        let msg = format!("Delete {}?", name);
                        match psp::dialog::confirm_dialog(&msg) {
                            Ok(psp::dialog::DialogResult::Confirm) => {
                                let full_path = if path.ends_with('/') {
                                    format!("{}{}", path, name)
                                } else {
                                    format!("{}/{}", path, name)
                                };
                                match psp::io::remove_file(&full_path) {
                                    Ok(()) => {
                                        term_lines.push(format!("Deleted: {}", full_path));
                                        *loaded = false;
                                    },
                                    Err(e) => {
                                        let _ = psp::dialog::error_dialog(e.0 as u32);
                                    },
                                }
                            },
                            _ => {}, // Cancelled or closed
                        }
                        // confirm_dialog/error_dialog close the GU list.
                        backend.reinit_gu_frame();
                    }
                },
                InputEvent::ButtonPress(Button::Triangle)
                    if classic_view == ClassicView::FileManager =>
                {
                    if umd_activated {
                        // SAFETY: deactivate UMD drive on exit.
                        unsafe {
                            psp::sys::sceUmdDeactivate(1, b"disc0:\0".as_ptr());
                        }
                        umd_activated = false;
                    }
                    classic_view = ClassicView::Dashboard;
                },

                // -- Photo viewer input --
                InputEvent::ButtonPress(Button::Up)
                    if classic_view == ClassicView::PhotoViewer && !pv_viewing =>
                {
                    if pv_selected > 0 {
                        pv_selected -= 1;
                        if pv_selected < pv_scroll {
                            pv_scroll = pv_selected;
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Down)
                    if classic_view == ClassicView::PhotoViewer && !pv_viewing =>
                {
                    if pv_selected + 1 < pv_entries.len() {
                        pv_selected += 1;
                        if pv_selected >= pv_scroll + FM_VISIBLE_ROWS {
                            pv_scroll = pv_selected - FM_VISIBLE_ROWS + 1;
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Confirm)
                    if classic_view == ClassicView::PhotoViewer && !pv_viewing =>
                {
                    if pv_selected < pv_entries.len() {
                        let entry = &pv_entries[pv_selected];
                        if entry.is_dir {
                            let dir_name = entry.name.clone();
                            if pv_path.ends_with('/') {
                                pv_path = format!("{}{}", pv_path, dir_name);
                            } else {
                                pv_path = format!("{}/{}", pv_path, dir_name);
                            }
                            pv_loaded = false;
                        } else {
                            // Async JPEG decode via background I/O thread.
                            let file_path = if pv_path.ends_with('/') {
                                format!("{}{}", pv_path, entry.name)
                            } else {
                                format!("{}/{}", pv_path, entry.name)
                            };
                            io.send(IoCmd::LoadTexture {
                                path: file_path,
                                max_w: SCREEN_WIDTH as i32,
                                max_h: SCREEN_HEIGHT as i32,
                            });
                            pv_loading = true;
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Cancel)
                    if classic_view == ClassicView::PhotoViewer =>
                {
                    if pv_viewing {
                        pv_viewing = false;
                    } else if let Some(pos) = pv_path.rfind('/') {
                        if pos > 0 && !pv_path[..pos].ends_with(':') {
                            pv_path.truncate(pos);
                        } else if pv_path.len() > pos + 1 {
                            pv_path.truncate(pos + 1);
                        } else {
                            classic_view = ClassicView::Dashboard;
                        }
                        pv_loaded = false;
                    } else {
                        classic_view = ClassicView::Dashboard;
                    }
                },
                InputEvent::ButtonPress(Button::Triangle)
                    if classic_view == ClassicView::PhotoViewer =>
                {
                    if pv_viewing {
                        pv_viewing = false;
                    } else {
                        classic_view = ClassicView::Dashboard;
                    }
                },

                // -- Music player input --
                InputEvent::ButtonPress(Button::Up)
                    if classic_view == ClassicView::MusicPlayer && !audio.is_playing() =>
                {
                    if mp_selected > 0 {
                        mp_selected -= 1;
                        if mp_selected < mp_scroll {
                            mp_scroll = mp_selected;
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Down)
                    if classic_view == ClassicView::MusicPlayer && !audio.is_playing() =>
                {
                    if mp_selected + 1 < mp_entries.len() {
                        mp_selected += 1;
                        if mp_selected >= mp_scroll + FM_VISIBLE_ROWS {
                            mp_scroll = mp_selected - FM_VISIBLE_ROWS + 1;
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Confirm)
                    if classic_view == ClassicView::MusicPlayer =>
                {
                    if audio.is_playing() {
                        // Toggle pause via background thread.
                        if audio.is_paused() {
                            audio.send(AudioCmd::Resume);
                        } else {
                            audio.send(AudioCmd::Pause);
                        }
                    } else if mp_selected < mp_entries.len() {
                        let entry = &mp_entries[mp_selected];
                        if entry.is_dir {
                            let dir_name = entry.name.clone();
                            if mp_path.ends_with('/') {
                                mp_path = format!("{}{}", mp_path, dir_name);
                            } else {
                                mp_path = format!("{}/{}", mp_path, dir_name);
                            }
                            mp_loaded = false;
                        } else {
                            // Play MP3 via background thread.
                            let file_path = if mp_path.ends_with('/') {
                                format!("{}{}", mp_path, entry.name)
                            } else {
                                format!("{}/{}", mp_path, entry.name)
                            };
                            mp_file_name = entry.name.clone();
                            audio.send(AudioCmd::LoadAndPlay(file_path));
                            term_lines.push(format!("Playing: {}", entry.name));
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Square)
                    if classic_view == ClassicView::MusicPlayer =>
                {
                    audio.send(AudioCmd::Stop);
                },
                InputEvent::ButtonPress(Button::Cancel)
                    if classic_view == ClassicView::MusicPlayer =>
                {
                    audio.send(AudioCmd::Stop);
                    if let Some(pos) = mp_path.rfind('/') {
                        if pos > 0 && !mp_path[..pos].ends_with(':') {
                            mp_path.truncate(pos);
                        } else if mp_path.len() > pos + 1 {
                            mp_path.truncate(pos + 1);
                        } else {
                            classic_view = ClassicView::Dashboard;
                        }
                        mp_loaded = false;
                    } else {
                        classic_view = ClassicView::Dashboard;
                    }
                },
                InputEvent::ButtonPress(Button::Triangle)
                    if classic_view == ClassicView::MusicPlayer =>
                {
                    classic_view = ClassicView::Dashboard;
                    // Audio keeps playing in background.
                },

                // -- Browser input --
                InputEvent::ButtonPress(Button::Square) if classic_view == ClassicView::Browser => {
                    // Open PSP OSK for URL input.
                    match psp::osk::OskBuilder::new("Enter URL")
                        .max_chars(256)
                        .initial_text(&br_url)
                        .show()
                    {
                        Ok(Some(text)) => {
                            br_url = text;
                            br_status_msg = String::from("Press X to load");
                        },
                        Ok(None) | Err(_) => {},
                    }
                    backend.reinit_gu_frame();
                },
                InputEvent::ButtonPress(Button::Confirm)
                    if classic_view == ClassicView::Browser =>
                {
                    // Init network on main thread (WiFi dialog needs GU).
                    if !oasis_backend_psp::network::is_net_initialized() {
                        if let Err(e) = oasis_backend_psp::network::ensure_net_init_pub() {
                            br_status_msg = format!("Net error: {e}");
                            backend.reinit_gu_frame();
                            continue;
                        }
                        backend.reinit_gu_frame();
                    }
                    br_loading = true;
                    br_status_msg = String::from("Loading...");
                    br_content_lines.clear();
                    io.send(IoCmd::HttpGet {
                        url: br_url.clone(),
                        tag: 0xBEEF,
                    });
                },
                InputEvent::ButtonPress(Button::Up) if classic_view == ClassicView::Browser => {
                    br_scroll = br_scroll.saturating_sub(3);
                },
                InputEvent::ButtonPress(Button::Down) if classic_view == ClassicView::Browser => {
                    if br_scroll + 3 < br_content_lines.len() {
                        br_scroll += 3;
                    }
                },
                InputEvent::ButtonPress(Button::Triangle)
                    if classic_view == ClassicView::Browser =>
                {
                    classic_view = ClassicView::Dashboard;
                },
                InputEvent::ButtonPress(Button::Cancel) if classic_view == ClassicView::Browser => {
                    classic_view = ClassicView::Dashboard;
                },

                // -- Radio input --
                InputEvent::ButtonPress(Button::Up)
                    if classic_view == ClassicView::Radio
                        && radio_status == RadioStatus::Stopped =>
                {
                    if radio_selected > 0 {
                        radio_selected -= 1;
                        if radio_selected < radio_scroll {
                            radio_scroll = radio_selected;
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Down)
                    if classic_view == ClassicView::Radio
                        && radio_status == RadioStatus::Stopped =>
                {
                    if radio_selected + 1 < RADIO_STATIONS.len() {
                        radio_selected += 1;
                        if radio_selected >= radio_scroll + FM_VISIBLE_ROWS {
                            radio_scroll = radio_selected - FM_VISIBLE_ROWS + 1;
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Confirm) if classic_view == ClassicView::Radio => {
                    if radio_status == RadioStatus::Stopped || radio_status == RadioStatus::Error {
                        if radio_selected < RADIO_STATIONS.len() {
                            // Init network on main thread (WiFi dialog needs GU).
                            if !oasis_backend_psp::network::is_net_initialized() {
                                if let Err(e) = oasis_backend_psp::network::ensure_net_init_pub() {
                                    radio_error_msg = format!("Net error: {e}");
                                    radio_status = RadioStatus::Error;
                                    backend.reinit_gu_frame();
                                    continue;
                                }
                                backend.reinit_gu_frame();
                            }
                            let station = &RADIO_STATIONS[radio_selected];
                            radio_station_name = String::from(station.name);
                            radio_now_playing.clear();
                            radio_status = RadioStatus::Connecting;
                            io.send(IoCmd::RadioConnect {
                                url: String::from(station.url),
                            });
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Square) if classic_view == ClassicView::Radio => {
                    if radio_status != RadioStatus::Stopped {
                        audio.send(AudioCmd::RadioStop);
                        radio_status = RadioStatus::Stopped;
                        radio_now_playing.clear();
                    }
                },
                InputEvent::ButtonPress(Button::Triangle) if classic_view == ClassicView::Radio => {
                    // Back to dashboard (radio keeps playing).
                    classic_view = ClassicView::Dashboard;
                },
                InputEvent::ButtonPress(Button::Cancel) if classic_view == ClassicView::Radio => {
                    // Stop + back.
                    if radio_status != RadioStatus::Stopped {
                        audio.send(AudioCmd::RadioStop);
                        radio_status = RadioStatus::Stopped;
                        radio_now_playing.clear();
                    }
                    classic_view = ClassicView::Dashboard;
                },

                // -- TV Guide input --
                InputEvent::ButtonPress(Button::Up)
                    if classic_view == ClassicView::TvGuide && tv_tuned.is_none() =>
                {
                    if tv_selected > 0 {
                        tv_selected -= 1;
                        if tv_selected < tv_scroll {
                            tv_scroll = tv_selected;
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Down)
                    if classic_view == ClassicView::TvGuide && tv_tuned.is_none() =>
                {
                    if tv_selected + 1 < tv_channels.len() {
                        tv_selected += 1;
                        if tv_selected >= tv_scroll + FM_VISIBLE_ROWS {
                            tv_scroll = tv_selected - FM_VISIBLE_ROWS + 1;
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Confirm)
                    if classic_view == ClassicView::TvGuide =>
                {
                    if tv_tuned.is_none() && !tv_downloading {
                        // Tune to selected channel.
                        if tv_selected < tv_catalogs.len() {
                            if let Some(catalog) = &tv_catalogs[tv_selected] {
                                let best = oasis_core::apps::tv_guide::select_smallest_for(
                                    &catalog.episodes,
                                    20_000_000, // 20MB max
                                    320,        // min width
                                );
                                if let Some(ep) = best {
                                    // Init network.
                                    if !oasis_backend_psp::network::is_net_initialized() {
                                        if let Err(e) =
                                            oasis_backend_psp::network::ensure_net_init_pub()
                                        {
                                            tv_error_msg = format!("Net: {e}");
                                            backend.reinit_gu_frame();
                                            continue;
                                        }
                                        backend.reinit_gu_frame();
                                    }
                                    let url =
                                        oasis_core::apps::tv_guide::ChannelCatalog::download_url(
                                            ep,
                                        );
                                    tv_now_playing = ep.title.clone();
                                    tv_downloading = true;
                                    tv_download_progress = 0.0;
                                    tv_error_msg.clear();
                                    tv_tuned = Some(tv_selected);
                                    io.send(IoCmd::VideoDownload {
                                        url,
                                        dest: String::from(
                                            "ms0:/PSP/GAME/OASISOS/tv_cache.mp4",
                                        ),
                                        tag: 0xBB00,
                                    });
                                } else {
                                    tv_error_msg = String::from("No suitable video found");
                                }
                            } else {
                                tv_error_msg = String::from("Channel catalog not loaded");
                            }
                        }
                    }
                },
                InputEvent::ButtonPress(Button::Cancel)
                    if classic_view == ClassicView::TvGuide =>
                {
                    if tv_tuned.is_some() {
                        // Untune: stop video + audio.
                        oasis_backend_psp::video::send_video_cmd(
                            oasis_backend_psp::video::VideoCmd::Stop,
                        );
                        audio.send(AudioCmd::VideoAudioStop);
                        if let Some(old) = tv_preview_tex.take() {
                            backend.destroy_texture_inner(old);
                        }
                        tv_tuned = None;
                        tv_downloading = false;
                        tv_now_playing.clear();
                        tv_error_msg.clear();
                    } else {
                        classic_view = ClassicView::Dashboard;
                    }
                },
                InputEvent::ButtonPress(Button::Triangle)
                    if classic_view == ClassicView::TvGuide =>
                {
                    // Back to dashboard (keep video playing in background).
                    classic_view = ClassicView::Dashboard;
                },

                _ => {},
            }
        }

        // -- Poll video decode frames --
        if tv_tuned.is_some() && !tv_downloading {
            if let Some(frame) = oasis_backend_psp::video::poll_video_frame() {
                if let Some(old) = tv_preview_tex.take() {
                    backend.destroy_texture_inner(old);
                }
                tv_preview_tex = backend.load_texture_inner(frame.width, frame.height, &frame.rgba);
            }
            // Check if video playback ended.
            if !oasis_backend_psp::video::is_video_playing() {
                if let Some(old) = tv_preview_tex.take() {
                    backend.destroy_texture_inner(old);
                }
                tv_tuned = None;
                tv_now_playing.clear();
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
        bottom_bar.current_page = dashboard.page;
        bottom_bar.total_pages = dashboard.page_count();
        bottom_bar.tick_animation(&active_theme);

        backend.clear_inner(Color::BLACK);
        // Wallpaper: 64x64 texture scaled to fullscreen by GE (bilinear).
        backend.blit_scaled(wallpaper_tex, 0, 0, SCREEN_WIDTH, SCREEN_HEIGHT);

        match app_mode {
            AppMode::Classic => {
                // Lazy-load directory entries for browser modes.
                if classic_view == ClassicView::FileManager && !fm_loaded {
                    fm_entries = oasis_backend_psp::list_directory(&fm_path);
                    fm_selected = 0;
                    fm_scroll = 0;
                    fm_loaded = true;
                }
                if classic_view == ClassicView::FileManager && !fm2_loaded {
                    fm2_entries = oasis_backend_psp::list_directory(&fm2_path);
                    fm2_selected = 0;
                    fm2_scroll = 0;
                    fm2_loaded = true;
                }
                if classic_view == ClassicView::PhotoViewer && !pv_loaded && !pv_viewing {
                    let all = oasis_backend_psp::list_directory(&pv_path);
                    pv_entries = all
                        .into_iter()
                        .filter(|e| {
                            e.is_dir || {
                                let lower: String =
                                    e.name.chars().map(|c| c.to_ascii_lowercase()).collect();
                                lower.ends_with(".jpg") || lower.ends_with(".jpeg")
                            }
                        })
                        .collect();
                    pv_selected = 0;
                    pv_scroll = 0;
                    pv_loaded = true;
                }
                if classic_view == ClassicView::MusicPlayer && !mp_loaded && !audio.is_playing() {
                    let all = oasis_backend_psp::list_directory(&mp_path);
                    mp_entries = all
                        .into_iter()
                        .filter(|e| {
                            e.is_dir || {
                                let lower: String =
                                    e.name.chars().map(|c| c.to_ascii_lowercase()).collect();
                                lower.ends_with(".mp3")
                            }
                        })
                        .collect();
                    mp_selected = 0;
                    mp_scroll = 0;
                    mp_loaded = true;
                }

                // Show or hide dashboard icons based on current view.
                let show_dashboard = classic_view == ClassicView::Dashboard
                    && !icons_hidden;
                if show_dashboard {
                    dashboard.tick_animation();
                    dashboard.update_sdi(&mut sdi, &active_theme);
                } else {
                    dashboard.hide_sdi(&mut sdi);
                }

                // Show or hide terminal SDI objects.
                if classic_view != ClassicView::Terminal {
                    terminal_sdi::set_terminal_visible(&mut sdi, false);
                }

                match classic_view {
                    ClassicView::Dashboard => {
                        backend.force_bitmap_font = true;
                        chrome::draw_button_hints(
                            &mut backend,
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
                        // SDI-based terminal rendering (Phase 4).
                        terminal_sdi::setup_terminal_objects(
                            &mut sdi,
                            &term_lines,
                            "/",
                            &term_input,
                            term_scroll,
                            &active_theme,
                            viz_frame % 30 < 15, // blinking cursor
                        );
                        backend.force_bitmap_font = true;
                        chrome::draw_button_hints(
                            &mut backend,
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
                        backend.force_bitmap_font = true;
                        views::draw_file_manager_dual(
                            &mut backend,
                            &fm_path,
                            &fm_entries,
                            fm_selected,
                            fm_scroll,
                            &fm2_path,
                            &fm2_entries,
                            fm2_selected,
                            fm2_scroll,
                            fm_active_panel,
                        );
                        chrome::draw_button_hints(
                            &mut backend,
                            &[("X", "Open"), ("O", "Back"), ("<>", "Panel"), ("^v", "Nav")],
                        );
                        backend.force_bitmap_font = false;
                    },
                    ClassicView::PhotoViewer => {
                        backend.force_bitmap_font = true;
                        if pv_viewing {
                            views::draw_photo_view(&mut backend, pv_tex, pv_img_w, pv_img_h);
                            chrome::draw_button_hints(&mut backend, &[("O", "Back")]);
                        } else if pv_loading {
                            desktop::draw_loading_indicator(&mut backend, "Decoding image...");
                        } else {
                            views::draw_photo_browser(
                                &mut backend,
                                &pv_path,
                                &pv_entries,
                                pv_selected,
                                pv_scroll,
                            );
                            chrome::draw_button_hints(
                                &mut backend,
                                &[("X", "View"), ("O", "Back"), ("^v", "Nav")],
                            );
                        }
                        backend.force_bitmap_font = false;
                    },
                    ClassicView::MusicPlayer => {
                        backend.force_bitmap_font = true;
                        if audio.is_playing() {
                            views::draw_music_player_threaded(
                                &mut backend,
                                &mp_file_name,
                                &audio,
                                viz_frame,
                            );
                            chrome::draw_button_hints(
                                &mut backend,
                                &[("X", "Pause"), ("[]", "Stop"), ("^v", "Back")],
                            );
                        } else {
                            views::draw_music_browser(
                                &mut backend,
                                &mp_path,
                                &mp_entries,
                                mp_selected,
                                mp_scroll,
                            );
                            chrome::draw_button_hints(
                                &mut backend,
                                &[("X", "Play"), ("O", "Back"), ("^v", "Nav")],
                            );
                        }
                        backend.force_bitmap_font = false;
                    },
                    ClassicView::Browser => {
                        backend.force_bitmap_font = true;
                        if br_loading {
                            desktop::draw_loading_indicator(&mut backend, "Loading page...");
                        } else {
                            views::draw_browser_view(
                                &mut backend,
                                &br_url,
                                &br_content_lines,
                                br_scroll,
                                &br_status_msg,
                            );
                        }
                        chrome::draw_button_hints(
                            &mut backend,
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
                        match radio_status {
                            RadioStatus::Stopped => {
                                views::draw_radio_stations(&mut backend, radio_selected, radio_scroll);
                                chrome::draw_button_hints(
                                    &mut backend,
                                    &[("X", "Tune"), ("^v", "Nav"), ("O", "Back")],
                                );
                            },
                            RadioStatus::Connecting => {
                                desktop::draw_loading_indicator(&mut backend, "Connecting...");
                            },
                            RadioStatus::Buffering | RadioStatus::Playing => {
                                views::draw_radio_playing(
                                    &mut backend,
                                    &radio_station_name,
                                    &radio_now_playing,
                                    radio_status == RadioStatus::Buffering,
                                    &audio,
                                    viz_frame,
                                );
                                chrome::draw_button_hints(
                                    &mut backend,
                                    &[("[]", "Stop"), ("^", "Back"), ("O", "Stop+Back")],
                                );
                            },
                            RadioStatus::Error => {
                                views::draw_radio_error(&mut backend, &radio_error_msg);
                                chrome::draw_button_hints(&mut backend, &[("X", "Retry"), ("O", "Back")]);
                            },
                        }
                        backend.force_bitmap_font = false;
                    },
                    ClassicView::TvGuide => {
                        backend.force_bitmap_font = true;
                        if tv_tuned.is_some() {
                            views::draw_tv_playing(
                                &mut backend,
                                &tv_now_playing,
                                tv_downloading,
                                tv_download_progress,
                                tv_preview_tex,
                                &tv_error_msg,
                            );
                            chrome::draw_button_hints(
                                &mut backend,
                                &[("O", "Untune"), ("^", "Back")],
                            );
                        } else if !tv_error_msg.is_empty() {
                            views::draw_tv_error(&mut backend, &tv_error_msg);
                            chrome::draw_button_hints(
                                &mut backend,
                                &[("X", "Retry"), ("O", "Back")],
                            );
                        } else {
                            views::draw_tv_channels(
                                &mut backend,
                                &tv_channels,
                                &tv_catalogs,
                                tv_selected,
                                tv_scroll,
                            );
                            chrome::draw_button_hints(
                                &mut backend,
                                &[("X", "Tune"), ("^v", "Nav"), ("O", "Back")],
                            );
                        }
                        backend.force_bitmap_font = false;
                    },
                }
            },

            AppMode::Desktop => {
                // Show dashboard icons behind windows in Desktop mode.
                if !icons_hidden {
                    dashboard.tick_animation();
                    dashboard.update_sdi(&mut sdi, &active_theme);
                } else {
                    dashboard.hide_sdi(&mut sdi);
                }
                // Hide terminal SDI objects in Desktop mode.
                terminal_sdi::set_terminal_visible(&mut sdi, false);

                // Pre-compute values for windowed app renderers.
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

                // Draw WM chrome (frames, titlebars) + clipped content.
                // Use bitmap font for app content (8px vs 12px system font).
                backend.force_bitmap_font = true;
                let _ =
                    wm.draw_with_clips(&mut sdi, &mut backend, |window_id, cx, cy, cw, ch, be| {
                        // Downcast back to PspBackend for direct calls.
                        // Since draw_with_clips passes &mut dyn SdiBackend, we use
                        // the trait methods here (which return Result).
                        match window_id {
                            "terminal" => {
                                desktop::draw_terminal_windowed(&term_lines, &term_input, cx, cy, cw, ch, be)
                            },
                            "filemgr" => desktop::draw_filemgr_windowed(
                                &fm_path,
                                &fm_entries,
                                fm_selected,
                                fm_scroll,
                                &fm2_path,
                                &fm2_entries,
                                fm2_selected,
                                fm2_scroll,
                                fm_active_panel,
                                cx,
                                cy,
                                cw,
                                ch,
                                be,
                            ),
                            "photos" => desktop::draw_photos_windowed(
                                pv_tex, pv_img_w, pv_img_h, pv_viewing, cx, cy, cw, ch, be,
                            ),
                            "music" => {
                                desktop::draw_music_windowed(&mp_file_name, &audio, cx, cy, cw, ch, be)
                            },
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
                            "network" => desktop::draw_network_windowed(&status, cx, cy, cw, ch, be),
                            "sysmon" => desktop::draw_sysmon_windowed(
                                &status,
                                &sysinfo,
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
                            "radio" => desktop::draw_radio_windowed(&audio, cx, cy, cw, ch, be),
                            _ => Ok(()),
                        }
                    });

                backend.force_bitmap_font = false;
            },
        }

        // Status bar + bottom bar (always visible, drawn on top via SDI).
        // Update URL text based on current mode/view.
        active_theme.bar.url_text = match (app_mode, classic_view) {
            (AppMode::Desktop, _) => "SYS://DESKTOP".to_string(),
            (_, ClassicView::Dashboard) => "SYS://DASHBOARD".to_string(),
            (_, ClassicView::Terminal) => "SYS://TERMINAL".to_string(),
            (_, ClassicView::FileManager) => {
                let active_path = if fm_active_panel == 0 {
                    &fm_path
                } else {
                    &fm2_path
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
                if tv_tuned.is_some() {
                    "SYS://TV_LIVE".to_string()
                } else {
                    "SYS://TV_GUIDE".to_string()
                }
            },
        };
        status_bar.update_sdi(&mut sdi, &active_theme, &skin_features);
        bottom_bar.update_sdi(&mut sdi, &active_theme, &skin_features);

        // On PSP, draw SDI in two passes to control cost:
        // - Base layer only when dashboard/terminal are active (icons, term lines)
        // - Overlay layer always (status bar, bottom bar at z=900)
        // This avoids 100+ unnecessary draw calls in non-dashboard views.
        let needs_base = match app_mode {
            AppMode::Classic => matches!(
                classic_view,
                ClassicView::Dashboard | ClassicView::Terminal
            ),
            AppMode::Desktop => false, // WM draws windows directly
        };
        if needs_base {
            let _ = sdi.draw_base_layer(&mut backend);
        }
        let _ = sdi.draw_overlay_layer(&mut backend);

        // Post-SDI overlays drawn directly on the backend.
        if app_mode == AppMode::Classic {
            match classic_view {
                ClassicView::Dashboard if !icons_hidden
                    && active_theme.icon.style == "vector" =>
                {
                    // Vector icons overlay.
                    let _ = oasis_core::vector_overlay::render_vector_background(
                        &mut backend,
                        &active_theme,
                        viz_frame,
                    );
                    let _ = dashboard.render_vector_icons(
                        &mut backend,
                        &active_theme,
                        viz_frame,
                    );
                },
                ClassicView::Terminal => {
                    // Terminal scrollbar (painted directly after SDI draw).
                    let _ = terminal_sdi::paint_terminal_scrollbar(
                        &mut backend,
                        term_lines.len(),
                        term_scroll,
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

// Remaining extracted modules: desktop, chrome, views, boot, theme, types.
// Command interpreter and utilities are in commands.rs module.
