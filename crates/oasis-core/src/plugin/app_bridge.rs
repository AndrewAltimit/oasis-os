//! Plugin-to-app bridge: allows plugins to register as launchable dashboard apps.
//!
//! Plugins that want to appear on the dashboard register a
//! [`PluginAppRegistration`] during `init()` via [`PluginHost::register_app`](super::traits::PluginHost::register_app).
//! The registration includes a factory that creates [`App`] instances on demand.
//!
//! When the user launches a plugin app from the dashboard, `AppRunner` calls
//! the factory to create a fresh `App` delegate, just like built-in apps.

use oasis_types::backend::Color;
use oasis_vfs::Vfs;

use crate::dashboard::AppEntry;
use oasis_app_core::App;

/// Category for plugin-registered apps. Used for dashboard grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppCategory {
    Media,
    Utility,
    Network,
    System,
    Game,
}

/// Factory trait for creating [`App`] instances from plugin registrations.
///
/// Plugins provide an implementation that constructs their app delegate.
/// The factory is called each time the user launches the app from the
/// dashboard, so each launch gets a fresh instance.
pub trait PluginAppFactory: Send + Sync {
    /// Create a new app instance.
    ///
    /// `path` is the VFS path assigned to the app (e.g. `/plugins/my-app`).
    fn create_app(&self, path: &str, vfs: &dyn Vfs) -> Box<dyn App>;
}

/// Blanket impl for closures: `Fn(&str, &dyn Vfs) -> Box<dyn App>`.
impl<F> PluginAppFactory for F
where
    F: Fn(&str, &dyn Vfs) -> Box<dyn App> + Send + Sync,
{
    fn create_app(&self, path: &str, vfs: &dyn Vfs) -> Box<dyn App> {
        (self)(path, vfs)
    }
}

/// Registration data for a plugin-provided app.
pub struct PluginAppRegistration {
    /// Display title on the dashboard.
    pub title: String,
    /// Dashboard category for sorting/grouping.
    pub category: AppCategory,
    /// Fallback display color (used when no icon is available).
    pub color: Color,
    /// Factory that creates `App` instances on demand.
    pub factory: Box<dyn PluginAppFactory>,
}

impl std::fmt::Debug for PluginAppRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginAppRegistration")
            .field("title", &self.title)
            .field("category", &self.category)
            .finish_non_exhaustive()
    }
}

impl PluginAppRegistration {
    /// Create a new registration with a factory closure.
    ///
    /// # Example
    ///
    /// ```ignore
    /// host.register_app(PluginAppRegistration::new(
    ///     "My App",
    ///     AppCategory::Utility,
    ///     |path, _vfs| Box::new(MyApp::new(path)),
    /// ));
    /// ```
    pub fn new<F>(title: &str, category: AppCategory, factory: F) -> Self
    where
        F: Fn(&str, &dyn Vfs) -> Box<dyn App> + Send + Sync + 'static,
    {
        Self {
            title: title.to_string(),
            category,
            color: Color {
                r: 100,
                g: 149,
                b: 237,
                a: 255,
            },
            factory: Box::new(factory),
        }
    }

