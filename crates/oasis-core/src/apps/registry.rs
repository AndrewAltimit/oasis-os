//! App registry: the launchable apps a host knows about.
//!
//! `oasis-core` does not know any concrete app. A host builds an
//! [`AppRegistry`] (the built-in apps come from the `oasis-apps` bundle's
//! `builtin_registry`, plugin apps are appended by
//! [`PluginManager::extend_registry`](crate::plugin::PluginManager::extend_registry))
//! and hands it to [`AppRunner::launch`](super::AppRunner::launch) /
//! [`AppRunner::launch_with_file`](super::AppRunner::launch_with_file).

use std::sync::Arc;

use crate::backend::Color;
use crate::dashboard::AppEntry;
use crate::sdi::SdiRegistry;
use crate::vfs::Vfs;

use super::app_trait::App;

/// Constructor of an app: `(vfs_path, vfs) -> app`.
pub type AppFactory = Arc<dyn Fn(&str, &dyn Vfs) -> Box<dyn App> + Send + Sync>;

/// Constructor of an app with a file pre-opened:
/// `(vfs_path, file_path, vfs) -> app`.
pub type FileFactory = Arc<dyn Fn(&str, &str, &dyn Vfs) -> Box<dyn App> + Send + Sync>;

/// Host-side media an app owns: the host stops it when the app's window
/// closes (otherwise audio keeps playing with no UI to control it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedMedia {
    /// The music player track (`/var/audio/*`).
    Music,
    /// The internet radio stream (`/var/radio/*`).
    Radio,
}

/// One launchable app.
#[derive(Clone)]
pub struct AppRegistration {
    /// Display title, also the lookup key (window ids derive from it).
    pub title: String,
    /// VFS path handed to the factory when launched by title (dashboard
    /// tiles carry their own path).
    pub path: String,
    /// Fallback tile color.
    pub color: Color,
    /// Whether the app gets a dashboard tile.
    pub on_dashboard: bool,
    /// Hide the app's own global SDI objects (fullscreen `update_sdi`
    /// render paths that create objects outside the shared app pools).
    pub hide_sdi: Option<fn(&mut SdiRegistry)>,
    /// Media the host stops when the app's window closes.
    pub owned_media: Option<OwnedMedia>,
    /// Registered by a plugin (replaced by
    /// [`PluginManager::extend_registry`](crate::plugin::PluginManager::extend_registry)).
    pub from_plugin: bool,
    factory: AppFactory,
    open_file: Option<FileFactory>,
}

impl std::fmt::Debug for AppRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppRegistration")
            .field("title", &self.title)
            .field("path", &self.path)
            .field("on_dashboard", &self.on_dashboard)
            .field("opens_files", &self.open_file.is_some())
            .field("owned_media", &self.owned_media)
            .field("from_plugin", &self.from_plugin)
            .finish_non_exhaustive()
    }
}

impl AppRegistration {
    /// Register `title`, built by `factory`. Defaults: path
    /// `/apps/<title>`, a grey tile color, no dashboard tile, no file
    /// support.
    pub fn new<F>(title: &str, factory: F) -> Self
    where
        F: Fn(&str, &dyn Vfs) -> Box<dyn App> + Send + Sync + 'static,
    {
        let entry = AppEntry::for_title(title);
        Self {
            title: entry.title,
            path: entry.path,
            color: entry.color,
            on_dashboard: false,
            hide_sdi: None,
            owned_media: None,
            from_plugin: false,
            factory: Arc::new(factory),
            open_file: None,
        }
    }

    /// Give the app a dashboard tile.
    pub fn on_dashboard(mut self) -> Self {
        self.on_dashboard = true;
        self
    }

    /// Let the app open files ("open with" hand-offs, `open <app> <file>`).
    pub fn open_file<F>(mut self, factory: F) -> Self
    where
        F: Fn(&str, &str, &dyn Vfs) -> Box<dyn App> + Send + Sync + 'static,
    {
        self.open_file = Some(Arc::new(factory));
        self
    }

    /// Set the cleanup for the app's own global SDI objects.
    pub fn hide_sdi(mut self, hide: fn(&mut SdiRegistry)) -> Self {
        self.hide_sdi = Some(hide);
        self
    }

    /// Declare media the host stops when the app's window closes.
    pub fn owns_media(mut self, media: OwnedMedia) -> Self {
        self.owned_media = Some(media);
        self
    }

    /// Override the VFS path used when launching by title.
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    /// Override the fallback tile color.
    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Whether the app can be launched with a file pre-opened.
    pub fn opens_files(&self) -> bool {
        self.open_file.is_some()
    }

    /// Construct the app at `path`.
    pub fn create(&self, path: &str, vfs: &dyn Vfs) -> Box<dyn App> {
        (self.factory)(path, vfs)
    }

