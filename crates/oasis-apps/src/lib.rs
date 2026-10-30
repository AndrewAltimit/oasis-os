//! The built-in OASIS_OS app bundle.
//!
//! `oasis-core` runs apps but knows none of them: a host builds an
//! [`AppRegistry`] and launches from it. This crate depends on every
//! `oasis-app-*` crate and provides
//!
//! - [`builtin_registry`]: one [`AppRegistration`] per built-in app, with
//!   the host's identity ([`HostInfo`]) baked into the apps that display
//!   it (Settings, System Monitor, Package Manager);
//! - [`register_commands`]: the app-specific terminal commands (`tv`);
//! - [`TvGuideRunnerExt`]: TV Guide access on an [`AppRunner`] for the
//!   hosts' video players.
//!
//! Editing an app crate rebuilds this bundle and the hosts, not
//! `oasis-core`. The PSP backend doesn't use `AppRunner` and imports the
//! app crates it needs (`oasis-app-tv-guide`) directly.

use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::{AppRegistration, AppRegistry, AppRunner, OwnedMedia, SimpleApp};
use oasis_core::sdi::SdiRegistry;
use oasis_core::terminal::CommandRegistry;

#[cfg(test)]
mod runner_tests;
mod tv_command;
#[cfg(feature = "video-embed")]
pub mod video_embed;

pub use tv_command::TvCmd;

// App crates whose types and IPC helpers hosts use directly (TV catalogs
// and players, the Settings high-contrast skin, the System Monitor probe,
// Photo Viewer / Music Player hand-offs).
pub use oasis_app_media as media;
pub use oasis_app_settings as settings;
pub use oasis_app_system_monitor as system_monitor;
pub use oasis_app_tv_guide as tv_guide;

/// Titles of the built-in apps (the registry keys; window ids derive from
/// them through `oasis_core::apps::window_id`).
pub mod titles {
    pub const FILE_MANAGER: &str = "File Manager";
    pub const SETTINGS: &str = "Settings";
    pub const TERMINAL: &str = "Terminal";
    pub const MUSIC_PLAYER: &str = "Music Player";
    pub const INTERNET_RADIO: &str = oasis_core::audio::RADIO_APP_TITLE;
    pub const PHOTO_VIEWER: &str = "Photo Viewer";
    pub const TEXT_EDITOR: &str = "Text Editor";
    pub const CALCULATOR: &str = "Calculator";
    pub const PAINT: &str = "Paint";
    pub const GAMES: &str = "Games";
    pub const TV_GUIDE: &str = "TV Guide";
    pub const SYSTEM_MONITOR: &str = "System Monitor";
    pub const NETWORK: &str = "Network";
    pub const PACKAGE_MANAGER: &str = "Package Manager";
    /// The WASM host's YouTube app (feature `video-embed`).
    pub const VIDEO_EMBED: &str = "Video Embed";
    /// Not a registered app: every host opens its own browser widget for
    /// this title.
    pub const BROWSER: &str = "Browser";
}

/// What the host tells the apps about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInfo {
    /// Platform shown by the System Monitor (e.g. `"Desktop (SDL3)"`).
    pub platform: String,
    /// Rendering backend name (System Monitor, Package Manager, and the
    /// Settings fallback before the host publishes its state).
    pub backend: String,
    /// Skin the Settings app shows before the host publishes its state.
    pub default_skin: String,
    /// Resolution the Settings app shows before the host publishes it.
    pub default_resolution: (u32, u32),
}

impl HostInfo {
    /// A host on `platform` rendering through `backend`, with the PSP
    /// baseline (`classic`, 480x272) as the Settings fallback.
    pub fn new(platform: impl Into<String>, backend: impl Into<String>) -> Self {
        Self {
            platform: platform.into(),
            backend: backend.into(),
            default_skin: "classic".to_string(),
            default_resolution: (480, 272),
        }
    }

    /// The desktop shell (`oasis-app`, SDL3).
    pub fn desktop() -> Self {
        Self::new("Desktop (SDL3)", "SDL3")
    }
}

impl Default for HostInfo {
    fn default() -> Self {
        Self::desktop()
    }
}

