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
    AudioCmd, AudioHandle, Button, CURSOR_H, CURSOR_W, Color, FileEntry, InputEvent, IoCmd,
    IoResponse, PspBackend, SCREEN_HEIGHT, SCREEN_WIDTH, SdiBackend, SdiRegistry, SfxId,
    StatusBarInfo, SystemInfo, TextureId, Trigger, WindowConfig, WindowManager, WindowType,
    WmEvent,
};

mod commands;

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
        // SAFETY: MT19937 context is stack-local, seed from CPU cycle counter.
        unsafe {
            let mut ctx = core::mem::zeroed();
            let seed: u32;
            core::arch::asm!("mfc0 {}, $9", out(reg) seed);
            sceKernelUtilsMt19937Init(&mut ctx, seed);
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
    // SAFETY: MT19937 context is stack-local, seed from CPU cycle counter.
    unsafe {
        let mut ctx = core::mem::zeroed();
        let seed: u32;
        core::arch::asm!("mfc0 {}, $9", out(reg) seed);
        sceKernelUtilsMt19937Init(&mut ctx, seed);
        for i in 0..len {
            *dest.add(i) = (sceKernelUtilsMt19937UInt(&mut ctx) & 0xFF) as u8;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Theme constants (matching oasis-core/src/theme.rs)
// ---------------------------------------------------------------------------

// Bar geometry.
const STATUSBAR_H: u32 = 18;
const BOTTOMBAR_H: u32 = 32;
const BOTTOMBAR_Y: i32 = (SCREEN_HEIGHT - BOTTOMBAR_H) as i32;
const CONTENT_TOP: u32 = STATUSBAR_H;
const CONTENT_H: u32 = SCREEN_HEIGHT - CONTENT_TOP - BOTTOMBAR_H;

// Two-layer bottom bar row constants.
const BOTTOM_UPPER_Y: i32 = BOTTOMBAR_Y;
const BOTTOM_UPPER_H: u32 = 16;
const BOTTOM_LOWER_Y: i32 = BOTTOMBAR_Y + BOTTOM_UPPER_H as i32;

// Font metrics.
const CHAR_W: i32 = 8;

// Bottom bar layout.
const R_HINT_W: i32 = 28;

// Icon theme (compact to fit 4 rows).
const ICON_W: u32 = 42;
const ICON_H: u32 = 40;
const ICON_STRIPE_H: u32 = 8;
const ICON_FOLD_SIZE: u32 = 7;
const ICON_GFX_H: u32 = 16;
const ICON_GFX_PAD: u32 = 3;
const ICON_LABEL_PAD: i32 = 1;

// Dashboard grid (3 columns, 4 rows = 12 icons per page, L/R pagination).
const GRID_COLS: usize = 3;
const GRID_ROWS: usize = 4;
const GRID_PAD_X: i32 = 15;
const GRID_PAD_Y: i32 = 2;
const CELL_W: i32 = 150;
const CELL_H: i32 = (CONTENT_H as i32 - 2 * GRID_PAD_Y) / GRID_ROWS as i32;
const ICONS_PER_PAGE: usize = GRID_COLS * GRID_ROWS;
const CURSOR_PAD: i32 = 3;

// Persistent configuration path on Memory Stick.
const CONFIG_PATH: &str = "ms0:/PSP/GAME/OASISOS/config.rcfg";

// Colors -- bar backgrounds (green-tinted opaque, matching PSIX reference).
const STATUSBAR_BG: Color = Color::rgba(30, 80, 30, 200);
const BAR_BG: Color = Color::rgba(30, 80, 30, 200);
const SEPARATOR: Color = Color::rgba(180, 220, 180, 80);

// Colors -- status bar.
const BATTERY_CLR: Color = Color::rgb(120, 255, 120);
// Colors -- bottom bar.
const URL_CLR: Color = Color::rgb(200, 200, 200);
const USB_CLR: Color = Color::rgb(140, 140, 140);
const R_HINT_CLR: Color = Color::rgba(255, 255, 255, 140);
// Colors -- visualizer & transport.
const VIZ_BAR_PEAK: Color = Color::rgba(180, 100, 220, 230);
const TRANSPORT_CLR: Color = Color::rgba(220, 220, 220, 200);
const TRANSPORT_ACTIVE: Color = Color::rgb(120, 255, 120);
const L_HINT_CLR: Color = Color::rgba(255, 255, 255, 140);

// Visualizer constants.
const VIZ_BAR_COUNT: i32 = 14;
const VIZ_BAR_W: i32 = 3;
const VIZ_BAR_GAP: i32 = 1;
const VIZ_BAR_MAX_H: i32 = 12;
const VIZ_BAR_MIN_H: i32 = 1;

// Colors -- chrome bezel (green-tinted, matching PSIX reference).
const BEZEL_FILL: Color = Color::rgba(50, 100, 50, 120);
const BEZEL_TOP: Color = Color::rgba(200, 240, 200, 140);
const BEZEL_BOTTOM: Color = Color::rgba(20, 50, 20, 160);
const BEZEL_LEFT: Color = Color::rgba(180, 220, 180, 100);
const BEZEL_RIGHT: Color = Color::rgba(30, 60, 30, 140);

// Colors -- icons.
const BODY_CLR: Color = Color::rgb(250, 250, 248);
const FOLD_CLR: Color = Color::rgb(210, 210, 205);
const OUTLINE_CLR: Color = Color::rgba(255, 255, 255, 180);
const SHADOW_CLR: Color = Color::rgba(0, 0, 0, 70);
const LABEL_CLR: Color = Color::rgba(255, 255, 255, 230);

// Icon graphic symbol colors.
const ICON_SYM_CLR: Color = Color::rgba(255, 255, 255, 200);

// Label shadow.
const LABEL_SHADOW: Color = Color::rgba(0, 0, 0, 120);

// Button hints.
const HINT_BG: Color = Color::rgba(0, 0, 0, 120);
const HINT_BTN_CLR: Color = Color::rgb(200, 200, 100);
const HINT_TEXT_CLR: Color = Color::rgb(180, 180, 180);
const HINT_Y_OFFSET: i32 = 10;

// Terminal.
const MAX_OUTPUT_LINES: usize = 20;
const TERM_INPUT_Y: i32 = BOTTOMBAR_Y - 14;

// File manager.
const FM_VISIBLE_ROWS: usize = 18;
const FM_ROW_H: i32 = 10;
const FM_START_Y: i32 = CONTENT_TOP as i32 + 14;

// ---------------------------------------------------------------------------
// App entries (matching oasis-core FALLBACK_COLORS)
// ---------------------------------------------------------------------------

struct AppEntry {
    id: &'static str,
    title: &'static str,
    color: Color,
}

static APPS: &[AppEntry] = &[
    AppEntry {
        id: "filemgr",
        title: "File Manager",
        color: Color::rgb(70, 130, 180),
    },
    AppEntry {
        id: "settings",
        title: "Settings",
        color: Color::rgb(60, 179, 113),
    },
    AppEntry {
        id: "network",
        title: "Network",
        color: Color::rgb(218, 165, 32),
    },
    AppEntry {
        id: "terminal",
        title: "Terminal",
        color: Color::rgb(178, 102, 178),
    },
    AppEntry {
        id: "music",
        title: "Music Player",
        color: Color::rgb(205, 92, 92),
    },
    AppEntry {
        id: "photos",
        title: "Photo Viewer",
        color: Color::rgb(100, 149, 237),
    },
    AppEntry {
        id: "packages",
        title: "Package Mgr",
        color: Color::rgb(70, 130, 180),
    },
    AppEntry {
        id: "sysmon",
        title: "Sys Monitor",
        color: Color::rgb(60, 179, 113),
    },
    AppEntry {
        id: "browser",
        title: "Browser",
        color: Color::rgb(50, 120, 200),
    },
    AppEntry {
        id: "radio",
        title: "Radio",
        color: Color::rgb(255, 140, 60),
    },
    AppEntry {
        id: "tvguide",
        title: "TV Guide",
        color: Color::rgb(0, 100, 200),
    },
];

// ---------------------------------------------------------------------------
// App modes (Classic = full-screen, Desktop = windowed WM)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum AppMode {
    /// Classic PSIX full-screen dashboard (existing behavior, default).
    Classic,
    /// Windowed desktop mode with floating windows managed by WM.
    Desktop,
}

// Classic sub-modes (within AppMode::Classic).
#[derive(Clone, Copy, PartialEq)]
enum ClassicView {
    Dashboard,
    Terminal,
    FileManager,
    PhotoViewer,
    MusicPlayer,
    Browser,
    Radio,
    TvGuide,
}

// ---------------------------------------------------------------------------
// Radio station list and status
// ---------------------------------------------------------------------------

struct RadioStation {
    name: &'static str,
    genre: &'static str,
    url: &'static str,
    bitrate: u32,
}

static RADIO_STATIONS: &[RadioStation] = &[
    RadioStation {
        name: "Drone Zone",
        genre: "ambient",
        url: "http://ice2.somafm.com/dronezone-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "DEF CON Radio",
        genre: "hacker",
        url: "http://ice2.somafm.com/defcon-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "Groove Salad",
        genre: "chill",
        url: "http://ice2.somafm.com/groovesalad-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "Space Station",
        genre: "space",
        url: "http://ice2.somafm.com/spacestation-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "Secret Agent",
        genre: "lounge",
        url: "http://ice2.somafm.com/secretagent-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "Lush",
        genre: "female vocal",
        url: "http://ice2.somafm.com/lush-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "Metal Detector",
        genre: "metal",
        url: "http://ice2.somafm.com/metal-128-mp3",
        bitrate: 128,
    },
    RadioStation {
        name: "Boot Liquor",
        genre: "americana",
        url: "http://ice2.somafm.com/bootliquor-128-mp3",
        bitrate: 128,
    },
];

#[derive(Clone, Copy, PartialEq)]
enum RadioStatus {
    Stopped,
    Connecting,
    Buffering,
    Playing,
    Error,
}

// ---------------------------------------------------------------------------
// Boot splash screen
// ---------------------------------------------------------------------------

/// Draw a boot splash screen with title, status text, and progress bar.
///
/// Uses fill_rect for the background (bypasses FAST_CLEAR on PPSSPP),
/// draws progress bar with fill_rects, then renders both text lines in
/// a **single** SpriteBatch + texture bind to avoid GE state issues on
/// PPSSPP with multiple sprite draws per frame during init.
fn show_boot_screen(backend: &mut PspBackend, status: &str, progress: u32) {
    use oasis_backend_psp::render::{FONT_ATLAS_H, FONT_ATLAS_W};
    use psp::gu_ext::SpriteBatch;

    let bg = Color::rgba(15, 15, 25, 255);
    backend.fill_rect_inner(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT, bg);

    // Progress bar (200px wide, centered).
    let title_y = SCREEN_HEIGHT as i32 / 2 - 30;
    let status_y = title_y + 16;
    let bar_w: u32 = 200;
    let bar_h: u32 = 6;
    let bar_x = (SCREEN_WIDTH as i32 - bar_w as i32) / 2;
    let bar_y = status_y + 20;
    backend.fill_rect_inner(bar_x, bar_y, bar_w, bar_h, Color::rgba(40, 40, 60, 200));
    let fill_w = (bar_w * progress.min(100)) / 100;
    if fill_w > 0 {
        backend.fill_rect_inner(bar_x, bar_y, fill_w, bar_h, Color::rgb(80, 140, 220));
    }

    // Single SpriteBatch for both title and status text.
    let title = "OASIS_OS";
    let atlas_cols: u32 = 16;
    let total_chars = title.len() + status.len();
    let mut batch = SpriteBatch::new(total_chars);

    let title_w = (title.len() as i32) * CHAR_W;
    let title_x = (SCREEN_WIDTH as i32 - title_w) / 2;
    let white_abgr = 0xFFFF_FFFFu32;
    let mut cx = title_x as f32;
    for ch in title.chars() {
        let idx = (ch as u32).wrapping_sub(32);
        let (u0, v0) = if idx < 95 {
            ((idx % atlas_cols * 8) as f32, (idx / atlas_cols * 8) as f32)
        } else {
            (0.0, 0.0)
        };
        batch.draw_rect(
            cx,
            title_y as f32,
            8.0,
            8.0,
            u0,
            v0,
            u0 + 8.0,
            v0 + 8.0,
            white_abgr,
        );
        cx += 8.0;
    }

    let status_w = (status.len() as i32) * CHAR_W;
    let status_x = (SCREEN_WIDTH as i32 - status_w) / 2;
    let status_abgr = 0xFFC8AAA0u32; // Color::rgb(160, 170, 200) in ABGR
    cx = status_x as f32;
    for ch in status.chars() {
        let idx = (ch as u32).wrapping_sub(32);
        let (u0, v0) = if idx < 95 {
            ((idx % atlas_cols * 8) as f32, (idx / atlas_cols * 8) as f32)
        } else {
            (0.0, 0.0)
        };
        batch.draw_rect(
            cx,
            status_y as f32,
            8.0,
            8.0,
            u0,
            v0,
            u0 + 8.0,
            v0 + 8.0,
            status_abgr,
        );
        cx += 8.0;
    }

    // Single texture bind + single flush for all text.
    // SAFETY: Within an active GU display list; font atlas pointer is
    // valid and non-null (set during backend.init()).
    unsafe {
        use psp::sys::{
            self, MipmapLevel, TextureColorComponent, TextureEffect, TexturePixelFormat,
        };
        use std::ffi::c_void;
        let uncached_atlas = psp::cache::UncachedPtr::from_cached_addr(backend.font_atlas())
            .as_ptr() as *const c_void;
        sys::sceGuTexMode(TexturePixelFormat::Psm8888, 0, 0, 0);
        sys::sceGuTexImage(
            MipmapLevel::None,
            FONT_ATLAS_W as i32,
            FONT_ATLAS_H as i32,
            FONT_ATLAS_W as i32,
            uncached_atlas,
        );
        sys::sceGuTexFunc(TextureEffect::Modulate, TextureColorComponent::Rgba);
        sys::sceGuTexFlush();
        sys::sceGuTexSync();
        batch.flush();
    }

    backend.swap_buffers_inner();
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn psp_main() {
    let _ = psp::callback::setup_exit_callback();

    let mut backend = PspBackend::new();
    backend.init();
    show_boot_screen(&mut backend, "Initializing...", 10);

    // Register exception handler (kernel mode only) for crash diagnostics.
    #[cfg(feature = "kernel-exception")]
    oasis_backend_psp::register_exception_handler();
    show_boot_screen(&mut backend, "Loading config...", 25);

    // Load persistent configuration.
    let mut config =
        psp::config::Config::load(CONFIG_PATH).unwrap_or_else(|_| psp::config::Config::new());

    // Set clock speed from config (default: max 333MHz).
    let clock_mhz = config.get_i32("clock_mhz").unwrap_or(333);
    let bus_mhz = config.get_i32("bus_mhz").unwrap_or(166);
    oasis_backend_psp::set_clock(clock_mhz, bus_mhz);

    // Query static hardware info.
    let sysinfo = SystemInfo::query();
    show_boot_screen(&mut backend, "Generating textures...", 40);

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
    show_boot_screen(&mut backend, "Setting up UI...", 60);

    // -- Window Manager (Desktop mode) --
    let psp_theme = oasis_backend_psp::psp_wm_theme();
    let mut wm = WindowManager::with_theme(SCREEN_WIDTH, SCREEN_HEIGHT, psp_theme);
    let mut sdi = SdiRegistry::new();

    // -- App mode --
    let mut app_mode = AppMode::Classic;
    let mut classic_view = ClassicView::Dashboard;

    let mut selected: usize = 0;
    let page: usize = 0;
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
        show_boot_screen(&mut backend, "Running self-test...", 90);
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
    show_boot_screen(&mut backend, "Starting workers...", 80);

    // Confirm button held state for pointer simulation.
    let mut _confirm_held = false;

    // Register power callback for sleep/wake handling (keep handle alive).
    let _power_cb = oasis_backend_psp::register_power_callback();

    // Frame timing via hardware tick counter.
    let mut frame_timer = psp::time::FrameTimer::new();
    show_boot_screen(&mut backend, "Ready", 100);
    psp::thread::sleep_ms(400);

    loop {
        let _dt = frame_timer.tick();
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
                        let text = strip_html(&html);
                        br_content_lines = wrap_text(&text, 58);
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
                        handle_wm_event(
                            &wm_event,
                            &mut term_lines,
                            &mut classic_view,
                            &mut app_mode,
                            &mut wm,
                            &mut sdi,
                            page,
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
                        // Open app launcher: cycle through apps and open as windows.
                        let idx = page * ICONS_PER_PAGE + selected;
                        if idx < APPS.len() {
                            let app = &APPS[idx];
                            open_app_window(&mut wm, &mut sdi, app.id, app.title);
                        }
                    },
                    InputEvent::ButtonPress(Button::Start) => {
                        // Toggle terminal window.
                        open_app_window(&mut wm, &mut sdi, "terminal", "Terminal");
                    },
                    // Dashboard navigation works in Desktop mode too.
                    InputEvent::ButtonPress(Button::Up) => {
                        if selected >= GRID_COLS {
                            selected -= GRID_COLS;
                            audio.send(AudioCmd::PlaySfx(SfxId::Click));
                        }
                    },
                    InputEvent::ButtonPress(Button::Down) => {
                        let page_start = page * ICONS_PER_PAGE;
                        let page_count = APPS.len().saturating_sub(page_start).min(ICONS_PER_PAGE);
                        if selected + GRID_COLS < page_count {
                            selected += GRID_COLS;
                            audio.send(AudioCmd::PlaySfx(SfxId::Click));
                        }
                    },
                    InputEvent::ButtonPress(Button::Left) => {
                        let page_start = page * ICONS_PER_PAGE;
                        let page_count = APPS.len().saturating_sub(page_start).min(ICONS_PER_PAGE);
                        if selected == 0 {
                            selected = if page_count > 0 { page_count - 1 } else { 0 };
                        } else {
                            selected -= 1;
                        }
                        audio.send(AudioCmd::PlaySfx(SfxId::Click));
                    },
                    InputEvent::ButtonPress(Button::Right) => {
                        let page_start = page * ICONS_PER_PAGE;
                        let page_count = APPS.len().saturating_sub(page_start).min(ICONS_PER_PAGE);
                        selected = (selected + 1) % page_count.max(1);
                        audio.send(AudioCmd::PlaySfx(SfxId::Click));
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

                // -- Dashboard input --
                InputEvent::ButtonPress(Button::Up) if classic_view == ClassicView::Dashboard => {
                    if selected >= GRID_COLS {
                        selected -= GRID_COLS;
                        audio.send(AudioCmd::PlaySfx(SfxId::Click));
                    }
                },
                InputEvent::ButtonPress(Button::Down) if classic_view == ClassicView::Dashboard => {
                    let page_start = page * ICONS_PER_PAGE;
                    let page_count = APPS.len().saturating_sub(page_start).min(ICONS_PER_PAGE);
                    if selected + GRID_COLS < page_count {
                        selected += GRID_COLS;
                        audio.send(AudioCmd::PlaySfx(SfxId::Click));
                    }
                },
                InputEvent::ButtonPress(Button::Left) if classic_view == ClassicView::Dashboard => {
                    let page_start = page * ICONS_PER_PAGE;
                    let page_count = APPS.len().saturating_sub(page_start).min(ICONS_PER_PAGE);
                    if selected == 0 {
                        selected = if page_count > 0 { page_count - 1 } else { 0 };
                    } else {
                        selected -= 1;
                    }
                    audio.send(AudioCmd::PlaySfx(SfxId::Click));
                },
                InputEvent::ButtonPress(Button::Right)
                    if classic_view == ClassicView::Dashboard =>
                {
                    let page_start = page * ICONS_PER_PAGE;
                    let page_count = APPS.len().saturating_sub(page_start).min(ICONS_PER_PAGE);
                    selected = (selected + 1) % page_count.max(1);
                    audio.send(AudioCmd::PlaySfx(SfxId::Click));
                },
                InputEvent::ButtonPress(Button::Confirm)
                    if classic_view == ClassicView::Dashboard =>
                {
                    audio.send(AudioCmd::PlaySfx(SfxId::Navigate));
                    let idx = page * ICONS_PER_PAGE + selected;
                    if idx < APPS.len() {
                        let app = &APPS[idx];
                        match app.title {
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
                                // Keep radio_status if already playing.
                            },
                            "TV Guide" => {
                                classic_view = ClassicView::TvGuide;
                                // Parse channels on first open.
                                if tv_channels.is_empty() {
                                    if let Ok(config) =
                                        oasis_core::apps::tv_guide::ChannelConfig::from_toml(
                                            oasis_core::apps::tv_guide::channel
                                                ::DEFAULT_CHANNELS_TOML,
                                        )
                                    {
                                        tv_channels = config.channel;
                                        tv_catalogs = vec![None; tv_channels.len()];
                                        // Fetch catalogs from IA for each channel.
                                        for (i, ch) in tv_channels.iter().enumerate() {
                                            for (si, src) in ch.source.iter().enumerate() {
                                                let api_path =
                                                    oasis_core::apps::tv_guide::ChannelCatalog
                                                        ::files_api_path(&src.item_id);
                                                let url = format!(
                                                    "https://archive.org{}",
                                                    api_path,
                                                );
                                                // Tag layout: 0xAA in bits 8..15,
                                                // channel index in bits 0..7,
                                                // source index in bits 16..19.
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
                                app_mode = AppMode::Desktop;
                                open_app_window(&mut wm, &mut sdi, app.id, app.title);
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

                match classic_view {
                    ClassicView::Dashboard => {
                        backend.force_bitmap_font = true;
                        if !icons_hidden {
                            draw_dashboard(&mut backend, selected, page, viz_frame);
                        }
                        draw_button_hints(
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
                        backend.force_bitmap_font = true;
                        draw_terminal(&mut backend, &term_lines, &term_input, term_scroll);
                        draw_button_hints(
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
                        draw_file_manager_dual(
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
                        draw_button_hints(
                            &mut backend,
                            &[("X", "Open"), ("O", "Back"), ("<>", "Panel"), ("^v", "Nav")],
                        );
                        backend.force_bitmap_font = false;
                    },
                    ClassicView::PhotoViewer => {
                        backend.force_bitmap_font = true;
                        if pv_viewing {
                            draw_photo_view(&mut backend, pv_tex, pv_img_w, pv_img_h);
                            draw_button_hints(&mut backend, &[("O", "Back")]);
                        } else if pv_loading {
                            draw_loading_indicator(&mut backend, "Decoding image...");
                        } else {
                            draw_photo_browser(
                                &mut backend,
                                &pv_path,
                                &pv_entries,
                                pv_selected,
                                pv_scroll,
                            );
                            draw_button_hints(
                                &mut backend,
                                &[("X", "View"), ("O", "Back"), ("^v", "Nav")],
                            );
                        }
                        backend.force_bitmap_font = false;
                    },
                    ClassicView::MusicPlayer => {
                        backend.force_bitmap_font = true;
                        if audio.is_playing() {
                            draw_music_player_threaded(
                                &mut backend,
                                &mp_file_name,
                                &audio,
                                viz_frame,
                            );
                            draw_button_hints(
                                &mut backend,
                                &[("X", "Pause"), ("[]", "Stop"), ("^v", "Back")],
                            );
                        } else {
                            draw_music_browser(
                                &mut backend,
                                &mp_path,
                                &mp_entries,
                                mp_selected,
                                mp_scroll,
                            );
                            draw_button_hints(
                                &mut backend,
                                &[("X", "Play"), ("O", "Back"), ("^v", "Nav")],
                            );
                        }
                        backend.force_bitmap_font = false;
                    },
                    ClassicView::Browser => {
                        backend.force_bitmap_font = true;
                        if br_loading {
                            draw_loading_indicator(&mut backend, "Loading page...");
                        } else {
                            draw_browser_view(
                                &mut backend,
                                &br_url,
                                &br_content_lines,
                                br_scroll,
                                &br_status_msg,
                            );
                        }
                        draw_button_hints(
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
                                draw_radio_stations(&mut backend, radio_selected, radio_scroll);
                                draw_button_hints(
                                    &mut backend,
                                    &[("X", "Tune"), ("^v", "Nav"), ("O", "Back")],
                                );
                            },
                            RadioStatus::Connecting => {
                                draw_loading_indicator(&mut backend, "Connecting...");
                            },
                            RadioStatus::Buffering | RadioStatus::Playing => {
                                draw_radio_playing(
                                    &mut backend,
                                    &radio_station_name,
                                    &radio_now_playing,
                                    radio_status == RadioStatus::Buffering,
                                    &audio,
                                    viz_frame,
                                );
                                draw_button_hints(
                                    &mut backend,
                                    &[("[]", "Stop"), ("^", "Back"), ("O", "Stop+Back")],
                                );
                            },
                            RadioStatus::Error => {
                                draw_radio_error(&mut backend, &radio_error_msg);
                                draw_button_hints(&mut backend, &[("X", "Retry"), ("O", "Back")]);
                            },
                        }
                        backend.force_bitmap_font = false;
                    },
                    ClassicView::TvGuide => {
                        backend.force_bitmap_font = true;
                        if tv_tuned.is_some() {
                            draw_tv_playing(
                                &mut backend,
                                &tv_now_playing,
                                tv_downloading,
                                tv_download_progress,
                                tv_preview_tex,
                                &tv_error_msg,
                            );
                            draw_button_hints(
                                &mut backend,
                                &[("O", "Untune"), ("^", "Back")],
                            );
                        } else if !tv_error_msg.is_empty() {
                            draw_tv_error(&mut backend, &tv_error_msg);
                            draw_button_hints(
                                &mut backend,
                                &[("X", "Retry"), ("O", "Back")],
                            );
                        } else {
                            draw_tv_channels(
                                &mut backend,
                                &tv_channels,
                                &tv_catalogs,
                                tv_selected,
                                tv_scroll,
                            );
                            draw_button_hints(
                                &mut backend,
                                &[("X", "Tune"), ("^v", "Nav"), ("O", "Back")],
                            );
                        }
                        backend.force_bitmap_font = false;
                    },
                }
            },

            AppMode::Desktop => {
                // Draw dashboard icons behind windows.
                if !icons_hidden {
                    backend.force_bitmap_font = true;
                    draw_dashboard(&mut backend, selected, page, viz_frame);
                    backend.force_bitmap_font = false;
                }

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
                                draw_terminal_windowed(&term_lines, &term_input, cx, cy, cw, ch, be)
                            },
                            "filemgr" => draw_filemgr_windowed(
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
                            "photos" => draw_photos_windowed(
                                pv_tex, pv_img_w, pv_img_h, pv_viewing, cx, cy, cw, ch, be,
                            ),
                            "music" => {
                                draw_music_windowed(&mp_file_name, &audio, cx, cy, cw, ch, be)
                            },
                            "settings" => draw_settings_windowed(
                                settings_clock,
                                settings_bus,
                                current_vol,
                                cx,
                                cy,
                                cw,
                                ch,
                                be,
                            ),
                            "network" => draw_network_windowed(&status, cx, cy, cw, ch, be),
                            "sysmon" => draw_sysmon_windowed(
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
                            "browser" => draw_browser_windowed(cx, cy, cw, ch, be),
                            "packages" => draw_packages_windowed(cx, cy, cw, ch, be),
                            "radio" => draw_radio_windowed(&audio, cx, cy, cw, ch, be),
                            _ => Ok(()),
                        }
                    });

                backend.force_bitmap_font = false;
            },
        }

        // Status bar + bottom bar (always visible, drawn on top).
        // Force bitmap font: all bar layouts use `len() * 8` fixed-width metrics.
        backend.force_bitmap_font = true;
        draw_status_bar(&mut backend, &status, &sysinfo);

        let url_text = match (app_mode, classic_view) {
            (AppMode::Desktop, _) => String::from("SYS://DESKTOP"),
            (_, ClassicView::Dashboard) => String::from("SYS://DASHBOARD"),
            (_, ClassicView::Terminal) => String::from("SYS://TERMINAL"),
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
            (_, ClassicView::PhotoViewer) => String::from("SYS://PHOTOS"),
            (_, ClassicView::MusicPlayer) => {
                if audio.is_playing() {
                    String::from("SYS://NOW_PLAY")
                } else {
                    String::from("SYS://MUSIC")
                }
            },
            (_, ClassicView::Browser) => String::from("SYS://BROWSER"),
            (_, ClassicView::Radio) => {
                if audio.is_radio_streaming() {
                    String::from("SYS://RADIO_ON")
                } else {
                    String::from("SYS://RADIO")
                }
            },
            (_, ClassicView::TvGuide) => {
                if tv_tuned.is_some() {
                    String::from("SYS://TV_LIVE")
                } else {
                    String::from("SYS://TV_GUIDE")
                }
            },
        };
        let desktop_wm = if app_mode == AppMode::Desktop {
            Some(&wm)
        } else {
            None
        };
        draw_bottom_bar(
            &mut backend,
            &audio,
            viz_frame,
            &status,
            &url_text,
            desktop_wm,
        );
        backend.force_bitmap_font = false;
        viz_frame = viz_frame.wrapping_add(1);

        // Cursor (always on top).
        let (cx, cy) = backend.cursor_pos();
        backend.blit_inner(cursor_tex, cx, cy, CURSOR_W, CURSOR_H);

        backend.swap_buffers_inner();
    }
}

// ---------------------------------------------------------------------------
// Desktop mode helpers
// ---------------------------------------------------------------------------

/// Check if coordinates are over a dashboard icon, returning the global index.
fn hit_test_dashboard_icon(x: i32, y: i32, page: usize) -> Option<usize> {
    let page_start = page * ICONS_PER_PAGE;
    let page_end = (page_start + ICONS_PER_PAGE).min(APPS.len());
    for i in 0..(page_end - page_start) {
        let col = (i % GRID_COLS) as i32;
        let row = (i / GRID_COLS) as i32;
        let cell_x = GRID_PAD_X + col * CELL_W;
        let cell_y = CONTENT_TOP as i32 + GRID_PAD_Y + row * CELL_H;
        let ix = cell_x + (CELL_W - ICON_W as i32) / 2;
        let iy = cell_y + 1;
        if x >= ix
            && x < ix + ICON_W as i32
            && y >= iy
            && y < iy + ICON_H as i32 + ICON_LABEL_PAD + 10
        {
            return Some(page_start + i);
        }
    }
    None
}

/// Open an app as a floating window (or focus if already open).
fn open_app_window(wm: &mut WindowManager, sdi: &mut SdiRegistry, app_id: &str, title: &str) {
    if wm.get_window(app_id).is_some() {
        let _ = wm.focus_window(app_id, sdi);
        return;
    }
    let config = WindowConfig {
        id: app_id.to_string(),
        title: title.to_string(),
        x: None,
        y: Some(STATUSBAR_H as i32 + 2),
        width: 300,
        height: 180,
        window_type: WindowType::AppWindow,
        always_on_top: false,
        modal: false,
    };
    let _ = wm.create_window(&config, sdi);
}

/// Handle WM events (window closed, desktop click opens apps, etc.).
fn handle_wm_event(
    event: &WmEvent,
    term_lines: &mut Vec<String>,
    _classic_view: &mut ClassicView,
    _app_mode: &mut AppMode,
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    page: usize,
) {
    match event {
        WmEvent::WindowClosed(id) => {
            term_lines.push(format!("[WM] Window closed: {}", id));
        },
        WmEvent::ContentClick(id, lx, ly) => {
            term_lines.push(format!("[WM] Click in {}: ({}, {})", id, lx, ly));
        },
        WmEvent::DesktopClick(x, y) => {
            if let Some(idx) = hit_test_dashboard_icon(*x, *y, page) {
                if idx < APPS.len() {
                    open_app_window(wm, sdi, APPS[idx].id, APPS[idx].title);
                }
            }
        },
        _ => {},
    }
}

/// Draw desktop window tabs in the bottom bar lower row.
fn draw_desktop_taskbar_row(backend: &mut PspBackend, wm: &WindowManager) {
    let y = BOTTOM_LOWER_Y + 2;

    // L hint.
    backend.draw_text_inner("<L", 4, BOTTOM_LOWER_Y + 4, 8, L_HINT_CLR);

    let active_id = wm.active_window();
    let mut tx = 24i32;

    for app in APPS {
        if wm.get_window(app.id).is_some() {
            let is_active = active_id == Some(app.id);
            let label_clr = if is_active {
                Color::WHITE
            } else {
                Color::rgb(160, 160, 160)
            };
            if is_active {
                let label_w = (app.title.len() as i32 * 8 + 8) as u32;
                backend.fill_rect_inner(tx - 2, y, label_w, 12, Color::rgba(60, 90, 160, 140));
            }
            backend.draw_text_inner(app.title, tx + 2, y + 1, 8, label_clr);
            tx += app.title.len() as i32 * 8 + 12;
        }
    }

    // R hint.
    backend.draw_text_inner(
        "R>",
        SCREEN_WIDTH as i32 - R_HINT_W,
        BOTTOM_LOWER_Y + 4,
        8,
        R_HINT_CLR,
    );
}

// ---------------------------------------------------------------------------
// Windowed content renderers (for draw_with_clips callback)
// ---------------------------------------------------------------------------

fn draw_terminal_windowed(
    lines: &[String],
    input: &str,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    let bg = Color::rgba(0, 0, 0, 200);
    be.fill_rect(cx, cy, cw, ch, bg)?;

    let max_lines = (ch as usize) / 9;
    let visible_start = if lines.len() > max_lines {
        lines.len() - max_lines
    } else {
        0
    };
    for (i, line) in lines[visible_start..].iter().enumerate() {
        let y = cy + 2 + i as i32 * 9;
        if y > cy + ch as i32 - 14 {
            break;
        }
        be.draw_text(line, cx + 2, y, 8, Color::rgb(0, 255, 0))?;
    }

    let prompt = format!("> {}_", input);
    be.draw_text(
        &prompt,
        cx + 2,
        cy + ch as i32 - 12,
        8,
        Color::rgb(0, 255, 0),
    )?;
    Ok(())
}

fn draw_filemgr_windowed(
    path_l: &str,
    entries_l: &[FileEntry],
    selected_l: usize,
    scroll_l: usize,
    path_r: &str,
    entries_r: &[FileEntry],
    selected_r: usize,
    scroll_r: usize,
    active_panel: usize,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(0, 0, 0, 200))?;

    let half_w = cw / 2;
    let div_x = cx + half_w as i32;

    // Panel path headers.
    let l_clr = if active_panel == 0 {
        Color::rgb(100, 200, 255)
    } else {
        Color::rgb(140, 140, 140)
    };
    let r_clr = if active_panel == 1 {
        Color::rgb(100, 200, 255)
    } else {
        Color::rgb(140, 140, 140)
    };
    be.draw_text(path_l, cx + 2, cy + 2, 8, l_clr)?;
    be.draw_text(path_r, div_x + 2, cy + 2, 8, r_clr)?;

    // Vertical divider.
    be.fill_rect(div_x, cy + 12, 1, ch - 12, Color::rgba(100, 200, 255, 80))?;

    // Draw each panel.
    let panels: [(&[FileEntry], usize, usize, i32, u32, bool); 2] = [
        (
            entries_l,
            selected_l,
            scroll_l,
            cx,
            half_w - 1,
            active_panel == 0,
        ),
        (
            entries_r,
            selected_r,
            scroll_r,
            div_x + 1,
            cw - half_w,
            active_panel == 1,
        ),
    ];
    let max_rows = ((ch as i32 - 14) / FM_ROW_H) as usize;

    for &(entries, selected, scroll, px, _pw, is_active) in &panels {
        let end = (scroll + max_rows).min(entries.len());
        for i in scroll..end {
            let entry = &entries[i];
            let row = (i - scroll) as i32;
            let y = cy + 14 + row * FM_ROW_H;
            if i == selected && is_active {
                be.fill_rect(
                    px,
                    y - 1,
                    half_w,
                    FM_ROW_H as u32,
                    Color::rgba(80, 120, 200, 100),
                )?;
            }
            let (prefix, clr) = if entry.is_dir {
                ("[D]", Color::rgb(255, 220, 80))
            } else {
                ("[F]", Color::rgb(180, 180, 180))
            };
            be.draw_text(prefix, px + 2, y, 8, clr)?;
            let name_clr = if entry.is_dir {
                Color::rgb(120, 220, 255)
            } else {
                Color::WHITE
            };
            be.draw_text(&entry.name, px + 28, y, 8, name_clr)?;
        }
    }
    Ok(())
}

fn draw_photos_windowed(
    tex: Option<TextureId>,
    img_w: u32,
    img_h: u32,
    viewing: bool,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::BLACK)?;
    if viewing {
        if let Some(t) = tex {
            let scale_w = cw as f32 / img_w as f32;
            let scale_h = ch as f32 / img_h as f32;
            let scale = if scale_w < scale_h { scale_w } else { scale_h };
            let dw = (img_w as f32 * scale) as u32;
            let dh = (img_h as f32 * scale) as u32;
            let dx = cx + ((cw - dw) / 2) as i32;
            let dy = cy + ((ch - dh) / 2) as i32;
            be.blit(t, dx, dy, dw, dh)?;
        }
    } else {
        be.draw_text(
            "Select photo from browser",
            cx + 4,
            cy + 4,
            8,
            Color::rgb(160, 160, 160),
        )?;
    }
    Ok(())
}

fn draw_music_windowed(
    file_name: &str,
    audio: &AudioHandle,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(0, 0, 0, 210))?;

    if audio.is_playing() {
        let center_x = cx + cw as i32 / 2;
        be.draw_text(file_name, cx + 4, cy + 4, 8, Color::rgb(255, 200, 200))?;
        let info = format!(
            "{}Hz {}kbps {}ch",
            audio.sample_rate(),
            audio.bitrate(),
            audio.channels(),
        );
        let info_x = center_x - (info.len() as i32 * 8) / 2;
        be.draw_text(&info, info_x, cy + 18, 8, Color::rgb(180, 180, 180))?;
        let status = if audio.is_paused() {
            "PAUSED"
        } else {
            "PLAYING"
        };
        let status_clr = if audio.is_paused() {
            Color::rgb(255, 200, 80)
        } else {
            Color::rgb(120, 255, 120)
        };
        let status_x = center_x - (status.len() as i32 * 8) / 2;
        be.draw_text(status, status_x, cy + ch as i32 / 2, 8, status_clr)?;
    } else {
        be.draw_text(
            "No track loaded",
            cx + 4,
            cy + 4,
            8,
            Color::rgb(160, 160, 160),
        )?;
    }
    Ok(())
}

fn draw_settings_windowed(
    clock_mhz: i32,
    bus_mhz: i32,
    vol_info: Option<(usize, usize)>,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(0, 20, 10, 210))?;
    be.draw_text("SETTINGS", cx + 4, cy + 2, 8, Color::rgb(60, 179, 113))?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let lbl = Color::rgb(160, 160, 160);
    let val = Color::WHITE;
    let mut y = cy + 16;
    let vx = cx + 110;

    be.draw_text("CPU Clock:", cx + 4, y, 8, lbl)?;
    be.draw_text(&format!("{} MHz", clock_mhz), vx, y, 8, val)?;
    y += 10;

    be.draw_text("Bus Clock:", cx + 4, y, 8, lbl)?;
    be.draw_text(&format!("{} MHz", bus_mhz), vx, y, 8, val)?;
    y += 10;

    let profile = match clock_mhz {
        333 => "Max Performance",
        266 => "Balanced",
        222 => "Power Save",
        _ => "Custom",
    };
    be.draw_text("Profile:", cx + 4, y, 8, lbl)?;
    be.draw_text(profile, vx, y, 8, val)?;
    y += 10;

    be.draw_text("Display:", cx + 4, y, 8, lbl)?;
    be.draw_text("480x272 RGBA8888", vx, y, 8, val)?;
    y += 10;

    if let Some((total, remaining)) = vol_info {
        let used_kb = (total - remaining) / 1024;
        let total_kb = total / 1024;
        be.draw_text("Tex Cache:", cx + 4, y, 8, lbl)?;
        be.draw_text(&format!("{}/{} KB", used_kb, total_kb), vx, y, 8, val)?;
    } else {
        be.draw_text("Tex Cache:", cx + 4, y, 8, lbl)?;
        be.draw_text("N/A (PSP-1000)", vx, y, 8, Color::rgb(140, 140, 140))?;
    }

    Ok(())
}

fn draw_network_windowed(
    status: &StatusBarInfo,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(15, 12, 0, 210))?;
    be.draw_text("NETWORK", cx + 4, cy + 2, 8, Color::rgb(218, 165, 32))?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let lbl = Color::rgb(160, 160, 160);
    let mut y = cy + 16;
    let vx = cx + 110;

    let (wifi_str, wifi_clr) = if status.wifi_on {
        ("ON", Color::rgb(100, 200, 255))
    } else {
        ("OFF", Color::rgb(255, 100, 100))
    };
    be.draw_text("WiFi Switch:", cx + 4, y, 8, lbl)?;
    be.draw_text(wifi_str, vx, y, 8, wifi_clr)?;
    y += 10;

    let (usb_str, usb_clr) = if status.usb_connected {
        ("Connected", Color::rgb(120, 255, 120))
    } else {
        ("Disconnected", Color::rgb(160, 160, 160))
    };
    be.draw_text("USB Cable:", cx + 4, y, 8, lbl)?;
    be.draw_text(usb_str, vx, y, 8, usb_clr)?;
    y += 10;

    let (ac_str, ac_clr) = if status.ac_power {
        ("Connected", Color::rgb(120, 255, 120))
    } else {
        ("Battery", Color::rgb(200, 200, 200))
    };
    be.draw_text("AC Power:", cx + 4, y, 8, lbl)?;
    be.draw_text(ac_str, vx, y, 8, ac_clr)?;
    y += 10;

    if status.battery_percent >= 0 {
        be.draw_text("Battery:", cx + 4, y, 8, lbl)?;
        be.draw_text(
            &format!("{}%", status.battery_percent),
            vx,
            y,
            8,
            Color::WHITE,
        )?;
    }

    Ok(())
}

fn draw_sysmon_windowed(
    status: &StatusBarInfo,
    sysinfo: &SystemInfo,
    fps: f32,
    free_kb: i32,
    max_blk_kb: i32,
    vol_info: Option<(usize, usize)>,
    usb_active: bool,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(0, 10, 20, 210))?;
    be.draw_text(
        "SYSTEM MONITOR",
        cx + 4,
        cy + 2,
        8,
        Color::rgb(60, 179, 113),
    )?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let lbl = Color::rgb(140, 140, 140);
    let val = Color::WHITE;
    let mut y = cy + 16;
    let vx = cx + 100;

    let fps_clr = if fps >= 55.0 {
        Color::rgb(120, 255, 120)
    } else if fps >= 30.0 {
        Color::rgb(255, 200, 80)
    } else {
        Color::rgb(255, 80, 80)
    };
    be.draw_text("FPS:", cx + 4, y, 8, lbl)?;
    be.draw_text(&format!("{:.1}", fps), vx, y, 8, fps_clr)?;
    y += 11;

    be.draw_text("CPU/Bus/ME:", cx + 4, y, 8, lbl)?;
    be.draw_text(
        &format!("{}/{}/{}", sysinfo.cpu_mhz, sysinfo.bus_mhz, sysinfo.me_mhz),
        vx,
        y,
        8,
        val,
    )?;
    y += 11;

    be.draw_text("Free RAM:", cx + 4, y, 8, lbl)?;
    be.draw_text(&format!("{} KB", free_kb), vx, y, 8, val)?;
    y += 11;

    be.draw_text("Max Block:", cx + 4, y, 8, lbl)?;
    be.draw_text(&format!("{} KB", max_blk_kb), vx, y, 8, val)?;
    y += 11;

    if let Some((total, remaining)) = vol_info {
        let used_kb = (total - remaining) / 1024;
        let total_kb = total / 1024;
        be.draw_text("Tex VRAM:", cx + 4, y, 8, lbl)?;
        be.draw_text(&format!("{}/{} KB", used_kb, total_kb), vx, y, 8, val)?;
        y += 11;
    }

    let bat_clr = if status.battery_charging || status.battery_percent >= 50 {
        Color::rgb(120, 255, 120)
    } else if status.battery_percent >= 20 {
        Color::rgb(255, 200, 80)
    } else {
        Color::rgb(255, 80, 80)
    };
    let bat_str = if status.battery_percent >= 0 {
        if status.battery_charging {
            format!("{}% CHG", status.battery_percent)
        } else {
            format!("{}%", status.battery_percent)
        }
    } else if status.ac_power {
        "AC".into()
    } else {
        "N/A".into()
    };
    be.draw_text("Battery:", cx + 4, y, 8, lbl)?;
    be.draw_text(&bat_str, vx, y, 8, bat_clr)?;
    y += 11;

    let wifi_str = if status.wifi_on { "ON" } else { "OFF" };
    let usb_str = if usb_active {
        "STORAGE"
    } else if status.usb_connected {
        "CONN"
    } else {
        "---"
    };
    be.draw_text("WiFi:", cx + 4, y, 8, lbl)?;
    be.draw_text(wifi_str, vx, y, 8, val)?;
    be.draw_text("USB:", cx + 150, y, 8, lbl)?;
    be.draw_text(usb_str, cx + 190, y, 8, val)?;

    Ok(())
}

fn draw_browser_windowed(
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(5, 10, 25, 210))?;
    be.draw_text("BROWSER", cx + 4, cy + 2, 8, Color::rgb(50, 120, 200))?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let lbl = Color::rgb(160, 160, 160);
    let mut y = cy + 20;
    be.draw_text("Web browser for PSP.", cx + 4, y, 8, lbl)?;
    y += 14;
    be.draw_text("Use Terminal to browse:", cx + 4, y, 8, lbl)?;
    y += 12;
    be.draw_text("  open <url>", cx + 4, y, 8, Color::rgb(120, 200, 255))?;
    y += 12;
    be.draw_text("  gemini <url>", cx + 4, y, 8, Color::rgb(120, 200, 255))?;
    y += 14;
    be.draw_text("Supports HTML, CSS, Gemini.", cx + 4, y, 8, lbl)?;
    Ok(())
}

fn draw_packages_windowed(
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(5, 10, 20, 210))?;
    be.draw_text("PACKAGE MGR", cx + 4, cy + 2, 8, Color::rgb(70, 130, 180))?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let lbl = Color::rgb(160, 160, 160);
    let mut y = cy + 20;
    be.draw_text("Manage homebrew packages.", cx + 4, y, 8, lbl)?;
    y += 14;
    be.draw_text("Use Terminal commands:", cx + 4, y, 8, lbl)?;
    y += 12;
    be.draw_text("  pkg list", cx + 4, y, 8, Color::rgb(120, 200, 255))?;
    y += 12;
    be.draw_text(
        "  pkg install <name>",
        cx + 4,
        y,
        8,
        Color::rgb(120, 200, 255),
    )?;
    Ok(())
}

fn draw_radio_windowed(
    _audio: &AudioHandle,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(20, 10, 0, 210))?;
    be.draw_text("RADIO", cx + 4, cy + 2, 8, Color::rgb(255, 140, 60))?;
    be.fill_rect(cx, cy + 12, cw, 1, Color::rgba(255, 255, 255, 40))?;

    let lbl = Color::rgb(160, 160, 160);
    let hi = Color::rgb(120, 200, 255);
    let mut y = cy + 20;

    be.draw_text(
        "Internet Radio Streaming",
        cx + 4,
        y,
        8,
        Color::rgb(255, 200, 80),
    )?;
    y += 14;
    be.draw_text("Stations: SomaFM (8 presets)", cx + 4, y, 8, lbl)?;
    y += 14;
    be.draw_text("In-game: L+R+Start to open", cx + 4, y, 8, hi)?;
    y += 12;
    be.draw_text("overlay and toggle radio.", cx + 4, y, 8, hi)?;
    y += 14;
    be.draw_text("Requires WiFi connection.", cx + 4, y, 8, lbl)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Loading indicator
// ---------------------------------------------------------------------------

fn draw_loading_indicator(backend: &mut PspBackend, msg: &str) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);
    let cx = SCREEN_WIDTH as i32 / 2;
    let cy = CONTENT_TOP as i32 + CONTENT_H as i32 / 2;
    let text_x = cx - (msg.len() as i32 * 8) / 2;
    backend.draw_text_inner(msg, text_x, cy, 8, Color::rgb(200, 200, 200));
}

