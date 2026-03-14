//! Plugin app registration and enumeration.

use crate::plugin::app_bridge::PluginAppRegistration;
use crate::vfs::Vfs;

use super::PluginManager;

impl PluginManager {
    /// Return all plugin-registered app registrations.
    ///
    /// The dashboard uses this to include plugin apps alongside built-in
    /// apps. The app runner uses it to create app instances on launch.
    pub fn plugin_apps(&self) -> &[PluginAppRegistration] {
        &self.plugin_apps
    }

    /// Find a plugin app registration by title and create an app instance.
    ///
    /// Returns `None` if no plugin app with the given title exists.
    pub fn create_plugin_app(
        &self,
        title: &str,
        vfs: &dyn Vfs,
    ) -> Option<Box<dyn oasis_app_core::App>> {
        self.plugin_apps
            .iter()
            .find(|r| r.title == title)
            .map(|r| r.create_app(vfs))
    }
}
