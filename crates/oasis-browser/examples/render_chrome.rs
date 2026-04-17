//! Render just the browser chrome (URL bar + buttons) to a PNG for
//! visual inspection of address-bar polish work.
//!
//! Usage:
//!   cargo run -p oasis-browser --example render_chrome -- /tmp/chrome.png
//!
//! Paints three panels stacked vertically:
//!   1. idle — URL bar displays the current URL, no focus
//!   2. focused — URL bar in edit mode with the URL selected
//!   3. typed — user has typed "git" after click-to-select-all

use std::fs;
use std::path::PathBuf;

use oasis_backend_ue5::Ue5Backend;
use oasis_browser::{BrowserConfig, BrowserWidget};
use oasis_types::backend::SdiCore;
use oasis_types::input::InputEvent;
use oasis_vfs::MemoryVfs;

fn make_browser() -> BrowserWidget {
    let mut cfg = BrowserConfig::default();
    cfg.features.home_url = "vfs://sites/home/index.html".into();
    BrowserWidget::new(cfg)
}

fn paint_strip(
    label: &str,
    y: i32,
    backend: &mut Ue5Backend,
    widget: &mut BrowserWidget,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    widget.set_window(0, y, width, height);
    widget.paint_chrome(backend)?;
    println!("  painted {label} at y={y}");
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_path = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/chrome.png".into()),
    );

    let width: u32 = 960;
    let chrome_h: u32 = 28;
    let gap: u32 = 12;
    let strips: u32 = 3;
    let total_h: u32 = strips * chrome_h + (strips - 1) * gap + 2 * gap;

    let mut backend = Ue5Backend::new(width, total_h);
    backend.clear(oasis_types::backend::Color {
        r: 220,
        g: 222,
        b: 228,
        a: 255,
    })?;

    let vfs = MemoryVfs::new();

    // Strip 1: idle — current URL shown.
    let mut w1 = make_browser();
    w1.navigate_vfs("https://old.reddit.com/r/rust/", &vfs);
    paint_strip("idle", gap as i32, &mut backend, &mut w1, width, chrome_h)?;

    // Strip 2: focused — URL bar in edit mode (whole URL selected).
    let mut w2 = make_browser();
    w2.navigate_vfs("https://old.reddit.com/r/rust/", &vfs);
    // Simulate clicking in the URL bar area.
    w2.set_window(0, 0, width, chrome_h);
    let bw = w2.config.button_width as i32;
    w2.handle_click(bw * 2 + 10, 10, &vfs);
    paint_strip(
        "focused/selected",
        (gap * 2 + chrome_h) as i32,
        &mut backend,
        &mut w2,
        width,
        chrome_h,
    )?;

    // Strip 3: user has typed "git" after click-to-select-all.
    let mut w3 = make_browser();
    w3.navigate_vfs("https://old.reddit.com/r/rust/", &vfs);
    w3.set_window(0, 0, width, chrome_h);
    w3.handle_click(bw * 2 + 10, 10, &vfs);
    w3.handle_input(&InputEvent::TextInput('g'), &vfs);
    w3.handle_input(&InputEvent::TextInput('i'), &vfs);
    w3.handle_input(&InputEvent::TextInput('t'), &vfs);
    paint_strip(
        "after-typing",
        (gap * 3 + chrome_h * 2) as i32,
        &mut backend,
        &mut w3,
        width,
        chrome_h,
    )?;

    let rgba = backend.buffer();
    let file = fs::File::create(&out_path)?;
    let mut encoder = png::Encoder::new(file, width, total_h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    println!("Wrote {}", out_path.display());
    Ok(())
}
