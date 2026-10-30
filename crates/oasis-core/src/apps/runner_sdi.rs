//! SDI rendering for the app runner (update_sdi / hide_sdi).

use crate::active_theme::ActiveTheme;
use crate::sdi::SdiRegistry;

use super::registry::AppRegistry;
use super::runner::AppRunner;

impl AppRunner {
    /// Render the app screen as SDI objects (single-display-interface mode).
    pub fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.delegate.update_sdi(sdi, at);
    }

    /// Hide all app-related SDI objects: the shared app chrome and line
    /// pools, then every registered app's own objects
    /// ([`AppRegistration::hide_sdi`](super::AppRegistration::hide_sdi)).
    ///
    /// Runs every frame outside app mode, so the shared pass is
    /// change-detecting and the per-app cleanups skip themselves while
    /// their objects don't exist.
    pub fn hide_sdi(apps: &AppRegistry, sdi: &mut SdiRegistry) {
        oasis_app_core::render::hide_app_sdi(sdi);
        apps.hide_sdi(sdi);
    }
}