// ---------------------------------------------------------------------------
// Dashboard rendering
// ---------------------------------------------------------------------------

fn draw_dashboard(backend: &mut PspBackend, selected: usize, page: usize, viz_frame: u32) {
    let page_start = page * ICONS_PER_PAGE;
    let page_end = (page_start + ICONS_PER_PAGE).min(APPS.len());
    let page_count = page_end - page_start;

    for i in 0..page_count {
        let app = &APPS[page_start + i];
        let col = (i % GRID_COLS) as i32;
        let row = (i / GRID_COLS) as i32;
        let cell_x = GRID_PAD_X + col * CELL_W;
        let cell_y = CONTENT_TOP as i32 + GRID_PAD_Y + row * CELL_H;
        let ix = cell_x + (CELL_W - ICON_W as i32) / 2;
        let iy = cell_y + 1;

        draw_icon(backend, app, ix, iy);

        // Label below icon with drop shadow.
        let label_y = iy + ICON_H as i32 + ICON_LABEL_PAD;
        let text_width = (app.title.len() as i32) * CHAR_W;
        let label_x = cell_x + (CELL_W - text_width) / 2;
        backend.draw_text_inner(app.title, label_x + 1, label_y + 1, 8, LABEL_SHADOW);
        backend.draw_text_inner(app.title, label_x, label_y, 8, LABEL_CLR);
    }

    // Pulsing border around selected icon.
    if page_count > 0 && selected < page_count {
        let sel_col = (selected % GRID_COLS) as i32;
        let sel_row = (selected / GRID_COLS) as i32;
        let cell_x = GRID_PAD_X + sel_col * CELL_W;
        let cell_y = CONTENT_TOP as i32 + GRID_PAD_Y + sel_row * CELL_H;
        let ix = cell_x + (CELL_W - ICON_W as i32) / 2;
        let iy = cell_y + 1;

        let pulse = ((libm::sinf(viz_frame as f32 * 0.08) + 1.0) * 0.5 * 80.0) as u8;
        let sel_clr = Color::rgba(255, 255, 255, 60 + pulse);
        let bx = ix - CURSOR_PAD;
        let by = iy - CURSOR_PAD;
        let bw = ICON_W + CURSOR_PAD as u32 * 2;
        let bh = ICON_H + CURSOR_PAD as u32 * 2;
        // Top edge.
        backend.fill_rect_inner(bx, by, bw, 2, sel_clr);
        // Bottom edge.
        backend.fill_rect_inner(bx, by + bh as i32 - 2, bw, 2, sel_clr);
        // Left edge.
        backend.fill_rect_inner(bx, by, 2, bh, sel_clr);
        // Right edge.
        backend.fill_rect_inner(bx + bw as i32 - 2, by, 2, bh, sel_clr);
    }
}

