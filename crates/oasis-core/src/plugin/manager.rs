//! Plugin lifecycle manager.
//!
//! Manages registration, initialization, per-frame updates, and shutdown
//! of plugins. Supports static registration (built-in plugins) and
//! manifest-based discovery from the VFS.

use serde::Deserialize;

use crate::error::{OasisError, PluginError, Result};
use crate::sdi::SdiRegistry;
use crate::terminal::CommandRegistry;
use crate::vfs::Vfs;

use super::app_bridge::PluginAppRegistration;
use super::traits::{
    PLUGIN_API_VERSION, Plugin, PluginCapabilities, PluginHost, PluginInfo, PluginState,
};

/// A loaded plugin with its runtime state.
struct LoadedPlugin {
    plugin: Box<dyn Plugin>,
    state: PluginState,
    /// Titles of apps registered by this plugin (for cleanup on unload).
    registered_app_titles: Vec<String>,
    /// Cached capabilities from `plugin.capabilities()`.
    capabilities: PluginCapabilities,
}

/// Plugin manifest from a TOML file in the VFS.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    /// Plugin name.
    pub name: String,
    /// Plugin version.
    #[serde(default)]
    pub version: String,
    /// Plugin author.
    #[serde(default)]
    pub author: String,
    /// One-line description.
    #[serde(default)]
    pub description: String,
    /// Path to the shared library (relative to plugin directory).
    #[serde(default)]
    pub library: String,
    /// Whether to auto-load on startup.
    #[serde(default)]
    pub auto_load: bool,
    /// Plugin configuration key-value pairs (from `[config]` section).
    #[serde(default)]
    pub config: std::collections::HashMap<String, PluginConfigValue>,
}

/// A typed configuration value for a plugin.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PluginConfigValue {
    /// Boolean value.
    Bool(bool),
    /// Integer value.
    Int(i64),
    /// Floating-point value.
    Float(f64),
    /// String value.
    Str(String),
}

/// Manages the plugin lifecycle.
pub struct PluginManager {
    plugins: Vec<LoadedPlugin>,
    /// App registrations from plugins (populated during init).
    plugin_apps: Vec<PluginAppRegistration>,
}

