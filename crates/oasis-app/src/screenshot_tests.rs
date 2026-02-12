//! Screenshot test harness for OASIS_OS visual regression testing.
//!
//! Renders specific scenarios and saves PNG screenshots to
//! `screenshots/tests/{scenario}/`. These are for human review -- not
//! CI-blocking.
//!
//! Usage:
//!   cargo run -p oasis-app --bin screenshot-tests
//!   cargo run -p oasis-app --bin screenshot-tests -- --scenario dashboard_classic
//!   cargo run -p oasis-app --bin screenshot-tests -- --skin xp
//!   cargo run -p oasis-app --bin screenshot-tests -- --report
//!
//! Output:
//!   screenshots/tests/{scenario}/actual.png
//!   screenshots/tests/report.html            (with --report)

use std::fs;
use std::path::{Path, PathBuf};

use oasis_backend_sdl::SdlBackend;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::{Color, SdiBackend};
use oasis_core::bottombar::BottomBar;
use oasis_core::browser::{BrowserConfig, BrowserWidget};
use oasis_core::config::OasisConfig;
use oasis_core::cursor::{self, CursorState};
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps};
use oasis_core::platform::DesktopPlatform;
use oasis_core::platform::{PowerService, TimeService};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::builtin::builtin_names;
use oasis_core::skin::resolve_skin;
use oasis_core::startmenu::StartMenuState;
use oasis_core::statusbar::StatusBar;
use oasis_core::vfs::{MemoryVfs, Vfs};
use oasis_core::wallpaper;
use oasis_core::wm::manager::WindowManager;
use oasis_core::wm::window::{WindowConfig, WindowType};

// ---------------------------------------------------------------------------
// CLI parsing
// ---------------------------------------------------------------------------

struct Args {
    /// Only run scenarios matching this filter.
    scenario_filter: Option<String>,
    /// Only run scenarios for this skin (skin matrix only).
    skin_filter: Option<String>,
    /// Generate an HTML comparison report.
    report: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        scenario_filter: None,
        skin_filter: None,
        report: false,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--scenario" => args.scenario_filter = iter.next(),
            "--skin" => args.skin_filter = iter.next(),
            "--report" => args.report = true,
            other => {
                eprintln!("Unknown argument: {other}");
                eprintln!(
                    "Usage: screenshot-tests [--scenario NAME] [--skin NAME] [--report]"
                );
                std::process::exit(1);
            }
        }
    }
    args
}

// ---------------------------------------------------------------------------
// Scenario definition
// ---------------------------------------------------------------------------

struct Scenario {
    name: String,
    category: &'static str,
}

fn all_scenarios() -> Vec<Scenario> {
    let mut scenarios = Vec::new();

    // Skin matrix: each skin x each view.
    let views = ["dashboard", "terminal", "start_menu", "windows", "browser"];
    let all_skins = all_skin_names();
    for skin in &all_skins {
        for view in &views {
            scenarios.push(Scenario {
                name: format!("{skin}_{view}"),
                category: "skin",
            });
        }
    }

    // Browser rendering test pages.
    let pages = [
        "basic_text",
        "colors_backgrounds",
        "box_model",
        "links",
        "lists",
        "table",
        "nested_layout",
        "long_page",
        "css_cascade",
        "reader_mode",
        "error_page",
        "empty_page",
        "gemini_page",
    ];
    for page in &pages {
        scenarios.push(Scenario {
            name: format!("browser_{page}"),
            category: "browser",
        });
    }

    // Widget gallery.
    scenarios.push(Scenario {
        name: "widget_gallery".to_string(),
        category: "widget",
    });

    // Window manager scenarios.
    let wm_views = [
        "wm_single_maximized",
        "wm_cascaded_windows",
        "wm_dialog_overlay",
    ];
    for view in &wm_views {
        scenarios.push(Scenario {
            name: view.to_string(),
            category: "wm",
        });
    }

    scenarios
}