/// Draw a PSIX document-style icon with 6 layers:
/// shadow, outline, body, stripe, fold, app graphic.
fn draw_icon(backend: &mut PspBackend, app: &AppEntry, ix: i32, iy: i32) {
    backend.fill_rect_inner(ix + 2, iy + 3, ICON_W + 2, ICON_H + 1, SHADOW_CLR);
    backend.fill_rect_inner(ix - 1, iy - 1, ICON_W + 2, ICON_H + 2, OUTLINE_CLR);
    backend.fill_rect_inner(ix, iy, ICON_W, ICON_H, BODY_CLR);
    backend.fill_rect_inner(ix, iy, ICON_W - ICON_FOLD_SIZE, ICON_STRIPE_H, app.color);
    backend.fill_rect_inner(
        ix + ICON_W as i32 - ICON_FOLD_SIZE as i32,
        iy,
        ICON_FOLD_SIZE,
        ICON_FOLD_SIZE,
        FOLD_CLR,
    );

    let gfx_w = ICON_W - 2 * ICON_GFX_PAD;
    let gx = ix + ICON_GFX_PAD as i32;
    let gy = iy + ICON_STRIPE_H as i32 + 3;
    let c = app.color;
    let gfx_color = Color::rgba(
        c.r.saturating_add(30),
        c.g.saturating_add(10),
        c.b.saturating_add(30),
        200,
    );
    backend.fill_rect_inner(gx, gy, gfx_w, ICON_GFX_H, gfx_color);

    // Per-app mini-graphic symbol.
    draw_icon_graphic(backend, app.id, gx, gy, gfx_w, ICON_GFX_H);
}

