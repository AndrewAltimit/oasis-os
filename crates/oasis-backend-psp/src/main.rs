//! PSP entry point for OASIS_OS.
//!
//! PSIX-style dashboard with document icons, tabbed status bar, chrome bezel
//! bottom bar, terminal mode, and windowed desktop mode with floating windows
//! managed by the oasis-core WindowManager.
//!
//! Audio playback and file I/O run on background threads to prevent frame drops.

#![feature(restricted_std)]
#![no_main]

use oasis_backend_psp::{
    AudioCmd, AudioHandle, Button, Color, FileEntry, InputEvent, IoCmd, IoResponse,
    PspBackend, SdiBackend, SdiRegistry, SfxId, StatusBarInfo, SystemInfo, TextureId,
    Trigger, WindowConfig, WindowManager, WindowType, WmEvent, CURSOR_H, CURSOR_W,
    SCREEN_HEIGHT, SCREEN_WIDTH,
};

mod commands;

psp::module_kernel!("OASIS_OS", 1, 0);

// ---------------------------------------------------------------------------
// Theme constants (matching oasis-core/src/theme.rs)
// ---------------------------------------------------------------------------

// Bar geometry.
const STATUSBAR_H: u32 = 18;
const TAB_ROW_H: u32 = 14;
const BOTTOMBAR_H: u32 = 32;
const BOTTOMBAR_Y: i32 = (SCREEN_HEIGHT - BOTTOMBAR_H) as i32;
const CONTENT_TOP: u32 = STATUSBAR_H + TAB_ROW_H;
const CONTENT_H: u32 = SCREEN_HEIGHT - CONTENT_TOP - BOTTOMBAR_H;

// Two-layer bottom bar row constants.
const BOTTOM_UPPER_Y: i32 = BOTTOMBAR_Y;
const BOTTOM_UPPER_H: u32 = 16;
const BOTTOM_LOWER_Y: i32 = BOTTOMBAR_Y + BOTTOM_UPPER_H as i32;

// Font metrics.
const CHAR_W: i32 = 8;

// Status bar tab layout (4 beveled chrome tabs).
const TAB_START_X: i32 = 34;
const TAB_W: i32 = 40;
const TAB_H: i32 = 12;
const TAB_GAP: i32 = 3;

// Bottom bar layout.
const PIPE_GAP: i32 = 5;
const R_HINT_W: i32 = 28;

// Icon theme (compact to fit 4 rows).
const ICON_W: u32 = 42;
const ICON_H: u32 = 40;
const ICON_STRIPE_H: u32 = 8;
const ICON_FOLD_SIZE: u32 = 7;
const ICON_GFX_H: u32 = 16;
const ICON_GFX_PAD: u32 = 3;
const ICON_LABEL_PAD: i32 = 1;

// Dashboard grid (2 columns, 4 rows = 8 icons, no pagination).
const GRID_COLS: usize = 2;
const GRID_ROWS: usize = 4;
const GRID_PAD_X: i32 = 16;
const GRID_PAD_Y: i32 = 2;
const CELL_W: i32 = 110;
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
const CATEGORY_CLR: Color = Color::rgb(220, 220, 220);
// Colors -- bottom bar.
const URL_CLR: Color = Color::rgb(200, 200, 200);
const USB_CLR: Color = Color::rgb(140, 140, 140);
const MEDIA_ACTIVE: Color = Color::WHITE;
const MEDIA_INACTIVE: Color = Color::rgb(170, 170, 170);
const PIPE_CLR: Color = Color::rgba(255, 255, 255, 60);
const R_HINT_CLR: Color = Color::rgba(255, 255, 255, 140);
// Colors -- visualizer & transport.
const VIZ_BAR_CLR: Color = Color::rgba(120, 60, 160, 200);
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
const HIGHLIGHT_CLR: Color = Color::rgba(255, 255, 255, 50);

// Terminal.
const MAX_OUTPUT_LINES: usize = 20;
const TERM_INPUT_Y: i32 = BOTTOMBAR_Y - 14;

// File manager.
const FM_VISIBLE_ROWS: usize = 18;
const FM_ROW_H: i32 = 10;
const FM_START_Y: i32 = CONTENT_TOP as i32 + 14;

// Desktop mode taskbar.
const TASKBAR_H: u32 = 12;

// ---------------------------------------------------------------------------
// App entries (matching oasis-core FALLBACK_COLORS)
// ---------------------------------------------------------------------------

struct AppEntry {
    id: &'static str,
    title: &'static str,
    color: Color,
}

static APPS: &[AppEntry] = &[
    AppEntry { id: "filemgr",  title: "File Manager", color: Color::rgb(70, 130, 180) },
    AppEntry { id: "settings", title: "Settings",     color: Color::rgb(60, 179, 113) },
    AppEntry { id: "network",  title: "Network",      color: Color::rgb(218, 165, 32) },
    AppEntry { id: "terminal", title: "Terminal",     color: Color::rgb(178, 102, 178) },
    AppEntry { id: "music",    title: "Music Player", color: Color::rgb(205, 92, 92) },
    AppEntry { id: "photos",   title: "Photo Viewer", color: Color::rgb(100, 149, 237) },
    AppEntry { id: "packages", title: "Package Mgr",  color: Color::rgb(70, 130, 180) },
    AppEntry { id: "sysmon",   title: "Sys Monitor",  color: Color::rgb(60, 179, 113) },
];

// ---------------------------------------------------------------------------
// Top tabs (cycled with L trigger)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum TopTab {
    Mso,
    Umd,
    Mod,
    Net,
}

impl TopTab {
    fn label(self) -> &'static str {
        match self {
            Self::Mso => "MSO",
            Self::Umd => "UMD",
            Self::Mod => "MOD",
            Self::Net => "NET",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Mso => Self::Umd,
            Self::Umd => Self::Mod,
            Self::Mod => Self::Net,
            Self::Net => Self::Mso,
        }
    }

    const ALL: &[TopTab] = &[TopTab::Mso, TopTab::Umd, TopTab::Mod, TopTab::Net];
}

// ---------------------------------------------------------------------------
// Media tabs (cycled with R trigger)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum MediaTab {
    None,
    Audio,
    Video,
    Image,
    File,
}

impl MediaTab {
    fn next(self) -> Self {
        match self {
            Self::None => Self::Audio,
            Self::Audio => Self::Video,
            Self::Video => Self::Image,
            Self::Image => Self::File,
            Self::File => Self::None,
        }
    }

