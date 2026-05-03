#![allow(clippy::unwrap_used)] // Test binary -- unwrap is acceptable.
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
//!   cargo run -p oasis-app --bin screenshot-tests -- --bless   # save golden baselines
//!   cargo run -p oasis-app --bin screenshot-tests -- --check   # compare against golden
//!
//! Output:
//!   screenshots/tests/{scenario}/actual.png
//!   screenshots/tests/{scenario}/golden.png  (with --bless)
//!   screenshots/tests/report.html            (with --report)

use std::fs;
use std::path::{Path, PathBuf};

use oasis_backend_sdl::SdlBackend;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::tv_guide::TvGuideState;
use oasis_core::apps::tv_guide::channel::{ChannelConfig, DEFAULT_CHANNELS_TOML};
use oasis_core::backend::{Color, SdiCore};
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
    /// Compare actual screenshots against golden files (exit 1 on mismatch).
    check: bool,
    /// Copy actual screenshots to golden files (first-time baseline).
    bless: bool,
    /// Render full-page browser screenshots (scrolls entire content height).
    full_page: bool,
    /// Override the render size. `None` uses `OasisConfig::default()`
    /// (480x272 PSP native). Browser fixtures designed for desktop
    /// widths are more legibly captured at e.g. 1024x768.
    size: Option<(u32, u32)>,
}