/// Draw a recognizable per-app symbol inside the icon graphic area.
fn draw_icon_graphic(backend: &mut PspBackend, app_id: &str, gx: i32, gy: i32, gw: u32, gh: u32) {
    let s = ICON_SYM_CLR;
    let cx = gx + gw as i32 / 2;
    let cy = gy + gh as i32 / 2;

    match app_id {
        "filemgr" => {
            // Folder: body rect + tab on top-left.
            backend.fill_rect_inner(cx - 8, cy - 2, 16, 8, s);
            backend.fill_rect_inner(cx - 8, cy - 5, 7, 3, s);
        },
        "settings" => {
            // Gear: 3x3 cross pattern (5 fill_rects).
            backend.fill_rect_inner(cx - 5, cy - 1, 10, 3, s);
            backend.fill_rect_inner(cx - 1, cy - 5, 3, 10, s);
            backend.fill_rect_inner(cx - 4, cy - 4, 3, 3, s);
            backend.fill_rect_inner(cx + 2, cy - 4, 3, 3, s);
            backend.fill_rect_inner(cx - 4, cy + 2, 3, 3, s);
        },
        "network" => {
            // WiFi arcs: 3 horizontal bars widening bottom-up.
            backend.fill_rect_inner(cx - 2, cy + 2, 5, 2, s);
            backend.fill_rect_inner(cx - 5, cy - 1, 11, 2, s);
            backend.fill_rect_inner(cx - 8, cy - 4, 17, 2, s);
        },
        "terminal" => {
            // >_ prompt text.
            backend.draw_text_inner(">_", cx - 8, cy - 4, 8, s);
        },
        "music" => {
            // Music note: stem + filled head.
            backend.fill_rect_inner(cx + 2, cy - 5, 2, 10, s);
            backend.fill_rect_inner(cx - 3, cy + 2, 5, 3, s);
        },
        "photos" => {
            // Mountain/landscape: stepped pyramid.
            backend.fill_rect_inner(cx - 8, cy + 2, 17, 2, s);
            backend.fill_rect_inner(cx - 5, cy - 1, 11, 3, s);
            backend.fill_rect_inner(cx - 2, cy - 4, 5, 3, s);
        },
        "packages" => {
            // Box/crate: outlined rect + cross divider.
            backend.fill_rect_inner(cx - 7, cy - 5, 15, 1, s);
            backend.fill_rect_inner(cx - 7, cy + 4, 15, 1, s);
            backend.fill_rect_inner(cx - 7, cy - 5, 1, 10, s);
            backend.fill_rect_inner(cx + 7, cy - 5, 1, 10, s);
            backend.fill_rect_inner(cx, cy - 5, 1, 10, s);
        },
        "sysmon" => {
            // Bar chart: 3 vertical bars at different heights.
            backend.fill_rect_inner(cx - 6, cy, 4, 5, s);
            backend.fill_rect_inner(cx - 1, cy - 3, 4, 8, s);
            backend.fill_rect_inner(cx + 4, cy - 5, 4, 10, s);
        },
        "browser" => {
            // Globe: circle outline approximation (H cross + V cross).
            backend.fill_rect_inner(cx - 7, cy - 1, 15, 2, s);
            backend.fill_rect_inner(cx - 1, cy - 7, 2, 14, s);
            backend.fill_rect_inner(cx - 6, cy - 5, 1, 10, s);
            backend.fill_rect_inner(cx + 6, cy - 5, 1, 10, s);
            backend.fill_rect_inner(cx - 5, cy - 6, 10, 1, s);
            backend.fill_rect_inner(cx - 5, cy + 6, 10, 1, s);
        },
        "radio" => {
            // Radio waves: antenna dot + arcs.
            backend.fill_rect_inner(cx - 1, cy + 2, 3, 3, s);
            backend.fill_rect_inner(cx - 4, cy - 1, 2, 4, s);
            backend.fill_rect_inner(cx + 3, cy - 1, 2, 4, s);
            backend.fill_rect_inner(cx - 7, cy - 4, 2, 6, s);
            backend.fill_rect_inner(cx + 6, cy - 4, 2, 6, s);
        },
        _ => {},
    }
}

