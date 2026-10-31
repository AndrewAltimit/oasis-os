//! Application runner -- manages launched app screens.
//!
//! A host owns an [`AppRegistry`] of launchable apps (the built-in apps are
//! bundled by the `oasis-apps` crate; plugins append theirs). When the user
//! launches one, an [`AppRunner`] is created from the registry: it wraps
//! the app and adds the host-facing bookkeeping (redraw tracking, input
//! routing, windowed drawing).
//!
//! The `App` trait (from `oasis-app-core`) is the interface each
//! application implements. `oasis-core` itself knows no concrete app
//! except the Terminal delegate and the placeholder, both [`SimpleApp`].

mod input;
mod registry;
mod runner;
pub mod simple_app;

pub use oasis_app_core::file_viewer::app_for_file;
pub use oasis_app_core::{App, AppAction, AppRequest, AsAny, ContentState};
pub use registry::{AppFactory, AppRegistration, AppRegistry, FileFactory, OwnedMedia};
pub use runner::AppRunner;
pub use simple_app::SimpleApp;

/// Window id of the app titled `title` ("Photo Viewer" -> "photo_viewer").
/// Hosts key app windows (and their runners) by it.
pub fn window_id(title: &str) -> String {
    title.to_lowercase().replace(' ', "_")
}

/// Size of a newly opened app window on a `screen_w` x `screen_h` screen:
/// 380x220 (the PSP-native size, which is what small screens get), growing
/// with the resolution to 40% x 45% of the screen so windows don't open as
/// postage stamps on large screens while several still fit side by side
/// over the dashboard. Never larger than the screen.
pub fn default_window_size(screen_w: u32, screen_h: u32) -> (u32, u32) {
    let w = (screen_w * 2 / 5).max(380).min(screen_w);
    let h = (screen_h * 9 / 20).max(220).min(screen_h);
    (w, h)
}

/// `(top, bottom)` screen rows the shell chrome keeps on a skin: the
/// status bar plus, when the skin shows it, the top tab row, and the
/// bottom bar. Hosts hand them to
/// [`crate::wm::WindowManager::set_placement_insets`] so new windows open
/// below the tab row instead of over it.
pub fn chrome_insets(
    at: &crate::active_theme::ActiveTheme,
    features: &crate::skin::SkinFeatures,
) -> (u32, u32) {
    let tab_row = if features.show_tabs {
        at.tab_row_height.max(u32::try_from(at.tab_h).unwrap_or(0))
    } else {
        0
    };
    (at.statusbar_height + tab_row, at.bottombar_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_id_lowercases_and_joins_words() {
        assert_eq!(window_id("Photo Viewer"), "photo_viewer");
        assert_eq!(window_id("Terminal"), "terminal");
    }

    #[test]
    fn default_window_size_scales_with_the_screen() {
        assert_eq!(default_window_size(480, 272), (380, 220));
        assert_eq!(default_window_size(1280, 720), (512, 324));
        assert_eq!(default_window_size(1920, 1080), (768, 486));
        // Never larger than the screen.
        assert_eq!(default_window_size(320, 200), (320, 200));
    }

    #[test]
    fn chrome_insets_count_the_tab_row_only_when_shown() {
        let skin = crate::skin::resolve_skin("psix-hifi").unwrap();
        let at = crate::active_theme::ActiveTheme::from_skin(&skin.theme)
            .with_screen_size(skin.manifest.screen_width, skin.manifest.screen_height);
        assert!(skin.features.show_tabs);
        let (top, bottom) = chrome_insets(&at, &skin.features);
        assert!(top >= at.statusbar_height + at.tab_row_height);
        assert_eq!(bottom, at.bottombar_height);
        let mut no_tabs = skin.features.clone();
        no_tabs.show_tabs = false;
        assert_eq!(chrome_insets(&at, &no_tabs).0, at.statusbar_height);
    }
}
