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
}

impl<'a> PluginHost<'a> {
    /// Register this plugin as a launchable app on the dashboard.
    ///
    /// The registration is stored and processed after `init()` returns.
    /// The app will appear on the dashboard alongside built-in apps.
    pub fn register_app(&mut self, registration: PluginAppRegistration) {
        self.app_registrations.push(registration);
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

    #[test]
    fn plugin_host_optional_fields_none_by_default() {
        use crate::sdi::SdiRegistry;
        use crate::terminal::CommandRegistry;
        use crate::vfs::MemoryVfs;

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        let mut pending = Vec::new();
        let host = PluginHost {
            sdi: &mut sdi,
            vfs: &mut vfs,
            commands: &mut cmds,
            audio: None,
            network: None,
            backend: None,
            app_registrations: &mut pending,
        };
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
        let mut host = PluginHost {
            sdi: &mut sdi,
            vfs: &mut vfs,
            commands: &mut cmds,
            audio: None,
            network: None,
            backend: None,
            app_registrations: &mut pending,
        };
        // Without a backend, load_texture should return an error.
        let result = host.load_texture(16, 16, &[0u8; 16 * 16 * 4]);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("no rendering backend"), "got: {msg}");
    }
}