// ---------------------------------------------------------------------------
// Status bar rendering
// ---------------------------------------------------------------------------

fn draw_status_bar(backend: &mut PspBackend, status: &StatusBarInfo, sysinfo: &SystemInfo) {
    backend.fill_rect_inner(0, 0, SCREEN_WIDTH, STATUSBAR_H, STATUSBAR_BG);
    // Gradient simulation: highlight strips at top.
    backend.fill_rect_inner(0, 0, SCREEN_WIDTH, 1, Color::rgba(255, 255, 255, 20));
    backend.fill_rect_inner(0, 1, SCREEN_WIDTH, 1, Color::rgba(255, 255, 255, 10));
    backend.fill_rect_inner(0, STATUSBAR_H as i32 - 1, SCREEN_WIDTH, 1, SEPARATOR);

    // -- Left side: battery percentage + charging bolt + WiFi + CPU MHz --

    // Battery percentage (color-coded).
    let bat_label = if status.battery_percent >= 0 {
        format!("{}%", status.battery_percent)
    } else if status.ac_power {
        String::from("AC")
    } else {
        String::from("---")
    };
    let bat_color = if status.battery_charging || status.ac_power {
        BATTERY_CLR
    } else if status.battery_percent < 20 {
        Color::rgb(255, 80, 80)
    } else {
        BATTERY_CLR
    };
    backend.draw_text_inner(&bat_label, 6, 5, 8, bat_color);
    let bat_w = bat_label.len() as i32 * CHAR_W;

    // Charging bolt indicator (Z shape) when battery is charging.
    let mut next_x = 6 + bat_w + 4;
    if status.battery_charging {
        let bolt_clr = Color::rgb(255, 220, 60);
        backend.fill_rect_inner(next_x + 1, 5, 3, 2, bolt_clr);
        backend.fill_rect_inner(next_x, 7, 3, 2, bolt_clr);
        backend.fill_rect_inner(next_x - 1, 9, 3, 2, bolt_clr);
        next_x += 7;
    }

    // WiFi indicator square.
    let wifi_x = next_x;
    if status.wifi_on {
        backend.fill_rect_inner(wifi_x, 7, 5, 5, Color::rgb(100, 200, 255));
    } else {
        let off = Color::rgb(100, 100, 100);
        backend.fill_rect_inner(wifi_x, 7, 5, 1, off);
        backend.fill_rect_inner(wifi_x, 11, 5, 1, off);
        backend.fill_rect_inner(wifi_x, 7, 1, 5, off);
        backend.fill_rect_inner(wifi_x + 4, 7, 1, 5, off);
    }

    // CPU MHz with filled-square indicator.
    let mhz_x = wifi_x + 8;
    backend.fill_rect_inner(mhz_x, 7, 5, 5, Color::WHITE);
    let mhz_label = format!("{} MHZ", sysinfo.cpu_mhz);
    backend.draw_text_inner(&mhz_label, mhz_x + 8, 5, 8, Color::WHITE);

    // -- Right side: time + day-of-week + full date --
    let date_label = format!(
        "{:02}:{:02} {} {} {}, {}",
        status.hour,
        status.minute,
        status.day_of_week,
        status.month_name(),
        status.day,
        status.year,
    );
    let date_w = date_label.len() as i32 * CHAR_W;
    let date_x = SCREEN_WIDTH as i32 - date_w - 6;
    backend.draw_text_inner(&date_label, date_x, 5, 8, Color::WHITE);
}

// ---------------------------------------------------------------------------
// Bottom bar rendering
// ---------------------------------------------------------------------------

fn draw_bottom_bar(
    backend: &mut PspBackend,
    audio: &AudioHandle,
    viz_frame: u32,
    status: &StatusBarInfo,
    url_text: &str,
    desktop_wm: Option<&WindowManager>,
) {
    // Full 32px bottom bar background with gradient simulation.
    backend.fill_rect_inner(0, BOTTOMBAR_Y, SCREEN_WIDTH, BOTTOMBAR_H, BAR_BG);
    backend.fill_rect_inner(0, BOTTOMBAR_Y, SCREEN_WIDTH, 1, SEPARATOR);
    backend.fill_rect_inner(
        0,
        BOTTOMBAR_Y + 1,
        SCREEN_WIDTH,
        1,
        Color::rgba(255, 255, 255, 15),
    );

    // -- Upper row (y=BOTTOM_UPPER_Y, 16px): URL bezel | Visualizer --

    // URL chrome bezel (left, 140px).
    let url_bx = 2i32;
    let url_bw = 140u32;
    let ubz_y = BOTTOM_UPPER_Y + 1;
    let ubz_h = BOTTOM_UPPER_H - 2;
    draw_chrome_bezel(backend, url_bx, ubz_y, url_bw, ubz_h);
    // Truncate URL text to fit bezel (max 16 chars).
    let max_url = 16;
    let display_url = if url_text.len() > max_url {
        &url_text[..url_text.floor_char_boundary(max_url)]
    } else {
        url_text
    };
    backend.draw_text_inner(display_url, 6, BOTTOM_UPPER_Y + 4, 8, URL_CLR);

    // Visualizer (center of upper row).
    draw_visualizer(backend, audio, viz_frame);

    // -- Lower row (y=BOTTOM_LOWER_Y, 16px) --
    backend.fill_rect_inner(
        0,
        BOTTOM_LOWER_Y,
        SCREEN_WIDTH,
        1,
        Color::rgba(255, 255, 255, 20),
    );

    if let Some(wm) = desktop_wm {
        // Desktop mode: show window tab buttons in lower row.
        draw_desktop_taskbar_row(backend, wm);
    } else {
        // Classic mode: transport | USB | battery bar.
        backend.draw_text_inner("<L", 4, BOTTOM_LOWER_Y + 4, 8, L_HINT_CLR);
        draw_transport_controls(backend, audio);
        backend.draw_text_inner("USB", 250, BOTTOM_LOWER_Y + 4, 8, USB_CLR);
        draw_battery_bar(backend, status);
        backend.draw_text_inner(
            "R>",
            SCREEN_WIDTH as i32 - R_HINT_W,
            BOTTOM_LOWER_Y + 4,
            8,
            R_HINT_CLR,
        );
    }
}

/// Draw animated music visualizer bars in center of upper bottom row.
fn draw_visualizer(backend: &mut PspBackend, audio: &AudioHandle, viz_frame: u32) {
    let total_viz_w = VIZ_BAR_COUNT * (VIZ_BAR_W + VIZ_BAR_GAP) - VIZ_BAR_GAP;
    let viz_x = (SCREEN_WIDTH as i32 - total_viz_w) / 2;
    let viz_base_y = BOTTOM_UPPER_Y + BOTTOM_UPPER_H as i32 - 2;
    let playing = audio.is_playing() && !audio.is_paused();

    for i in 0..VIZ_BAR_COUNT {
        let bar_h = if playing {
            // Composite waveform: two sine waves per bar.
            let t = viz_frame as f32 * 0.12;
            let freq1 = 0.7 + (i as f32) * 0.3;
            let freq2 = 1.4 + (i as f32) * 0.15;
            let phase = (i as f32) * 1.1;
            let val =
                libm::sinf(t * freq1 + phase) * 0.6 + libm::sinf(t * freq2 + phase * 0.7) * 0.4;
            let norm = (val + 1.0) * 0.5;
            VIZ_BAR_MIN_H + ((VIZ_BAR_MAX_H - VIZ_BAR_MIN_H) as f32 * norm) as i32
        } else {
            VIZ_BAR_MIN_H
        };
        let bx = viz_x + i * (VIZ_BAR_W + VIZ_BAR_GAP);
        let by = viz_base_y - bar_h;
        // Per-bar color tint for visual interest.
        let r = (120 + ((i * 4) as u8).min(40)) as u8;
        let b = (160 + ((i * 3) as u8).min(30)) as u8;
        let bar_clr = Color::rgba(r, 60, b, 200);
        backend.fill_rect_inner(bx, by, VIZ_BAR_W as u32, bar_h as u32, bar_clr);
        // Peak highlight (top 1px).
        if bar_h > 1 {
            backend.fill_rect_inner(bx, by, VIZ_BAR_W as u32, 1, VIZ_BAR_PEAK);
        }
    }
}

/// Draw transport controls in the lower bottom row.
fn draw_transport_controls(backend: &mut PspBackend, audio: &AudioHandle) {
    let y = BOTTOM_LOWER_Y + 4;
    let mut tx = 36i32;
    let playing = audio.is_playing();
    let paused = audio.is_paused();

    // Rewind.
    backend.draw_text_inner("<<", tx, y, 8, TRANSPORT_CLR);
    tx += 20;

    // Pause (two 2x8 bars, highlighted green when paused).
    let pause_clr = if playing && paused {
        TRANSPORT_ACTIVE
    } else {
        TRANSPORT_CLR
    };
    backend.fill_rect_inner(tx, y, 2, 8, pause_clr);
    backend.fill_rect_inner(tx + 4, y, 2, 8, pause_clr);
    tx += 12;

    // Play arrow (highlighted green when playing and not paused).
    let play_clr = if playing && !paused {
        TRANSPORT_ACTIVE
    } else {
        TRANSPORT_CLR
    };
    backend.draw_text_inner(">", tx, y, 8, play_clr);
    tx += 14;

    // Forward.
    backend.draw_text_inner(">>", tx, y, 8, TRANSPORT_CLR);
    tx += 20;

    // Stop (6x6 filled square, highlighted green when stopped).
    let stop_clr = if !playing {
        TRANSPORT_ACTIVE
    } else {
        TRANSPORT_CLR
    };
    backend.fill_rect_inner(tx, y + 1, 6, 6, stop_clr);
}

