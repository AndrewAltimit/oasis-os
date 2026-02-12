//! Screenshot capture tool for PSIX visual comparison.
//!
//! Renders the OASIS_OS UI in several states and saves PNG screenshots
//! to `screenshots/{skin_name}/` next to the repo root. Compare these
//! against `Psixpsp.png` to iterate on the visual design.
//!
//! Usage:
//!   cargo run -p oasis-app --bin oasis-screenshot [skin_name]
//!   OASIS_SKIN=xp cargo run -p oasis-app --bin oasis-screenshot
//!
//! Output:
//!   screenshots/{skin}/01_dashboard.png   -- Main dashboard view
//!   screenshots/{skin}/02_media_tab.png   -- AUDIO media tab selected
//!   screenshots/{skin}/03_mods_tab.png    -- MODS top tab selected
//!   screenshots/{skin}/04_terminal.png    -- Terminal mode

use std::fs;
use std::path::Path;

use oasis_backend_sdl::SdlBackend;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::{Color, SdiBackend};
use oasis_core::bottombar::{BottomBar, MediaTab};
use oasis_core::cursor::{self, CursorState};
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps};
use oasis_core::platform::DesktopPlatform;
use oasis_core::platform::{PowerService, TimeService};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::resolve_skin;
use oasis_core::startmenu::StartMenuState;
use oasis_core::statusbar::StatusBar;
use oasis_core::vfs::MemoryVfs;
use oasis_core::wallpaper;
use oasis_core::wm::{WindowConfig, WindowManager, WindowType};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let skin_name = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("OASIS_SKIN").ok())
        .unwrap_or_else(|| "classic".to_string());
    let skin = resolve_skin(&skin_name)?;

    // Use the skin's screen dimensions (e.g. 800x600 for desktop, 480x272 for PSP skins).
    let w = skin.manifest.screen_width;
    let h = skin.manifest.screen_height;

    let mut backend = SdlBackend::new("OASIS Screenshot", w, h)?;
    backend.init(w, h)?;

    let platform = DesktopPlatform::new();
    let mut vfs = MemoryVfs::new();
    populate_demo_vfs(&mut vfs);

    let active_theme = ActiveTheme::from_skin(&skin.theme);

    let apps = discover_apps(&vfs, "/apps", Some("OASISOS"))?;
    let dash_config = DashboardConfig::from_features(&skin.features, &active_theme);
    let dashboard = DashboardState::new(dash_config, apps);
    let mut status_bar = StatusBar::new();
    let mut bottom_bar = BottomBar::new();
    bottom_bar.total_pages = dashboard.page_count();

    // Start menu (when enabled by skin).
    let start_menu = if skin.features.start_menu {
        Some(StartMenuState::new_with_theme(
            StartMenuState::default_items(),
            &active_theme,
        ))
    } else {
        None
    };

    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);

    // Wallpaper.
    let wallpaper_tex = {
        let wp_data = wallpaper::generate_from_config(w, h, &active_theme);
        backend.load_texture(w, h, &wp_data)?
    };
    {
        let obj = sdi.create("wallpaper");
        obj.x = 0;
        obj.y = 0;
        obj.w = w;
        obj.h = h;
        obj.texture = Some(wallpaper_tex);
        obj.z = -1000;
    }

    // Mouse cursor (position it near center for the screenshot).
    let mut mouse_cursor = CursorState::new(w, h);
    {
        let (cursor_pixels, cw, ch) = cursor::generate_cursor_pixels();
        let cursor_tex = backend.load_texture(cw, ch, &cursor_pixels)?;
        mouse_cursor.update_sdi(&mut sdi);
        if let Ok(obj) = sdi.get_mut("mouse_cursor") {
            obj.texture = Some(cursor_tex);
        }
    }
    mouse_cursor.set_position(240, 136);

    // Update system info once.
    let time = platform.now().ok();
    let power = platform.power_info().ok();
    status_bar.update_info(time.as_ref(), power.as_ref());

    // Create skin-specific output directory.
    let out_dir = Path::new("screenshots").join(&skin_name);
    fs::create_dir_all(&out_dir)?;

    let has_dashboard = skin.features.dashboard;
    let has_wm = skin.features.window_manager;

    // For WM skins, create demo windows.
    let mut wm = if has_wm {
        let wm_theme = skin.theme.build_wm_theme();
        let mut wm = WindowManager::with_theme(w, h, wm_theme);
        let term_cfg = WindowConfig {
            id: "demo_terminal".to_string(),
            title: "Terminal".to_string(),
            x: Some(40),
            y: Some(30),
            width: 400,
            height: 260,
            window_type: WindowType::AppWindow,
        };
        wm.create_window(&term_cfg, &mut sdi)?;
        let fm_cfg = WindowConfig {
            id: "demo_files".to_string(),
            title: "File Manager".to_string(),
            x: Some(200),
            y: Some(100),
            width: 350,
            height: 220,
            window_type: WindowType::AppWindow,
        };
        wm.create_window(&fm_cfg, &mut sdi)?;
        Some(wm)
    } else {
        None
    };

    // -- Screenshot 1: Dashboard --
    if has_dashboard {
        dashboard.update_sdi(&mut sdi, &active_theme);
        status_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
        bottom_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
        if let Some(ref sm) = start_menu {
            sm.update_sdi(&mut sdi, &active_theme);
        }
    } else if has_wm {
        // WM desktop: windows are already created, show them as-is.
    } else if skin.features.terminal {
        // Terminal-only skins: populate the skin's own terminal objects.
        populate_skin_terminal(&mut sdi, &DEMO_OUTPUT, "/home/user", "ls");
    }
    mouse_cursor.update_sdi(&mut sdi);
    render_and_save(
        &mut backend,
        &mut sdi,
        w,
        h,
        out_dir.join("01_dashboard.png"),
    )?;
    log::info!("Saved 01_dashboard.png");

    // -- Screenshot 2: AUDIO media tab --
    if has_dashboard {
        bottom_bar.active_tab = MediaTab::Audio;
        dashboard.hide_sdi(&mut sdi);
        if let Some(ref sm) = start_menu {
            sm.hide_sdi(&mut sdi);
        }
        status_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
        bottom_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
        update_media_page(&mut sdi, &bottom_bar);
    }
    mouse_cursor.update_sdi(&mut sdi);
    render_and_save(
        &mut backend,
        &mut sdi,
        w,
        h,
        out_dir.join("02_media_tab.png"),
    )?;
    log::info!("Saved 02_media_tab.png");

    // -- Screenshot 3: MODS top tab --
    if has_dashboard {
        bottom_bar.active_tab = MediaTab::None;
        status_bar.active_tab = oasis_core::statusbar::TopTab::Mods;
        hide_media_page(&mut sdi);
        dashboard.update_sdi(&mut sdi, &active_theme);
        status_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
        bottom_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
        if let Some(ref sm) = start_menu {
            sm.update_sdi(&mut sdi, &active_theme);
        }
    }
    mouse_cursor.update_sdi(&mut sdi);
    render_and_save(
        &mut backend,
        &mut sdi,
        w,
        h,
        out_dir.join("03_mods_tab.png"),
    )?;
    log::info!("Saved 03_mods_tab.png");

    // -- Screenshot 4: Terminal mode --
    if has_dashboard {
        dashboard.hide_sdi(&mut sdi);
        StatusBar::hide_sdi(&mut sdi);
        BottomBar::hide_sdi(&mut sdi);
        if let Some(ref sm) = start_menu {
            sm.hide_sdi(&mut sdi);
        }
        hide_media_page(&mut sdi);
        setup_terminal_objects(&mut sdi, &DEMO_OUTPUT, "/home/user", "ls");
    } else if let Some(ref mut wm) = wm {
        // WM desktop: close file manager, keep only terminal window.
        let _ = wm.close_window("demo_files", &mut sdi);
    }
    // Terminal-only skins already have their terminal populated from screenshot 1.
    mouse_cursor.update_sdi(&mut sdi);
    render_and_save(
        &mut backend,
        &mut sdi,
        w,
        h,
        out_dir.join("04_terminal.png"),
    )?;
    log::info!("Saved 04_terminal.png");

    backend.shutdown()?;

    println!("Screenshots saved to {}/", out_dir.display());
    println!("Compare against Psixpsp.png at the repo root.");
    Ok(())
}