/// Every built-in app, in dashboard order. Apps with a dashboard tile are
/// marked [`AppRegistration::on_dashboard`]; the hosts also show the
/// Browser ([`titles::BROWSER`]), which they implement themselves.
pub fn builtin_registry(host: &HostInfo) -> AppRegistry {
    use titles::*;

    let mut apps = AppRegistry::new();
    apps.register(
        AppRegistration::new(FILE_MANAGER, |path, vfs| {
            Box::new(oasis_app_file_manager::FileManagerApp::new(path, vfs))
        })
        .on_dashboard(),
    );
    let (skin, (w, h), backend) = (
        host.default_skin.clone(),
        host.default_resolution,
        host.backend.clone(),
    );
    apps.register(
        AppRegistration::new(SETTINGS, move |path, vfs| {
            // The shell publishes the running skin / resolution / backend;
            // the host's defaults only cover the first frame before that.
            Box::new(oasis_app_settings::SettingsApp::from_vfs(
                path, vfs, &skin, w, h, &backend,
            ))
        })
        .on_dashboard(),
    );
    apps.register(
        AppRegistration::new(TERMINAL, |path, _vfs| Box::new(SimpleApp::terminal(path)))
            .on_dashboard(),
    );
    apps.register(
        AppRegistration::new(MUSIC_PLAYER, |path, vfs| {
            Box::new(oasis_app_media::BrowsingApp::music_player(path, vfs))
        })
        .open_file(|path, file, vfs| {
            Box::new(oasis_app_media::BrowsingApp::music_player_at(
                path, file, vfs,
            ))
        })
        .owns_media(OwnedMedia::Music)
        .on_dashboard(),
    );
    apps.register(
        AppRegistration::new(INTERNET_RADIO, |path, vfs| {
            Box::new(oasis_app_radio::RadioApp::new(path, vfs))
        })
        .owns_media(OwnedMedia::Radio)
        .on_dashboard(),
    );
    apps.register(
        AppRegistration::new(PHOTO_VIEWER, |path, vfs| {
            Box::new(oasis_app_media::BrowsingApp::photo_viewer(path, vfs))
        })
        .open_file(|path, file, vfs| {
            Box::new(oasis_app_media::BrowsingApp::photo_viewer_at(
                path, file, vfs,
            ))
        })
        .on_dashboard(),
    );
    apps.register(
        AppRegistration::new(TEXT_EDITOR, |path, _vfs| {
            Box::new(oasis_app_text_editor::TextEditorApp::new(path))
        })
        .open_file(|_path, file, vfs| {
            Box::new(oasis_app_text_editor::TextEditorApp::open_from_vfs(
                file, vfs,
            ))
        })
        .hide_sdi(hide_notepad_sdi)
        .on_dashboard(),
    );
    apps.register(
        AppRegistration::new(CALCULATOR, |path, _vfs| {
            Box::new(oasis_app_calculator::CalculatorApp::new(path))
        })
        .on_dashboard(),
    );
    apps.register(
        AppRegistration::new(PAINT, |path, _vfs| {
            Box::new(oasis_app_paint::PaintApp::new(path))
        })
        .on_dashboard(),
    );
    apps.register(
        AppRegistration::new(GAMES, |path, _vfs| {
            Box::new(oasis_app_games::GamesApp::new(path))
        })
        .on_dashboard(),
    );
    apps.register(
        AppRegistration::new(TV_GUIDE, |path, vfs| {
            Box::new(oasis_app_tv_guide::TvGuideApp::new(
                path,
                vfs,
                &ActiveTheme::default(),
            ))
        })
        .hide_sdi(hide_tv_guide_sdi)
        .on_dashboard(),
    );
    // Info screens without a tile (open them with `open <app>`, MCP
    // `open_app` or `OASIS_APP`).
    let (platform, backend) = (host.platform.clone(), host.backend.clone());
    apps.register(AppRegistration::new(SYSTEM_MONITOR, move |path, vfs| {
        Box::new(oasis_app_system_monitor::SystemMonitorApp::new(
            path,
            &platform,
            &backend,
            vfs.vfs_type(),
            0,
        ))
    }));
    apps.register(AppRegistration::new(NETWORK, |path, vfs| {
        Box::new(oasis_app_system_monitor::NetworkApp::new(path, vfs))
    }));
    let backend = host.backend.clone();
    apps.register(AppRegistration::new(PACKAGE_MANAGER, move |path, _vfs| {
        Box::new(oasis_app_system_monitor::PackageManagerApp::new(
            path, &backend,
        ))
    }));
    #[cfg(feature = "video-embed")]
    apps.register(
        AppRegistration::new(VIDEO_EMBED, |path, _vfs| {
            Box::new(video_embed::VideoEmbedApp::new(path))
        })
        .with_path("/apps/video-embed")
        .with_color(oasis_core::backend::Color::rgb(255, 0, 0))
        .on_dashboard(),
    );
    apps
}

