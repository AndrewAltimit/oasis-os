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

    /// The plugin list the host publishes at
    /// [`PLUGIN_STATUS_PATH`](crate::ipc::PLUGIN_STATUS_PATH): one line
    /// per plugin, `<name> (<version>) [<state>] <description>`.
    pub fn status_text(&self) -> String {
        let mut out = String::new();
        for p in &self.plugins {
            let info = p.plugin.info();
            let state = match p.state {
                PluginState::Registered => "registered",
                PluginState::Active => "active",
                PluginState::Stopped => "stopped",
            };
            out.push_str(&format!(
                "{} ({}) [{state}] {}\n",
                info.name, info.version, info.description
            ));
        }
        out
    }

    /// Check if a plugin with the given name is loaded.
    pub fn is_loaded(&self, name: &str) -> bool {
        self.plugins.iter().any(|p| p.plugin.info().name == name)
    }
}