    const LABELS: &[&str] = &["AUDIO", "VIDEO", "IMAGE", "FILE"];
    const TABS: &[MediaTab] = &[MediaTab::Audio, MediaTab::Video, MediaTab::Image, MediaTab::File];
}

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
fn show_boot_screen(
    backend: &mut PspBackend,
    status: &str,
    progress: u32,
) {
    use oasis_backend_psp::render::{FONT_ATLAS_W, FONT_ATLAS_H};
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
    backend.fill_rect_inner(
        bar_x, bar_y, bar_w, bar_h, Color::rgba(40, 40, 60, 200),
    );
    let fill_w = (bar_w * progress.min(100)) / 100;
    if fill_w > 0 {
        backend.fill_rect_inner(
            bar_x, bar_y, fill_w, bar_h, Color::rgb(80, 140, 220),
        );
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
        batch.draw_rect(cx, title_y as f32, 8.0, 8.0, u0, v0, u0 + 8.0, v0 + 8.0, white_abgr);
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
        batch.draw_rect(cx, status_y as f32, 8.0, 8.0, u0, v0, u0 + 8.0, v0 + 8.0, status_abgr);
        cx += 8.0;
    }

    // Single texture bind + single flush for all text.
    // SAFETY: Within an active GU display list; font atlas pointer is
    // valid and non-null (set during backend.init()).
    unsafe {
        use std::ffi::c_void;
        use psp::sys::{
            self, MipmapLevel, TextureColorComponent, TextureEffect,
            TexturePixelFormat,
        };
        let uncached_atlas =
            psp::cache::UncachedPtr::from_cached_addr(backend.font_atlas())
                .as_ptr() as *const c_void;
        sys::sceGuTexMode(TexturePixelFormat::Psm8888, 0, 0, 0);
        sys::sceGuTexImage(
            MipmapLevel::None,
            FONT_ATLAS_W as i32,
            FONT_ATLAS_H as i32,
            FONT_ATLAS_W as i32,
            uncached_atlas,
        );
        sys::sceGuTexFunc(
            TextureEffect::Modulate,
            TextureColorComponent::Rgba,
        );
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

    // Register exception handler (kernel mode) for crash diagnostics.
    oasis_backend_psp::register_exception_handler();
    show_boot_screen(&mut backend, "Loading config...", 25);

    // Load persistent configuration.
    let mut config = psp::config::Config::load(CONFIG_PATH)
        .unwrap_or_else(|_| psp::config::Config::new());

    // Set clock speed from config (default: max 333MHz).
    let clock_mhz = config.get_i32("clock_mhz").unwrap_or(333);
    let bus_mhz = config.get_i32("bus_mhz").unwrap_or(166);
    oasis_backend_psp::set_clock(clock_mhz, bus_mhz);

    // Query static hardware info (kernel mode, once at startup).
    let sysinfo = SystemInfo::query();
    show_boot_screen(&mut backend, "Generating textures...", 40);

    // Load wallpaper texture.
    let wallpaper_data = oasis_backend_psp::generate_gradient(SCREEN_WIDTH, SCREEN_HEIGHT);
    let wallpaper_tex = backend
        .load_texture_inner(SCREEN_WIDTH, SCREEN_HEIGHT, &wallpaper_data)
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
    let mut top_tab = TopTab::Mso;
    let mut media_tab = MediaTab::None;
    let mut icons_hidden: bool = false;
    let mut viz_frame: u32 = 0;

    // Terminal state.
    let vol_info = backend.volatile_mem_info();
    let mut term_lines: Vec<String> = vec![
        String::from("OASIS_OS v0.1.0 [PSP] (kernel mode)"),
        format!(
            "CPU: {}MHz  Bus: {}MHz  ME: {}MHz",
            sysinfo.cpu_mhz, sysinfo.bus_mhz, sysinfo.me_mhz,
        ),
        if let Some((total, _)) = vol_info {
            format!("Texture cache: {} KB volatile RAM claimed", total / 1024)
        } else {
            String::from("Texture cache: main heap only (PSP-1000)")
        },
        String::from("Type 'help' for commands. Start=terminal, Select=desktop."),
        String::new(),
    ];
    let mut term_input = String::new();

    // Try to restore previous terminal history from save data (silent).
    if let Ok(saved) = commands::load_terminal_history() {
        if !saved.is_empty() {
            term_lines.push(String::from("(restored previous session)"));
            term_lines.extend(saved);
            term_lines.push(String::new());
        }
    }

    // File manager state.
    let mut fm_path = String::from("ms0:/");
    let mut fm_entries: Vec<FileEntry> = Vec::new();
    let mut fm_selected: usize = 0;
    let mut fm_scroll: usize = 0;
    let mut fm_loaded = false;

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

    // Single background worker thread handles both audio and file I/O.
    let (audio, io) = oasis_backend_psp::spawn_workers();
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
                }
                IoResponse::Error { path, msg } => {
                    term_lines.push(format!("I/O error: {} - {}", path, msg));
                    pv_loading = false;
                }
                IoResponse::FileReady { .. } => {}
                IoResponse::HttpDone { tag: _, status_code, body } => {
                    let preview = String::from_utf8_lossy(
                        &body[..body.len().min(256)],
                    );
                    term_lines.push(format!(
                        "HTTP {status_code} ({} bytes): {preview}",
                        body.len(),
                    ));
                }
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
                    }
                    InputEvent::ButtonRelease(Button::Confirm) => {
                        _confirm_held = false;
                        let (cx, cy) = backend.cursor_pos();
                        let ptr_event = InputEvent::PointerRelease { x: cx, y: cy };
                        wm.handle_input(&ptr_event, &mut sdi);
                    }
                    InputEvent::CursorMove { x, y } => {
                        // Always forward cursor moves when in Desktop mode.
                        let move_event = InputEvent::CursorMove { x: *x, y: *y };
                        wm.handle_input(&move_event, &mut sdi);
                    }
                    InputEvent::ButtonPress(Button::Select) => {
                        // Toggle back to Classic mode.
                        app_mode = AppMode::Classic;
                        classic_view = ClassicView::Dashboard;
                    }
                    InputEvent::ButtonPress(Button::Triangle) => {
                        // Open app launcher: cycle through apps and open as windows.
                        let idx = page * ICONS_PER_PAGE + selected;
                        if idx < APPS.len() {
                            let app = &APPS[idx];
                            open_app_window(&mut wm, &mut sdi, app.id, app.title);
                        }
                    }
                    InputEvent::ButtonPress(Button::Start) => {
                        // Toggle terminal window.
                        open_app_window(&mut wm, &mut sdi, "terminal", "Terminal");
                    }
                    // Dashboard navigation works in Desktop mode too.
                    InputEvent::ButtonPress(Button::Up) => {
                        if selected >= GRID_COLS {
                            selected -= GRID_COLS;
                        }
                    }
                    InputEvent::ButtonPress(Button::Down) => {
                        let page_start = page * ICONS_PER_PAGE;
                        let page_count =
                            APPS.len().saturating_sub(page_start).min(ICONS_PER_PAGE);
                        if selected + GRID_COLS < page_count {
                            selected += GRID_COLS;
                        }
                    }
                    InputEvent::ButtonPress(Button::Left) => {
                        let page_start = page * ICONS_PER_PAGE;
                        let page_count =
                            APPS.len().saturating_sub(page_start).min(ICONS_PER_PAGE);
                        if selected == 0 {
                            selected = if page_count > 0 { page_count - 1 } else { 0 };
                        } else {
                            selected -= 1;
                        }
                    }
                    InputEvent::ButtonPress(Button::Right) => {
                        let page_start = page * ICONS_PER_PAGE;
                        let page_count =
                            APPS.len().saturating_sub(page_start).min(ICONS_PER_PAGE);
                        selected = (selected + 1) % page_count.max(1);
                    }
                    InputEvent::TriggerPress(Trigger::Left) => {
                        top_tab = top_tab.next();
                    }
                    InputEvent::TriggerPress(Trigger::Right) => {
                        media_tab = media_tab.next();
                    }
                    InputEvent::Quit => return,
                    _ => {}
                }
                continue; // Skip classic input handling.
            }

            // -- Classic mode input --
            match event {
                InputEvent::Quit => return,

                InputEvent::ButtonPress(Button::Start) => {
                    classic_view = match classic_view {
                        ClassicView::Dashboard => ClassicView::Terminal,
                        ClassicView::Terminal => ClassicView::Dashboard,
                        ClassicView::FileManager => ClassicView::Dashboard,
                        ClassicView::PhotoViewer => ClassicView::Dashboard,
                        ClassicView::MusicPlayer => ClassicView::Dashboard,
                    };
                }

                InputEvent::ButtonPress(Button::Select) if classic_view == ClassicView::Dashboard => {
                    // Toggle to Desktop mode.
                    app_mode = AppMode::Desktop;
                }

                // -- Dashboard input --
                InputEvent::ButtonPress(Button::Up) if classic_view == ClassicView::Dashboard => {
                    if selected >= GRID_COLS {
                        selected -= GRID_COLS;
                    }
                }
                InputEvent::ButtonPress(Button::Down) if classic_view == ClassicView::Dashboard => {
                    let page_start = page * ICONS_PER_PAGE;
                    let page_count =
                        APPS.len().saturating_sub(page_start).min(ICONS_PER_PAGE);
                    if selected + GRID_COLS < page_count {
                        selected += GRID_COLS;
                    }
                }
                InputEvent::ButtonPress(Button::Left) if classic_view == ClassicView::Dashboard => {
                    let page_start = page * ICONS_PER_PAGE;
                    let page_count =
                        APPS.len().saturating_sub(page_start).min(ICONS_PER_PAGE);
                    if selected == 0 {
                        selected = if page_count > 0 { page_count - 1 } else { 0 };
                    } else {
                        selected -= 1;
                    }
                }
                InputEvent::ButtonPress(Button::Right) if classic_view == ClassicView::Dashboard => {
                    let page_start = page * ICONS_PER_PAGE;
                    let page_count =
                        APPS.len().saturating_sub(page_start).min(ICONS_PER_PAGE);
                    selected = (selected + 1) % page_count.max(1);
                }
                InputEvent::ButtonPress(Button::Confirm) if classic_view == ClassicView::Dashboard => {
                    let idx = page * ICONS_PER_PAGE + selected;
                    if idx < APPS.len() {
                        let app = &APPS[idx];
                        match app.title {
                            "Terminal" => {
                                classic_view = ClassicView::Terminal;
                            }
                            "File Manager" => {
                                classic_view = ClassicView::FileManager;
                                fm_loaded = false;
                            }
                            "Photo Viewer" => {
                                classic_view = ClassicView::PhotoViewer;
                                pv_viewing = false;
                                pv_loaded = false;
                            }
                            "Music Player" => {
                                classic_view = ClassicView::MusicPlayer;
                                mp_loaded = false;
                            }
                            _ => {
                                term_lines.push(format!("Launched: {}", app.title));
                            }
                        }
                    }
                }
                InputEvent::ButtonPress(Button::Cancel) if classic_view == ClassicView::Dashboard => {
                    icons_hidden = !icons_hidden;
                }

                // Trigger cycling.
                InputEvent::TriggerPress(Trigger::Left) if classic_view == ClassicView::Dashboard => {
                    top_tab = top_tab.next();
                }
                InputEvent::TriggerPress(Trigger::Right) if classic_view == ClassicView::Dashboard => {
                    media_tab = media_tab.next();
                }

                // -- Terminal input --
                InputEvent::ButtonPress(Button::Confirm) if classic_view == ClassicView::Terminal => {
                    let cmd = term_input.clone();
                    term_lines.push(format!("> {}", cmd));
                    // Handle SFX commands via worker thread.
                    let output = match cmd.trim() {
                        "sfx click" => {
                            audio.send(AudioCmd::PlaySfx(SfxId::Click));
                            vec!["SFX: click".into()]
                        }
                        "sfx nav" => {
                            audio.send(AudioCmd::PlaySfx(SfxId::Navigate));
                            vec!["SFX: navigate".into()]
                        }
                        "sfx error" => {
                            audio.send(AudioCmd::PlaySfx(SfxId::Error));
                            vec!["SFX: error".into()]
                        }
                        "save" => {
                            match commands::save_terminal_history(&term_lines) {
                                Ok(()) => vec!["State saved.".into()],
                                Err(e) => vec![format!("Save failed: {e}")],
                            }
                        }
                        "load" => {
                            match commands::load_terminal_history() {
                                Ok(lines) => {
                                    term_lines.clear();
                                    term_lines.extend(lines);
                                    vec!["State restored.".into()]
                                }
                                Err(e) => vec![format!("Load failed: {e}")],
                            }
                        }
                        "usb mount" => {
                            if usb_storage.is_some() {
                                vec!["USB storage already active.".into()]
                            } else {
                                match psp::usb::start_bus() {
                                    Ok(()) => match psp::usb::UsbStorageMode::activate() {
                                        Ok(handle) => {
                                            usb_storage = Some(handle);
                                            vec!["USB storage mode active. Connect cable to PC.".into()]
                                        }
                                        Err(e) => vec![format!("USB activate failed: {e}")],
                                    },
                                    Err(e) => vec![format!("USB bus start failed: {e}")],
                                }
                            }
                        }
                        "usb unmount" | "usb eject" => {
                            if usb_storage.take().is_some() {
                                vec!["USB storage mode deactivated.".into()]
                            } else {
                                vec!["USB storage not active.".into()]
                            }
                        }
                        "usb" | "usb status" => {
                            let connected = psp::usb::is_connected();
                            let established = psp::usb::is_established();
                            let active = usb_storage.is_some();
                            vec![
                                format!("USB cable: {}", if connected { "connected" } else { "disconnected" }),
                                format!("Storage mode: {}", if active { "ACTIVE" } else { "inactive" }),
                                format!("Host mounted: {}", if established { "yes" } else { "no" }),
                            ]
                        }
                        _ if cmd.trim().starts_with("play ") => {
                            let path = cmd.trim().strip_prefix("play ").unwrap().trim();
                            audio.send(AudioCmd::LoadAndPlay(path.to_string()));
                            mp_file_name = path.to_string();
                            vec![format!("Playing: {}", path)]
                        }
                        "pause" => {
                            audio.send(AudioCmd::Pause);
                            vec!["Paused.".into()]
                        }
                        "resume" => {
                            audio.send(AudioCmd::Resume);
                            vec!["Resumed.".into()]
                        }
                        "stop" => {
                            audio.send(AudioCmd::Stop);
                            vec!["Stopped.".into()]
                        }
                        _ => commands::execute_command(&cmd, &mut config),
                    };
                    for line in output {
                        term_lines.push(line);
                    }
                    term_input.clear();
                    while term_lines.len() > 200 {
                        term_lines.remove(0);
                    }
                }
                InputEvent::ButtonPress(Button::Square) if classic_view == ClassicView::Terminal => {
                    // Open PSP on-screen keyboard for command input.
                    match psp::osk::OskBuilder::new("Enter command")
                        .max_chars(256)
                        .initial_text(&term_input)
                        .show()
                    {
                        Ok(Some(text)) => {
                            term_input = text;
                        }
                        Ok(None) | Err(_) => {} // Cancelled or unsupported (PPSSPP)
                    }
                }
                InputEvent::ButtonPress(Button::Up) if classic_view == ClassicView::Terminal => {
                    term_lines.push(String::from("> help"));
                    let output = commands::execute_command("help", &mut config);
                    for line in output {
                        term_lines.push(line);
                    }
                }
                InputEvent::ButtonPress(Button::Down) if classic_view == ClassicView::Terminal => {
                    term_lines.push(String::from("> status"));
                    let output = commands::execute_command("status", &mut config);
                    for line in output {
                        term_lines.push(line);
                    }
                }

                // -- File manager input --
                InputEvent::ButtonPress(Button::Up) if classic_view == ClassicView::FileManager => {
                    if fm_selected > 0 {
                        fm_selected -= 1;
                        if fm_selected < fm_scroll {
                            fm_scroll = fm_selected;
                        }
                    }
                }
                InputEvent::ButtonPress(Button::Down) if classic_view == ClassicView::FileManager => {
                    if fm_selected + 1 < fm_entries.len() {
                        fm_selected += 1;
                        if fm_selected >= fm_scroll + FM_VISIBLE_ROWS {
                            fm_scroll = fm_selected - FM_VISIBLE_ROWS + 1;
                        }
                    }
                }
                InputEvent::ButtonPress(Button::Confirm) if classic_view == ClassicView::FileManager => {
                    if fm_selected < fm_entries.len() && fm_entries[fm_selected].is_dir {
                        let dir_name = fm_entries[fm_selected].name.clone();
                        if fm_path.ends_with('/') {
                            fm_path = format!("{}{}", fm_path, dir_name);
                        } else {
                            fm_path = format!("{}/{}", fm_path, dir_name);
                        }
                        fm_loaded = false;
                    }
                }
                InputEvent::ButtonPress(Button::Cancel) if classic_view == ClassicView::FileManager => {
                    if let Some(pos) = fm_path.rfind('/') {
                        if pos > 0 && !fm_path[..pos].ends_with(':') {
                            fm_path.truncate(pos);
                        } else if fm_path.len() > pos + 1 {
                            fm_path.truncate(pos + 1);
                        } else {
                            classic_view = ClassicView::Dashboard;
                        }
                        fm_loaded = false;
                    } else {
                        classic_view = ClassicView::Dashboard;
                    }
                }
                InputEvent::ButtonPress(Button::Square) if classic_view == ClassicView::FileManager => {
                    // Delete selected file with confirmation dialog.
                    if fm_selected < fm_entries.len() && !fm_entries[fm_selected].is_dir {
                        let name = &fm_entries[fm_selected].name;
                        let msg = format!("Delete {}?", name);
                        match psp::dialog::confirm_dialog(&msg) {
                            Ok(psp::dialog::DialogResult::Confirm) => {
                                let full_path = if fm_path.ends_with('/') {
                                    format!("{}{}", fm_path, name)
                                } else {
                                    format!("{}/{}", fm_path, name)
                                };
                                match psp::io::remove_file(&full_path) {
                                    Ok(()) => {
                                        term_lines.push(format!(
                                            "Deleted: {}", full_path
                                        ));
                                        fm_loaded = false;
                                    }
                                    Err(e) => {
                                        let _ = psp::dialog::error_dialog(
                                            e.0 as u32,
                                        );
                                    }
                                }
                            }
                            _ => {} // Cancelled or closed
                        }
                    }
                }
                InputEvent::ButtonPress(Button::Triangle) if classic_view == ClassicView::FileManager => {
                    classic_view = ClassicView::Dashboard;
                }

                // -- Photo viewer input --
                InputEvent::ButtonPress(Button::Up) if classic_view == ClassicView::PhotoViewer && !pv_viewing => {
                    if pv_selected > 0 {
                        pv_selected -= 1;
                        if pv_selected < pv_scroll {
                            pv_scroll = pv_selected;
                        }
                    }
                }
                InputEvent::ButtonPress(Button::Down) if classic_view == ClassicView::PhotoViewer && !pv_viewing => {
                    if pv_selected + 1 < pv_entries.len() {
                        pv_selected += 1;
                        if pv_selected >= pv_scroll + FM_VISIBLE_ROWS {
                            pv_scroll = pv_selected - FM_VISIBLE_ROWS + 1;
                        }
                    }
                }
                InputEvent::ButtonPress(Button::Confirm) if classic_view == ClassicView::PhotoViewer && !pv_viewing => {
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
                }
                InputEvent::ButtonPress(Button::Cancel) if classic_view == ClassicView::PhotoViewer => {
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
                }
                InputEvent::ButtonPress(Button::Triangle) if classic_view == ClassicView::PhotoViewer => {
                    if pv_viewing {
                        pv_viewing = false;
                    } else {
                        classic_view = ClassicView::Dashboard;
                    }
                }

                // -- Music player input --
                InputEvent::ButtonPress(Button::Up) if classic_view == ClassicView::MusicPlayer && !audio.is_playing() => {
                    if mp_selected > 0 {
                        mp_selected -= 1;
                        if mp_selected < mp_scroll {
                            mp_scroll = mp_selected;
                        }
                    }
                }
                InputEvent::ButtonPress(Button::Down) if classic_view == ClassicView::MusicPlayer && !audio.is_playing() => {
                    if mp_selected + 1 < mp_entries.len() {
                        mp_selected += 1;
                        if mp_selected >= mp_scroll + FM_VISIBLE_ROWS {
                            mp_scroll = mp_selected - FM_VISIBLE_ROWS + 1;
                        }
                    }
                }
                InputEvent::ButtonPress(Button::Confirm) if classic_view == ClassicView::MusicPlayer => {
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
                }
                InputEvent::ButtonPress(Button::Square) if classic_view == ClassicView::MusicPlayer => {
                    audio.send(AudioCmd::Stop);
                }
                InputEvent::ButtonPress(Button::Cancel) if classic_view == ClassicView::MusicPlayer => {
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
                }
                InputEvent::ButtonPress(Button::Triangle) if classic_view == ClassicView::MusicPlayer => {
                    classic_view = ClassicView::Dashboard;
                    // Audio keeps playing in background.
                }

                _ => {}
            }
        }

        // -- Render --
        let status = StatusBarInfo::poll();
        let fps = frame_timer.fps();
        let usb_active = usb_storage.is_some();

        backend.clear_inner(Color::BLACK);

        // Wallpaper.
        backend.blit_inner(wallpaper_tex, 0, 0, SCREEN_WIDTH, SCREEN_HEIGHT);

        match app_mode {
            AppMode::Classic => {
                // Lazy-load directory entries for browser modes.
                if classic_view == ClassicView::FileManager && !fm_loaded {
                    fm_entries = oasis_backend_psp::list_directory(&fm_path);
                    fm_selected = 0;
                    fm_scroll = 0;
                    fm_loaded = true;
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
                        if !icons_hidden {
                            draw_dashboard(&mut backend, selected, page);
                        }
                    }
                    ClassicView::Terminal => {
                        backend.force_bitmap_font = true;
                        draw_terminal(&mut backend, &term_lines, &term_input);
                        backend.force_bitmap_font = false;
                    }
                    ClassicView::FileManager => {
                        backend.force_bitmap_font = true;
                        draw_file_manager(
                            &mut backend,
                            &fm_path,
                            &fm_entries,
                            fm_selected,
                            fm_scroll,
                        );
                        backend.force_bitmap_font = false;
                    }
                    ClassicView::PhotoViewer => {
                        if pv_viewing {
                            draw_photo_view(&mut backend, pv_tex, pv_img_w, pv_img_h);
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
                        }
                    }
                    ClassicView::MusicPlayer => {
                        if audio.is_playing() {
                            draw_music_player_threaded(&mut backend, &mp_file_name, &audio);
                        } else {
                            draw_music_browser(
                                &mut backend,
                                &mp_path,
                                &mp_entries,
                                mp_selected,
                                mp_scroll,
                            );
                        }
                    }
                }
            }

            AppMode::Desktop => {
                // Draw dashboard icons behind windows.
                if !icons_hidden {
                    draw_dashboard(&mut backend, selected, page);
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
                let _ = wm.draw_with_clips(
                    &sdi,
                    &mut backend,
                    |window_id, cx, cy, cw, ch, be| {
                        // Downcast back to PspBackend for direct calls.
                        // Since draw_with_clips passes &mut dyn SdiBackend, we use
                        // the trait methods here (which return Result).
                        match window_id {
                            "terminal" => {
                                draw_terminal_windowed(
                                    &term_lines, &term_input, cx, cy, cw, ch, be,
                                )
                            }
                            "filemgr" => {
                                draw_filemgr_windowed(
                                    &fm_path, &fm_entries, fm_selected, fm_scroll, cx, cy,
                                    cw, ch, be,
                                )
                            }
                            "photos" => {
                                draw_photos_windowed(
                                    pv_tex, pv_img_w, pv_img_h, pv_viewing, cx, cy, cw, ch,
                                    be,
                                )
                            }
                            "music" => {
                                draw_music_windowed(
                                    &mp_file_name, &audio, cx, cy, cw, ch, be,
                                )
                            }
                            "settings" => {
                                draw_settings_windowed(
                                    settings_clock, settings_bus, current_vol,
                                    cx, cy, cw, ch, be,
                                )
                            }
                            "network" => {
                                draw_network_windowed(
                                    &status, cx, cy, cw, ch, be,
                                )
                            }
                            "sysmon" => {
                                draw_sysmon_windowed(
                                    &status, &sysinfo, fps, free_kb, max_blk_kb,
                                    current_vol, usb_active,
                                    cx, cy, cw, ch, be,
                                )
                            }
                            _ => Ok(()),
                        }
                    },
                );

                backend.force_bitmap_font = false;

                // Desktop mode taskbar at bottom.
                draw_desktop_taskbar(&mut backend, &wm);
            }
        }

        // Status bar + bottom bar (always visible, drawn on top).
        draw_status_bar(&mut backend, top_tab, &status, &sysinfo);
        draw_bottom_bar(&mut backend, media_tab, &audio, viz_frame, &status);
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
        y: Some(STATUSBAR_H as i32 + TAB_ROW_H as i32 + 2),
        width: 300,
        height: 180,
        window_type: WindowType::AppWindow,
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
        }
        WmEvent::ContentClick(id, lx, ly) => {
            term_lines.push(format!("[WM] Click in {}: ({}, {})", id, lx, ly));
        }
        WmEvent::DesktopClick(x, y) => {
            if let Some(idx) = hit_test_dashboard_icon(*x, *y, page) {
                if idx < APPS.len() {
                    open_app_window(wm, sdi, APPS[idx].id, APPS[idx].title);
                }
            }
        }
        _ => {}
    }
}

