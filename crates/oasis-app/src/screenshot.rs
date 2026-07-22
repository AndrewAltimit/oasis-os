//! Screenshot capture tool for PSIX visual comparison.
//!
//! Renders the OASIS_OS UI in several states and saves PNG screenshots
//! to `screenshots/{skin_name}/` next to the repo root. Compare these
//! against `Psixpsp.png` to iterate on the visual design.
//!
//! Usage:
//!   cargo run -p oasis-app --bin oasis-screenshot             # classic only
//!   cargo run -p oasis-app --bin oasis-screenshot xp          # single skin
//!   cargo run -p oasis-app --bin oasis-screenshot --all       # all skins
//!   OASIS_SKIN=xp cargo run -p oasis-app --bin oasis-screenshot
//!
//! Output:
//!   screenshots/{skin}/01_dashboard.png   -- Main dashboard view
//!   screenshots/{skin}/02_media_tab.png   -- AUDIO media tab selected
//!   screenshots/{skin}/03_mods_tab.png    -- MODS top tab selected
//!   screenshots/{skin}/04_terminal.png    -- Terminal mode

mod capture_assets;

use std::fs;
use std::path::Path;

use oasis_backend_sdl::SdlBackend;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::{Color, SdiCore};
use oasis_core::bottombar::{BottomBar, MediaTab};
use oasis_core::cursor::{self, CursorState};
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps_themed};
use oasis_core::platform::DesktopPlatform;
use oasis_core::platform::{PowerService, TimeService};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::builtin::builtin_names;
use oasis_core::skin::resolve_skin;
use oasis_core::startmenu::StartMenuState;
use oasis_core::statusbar::StatusBar;
use oasis_core::vfs::MemoryVfs;
use oasis_core::wallpaper;
use oasis_core::wm::{WindowConfig, WindowManager, WindowType};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let arg = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("OASIS_SKIN").ok());

    if arg.as_deref() == Some("--all") {
        let names = all_skin_names();
        println!("Capturing {} skins...", names.len());
        for name in &names {
            capture_skin(name)?;
        }
        println!("All {} skins captured to screenshots/", names.len());
    } else {
        let skin_name = arg.unwrap_or_else(|| "classic".to_string());
        capture_skin(&skin_name)?;
        println!("Screenshots saved to screenshots/{skin_name}/");
        println!("Compare against Psixpsp.png at the repo root.");
    }

    Ok(())
}

/// All available skin names (external TOML skins + built-in skins).
fn all_skin_names() -> Vec<String> {
    let mut names = vec!["classic".to_string()];
    for name in builtin_names() {
        names.push(name.to_string());
    }
    names
}

