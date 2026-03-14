//! Plugin system -- runtime-extensible functionality via static or dynamic plugins.
//!
//! Plugins implement the `Plugin` trait and interact with the OS through
//! a `PluginHost` that provides access to the SDI scene graph, command
//! registry, and virtual file system.
//!
//! Plugins can also register as launchable dashboard apps via the
//! [`app_bridge`] module.

pub mod app_bridge;
pub mod event_bus;
pub mod examples;
pub mod manager;
pub mod traits;

pub use app_bridge::{AppCategory, PluginAppFactory, PluginAppRegistration};
pub use event_bus::{EventBus, PluginEvent};
pub use examples::register_builtin_plugins;
pub use manager::{PluginManager, PluginManifest};
pub use traits::{
    PLUGIN_API_VERSION, Plugin, PluginCapabilities, PluginHost, PluginInfo, PluginState,
};