/// Hide the TV Guide's own SDI objects. Both TV Guide render paths (grid
/// and expanded video) create `tv_hdr_bg` before any other `tv_*` object,
/// so until it exists there is nothing to hide: skip the ~200 name formats
/// and lookups the full pass costs every frame.
fn hide_tv_guide_sdi(sdi: &mut SdiRegistry) {
    if sdi.contains("tv_hdr_bg") {
        oasis_app_tv_guide::TvGuideState::hide_sdi(sdi);
    }
}

/// Hide the Text Editor's Notepad chrome (the text-editor crate owns the
/// pool sizes). `np_menu_bg` is the first `np_*` object its renderer
/// creates (same skip as [`hide_tv_guide_sdi`]).
fn hide_notepad_sdi(sdi: &mut SdiRegistry) {
    if sdi.contains("np_menu_bg") {
        oasis_app_text_editor::hide_notepad_sdi_objects(sdi);
    }
}

/// Register the app-specific terminal commands (`tv`).
pub fn register_commands(reg: &mut CommandRegistry) {
    tv_command::register_tv_commands(reg);
}

/// TV Guide access on a runner, for the hosts' catalog fetchers and video
/// players. `None` / no-op when the runner isn't the TV Guide.
pub trait TvGuideRunnerExt {
    /// The TV Guide's state. Doesn't schedule a redraw: hosts poll it every
    /// frame; visible changes go through [`Self::refresh_tv_text`] (or
    /// happen while video playback already forces redraws).
    fn tv_guide_state(&mut self) -> Option<&mut oasis_app_tv_guide::TvGuideState>;

    /// Refresh the TV Guide's text after catalog changes and schedule a
    /// redraw.
    fn refresh_tv_text(&mut self);
}

impl TvGuideRunnerExt for AppRunner {
    fn tv_guide_state(&mut self) -> Option<&mut oasis_app_tv_guide::TvGuideState> {
        self.delegate_as_mut_untracked::<oasis_app_tv_guide::TvGuideApp>()
            .map(|app| &mut app.guide)
    }

