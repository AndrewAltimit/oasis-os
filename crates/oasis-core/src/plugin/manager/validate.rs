//! Plugin API version validation.

use crate::error::{OasisError, PluginError, Result};

use crate::plugin::traits::{PLUGIN_API_VERSION, PluginInfo};

/// Validate a plugin's API version against the host.
pub(crate) fn validate_api_version(info: &PluginInfo) -> Result<()> {
    if info.api_version != PLUGIN_API_VERSION {
        return Err(OasisError::Plugin(PluginError::ApiMismatch {
            plugin: info.name.clone(),
            expected: PLUGIN_API_VERSION,
            found: info.api_version,
        }));
    }
    Ok(())
}
