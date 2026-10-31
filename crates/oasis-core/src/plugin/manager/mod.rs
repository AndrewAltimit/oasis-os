//! Plugin lifecycle manager.
//!
//! Manages registration, initialization, per-frame updates, and shutdown
//! of plugins. Supports static registration (built-in plugins) and
//! manifest-based discovery from the VFS.

mod app_registry;
mod discovery;
mod host;
mod lifecycle;
mod registry;
mod validate;

use serde::Deserialize;

#[cfg(test)]
use crate::error::OasisError;
use crate::error::Result;
use crate::sdi::SdiRegistry;
use crate::terminal::CommandRegistry;
use crate::vfs::Vfs;

use super::app_bridge::PluginAppRegistration;
use super::traits::{
    Plugin, PluginCapabilities, PluginContext, PluginHost, PluginInfo, PluginState,
};

/// A loaded plugin with its runtime state.
struct LoadedPlugin {
    plugin: Box<dyn Plugin>,
    state: PluginState,
    /// Titles of apps registered by this plugin (for cleanup on unload).
    registered_app_titles: Vec<String>,
    /// Commands this plugin's `init` registered (removed on stop/unload).
    registered_commands: Vec<String>,
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
    /// What the host told us about the frame (screen size, time), handed
    /// to every lifecycle call as [`PluginHost::context`].
    context: PluginContext,
}

impl PluginManager {
    /// Create a new empty plugin manager.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            plugin_apps: Vec::new(),
            context: PluginContext::default(),
        }
    }

    /// Set the host context (screen size, wall-clock time) plugins see
    /// from the next lifecycle call on. Hosts call this before `init_all`
    /// and before each `update_all`.
    pub fn set_context(&mut self, context: PluginContext) {
        self.context = context;
    }

    /// The host context last set with [`set_context`](Self::set_context).
    pub fn context(&self) -> PluginContext {
        self.context
    }

    /// Validate a plugin's API version against the host.
    fn validate_api_version(info: &PluginInfo) -> Result<()> {
        validate::validate_api_version(info)
    }

    /// Build a `PluginHost` for the given plugin index.
    fn build_host<'a>(
        plugin: &LoadedPlugin,
        context: PluginContext,
        sdi: &'a mut SdiRegistry,
        vfs: &'a mut dyn Vfs,
        commands: &'a mut CommandRegistry,
        pending_apps: &'a mut Vec<PluginAppRegistration>,
        capabilities_override: Option<PluginCapabilities>,
    ) -> PluginHost<'a> {
        host::build_host(
            plugin,
            context,
            sdi,
            vfs,
            commands,
            pending_apps,
            capabilities_override,
        )
    }

    /// Discover plugin manifests from the VFS plugin directory.
    ///
    /// Scans `/etc/oasis-os/plugins/` for `plugin.toml` files and returns
    /// their parsed manifests. This does NOT load the plugins -- it only
    /// discovers what's available.
    pub fn discover_manifests(vfs: &mut dyn Vfs) -> Vec<PluginManifest> {
        discovery::discover_manifests(vfs)
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