/// Capture all 4 screenshots for a single skin.
fn capture_skin(skin_name: &str) -> anyhow::Result<()> {
    let skin = resolve_skin(skin_name)?;

    let w = skin.manifest.screen_width;
    let h = skin.manifest.screen_height;

    let mut backend = SdlBackend::new("OASIS Screenshot", w, h)?;
    backend.init(w, h)?;

    let platform = DesktopPlatform::new();
    let mut vfs = MemoryVfs::new();
    populate_demo_vfs(&mut vfs);

    let active_theme = ActiveTheme::from_skin(&skin.theme)
        .with_screen_size(w, h)
        .with_features(&skin.features);

    // Themed discovery mirrors the real shell (main.rs): skins overriding
    // `icon_overrides.fallback_colors` get their emblem palette here too.
    // The default table matches `discover_apps`, so other skins are
    // pixel-identical.
    let apps = discover_apps_themed(
        &vfs,
        "/apps",
        Some("OASISOS"),
        &active_theme.icon.fallback_colors,
    )?;
    let dash_config = DashboardConfig::from_features(&skin.features, &active_theme);
    let mut dashboard = DashboardState::new(dash_config, apps);
    let mut status_bar = StatusBar::new();
    let mut bottom_bar = BottomBar::new();
    bottom_bar.total_pages = dashboard.page_count();

    // Start menu (when enabled by skin).
    let start_menu = if skin.features.start_menu {
        Some(StartMenuState::new_with_theme(
            StartMenuState::default_items(&active_theme),
            &active_theme,
        ))
    } else {
        None
    };

    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);
    capture_assets::setup(
        &skin,
        &active_theme,
        &mut sdi,
        &mut backend,
        &mut status_bar,
    );

    // Wallpaper — skip for shader skins (shader replaces the wallpaper).
    // Also hide any opaque content_bg that the skin layout creates, since
    // it would cover the shader output.
    let has_shader = oasis_core::vector_overlay::get_shader_layer(&active_theme).is_some();
    if has_shader && let Ok(obj) = sdi.get_mut("content_bg") {
        obj.visible = false;
    }
    if !has_shader {
        let wallpaper_tex = {
            let wp_data = wallpaper::generate_with_assets(w, h, &active_theme, &skin.assets);
            backend.load_texture(w, h, &wp_data)?
        };
        let obj = sdi.create("wallpaper");
        obj.x = 0;
        obj.y = 0;
        obj.w = w;
        obj.h = h;
        obj.texture = Some(wallpaper_tex);
        obj.z = -1000;
    }

    // Mouse cursor (position it near center for the screenshot). Skins
    // with a themed `[cursor]` texture show it instead of the procedural
    // arrow, mirroring the runtime software-cursor path.
    let mut mouse_cursor = CursorState::new(w, h);
    mouse_cursor.scale = active_theme.cursor_scale;
    {
        let themed = capture_assets::themed_cursor(&skin, &active_theme);
        let is_themed = themed.is_some();
        let (cursor_pixels, cw, ch) = themed.unwrap_or_else(|| {
            cursor::generate_cursor_pixels_themed(
                active_theme.cursor_scale,
                active_theme.cursor_fill,
                active_theme.cursor_outline,
            )
        });
        let cursor_tex = backend.load_texture(cw, ch, &cursor_pixels)?;
        if is_themed {
            mouse_cursor.size = Some((cw, ch));
            mouse_cursor.hotspot = active_theme.cursor_hotspot;
        }
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
    bottom_bar.update_info(time.as_ref());

    // Create skin-specific output directory.
    let out_dir = Path::new("screenshots").join(skin_name);
    fs::create_dir_all(&out_dir)?;

    let has_dashboard = skin.features.dashboard;
    let has_wm = skin.features.window_manager;

    // For WM skins, create the window manager.
    // For WM-only skins (no dashboard), create demo windows immediately.
    // For dashboard+WM skins, defer window creation to screenshot 4 so
    // dashboard screenshots 1-3 don't have overlapping windows.
    let mut wm = if has_wm {
        let mut wm_theme = skin.theme.build_wm_theme();
        capture_assets::resolve_wm_patches(&skin, &mut wm_theme, &mut backend);
        let mut wm = WindowManager::with_theme(w, h, wm_theme);
        if !has_dashboard {
            create_demo_windows_with_content(&mut wm, &mut sdi, w, h, &active_theme)?;
        }
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
        populate_skin_terminal(&mut sdi, &DEMO_OUTPUT, "/home/user", "ls");
    }
    mouse_cursor.update_sdi(&mut sdi);
    render_and_save_inner(
        &mut backend,
        &mut sdi,
        w,
        h,
        out_dir.join("01_dashboard.png"),
        active_theme.clear_color,
        Some(VectorCtx {
            dashboard: &dashboard,
            theme: &active_theme,
        }),
    )?;
    log::info!("Saved {skin_name}/01_dashboard.png");

    // -- Screenshot 2: AUDIO media tab --
    if has_dashboard {
        bottom_bar.active_tab = MediaTab::Audio;
        dashboard.hide_sdi(&mut sdi);
        if let Some(ref sm) = start_menu {
            sm.hide_sdi(&mut sdi);
        }
        status_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
        bottom_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
        update_media_page(&mut sdi, &bottom_bar, &active_theme);
    }
    mouse_cursor.update_sdi(&mut sdi);
    render_and_save(
        &mut backend,
        &mut sdi,
        w,
        h,
        out_dir.join("02_media_tab.png"),
        active_theme.clear_color,
    )?;
    log::info!("Saved {skin_name}/02_media_tab.png");

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
    // Vector-icon skins render glyphs outside SDI, so the dashboard view
    // needs VectorCtx here just like screenshot 1.
    render_and_save_inner(
        &mut backend,
        &mut sdi,
        w,
        h,
        out_dir.join("03_mods_tab.png"),
        active_theme.clear_color,
        if has_dashboard {
            Some(VectorCtx {
                dashboard: &dashboard,
                theme: &active_theme,
            })
        } else {
            None
        },
    )?;
    log::info!("Saved {skin_name}/03_mods_tab.png");

    // -- Screenshot 4: Terminal mode --
    if has_dashboard {
        dashboard.hide_sdi(&mut sdi);
        StatusBar::hide_sdi(&mut sdi);
        BottomBar::hide_sdi(&mut sdi);
        if let Some(ref sm) = start_menu {
            sm.hide_sdi(&mut sdi);
        }
        hide_media_page(&mut sdi);
        if has_wm {
            if let Some(ref mut wm) = wm {
                let term_cfg = WindowConfig {
                    id: "demo_terminal".to_string(),
                    title: "Terminal".to_string(),
                    x: Some(40),
                    y: Some(30),
                    width: (w as i32 - 80).max(300) as u32,
                    height: (h as i32 - 60).max(200) as u32,
                    window_type: WindowType::AppWindow,
                    always_on_top: false,
                    modal: false,
                };
                wm.create_window(&term_cfg, &mut sdi)?;
                populate_window_content(
                    &mut sdi,
                    "demo_terminal",
                    &DEMO_TERMINAL_CONTENT,
                    oasis_core::terminal_sdi::TerminalColors::from_theme(&active_theme).output,
                    8,
                );
            }
        } else {
            setup_terminal_objects(&mut sdi, &DEMO_OUTPUT, "/home/user", "ls", &active_theme);
        }
    } else if let Some(ref mut wm) = wm {
        let _ = wm.close_window("demo_files", &mut sdi);
        // Clean up manually-created content text objects.
        for i in 0..20 {
            let _ = sdi.destroy(&format!("demo_files_line_{i}"));
        }
    }
    mouse_cursor.update_sdi(&mut sdi);
    render_and_save(
        &mut backend,
        &mut sdi,
        w,
        h,
        out_dir.join("04_terminal.png"),
        active_theme.clear_color,
    )?;
    log::info!("Saved {skin_name}/04_terminal.png");

    backend.shutdown()?;
    Ok(())
}

