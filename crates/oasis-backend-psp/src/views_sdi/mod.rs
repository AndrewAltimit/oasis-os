//! SDI-based classic view renderers.
//!
//! Each view creates a set of named [`SdiObject`]s in the registry, updates
//! their properties every frame, and hides them when switching away.  The
//! actual drawing is handled by [`SdiRegistry::draw_base_layer`].
//!
//! Object names are prefixed per view (`radio_`, `tv_`, `photo_`, `browser_`,
//! `music_`, `fm_`) to avoid collisions.

mod browser;
mod file_manager;
pub(crate) mod helpers;
mod list_view;
mod music;
mod photo;
mod radio;
mod settings;
mod tv_guide;

use oasis_core::sdi::SdiRegistry;

use crate::types::KioskApp;

// Re-export all public view functions.
// Browser SDI views retained for potential future fallback use.
#[allow(unused_imports)]
pub(crate) use browser::{setup_browser, update_browser};
pub(crate) use file_manager::{setup_file_manager, update_file_manager};
pub(crate) use helpers::hide_all;
pub(crate) use music::{setup_music_browser, update_music_browser};
pub(crate) use photo::{
    setup_photo_browser, setup_photo_view, update_photo_browser, update_photo_view,
};
pub(crate) use radio::{setup_radio, update_radio};
pub(crate) use settings::{setup_settings, update_settings};
pub(crate) use tv_guide::{setup_tv_channels, update_tv_channels};

/// Set up SDI objects for the given kiosk app.  Idempotent -- safe to call
/// every time a view is entered.
pub(crate) fn setup_kiosk(sdi: &mut SdiRegistry, app: KioskApp) {
    match app {
        KioskApp::Radio => setup_radio(sdi),
        KioskApp::TvGuide => setup_tv_channels(sdi),
        KioskApp::PhotoViewer => {
            setup_photo_browser(sdi);
            setup_photo_view(sdi);
        },
        KioskApp::MusicPlayer => setup_music_browser(sdi),
        KioskApp::Browser => {
            // Browser now uses BrowserWidget::paint() directly.
            // No SDI objects needed.
        },
        KioskApp::FileManager => setup_file_manager(sdi),
        KioskApp::Settings => setup_settings(sdi),
        // Terminal has its own SDI setup; None = dashboard.
        KioskApp::Terminal | KioskApp::None => {},
    }
}
