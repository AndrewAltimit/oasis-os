//! Plugin app registration and enumeration.

use crate::apps::AppRegistry;
use crate::plugin::app_bridge::{PluginAppRegistration, sync_registry};

use super::PluginManager;

impl PluginManager {
    /// Return all plugin-registered app registrations.
    pub fn plugin_apps(&self) -> &[PluginAppRegistration] {
        &self.plugin_apps
    }

    /// Make `registry` hold exactly the currently registered plugin apps
    /// (previous plugin entries are dropped, built-in apps are kept). Hosts
    /// call this after plugin init (and after loading or unloading a
    /// plugin) so plugin apps launch, list and auto-launch like built-ins.
    pub fn extend_registry(&self, registry: &mut AppRegistry) {
        sync_registry(&self.plugin_apps, registry);
    }
}