/// Render the current SDI scene and save a PNG screenshot.
fn render_and_save(
    backend: &mut SdlBackend,
    sdi: &mut SdiRegistry,
    w: u32,
    h: u32,
    path: std::path::PathBuf,
) -> anyhow::Result<()> {
    backend.clear(Color::rgb(10, 10, 18))?;
    sdi.draw(backend)?;
    backend.swap_buffers()?;

    // Need to render again after swap so read_pixels gets the presented frame.
    backend.clear(Color::rgb(10, 10, 18))?;
    sdi.draw(backend)?;

    let pixels = backend.read_pixels(0, 0, w, h)?;
    save_png(&path, w, h, &pixels)?;
    Ok(())
}

/// Save RGBA pixel data as a PNG file.
fn save_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> anyhow::Result<()> {
    let file = fs::File::create(path)?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}

const DEMO_OUTPUT: [&str; 5] = [
    "OASIS_OS v0.1.0 -- Type 'help' for commands",
    "F1=terminal  F2=on-screen keyboard  Escape=quit",
    "",
    "> status",
    "System: OASIS_OS v0.1.0  CPU: 333MHz  Battery: 75%",
];

/// Populate a skin's own terminal layout objects with demo content.
///
/// Creates individual line objects within the skin's `terminal_output` area,
/// since SDI objects render single-line text only.
fn populate_skin_terminal(sdi: &mut SdiRegistry, lines: &[&str], cwd: &str, input: &str) {
    // Read position/style from the skin's terminal_output object.
    let (base_x, base_y, font_size, text_color) = if let Ok(obj) = sdi.get_mut("terminal_output") {
        let info = (obj.x, obj.y, obj.font_size, obj.text_color);
        obj.visible = true;
        info
    } else {
        (4, 120, 8, Color::rgb(0, 187, 187))
    };

    let line_h = (font_size as i32).max(10) + 2;
    for (i, line) in lines.iter().enumerate() {
        let name = format!("term_line_{i}");
        if !sdi.contains(&name) {
            let obj = sdi.create(&name);
            obj.x = base_x + 2;
            obj.y = base_y + 2 + (i as i32) * line_h;
            obj.font_size = font_size;
            obj.text_color = text_color;
            obj.w = 0;
            obj.h = 0;
        }
        if let Ok(obj) = sdi.get_mut(&name) {
            obj.text = if line.is_empty() {
                None
            } else {
                Some(line.to_string())
            };
            obj.visible = true;
        }
    }

    if let Ok(obj) = sdi.get_mut("terminal_prompt") {
        obj.text = Some(format!("{cwd}> {input}_"));
        obj.visible = true;
    }
}