fn parse_args() -> Args {
    let mut args = Args {
        scenario_filter: None,
        skin_filter: None,
        report: false,
        check: false,
        bless: false,
        full_page: false,
        size: None,
    };
    let usage = "Usage: screenshot-tests [--scenario NAME] [--skin NAME] \
                 [--report] [--check] [--bless] [--full-page] [--size WxH]";
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--scenario" => {
                args.scenario_filter = Some(iter.next().unwrap_or_else(|| {
                    eprintln!("--scenario requires a value");
                    eprintln!("{usage}");
                    std::process::exit(1);
                }));
            },
            "--skin" => {
                args.skin_filter = Some(iter.next().unwrap_or_else(|| {
                    eprintln!("--skin requires a value");
                    eprintln!("{usage}");
                    std::process::exit(1);
                }));
            },
            "--report" => args.report = true,
            "--check" => args.check = true,
            "--bless" => args.bless = true,
            "--full-page" => args.full_page = true,
            "--size" => {
                let val = iter.next().unwrap_or_else(|| {
                    eprintln!("--size requires WxH (e.g. 1024x768)");
                    eprintln!("{usage}");
                    std::process::exit(1);
                });
                let (wstr, hstr) = val.split_once('x').unwrap_or_else(|| {
                    eprintln!("--size expects WxH, got {val:?}");
                    std::process::exit(1);
                });
                let w: u32 = wstr.parse().unwrap_or_else(|_| {
                    eprintln!("invalid width in --size: {wstr}");
                    std::process::exit(1);
                });
                let h: u32 = hstr.parse().unwrap_or_else(|_| {
                    eprintln!("invalid height in --size: {hstr}");
                    std::process::exit(1);
                });
                args.size = Some((w, h));
            },
            other => {
                eprintln!("Unknown argument: {other}");
                eprintln!("{usage}");
                std::process::exit(1);
            },
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
        "images",
        "wikipedia",
        "wikipedia_real",
        "google",
        "wiki_article",
        "news_article",
        "font_stack",
        "font_face",
        "font_inherit",
        "web_fonts",
        "reddit_listing",
        "reddit_comments",
        "reddit_listing_real",
        "reddit_comments_real",
        "reddit_listing_inlinecss",
        "reddit_comments_inlinecss",
        "reddit_listing_basecss",
        "bfc_float_test",
        "direct_image_url",
    ];
    for page in &pages {
        scenarios.push(Scenario {
            name: format!("browser_{page}"),
            category: "browser",
        });
    }

    // Live network scenarios. Opt-in via `OASIS_NETWORK_SCREENSHOTS=1`
    // so normal runs never flake on DNS / latency.
    if std::env::var("OASIS_NETWORK_SCREENSHOTS").ok().as_deref() == Some("1") {
        let live_urls = [
            ("wikipedia_live", "https://www.wikipedia.org/"),
            ("github_live", "https://github.com/"),
        ];
        for (name, _url) in &live_urls {
            scenarios.push(Scenario {
                name: format!("browser_{name}"),
                category: "browser_live",
            });
        }
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

    // TV Guide scenarios.
    let tv_views = [
        "tv_guide_loading",
        "tv_guide_error",
        "tv_guide_populated",
        "tv_guide_tuned",
    ];
    for view in &tv_views {
        scenarios.push(Scenario {
            name: view.to_string(),
            category: "tv_guide",
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

/// Render the full browser page by scrolling in strips and compositing.
///
/// After the first paint, reads `content_height` from the scroll state.
/// Then renders each viewport-height strip, compositing into a single
/// tall RGBA buffer. The chrome (URL bar) is included in the first strip
/// only; subsequent strips show only page content.
fn render_browser_fullpage_and_save(
    backend: &mut SdlBackend,
    browser: &mut BrowserWidget,
    w: u32,
    h: u32,
    path: &Path,
) -> anyhow::Result<()> {
    // First paint to compute layout and content_height.
    backend.clear(Color::rgb(255, 255, 255))?;
    browser.paint(backend)?;
    backend.swap_buffers()?;

    let content_height = browser.scroll().content_height;
    let viewport_h = browser.scroll().viewport_height;

    if content_height <= viewport_h || viewport_h <= 0 {
        // Content fits in viewport -- single-frame capture.
        backend.clear(Color::rgb(255, 255, 255))?;
        browser.paint(backend)?;
        let pixels = backend.read_pixels(0, 0, w, h)?;
        save_png(path, w, h, &pixels)?;
        return Ok(());
    }

    // Calculate total output height: chrome area + full content.
    let chrome_h = h - viewport_h as u32;
    let total_h = chrome_h + content_height as u32;
    let row_bytes = (w * 4) as usize;
    let mut full_pixels = vec![255u8; row_bytes * total_h as usize];

    // Render in strips by scrolling.
    let mut y_offset: u32 = 0;
    let mut scroll_y: i32 = 0;

    while (y_offset as i32) < total_h as i32 {
        browser.scroll_mut().scroll_y = scroll_y;

        backend.clear(Color::rgb(255, 255, 255))?;
        browser.paint(backend)?;

        let strip = backend.read_pixels(0, 0, w, h)?;

        if y_offset == 0 {
            // First strip: copy the entire frame (chrome + content).
            let copy_rows = (h as usize).min(total_h as usize);
            let copy_bytes = copy_rows * row_bytes;
            full_pixels[..copy_bytes].copy_from_slice(&strip[..copy_bytes]);
            y_offset = h;
            scroll_y += viewport_h;
        } else {
            // Subsequent strips: copy only the content area (skip chrome).
            let src_start = chrome_h as usize * row_bytes;
            let remaining = total_h as usize - y_offset as usize;
            let copy_rows = (viewport_h as usize).min(remaining);
            let dst_start = y_offset as usize * row_bytes;
            let copy_bytes = copy_rows * row_bytes;
            if src_start + copy_bytes <= strip.len() && dst_start + copy_bytes <= full_pixels.len()
            {
                full_pixels[dst_start..dst_start + copy_bytes]
                    .copy_from_slice(&strip[src_start..src_start + copy_bytes]);
            }
            y_offset += copy_rows as u32;
            scroll_y += viewport_h;
        }
    }

    // Reset scroll position.
    browser.scroll_mut().scroll_y = 0;

    save_png(path, w, total_h, &full_pixels)?;
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

// ---------------------------------------------------------------------------
// Golden file comparison (--check / --bless)
// ---------------------------------------------------------------------------

/// Read a PNG file and return raw RGBA bytes.
fn read_png_rgba(path: &Path) -> anyhow::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    reader.next_frame(&mut buf)?;
    let info = reader.info();
    buf.truncate((info.width * info.height * 4) as usize);
    Ok(buf)
}

/// Compute a simple hash of pixel data for fast comparison.
///
/// Uses FNV-1a (64-bit) which is trivial to implement and fast on small data.
fn hash_pixels(data: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Bless a scenario: copy actual.png to golden.png.
fn bless_golden(out_dir: &Path) -> anyhow::Result<()> {
    let actual = out_dir.join("actual.png");
    let golden = out_dir.join("golden.png");
    if actual.exists() {
        fs::copy(&actual, &golden)?;
    }
    Ok(())
}

/// Check a scenario: compare actual.png against golden.png.
///
/// Returns `Ok(true)` if they match (within threshold), `Ok(false)` if they
/// differ, or `Err` if golden.png doesn't exist.
fn check_golden(out_dir: &Path) -> anyhow::Result<bool> {
    let actual = out_dir.join("actual.png");
    let golden = out_dir.join("golden.png");
    if !golden.exists() {
        anyhow::bail!(
            "golden.png not found (run with --bless first): {}",
            golden.display()
        );
    }
    let actual_pixels = read_png_rgba(&actual)?;
    let golden_pixels = read_png_rgba(&golden)?;

    // Fast-path: exact hash match.
    if hash_pixels(&actual_pixels) == hash_pixels(&golden_pixels) {
        return Ok(true);
    }

    // If sizes differ, they definitely don't match.
    if actual_pixels.len() != golden_pixels.len() {
        let diff_path = out_dir.join("diff.txt");
        fs::write(
            &diff_path,
            format!(
                "size mismatch: actual={} golden={}",
                actual_pixels.len(),
                golden_pixels.len()
            ),
        )?;
        return Ok(false);
    }

    // Pixel-diff: count differing pixels.
    let total_pixels = actual_pixels.len() / 4;
    let mut diff_count = 0u64;
    for i in 0..total_pixels {
        let base = i * 4;
        if actual_pixels[base] != golden_pixels[base]
            || actual_pixels[base + 1] != golden_pixels[base + 1]
            || actual_pixels[base + 2] != golden_pixels[base + 2]
            || actual_pixels[base + 3] != golden_pixels[base + 3]
        {
            diff_count += 1;
        }
    }

    let diff_pct = (diff_count as f64 / total_pixels as f64) * 100.0;
    let diff_path = out_dir.join("diff.txt");
    fs::write(
        &diff_path,
        format!("{diff_count}/{total_pixels} pixels differ ({diff_pct:.2}%)"),
    )?;

    // Generate diff image: red pixels where differences exist.
    generate_diff_image(out_dir, &actual_pixels, &golden_pixels)?;

    // Threshold: <0.1% difference counts as a match.
    Ok(diff_pct < 0.1)
}

/// Generate a diff image highlighting pixel differences in red.
fn generate_diff_image(out_dir: &Path, actual: &[u8], golden: &[u8]) -> anyhow::Result<()> {
    let total_pixels = actual.len() / 4;
    let mut diff = vec![0u8; actual.len()];
    for i in 0..total_pixels {
        let base = i * 4;
        let differs = actual[base] != golden[base]
            || actual[base + 1] != golden[base + 1]
            || actual[base + 2] != golden[base + 2]
            || actual[base + 3] != golden[base + 3];
        if differs {
            diff[base] = 255; // R
            diff[base + 1] = 0; // G
            diff[base + 2] = 0; // B
            diff[base + 3] = 255; // A
        } else {
            // Dim the matching pixels.
            diff[base] = actual[base] / 3;
            diff[base + 1] = actual[base + 1] / 3;
            diff[base + 2] = actual[base + 2] / 3;
            diff[base + 3] = 255;
        }
    }
    // We need the image dimensions to save. Read them from golden.png.
    let golden_path = out_dir.join("golden.png");
    let file = fs::File::open(&golden_path)?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let reader = decoder.read_info()?;
    let info = reader.info();
    save_png(&out_dir.join("diff.png"), info.width, info.height, &diff)?;
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
        "Internet Radio",
        "Photo Viewer",
        "Package Manager",
        "System Monitor",
        "Browser",
        "TV Guide",
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
    let mut dashboard = DashboardState::new(dash_config, apps);
    let mut status_bar = StatusBar::new();
    let mut bottom_bar = BottomBar::new();
    bottom_bar.total_pages = dashboard.page_count();

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
    mouse_cursor.scale = active_theme.cursor_scale;
    {
        let (cursor_pixels, cw, ch) = cursor::generate_cursor_pixels(active_theme.cursor_scale);
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
    bottom_bar.update_info(time.as_ref());

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
        },
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
        },
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
        },
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
                    always_on_top: false,
                    modal: false,
                },
                WindowConfig {
                    id: "win2".to_string(),
                    title: "Settings".to_string(),
                    x: Some(80),
                    y: Some(60),
                    width: 180,
                    height: 130,
                    window_type: WindowType::AppWindow,
                    always_on_top: false,
                    modal: false,
                },
                WindowConfig {
                    id: "win3".to_string(),
                    title: "Terminal".to_string(),
                    x: Some(140),
                    y: Some(90),
                    width: 220,
                    height: 140,
                    window_type: WindowType::AppWindow,
                    always_on_top: false,
                    modal: false,
                },
            ];
            for cfg in &configs {
                wm.create_window(cfg, &mut sdi)?;
            }
            mouse_cursor.set_position(250, 130);
            mouse_cursor.update_sdi(&mut sdi);
            render_and_save(backend, &mut sdi, w, h, &out_dir.join("actual.png"))?;
        },
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
        },
        _ => {
            log::warn!("Unknown skin view: {view}");
        },
    }

    // Clean up textures to avoid resource leaks.
    backend.destroy_texture(wallpaper_tex)?;
    if let Ok(obj) = sdi.get_mut("mouse_cursor")
        && let Some(tex) = obj.texture.take()
    {
        backend.destroy_texture(tex)?;
    }

    Ok(())
}