/// Create demo windows with content for WM-only skins.
///
/// Creates terminal first (background), then file manager (foreground).
/// Content is populated immediately after each window so the SDI draw
/// order naturally layers them correctly.
fn create_demo_windows_with_content(
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    w: u32,
    h: u32,
    at: &ActiveTheme,
) -> anyhow::Result<()> {
    let win_margin = (w / 12) as i32;
    let term_w = (w as i32 - win_margin * 2).max(300) as u32;
    let term_h = (h as i32 - win_margin * 2).max(200) as u32;
    let term_cfg = WindowConfig {
        id: "demo_terminal".to_string(),
        title: "Terminal".to_string(),
        x: Some(win_margin),
        y: Some(win_margin),
        width: term_w,
        height: term_h,
        window_type: WindowType::AppWindow,
        always_on_top: false,
        modal: false,
    };
    wm.create_window(&term_cfg, sdi)?;
    populate_window_content(
        sdi,
        "demo_terminal",
        &DEMO_TERMINAL_CONTENT,
        oasis_core::terminal_sdi::TerminalColors::from_theme(at).output,
        8,
    );

    let fm_w = (w * 7 / 10).max(300);
    let fm_h = (h * 7 / 10).max(200);
    let fm_x = (w as i32 - fm_w as i32) / 2 + win_margin;
    let fm_y = (h as i32 - fm_h as i32) / 2 + win_margin / 2;
    let fm_cfg = WindowConfig {
        id: "demo_files".to_string(),
        title: "File Manager".to_string(),
        x: Some(fm_x),
        y: Some(fm_y),
        width: fm_w,
        height: fm_h,
        window_type: WindowType::AppWindow,
        always_on_top: false,
        modal: false,
    };
    wm.create_window(&fm_cfg, sdi)?;
    populate_window_content(sdi, "demo_files", &DEMO_FILEMANAGER_CONTENT, at.app.text, 8);
    Ok(())
}

