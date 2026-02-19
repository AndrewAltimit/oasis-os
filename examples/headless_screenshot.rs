//! Headless screenshot example.
//!
//! Renders the OASIS_OS desktop to a PNG file without opening a window.
//! Uses the UE5 backend (software RGBA buffer) since it doesn't require
//! a display server.
//!
//! ```bash
//! cargo run --example headless_screenshot
//! ```

use oasis_backend_ue5::Ue5Backend;
use oasis_core::sdi::SdiScene;
use oasis_skin::{ActiveTheme, resolve_skin};
use oasis_types::backend::SdiBackend;

fn main() {
    let width: u32 = 480;
    let height: u32 = 272;

    // 1. Create the UE5 software renderer (no GPU needed).
    let mut backend = Ue5Backend::new(width, height);

    // 2. Load skin and theme.
    let skin = resolve_skin("classic").expect("skin load failed");
    let theme = ActiveTheme::from_skin(&skin.theme);

    // 3. Build the SDI scene.
    let mut sdi = SdiScene::new();
    oasis_core::sdi_setup::build_scene(&mut sdi, &skin.layout, &theme);

    // 4. Render one frame.
    let bg = theme.background_color();
    backend.clear(bg);
    sdi.draw(&mut backend);
    backend.swap_buffers();

    // 5. Read pixels from the backend.
    let pixels = backend.read_pixels();
    println!(
        "Rendered {}x{} frame ({} bytes)",
        width,
        height,
        pixels.len()
    );

    // 6. Save as PNG (requires the `png` crate).
    let output_path = "screenshot.png";
    save_rgba_png(output_path, &pixels, width, height);
    println!("Screenshot saved to {output_path}");
}

fn save_rgba_png(path: &str, rgba: &[u8], width: u32, height: u32) {
    let file = std::fs::File::create(path).expect("Failed to create file");
    let w = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("PNG header failed");
    writer.write_image_data(rgba).expect("PNG write failed");
}
