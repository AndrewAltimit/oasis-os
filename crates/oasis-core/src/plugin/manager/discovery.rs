//! Plugin manifest discovery from the VFS.

use crate::vfs::Vfs;

use super::PluginManifest;

/// Discover plugin manifests from the VFS plugin directory.
///
/// Scans `/etc/oasis-os/plugins/` for `plugin.toml` files and returns
/// their parsed manifests. This does NOT load the plugins -- it only
/// discovers what's available.
pub(crate) fn discover_manifests(vfs: &mut dyn Vfs) -> Vec<PluginManifest> {
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