/// Return the canonical https base URL for a `*_real` fixture so
/// protocol-relative and root-relative resources resolve to the live
/// site during `OASIS_LIVE_CSS=1` captures.
fn reddit_real_base_url(page_name: &str) -> Option<&'static str> {
    match page_name {
        "reddit_listing_real" => Some("https://old.reddit.com/r/rust/"),
        "reddit_comments_real" => Some("https://old.reddit.com/r/rust/comments/"),
        _ => None,
    }
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
    full_page: bool,
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
        },
        "images" => {
            // Use navigate_vfs so images are decoded from VFS.
            let vfs = make_image_test_vfs();
            browser.navigate_vfs("vfs://test/images.html", &vfs);
            String::new() // Already loaded.
        },
        "direct_image_url" => {
            // Direct navigation to an image URL: the engine wraps the
            // bytes in `<html><body><img>` chrome. Validates the
            // `process_response` image branch end-to-end.
            let vfs = make_image_test_vfs();
            browser.navigate_vfs("vfs://test/red_16x16.bmp", &vfs);
            for _ in 0..5 {
                browser.tick(&vfs);
            }
            String::new()
        },
        "web_fonts" => {
            // Use navigate_vfs so the TTF font file resolves from VFS.
            let vfs = make_web_fonts_test_vfs();
            browser.navigate_vfs("vfs://test/web_fonts.html", &vfs);
            // Pump tick() to trigger font loading.
            for _ in 0..5 {
                browser.tick(&vfs);
            }
            String::new() // Already loaded.
        },
        _ => {
            let fixture_path = format!("test-fixtures/html/{page_name}.html");
            let content = fs::read_to_string(&fixture_path).unwrap_or_else(|_| {
                format!("<html><body><p>Missing fixture: {fixture_path}</p></body></html>")
            });
            // For `*_real` fixtures (real-world HTML snapshots), enable a
            // TLS provider and tick the widget until external stylesheet
            // fetches settle. These fixtures reference external CSS files
            // on the live web, and without fetching them old.reddit renders
            // as an unstyled vertical list. Opt-in via `OASIS_LIVE_CSS=1`
            // so the default test run never depends on network.
            let live_css = page_name.ends_with("_real")
                && std::env::var("OASIS_LIVE_CSS").ok().as_deref() == Some("1");
            if live_css {
                use oasis_core::net::RustlsTlsProvider;
                browser.set_tls_provider(Box::new(RustlsTlsProvider::new()));
            }
            // Real-world fixtures (`*_real`) get an https base URL so
            // protocol-relative references like `//cdn/foo.css` resolve
            // to https rather than file://. Offline fixtures keep the
            // original file:// scheme so their relative references stay
            // within the fixture directory.
            let base_url = if live_css {
                reddit_real_base_url(page_name)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("file://test/{page_name}.html"))
            } else {
                format!("file://test/{page_name}.html")
            };
            browser.load_html(&content, &base_url);
            if live_css {
                let empty_vfs = MemoryVfs::new();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
                loop {
                    browser.tick(&empty_vfs);
                    let in_flight = browser.io_thread_in_flight().unwrap_or(0);
                    if in_flight == 0 {
                        break;
                    }
                    if std::time::Instant::now() >= deadline {
                        log::warn!(
                            "{}: {} external stylesheet(s) still pending at deadline",
                            page_name,
                            in_flight
                        );
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                // One more tick so any final apply gets picked up.
                browser.tick(&empty_vfs);
            }
            content
        },
    };
    let _ = html; // Suppress unused warning.

    render_browser_and_save(backend, &mut browser, w, h, &out_dir.join("actual.png"))?;

    if full_page {
        render_browser_fullpage_and_save(
            backend,
            &mut browser,
            w,
            h,
            &out_dir.join("fullpage.png"),
        )?;
    }
    Ok(())
}

