//! Plugin system -- runtime-extensible functionality via static or dynamic plugins.
//!
//! Plugins implement the `Plugin` trait and interact with the OS through
//! a `PluginHost` that provides access to the SDI scene graph, command
//! registry, and virtual file system.

pub mod event_bus;
pub mod examples;
pub mod manager;
pub mod traits;

pub use event_bus::{EventBus, PluginEvent};
pub use examples::register_builtin_plugins;
pub use manager::{PluginManager, PluginManifest};
pub use traits::{Plugin, PluginHost, PluginInfo, PluginState};