fn all_skin_names() -> Vec<String> {
    let mut names: Vec<String> = builtin_names().iter().map(|s| s.to_string()).collect();
    names.insert(0, "classic".to_string());
    names
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

fn render_and_save(
    backend: &mut SdlBackend,
    sdi: &mut SdiRegistry,
    w: u32,
    h: u32,
    path: &Path,
) -> anyhow::Result<()> {
    backend.clear(Color::rgb(10, 10, 18))?;
    sdi.draw(backend)?;
    backend.swap_buffers()?;

    // Render again after swap so read_pixels gets the presented frame.
    backend.clear(Color::rgb(10, 10, 18))?;
    sdi.draw(backend)?;

    let pixels = backend.read_pixels(0, 0, w, h)?;
    save_png(path, w, h, &pixels)?;
    Ok(())
}

/// Render the browser widget directly (bypasses SDI -- browser paints to the
/// backend framebuffer).
fn render_browser_and_save(
    backend: &mut SdlBackend,
    browser: &mut BrowserWidget,
    w: u32,
    h: u32,
    path: &Path,
) -> anyhow::Result<()> {
    backend.clear(Color::rgb(255, 255, 255))?;
    browser.paint(backend)?;
    backend.swap_buffers()?;

    backend.clear(Color::rgb(255, 255, 255))?;
    browser.paint(backend)?;

    let pixels = backend.read_pixels(0, 0, w, h)?;
    save_png(path, w, h, &pixels)?;
    Ok(())
}

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

fn populate_demo_vfs(vfs: &mut MemoryVfs) {
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

fn setup_terminal_objects(sdi: &mut SdiRegistry, lines: &[String], cwd: &str, input: &str) {
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

    for i in 0..12 {
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
            obj.text = lines.get(i).cloned();
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
        obj.text = Some(format!("{cwd}> {input}_"));
        obj.visible = true;
    }
}

fn hide_terminal_objects(sdi: &mut SdiRegistry) {
    for name in ["terminal_bg", "term_input_bg", "term_prompt"] {
        if let Ok(obj) = sdi.get_mut(name) {
            obj.visible = false;
        }
    }
    for i in 0..12 {
        let name = format!("term_line_{i}");
        if let Ok(obj) = sdi.get_mut(&name) {
            obj.visible = false;
        }
    }
}

// ---------------------------------------------------------------------------
// Skin matrix scenarios
// ---------------------------------------------------------------------------

fn run_skin_scenario(
    backend: &mut SdlBackend,
    skin_name: &str,
    view: &str,
    out_dir: &Path,
    w: u32,
    h: u32,
) -> anyhow::Result<()> {
    let skin = resolve_skin(skin_name)?;
    let active_theme = ActiveTheme::from_skin(&skin.theme);
    let platform = DesktopPlatform::new();

    let mut vfs = MemoryVfs::new();
    populate_demo_vfs(&mut vfs);

    let apps = discover_apps(&vfs, "/apps", Some("OASISOS"))?;
    let dash_config = DashboardConfig::from_features(&skin.features, &active_theme);
    let dashboard = DashboardState::new(dash_config, apps);
    let mut status_bar = StatusBar::new();
    let mut bottom_bar = BottomBar::new();
    bottom_bar.total_pages = dashboard.page_count();

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
    let wp_data = wallpaper::generate_from_config(w, h, &active_theme);
    let wallpaper_tex = backend.load_texture(w, h, &wp_data)?;
    {
        let obj = sdi.create("wallpaper");
        obj.x = 0;
        obj.y = 0;
        obj.w = w;
        obj.h = h;
        obj.texture = Some(wallpaper_tex);
        obj.z = -1000;
    }

    // Cursor.
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

    // System info.
    let time = platform.now().ok();
    let power = platform.power_info().ok();
    status_bar.update_info(time.as_ref(), power.as_ref());

    match view {
        "dashboard" => {
            dashboard.update_sdi(&mut sdi, &active_theme);
            status_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
            bottom_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
            if let Some(ref sm) = start_menu {
                sm.update_sdi(&mut sdi, &active_theme);
            }
            mouse_cursor.update_sdi(&mut sdi);
            render_and_save(backend, &mut sdi, w, h, &out_dir.join("actual.png"))?;
        }
        "terminal" => {
            dashboard.hide_sdi(&mut sdi);
            StatusBar::hide_sdi(&mut sdi);
            BottomBar::hide_sdi(&mut sdi);
            if let Some(ref sm) = start_menu {
                sm.hide_sdi(&mut sdi);
            }
            setup_terminal_objects(
                &mut sdi,
                &[
                    "OASIS_OS v0.1.0 -- Type 'help' for commands".to_string(),
                    String::new(),
                    "> ls /home/user".to_string(),
                    "music/  photos/  readme.txt".to_string(),
                    String::new(),
                    "> cat /etc/hostname".to_string(),
                    "oasis".to_string(),
                ],
                "/home/user",
                "status",
            );
            mouse_cursor.update_sdi(&mut sdi);
            render_and_save(backend, &mut sdi, w, h, &out_dir.join("actual.png"))?;
        }
        "start_menu" => {
            dashboard.update_sdi(&mut sdi, &active_theme);
            status_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
            bottom_bar.update_sdi(&mut sdi, &active_theme, &skin.features);
            // Update start menu SDI (visible by default after update).
            if let Some(ref sm) = start_menu {
                sm.update_sdi(&mut sdi, &active_theme);
            }
            mouse_cursor.set_position(40, 250);
            mouse_cursor.update_sdi(&mut sdi);
            render_and_save(backend, &mut sdi, w, h, &out_dir.join("actual.png"))?;
        }
        "windows" => {
            dashboard.hide_sdi(&mut sdi);
            StatusBar::hide_sdi(&mut sdi);
            BottomBar::hide_sdi(&mut sdi);
            if let Some(ref sm) = start_menu {
                sm.hide_sdi(&mut sdi);
            }
            hide_terminal_objects(&mut sdi);

            let mut wm = WindowManager::new(w, h);
            let configs = [
                WindowConfig {
                    id: "win1".to_string(),
                    title: "File Manager".to_string(),
                    x: Some(20),
                    y: Some(30),
                    width: 200,
                    height: 150,
                    window_type: WindowType::AppWindow,
                },
                WindowConfig {
                    id: "win2".to_string(),
                    title: "Settings".to_string(),
                    x: Some(80),
                    y: Some(60),
                    width: 180,
                    height: 130,
                    window_type: WindowType::AppWindow,
                },
                WindowConfig {
                    id: "win3".to_string(),
                    title: "Terminal".to_string(),
                    x: Some(140),
                    y: Some(90),
                    width: 220,
                    height: 140,
                    window_type: WindowType::AppWindow,
                },
            ];
            for cfg in &configs {
                wm.create_window(cfg, &mut sdi)?;
            }
            mouse_cursor.set_position(250, 130);
            mouse_cursor.update_sdi(&mut sdi);
            render_and_save(backend, &mut sdi, w, h, &out_dir.join("actual.png"))?;
        }
        "browser" => {
            dashboard.hide_sdi(&mut sdi);
            StatusBar::hide_sdi(&mut sdi);
            BottomBar::hide_sdi(&mut sdi);
            if let Some(ref sm) = start_menu {
                sm.hide_sdi(&mut sdi);
            }
            hide_terminal_objects(&mut sdi);

            let browser_config = BrowserConfig::from_skin_theme(&skin.theme);
            let mut browser = BrowserWidget::new(browser_config);
            browser.set_window(0, 0, w, h);

            let html = "<html><body>\
                <h1>OASIS Browser</h1>\
                <p>Welcome to the built-in browser engine.</p>\
                <p><a href=\"/page2\">Sample link</a></p>\
                <div style=\"background:#eee;padding:8px;margin:8px;\">\
                  <p>Styled content block</p>\
                </div>\
                </body></html>";
            browser.load_html(html, "vfs://test/index.html");

            // Render browser directly (it paints to the backend, not SDI).
            // First render wallpaper + SDI, then browser on top.
            backend.clear(Color::rgb(10, 10, 18))?;
            sdi.draw(backend)?;
            browser.paint(backend)?;
            backend.swap_buffers()?;

            backend.clear(Color::rgb(10, 10, 18))?;
            sdi.draw(backend)?;
            browser.paint(backend)?;
            let pixels = backend.read_pixels(0, 0, w, h)?;
            save_png(&out_dir.join("actual.png"), w, h, &pixels)?;
        }
        _ => {
            log::warn!("Unknown skin view: {view}");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Browser page scenarios
// ---------------------------------------------------------------------------

fn run_browser_scenario(
    backend: &mut SdlBackend,
    page_name: &str,
    out_dir: &Path,
    w: u32,
    h: u32,
) -> anyhow::Result<()> {
    let mut browser = BrowserWidget::new(BrowserConfig::default());
    browser.set_window(0, 0, w, h);

    let html = match page_name {
        "gemini_page" => {
            // Load Gemini content.
            let gmi = include_str!("../../../test-fixtures/gemini/test_page.gmi");
            // Render Gemini as HTML via the browser's Gemini renderer.
            let html = format!(
                "<html><body><pre>{}</pre></body></html>",
                gmi.replace('<', "&lt;").replace('>', "&gt;")
            );
            browser.load_html(&html, "gemini://test/page.gmi");
            String::new() // Already loaded.
        }
        _ => {
            let fixture_path = format!("test-fixtures/html/{page_name}.html");
            let content = fs::read_to_string(&fixture_path).unwrap_or_else(|_| {
                format!(
                    "<html><body><p>Missing fixture: {fixture_path}</p></body></html>"
                )
            });
            browser.load_html(&content, &format!("file://test/{page_name}.html"));
            content
        }
    };
    let _ = html; // Suppress unused warning.

    render_browser_and_save(backend, &mut browser, w, h, &out_dir.join("actual.png"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Widget gallery scenario
// ---------------------------------------------------------------------------

fn run_widget_gallery(
    backend: &mut SdlBackend,
    out_dir: &Path,
    w: u32,
    h: u32,
) -> anyhow::Result<()> {
    let mut sdi = SdiRegistry::new();

    // Background.
    {
        let obj = sdi.create("gallery_bg");
        obj.x = 0;
        obj.y = 0;
        obj.w = w;
        obj.h = h;
        obj.color = Color::rgb(240, 240, 240);
        obj.z = -100;
    }

    // Title.
    {
        let obj = sdi.create("gallery_title");
        obj.x = 8;
        obj.y = 4;
        obj.text = Some("Widget Gallery".to_string());
        obj.font_size = 14;
        obj.text_color = Color::rgb(0, 0, 0);
    }

    // Buttons.
    let button_labels = ["Normal", "Hover", "Pressed", "Disabled"];
    let button_colors = [
        Color::rgb(100, 149, 237), // Normal (cornflower blue)
        Color::rgb(120, 169, 255), // Hover
        Color::rgb(70, 119, 207),  // Pressed
        Color::rgb(180, 180, 180), // Disabled
    ];
    for (i, (label, color)) in button_labels.iter().zip(&button_colors).enumerate() {
        let x = 8 + (i as i32) * 58;
        let name = format!("btn_{i}");
        let obj = sdi.create(&name);
        obj.x = x;
        obj.y = 24;
        obj.w = 54;
        obj.h = 20;
        obj.color = *color;
        obj.border_radius = Some(4);
        let text_name = format!("btn_text_{i}");
        let tobj = sdi.create(&text_name);
        tobj.x = x + 4;
        tobj.y = 28;
        tobj.text = Some(label.to_string());
        tobj.font_size = 8;
        tobj.text_color = Color::rgb(255, 255, 255);
    }

    // Cards.
    for i in 0..3_i32 {
        let x = 8 + i * 80;
        let name = format!("card_{i}");
        let obj = sdi.create(&name);
        obj.x = x;
        obj.y = 52;
        obj.w = 76;
        obj.h = 50;
        obj.color = Color::rgb(255, 255, 255);
        obj.border_radius = Some(6);
        obj.shadow_level = Some(2);
        let title_name = format!("card_title_{i}");
        let tobj = sdi.create(&title_name);
        tobj.x = x + 4;
        tobj.y = 56;
        tobj.text = Some(format!("Card {}", i + 1));
        tobj.font_size = 10;
        tobj.text_color = Color::rgb(40, 40, 40);
    }

    // Progress bars.
    let percentages = [0, 50, 100];
    for (i, &pct) in percentages.iter().enumerate() {
        let y = 110 + (i as i32) * 18;
        // Track.
        let track = sdi.create(format!("prog_track_{i}"));
        track.x = 8;
        track.y = y;
        track.w = 200;
        track.h = 12;
        track.color = Color::rgb(200, 200, 200);
        track.border_radius = Some(6);
        // Fill.
        let fill_w = (200 * pct / 100).max(1) as u32;
        let fill = sdi.create(format!("prog_fill_{i}"));
        fill.x = 8;
        fill.y = y;
        fill.w = fill_w;
        fill.h = 12;
        fill.color = Color::rgb(76, 175, 80);
        fill.border_radius = Some(6);
        // Label.
        let label = sdi.create(format!("prog_label_{i}"));
        label.x = 212;
        label.y = y + 2;
        label.text = Some(format!("{pct}%"));
        label.font_size = 8;
        label.text_color = Color::rgb(60, 60, 60);
    }

    // Toggle switches.
    for (i, on) in [true, false].iter().enumerate() {
        let x = 8 + (i as i32) * 50;
        let y = 168;
        let track = sdi.create(format!("toggle_track_{i}"));
        track.x = x;
        track.y = y;
        track.w = 36;
        track.h = 16;
        track.color = if *on {
            Color::rgb(76, 175, 80)
        } else {
            Color::rgb(180, 180, 180)
        };
        track.border_radius = Some(8);

        let knob = sdi.create(format!("toggle_knob_{i}"));
        knob.x = if *on { x + 20 } else { x + 2 };
        knob.y = y + 2;
        knob.w = 12;
        knob.h = 12;
        knob.color = Color::rgb(255, 255, 255);
        knob.border_radius = Some(6);

        let label = sdi.create(format!("toggle_label_{i}"));
        label.x = x;
        label.y = y + 20;
        label.text = Some(if *on { "ON" } else { "OFF" }.to_string());
        label.font_size = 8;
        label.text_color = Color::rgb(60, 60, 60);
    }

    // Text fields.
    let field_contents = ["", "Hello, OASIS!", "Cursor here|"];
    for (i, &text) in field_contents.iter().enumerate() {
        let y = 200 + (i as i32) * 22;
        let bg = sdi.create(format!("field_bg_{i}"));
        bg.x = 8;
        bg.y = y;
        bg.w = 180;
        bg.h = 18;
        bg.color = Color::rgb(255, 255, 255);
        bg.border_radius = Some(3);
        bg.stroke_width = Some(1);
        bg.stroke_color = Some(Color::rgb(180, 180, 180));

        let txt = sdi.create(format!("field_text_{i}"));
        txt.x = 12;
        txt.y = y + 4;
        txt.text = if text.is_empty() {
            Some("Placeholder...".to_string())
        } else {
            Some(text.to_string())
        };
        txt.font_size = 8;
        txt.text_color = if text.is_empty() {
            Color::rgb(160, 160, 160)
        } else {
            Color::rgb(0, 0, 0)
        };
    }

    // Tab bar.
    let tabs = ["Home", "Browse", "Settings", "About"];
    for (i, &tab) in tabs.iter().enumerate() {
        let x = 250 + (i as i32) * 56;
        let active = i == 0;
        let bg = sdi.create(format!("tab_bg_{i}"));
        bg.x = x;
        bg.y = 24;
        bg.w = 54;
        bg.h = 20;
        bg.color = if active {
            Color::rgb(100, 149, 237)
        } else {
            Color::rgb(220, 220, 220)
        };
        bg.border_radius = Some(4);

        let label = sdi.create(format!("tab_label_{i}"));
        label.x = x + 4;
        label.y = 28;
        label.text = Some(tab.to_string());
        label.font_size = 8;
        label.text_color = if active {
            Color::rgb(255, 255, 255)
        } else {
            Color::rgb(60, 60, 60)
        };
    }

    // List view items.
    let list_items = ["Item 1", "Item 2 (selected)", "Item 3", "Item 4", "Item 5"];
    for (i, &item) in list_items.iter().enumerate() {
        let y = 52 + (i as i32) * 18;
        let selected = i == 1;
        let bg = sdi.create(format!("list_bg_{i}"));
        bg.x = 250;
        bg.y = y;
        bg.w = 220;
        bg.h = 17;
        bg.color = if selected {
            Color::rgb(100, 149, 237)
        } else if i % 2 == 0 {
            Color::rgb(248, 248, 248)
        } else {
            Color::rgb(255, 255, 255)
        };

        let label = sdi.create(format!("list_label_{i}"));
        label.x = 254;
        label.y = y + 4;
        label.text = Some(item.to_string());
        label.font_size = 8;
        label.text_color = if selected {
            Color::rgb(255, 255, 255)
        } else {
            Color::rgb(40, 40, 40)
        };
    }

    render_and_save(backend, &mut sdi, w, h, &out_dir.join("actual.png"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Window manager scenarios
// ---------------------------------------------------------------------------

fn run_wm_scenario(
    backend: &mut SdlBackend,
    scenario: &str,
    out_dir: &Path,
    w: u32,
    h: u32,
) -> anyhow::Result<()> {
    let skin = resolve_skin("classic")?;
    let active_theme = ActiveTheme::from_skin(&skin.theme);

    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);

    // Wallpaper.
    let wp_data = wallpaper::generate_from_config(w, h, &active_theme);
    let wallpaper_tex = backend.load_texture(w, h, &wp_data)?;
    {
        let obj = sdi.create("wallpaper");
        obj.x = 0;
        obj.y = 0;
        obj.w = w;
        obj.h = h;
        obj.texture = Some(wallpaper_tex);
        obj.z = -1000;
    }

    let mut wm = WindowManager::new(w, h);

    match scenario {
        "wm_single_maximized" => {
            let cfg = WindowConfig {
                id: "max_win".to_string(),
                title: "Maximized Window".to_string(),
                x: Some(10),
                y: Some(10),
                width: 200,
                height: 150,
                window_type: WindowType::AppWindow,
            };
            wm.create_window(&cfg, &mut sdi)?;
            wm.maximize_window("max_win", &mut sdi)?;
        }
        "wm_cascaded_windows" => {
            let configs = [
                ("win_a", "File Manager", 20, 30),
                ("win_b", "Settings", 50, 60),
                ("win_c", "Browser", 80, 90),
            ];
            for (id, title, x, y) in &configs {
                let cfg = WindowConfig {
                    id: id.to_string(),
                    title: title.to_string(),
                    x: Some(*x),
                    y: Some(*y),
                    width: 200,
                    height: 140,
                    window_type: WindowType::AppWindow,
                };
                wm.create_window(&cfg, &mut sdi)?;
            }
            // Focus middle window for visual interest.
            wm.focus_window("win_b", &mut sdi)?;
        }
        "wm_dialog_overlay" => {
            let app_cfg = WindowConfig {
                id: "app_win".to_string(),
                title: "Application".to_string(),
                x: Some(30),
                y: Some(40),
                width: 260,
                height: 180,
                window_type: WindowType::AppWindow,
            };
            wm.create_window(&app_cfg, &mut sdi)?;

            let dlg_cfg = WindowConfig {
                id: "dialog".to_string(),
                title: "Confirm Action".to_string(),
                x: Some(100),
                y: Some(80),
                width: 180,
                height: 100,
                window_type: WindowType::Dialog,
            };
            wm.create_window(&dlg_cfg, &mut sdi)?;
        }
        _ => {
            log::warn!("Unknown WM scenario: {scenario}");
        }
    }

    render_and_save(backend, &mut sdi, w, h, &out_dir.join("actual.png"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// HTML report generation
// ---------------------------------------------------------------------------

fn generate_report(base_dir: &Path, scenarios: &[Scenario]) -> anyhow::Result<()> {
    let mut html = String::from(
        "<!DOCTYPE html>\n\
         <html><head>\n\
         <title>OASIS_OS Screenshot Test Report</title>\n\
         <style>\n\
           body { font-family: sans-serif; margin: 20px; background: #f5f5f5; }\n\
           h1 { color: #333; }\n\
           .grid { display: flex; flex-wrap: wrap; gap: 16px; }\n\
           .card { background: white; border-radius: 8px; padding: 12px;\n\
                   box-shadow: 0 2px 4px rgba(0,0,0,0.1); max-width: 500px; }\n\
           .card h3 { margin: 0 0 8px 0; font-size: 14px; color: #555; }\n\
           .card img { max-width: 480px; border: 1px solid #ddd; image-rendering: pixelated; }\n\
           .category { margin: 24px 0 8px 0; color: #666; border-bottom: 1px solid #ddd;\n\
                        padding-bottom: 4px; }\n\
         </style>\n\
         </head><body>\n\
         <h1>OASIS_OS Screenshot Test Report</h1>\n",
    );

    let mut current_category = "";
    for scenario in scenarios {
        if scenario.category != current_category {
            current_category = scenario.category;
            html.push_str(&format!("<h2 class=\"category\">{current_category}</h2>\n"));
            html.push_str("<div class=\"grid\">\n");
        }

        let img_path = format!("{}/actual.png", scenario.name);
        let full_path = base_dir.join(&scenario.name).join("actual.png");
        if full_path.exists() {
            html.push_str(&format!(
                "<div class=\"card\">\n\
                   <h3>{}</h3>\n\
                   <img src=\"{}\" alt=\"{}\">\n\
                 </div>\n",
                scenario.name, img_path, scenario.name
            ));
        }
    }
    html.push_str("</div>\n</body></html>\n");

    fs::write(base_dir.join("report.html"), &html)?;
    println!("Report saved to {}/report.html", base_dir.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = parse_args();

    let config = OasisConfig::default();
    let w = config.screen_width;
    let h = config.screen_height;

    let mut backend = SdlBackend::new("OASIS Screenshot Tests", w, h)?;
    backend.init(w, h)?;

    let base_dir = PathBuf::from("screenshots/tests");
    fs::create_dir_all(&base_dir)?;

    let scenarios = all_scenarios();
    let mut completed = 0;
    let mut failed = 0;

    for scenario in &scenarios {
        // Apply filters.
        if let Some(ref filter) = args.scenario_filter
            && !scenario.name.contains(filter.as_str())
        {
            continue;
        }
        if let Some(ref skin_filter) = args.skin_filter
            && scenario.category == "skin"
            && !scenario.name.starts_with(skin_filter.as_str())
        {
            continue;
        }

        let out_dir = base_dir.join(&scenario.name);
        fs::create_dir_all(&out_dir)?;

        log::info!("Running scenario: {}", scenario.name);

        let result = match scenario.category {
            "skin" => {
                // Parse "{skin}_{view}" from name.
                let all_skins = all_skin_names();
                let (skin, view) = all_skins
                    .iter()
                    .find_map(|s| {
                        scenario
                            .name
                            .strip_prefix(s.as_str())
                            .and_then(|rest| rest.strip_prefix('_'))
                            .map(|view| (s.as_str(), view))
                    })
                    .unwrap_or(("classic", "dashboard"));
                run_skin_scenario(&mut backend, skin, view, &out_dir, w, h)
            }
            "browser" => {
                let page = scenario
                    .name
                    .strip_prefix("browser_")
                    .unwrap_or(&scenario.name);
                run_browser_scenario(&mut backend, page, &out_dir, w, h)
            }
            "widget" => run_widget_gallery(&mut backend, &out_dir, w, h),
            "wm" => run_wm_scenario(&mut backend, &scenario.name, &out_dir, w, h),
            _ => {
                log::warn!("Unknown category: {}", scenario.category);
                Ok(())
            }
        };

        match result {
            Ok(()) => {
                completed += 1;
                println!("  OK  {}", scenario.name);
            }
            Err(e) => {
                failed += 1;
                eprintln!("  FAIL  {}: {e}", scenario.name);
            }
        }
    }

    if args.report {
        generate_report(&base_dir, &scenarios)?;
    }

    backend.shutdown()?;

    println!();
    println!("Screenshot tests: {completed} passed, {failed} failed");
    if completed + failed == 0 {
        println!("(No scenarios matched the filter)");
    }

    Ok(())
}
