//! Minimal SDL desktop example.
//!
//! Demonstrates the smallest possible OASIS_OS application using the
//! SDL2 backend. Run with:
//!
//! ```bash
//! cargo run --example minimal_sdl
//! ```

use oasis_backend_sdl::SdlBackend;
use oasis_core::apps::discover_apps;
use oasis_core::dashboard::{DashboardConfig, DashboardState};
use oasis_core::sdi::SdiScene;
use oasis_skin::{ActiveTheme, resolve_skin};
use oasis_types::backend::SdiBackend;
use oasis_vfs::MemoryVfs;

fn main() {
    // 1. Create SDL backend at native resolution.
    let mut backend =
        SdlBackend::new("OASIS_OS - Minimal Example", 480, 272).expect("SDL init failed");
    backend.init().expect("backend init failed");

    // 2. Load the default skin and create theme.
    let skin = resolve_skin("classic").expect("skin load failed");
    let theme = ActiveTheme::from_skin(&skin.theme);

    // 3. Set up a minimal VFS with demo content.
    let mut vfs = MemoryVfs::new();
    oasis_core::vfs_setup::populate_demo_vfs(&mut vfs);

    // 4. Discover apps and create dashboard.
    let apps = discover_apps(&vfs, "/apps", Some("OASISOS"));
    let dash_config = DashboardConfig::from_features(&skin.features);
    let mut dashboard = DashboardState::new(apps, dash_config);

    // 5. Build SDI scene from skin layout.
    let mut sdi = SdiScene::new();
    oasis_core::sdi_setup::build_scene(&mut sdi, &skin.layout, &theme);

    // 6. Main loop.
    let mut running = true;
    while running {
        // Poll input.
        let events = backend.poll_events();
        for event in &events {
            if matches!(event, oasis_types::backend::InputEvent::Quit) {
                running = false;
            }
        }

        // Update dashboard (handles input, animations).
        dashboard.update(&events, &theme);

        // Clear and draw scene.
        let bg = theme.background_color();
        backend.clear(bg);
        sdi.draw(&mut backend);
        backend.swap_buffers();
    }
}
