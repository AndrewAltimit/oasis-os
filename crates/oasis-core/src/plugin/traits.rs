//! Plugin trait and host API definitions.
//!
//! Plugins extend OASIS_OS with new commands, UI elements, and behaviors.
//! They interact with the OS through a `PluginHost` that provides access
//! to the SDI scene graph, command registry, and virtual file system.

use crate::backend::{AudioBackend, NetworkBackend, SdiCore, TextureId};
use crate::error::Result;
use crate::sdi::SdiRegistry;
use crate::terminal::CommandRegistry;
use crate::vfs::Vfs;

use super::app_bridge::PluginAppRegistration;

/// Current plugin API version. Incremented on breaking changes only.
pub const PLUGIN_API_VERSION: u32 = 1;

/// Declares which OS subsystems a plugin needs access to.
///
/// Plugins return this from [`Plugin::capabilities()`]. The default
/// implementation grants all capabilities for backwards compatibility.
/// `PluginHost` uses soft enforcement: access to a subsystem that the
/// plugin did not declare logs a warning and returns an error rather
/// than panicking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCapabilities {
    /// Read files from the virtual file system.
    pub vfs_read: bool,
    /// Write files to the virtual file system.
    pub vfs_write: bool,
    /// Register terminal commands.
    pub commands: bool,
    /// Access audio playback.
    pub audio: bool,
    /// Access network (TCP) connections.
    pub network: bool,
    /// Register as a launchable dashboard app.
    pub app_registration: bool,
}

impl Default for PluginCapabilities {
    /// Defaults to all capabilities enabled (backwards compatible).
    fn default() -> Self {
        Self::all()
    }
}

impl PluginCapabilities {
    /// All capabilities enabled. Used as the default so existing
    /// plugins that do not override `capabilities()` keep working.
    pub fn all() -> Self {
        Self {
            vfs_read: true,
            vfs_write: true,
            commands: true,
            audio: true,
            network: true,
            app_registration: true,
        }
    }

    /// No capabilities enabled. Useful as a starting point for
    /// restrictive plugins that opt in to specific subsystems.
    pub fn none() -> Self {
        Self {
            vfs_read: false,
            vfs_write: false,
            commands: false,
            audio: false,
            network: false,
            app_registration: false,
        }
    }
}

/// Metadata about a plugin.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    /// Plugin name (unique identifier).
    pub name: String,
    /// Semantic version string.
    pub version: String,
    /// Plugin author.
    pub author: String,
    /// One-line description.
    pub description: String,
    /// Plugin API version this plugin was compiled against.
    /// Must match [`PLUGIN_API_VERSION`] at load time.
    pub api_version: u32,
}

impl PluginInfo {
    /// Create a new `PluginInfo` with the given name and version.
    ///
    /// `api_version` defaults to [`PLUGIN_API_VERSION`].
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            author: String::new(),
            description: String::new(),
            api_version: PLUGIN_API_VERSION,
        }
    }

    /// Builder method to set the author.
    pub fn with_author(mut self, author: &str) -> Self {
        self.author = author.to_string();
        self
    }

    /// Builder method to set the description.
    pub fn with_description(mut self, description: &str) -> Self {
        self.description = description.to_string();
        self
    }

    /// Builder method to override the API version.
    pub fn with_api_version(mut self, api_version: u32) -> Self {
        self.api_version = api_version;
        self
    }
}

/// Host-side context passed to plugins during lifecycle calls.
///
/// Provides access to OS services that plugins can use to register
/// commands, create UI elements, read/write files, play audio, and
/// make network requests.
///
/// Access to subsystems is gated by [`PluginCapabilities`]. Use the
/// `checked_*` methods for capability-aware access. Direct field access
/// is still available for backwards compatibility but bypasses checks.
pub struct PluginHost<'a> {
    /// SDI scene graph for creating/modifying UI elements.
    pub sdi: &'a mut SdiRegistry,
    /// Virtual file system for reading/writing files.
    pub vfs: &'a mut dyn Vfs,
    /// Command registry for registering new commands.
    pub commands: &'a mut CommandRegistry,
    /// Audio backend for playback. `None` in headless/screenshot mode.
    pub audio: Option<&'a mut dyn AudioBackend>,
    /// Network backend for TCP connections. `None` if networking is
    /// unavailable.
    pub network: Option<&'a mut dyn NetworkBackend>,
    /// Rendering backend for texture loading. `None` if no backend is
    /// available (e.g. during headless init).
    pub backend: Option<&'a mut dyn SdiCore>,
    /// Accumulator for plugin app registrations. Processed by the
    /// manager after `init()` returns.
    pub(crate) app_registrations: &'a mut Vec<PluginAppRegistration>,
    /// Capabilities declared by the plugin. Set by the manager before
    /// each lifecycle call.
    pub(crate) capabilities: PluginCapabilities,
    /// Plugin name, used for capability-violation log messages.
    pub(crate) plugin_name: String,
}