    fn refresh_tv_text(&mut self) {
        if let Some(tv) = self.delegate_as_mut::<oasis_app_tv_guide::TvGuideApp>() {
            tv.refresh_text();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_core::dashboard::AppEntry;
    use oasis_core::vfs::MemoryVfs;

    /// Every title the dashboard can show for a built-in app: the
    /// registry (minus the WASM-only video embed) plus the host browser.
    fn builtin_titles() -> Vec<String> {
        let apps = builtin_registry(&HostInfo::default());
        apps.titles()
            .filter(|t| *t != titles::VIDEO_EMBED)
            .chain([titles::BROWSER])
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn every_builtin_title_has_its_own_icon_category() {
        use oasis_core::vector::icon_set::IconCategory;
        let mut seen = std::collections::HashMap::new();
        for title in builtin_titles() {
            let cat = IconCategory::from_app_title(&title);
            assert_ne!(cat, IconCategory::Generic, "{title} has no icon category");
            if let Some(prev) = seen.insert(format!("{cat:?}"), title.clone()) {
                panic!("{title} shares icon category {cat:?} with {prev}");
            }
        }
    }

    /// Dashboard labels of every built-in title stay inside their cell on
    /// every built-in skin at 480x272, in both grid and free layout, and
    /// are never truncated on PSP-native skins.
    #[test]
    fn every_builtin_title_fits_its_dashboard_cell() {
        use oasis_core::active_theme::ActiveTheme;
        use oasis_core::dashboard::{DashboardConfig, DashboardState};
        use oasis_core::sdi::SdiRegistry;
        use oasis_core::skin::builtin::{builtin_names, load_builtin};
        use oasis_core::text_fit::{ELLIPSIS, text_width};

        let tiles: Vec<AppEntry> = builtin_titles()
            .iter()
            .map(|t| AppEntry::for_title(t))
            .collect();
        for &skin_name in builtin_names() {
            let skin = load_builtin(skin_name).unwrap();
            let at = ActiveTheme::from_skin(&skin.theme)
                .with_screen_size(480, 272)
                .with_features(&skin.features);
            let psp_native = skin.manifest.screen_width == 480;
            for free in [false, true] {
                let mut cfg = DashboardConfig::from_features(&skin.features, &at);
                cfg.free_layout = free;
                let cell_w = if free { cfg.free_cell_w } else { cfg.cell_w };
                let mut dash = DashboardState::new(cfg, tiles.clone());
                let mut labelled = 0;
                for _ in 0..dash.page_count() {
                    let mut sdi = SdiRegistry::new();
                    dash.update_sdi(&mut sdi, &at);
                    for name in sdi.names().filter(|n| n.starts_with("icon_label")) {
                        let obj = sdi.get(name).unwrap();
                        let Some(line) = obj.text.as_deref().filter(|_| obj.visible) else {
                            continue;
                        };
                        labelled += 1;
                        let what = format!("{skin_name} free={free}: {line:?}");
                        assert!(!psp_native || !line.ends_with(ELLIPSIS), "{what} truncated");
                        let w = text_width(line, obj.font_size);
                        assert!(w < cell_w, "{what} is {w}px in a {cell_w}px cell");
                    }
                    dash.next_page();
                    dash.tick_animation();
                }
                assert!(
                    labelled >= tiles.len(),
                    "{skin_name}: only {labelled} labels"
                );
            }
        }
    }

    #[test]
    fn every_registration_launches_its_own_title() {
        let apps = builtin_registry(&HostInfo::default());
        let vfs = MemoryVfs::new();
        for reg in apps.iter() {
            let runner = AppRunner::launch(&apps, &reg.to_app_entry(), &vfs);
            assert_eq!(runner.title, reg.title);
            assert!(!runner.lines().iter().any(|l| l.contains("No content")));
        }
    }

    #[test]
    fn file_capable_apps_open_files() {
        let apps = builtin_registry(&HostInfo::default());
        let file_apps: Vec<&str> = apps
            .iter()
            .filter(|r| r.opens_files())
            .map(|r| r.title.as_str())
            .collect();
        assert_eq!(
            file_apps,
            [
                titles::MUSIC_PLAYER,
                titles::PHOTO_VIEWER,
                titles::TEXT_EDITOR
            ]
        );
    }

    #[test]
    fn info_screens_have_no_tile() {
        let apps = builtin_registry(&HostInfo::default());
        for t in [
            titles::SYSTEM_MONITOR,
            titles::NETWORK,
            titles::PACKAGE_MANAGER,
        ] {
            assert!(!apps.get(t).unwrap().on_dashboard, "{t}");
        }
        assert!(
            apps.get(titles::BROWSER).is_none(),
            "the browser is host-provided"
        );
    }

    #[test]
    fn host_info_reaches_the_info_apps() {
        let host = HostInfo::new("Browser (WASM)", "Canvas 2D");
        let apps = builtin_registry(&host);
        let vfs = MemoryVfs::new();
        let entry = AppEntry::for_title(titles::SYSTEM_MONITOR);
        let mut sysmon = AppRunner::launch(&apps, &entry, &vfs);
        let app = sysmon
            .delegate_as_mut::<oasis_app_system_monitor::SystemMonitorApp>()
            .unwrap();
        assert_eq!(app.status().platform.as_deref(), Some("Browser (WASM)"));
        assert_eq!(app.status().backend.as_deref(), Some("Canvas 2D"));
        let pm = AppRunner::launch(&apps, &AppEntry::for_title(titles::PACKAGE_MANAGER), &vfs);
        assert!(pm.lines().iter().any(|l| l.contains("Canvas 2D")));
    }

    #[test]
    fn media_apps_own_their_media() {
        let apps = builtin_registry(&HostInfo::default());
        assert_eq!(
            apps.owned_media(titles::MUSIC_PLAYER),
            Some(OwnedMedia::Music)
        );
        assert_eq!(
            apps.owned_media(titles::INTERNET_RADIO),
            Some(OwnedMedia::Radio)
        );
        assert_eq!(apps.owned_media(titles::PAINT), None);
    }

    #[test]
    fn tv_guide_ext_reaches_only_the_tv_guide() {
        let apps = builtin_registry(&HostInfo::default());
        let vfs = MemoryVfs::new();
        let mut tv = AppRunner::launch(&apps, &AppEntry::for_title(titles::TV_GUIDE), &vfs);
        tv.mark_drawn();
        assert!(tv.tv_guide_state().is_some());
        assert!(!tv.wants_frame(), "polling the guide doesn't redraw");
        tv.refresh_tv_text();
        assert!(tv.wants_frame());
        let mut paint = AppRunner::launch(&apps, &AppEntry::for_title(titles::PAINT), &vfs);
        assert!(paint.tv_guide_state().is_none());
    }
}