    /// Construct the app at `path` with `file_path` pre-opened, or `None`
    /// when the app does not open files.
    pub fn create_for_file(
        &self,
        path: &str,
        file_path: &str,
        vfs: &dyn Vfs,
    ) -> Option<Box<dyn App>> {
        self.open_file.as_ref().map(|f| f(path, file_path, vfs))
    }

    /// A tile-less dashboard entry for launching the app by title.
    pub fn to_app_entry(&self) -> AppEntry {
        AppEntry {
            title: self.title.clone(),
            path: self.path.clone(),
            icon_png: Vec::new(),
            color: self.color,
        }
    }
}

/// The launchable apps of a host, in registration order.
#[derive(Debug, Clone, Default)]
pub struct AppRegistry {
    apps: Vec<AppRegistration>,
    /// Non-plugin apps a plugin app currently shadows (same title), put
    /// back by [`Self::remove_plugin_apps`].
    shadowed: Vec<AppRegistration>,
}

impl AppRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `reg`, replacing an app with the same title (so a plugin can
    /// shadow a built-in; [`Self::remove_plugin_apps`] brings the
    /// built-in back).
    pub fn register(&mut self, reg: AppRegistration) {
        match self.apps.iter_mut().find(|a| a.title == reg.title) {
            Some(slot) => {
                let old = std::mem::replace(slot, reg);
                if slot.from_plugin && !old.from_plugin {
                    self.shadowed.push(old);
                }
            },
            None => self.apps.push(reg),
        }
    }

    /// Remove the app titled `title`. Returns whether it was registered.
    pub fn unregister(&mut self, title: &str) -> bool {
        let before = self.apps.len();
        self.apps.retain(|a| a.title != title);
        self.apps.len() != before
    }

    /// Remove every plugin-registered app, restoring the apps they
    /// shadowed (at their original place in the order).
    pub fn remove_plugin_apps(&mut self) {
        for app in &mut self.apps {
            if app.from_plugin
                && let Some(i) = self.shadowed.iter().position(|s| s.title == app.title)
            {
                *app = self.shadowed.swap_remove(i);
            }
        }
        self.apps.retain(|a| !a.from_plugin);
        self.shadowed.clear();
    }

    /// All registrations, in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &AppRegistration> {
        self.apps.iter()
    }

    /// Titles of all registered apps, in registration order.
    pub fn titles(&self) -> impl Iterator<Item = &str> {
        self.apps.iter().map(|a| a.title.as_str())
    }

    /// Number of registered apps.
    pub fn len(&self) -> usize {
        self.apps.len()
    }

    /// Whether no app is registered.
    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    /// The app titled exactly `title`.
    pub fn get(&self, title: &str) -> Option<&AppRegistration> {
        self.apps.iter().find(|a| a.title == title)
    }

    /// The app titled `title`, ignoring ASCII case (for typed / agent
    /// input: `open text editor`, `OASIS_APP=paint`).
    pub fn find(&self, title: &str) -> Option<&AppRegistration> {
        self.get(title).or_else(|| {
            self.apps
                .iter()
                .find(|a| a.title.eq_ignore_ascii_case(title))
        })
    }

    /// Whether an app titled exactly `title` is registered.
    pub fn contains(&self, title: &str) -> bool {
        self.get(title).is_some()
    }

    /// Construct the app of dashboard entry `app` (at the entry's path).
    pub fn create(&self, app: &AppEntry, vfs: &dyn Vfs) -> Option<Box<dyn App>> {
        self.get(&app.title).map(|r| r.create(&app.path, vfs))
    }

    /// Construct the app of `app` with `file_path` pre-opened. `None` when
    /// the app is unknown or does not open files.
    pub fn create_for_file(
        &self,
        app: &AppEntry,
        file_path: &str,
        vfs: &dyn Vfs,
    ) -> Option<Box<dyn App>> {
        self.get(&app.title)?
            .create_for_file(&app.path, file_path, vfs)
    }

    /// Media the app titled `title` owns (see [`OwnedMedia`]).
    pub fn owned_media(&self, title: &str) -> Option<OwnedMedia> {
        self.get(title).and_then(|a| a.owned_media)
    }

    /// Run every app's [`AppRegistration::hide_sdi`] cleanup.
    pub fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        for hide in self.apps.iter().filter_map(|a| a.hide_sdi) {
            hide(sdi);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::simple_app::SimpleApp;
    use crate::vfs::MemoryVfs;

    fn simple(title: &'static str) -> AppRegistration {
        AppRegistration::new(title, move |path, _vfs| {
            Box::new(SimpleApp::new(title, path, vec![title.to_string()]))
        })
    }

    #[test]
    fn registration_defaults() {
        let reg = simple("Notes");
        assert_eq!(reg.path, "/apps/Notes");
        assert!(!reg.on_dashboard);
        assert!(!reg.opens_files());
        assert!(reg.owned_media.is_none());
        let entry = reg.to_app_entry();
        assert_eq!(entry.title, "Notes");
        assert_eq!(entry.path, "/apps/Notes");
    }

    #[test]
    fn create_uses_the_entry_path() {
        let mut apps = AppRegistry::new();
        apps.register(simple("Notes"));
        let vfs = MemoryVfs::new();
        let mut entry = AppEntry::for_title("Notes");
        entry.path = "/apps/notes-dir".into();
        let app = apps.create(&entry, &vfs).unwrap();
        assert_eq!(app.path(), "/apps/notes-dir");
        assert!(apps.create(&AppEntry::for_title("Missing"), &vfs).is_none());
    }

    #[test]
    fn register_replaces_same_title() {
        let mut apps = AppRegistry::new();
        apps.register(simple("Notes"));
        apps.register(simple("Notes").on_dashboard());
        assert_eq!(apps.len(), 1);
        assert!(apps.get("Notes").unwrap().on_dashboard);
        assert!(apps.unregister("Notes"));
        assert!(apps.is_empty());
    }

    #[test]
    fn find_ignores_case_but_prefers_exact() {
        let mut apps = AppRegistry::new();
        apps.register(simple("Text Editor"));
        assert_eq!(apps.find("text editor").unwrap().title, "Text Editor");
        assert!(apps.get("text editor").is_none());
        assert!(apps.find("editor").is_none());
    }

    #[test]
    fn file_factory_is_optional() {
        let mut apps = AppRegistry::new();
        apps.register(simple("Plain"));
        apps.register(simple("Viewer").open_file(|path, file, _vfs| {
            Box::new(SimpleApp::new("Viewer", path, vec![file.to_string()]))
        }));
        let vfs = MemoryVfs::new();
        let viewer = AppEntry::for_title("Viewer");
        let app = apps.create_for_file(&viewer, "/a.txt", &vfs).unwrap();
        assert_eq!(app.lines(), ["/a.txt".to_string()]);
        assert!(
            apps.create_for_file(&AppEntry::for_title("Plain"), "/a.txt", &vfs)
                .is_none()
        );
    }

    #[test]
    fn plugin_apps_are_removed_together() {
        let mut apps = AppRegistry::new();
        apps.register(simple("Builtin"));
        let mut plugin = simple("Plugin");
        plugin.from_plugin = true;
        apps.register(plugin);
        apps.remove_plugin_apps();
        assert_eq!(apps.titles().collect::<Vec<_>>(), ["Builtin"]);
    }

    /// A plugin app may shadow a built-in with the same title; removing
    /// the plugin apps (every registry re-sync does) used to delete the
    /// built-in for the rest of the session.
    #[test]
    fn removing_a_shadowing_plugin_app_restores_the_builtin() {
        let mut apps = AppRegistry::new();
        apps.register(simple("First"));
        apps.register(simple("Calculator"));
        apps.register(simple("Last"));
        let mut plugin = simple("Calculator").with_path("/plugins/calculator");
        plugin.from_plugin = true;
        apps.register(plugin);
        assert_eq!(apps.get("Calculator").unwrap().path, "/plugins/calculator");
        apps.remove_plugin_apps();
        assert_eq!(
            apps.titles().collect::<Vec<_>>(),
            ["First", "Calculator", "Last"]
        );
        let builtin = apps.get("Calculator").unwrap();
        assert!(!builtin.from_plugin);
        assert_ne!(builtin.path, "/plugins/calculator");
        // A second removal is a no-op.
        apps.remove_plugin_apps();
        assert_eq!(apps.titles().count(), 3);
    }

    #[test]
    fn hide_sdi_runs_every_cleanup() {
        fn hide(sdi: &mut SdiRegistry) {
            if let Ok(obj) = sdi.get_mut("custom_obj") {
                obj.visible = false;
            }
        }
        let mut apps = AppRegistry::new();
        apps.register(simple("Custom").hide_sdi(hide));
        let mut sdi = SdiRegistry::new();
        sdi.create("custom_obj").visible = true;
        apps.hide_sdi(&mut sdi);
        assert!(!sdi.get("custom_obj").unwrap().visible);
    }

    #[test]
    fn owned_media_by_title() {
        let mut apps = AppRegistry::new();
        apps.register(simple("Radio").owns_media(OwnedMedia::Radio));
        assert_eq!(apps.owned_media("Radio"), Some(OwnedMedia::Radio));
        assert_eq!(apps.owned_media("Missing"), None);
    }
}