/// Draw horizontal battery bar in the lower bottom row.
fn draw_battery_bar(backend: &mut PspBackend, status: &StatusBarInfo) {
    let bar_x = 310i32;
    let bar_y = BOTTOM_LOWER_Y + 4;
    let bar_w = 60u32;
    let bar_h = 8u32;

    // Outline.
    backend.fill_rect_inner(bar_x, bar_y, bar_w, 1, Color::rgba(200, 200, 200, 140));
    backend.fill_rect_inner(
        bar_x,
        bar_y + bar_h as i32 - 1,
        bar_w,
        1,
        Color::rgba(200, 200, 200, 140),
    );
    backend.fill_rect_inner(bar_x, bar_y, 1, bar_h, Color::rgba(200, 200, 200, 140));
    backend.fill_rect_inner(
        bar_x + bar_w as i32 - 1,
        bar_y,
        1,
        bar_h,
        Color::rgba(200, 200, 200, 140),
    );

    // Dark bg fill.
    backend.fill_rect_inner(
        bar_x + 1,
        bar_y + 1,
        bar_w - 2,
        bar_h - 2,
        Color::rgba(20, 20, 20, 180),
    );

    // Battery nub on right side.
    backend.fill_rect_inner(
        bar_x + bar_w as i32,
        bar_y + 2,
        2,
        4,
        Color::rgba(200, 200, 200, 140),
    );

    // Colored fill proportional to battery_percent.
    let pct = if status.battery_percent >= 0 {
        status.battery_percent.min(100) as u32
    } else {
        0
    };
    let fill_w = ((bar_w - 2) * pct) / 100;
    if fill_w > 0 {
        let fill_clr = if pct >= 50 {
            Color::rgb(120, 255, 120)
        } else if pct >= 20 {
            Color::rgb(255, 200, 80)
        } else {
            Color::rgb(255, 80, 80)
        };
        backend.fill_rect_inner(bar_x + 1, bar_y + 1, fill_w, bar_h - 2, fill_clr);
    }
}

/// Draw a chrome/metallic bezel (fill + 4 corner-trimmed edges).
fn draw_chrome_bezel(backend: &mut PspBackend, x: i32, y: i32, w: u32, h: u32) {
    backend.fill_rect_inner(x, y, w, h, BEZEL_FILL);
    // Top/bottom edges trimmed 1px each side for pseudo-rounded corners.
    backend.fill_rect_inner(x + 1, y, w - 2, 1, BEZEL_TOP);
    backend.fill_rect_inner(x + 1, y + h as i32 - 1, w - 2, 1, BEZEL_BOTTOM);
    // Left/right edges trimmed 1px each end.
    backend.fill_rect_inner(x, y + 1, 1, h - 2, BEZEL_LEFT);
    backend.fill_rect_inner(x + w as i32 - 1, y + 1, 1, h - 2, BEZEL_RIGHT);
}

// ---------------------------------------------------------------------------
// Shared UI helpers (button hints, view headers)
// ---------------------------------------------------------------------------

/// Draw contextual button hints at the bottom of the content area.
fn draw_button_hints(backend: &mut PspBackend, hints: &[(&str, &str)]) {
    let y = BOTTOMBAR_Y - HINT_Y_OFFSET;
    backend.fill_rect_inner(0, y, SCREEN_WIDTH, HINT_Y_OFFSET as u32, HINT_BG);
    let mut x = 6i32;
    for (btn, label) in hints {
        backend.draw_text_inner(btn, x, y + 1, 8, HINT_BTN_CLR);
        x += btn.len() as i32 * 8 + 2;
        backend.draw_text_inner(label, x, y + 1, 8, HINT_TEXT_CLR);
        x += label.len() as i32 * 8 + 10;
    }
}

/// Draw a consistent view header with colored title and optional path.
fn draw_view_header(backend: &mut PspBackend, title: &str, title_clr: Color, path: Option<&str>) {
    backend.draw_text_inner(title, 4, CONTENT_TOP as i32 + 3, 8, title_clr);
    if let Some(p) = path {
        let path_x = 4 + title.len() as i32 * 8 + 8;
        backend.draw_text_inner(
            p,
            path_x,
            CONTENT_TOP as i32 + 3,
            8,
            Color::rgb(160, 160, 160),
        );
    }
    backend.fill_rect_inner(
        0,
        FM_START_Y - 2,
        SCREEN_WIDTH,
        1,
        Color::rgba(255, 255, 255, 40),
    );
}

// ---------------------------------------------------------------------------
// Terminal rendering (classic full-screen)
// ---------------------------------------------------------------------------

fn draw_terminal(backend: &mut PspBackend, lines: &[String], input: &str, scroll_back: usize) {
    let bg = Color::rgba(0, 0, 0, 180);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    // Compute visible window: scroll_back=0 means show the latest lines.
    let end = lines.len().saturating_sub(scroll_back);
    let start = end.saturating_sub(MAX_OUTPUT_LINES);
    for (i, line) in lines[start..end].iter().enumerate() {
        let y = CONTENT_TOP as i32 + 4 + i as i32 * 9;
        if y > TERM_INPUT_Y - 12 {
            break;
        }
        backend.draw_text_inner(line, 4, y, 8, Color::rgb(0, 255, 0));
    }

    // Scroll indicator when not at bottom.
    if scroll_back > 0 {
        let indicator = format!("-- scroll: +{} --", scroll_back);
        backend.draw_text_inner(
            &indicator,
            SCREEN_WIDTH as i32 - indicator.len() as i32 * 8 - 4,
            TERM_INPUT_Y - 10,
            8,
            Color::rgb(180, 180, 0),
        );
    }

    let prompt = format!("> {}_", input);
    backend.draw_text_inner(&prompt, 4, TERM_INPUT_Y, 8, Color::rgb(0, 255, 0));
}

// ---------------------------------------------------------------------------
// File manager rendering (classic full-screen)
// ---------------------------------------------------------------------------

fn draw_file_manager_dual(
    backend: &mut PspBackend,
    path_l: &str,
    entries_l: &[FileEntry],
    selected_l: usize,
    scroll_l: usize,
    path_r: &str,
    entries_r: &[FileEntry],
    selected_r: usize,
    scroll_r: usize,
    active_panel: usize,
) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    // Header with both panel paths.
    let header = if active_panel == 0 {
        format!("[L] {}  |  {}", path_l, path_r)
    } else {
        format!("{}  |  [R] {}", path_l, path_r)
    };
    draw_view_header(
        backend,
        "FILE MGR",
        Color::rgb(100, 200, 255),
        Some(&header),
    );

    // Vertical divider.
    let half_w = SCREEN_WIDTH / 2;
    let div_x = half_w as i32;
    backend.fill_rect_inner(
        div_x,
        CONTENT_TOP as i32 + 12,
        1,
        CONTENT_H - 12,
        Color::rgba(100, 200, 255, 80),
    );

    // Active panel indicator (bright line at top of active panel).
    let indicator_x = if active_panel == 0 { 0 } else { div_x + 1 };
    let indicator_w = if active_panel == 0 {
        half_w - 1
    } else {
        half_w
    };
    backend.fill_rect_inner(
        indicator_x,
        CONTENT_TOP as i32 + 12,
        indicator_w,
        1,
        Color::rgb(100, 200, 255),
    );

    // Draw each panel.
    let panels: [(&[FileEntry], usize, usize, i32, u32, bool); 2] = [
        (
            entries_l,
            selected_l,
            scroll_l,
            0,
            half_w - 1,
            active_panel == 0,
        ),
        (
            entries_r,
            selected_r,
            scroll_r,
            div_x + 1,
            half_w,
            active_panel == 1,
        ),
    ];
    // Half the visible rows since panels are narrower but same height.
    let panel_rows = FM_VISIBLE_ROWS;

    for &(entries, selected, scroll, px, pw, is_active) in &panels {
        if entries.is_empty() {
            backend.draw_text_inner("(empty)", px + 4, FM_START_Y, 8, Color::rgb(140, 140, 140));
            continue;
        }

        let end = (scroll + panel_rows).min(entries.len());
        for i in scroll..end {
            let entry = &entries[i];
            let row = (i - scroll) as i32;
            let y = FM_START_Y + row * FM_ROW_H;

            if i == selected && is_active {
                backend.fill_rect_inner(
                    px,
                    y - 1,
                    pw,
                    FM_ROW_H as u32,
                    Color::rgba(80, 120, 200, 100),
                );
            }

            let (prefix, prefix_clr) = if entry.is_dir {
                ("[D]", Color::rgb(255, 220, 80))
            } else {
                ("[F]", Color::rgb(180, 180, 180))
            };
            backend.draw_text_inner(prefix, px + 2, y, 8, prefix_clr);

            let name_color = if entry.is_dir {
                Color::rgb(120, 220, 255)
            } else {
                Color::WHITE
            };
            // Max chars for half-width panel (~28 chars at 8px each).
            let max_name_chars = ((pw as i32 - 32) / CHAR_W).max(4) as usize;
            let display_name = if entry.name.len() > max_name_chars {
                let truncated: String = entry.name.chars().take(max_name_chars - 2).collect();
                format!("{}..", truncated)
            } else {
                entry.name.clone()
            };
            backend.draw_text_inner(&display_name, px + 28, y, 8, name_color);
        }

        // Scroll indicator per panel.
        if entries.len() > panel_rows {
            let ratio = selected as f32 / (entries.len() - 1).max(1) as f32;
            let track_h = CONTENT_H as i32 - 16;
            let dot_y = FM_START_Y + (ratio * track_h as f32) as i32;
            let dot_x = px + pw as i32 - 4;
            backend.fill_rect_inner(dot_x, dot_y, 3, 8, Color::rgba(255, 255, 255, 120));
        }
    }
}

// ---------------------------------------------------------------------------
// Photo viewer rendering (classic full-screen)
// ---------------------------------------------------------------------------

fn draw_photo_browser(
    backend: &mut PspBackend,
    path: &str,
    entries: &[FileEntry],
    selected: usize,
    scroll: usize,
) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(
        backend,
        "PHOTO VIEWER",
        Color::rgb(100, 149, 237),
        Some(path),
    );

    if entries.is_empty() {
        backend.draw_text_inner(
            "No images found (.jpg/.jpeg)",
            8,
            FM_START_Y,
            8,
            Color::rgb(140, 140, 140),
        );
        return;
    }

    let end = (scroll + FM_VISIBLE_ROWS).min(entries.len());
    for i in scroll..end {
        let entry = &entries[i];
        let row = (i - scroll) as i32;
        let y = FM_START_Y + row * FM_ROW_H;

        if i == selected {
            backend.fill_rect_inner(
                0,
                y - 1,
                SCREEN_WIDTH,
                FM_ROW_H as u32,
                Color::rgba(80, 120, 200, 100),
            );
        }

        let (prefix, prefix_clr) = if entry.is_dir {
            ("[D]", Color::rgb(255, 220, 80))
        } else {
            ("[I]", Color::rgb(100, 200, 255))
        };
        backend.draw_text_inner(prefix, 4, y, 8, prefix_clr);

        let name_color = if entry.is_dir {
            Color::rgb(120, 220, 255)
        } else {
            Color::WHITE
        };
        let max_name_chars = 44;
        let display_name = if entry.name.len() > max_name_chars {
            let truncated: String = entry.name.chars().take(max_name_chars - 2).collect();
            format!("{}..", truncated)
        } else {
            entry.name.clone()
        };
        backend.draw_text_inner(&display_name, 32, y, 8, name_color);

        if !entry.is_dir {
            let size_str = oasis_backend_psp::format_size(entry.size);
            let size_x = 480 - (size_str.len() as i32 * 8) - 4;
            backend.draw_text_inner(&size_str, size_x, y, 8, Color::rgb(180, 180, 180));
        }
    }

    // Scrollbar.
    if entries.len() > FM_VISIBLE_ROWS {
        let ratio = selected as f32 / (entries.len() - 1).max(1) as f32;
        let track_h = CONTENT_H as i32 - 16;
        let dot_y = FM_START_Y + (ratio * track_h as f32) as i32;
        backend.fill_rect_inner(
            SCREEN_WIDTH as i32 - 4,
            dot_y,
            3,
            8,
            Color::rgba(255, 255, 255, 120),
        );
    }
}

fn draw_photo_view(backend: &mut PspBackend, tex: Option<TextureId>, img_w: u32, img_h: u32) {
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, Color::BLACK);

    if let Some(t) = tex {
        let max_w = SCREEN_WIDTH;
        let max_h = CONTENT_H;
        let scale_w = max_w as f32 / img_w as f32;
        let scale_h = max_h as f32 / img_h as f32;
        let scale = if scale_w < scale_h { scale_w } else { scale_h };
        let draw_w = (img_w as f32 * scale) as u32;
        let draw_h = (img_h as f32 * scale) as u32;
        let draw_x = ((max_w - draw_w) / 2) as i32;
        let draw_y = CONTENT_TOP as i32 + ((max_h - draw_h) / 2) as i32;

        backend.blit_inner(t, draw_x, draw_y, draw_w, draw_h);
    } else {
        backend.draw_text_inner("Failed to load image", 160, 130, 8, Color::rgb(255, 80, 80));
    }
}

// ---------------------------------------------------------------------------
// Music player rendering (classic full-screen, threaded audio)
// ---------------------------------------------------------------------------