impl<'a> PluginHost<'a> {
    /// Return the capabilities currently in effect for this plugin.
    pub fn capabilities(&self) -> &PluginCapabilities {
        &self.capabilities
    }

    /// Register this plugin as a launchable app on the dashboard.
    ///
    /// Requires [`PluginCapabilities::app_registration`]. If the
    /// capability is not declared, logs a warning and returns an error.
    pub fn register_app(&mut self, registration: PluginAppRegistration) -> Result<()> {
        if !self.capabilities.app_registration {
            log::warn!(
                "Plugin '{}' tried to register an app without app_registration capability",
                self.plugin_name,
            );
            return Err(crate::error::OasisError::Plugin(
                format!(
                    "plugin '{}' lacks app_registration capability",
                    self.plugin_name
                )
                .into(),
            ));
        }
        self.app_registrations.push(registration);
        Ok(())
    }

    /// Capability-checked access to the command registry.
    ///
    /// Returns `Err` if `commands` capability is not declared.
    pub fn checked_commands(&mut self) -> Result<&mut CommandRegistry> {
        if !self.capabilities.commands {
            log::warn!(
                "Plugin '{}' tried to access commands without capability",
                self.plugin_name,
            );
            return Err(crate::error::OasisError::Plugin(
                format!("plugin '{}' lacks commands capability", self.plugin_name).into(),
            ));
        }
        Ok(self.commands)
    }

    /// Capability-checked read access to the VFS.
    ///
    /// Returns `Err` if `vfs_read` capability is not declared.
    pub fn checked_vfs_read(&self) -> Result<&dyn Vfs> {
        if !self.capabilities.vfs_read {
            log::warn!(
                "Plugin '{}' tried to read VFS without vfs_read capability",
                self.plugin_name,
            );
            return Err(crate::error::OasisError::Plugin(
                format!("plugin '{}' lacks vfs_read capability", self.plugin_name).into(),
            ));
        }
        Ok(self.vfs)
    }

    /// Capability-checked write access to the VFS.
    ///
    /// Returns `Err` if `vfs_write` capability is not declared.
    pub fn checked_vfs_write(&mut self) -> Result<&mut dyn Vfs> {
        if !self.capabilities.vfs_write {
            log::warn!(
                "Plugin '{}' tried to write VFS without vfs_write capability",
                self.plugin_name,
            );
            return Err(crate::error::OasisError::Plugin(
                format!("plugin '{}' lacks vfs_write capability", self.plugin_name).into(),
            ));
        }
        Ok(self.vfs)
    }

    /// Check whether the `audio` capability is declared.
    ///
    /// Returns `Err` if the capability is not declared. On success the
    /// caller can access `self.audio` directly.
    pub fn check_audio(&self) -> Result<()> {
        if !self.capabilities.audio {
            log::warn!(
                "Plugin '{}' tried to access audio without capability",
                self.plugin_name,
            );
            return Err(crate::error::OasisError::Plugin(
                format!("plugin '{}' lacks audio capability", self.plugin_name).into(),
            ));
        }
        Ok(())
    }

    /// Check whether the `network` capability is declared.
    ///
    /// Returns `Err` if the capability is not declared. On success the
    /// caller can access `self.network` directly.
    pub fn check_network(&self) -> Result<()> {
        if !self.capabilities.network {
            log::warn!(
                "Plugin '{}' tried to access network without capability",
                self.plugin_name,
            );
            return Err(crate::error::OasisError::Plugin(
                format!("plugin '{}' lacks network capability", self.plugin_name).into(),
            ));
        }
        Ok(())
    }

    /// Load a texture from raw RGBA pixel data.
    ///
    /// Returns a texture handle that can be assigned to SDI objects.
    /// Requires a rendering backend (`host.backend` must be `Some`).
    pub fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId> {
        let backend = self.backend.as_mut().ok_or_else(|| {
            crate::error::OasisError::Backend("no rendering backend available".into())
        })?;
        backend.load_texture(width, height, rgba_data)
    }

    /// Destroy a previously loaded texture.
    ///
    /// Requires a rendering backend (`host.backend` must be `Some`).
    pub fn destroy_texture(&mut self, tex: TextureId) -> Result<()> {
        let backend = self.backend.as_mut().ok_or_else(|| {
            crate::error::OasisError::Backend("no rendering backend available".into())
        })?;
        backend.destroy_texture(tex)
    }
}

