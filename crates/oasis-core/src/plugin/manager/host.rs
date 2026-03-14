//! Plugin host construction.

use crate::sdi::SdiRegistry;
use crate::terminal::CommandRegistry;
use crate::vfs::Vfs;

use crate::plugin::app_bridge::PluginAppRegistration;
use crate::plugin::traits::{PluginCapabilities, PluginHost};

use super::LoadedPlugin;

/// Build a `PluginHost` for the given plugin.
pub(crate) fn build_host<'a>(
    plugin: &LoadedPlugin,
    sdi: &'a mut SdiRegistry,
    vfs: &'a mut dyn Vfs,
    commands: &'a mut CommandRegistry,
    pending_apps: &'a mut Vec<PluginAppRegistration>,
    capabilities_override: Option<PluginCapabilities>,
) -> PluginHost<'a> {
    PluginHost {
        sdi,
        vfs,
        commands,
        audio: None,
        network: None,
        backend: None,
        app_registrations: pending_apps,
        capabilities: capabilities_override.unwrap_or_else(|| plugin.capabilities.clone()),
        plugin_name: plugin.plugin.info().name.clone(),
    }
}
