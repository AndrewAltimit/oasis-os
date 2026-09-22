//! Headless screenshot example.
//!
//! Renders a skinned OASIS_OS dashboard into a PNG without opening a
//! window. Uses the UE5 backend (a pure-software RGBA framebuffer), so it
//! needs no GPU or display server.
//!
//! ```bash
//! cargo run -p oasis-backend-ue5 --example headless_screenshot            # classic
//! cargo run -p oasis-backend-ue5 --example headless_screenshot -- xp out.png
//! ```

use oasis_backend_ue5::Ue5Backend;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::SdiCore;
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::resolve_skin;
use oasis_core::vfs::{MemoryVfs, Vfs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let skin_name = args.next().unwrap_or_else(|| "classic".to_string());
    let output_path = args.next().unwrap_or_else(|| "screenshot.png".to_string());

    // 1. Load the skin and derive the runtime theme.
    let skin = resolve_skin(&skin_name)?;
    let (w, h) = (skin.manifest.screen_width, skin.manifest.screen_height);
    let theme = ActiveTheme::from_skin(&skin.theme)
        .with_screen_size(w, h)
        .with_features(&skin.features);

    // 2. Create the software renderer (no GPU needed).
    let mut backend = Ue5Backend::new(w, h);
    backend.init(w, h)?;

    // 3. A tiny in-memory VFS: every directory under /apps becomes a
    //    dashboard icon (an EBOOT.PBP inside would supply title + icon).
    let mut vfs = MemoryVfs::new();
    for app in ["Browser", "Terminal", "Files", "Music", "Settings"] {
        vfs.mkdir(&format!("/apps/{app}"))?;
    }
    let apps = discover_apps(&vfs, "/apps", None)?;

    // 4. Build the scene: skin layout objects + dashboard icons.
    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);
    let config = DashboardConfig::from_features(&skin.features, &theme);
    let mut dashboard = DashboardState::new(config, apps);
    dashboard.update_sdi(&mut sdi, &theme);

    // 5. Render one frame.
    backend.clear(theme.clear_color)?;
    sdi.draw(&mut backend)?;
    backend.swap_buffers()?;

    // 6. Read the framebuffer back and save it.
    let pixels = backend.read_pixels(0, 0, w, h)?;
    save_rgba_png(&output_path, &pixels, w, h)?;
    println!(
        "Rendered {w}x{h} '{}' frame to {output_path}",
        skin.manifest.name
    );
    Ok(())
}

fn save_rgba_png(
    path: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}