/// Render the current SDI scene and save a PNG screenshot.
fn render_and_save(
    backend: &mut SdlBackend,
    sdi: &mut SdiRegistry,
    w: u32,
    h: u32,
    path: std::path::PathBuf,
    clear_color: Color,
) -> anyhow::Result<()> {
    render_and_save_inner(backend, sdi, w, h, path, clear_color, None)
}

/// Vector-capable rendering context for screenshots.
struct VectorCtx<'a> {
    dashboard: &'a DashboardState,
    theme: &'a ActiveTheme,
}

/// Draw `[[chrome_layers]]` in the overlay pass, mirroring the main loop.
fn render_chrome(
    b: &mut SdlBackend,
    vector: Option<&VectorCtx<'_>>,
    fixed_frame: u32,
) -> anyhow::Result<()> {
    if let Some(v) = vector
        && !v.theme.chrome_layers.is_empty()
    {
        let mut cache = oasis_core::vector_overlay::LayerOpsCache::new();
        oasis_core::vector_overlay::render_vector_chrome(b, v.theme, fixed_frame, &mut cache)?;
    }
    Ok(())
}

fn render_and_save_inner(
    backend: &mut SdlBackend,
    sdi: &mut SdiRegistry,
    w: u32,
    h: u32,
    path: std::path::PathBuf,
    clear_color: Color,
    vector: Option<VectorCtx<'_>>,
) -> anyhow::Result<()> {
    // Create shader bridge if the theme has a shader background layer.
    let shader_info = vector
        .as_ref()
        .and_then(|v| oasis_core::vector_overlay::get_shader_layer(v.theme));
    let mut shader_bridge = shader_info
        .as_ref()
        .and_then(|_| oasis_backend_sdl::shader_bridge::SdlShaderBridge::new(w, h));

    let render_once = |b: &mut SdlBackend,
                       s: &mut SdiRegistry,
                       bridge: &mut Option<oasis_backend_sdl::shader_bridge::SdlShaderBridge>|
     -> anyhow::Result<()> {
        b.clear(clear_color)?;
        // Render shader wallpaper first if present (replaces bg clear).
        // OASIS_FIXED_FRAME overrides the frame number for deterministic
        // shader captures in CI screenshot regression tests.
        let fixed_frame: u32 = std::env::var("OASIS_FIXED_FRAME")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);
        if let (Some(br), Some(info)) = (bridge.as_mut(), shader_info.as_ref()) {
            let time = fixed_frame as f32 / 60.0;
            br.render_and_blit(b, &info.name, time, &info.params);
        }
        if let Some(ref v) = vector
            && (v.theme.icon.style == "vector" || !v.theme.background_layers.is_empty())
        {
            s.draw_base_layer(b)?;
            oasis_core::vector_overlay::render_vector_background(b, v.theme, fixed_frame)?;
            v.dashboard.render_vector_icons(b, v.theme, fixed_frame)?;
            s.draw_overlay_layer(b)?;
            render_chrome(b, vector.as_ref(), fixed_frame)?;
            return Ok(());
        }
        s.draw(b)?;
        render_chrome(b, vector.as_ref(), fixed_frame)?;
        Ok(())
    };

    render_once(backend, sdi, &mut shader_bridge)?;
    backend.swap_buffers()?;
    render_once(backend, sdi, &mut shader_bridge)?;

    let pixels = backend.read_pixels(0, 0, w, h)?;
    save_png(&path, w, h, &pixels)?;
    Ok(())
}