/// Draw the desktop mode taskbar showing open windows.
fn draw_desktop_taskbar(backend: &mut PspBackend, wm: &WindowManager) {
    let bar_y = BOTTOMBAR_Y;
    backend.fill_rect_inner(0, bar_y, SCREEN_WIDTH, TASKBAR_H, Color::rgba(0, 0, 0, 160));
    backend.fill_rect_inner(0, bar_y, SCREEN_WIDTH, 1, Color::rgba(255, 255, 255, 40));

    let active_id = wm.active_window();
    let mut tx = 4i32;

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
                backend.fill_rect_inner(tx - 2, bar_y + 1, label_w, TASKBAR_H - 2, Color::rgba(60, 90, 160, 140));
            }
            backend.draw_text_inner(app.title, tx + 2, bar_y, 8, label_clr);
            tx += app.title.len() as i32 * 8 + 12;
        }
    }
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
    be.draw_text(&prompt, cx + 2, cy + ch as i32 - 12, 8, Color::rgb(0, 255, 0))?;
    Ok(())
}

fn draw_filemgr_windowed(
    path: &str,
    entries: &[FileEntry],
    selected: usize,
    scroll: usize,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    be: &mut dyn SdiBackend,
) -> oasis_backend_psp::OasisResult<()> {
    be.fill_rect(cx, cy, cw, ch, Color::rgba(0, 0, 0, 200))?;
    be.draw_text(path, cx + 2, cy + 2, 8, Color::rgb(100, 200, 255))?;

    let max_rows = ((ch as i32 - 14) / FM_ROW_H) as usize;
    let end = (scroll + max_rows).min(entries.len());
    for i in scroll..end {
        let entry = &entries[i];
        let row = (i - scroll) as i32;
        let y = cy + 14 + row * FM_ROW_H;
        if i == selected {
            be.fill_rect(cx, y - 1, cw, FM_ROW_H as u32, Color::rgba(80, 120, 200, 100))?;
        }
        let (prefix, clr) = if entry.is_dir {
            ("[D]", Color::rgb(255, 220, 80))
        } else {
            ("[F]", Color::rgb(180, 180, 180))
        };
        be.draw_text(prefix, cx + 2, y, 8, clr)?;
        let name_clr = if entry.is_dir {
            Color::rgb(120, 220, 255)
        } else {
            Color::WHITE
        };
        be.draw_text(&entry.name, cx + 28, y, 8, name_clr)?;
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
        be.draw_text("Select photo from browser", cx + 4, cy + 4, 8, Color::rgb(160, 160, 160))?;
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
        let status = if audio.is_paused() { "PAUSED" } else { "PLAYING" };
        let status_clr = if audio.is_paused() {
            Color::rgb(255, 200, 80)
        } else {
            Color::rgb(120, 255, 120)
        };
        let status_x = center_x - (status.len() as i32 * 8) / 2;
        be.draw_text(status, status_x, cy + ch as i32 / 2, 8, status_clr)?;
    } else {
        be.draw_text("No track loaded", cx + 4, cy + 4, 8, Color::rgb(160, 160, 160))?;
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
        be.draw_text(&format!("{}%", status.battery_percent), vx, y, 8, Color::WHITE)?;
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
    be.draw_text("SYSTEM MONITOR", cx + 4, cy + 2, 8, Color::rgb(60, 179, 113))?;
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
        vx, y, 8, val,
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

fn draw_dashboard(backend: &mut PspBackend, selected: usize, page: usize) {
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

        // Label below icon (centered under cell).
        let label_y = iy + ICON_H as i32 + ICON_LABEL_PAD;
        let text_width = (app.title.len() as i32) * CHAR_W;
        let label_x = cell_x + (CELL_W - text_width) / 2;
        backend.draw_text_inner(app.title, label_x, label_y, 8, LABEL_CLR);
    }

    // Cursor highlight around selected icon.
    if page_count > 0 && selected < page_count {
        let sel_col = (selected % GRID_COLS) as i32;
        let sel_row = (selected / GRID_COLS) as i32;
        let cell_x = GRID_PAD_X + sel_col * CELL_W;
        let cell_y = CONTENT_TOP as i32 + GRID_PAD_Y + sel_row * CELL_H;
        let ix = cell_x + (CELL_W - ICON_W as i32) / 2;
        let iy = cell_y + 1;
        backend.fill_rect_inner(
            ix - CURSOR_PAD,
            iy - CURSOR_PAD,
            ICON_W + CURSOR_PAD as u32 * 2,
            ICON_H + CURSOR_PAD as u32 * 2,
            HIGHLIGHT_CLR,
        );
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
    let c = app.color;
    let gfx_color = Color::rgba(
        c.r.saturating_add(30),
        c.g.saturating_add(10),
        c.b.saturating_add(30),
        200,
    );
    backend.fill_rect_inner(
        ix + ICON_GFX_PAD as i32,
        iy + ICON_STRIPE_H as i32 + 3,
        gfx_w,
        ICON_GFX_H,
        gfx_color,
    );
}

// ---------------------------------------------------------------------------
// Status bar rendering
// ---------------------------------------------------------------------------

fn draw_status_bar(
    backend: &mut PspBackend,
    active_tab: TopTab,
    status: &StatusBarInfo,
    sysinfo: &SystemInfo,
) {
    backend.fill_rect_inner(0, 0, SCREEN_WIDTH, STATUSBAR_H, STATUSBAR_BG);
    backend.fill_rect_inner(0, STATUSBAR_H as i32 - 1, SCREEN_WIDTH, 1, SEPARATOR);

    // -- Left side: battery percentage + CPU MHz + version --

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

    // CPU MHz with filled-square indicator (matching PSIX reference).
    let bat_w = bat_label.len() as i32 * CHAR_W;
    let mhz_x = 6 + bat_w + 8;
    // Draw small filled square indicator (4x4 px).
    backend.fill_rect_inner(mhz_x, 7, 5, 5, Color::WHITE);
    let mhz_label = format!("{} MHZ", sysinfo.cpu_mhz);
    backend.draw_text_inner(&mhz_label, mhz_x + 8, 5, 8, Color::WHITE);

    // Version string (centered-ish).
    let ver_label = "Version 0.1 Public";
    let ver_w = ver_label.len() as i32 * CHAR_W;
    let ver_x = (SCREEN_WIDTH as i32 - ver_w) / 2;
    backend.draw_text_inner(ver_label, ver_x, 5, 8, Color::WHITE);

    // -- Right side: time + full date --
    let date_label = format!(
        "{:02}:{:02} {} {}, {}",
        status.hour,
        status.minute,
        status.month_name(),
        status.day,
        status.year,
    );
    let date_w = date_label.len() as i32 * CHAR_W;
    let date_x = SCREEN_WIDTH as i32 - date_w - 6;
    backend.draw_text_inner(&date_label, date_x, 5, 8, Color::WHITE);

    // -- Tab row --
    backend.draw_text_inner("OSS", 6, STATUSBAR_H as i32 + 3, 8, CATEGORY_CLR);

    let tab_y = STATUSBAR_H as i32;
    for (i, tab) in TopTab::ALL.iter().enumerate() {
        let x = TAB_START_X + (i as i32) * (TAB_W + TAB_GAP);

        if *tab == active_tab {
            // Beveled 3D chrome bezel for active tab.
            draw_chrome_bezel(backend, x, tab_y, TAB_W as u32, TAB_H as u32);
        } else {
            // Subtle border-only for inactive tabs.
            let border = Color::rgba(200, 255, 200, 60);
            backend.fill_rect_inner(x, tab_y, TAB_W as u32, 1, border);
            backend.fill_rect_inner(x, tab_y + TAB_H - 1, TAB_W as u32, 1, border);
            backend.fill_rect_inner(x, tab_y, 1, TAB_H as u32, border);
            backend.fill_rect_inner(x + TAB_W - 1, tab_y, 1, TAB_H as u32, border);
        }

        let label = tab.label();
        let text_w = label.len() as i32 * CHAR_W;
        let tx = x + (TAB_W - text_w) / 2;
        let text_color = if *tab == active_tab {
            Color::WHITE
        } else {
            Color::rgb(160, 160, 160)
        };
        backend.draw_text_inner(label, tx.max(x + 2), tab_y + 2, 8, text_color);
    }
}

// ---------------------------------------------------------------------------
// Bottom bar rendering
// ---------------------------------------------------------------------------

fn draw_bottom_bar(
    backend: &mut PspBackend,
    active_media: MediaTab,
    audio: &AudioHandle,
    viz_frame: u32,
    status: &StatusBarInfo,
) {
    // Full 32px bottom bar background.
    backend.fill_rect_inner(0, BOTTOMBAR_Y, SCREEN_WIDTH, BOTTOMBAR_H, BAR_BG);
    backend.fill_rect_inner(0, BOTTOMBAR_Y, SCREEN_WIDTH, 1, SEPARATOR);

    // -- Upper row (y=BOTTOM_UPPER_Y, 16px): URL bezel | Visualizer | Media tabs bezel --

    // URL chrome bezel (left, 140px).
    let url_bx = 2i32;
    let url_bw = 140u32;
    let ubz_y = BOTTOM_UPPER_Y + 1;
    let ubz_h = BOTTOM_UPPER_H - 2;
    draw_chrome_bezel(backend, url_bx, ubz_y, url_bw, ubz_h);
    backend.draw_text_inner("HTTP://OASIS.LOCAL", 6, BOTTOM_UPPER_Y + 4, 8, URL_CLR);

    // Visualizer (center of upper row).
    draw_visualizer(backend, audio, viz_frame);

    // Media tabs chrome bezel (right).
    let labels_w: i32 =
        MediaTab::LABELS.iter().map(|l| l.len() as i32 * CHAR_W).sum();
    let pipes_w = (MediaTab::LABELS.len() as i32 - 1) * (PIPE_GAP * 2 + CHAR_W);
    let total_w = labels_w + pipes_w;
    let tabs_x = SCREEN_WIDTH as i32 - total_w - 8;

    let tab_bx = tabs_x - 4;
    let tab_bw = (total_w + 10) as u32;
    draw_chrome_bezel(backend, tab_bx, ubz_y, tab_bw, ubz_h);

    let mut cx = tabs_x;
    for (i, label) in MediaTab::LABELS.iter().enumerate() {
        let tab = MediaTab::TABS[i];
        let color = if tab == active_media {
            MEDIA_ACTIVE
        } else {
            MEDIA_INACTIVE
        };
        backend.draw_text_inner(label, cx, BOTTOM_UPPER_Y + 4, 8, color);
        cx += label.len() as i32 * CHAR_W;

        if i < MediaTab::LABELS.len() - 1 {
            cx += PIPE_GAP;
            backend.draw_text_inner("|", cx, BOTTOM_UPPER_Y + 4, 8, PIPE_CLR);
            cx += CHAR_W + PIPE_GAP;
        }
    }

    // -- Lower row (y=BOTTOM_LOWER_Y, 16px): L hint | transport | USB | battery bar | R hint --
    backend.fill_rect_inner(
        0, BOTTOM_LOWER_Y, SCREEN_WIDTH, 1,
        Color::rgba(255, 255, 255, 20),
    );

    // L hint.
    backend.draw_text_inner("<L", 4, BOTTOM_LOWER_Y + 4, 8, L_HINT_CLR);

    // Transport controls.
    draw_transport_controls(backend, audio);

    // USB label.
    backend.draw_text_inner("USB", 250, BOTTOM_LOWER_Y + 4, 8, USB_CLR);

    // Battery bar.
    draw_battery_bar(backend, status);

    // R hint.
    backend.draw_text_inner(
        "R>", SCREEN_WIDTH as i32 - R_HINT_W, BOTTOM_LOWER_Y + 4, 8, R_HINT_CLR,
    );
}

/// Draw animated music visualizer bars in center of upper bottom row.
fn draw_visualizer(
    backend: &mut PspBackend,
    audio: &AudioHandle,
    viz_frame: u32,
) {
    let total_viz_w = VIZ_BAR_COUNT * (VIZ_BAR_W + VIZ_BAR_GAP) - VIZ_BAR_GAP;
    let viz_x = (SCREEN_WIDTH as i32 - total_viz_w) / 2;
    let viz_base_y = BOTTOM_UPPER_Y + BOTTOM_UPPER_H as i32 - 2;
    let playing = audio.is_playing() && !audio.is_paused();

    for i in 0..VIZ_BAR_COUNT {
        let bar_h = if playing {
            // Animated bars using sinf with different frequencies/phases.
            let t = viz_frame as f32 * 0.12;
            let freq = 0.7 + (i as f32) * 0.3;
            let phase = (i as f32) * 1.1;
            let val = libm::sinf(t * freq + phase);
            let norm = (val + 1.0) * 0.5; // 0..1
            VIZ_BAR_MIN_H + ((VIZ_BAR_MAX_H - VIZ_BAR_MIN_H) as f32 * norm) as i32
        } else {
            VIZ_BAR_MIN_H
        };
        let bx = viz_x + i * (VIZ_BAR_W + VIZ_BAR_GAP);
        let by = viz_base_y - bar_h;
        backend.fill_rect_inner(bx, by, VIZ_BAR_W as u32, bar_h as u32, VIZ_BAR_CLR);
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
    let stop_clr = if !playing { TRANSPORT_ACTIVE } else { TRANSPORT_CLR };
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
        bar_x, bar_y + bar_h as i32 - 1, bar_w, 1,
        Color::rgba(200, 200, 200, 140),
    );
    backend.fill_rect_inner(bar_x, bar_y, 1, bar_h, Color::rgba(200, 200, 200, 140));
    backend.fill_rect_inner(
        bar_x + bar_w as i32 - 1, bar_y, 1, bar_h,
        Color::rgba(200, 200, 200, 140),
    );

    // Dark bg fill.
    backend.fill_rect_inner(
        bar_x + 1, bar_y + 1, bar_w - 2, bar_h - 2,
        Color::rgba(20, 20, 20, 180),
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

/// Draw a chrome/metallic bezel (fill + 4 edges).
fn draw_chrome_bezel(backend: &mut PspBackend, x: i32, y: i32, w: u32, h: u32) {
    backend.fill_rect_inner(x, y, w, h, BEZEL_FILL);
    backend.fill_rect_inner(x, y, w, 1, BEZEL_TOP);
    backend.fill_rect_inner(x, y + h as i32 - 1, w, 1, BEZEL_BOTTOM);
    backend.fill_rect_inner(x, y, 1, h, BEZEL_LEFT);
    backend.fill_rect_inner(x + w as i32 - 1, y, 1, h, BEZEL_RIGHT);
}

// ---------------------------------------------------------------------------
// Terminal rendering (classic full-screen)
// ---------------------------------------------------------------------------

fn draw_terminal(backend: &mut PspBackend, lines: &[String], input: &str) {
    let bg = Color::rgba(0, 0, 0, 180);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    let visible_start = if lines.len() > MAX_OUTPUT_LINES {
        lines.len() - MAX_OUTPUT_LINES
    } else {
        0
    };
    for (i, line) in lines[visible_start..].iter().enumerate() {
        let y = CONTENT_TOP as i32 + 4 + i as i32 * 9;
        if y > TERM_INPUT_Y - 12 {
            break;
        }
        backend.draw_text_inner(line, 4, y, 8, Color::rgb(0, 255, 0));
    }

    let prompt = format!("> {}_", input);
    backend.draw_text_inner(&prompt, 4, TERM_INPUT_Y, 8, Color::rgb(0, 255, 0));
}

// ---------------------------------------------------------------------------
// File manager rendering (classic full-screen)
// ---------------------------------------------------------------------------

fn draw_file_manager(
    backend: &mut PspBackend,
    path: &str,
    entries: &[FileEntry],
    selected: usize,
    scroll: usize,
) {
    let bg = Color::rgba(0, 0, 0, 200);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    backend.draw_text_inner(path, 4, CONTENT_TOP as i32 + 3, 8, Color::rgb(100, 200, 255));

    let header_y = CONTENT_TOP as i32 + 3;
    backend.draw_text_inner("SIZE", 400, header_y, 8, Color::rgb(160, 160, 160));

    backend.fill_rect_inner(
        0,
        FM_START_Y - 2,
        SCREEN_WIDTH,
        1,
        Color::rgba(255, 255, 255, 40),
    );

    if entries.is_empty() {
        backend.draw_text_inner("(empty directory)", 8, FM_START_Y, 8, Color::rgb(140, 140, 140));
        return;
    }

    let end = (scroll + FM_VISIBLE_ROWS).min(entries.len());
    for i in scroll..end {
        let entry = &entries[i];
        let row = (i - scroll) as i32;
        let y = FM_START_Y + row * FM_ROW_H;

        if i == selected {
            backend.fill_rect_inner(0, y - 1, SCREEN_WIDTH, FM_ROW_H as u32, Color::rgba(80, 120, 200, 100));
        }

        let (prefix, prefix_clr) = if entry.is_dir {
            ("[D]", Color::rgb(255, 220, 80))
        } else {
            ("[F]", Color::rgb(180, 180, 180))
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

    if entries.len() > FM_VISIBLE_ROWS {
        let ratio = selected as f32 / (entries.len() - 1).max(1) as f32;
        let track_h = CONTENT_H as i32 - 16;
        let dot_y = FM_START_Y + (ratio * track_h as f32) as i32;
        backend.fill_rect_inner(SCREEN_WIDTH as i32 - 4, dot_y, 3, 8, Color::rgba(255, 255, 255, 120));
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

    backend.draw_text_inner("PHOTO VIEWER", 4, CONTENT_TOP as i32 + 3, 8, Color::rgb(100, 149, 237));
    backend.draw_text_inner(path, 110, CONTENT_TOP as i32 + 3, 8, Color::rgb(160, 160, 160));

    backend.fill_rect_inner(0, FM_START_Y - 2, SCREEN_WIDTH, 1, Color::rgba(255, 255, 255, 40));

    if entries.is_empty() {
        backend.draw_text_inner("No images found (.jpg/.jpeg)", 8, FM_START_Y, 8, Color::rgb(140, 140, 140));
        return;
    }

    let end = (scroll + FM_VISIBLE_ROWS).min(entries.len());
    for i in scroll..end {
        let entry = &entries[i];
        let row = (i - scroll) as i32;
        let y = FM_START_Y + row * FM_ROW_H;

        if i == selected {
            backend.fill_rect_inner(0, y - 1, SCREEN_WIDTH, FM_ROW_H as u32, Color::rgba(80, 120, 200, 100));
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
}

fn draw_photo_view(
    backend: &mut PspBackend,
    tex: Option<TextureId>,
    img_w: u32,
    img_h: u32,
) {
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

    backend.draw_text_inner("MUSIC PLAYER", 4, CONTENT_TOP as i32 + 3, 8, Color::rgb(205, 92, 92));
    backend.draw_text_inner(path, 110, CONTENT_TOP as i32 + 3, 8, Color::rgb(160, 160, 160));

    backend.fill_rect_inner(0, FM_START_Y - 2, SCREEN_WIDTH, 1, Color::rgba(255, 255, 255, 40));

    if entries.is_empty() {
        backend.draw_text_inner("No MP3 files found", 8, FM_START_Y, 8, Color::rgb(140, 140, 140));
        return;
    }

    let end = (scroll + FM_VISIBLE_ROWS).min(entries.len());
    for i in scroll..end {
        let entry = &entries[i];
        let row = (i - scroll) as i32;
        let y = FM_START_Y + row * FM_ROW_H;

        if i == selected {
            backend.fill_rect_inner(0, y - 1, SCREEN_WIDTH, FM_ROW_H as u32, Color::rgba(200, 80, 80, 100));
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
}

/// Draw the now-playing music player UI (using threaded AudioHandle).
fn draw_music_player_threaded(
    backend: &mut PspBackend,
    file_name: &str,
    audio: &AudioHandle,
) {
    let bg = Color::rgba(0, 0, 0, 210);
    backend.fill_rect_inner(0, CONTENT_TOP as i32, SCREEN_WIDTH, CONTENT_H, bg);

    let cx = SCREEN_WIDTH as i32 / 2;
    let title_color = Color::rgb(255, 200, 200);
    let info_color = Color::rgb(180, 180, 180);

    // Album art placeholder.
    let art_size: u32 = 80;
    let art_x = cx - art_size as i32 / 2;
    let art_y = CONTENT_TOP as i32 + 20;
    backend.fill_rect_inner(art_x, art_y, art_size, art_size, Color::rgb(205, 92, 92));
    backend.fill_rect_inner(art_x + 2, art_y + 2, art_size - 4, art_size - 4, Color::rgb(60, 30, 30));
    backend.draw_text_inner("MP3", art_x + 22, art_y + 34, 8, Color::rgb(205, 92, 92));

    // Track name.
    let max_chars = 50;
    let display_name = if file_name.len() > max_chars {
        let truncated: String = file_name.chars().take(max_chars - 2).collect();
        format!("{}..", truncated)
    } else {
        file_name.to_string()
    };
    let name_x = cx - (display_name.len() as i32 * 8) / 2;
    backend.draw_text_inner(&display_name, name_x, art_y + art_size as i32 + 12, 8, title_color);

    // Format info from atomic state.
    let info = format!(
        "{}Hz  {}kbps  {}ch",
        audio.sample_rate(),
        audio.bitrate(),
        audio.channels(),
    );
    let info_x = cx - (info.len() as i32 * 8) / 2;
    backend.draw_text_inner(&info, info_x, art_y + art_size as i32 + 26, 8, info_color);

    let status = if audio.is_paused() { "PAUSED" } else { "PLAYING" };
    let status_clr = if audio.is_paused() {
        Color::rgb(255, 200, 80)
    } else {
        Color::rgb(120, 255, 120)
    };
    let status_x = cx - (status.len() as i32 * 8) / 2;
    backend.draw_text_inner(status, status_x, art_y + art_size as i32 + 44, 8, status_clr);

    let hint = "X:Pause  []:Stop  O:Back";
    let hint_x = cx - (hint.len() as i32 * 8) / 2;
    backend.draw_text_inner(hint, hint_x, BOTTOMBAR_Y - 16, 8, Color::rgb(140, 140, 140));
}

// Command interpreter and utilities are in commands.rs module.
