//! Plugin lifecycle methods: init, update, shutdown, unload.

use crate::error::{OasisError, Result};
use crate::sdi::SdiRegistry;
use crate::terminal::CommandRegistry;
use crate::vfs::Vfs;

use crate::plugin::traits::{PluginCapabilities, PluginHost, PluginState};

use super::PluginManager;

impl PluginManager {
    /// Initialize all registered (but not yet active) plugins.
    ///
    /// Errors from individual plugins are logged and skipped so that one
    /// bad plugin does not prevent the rest from loading.
    pub fn init_all(
        &mut self,
        sdi: &mut SdiRegistry,
        vfs: &mut dyn Vfs,
        commands: &mut CommandRegistry,
    ) {
        for i in 0..self.plugins.len() {
            if self.plugins[i].state == PluginState::Registered {
                let name = self.plugins[i].plugin.info().name.clone();
                if let Err(e) = Self::validate_api_version(&self.plugins[i].plugin.info()) {
                    log::error!("Failed to init plugin '{name}': {e}");
                    continue;
                }
                let mut pending_apps = Vec::new();
                let init_result = {
                    let mut host = Self::build_host(
                        &self.plugins[i],
                        sdi,
                        vfs,
                        commands,
                        &mut pending_apps,
                        None,
                    );
                    self.plugins[i].plugin.init(&mut host)
                };
                if let Err(e) = init_result {
                    log::error!("Failed to init plugin '{name}': {e}");
                    continue;
                }
                self.plugins[i].state = PluginState::Active;
                self.collect_apps(i, pending_apps, false);
            }
        }
    }

    /// Initialize a single plugin by name.
    pub fn init_plugin(
        &mut self,
        name: &str,
        sdi: &mut SdiRegistry,
        vfs: &mut dyn Vfs,
        commands: &mut CommandRegistry,
    ) -> Result<()> {
        let idx = self
            .plugins
            .iter()
            .position(|p| p.plugin.info().name == name)
            .ok_or_else(|| OasisError::Plugin(format!("plugin not found: {name}").into()))?;
        if self.plugins[idx].state != PluginState::Registered {
            return Err(OasisError::Plugin(
                format!(
                    "plugin '{name}' is already {}",
                    if self.plugins[idx].state == PluginState::Active {
                        "active"
                    } else {
                        "stopped"
                    }
                )
                .into(),
            ));
        }
        Self::validate_api_version(&self.plugins[idx].plugin.info())?;
        let mut pending_apps = Vec::new();
        {
            let mut host = Self::build_host(
                &self.plugins[idx],
                sdi,
                vfs,
                commands,
                &mut pending_apps,
                None,
            );
            self.plugins[idx].plugin.init(&mut host)?;
        }
        self.plugins[idx].state = PluginState::Active;
        self.collect_apps(idx, pending_apps, false);
        Ok(())
    }

    /// Call `update()` on all active plugins.
    ///
    /// Errors from individual plugins are logged and skipped so that one
    /// bad plugin does not prevent the rest from updating.
    pub fn update_all(
        &mut self,
        sdi: &mut SdiRegistry,
        vfs: &mut dyn Vfs,
        commands: &mut CommandRegistry,
    ) {
        for i in 0..self.plugins.len() {
            if self.plugins[i].state == PluginState::Active {
                let name = self.plugins[i].plugin.info().name.clone();
                let mut pending_apps = Vec::new();
                let update_result = {
                    let mut host = Self::build_host(
                        &self.plugins[i],
                        sdi,
                        vfs,
                        commands,
                        &mut pending_apps,
                        None,
                    );
                    self.plugins[i].plugin.update(&mut host)
                };
                if let Err(e) = update_result {
                    log::error!("Failed to update plugin '{name}': {e}");
                    continue;
                }
                self.collect_apps(i, pending_apps, true);
            }
        }
    }

    /// Shutdown all active plugins.
    ///
    /// Errors from individual plugins are logged and skipped so that one
    /// bad plugin does not prevent the rest from shutting down.
    pub fn shutdown_all(
        &mut self,
        sdi: &mut SdiRegistry,
        vfs: &mut dyn Vfs,
        commands: &mut CommandRegistry,
    ) {
        for i in 0..self.plugins.len() {
            if self.plugins[i].state == PluginState::Active {
                let name = self.plugins[i].plugin.info().name.clone();
                let mut pending_apps = Vec::new();
                let shutdown_result = {
                    let mut host = Self::build_host(
                        &self.plugins[i],
                        sdi,
                        vfs,
                        commands,
                        &mut pending_apps,
                        Some(PluginCapabilities::all()),
                    );
                    self.plugins[i].plugin.shutdown(&mut host)
                };
                if let Err(e) = shutdown_result {
                    log::error!("Failed to shutdown plugin '{name}': {e}");
                }
                self.plugins[i].state = PluginState::Stopped;
            }
        }
    }

    /// Shutdown and remove a plugin by name.
    pub fn unload(
        &mut self,
        name: &str,
        sdi: &mut SdiRegistry,
        vfs: &mut dyn Vfs,
        commands: &mut CommandRegistry,
    ) -> Result<()> {
        let idx = self
            .plugins
            .iter()
            .position(|p| p.plugin.info().name == name)
            .ok_or_else(|| OasisError::Plugin(format!("plugin not found: {name}").into()))?;

        let loaded = &mut self.plugins[idx];
        if loaded.state == PluginState::Active {
            let plugin_name = loaded.plugin.info().name.clone();
            let mut pending_apps = Vec::new();
            // Grant all capabilities during shutdown for cleanup.
            let mut host = PluginHost {
                sdi,
                vfs,
                commands,
                audio: None,
                network: None,
                backend: None,
                app_registrations: &mut pending_apps,
                capabilities: PluginCapabilities::all(),
                plugin_name,
            };
            loaded.plugin.shutdown(&mut host)?;
        }
        // Remove apps registered by this plugin.
        let titles = &self.plugins[idx].registered_app_titles;
        self.plugin_apps.retain(|a| !titles.contains(&a.title));
        self.plugins.remove(idx);
        Ok(())
    }
}
