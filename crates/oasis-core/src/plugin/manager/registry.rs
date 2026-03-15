//! Plugin registry: lookup, enumeration, and capability queries.

use crate::plugin::traits::{PluginInfo, PluginState};

use super::PluginManager;

impl PluginManager {
    /// List all plugins with their info and state.
    pub fn list(&self) -> Vec<(PluginInfo, PluginState)> {
        self.plugins
            .iter()
            .map(|p| (p.plugin.info(), p.state))
            .collect()
    }

    /// Return the number of loaded plugins.
    pub fn count(&self) -> usize {
        self.plugins.len()
    }

    /// Return the number of active plugins.
    pub fn active_count(&self) -> usize {
        self.plugins
            .iter()
            .filter(|p| p.state == PluginState::Active)
            .count()
    }

    /// Check if a plugin with the given name is loaded.
    pub fn is_loaded(&self, name: &str) -> bool {
        self.plugins.iter().any(|p| p.plugin.info().name == name)
    }
}