/// Fetch a real-world URL over the network and render the result.
///
/// Sets up a `RustlsTlsProvider` on the widget, kicks off a
/// `navigate_vfs` against an HTTPS URL, then spins calling `tick()`
/// until the I/O thread completes (or a 30s deadline elapses). Verifies
/// that the final state is not an error — the underlying reason this
/// exists is the HTTP/2 landing, which only takes effect when the
/// `SharedTlsProvider` forwarder in `widget/pipeline.rs` forwards ALPN
/// into the real rustls provider.
fn run_browser_live_scenario(
    backend: &mut SdlBackend,
    page_name: &str,
    out_dir: &Path,
    w: u32,
    h: u32,
) -> anyhow::Result<()> {
    use oasis_core::browser::LoadingState;
    use oasis_core::net::RustlsTlsProvider;

    let url = match page_name {
        "wikipedia_live" => "https://www.wikipedia.org/",
        "github_live" => "https://github.com/",
        other => anyhow::bail!("unknown live scenario: {other}"),
    };

    let mut browser = BrowserWidget::new(BrowserConfig::default());
    browser.set_window(0, 0, w, h);
    browser.set_tls_provider(Box::new(RustlsTlsProvider::new()));

    // The navigate_vfs entry point handles http(s) URLs by routing
    // through the I/O thread when TLS is configured. An empty VFS is
    // fine — it will miss and fall through to network.
    let empty_vfs = MemoryVfs::new();
    browser.navigate_vfs(url, &empty_vfs);

    // Pump tick() until the load resolves or we time out.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        browser.tick(&empty_vfs);
        match browser.loading_state() {
            LoadingState::Idle => break,
            LoadingState::Error => {
                anyhow::bail!(
                    "live load for {url} failed: {}",
                    browser.error_message().unwrap_or("(no message)")
                );
            },
            LoadingState::Loading => {},
        }
        if std::time::Instant::now() > deadline {
            anyhow::bail!("live load for {url} timed out after 30s");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Final paint + capture.
    render_browser_and_save(backend, &mut browser, w, h, &out_dir.join("actual.png"))?;
    Ok(())
}

/// Build a VFS for the images screenshot test with an inline BMP and
/// the HTML fixture from test-fixtures/html/images.html.
fn make_image_test_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/test").ok();

    // Read the fixture HTML.
    let html = fs::read_to_string("test-fixtures/html/images.html")
        .unwrap_or_else(|_| "<html><body><p>Missing images.html fixture</p></body></html>".into());
    vfs.write("/test/images.html", html.as_bytes()).unwrap();

    // Create a minimal 16x16 24-bit BMP (solid red).
    let bmp = make_test_bmp_16x16();
    vfs.write("/test/red_16x16.bmp", &bmp).unwrap();

    vfs
}