impl PluginManager {
    /// Create a new empty plugin manager.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            plugin_apps: Vec::new(),
        }
    }

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
            capabilities,
        });
    }

    /// Validate a plugin's API version against the host.
    fn validate_api_version(info: &PluginInfo) -> Result<()> {
        if info.api_version != PLUGIN_API_VERSION {
            return Err(OasisError::Plugin(PluginError::ApiMismatch {
                plugin: info.name.clone(),
                expected: PLUGIN_API_VERSION,
                found: info.api_version,
            }));
        }
        Ok(())
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
            if self.plugins[i].state == PluginState::Registered {
                let name = self.plugins[i].plugin.info().name.clone();
                if let Err(e) = Self::validate_api_version(&self.plugins[i].plugin.info()) {
                    log::error!("Failed to init plugin '{name}': {e}");
                    continue;
                }
                let mut pending_apps = Vec::new();
                let init_result = {
                    let mut host = PluginHost {
                        sdi,
                        vfs,
                        commands,
                        audio: None,
                        network: None,
                        backend: None,
                        app_registrations: &mut pending_apps,
                        capabilities: self.plugins[i].capabilities.clone(),
                        plugin_name: name.clone(),
                    };
                    self.plugins[i].plugin.init(&mut host)
                };
                if let Err(e) = init_result {
                    log::error!("Failed to init plugin '{name}': {e}");
                    continue;
                }
                self.plugins[i].state = PluginState::Active;
                self.plugins[i]
                    .registered_app_titles
                    .extend(pending_apps.iter().map(|a| a.title.clone()));
                self.plugin_apps.extend(pending_apps);
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
        let plugin_name = self.plugins[idx].plugin.info().name.clone();
        let mut pending_apps = Vec::new();
        {
            let mut host = PluginHost {
                sdi,
                vfs,
                commands,
                audio: None,
                network: None,
                backend: None,
                app_registrations: &mut pending_apps,
                capabilities: self.plugins[idx].capabilities.clone(),
                plugin_name,
            };
            self.plugins[idx].plugin.init(&mut host)?;
        }
        self.plugins[idx].state = PluginState::Active;
        // Move collected app registrations into the manager.
        self.plugins[idx]
            .registered_app_titles
            .extend(pending_apps.iter().map(|a| a.title.clone()));
        self.plugin_apps.extend(pending_apps);
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
                    let mut host = PluginHost {
                        sdi,
                        vfs,
                        commands,
                        audio: None,
                        network: None,
                        backend: None,
                        app_registrations: &mut pending_apps,
                        capabilities: self.plugins[i].capabilities.clone(),
                        plugin_name: name.clone(),
                    };
                    self.plugins[i].plugin.update(&mut host)
                };
                if let Err(e) = update_result {
                    log::error!("Failed to update plugin '{name}': {e}");
                    continue;
                }
                // Plugins can register apps during update too (rare but supported).
                // Deduplicate by title to prevent accumulation across frames.
                for app in pending_apps {
                    if self.plugin_apps.iter().any(|a| a.title == app.title) {
                        log::warn!(
                            "Plugin app '{}' already registered, ignoring duplicate",
                            app.title,
                        );
                    } else {
                        self.plugins[i]
                            .registered_app_titles
                            .push(app.title.clone());
                        self.plugin_apps.push(app);
                    }
                }
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
                    // Grant all capabilities during shutdown so plugins
                    // can always clean up their resources.
                    let mut host = PluginHost {
                        sdi,
                        vfs,
                        commands,
                        audio: None,
                        network: None,
                        backend: None,
                        app_registrations: &mut pending_apps,
                        capabilities: PluginCapabilities::all(),
                        plugin_name: name.clone(),
                    };
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

    /// Discover plugin manifests from the VFS plugin directory.
    ///
    /// Scans `/etc/oasis-os/plugins/` for `plugin.toml` files and returns
    /// their parsed manifests. This does NOT load the plugins -- it only
    /// discovers what's available.
    pub fn discover_manifests(vfs: &mut dyn Vfs) -> Vec<PluginManifest> {
        let plugin_dir = "/etc/oasis-os/plugins";
        if !vfs.exists(plugin_dir) {
            return Vec::new();
        }
        let entries = match vfs.readdir(plugin_dir) {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };

        let mut manifests = Vec::new();
        for entry in &entries {
            if entry.kind == crate::vfs::EntryKind::Directory {
                let manifest_path = format!("{plugin_dir}/{}/plugin.toml", entry.name);
                if vfs.exists(&manifest_path)
                    && let Ok(data) = vfs.read(&manifest_path)
                {
                    let toml_str = String::from_utf8_lossy(&data);
                    if let Ok(manifest) = toml::from_str::<PluginManifest>(&toml_str) {
                        manifests.push(manifest);
                    }
                }
            }
        }
        manifests
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::traits::{Plugin, PluginHost, PluginInfo};
    use crate::sdi::SdiRegistry;
    use crate::terminal::CommandRegistry;
    use crate::vfs::{MemoryVfs, Vfs};

    /// Minimal test plugin that tracks lifecycle calls.
    struct TestPlugin {
        init_called: bool,
        update_count: u32,
        shutdown_called: bool,
    }

    impl TestPlugin {
        fn new() -> Self {
            Self {
                init_called: false,
                update_count: 0,
                shutdown_called: false,
            }
        }
    }

    impl Plugin for TestPlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo::new("test-plugin", "1.0.0")
                .with_author("Test")
                .with_description("A test plugin")
        }
        fn init(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            self.init_called = true;
            Ok(())
        }
        fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            self.update_count += 1;
            Ok(())
        }
        fn shutdown(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            self.shutdown_called = true;
            Ok(())
        }
    }

    /// Plugin that creates an SDI object during init.
    struct SdiPlugin;
    impl Plugin for SdiPlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo::new("sdi-plugin", "1.0.0")
        }
        fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
            let obj = host.sdi.create("plugin_widget");
            obj.x = 10;
            obj.y = 20;
            Ok(())
        }
        fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
        fn shutdown(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
            let _ = host.sdi.destroy("plugin_widget");
            Ok(())
        }
    }

    #[test]
    fn register_and_init() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(TestPlugin::new()));
        assert_eq!(mgr.count(), 1);
        assert_eq!(mgr.active_count(), 0);

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        assert_eq!(mgr.active_count(), 1);
    }

    #[test]
    fn update_active_plugins() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(TestPlugin::new()));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);

        mgr.update_all(&mut sdi, &mut vfs, &mut cmds);
        mgr.update_all(&mut sdi, &mut vfs, &mut cmds);

        let plugins = mgr.list();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].1, PluginState::Active);
    }

    #[test]
    fn shutdown_all_plugins() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(TestPlugin::new()));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        mgr.shutdown_all(&mut sdi, &mut vfs, &mut cmds);

        let plugins = mgr.list();
        assert_eq!(plugins[0].1, PluginState::Stopped);
        assert_eq!(mgr.active_count(), 0);
    }

    #[test]
    fn unload_plugin() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(TestPlugin::new()));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        assert!(mgr.is_loaded("test-plugin"));

        mgr.unload("test-plugin", &mut sdi, &mut vfs, &mut cmds)
            .unwrap();
        assert!(!mgr.is_loaded("test-plugin"));
        assert_eq!(mgr.count(), 0);
    }

    #[test]
    fn unload_missing_plugin() {
        let mut mgr = PluginManager::new();
        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        assert!(
            mgr.unload("nonexistent", &mut sdi, &mut vfs, &mut cmds)
                .is_err()
        );
    }

    #[test]
    fn plugin_creates_sdi_objects() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(SdiPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);

        assert!(sdi.contains("plugin_widget"));
        let obj = sdi.get("plugin_widget").unwrap();
        assert_eq!(obj.x, 10);
        assert_eq!(obj.y, 20);
    }

    #[test]
    fn plugin_cleans_up_sdi_on_shutdown() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(SdiPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        assert!(sdi.contains("plugin_widget"));

        mgr.shutdown_all(&mut sdi, &mut vfs, &mut cmds);
        assert!(!sdi.contains("plugin_widget"));
    }

    #[test]
    fn init_plugin_by_name() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(TestPlugin::new()));
        mgr.register_static(Box::new(SdiPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        // Only init one plugin.
        mgr.init_plugin("test-plugin", &mut sdi, &mut vfs, &mut cmds)
            .unwrap();
        assert_eq!(mgr.active_count(), 1);
        assert!(!sdi.contains("plugin_widget")); // SdiPlugin not initialized.
    }

    #[test]
    fn init_already_active_fails() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(TestPlugin::new()));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);

        assert!(
            mgr.init_plugin("test-plugin", &mut sdi, &mut vfs, &mut cmds)
                .is_err()
        );
    }

    #[test]
    fn list_plugins() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(TestPlugin::new()));
        mgr.register_static(Box::new(SdiPlugin));

        let list = mgr.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0.name, "test-plugin");
        assert_eq!(list[0].1, PluginState::Registered);
        assert_eq!(list[1].0.name, "sdi-plugin");
    }

    #[test]
    fn discover_manifests_empty() {
        let mut vfs = MemoryVfs::new();
        let manifests = PluginManager::discover_manifests(&mut vfs);
        assert!(manifests.is_empty());
    }

    #[test]
    fn discover_manifests_from_vfs() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/etc").unwrap();
        vfs.mkdir("/etc/oasis-os").unwrap();
        vfs.mkdir("/etc/oasis-os/plugins").unwrap();
        vfs.mkdir("/etc/oasis-os/plugins/my-plugin").unwrap();
        vfs.write(
            "/etc/oasis-os/plugins/my-plugin/plugin.toml",
            br#"
name = "my-plugin"
version = "2.0"
author = "Test"
description = "A discovered plugin"
library = "libmyplugin.so"
auto_load = true
"#,
        )
        .unwrap();

        let manifests = PluginManager::discover_manifests(&mut vfs);
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].name, "my-plugin");
        assert_eq!(manifests[0].version, "2.0");
        assert!(manifests[0].auto_load);
    }

    #[test]
    fn multiple_plugins_lifecycle() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(TestPlugin::new()));
        mgr.register_static(Box::new(SdiPlugin));
        assert_eq!(mgr.count(), 2);

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        assert_eq!(mgr.active_count(), 2);
        assert!(sdi.contains("plugin_widget"));

        mgr.update_all(&mut sdi, &mut vfs, &mut cmds);

        mgr.unload("sdi-plugin", &mut sdi, &mut vfs, &mut cmds)
            .unwrap();
        assert_eq!(mgr.count(), 1);
        assert!(!sdi.contains("plugin_widget"));
        assert_eq!(mgr.active_count(), 1);

        mgr.shutdown_all(&mut sdi, &mut vfs, &mut cmds);
        assert_eq!(mgr.active_count(), 0);
    }

    // -- API version validation tests --

    /// Plugin with a mismatched API version.
    struct BadVersionPlugin;
    impl Plugin for BadVersionPlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo::new("bad-version", "1.0.0").with_api_version(999)
        }
        fn init(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
        fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
        fn shutdown(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn api_version_mismatch_init_all() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(BadVersionPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        // init_all logs the error and skips the plugin instead of
        // propagating, so it completes without panic and the bad
        // plugin stays inactive.
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        assert_eq!(mgr.active_count(), 0);
    }

    #[test]
    fn api_version_mismatch_init_plugin() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(BadVersionPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        let result = mgr.init_plugin("bad-version", &mut sdi, &mut vfs, &mut cmds);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("API version mismatch"), "got: {msg}");
    }

    // -- Plugin app registration tests --

    /// Plugin that registers an app during init.
    struct AppPlugin;
    impl Plugin for AppPlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo::new("app-plugin", "1.0.0")
        }
        fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
            host.register_app(PluginAppRegistration::new(
                "Plugin App",
                crate::plugin::AppCategory::Utility,
                |path, _vfs| {
                    // Return a simple placeholder app.
                    Box::new(crate::apps::simple_app::SimpleApp::new(
                        "Plugin App",
                        path,
                        vec!["Plugin app content".to_string()],
                    ))
                },
            ))?;
            Ok(())
        }
        fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
        fn shutdown(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn plugin_registers_app() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(AppPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        assert!(mgr.plugin_apps().is_empty());
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);

        assert_eq!(mgr.plugin_apps().len(), 1);
        assert_eq!(mgr.plugin_apps()[0].title, "Plugin App");
    }

    #[test]
    fn create_plugin_app_found() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(AppPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);

        let app = mgr.create_plugin_app("Plugin App", &vfs);
        assert!(app.is_some());
        assert_eq!(app.unwrap().title(), "Plugin App");
    }

    #[test]
    fn create_plugin_app_not_found() {
        let mgr = PluginManager::new();
        let vfs = MemoryVfs::new();
        assert!(mgr.create_plugin_app("Nonexistent", &vfs).is_none());
    }

    #[test]
    fn plugin_apps_empty_by_default() {
        let mgr = PluginManager::new();
        assert!(mgr.plugin_apps().is_empty());
    }

    // -- Phase 3.6: Plugin adversarial & edge-case tests --

    /// Plugin that fails during init.
    struct FailInitPlugin;
    impl Plugin for FailInitPlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo::new("fail-init", "1.0.0")
        }
        fn init(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Err(OasisError::Plugin("init explosion".into()))
        }
        fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
        fn shutdown(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
    }

    /// Plugin that fails during update.
    struct FailUpdatePlugin {
        update_count: u32,
    }
    impl FailUpdatePlugin {
        fn new() -> Self {
            Self { update_count: 0 }
        }
    }
    impl Plugin for FailUpdatePlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo::new("fail-update", "1.0.0")
        }
        fn init(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
        fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            self.update_count += 1;
            Err(OasisError::Plugin("update explosion".into()))
        }
        fn shutdown(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
    }

    /// Plugin that fails during shutdown.
    struct FailShutdownPlugin;
    impl Plugin for FailShutdownPlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo::new("fail-shutdown", "1.0.0")
        }
        fn init(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
        fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
        fn shutdown(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Err(OasisError::Plugin("shutdown explosion".into()))
        }
    }

    /// Plugin with a duplicate name.
    struct DuplicatePlugin;
    impl Plugin for DuplicatePlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo::new("test-plugin", "2.0.0")
        }
        fn init(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
        fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
        fn shutdown(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
    }

    /// Plugin that writes a VFS file during init and reads it
    /// during update.
    struct VfsPlugin {
        read_data: Option<Vec<u8>>,
    }
    impl VfsPlugin {
        fn new() -> Self {
            Self { read_data: None }
        }
    }
    impl Plugin for VfsPlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo::new("vfs-plugin", "1.0.0")
        }
        fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
            host.vfs.write("/plugin_data.txt", b"hello vfs")?;
            Ok(())
        }
        fn update(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
            self.read_data = Some(host.vfs.read("/plugin_data.txt")?);
            Ok(())
        }
        fn shutdown(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
            host.vfs.remove("/plugin_data.txt")?;
            Ok(())
        }
    }

    #[test]
    fn init_all_skips_failed_plugin() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(FailInitPlugin));
        mgr.register_static(Box::new(TestPlugin::new()));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        // init_all logs the error for FailInitPlugin but
        // continues to successfully init TestPlugin.
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        assert_eq!(mgr.active_count(), 1);
    }

    #[test]
    fn init_plugin_not_found() {
        let mut mgr = PluginManager::new();
        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        let result = mgr.init_plugin("ghost", &mut sdi, &mut vfs, &mut cmds);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("plugin not found"),);
    }

    #[test]
    fn init_stopped_plugin_fails() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(TestPlugin::new()));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        mgr.shutdown_all(&mut sdi, &mut vfs, &mut cmds);

        // Plugin is now Stopped -- re-init should fail.
        let result = mgr.init_plugin("test-plugin", &mut sdi, &mut vfs, &mut cmds);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("stopped"),);
    }

    #[test]
    fn update_all_skips_failed_plugin() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(FailUpdatePlugin::new()));
        mgr.register_static(Box::new(TestPlugin::new()));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        assert_eq!(mgr.active_count(), 2);
        // update_all logs the error for FailUpdatePlugin but
        // continues to update TestPlugin without aborting.
        mgr.update_all(&mut sdi, &mut vfs, &mut cmds);
    }

    #[test]
    fn shutdown_all_skips_failed_plugin() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(FailShutdownPlugin));
        mgr.register_static(Box::new(TestPlugin::new()));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        assert_eq!(mgr.active_count(), 2);
        // shutdown_all logs the error for FailShutdownPlugin but
        // continues to shut down TestPlugin. Both end up Stopped.
        mgr.shutdown_all(&mut sdi, &mut vfs, &mut cmds);
        assert_eq!(mgr.active_count(), 0);
    }

    #[test]
    fn unload_shutdown_error_propagates() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(FailShutdownPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        let result = mgr.unload("fail-shutdown", &mut sdi, &mut vfs, &mut cmds);
        assert!(result.is_err());
    }

    #[test]
    fn unload_registered_plugin_no_shutdown() {
        // Unloading a Registered (never initialized) plugin
        // should not call shutdown.
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(FailShutdownPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        // Do NOT init -- unload should succeed because
        // shutdown is only called on Active plugins.
        let result = mgr.unload("fail-shutdown", &mut sdi, &mut vfs, &mut cmds);
        assert!(result.is_ok());
        assert_eq!(mgr.count(), 0);
    }

    #[test]
    fn duplicate_name_both_registered() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(TestPlugin::new()));
        mgr.register_static(Box::new(DuplicatePlugin));
        assert_eq!(mgr.count(), 2);

        // Both have name "test-plugin".
        // init_plugin finds the first one.
        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        mgr.init_plugin("test-plugin", &mut sdi, &mut vfs, &mut cmds)
            .unwrap();
        assert_eq!(mgr.active_count(), 1);

        // Second init of same name should fail (first is Active).
        let result = mgr.init_plugin("test-plugin", &mut sdi, &mut vfs, &mut cmds);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already active"),);
    }

    #[test]
    fn update_skips_non_active_plugins() {
        let mut mgr = PluginManager::new();
        // Register but do not init -- update should be a no-op.
        mgr.register_static(Box::new(FailUpdatePlugin::new()));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        // update_all is a no-op because FailUpdatePlugin
        // is in Registered state, not Active.
        mgr.update_all(&mut sdi, &mut vfs, &mut cmds);
    }

    #[test]
    fn shutdown_skips_non_active_plugins() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(FailShutdownPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        // Plugin is Registered, not Active -- shutdown is
        // a no-op.
        mgr.shutdown_all(&mut sdi, &mut vfs, &mut cmds);
    }

    #[test]
    fn empty_manager_operations() {
        let mut mgr = PluginManager::new();
        assert_eq!(mgr.count(), 0);
        assert_eq!(mgr.active_count(), 0);
        assert!(!mgr.is_loaded("anything"));
        assert!(mgr.list().is_empty());

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        // All lifecycle ops on empty manager should succeed.
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        mgr.update_all(&mut sdi, &mut vfs, &mut cmds);
        mgr.shutdown_all(&mut sdi, &mut vfs, &mut cmds);
    }

    #[test]
    fn plugin_vfs_interaction() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(VfsPlugin::new()));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);

        // VfsPlugin wrote a file during init -- verify.
        assert!(vfs.exists("/plugin_data.txt"));
        let data = vfs.read("/plugin_data.txt").unwrap();
        assert_eq!(data, b"hello vfs");

        mgr.update_all(&mut sdi, &mut vfs, &mut cmds);

        // Shutdown should clean up the VFS file.
        mgr.shutdown_all(&mut sdi, &mut vfs, &mut cmds);
        assert!(!vfs.exists("/plugin_data.txt"));
    }

    #[test]
    fn plugin_default_manager() {
        let mgr = PluginManager::default();
        assert_eq!(mgr.count(), 0);
    }

    #[test]
    fn discover_invalid_toml_ignored() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/etc").unwrap();
        vfs.mkdir("/etc/oasis-os").unwrap();
        vfs.mkdir("/etc/oasis-os/plugins").unwrap();
        vfs.mkdir("/etc/oasis-os/plugins/bad").unwrap();
        vfs.write(
            "/etc/oasis-os/plugins/bad/plugin.toml",
            b"this is {{{ not valid toml !!!",
        )
        .unwrap();

        let manifests = PluginManager::discover_manifests(&mut vfs);
        assert!(
            manifests.is_empty(),
            "invalid TOML should be silently skipped",
        );
    }

    #[test]
    fn discover_skips_files_in_plugin_dir() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/etc").unwrap();
        vfs.mkdir("/etc/oasis-os").unwrap();
        vfs.mkdir("/etc/oasis-os/plugins").unwrap();
        // Write a file (not a directory) directly in plugins/.
        vfs.write("/etc/oasis-os/plugins/stray.txt", b"not a plugin")
            .unwrap();

        let manifests = PluginManager::discover_manifests(&mut vfs);
        assert!(manifests.is_empty());
    }

    #[test]
    fn discover_multiple_manifests() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/etc").unwrap();
        vfs.mkdir("/etc/oasis-os").unwrap();
        vfs.mkdir("/etc/oasis-os/plugins").unwrap();

        for name in &["alpha", "beta", "gamma"] {
            let dir = format!("/etc/oasis-os/plugins/{name}");
            vfs.mkdir(&dir).unwrap();
            let toml = format!("name = \"{name}\"\nversion = \"1.0\"\n");
            vfs.write(&format!("{dir}/plugin.toml"), toml.as_bytes())
                .unwrap();
        }

        let manifests = PluginManager::discover_manifests(&mut vfs);
        assert_eq!(manifests.len(), 3);

        let names: Vec<&str> = manifests.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
        assert!(names.contains(&"gamma"));
    }

    #[test]
    fn manifest_missing_optional_fields() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/etc").unwrap();
        vfs.mkdir("/etc/oasis-os").unwrap();
        vfs.mkdir("/etc/oasis-os/plugins").unwrap();
        vfs.mkdir("/etc/oasis-os/plugins/minimal").unwrap();
        // Only required field is `name`.
        vfs.write(
            "/etc/oasis-os/plugins/minimal/plugin.toml",
            b"name = \"minimal\"\n",
        )
        .unwrap();

        let manifests = PluginManager::discover_manifests(&mut vfs);
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].name, "minimal");
        assert!(manifests[0].version.is_empty());
        assert!(manifests[0].author.is_empty());
        assert!(manifests[0].description.is_empty());
        assert!(manifests[0].library.is_empty());
        assert!(!manifests[0].auto_load);
    }

    #[test]
    fn is_loaded_after_unload() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(TestPlugin::new()));
        assert!(mgr.is_loaded("test-plugin"));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        mgr.unload("test-plugin", &mut sdi, &mut vfs, &mut cmds)
            .unwrap();
        assert!(!mgr.is_loaded("test-plugin"));
    }

    #[test]
    fn list_shows_correct_states() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(TestPlugin::new()));
        mgr.register_static(Box::new(SdiPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();

        // Init only one plugin.
        mgr.init_plugin("sdi-plugin", &mut sdi, &mut vfs, &mut cmds)
            .unwrap();

        let list = mgr.list();
        assert_eq!(list.len(), 2);
        // test-plugin should still be Registered.
        let tp = list
            .iter()
            .find(|(info, _)| info.name == "test-plugin")
            .unwrap();
        assert_eq!(tp.1, PluginState::Registered);
        // sdi-plugin should be Active.
        let sp = list
            .iter()
            .find(|(info, _)| info.name == "sdi-plugin")
            .unwrap();
        assert_eq!(sp.1, PluginState::Active);
    }
}