/// The plugin interface that all plugins must implement.
///
/// Lifecycle:
/// 1. `info()` -- called to get plugin metadata (before init)
/// 2. `init()` -- called once when the plugin is loaded
/// 3. `update()` -- called once per frame (optional work)
/// 4. `shutdown()` -- called when the plugin is unloaded
pub trait Plugin {
    /// Return plugin metadata.
    fn info(&self) -> PluginInfo;

    /// Declare which OS subsystems this plugin needs.
    ///
    /// The default returns [`PluginCapabilities::all()`] so that existing
    /// plugins keep working without changes. Override this to restrict
    /// access (principle of least privilege).
    fn capabilities(&self) -> PluginCapabilities {
        PluginCapabilities::all()
    }

    /// Initialize the plugin. Register commands, create SDI objects, etc.
    fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()>;

    /// Per-frame update. Called once per main loop iteration.
    /// Most plugins can leave this as a no-op.
    fn update(&mut self, host: &mut PluginHost<'_>) -> Result<()>;

    /// Shutdown the plugin. Clean up SDI objects, deregister resources.
    fn shutdown(&mut self, host: &mut PluginHost<'_>) -> Result<()>;
}

/// Current state of a loaded plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    /// Plugin is registered but not yet initialized.
    Registered,
    /// Plugin has been initialized and is running.
    Active,
    /// Plugin has been shut down.
    Stopped,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_info_builder() {
        let info = PluginInfo::new("test-plugin", "1.0.0")
            .with_author("Test Author")
            .with_description("A test plugin");
        assert_eq!(info.name, "test-plugin");
        assert_eq!(info.version, "1.0.0");
        assert_eq!(info.author, "Test Author");
        assert_eq!(info.description, "A test plugin");
        assert_eq!(info.api_version, PLUGIN_API_VERSION);
    }

    #[test]
    fn plugin_info_defaults() {
        let info = PluginInfo::new("minimal", "0.1");
        assert_eq!(info.name, "minimal");
        assert!(info.author.is_empty());
        assert!(info.description.is_empty());
        assert_eq!(info.api_version, PLUGIN_API_VERSION);
    }

    #[test]
    fn plugin_info_custom_api_version() {
        let info = PluginInfo::new("test", "1.0").with_api_version(99);
        assert_eq!(info.api_version, 99);
    }

    /// Helper to create a `PluginHost` for tests with the given capabilities.
    fn make_test_host<'a>(
        sdi: &'a mut crate::sdi::SdiRegistry,
        vfs: &'a mut dyn crate::vfs::Vfs,
        commands: &'a mut crate::terminal::CommandRegistry,
        pending: &'a mut Vec<crate::plugin::PluginAppRegistration>,
        capabilities: PluginCapabilities,
    ) -> PluginHost<'a> {
        PluginHost {
            sdi,
            vfs,
            commands,
            audio: None,
            network: None,
            backend: None,
            app_registrations: pending,
            capabilities,
            plugin_name: "test".to_string(),
        }
    }

    #[test]
    fn plugin_host_optional_fields_none_by_default() {
        use crate::sdi::SdiRegistry;
        use crate::terminal::CommandRegistry;
        use crate::vfs::MemoryVfs;

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        let mut pending = Vec::new();
        let host = make_test_host(
            &mut sdi,
            &mut vfs,
            &mut cmds,
            &mut pending,
            PluginCapabilities::all(),
        );
        assert!(host.audio.is_none());
        assert!(host.network.is_none());
        assert!(host.backend.is_none());
    }

    #[test]
    fn plugin_host_load_texture_requires_backend() {
        use crate::sdi::SdiRegistry;
        use crate::terminal::CommandRegistry;
        use crate::vfs::MemoryVfs;

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        let mut pending = Vec::new();
        let mut host = make_test_host(
            &mut sdi,
            &mut vfs,
            &mut cmds,
            &mut pending,
            PluginCapabilities::all(),
        );
        // Without a backend, load_texture should return an error.
        let result = host.load_texture(16, 16, &[0u8; 16 * 16 * 4]);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("no rendering backend"), "got: {msg}");
    }

    #[test]
    fn capabilities_all_enables_everything() {
        let caps = PluginCapabilities::all();
        assert!(caps.vfs_read);
        assert!(caps.vfs_write);
        assert!(caps.commands);
        assert!(caps.audio);
        assert!(caps.network);
        assert!(caps.app_registration);
    }

    #[test]
    fn capabilities_none_disables_everything() {
        let caps = PluginCapabilities::none();
        assert!(!caps.vfs_read);
        assert!(!caps.vfs_write);
        assert!(!caps.commands);
        assert!(!caps.audio);
        assert!(!caps.network);
        assert!(!caps.app_registration);
    }

    #[test]
    fn capabilities_default_is_all() {
        assert_eq!(PluginCapabilities::default(), PluginCapabilities::all());
    }

    #[test]
    fn checked_commands_denied_without_capability() {
        use crate::sdi::SdiRegistry;
        use crate::terminal::CommandRegistry;
        use crate::vfs::MemoryVfs;

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        let mut pending = Vec::new();
        let mut host = make_test_host(
            &mut sdi,
            &mut vfs,
            &mut cmds,
            &mut pending,
            PluginCapabilities::none(),
        );
        assert!(host.checked_commands().is_err());
    }

    #[test]
    fn checked_commands_allowed_with_capability() {
        use crate::sdi::SdiRegistry;
        use crate::terminal::CommandRegistry;
        use crate::vfs::MemoryVfs;

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        let mut pending = Vec::new();
        let caps = PluginCapabilities {
            commands: true,
            ..PluginCapabilities::none()
        };
        let mut host = make_test_host(&mut sdi, &mut vfs, &mut cmds, &mut pending, caps);
        assert!(host.checked_commands().is_ok());
    }

    #[test]
    fn checked_vfs_read_denied_without_capability() {
        use crate::sdi::SdiRegistry;
        use crate::terminal::CommandRegistry;
        use crate::vfs::MemoryVfs;

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        let mut pending = Vec::new();
        let host = make_test_host(
            &mut sdi,
            &mut vfs,
            &mut cmds,
            &mut pending,
            PluginCapabilities::none(),
        );
        assert!(host.checked_vfs_read().is_err());
    }

    #[test]
    fn checked_vfs_write_denied_without_capability() {
        use crate::sdi::SdiRegistry;
        use crate::terminal::CommandRegistry;
        use crate::vfs::MemoryVfs;

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        let mut pending = Vec::new();
        let mut host = make_test_host(
            &mut sdi,
            &mut vfs,
            &mut cmds,
            &mut pending,
            PluginCapabilities::none(),
        );
        assert!(host.checked_vfs_write().is_err());
    }

    #[test]
    fn check_audio_denied_without_capability() {
        use crate::sdi::SdiRegistry;
        use crate::terminal::CommandRegistry;
        use crate::vfs::MemoryVfs;

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        let mut pending = Vec::new();
        let host = make_test_host(
            &mut sdi,
            &mut vfs,
            &mut cmds,
            &mut pending,
            PluginCapabilities::none(),
        );
        assert!(host.check_audio().is_err());
    }

    #[test]
    fn check_network_denied_without_capability() {
        use crate::sdi::SdiRegistry;
        use crate::terminal::CommandRegistry;
        use crate::vfs::MemoryVfs;

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        let mut pending = Vec::new();
        let host = make_test_host(
            &mut sdi,
            &mut vfs,
            &mut cmds,
            &mut pending,
            PluginCapabilities::none(),
        );
        assert!(host.check_network().is_err());
    }

    #[test]
    fn register_app_denied_without_capability() {
        use crate::sdi::SdiRegistry;
        use crate::terminal::CommandRegistry;
        use crate::vfs::MemoryVfs;

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        let mut pending = Vec::new();
        let mut host = make_test_host(
            &mut sdi,
            &mut vfs,
            &mut cmds,
            &mut pending,
            PluginCapabilities::none(),
        );
        let reg = crate::plugin::PluginAppRegistration::new(
            "Test",
            crate::plugin::AppCategory::Utility,
            |path, _vfs| {
                Box::new(crate::apps::simple_app::SimpleApp::new(
                    "Test",
                    path,
                    vec![],
                ))
            },
        );
        assert!(host.register_app(reg).is_err());
        assert!(pending.is_empty());
    }

    #[test]
    fn register_app_allowed_with_capability() {
        use crate::sdi::SdiRegistry;
        use crate::terminal::CommandRegistry;
        use crate::vfs::MemoryVfs;

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        let mut pending = Vec::new();
        let caps = PluginCapabilities {
            app_registration: true,
            ..PluginCapabilities::none()
        };
        let mut host = make_test_host(&mut sdi, &mut vfs, &mut cmds, &mut pending, caps);
        let reg = crate::plugin::PluginAppRegistration::new(
            "Test",
            crate::plugin::AppCategory::Utility,
            |path, _vfs| {
                Box::new(crate::apps::simple_app::SimpleApp::new(
                    "Test",
                    path,
                    vec![],
                ))
            },
        );
        assert!(host.register_app(reg).is_ok());
        assert_eq!(pending.len(), 1);
    }
}