/// Build a minimal 16x16 solid-red 24-bit BMP for image testing.
fn make_test_bmp_16x16() -> Vec<u8> {
    let w: u32 = 16;
    let h: u32 = 16;
    let bpp: u16 = 24;
    let row_bytes = (w * 3).div_ceil(4) * 4;
    let pixel_data_size = row_bytes * h;
    let file_size = 54 + pixel_data_size;

    let mut bmp = vec![0u8; file_size as usize];
    bmp[0] = b'B';
    bmp[1] = b'M';
    bmp[2..6].copy_from_slice(&file_size.to_le_bytes());
    bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&(w as i32).to_le_bytes());
    bmp[22..26].copy_from_slice(&(h as i32).to_le_bytes());
    bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&bpp.to_le_bytes());
    bmp[30..34].copy_from_slice(&0u32.to_le_bytes());

    // Fill with solid red (BGR = 0,0,255).
    for row in 0..h {
        for col in 0..w {
            let off = 54 + (row * row_bytes + col * 3) as usize;
            if off + 2 < bmp.len() {
                bmp[off] = 0;
                bmp[off + 1] = 0;
                bmp[off + 2] = 255;
            }
        }
    }
    bmp
}

/// Build a VFS for the web_fonts screenshot test with the HTML fixture
/// and the minimal test TTF.
fn make_web_fonts_test_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/test").ok();

    // Read the fixture HTML.
    let html = fs::read_to_string("test-fixtures/html/web_fonts.html").unwrap_or_else(|_| {
        "<html><body><p>Missing web_fonts.html fixture</p></body></html>".into()
    });
    vfs.write("/test/web_fonts.html", html.as_bytes()).unwrap();

    // Include the minimal test TTF from oasis-browser's test data.
    let ttf = include_bytes!("../../oasis-browser/test_data/minimal.ttf");
    vfs.write("/test/test-font.ttf", ttf).unwrap();

    vfs
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
                always_on_top: false,
                modal: false,
            };
            wm.create_window(&cfg, &mut sdi)?;
            wm.maximize_window("max_win", &mut sdi)?;
        },
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
                    always_on_top: false,
                    modal: false,
                };
                wm.create_window(&cfg, &mut sdi)?;
            }
            // Focus middle window for visual interest.
            wm.focus_window("win_b", &mut sdi)?;
        },
        "wm_dialog_overlay" => {
            let app_cfg = WindowConfig {
                id: "app_win".to_string(),
                title: "Application".to_string(),
                x: Some(30),
                y: Some(40),
                width: 260,
                height: 180,
                window_type: WindowType::AppWindow,
                always_on_top: false,
                modal: false,
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
                always_on_top: false,
                modal: false,
            };
            wm.create_window(&dlg_cfg, &mut sdi)?;
        },
        _ => {
            log::warn!("Unknown WM scenario: {scenario}");
        },
    }

    render_and_save(backend, &mut sdi, w, h, &out_dir.join("actual.png"))?;

    // Clean up textures to avoid resource leaks.
    backend.destroy_texture(wallpaper_tex)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// TV Guide scenarios
