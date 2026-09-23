//! Minimal SDL desktop example.
//!
//! The smallest useful OASIS_OS shell: an SDL3 window showing a skinned
//! dashboard you can navigate with the arrow keys. Enter prints the
//! selected app; Escape or closing the window quits. The full shell (apps,
//! terminal, browser, window manager) lives in `crates/oasis-app`.
//!
//! ```bash
//! cargo run -p oasis-backend-sdl --example minimal_sdl            # classic
//! cargo run -p oasis-backend-sdl --example minimal_sdl -- xp
//! ```

use oasis_backend_sdl::SdlBackend;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::{InputBackend, SdiCore};
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps};
use oasis_core::input::{Button, InputEvent};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::resolve_skin;
use oasis_core::vfs::{MemoryVfs, Vfs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let skin_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "classic".to_string());

    // 1. Load the skin and derive the runtime theme.
    let skin = resolve_skin(&skin_name)?;
    let (w, h) = (skin.manifest.screen_width, skin.manifest.screen_height);
    let theme = ActiveTheme::from_skin(&skin.theme)
        .with_screen_size(w, h)
        .with_features(&skin.features);

    // 2. Create the SDL3 backend (window + renderer + input).
    let mut backend = SdlBackend::new("OASIS_OS - Minimal Example", w, h)?;
    backend.init(w, h)?;

    // 3. A tiny in-memory VFS: each directory under /apps is an icon.
    let mut vfs = MemoryVfs::new();
    for app in [
        "Browser", "Terminal", "Files", "Music", "Photos", "Settings",
    ] {
        vfs.mkdir(&format!("/apps/{app}"))?;
    }
    let apps = discover_apps(&vfs, "/apps", None)?;

    // 4. Dashboard + scene graph built from the skin layout.
    let config = DashboardConfig::from_features(&skin.features, &theme);
    let mut dashboard = DashboardState::new(config, apps);
    let mut sdi = SdiRegistry::new();
    skin.apply_layout(&mut sdi);

    // 5. Main loop: input -> state -> scene -> draw.
    'running: loop {
        for event in backend.poll_events() {
            match event {
                InputEvent::Quit | InputEvent::ButtonPress(Button::Cancel) => break 'running,
                InputEvent::ButtonPress(Button::Confirm) => {
                    if let Some(app) = dashboard.selected_app() {
                        println!("Selected: {}", app.title);
                    }
                },
                InputEvent::ButtonPress(button) => dashboard.handle_input(&button),
                _ => {},
            }
        }
        dashboard.tick_animation();
        dashboard.update_sdi(&mut sdi, &theme);

        backend.clear(theme.clear_color)?;
        sdi.draw(&mut backend)?;
        backend.swap_buffers()?;
        std::thread::sleep(std::time::Duration::from_millis(16));
    }

    backend.shutdown()?;
    Ok(())
}
