//! Plugin lifecycle methods: register, init, update, shutdown, unload.

use std::collections::HashSet;

use crate::error::{OasisError, Result};
use crate::sdi::SdiRegistry;
use crate::terminal::CommandRegistry;
use crate::vfs::Vfs;

use crate::plugin::app_bridge::PluginAppRegistration;
use crate::plugin::traits::{Plugin, PluginCapabilities, PluginState};

use super::{LoadedPlugin, PluginManager};

impl PluginManager {
    /// Register a static (built-in) plugin.
    ///
    /// The plugin is added in `Registered` state and must be initialized
    /// via `init_all()` or `init_plugin()`.
    pub fn register_static(&mut self, plugin: Box<dyn Plugin>) {
        let capabilities = plugin.capabilities();
        self.plugins.push(LoadedPlugin {
            plugin,
            state: PluginState::Registered,
            registered_app_titles: Vec::new(),
            registered_commands: Vec::new(),
            capabilities,
        });
    }

    /// Collect app registrations from a lifecycle call into the manager.
    ///
    /// Titles are unique across all plugins (first registration wins):
    /// [`unload`](Self::unload) removes apps by the titles a plugin
    /// recorded, so a duplicate recorded by a second plugin would take the
    /// first plugin's app with it.
    pub(super) fn collect_apps(&mut self, idx: usize, pending_apps: Vec<PluginAppRegistration>) {
        for app in pending_apps {
            if self.plugin_apps.iter().any(|a| a.title == app.title) {
                log::warn!(
                    "Plugin app '{}' already registered, ignoring duplicate",
                    app.title,
                );
            } else {
                self.plugins[idx]
                    .registered_app_titles
                    .push(app.title.clone());
                self.plugin_apps.push(app);
            }
        }
    }

    /// Run `init()` on plugin `idx` (Registered or Stopped) and make it
    /// Active. Records the commands it registered (new names in
    /// `commands`) so stopping it can remove them again.
    fn activate(
        &mut self,
        idx: usize,
        sdi: &mut SdiRegistry,
        vfs: &mut dyn Vfs,
        commands: &mut CommandRegistry,
    ) -> Result<()> {
        Self::validate_api_version(&self.plugins[idx].plugin.info())?;
        let before: HashSet<String> = commands.command_names().map(str::to_string).collect();
        let mut pending_apps = Vec::new();
        {
            let mut host = Self::build_host(
                &self.plugins[idx],
                self.context,
                sdi,
                vfs,
                commands,
                &mut pending_apps,
                None,
            );
            self.plugins[idx].plugin.init(&mut host)?;
        }
        self.plugins[idx].registered_commands = commands
            .command_names()
            .filter(|n| !before.contains(*n))
            .map(str::to_string)
            .collect();
        self.plugins[idx].state = PluginState::Active;
        self.collect_apps(idx, pending_apps);
        Ok(())
    }

    /// Run `shutdown()` on the active plugin `idx` (with every capability,
    /// for cleanup), mark it Stopped, and drop the apps and commands it
    /// registered.
    fn deactivate(
        &mut self,
        idx: usize,
        sdi: &mut SdiRegistry,
        vfs: &mut dyn Vfs,
        commands: &mut CommandRegistry,
    ) -> Result<()> {
        let mut pending_apps = Vec::new();
        let result = {
            let mut host = Self::build_host(
                &self.plugins[idx],
                self.context,
                sdi,
                vfs,
                commands,
                &mut pending_apps,
                Some(PluginCapabilities::all()),
            );
            self.plugins[idx].plugin.shutdown(&mut host)
        };
        self.plugins[idx].state = PluginState::Stopped;
        let loaded = &mut self.plugins[idx];
        for name in loaded.registered_commands.drain(..) {
            commands.unregister(&name);
        }
        let titles = std::mem::take(&mut loaded.registered_app_titles);
        self.plugin_apps.retain(|a| !titles.contains(&a.title));
        result
    }

    /// Index of the plugin named `name`.
    fn index_of(&self, name: &str) -> Result<usize> {
        self.plugins
            .iter()
            .position(|p| p.plugin.info().name == name)
            .ok_or_else(|| OasisError::Plugin(format!("plugin not found: {name}").into()))
    }

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
            if self.plugins[i].state == PluginState::Registered
                && let Err(e) = self.activate(i, sdi, vfs, commands)
            {
                let name = self.plugins[i].plugin.info().name;
                log::error!("Failed to init plugin '{name}': {e}");
            }
        }
    }

    /// Initialize a single plugin by name: a registered one, or a stopped
    /// one again (`plugin load` after `plugin unload`; statically linked
    /// plugins must therefore accept `init` after `shutdown`).
    pub fn init_plugin(
        &mut self,
        name: &str,
        sdi: &mut SdiRegistry,
        vfs: &mut dyn Vfs,
        commands: &mut CommandRegistry,
    ) -> Result<()> {
        let idx = self.index_of(name)?;
        if self.plugins[idx].state == PluginState::Active {
            return Err(OasisError::Plugin(
                format!("plugin '{name}' is already active").into(),
            ));
        }
        self.activate(idx, sdi, vfs, commands)
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
                let mut pending_apps = Vec::new();
                let update_result = {
                    let mut host = Self::build_host(
                        &self.plugins[i],
                        self.context,
                        sdi,
                        vfs,
                        commands,
                        &mut pending_apps,
                        None,
                    );
                    self.plugins[i].plugin.update(&mut host)
                };
                if let Err(e) = update_result {
                    let name = self.plugins[i].plugin.info().name;
                    log::error!("Failed to update plugin '{name}': {e}");
                    continue;
                }
                self.collect_apps(i, pending_apps);
            }
        }
    }

    /// Shutdown all active plugins (host exit).
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
            if self.plugins[i].state == PluginState::Active
                && let Err(e) = self.deactivate(i, sdi, vfs, commands)
            {
                let name = self.plugins[i].plugin.info().name;
                log::error!("Failed to shutdown plugin '{name}': {e}");
            }
        }
    }

    /// Shut one active plugin down but keep it registered (state
    /// `Stopped`), so [`init_plugin`](Self::init_plugin) can bring it
    /// back: the terminal's `plugin unload`. Its apps and commands are
    /// removed; the plugin's `shutdown` removes its SDI objects.
    pub fn stop_plugin(
        &mut self,
        name: &str,
        sdi: &mut SdiRegistry,
        vfs: &mut dyn Vfs,
        commands: &mut CommandRegistry,
    ) -> Result<()> {
        let idx = self.index_of(name)?;
        if self.plugins[idx].state != PluginState::Active {
            return Err(OasisError::Plugin(
                format!("plugin '{name}' is not active").into(),
            ));
        }
        self.deactivate(idx, sdi, vfs, commands)
    }

    /// Shutdown and remove a plugin by name.
    pub fn unload(
        &mut self,
        name: &str,
        sdi: &mut SdiRegistry,
        vfs: &mut dyn Vfs,
        commands: &mut CommandRegistry,
    ) -> Result<()> {
        let idx = self.index_of(name)?;
        let result = if self.plugins[idx].state == PluginState::Active {
            self.deactivate(idx, sdi, vfs, commands)
        } else {
            Ok(())
        };
        if result.is_ok() {
            self.plugins.remove(idx);
        }
        result
    }
}