// ---------------------------------------------------------------------------

fn run_tv_guide_scenario(
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

    // Create a window for the TV Guide.
    let cfg = WindowConfig {
        id: "tv_guide".to_string(),
        title: "TV Guide".to_string(),
        x: Some(20),
        y: Some(20),
        width: w.saturating_sub(40),
        height: h.saturating_sub(40),
        window_type: WindowType::AppWindow,
        always_on_top: false,
        modal: false,
    };
    wm.create_window(&cfg, &mut sdi)?;

    // Build guide state based on scenario variant.
    let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML)?;
    let mut guide = TvGuideState::new(&config, &ActiveTheme::default());

    match scenario {
        "tv_guide_loading" => {
            // Default state: no catalogs, no fetch attempted.
        },
        "tv_guide_error" => {
            guide.fetch_attempted = true;
            guide.fetch_error = Some("Network timeout: archive.org".to_string());
        },
        "tv_guide_populated" => {
            // Inject mock catalogs for all channels.
            for (i, ch) in guide.channels.clone().iter().enumerate() {
                let catalog = oasis_core::apps::tv_guide::catalog::ChannelCatalog::new(ch.number);
                let mut cat = catalog;
                let episodes: Vec<oasis_core::apps::tv_guide::VideoEpisode> = (0..5)
                    .map(|j| oasis_core::apps::tv_guide::VideoEpisode {
                        item_id: format!("mock-{}-{j}", ch.number),
                        filename: format!("ep{j:02}.mp4"),
                        title: format!("Episode {}", j + 1),
                        duration_secs: 1800.0,
                        width: 640,
                        height: 480,
                        size_bytes: 50_000_000,
                        format: "MPEG4".into(),
                        original: None,
                    })
                    .collect();
                cat.add_episodes(episodes);
                guide.catalogs[i] = Some(cat);
                guide.rebuild_cached_schedule(i);
            }
            guide.fetch_attempted = true;
        },
        "tv_guide_tuned" => {
            // Populated + tuned to channel 0.
            for (i, ch) in guide.channels.clone().iter().enumerate() {
                let mut cat = oasis_core::apps::tv_guide::catalog::ChannelCatalog::new(ch.number);
                let episodes: Vec<oasis_core::apps::tv_guide::VideoEpisode> = (0..5)
                    .map(|j| oasis_core::apps::tv_guide::VideoEpisode {
                        item_id: format!("mock-{}-{j}", ch.number),
                        filename: format!("ep{j:02}.mp4"),
                        title: format!("Episode {}", j + 1),
                        duration_secs: 1800.0,
                        width: 640,
                        height: 480,
                        size_bytes: 50_000_000,
                        format: "MPEG4".into(),
                        original: None,
                    })
                    .collect();
                cat.add_episodes(episodes);
                guide.catalogs[i] = Some(cat);
                guide.rebuild_cached_schedule(i);
            }
            guide.fetch_attempted = true;
            guide.tuned_channel = Some(0);
        },
        _ => {
            log::warn!("Unknown TV Guide scenario: {scenario}");
        },
    }

    // Render the guide's SDI objects.
    guide.update_sdi(&mut sdi, &active_theme);

    // Also render text content into the window via draw_with_clips.
    let lines = guide.text_content();
    wm.draw_with_clips(&mut sdi, backend, |window_id, cx, cy, cw, ch, be| {
        if window_id == "tv_guide" {
            // Draw content background.
            be.fill_rect(cx, cy, cw, ch, active_theme.app.bg)?;
            // Draw title.
            be.draw_text(
                "TV Guide",
                cx + 4,
                cy + 2,
                12,
                active_theme.app.title_bar_text,
            )?;
            be.fill_rect(
                cx,
                cy + active_theme.app.title_bar_height as i32 - 4,
                cw,
                1,
                active_theme.app.divider,
            )?;
            // Draw text lines.
            let line_h = active_theme.terminal_line_height.max(12) as i32;
            let max_lines = ((ch as i32 - line_h - 4) / line_h).max(0) as usize;
            for (i, line) in lines.iter().take(max_lines).enumerate() {
                let y = cy + active_theme.app.title_bar_height as i32 + i as i32 * line_h;
                be.draw_text(line, cx + 4, y, 12, active_theme.app.text)?;
            }
        }
        Ok(())
    })?;

    render_and_save(backend, &mut sdi, w, h, &out_dir.join("actual.png"))?;

    backend.destroy_texture(wallpaper_tex)?;

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
            if !current_category.is_empty() {
                html.push_str("</div>\n");
            }
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
                   <img src=\"{}\" alt=\"{}\">\n",
                scenario.name, img_path, scenario.name
            ));
            // Include full-page screenshot if it exists.
            let fp_path = base_dir.join(&scenario.name).join("fullpage.png");
            if fp_path.exists() {
                let fp_img = format!("{}/fullpage.png", scenario.name);
                html.push_str(&format!(
                    "  <details><summary>Full page</summary>\n\
                       <img src=\"{fp_img}\" alt=\"{name} full page\" \
                       style=\"max-width:480px\">\n\
                       </details>\n",
                    name = scenario.name,
                ));
            }
            html.push_str("</div>\n");
        }
    }
    if !current_category.is_empty() {
        html.push_str("</div>\n");
    }
    html.push_str("</body></html>\n");

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
    let (w, h) = args
        .size
        .unwrap_or((config.screen_width, config.screen_height));

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
            },
            "browser" => {
                let page = scenario
                    .name
                    .strip_prefix("browser_")
                    .unwrap_or(&scenario.name);
                run_browser_scenario(&mut backend, page, &out_dir, w, h, args.full_page)
            },
            "browser_live" => {
                let page = scenario
                    .name
                    .strip_prefix("browser_")
                    .unwrap_or(&scenario.name);
                run_browser_live_scenario(&mut backend, page, &out_dir, w, h)
            },
            "widget" => run_widget_gallery(&mut backend, &out_dir, w, h),
            "wm" => run_wm_scenario(&mut backend, &scenario.name, &out_dir, w, h),
            "tv_guide" => run_tv_guide_scenario(&mut backend, &scenario.name, &out_dir, w, h),
            _ => {
                anyhow::bail!("Unknown scenario category: {}", scenario.category);
            },
        };

        match result {
            Ok(()) => {
                if args.bless {
                    bless_golden(&out_dir)?;
                    completed += 1;
                    println!("  BLESS  {}", scenario.name);
                } else if args.check {
                    match check_golden(&out_dir) {
                        Ok(true) => {
                            completed += 1;
                            println!("  MATCH  {}", scenario.name);
                        },
                        Ok(false) => {
                            failed += 1;
                            eprintln!("  MISMATCH  {}", scenario.name);
                        },
                        Err(e) => {
                            failed += 1;
                            eprintln!("  SKIP  {}: {e}", scenario.name);
                        },
                    }
                } else {
                    completed += 1;
                    println!("  OK  {}", scenario.name);
                }
            },
            Err(e) => {
                failed += 1;
                eprintln!("  FAIL  {}: {e}", scenario.name);
            },
        }
    }

    if args.report {
        generate_report(&base_dir, &scenarios)?;
    }

    backend.shutdown()?;

    println!();
    if args.check {
        println!("Screenshot check: {completed} matched, {failed} mismatched/missing");
    } else if args.bless {
        println!("Screenshot bless: {completed} golden files updated");
    } else {
        println!("Screenshot tests: {completed} passed, {failed} failed");
    }
    if completed + failed == 0 {
        println!("(No scenarios matched the filter)");
    }

    if args.check && failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}