/// Save RGBA pixel data as a PNG file.
///
/// The alpha channel is forced opaque: a window framebuffer has no
/// transparency, and SDL's software renderer otherwise leaves a per-primitive
/// blend artifact there that would make the gallery PNGs partly see-through.
fn save_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> anyhow::Result<()> {
    let file = fs::File::create(path)?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    let mut opaque = rgba.to_vec();
    for px in opaque.as_chunks_mut::<4>().0.iter_mut() {
        px[3] = 255;
    }
    writer.write_image_data(&opaque)?;
    Ok(())
}

const DEMO_OUTPUT: [&str; 5] = [
    "OASIS_OS v0.1.0 -- Type 'help' for commands",
    "F1=terminal  F2=on-screen keyboard  Escape=quit",
    "",
    "> status",
    "System: OASIS_OS v0.1.0  CPU: 333MHz  Battery: 75%",
];

const DEMO_TERMINAL_CONTENT: [&str; 8] = [
    "OASIS_OS v0.1.0 -- Type 'help' for commands",
    "",
    "> ls /home/user",
    "readme.txt  music/  photos/",
    "",
    "> status",
    "System: OASIS_OS v0.1.0  CPU: 333MHz  Battery: 75%",
    "/home/user> _",
];

const DEMO_FILEMANAGER_CONTENT: [&str; 7] = [
    " /home/user",
    " --------------------------------",
    "  readme.txt          20 B",
    "  music/",
    "    ambient_dawn.mp3  194 KB",
    "  photos/",
    "    sample_landscape.png  6 KB",
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

fn update_media_page(sdi: &mut SdiRegistry, bottom_bar: &BottomBar, at: &ActiveTheme) {
    let page_name = "media_page_text";
    if !sdi.contains(page_name) {
        let obj = sdi.create(page_name);
        obj.font_size = at.font_heading;
        obj.text_color = at.app.text;
        obj.w = 0;
        obj.h = 0;
    }
    let page_str = format!("[ {} Page ]", bottom_bar.active_tab.label());
    if let Ok(obj) = sdi.get_mut(page_name) {
        obj.x = (at.screen_w as i32) / 2 - (page_str.len() as i32 * at.font_heading as i32 / 2);
        obj.y = (at.screen_h as i32) / 2 - 16;
        obj.visible = true;
        obj.text = Some(page_str);
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
    at: &ActiveTheme,
) {
    let title_h = at.app.title_bar_height;
    let content_x = 4i32;
    let content_y = (title_h + 4) as i32;
    let content_w = at.screen_w.saturating_sub(8);
    let line_h = at.terminal_line_height;
    let font_size = line_h.saturating_sub(4).max(8) as u16;
    let usable_h = at.screen_h - title_h - at.statusbar_height - at.bottombar_height - 14;
    let tc = oasis_core::terminal_sdi::TerminalColors::from_theme(at);

    if !sdi.contains("terminal_bg") {
        let obj = sdi.create("terminal_bg");
        obj.x = content_x;
        obj.y = content_y;
        obj.w = content_w;
        obj.h = usable_h;
        obj.color = tc.bg;
    }
    if let Ok(obj) = sdi.get_mut("terminal_bg") {
        obj.visible = true;
    }

    let max_lines = (usable_h / line_h).max(1) as usize;
    for i in 0..max_lines {
        let name = format!("term_line_{i}");
        if !sdi.contains(&name) {
            let obj = sdi.create(&name);
            obj.x = content_x + 4;
            obj.y = content_y + 2 + (i as i32) * (line_h as i32);
            obj.font_size = font_size;
            obj.text_color = tc.output;
            obj.w = 0;
            obj.h = 0;
        }
        if let Ok(obj) = sdi.get_mut(&name) {
            obj.text = output_lines.get(i).map(|s| s.to_string());
            obj.visible = true;
        }
    }

    let input_y = content_y + (usable_h as i32) - (line_h as i32) - 2;
    let input_bg_color = tc.input_bg;
    if !sdi.contains("term_input_bg") {
        let obj = sdi.create("term_input_bg");
        obj.x = content_x;
        obj.y = input_y;
        obj.w = content_w;
        obj.h = line_h + 4;
        obj.color = input_bg_color;
    }
    if let Ok(obj) = sdi.get_mut("term_input_bg") {
        obj.visible = true;
    }

    if !sdi.contains("term_prompt") {
        let obj = sdi.create("term_prompt");
        obj.x = content_x + 4;
        obj.y = input_y + 2;
        obj.font_size = font_size;
        obj.text_color = tc.prompt;
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut("term_prompt") {
        obj.text = Some(format!("{cwd}> {input_buf}_"));
        obj.visible = true;
    }
}

/// Populate a window's content area with demo text lines.
///
/// Text objects are created at z=0 (default) so they sort by creation order.
/// Call immediately after `create_window()` so the text objects appear between
/// this window's content bg and the next window's chrome.
fn populate_window_content(
    sdi: &mut SdiRegistry,
    window_id: &str,
    lines: &[&str],
    text_color: Color,
    font_size: u16,
) {
    // Read the window's content area from its SDI object (WM uses dot separator).
    let content_name = format!("{window_id}.content");
    let (cx, cy, _cw, ch) = if let Ok(obj) = sdi.get_mut(&content_name) {
        (obj.x, obj.y, obj.w, obj.h)
    } else {
        return;
    };

    let line_h = (font_size as i32).max(10) + 2;
    let max_lines = ((ch as i32) / line_h).max(1) as usize;
    for (i, line) in lines.iter().take(max_lines).enumerate() {
        let name = format!("{window_id}_line_{i}");
        if !sdi.contains(&name) {
            let obj = sdi.create(&name);
            obj.x = cx + 6;
            obj.y = cy + 4 + (i as i32) * line_h;
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
}

fn populate_demo_vfs(vfs: &mut MemoryVfs) {
    use oasis_core::vfs::Vfs;

    vfs.mkdir("/home").expect("VFS mkdir /home");
    vfs.mkdir("/home/user").expect("VFS mkdir /home/user");
    vfs.mkdir("/etc").expect("VFS mkdir /etc");
    vfs.mkdir("/tmp").expect("VFS mkdir /tmp");
    vfs.write("/home/user/readme.txt", b"Welcome to OASIS_OS!")
        .expect("VFS write readme.txt");
    vfs.write("/etc/hostname", b"oasis")
        .expect("VFS write hostname");
    vfs.write("/etc/version", b"0.1.0")
        .expect("VFS write version");

    vfs.mkdir("/apps").expect("VFS mkdir /apps");
    for name in &[
        "File Manager",
        "Settings",
        "Network",
        "Terminal",
        "Music Player",
        "Internet Radio",
        "Photo Viewer",
        "Package Manager",
        "System Monitor",
        "Browser",
        "TV Guide",
    ] {
        vfs.mkdir(&format!("/apps/{name}"))
            .expect("VFS mkdir app directory");
    }

    vfs.mkdir("/home/user/music").expect("VFS mkdir music");
    vfs.mkdir("/home/user/photos").expect("VFS mkdir photos");
}