fn draw_music_browser(
    backend: &mut PspBackend,
    path: &str,
    entries: &[FileEntry],
    selected: usize,
    scroll: usize,
) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(backend, "MUSIC PLAYER", Color::rgb(205, 92, 92), Some(path));

    if entries.is_empty() {
        backend.draw_text_inner(
            "No MP3 files found",
            8,
            FM_START_Y,
            8,
            Color::rgb(140, 140, 140),
        );
        return;
    }

    let end = (scroll + FM_VISIBLE_ROWS).min(entries.len());
    for i in scroll..end {
        let entry = &entries[i];
        let row = (i - scroll) as i32;
        let y = FM_START_Y + row * FM_ROW_H;

        if i == selected {
            backend.fill_rect_inner(
                0,
                y - 1,
                SCREEN_WIDTH,
                FM_ROW_H as u32,
                Color::rgba(200, 80, 80, 100),
            );
        }

        let (prefix, prefix_clr) = if entry.is_dir {
            ("[D]", Color::rgb(255, 220, 80))
        } else {
            ("[M]", Color::rgb(205, 92, 92))
        };
        backend.draw_text_inner(prefix, 4, y, 8, prefix_clr);

        let name_color = if entry.is_dir {
            Color::rgb(120, 220, 255)
        } else {
            Color::WHITE
        };
        let max_name_chars = 44;
        let display_name = if entry.name.len() > max_name_chars {
            let truncated: String = entry.name.chars().take(max_name_chars - 2).collect();
            format!("{}..", truncated)
        } else {
            entry.name.clone()
        };
        backend.draw_text_inner(&display_name, 32, y, 8, name_color);

        if !entry.is_dir {
            let size_str = oasis_backend_psp::format_size(entry.size);
            let size_x = 480 - (size_str.len() as i32 * 8) - 4;
            backend.draw_text_inner(&size_str, size_x, y, 8, Color::rgb(180, 180, 180));
        }
    }

    // Scrollbar.
    if entries.len() > FM_VISIBLE_ROWS {
        let ratio = selected as f32 / (entries.len() - 1).max(1) as f32;
        let track_h = CONTENT_H as i32 - 16;
        let dot_y = FM_START_Y + (ratio * track_h as f32) as i32;
        backend.fill_rect_inner(
            SCREEN_WIDTH as i32 - 4,
            dot_y,
            3,
            8,
            Color::rgba(255, 255, 255, 120),
        );
    }
}

/// Draw the now-playing music player UI (using threaded AudioHandle).
fn draw_music_player_threaded(
    backend: &mut PspBackend,
    file_name: &str,
    audio: &AudioHandle,
    viz_frame: u32,
) {
    let bg = Color::rgba(0, 0, 0, 210);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    let cx = SCREEN_WIDTH as i32 / 2;
    let title_color = Color::rgb(255, 200, 200);
    let info_color = Color::rgb(180, 180, 180);

    // Now-playing visualizer above album art.
    draw_now_playing_visualizer(backend, audio, viz_frame);

    // Album art placeholder.
    let art_size: u32 = 70;
    let art_x = cx - art_size as i32 / 2;
    let art_y = CONTENT_TOP as i32 + 44;
    backend.fill_rect_inner(art_x, art_y, art_size, art_size, Color::rgb(205, 92, 92));
    backend.fill_rect_inner(
        art_x + 2,
        art_y + 2,
        art_size - 4,
        art_size - 4,
        Color::rgb(60, 30, 30),
    );
    backend.draw_text_inner("MP3", art_x + 22, art_y + 28, 8, Color::rgb(205, 92, 92));

    // Track name.
    let max_chars = 50;
    let display_name = if file_name.len() > max_chars {
        let truncated: String = file_name.chars().take(max_chars - 2).collect();
        format!("{}..", truncated)
    } else {
        file_name.to_string()
    };
    let name_x = cx - (display_name.len() as i32 * 8) / 2;
    backend.draw_text_inner(
        &display_name,
        name_x,
        art_y + art_size as i32 + 8,
        8,
        title_color,
    );

    // Format info from atomic state.
    let info = format!(
        "{}Hz  {}kbps  {}ch",
        audio.sample_rate(),
        audio.bitrate(),
        audio.channels(),
    );
    let info_x = cx - (info.len() as i32 * 8) / 2;
    backend.draw_text_inner(&info, info_x, art_y + art_size as i32 + 20, 8, info_color);

    // Progress bar.
    let pos = audio.position_ms();
    let dur = audio.duration_ms();
    let bar_w: u32 = 260;
    let bar_x = cx - bar_w as i32 / 2;
    let bar_y = art_y + art_size as i32 + 32;

    // Track bar outline.
    backend.fill_rect_inner(bar_x, bar_y, bar_w, 4, Color::rgba(80, 80, 80, 180));
    // Fill.
    if dur > 0 {
        let fill = ((bar_w as u64 * pos) / dur).min(bar_w as u64) as u32;
        if fill > 0 {
            backend.fill_rect_inner(bar_x, bar_y, fill, 4, Color::rgb(205, 92, 92));
        }
    }
    // Time labels.
    let pos_s = (pos / 1000) as u32;
    let dur_s = (dur / 1000) as u32;
    let time_str = format!(
        "{}:{:02} / {}:{:02}",
        pos_s / 60,
        pos_s % 60,
        dur_s / 60,
        dur_s % 60,
    );
    let time_x = cx - (time_str.len() as i32 * 8) / 2;
    backend.draw_text_inner(&time_str, time_x, bar_y + 6, 8, info_color);

    let status = if audio.is_paused() {
        "PAUSED"
    } else {
        "PLAYING"
    };
    let status_clr = if audio.is_paused() {
        Color::rgb(255, 200, 80)
    } else {
        Color::rgb(120, 255, 120)
    };
    let status_x = cx - (status.len() as i32 * 8) / 2;
    backend.draw_text_inner(status, status_x, bar_y + 20, 8, status_clr);
}

/// Draw a larger visualizer for the now-playing music player view.
fn draw_now_playing_visualizer(backend: &mut PspBackend, audio: &AudioHandle, viz_frame: u32) {
    let bar_count: i32 = 20;
    let bar_w: i32 = 6;
    let bar_gap: i32 = 2;
    let max_h: i32 = 30;
    let min_h: i32 = 2;
    let total_w = bar_count * (bar_w + bar_gap) - bar_gap;
    let viz_x = (SCREEN_WIDTH as i32 - total_w) / 2;
    let viz_base_y = CONTENT_TOP as i32 + 40;
    let playing = (audio.is_playing() && !audio.is_paused())
        || (audio.is_radio_streaming() && !audio.is_radio_buffering());

    for i in 0..bar_count {
        let bar_h = if playing {
            let t = viz_frame as f32 * 0.12;
            let freq1 = 0.7 + (i as f32) * 0.25;
            let freq2 = 1.4 + (i as f32) * 0.15;
            let phase = (i as f32) * 1.1;
            let val =
                libm::sinf(t * freq1 + phase) * 0.6 + libm::sinf(t * freq2 + phase * 0.7) * 0.4;
            let norm = (val + 1.0) * 0.5;
            min_h + ((max_h - min_h) as f32 * norm) as i32
        } else {
            min_h
        };
        let bx = viz_x + i * (bar_w + bar_gap);
        let by = viz_base_y - bar_h;
        let r = (120 + ((i * 4) as u8).min(40)) as u8;
        let b = (160 + ((i * 3) as u8).min(30)) as u8;
        let bar_clr = Color::rgba(r, 60, b, 200);
        backend.fill_rect_inner(bx, by, bar_w as u32, bar_h as u32, bar_clr);
        if bar_h > 2 {
            backend.fill_rect_inner(bx, by, bar_w as u32, 1, VIZ_BAR_PEAK);
        }
    }
}

// ---------------------------------------------------------------------------
// Browser rendering (classic full-screen)
// ---------------------------------------------------------------------------