    /// Builder method to set the display color.
    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Derive a VFS path from the plugin title.
    fn vfs_path(&self) -> String {
        format!(
            "/plugins/{}",
            self.title
                .to_lowercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                })
                .collect::<String>()
        )
    }

    /// Convert to an [`AppEntry`] for dashboard display.
    pub fn to_app_entry(&self) -> AppEntry {
        AppEntry {
            title: self.title.clone(),
            path: self.vfs_path(),
            icon_png: Vec::new(),
            color: self.color,
        }
    }

    /// Create an [`App`] instance via the factory.
    pub fn create_app(&self, vfs: &dyn Vfs) -> Box<dyn App> {
        let path = self.vfs_path();
        self.factory.create_app(&path, vfs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_app_core::{App, AppAction};
    use oasis_sdi::SdiRegistry;
    use oasis_skin::ActiveTheme;
    use oasis_types::backend::SdiBackend;
    use oasis_types::input::Button;
    use oasis_vfs::MemoryVfs;

    /// Minimal test app for verifying the factory pattern.
    #[derive(Debug)]
    struct StubApp {
        title: String,
        path: String,
    }

    impl StubApp {
        fn new(title: &str, path: &str) -> Self {
            Self {
                title: title.to_string(),
                path: path.to_string(),
            }
        }
    }

    impl App for StubApp {
        fn title(&self) -> &str {
            &self.title
        }
        fn path(&self) -> &str {
            &self.path
        }
        fn handle_input(&mut self, _: &Button, _: &dyn Vfs) -> AppAction {
            AppAction::None
        }
        fn update_sdi(&mut self, _: &mut SdiRegistry, _: &ActiveTheme) {}
        fn draw_windowed(
            &self,
            _cx: i32,
            _cy: i32,
            _cw: u32,
            _ch: u32,
            _backend: &mut dyn SdiBackend,
            _at: &ActiveTheme,
        ) -> oasis_types::error::Result<()> {
            Ok(())
        }
        fn hide_sdi(&self, _: &mut SdiRegistry) {}
        fn lines(&self) -> &[String] {
            &[]
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn registration_new() {
        let reg = PluginAppRegistration::new("Test App", AppCategory::Utility, |path, _vfs| {
            Box::new(StubApp::new("Test App", path))
        });
        assert_eq!(reg.title, "Test App");
        assert_eq!(reg.category, AppCategory::Utility);
    }

    #[test]
    fn registration_to_app_entry() {
        let reg = PluginAppRegistration::new("My Plugin", AppCategory::Media, |path, _vfs| {
            Box::new(StubApp::new("My Plugin", path))
        });
        let entry = reg.to_app_entry();
        assert_eq!(entry.title, "My Plugin");
        assert_eq!(entry.path, "/plugins/my-plugin");
    }

    #[test]
    fn registration_create_app() {
        let reg = PluginAppRegistration::new("Factory Test", AppCategory::Utility, |path, _vfs| {
            Box::new(StubApp::new("Factory Test", path))
        });
        let vfs = MemoryVfs::new();
        let app = reg.create_app(&vfs);
        assert_eq!(app.title(), "Factory Test");
        assert_eq!(app.path(), "/plugins/factory-test");
    }

    #[test]
    fn registration_with_color() {
        let color = Color {
            r: 255,
            g: 0,
            b: 128,
            a: 255,
        };
        let reg = PluginAppRegistration::new("Colored", AppCategory::Game, |path, _vfs| {
            Box::new(StubApp::new("Colored", path))
        })
        .with_color(color);
        assert_eq!(reg.color, color);
        assert_eq!(reg.to_app_entry().color, color);
    }

    #[test]
    fn registration_debug() {
        let reg = PluginAppRegistration::new("Debug Test", AppCategory::System, |path, _vfs| {
            Box::new(StubApp::new("Debug Test", path))
        });
        let dbg = format!("{reg:?}");
        assert!(dbg.contains("Debug Test"));
        assert!(dbg.contains("System"));
    }

    #[test]
    fn category_eq() {
        assert_eq!(AppCategory::Media, AppCategory::Media);
        assert_ne!(AppCategory::Media, AppCategory::Utility);
    }

    #[test]
    fn factory_closure_blanket_impl() {
        // Verify that a plain closure works as a PluginAppFactory.
        let factory: Box<dyn PluginAppFactory> =
            Box::new(|path: &str, _vfs: &dyn Vfs| -> Box<dyn App> {
                Box::new(StubApp::new("Closure", path))
            });
        let vfs = MemoryVfs::new();
        let app = factory.create_app("/test", &vfs);
        assert_eq!(app.title(), "Closure");
    }

    #[test]
    fn multiple_registrations() {
        let mut regs = Vec::new();
        for name in &["App A", "App B", "App C"] {
            let n = name.to_string();
            regs.push(PluginAppRegistration::new(
                name,
                AppCategory::Utility,
                move |path, _vfs| Box::new(StubApp::new(&n, path)),
            ));
        }
        assert_eq!(regs.len(), 3);

        let vfs = MemoryVfs::new();
        let _titles: Vec<&str> = regs
            .iter()
            .map(|r| r.create_app(&vfs).title().to_string())
            .collect::<Vec<_>>()
            .iter()
            .map(|s| s.as_str())
            .collect();
        // Can't easily collect &str from temporary, just check count.
        assert_eq!(regs.len(), 3);

        // Verify each factory produces the right app.
        for reg in &regs {
            let app = reg.create_app(&vfs);
            assert_eq!(app.title(), reg.title);
        }
    }
}