fn update_media_page(sdi: &mut SdiRegistry, bottom_bar: &BottomBar) {
    let page_name = "media_page_text";
    if !sdi.contains(page_name) {
        let obj = sdi.create(page_name);
        obj.font_size = 14;
        obj.text_color = Color::rgb(160, 200, 180);
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut(page_name) {
        obj.x = 160;
        obj.y = 120;
        obj.visible = true;
        obj.text = Some(format!("[ {} Page ]", bottom_bar.active_tab.label()));
    }
}

fn hide_media_page(sdi: &mut SdiRegistry) {
    for name in &["media_page_text", "media_page_hint"] {
        if let Ok(obj) = sdi.get_mut(name) {
            obj.visible = false;
        }
    }
}

fn setup_terminal_objects(
    sdi: &mut SdiRegistry,
    output_lines: &[&str],
    cwd: &str,
    input_buf: &str,
) {
    if !sdi.contains("terminal_bg") {
        let obj = sdi.create("terminal_bg");
        obj.x = 4;
        obj.y = 26;
        obj.w = 472;
        obj.h = 220;
        obj.color = Color::rgb(12, 12, 20);
    }
    if let Ok(obj) = sdi.get_mut("terminal_bg") {
        obj.visible = true;
    }

    let max_lines = 12;
    for i in 0..max_lines {
        let name = format!("term_line_{i}");
        if !sdi.contains(&name) {
            let obj = sdi.create(&name);
            obj.x = 8;
            obj.y = 28 + (i as i32) * 16;
            obj.font_size = 12;
            obj.text_color = Color::rgb(0, 200, 0);
            obj.w = 0;
            obj.h = 0;
        }
        if let Ok(obj) = sdi.get_mut(&name) {
            obj.text = output_lines.get(i).map(|s| s.to_string());
            obj.visible = true;
        }
    }

    if !sdi.contains("term_input_bg") {
        let obj = sdi.create("term_input_bg");
        obj.x = 4;
        obj.y = 248;
        obj.w = 472;
        obj.h = 20;
        obj.color = Color::rgb(20, 20, 35);
    }
    if let Ok(obj) = sdi.get_mut("term_input_bg") {
        obj.visible = true;
    }

    if !sdi.contains("term_prompt") {
        let obj = sdi.create("term_prompt");
        obj.x = 8;
        obj.y = 250;
        obj.font_size = 12;
        obj.text_color = Color::rgb(100, 200, 255);
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut("term_prompt") {
        obj.text = Some(format!("{cwd}> {input_buf}_"));
        obj.visible = true;
    }
}

fn populate_demo_vfs(vfs: &mut MemoryVfs) {
    use oasis_core::vfs::Vfs;

    vfs.mkdir("/home").unwrap();
    vfs.mkdir("/home/user").unwrap();
    vfs.mkdir("/etc").unwrap();
    vfs.mkdir("/tmp").unwrap();
    vfs.write("/home/user/readme.txt", b"Welcome to OASIS_OS!")
        .unwrap();
    vfs.write("/etc/hostname", b"oasis").unwrap();
    vfs.write("/etc/version", b"0.1.0").unwrap();

    vfs.mkdir("/apps").unwrap();
    for name in &[
        "File Manager",
        "Settings",
        "Network",
        "Terminal",
        "Music Player",
        "Photo Viewer",
        "Package Manager",
        "System Monitor",
    ] {
        vfs.mkdir(&format!("/apps/{name}")).unwrap();
    }

    vfs.mkdir("/home/user/music").unwrap();
    vfs.mkdir("/home/user/photos").unwrap();
}