/// Strip HTML tags and decode common entities.
fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut i = 0;
    let bytes = html.as_bytes();

    while i < bytes.len() {
        if in_script {
            // Look for </script>.
            if i + 8 < bytes.len() {
                let window: &[u8] = &bytes[i..i + 9];
                let lower: Vec<u8> = window.iter().map(|b| b.to_ascii_lowercase()).collect();
                if lower == b"</script>" {
                    in_script = false;
                    i += 9;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        if in_style {
            if i + 7 < bytes.len() {
                let window: &[u8] = &bytes[i..i + 8];
                let lower: Vec<u8> = window.iter().map(|b| b.to_ascii_lowercase()).collect();
                if lower == b"</style>" {
                    in_style = false;
                    i += 8;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        if bytes[i] == b'<' {
            // Check for <script or <style.
            if i + 7 < bytes.len() {
                let peek: Vec<u8> = bytes[i + 1..i + 7]
                    .iter()
                    .map(|b| b.to_ascii_lowercase())
                    .collect();
                if peek == b"script" {
                    in_script = true;
                    in_tag = true;
                    i += 1;
                    continue;
                }
                if peek.starts_with(b"style") {
                    in_style = true;
                    in_tag = true;
                    i += 1;
                    continue;
                }
            }
            in_tag = true;
            // Insert newline for block elements.
            if i + 2 < bytes.len() {
                let next = bytes[i + 1].to_ascii_lowercase();
                if next == b'p'
                    || next == b'h'
                    || (next == b'b'
                        && i + 3 < bytes.len()
                        && bytes[i + 2].to_ascii_lowercase() == b'r')
                    || (next == b'd'
                        && i + 3 < bytes.len()
                        && bytes[i + 2].to_ascii_lowercase() == b'i')
                    || (next == b'l'
                        && i + 3 < bytes.len()
                        && bytes[i + 2].to_ascii_lowercase() == b'i')
                {
                    out.push('\n');
                }
            }
            i += 1;
            continue;
        }
        if bytes[i] == b'>' {
            in_tag = false;
            i += 1;
            continue;
        }
        if in_tag {
            i += 1;
            continue;
        }
        // Decode entities.
        if bytes[i] == b'&' {
            if i + 4 < bytes.len() && &bytes[i..i + 4] == b"&lt;" {
                out.push('<');
                i += 4;
                continue;
            }
            if i + 4 < bytes.len() && &bytes[i..i + 4] == b"&gt;" {
                out.push('>');
                i += 4;
                continue;
            }
            if i + 5 < bytes.len() && &bytes[i..i + 5] == b"&amp;" {
                out.push('&');
                i += 5;
                continue;
            }
            if i + 6 < bytes.len() && &bytes[i..i + 6] == b"&nbsp;" {
                out.push(' ');
                i += 6;
                continue;
            }
            if i + 6 < bytes.len() && &bytes[i..i + 6] == b"&quot;" {
                out.push('"');
                i += 6;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Word-wrap text to `max_chars` columns.
fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let trimmed = paragraph.trim();
        if trimmed.is_empty() {
            if lines.last().map_or(true, |l: &String| !l.is_empty()) {
                lines.push(String::new());
            }
            continue;
        }
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        let mut line = String::new();
        for word in &words {
            if line.is_empty() {
                line = word.to_string();
            } else if line.len() + 1 + word.len() <= max_chars {
                line.push(' ');
                line.push_str(word);
            } else {
                lines.push(line);
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    lines
}

fn draw_browser_view(
    backend: &mut PspBackend,
    url: &str,
    lines: &[String],
    scroll: usize,
    status_msg: &str,
) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(backend, "BROWSER", Color::rgb(50, 120, 200), None);

    // URL bar.
    let url_y = CONTENT_TOP as i32 + 3;
    let url_x = 4 + 7 * CHAR_W + 8;
    let display_url = if url.len() > 45 {
        let trunc: String = url.chars().take(43).collect();
        format!("{}..", trunc)
    } else {
        url.to_string()
    };
    backend.draw_text_inner(&display_url, url_x, url_y, 8, Color::rgb(120, 180, 255));

    // Status line.
    backend.draw_text_inner(status_msg, 4, FM_START_Y - 1, 8, Color::rgb(160, 160, 160));

    // Content area.
    let text_start_y = FM_START_Y + 10;
    let visible_rows = ((BOTTOMBAR_Y - HINT_Y_OFFSET - text_start_y) / 9) as usize;
    let end = (scroll + visible_rows).min(lines.len());
    for i in scroll..end {
        let row = (i - scroll) as i32;
        let y = text_start_y + row * 9;
        backend.draw_text_inner(&lines[i], 4, y, 8, Color::rgb(220, 220, 220));
    }

    // Scroll indicator.
    if lines.len() > visible_rows && !lines.is_empty() {
        let ratio = scroll as f32 / (lines.len() - 1).max(1) as f32;
        let track_h = CONTENT_H as i32 - 30;
        let dot_y = text_start_y + (ratio * track_h as f32) as i32;
        backend.fill_rect_inner(
            SCREEN_WIDTH as i32 - 4,
            dot_y,
            3,
            8,
            Color::rgba(255, 255, 255, 120),
        );
    }
}

// ---------------------------------------------------------------------------
// Radio rendering (classic full-screen)
// ---------------------------------------------------------------------------

fn draw_radio_stations(backend: &mut PspBackend, selected: usize, scroll: usize) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(backend, "RADIO", Color::rgb(255, 140, 60), None);

    if RADIO_STATIONS.is_empty() {
        backend.draw_text_inner("No stations", 8, FM_START_Y, 8, Color::rgb(140, 140, 140));
        return;
    }

    let end = (scroll + FM_VISIBLE_ROWS).min(RADIO_STATIONS.len());
    for i in scroll..end {
        let station = &RADIO_STATIONS[i];
        let row = (i - scroll) as i32;
        let y = FM_START_Y + row * FM_ROW_H;

        if i == selected {
            backend.fill_rect_inner(
                0,
                y - 1,
                SCREEN_WIDTH,
                FM_ROW_H as u32,
                Color::rgba(255, 140, 60, 100),
            );
        }

        // Radio icon.
        backend.draw_text_inner("[R]", 4, y, 8, Color::rgb(255, 140, 60));

        // Station name.
        backend.draw_text_inner(station.name, 32, y, 8, Color::WHITE);

        // Genre.
        let genre_x = 230;
        backend.draw_text_inner(station.genre, genre_x, y, 8, Color::rgb(160, 160, 160));

        // Bitrate.
        let br_str = format!("{}k", station.bitrate);
        let br_x = 480 - (br_str.len() as i32 * 8) - 4;
        backend.draw_text_inner(&br_str, br_x, y, 8, Color::rgb(140, 140, 140));
    }

    // Scrollbar.
    if RADIO_STATIONS.len() > FM_VISIBLE_ROWS {
        let ratio = selected as f32 / (RADIO_STATIONS.len() - 1).max(1) as f32;
        let track_h = CONTENT_H as i32 - 16;
        let dot_y = FM_START_Y + (ratio * track_h as f32) as i32;
        backend.fill_rect_inner(
            SCREEN_WIDTH as i32 - 4,
            dot_y,
            3,
            8,
            Color::rgba(255, 255, 255, 120),
        );
    }
}

fn draw_radio_playing(
    backend: &mut PspBackend,
    station_name: &str,
    now_playing: &str,
    is_buffering: bool,
    audio: &AudioHandle,
    viz_frame: u32,
) {
    let bg = Color::rgba(0, 0, 0, 210);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    let cx = SCREEN_WIDTH as i32 / 2;

    // Visualizer (reuse music player's).
    draw_now_playing_visualizer(backend, audio, viz_frame);

    // Radio icon placeholder.
    let art_size: u32 = 70;
    let art_x = cx - art_size as i32 / 2;
    let art_y = CONTENT_TOP as i32 + 44;
    backend.fill_rect_inner(art_x, art_y, art_size, art_size, Color::rgb(255, 140, 60));
    backend.fill_rect_inner(
        art_x + 2,
        art_y + 2,
        art_size - 4,
        art_size - 4,
        Color::rgb(60, 40, 15),
    );
    backend.draw_text_inner("RADIO", art_x + 12, art_y + 28, 8, Color::rgb(255, 140, 60));

    // Station name.
    let max_chars = 50;
    let display_name = if station_name.len() > max_chars {
        let trunc: String = station_name.chars().take(max_chars - 2).collect();
        format!("{}..", trunc)
    } else {
        station_name.to_string()
    };
    let name_x = cx - (display_name.len() as i32 * 8) / 2;
    backend.draw_text_inner(
        &display_name,
        name_x,
        art_y + art_size as i32 + 8,
        8,
        Color::rgb(255, 200, 150),
    );

    // Now playing (ICY metadata).
    if !now_playing.is_empty() {
        let np_display = if now_playing.len() > max_chars {
            let trunc: String = now_playing.chars().take(max_chars - 2).collect();
            format!("{}..", trunc)
        } else {
            now_playing.to_string()
        };
        let np_x = cx - (np_display.len() as i32 * 8) / 2;
        backend.draw_text_inner(
            &np_display,
            np_x,
            art_y + art_size as i32 + 20,
            8,
            Color::rgb(180, 180, 180),
        );
    }

    // Status.
    let status = if is_buffering {
        "BUFFERING"
    } else {
        "STREAMING"
    };
    let status_clr = if is_buffering {
        Color::rgb(255, 200, 80)
    } else {
        Color::rgb(120, 255, 120)
    };
    let status_x = cx - (status.len() as i32 * 8) / 2;
    backend.draw_text_inner(
        status,
        status_x,
        art_y + art_size as i32 + 36,
        8,
        status_clr,
    );
}

fn draw_radio_error(backend: &mut PspBackend, error_msg: &str) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(backend, "RADIO", Color::rgb(255, 140, 60), None);

    let cx = SCREEN_WIDTH as i32 / 2;
    let cy = CONTENT_TOP as i32 + CONTENT_H as i32 / 2;

    backend.draw_text_inner(
        "Connection Error",
        cx - 8 * 8,
        cy - 12,
        8,
        Color::rgb(255, 80, 80),
    );

    let max_chars = 55;
    let display_msg = if error_msg.len() > max_chars {
        let trunc: String = error_msg.chars().take(max_chars - 2).collect();
        format!("{}..", trunc)
    } else {
        error_msg.to_string()
    };
    let msg_x = cx - (display_msg.len() as i32 * 8) / 2;
    backend.draw_text_inner(&display_msg, msg_x, cy + 4, 8, Color::rgb(200, 200, 200));

    backend.draw_text_inner(
        "Press X to retry or O to go back",
        cx - 16 * 8,
        cy + 20,
        8,
        Color::rgb(140, 140, 140),
    );
}

// ---------------------------------------------------------------------------
// TV Guide drawing functions
// ---------------------------------------------------------------------------

/// Draw the TV Guide channel list (browsing mode).
fn draw_tv_channels(
    backend: &mut PspBackend,
    channels: &[oasis_core::apps::tv_guide::Channel],
    catalogs: &[Option<oasis_core::apps::tv_guide::ChannelCatalog>],
    selected: usize,
    scroll: usize,
) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(backend, "TV GUIDE", Color::rgb(0, 100, 200), None);

    if channels.is_empty() {
        backend.draw_text_inner("No channels", 8, FM_START_Y, 8, Color::rgb(140, 140, 140));
        return;
    }

    let end = (scroll + FM_VISIBLE_ROWS).min(channels.len());
    for i in scroll..end {
        let ch = &channels[i];
        let row = (i - scroll) as i32;
        let y = FM_START_Y + row * FM_ROW_H;

        if i == selected {
            backend.fill_rect_inner(
                0,
                y - 1,
                SCREEN_WIDTH,
                FM_ROW_H as u32,
                Color::rgba(0, 100, 200, 100),
            );
        }

        // Channel number.
        let num_str = format!("{:2}", ch.number);
        backend.draw_text_inner(&num_str, 4, y, 8, Color::rgb(0, 160, 255));
        // Call sign.
        backend.draw_text_inner(&ch.call_sign, 28, y, 8, Color::WHITE);
        // Channel name.
        let name_x = 80;
        let max_name = 25;
        let display_name = if ch.name.len() > max_name {
            let trunc: String = ch.name.chars().take(max_name - 2).collect();
            format!("{}..", trunc)
        } else {
            ch.name.clone()
        };
        backend.draw_text_inner(&display_name, name_x, y, 8, Color::rgb(200, 200, 200));
        // Status indicator (loaded / loading).
        let status_x = 380;
        if i < catalogs.len() {
            if let Some(cat) = &catalogs[i] {
                let ep_str = format!("{}ep", cat.episodes.len());
                backend.draw_text_inner(&ep_str, status_x, y, 8, Color::rgb(120, 200, 120));
            } else {
                backend.draw_text_inner("...", status_x, y, 8, Color::rgb(180, 180, 80));
            }
        }
        // Genre.
        let genre_x = 430;
        let genre_display = if ch.genre.len() > 6 {
            let trunc: String = ch.genre.chars().take(5).collect();
            format!("{}", trunc)
        } else {
            ch.genre.clone()
        };
        backend.draw_text_inner(&genre_display, genre_x, y, 8, Color::rgb(140, 140, 140));
    }

    // Scrollbar.
    if channels.len() > FM_VISIBLE_ROWS {
        let ratio = selected as f32 / (channels.len() - 1).max(1) as f32;
        let track_h = CONTENT_H as i32 - 16;
        let dot_y = FM_START_Y + (ratio * track_h as f32) as i32;
        backend.fill_rect_inner(
            SCREEN_WIDTH as i32 - 4,
            dot_y,
            3,
            8,
            Color::rgba(255, 255, 255, 120),
        );
    }
}

/// Draw the TV Guide "now playing" / downloading view.
fn draw_tv_playing(
    backend: &mut PspBackend,
    now_playing: &str,
    downloading: bool,
    progress: f32,
    preview_tex: Option<TextureId>,
    error_msg: &str,
) {
    let bg = Color::rgba(0, 0, 0, 210);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    let cx = SCREEN_WIDTH as i32 / 2;

    if downloading {
        // Download progress view.
        draw_view_header(backend, "TV GUIDE", Color::rgb(0, 100, 200), None);

        let pct = (progress * 100.0) as u32;
        let status = format!("Downloading... {}%", pct);
        let status_x = cx - (status.len() as i32 * 8) / 2;
        backend.draw_text_inner(
            &status,
            status_x,
            CONTENT_TOP as i32 + 60,
            8,
            Color::rgb(255, 200, 80),
        );

        // Progress bar.
        let bar_w: u32 = 300;
        let bar_h: u32 = 8;
        let bar_x = cx - bar_w as i32 / 2;
        let bar_y = CONTENT_TOP as i32 + 80;
        backend.fill_rect_inner(bar_x, bar_y, bar_w, bar_h, Color::rgba(40, 40, 60, 200));
        let fill_w = (bar_w as f32 * progress) as u32;
        if fill_w > 0 {
            backend.fill_rect_inner(bar_x, bar_y, fill_w, bar_h, Color::rgb(0, 160, 255));
        }

        // Episode title.
        let max_chars = 50;
        let display = if now_playing.len() > max_chars {
            let trunc: String = now_playing.chars().take(max_chars - 2).collect();
            format!("{}..", trunc)
        } else {
            now_playing.to_string()
        };
        let title_x = cx - (display.len() as i32 * 8) / 2;
        backend.draw_text_inner(&display, title_x, bar_y + 20, 8, Color::rgb(180, 180, 180));
    } else if let Some(tex) = preview_tex {
        // Video playing -- show the decoded frame.
        // Scale to fit within the content area while preserving aspect ratio.
        let max_w = SCREEN_WIDTH;
        let max_h = CONTENT_H;
        backend.blit_inner(tex, 0, CONTENT_TOP as i32, max_w, max_h);

        // LIVE indicator.
        backend.fill_rect_inner(
            SCREEN_WIDTH as i32 - 48,
            CONTENT_TOP as i32 + 4,
            44,
            12,
            Color::rgba(200, 0, 0, 200),
        );
        backend.draw_text_inner(
            "LIVE",
            SCREEN_WIDTH as i32 - 40,
            CONTENT_TOP as i32 + 6,
            8,
            Color::WHITE,
        );

        // Title overlay at bottom.
        let max_chars = 50;
        let display = if now_playing.len() > max_chars {
            let trunc: String = now_playing.chars().take(max_chars - 2).collect();
            format!("{}..", trunc)
        } else {
            now_playing.to_string()
        };
        let title_y = BOTTOMBAR_Y - 14;
        backend.fill_rect_inner(0, title_y - 2, SCREEN_WIDTH, 12, Color::rgba(0, 0, 0, 160));
        backend.draw_text_inner(&display, 4, title_y, 8, Color::WHITE);
    } else {
        // No video frame yet but not downloading -- audio only or ended.
        draw_view_header(backend, "TV GUIDE", Color::rgb(0, 100, 200), None);

        let status = if !error_msg.is_empty() {
            error_msg
        } else {
            "Playing audio..."
        };
        let status_x = cx - (status.len() as i32 * 8) / 2;
        let status_clr = if error_msg.is_empty() {
            Color::rgb(120, 255, 120)
        } else {
            Color::rgb(255, 80, 80)
        };
        backend.draw_text_inner(status, status_x, CONTENT_TOP as i32 + 80, 8, status_clr);

        // Episode title.
        let max_chars = 50;
        let display = if now_playing.len() > max_chars {
            let trunc: String = now_playing.chars().take(max_chars - 2).collect();
            format!("{}..", trunc)
        } else {
            now_playing.to_string()
        };
        let title_x = cx - (display.len() as i32 * 8) / 2;
        backend.draw_text_inner(
            &display,
            title_x,
            CONTENT_TOP as i32 + 100,
            8,
            Color::rgb(180, 180, 180),
        );
    }
}

/// Draw TV Guide error screen.
fn draw_tv_error(backend: &mut PspBackend, error_msg: &str) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    draw_view_header(backend, "TV GUIDE", Color::rgb(0, 100, 200), None);

    let cx = SCREEN_WIDTH as i32 / 2;
    let cy = CONTENT_TOP as i32 + CONTENT_H as i32 / 2;

    backend.draw_text_inner("Error", cx - 2 * 8, cy - 12, 8, Color::rgb(255, 80, 80));

    let max_chars = 55;
    let display_msg = if error_msg.len() > max_chars {
        let trunc: String = error_msg.chars().take(max_chars - 2).collect();
        format!("{}..", trunc)
    } else {
        error_msg.to_string()
    };
    let msg_x = cx - (display_msg.len() as i32 * 8) / 2;
    backend.draw_text_inner(&display_msg, msg_x, cy + 4, 8, Color::rgb(200, 200, 200));

    backend.draw_text_inner(
        "Press X to retry or O to go back",
        cx - 16 * 8,
        cy + 20,
        8,
        Color::rgb(140, 140, 140),
    );
}

// Command interpreter and utilities are in commands.rs module.
