//! App registry -- static table of app factories and lookup function.

use crate::active_theme::ActiveTheme;
use crate::vfs::Vfs;

/// Factory function signature for app constructors.
///
/// Every registered app receives the VFS path and a reference to the virtual
/// file system.  Apps that need additional configuration (e.g. Settings,
/// System Monitor) bake their defaults into the closure.
pub(crate) type AppFactory = fn(&str, &dyn Vfs) -> Box<dyn super::app_trait::App>;

/// Static registry of `(app_title, factory)` pairs.
///
/// The order does not matter -- lookup is linear by title.  To register a new
/// app, add a single entry here.
pub(crate) const APP_REGISTRY: &[(&str, AppFactory)] = &[
    ("File Manager", |path, vfs| {
        Box::new(oasis_app_file_manager::FileManagerApp::new(path, vfs))
    }),
    ("Settings", |path, vfs| {
        // Read the shell-published state (skin/resolution/backend) from VFS
        // so the Settings UI reflects the actually-running configuration
        // rather than compile-time defaults. Falls back to the PSP-native
        // baseline if the shell hasn't populated the state yet.
        Box::new(oasis_app_settings::SettingsApp::from_vfs(
            path, vfs, "classic", 480, 272, "SDL3",
        ))
    }),
    ("Network", |path, _vfs| {
        Box::new(super::simple_app::SimpleApp::network(
            path, false, 9000, false,
        ))
    }),
    ("Package Manager", |path, _vfs| {
        Box::new(super::simple_app::SimpleApp::package_manager(path))
    }),
    ("Browser", |path, _vfs| {
        Box::new(super::simple_app::SimpleApp::browser(path))
    }),
    ("System Monitor", |path, _vfs| {
        Box::new(super::simple_app::SimpleApp::system_monitor(
            path,
            "Desktop (SDL2)",
            "SDL2",
            0,
        ))
    }),
    ("Terminal", |path, _vfs| {
        Box::new(super::simple_app::SimpleApp::terminal(path))
    }),
    ("Music Player", |path, vfs| {
        Box::new(oasis_app_media::BrowsingApp::music_player(path, vfs))
    }),
    ("Photo Viewer", |path, vfs| {
        Box::new(oasis_app_media::BrowsingApp::photo_viewer(path, vfs))
    }),
    ("Text Editor", |path, _vfs| {
        Box::new(oasis_app_text_editor::TextEditorApp::new(path))
    }),
    ("Calculator", |path, _vfs| {
        Box::new(oasis_app_calculator::CalculatorApp::new(path))
    }),
    ("Clock", |path, _vfs| {
        Box::new(oasis_app_clock::ClockApp::new(path))
    }),
    ("Paint", |path, _vfs| {
        Box::new(oasis_app_paint::PaintApp::new(path))
    }),
    ("Games", |path, _vfs| {
        Box::new(oasis_app_games::GamesApp::new(path))
    }),
    ("Internet Radio", |path, vfs| {
        Box::new(oasis_app_radio::RadioApp::new(path, vfs))
    }),
    ("TV Guide", |path, vfs| {
        Box::new(oasis_app_tv_guide::TvGuideApp::new(
            path,
            vfs,
            &ActiveTheme::default(),
        ))
    }),
];

/// Look up an app by title in the registry and construct it.
///
/// Returns `None` if the title is not registered (falls through to the
/// generic placeholder in `AppRunner::launch`).
pub(crate) fn create_app_delegate(
    title: &str,
    path: &str,
    vfs: &dyn Vfs,
) -> Option<Box<dyn super::app_trait::App>> {
    // Feature-gated app: Video Embed (WASM YouTube).
    #[cfg(feature = "wasm-youtube")]
    if title == "Video Embed" {
        return Some(Box::new(super::video_embed::VideoEmbedApp::new(path)));
    }

    APP_REGISTRY
        .iter()
        .find(|(name, _)| *name == title)
        .map(|(_, factory)| factory(path, vfs))
}

/// Create an app delegate with `file_path` pre-opened. Used by File Manager
/// when the user Confirms on a typed file (image, audio, text) to hand off
/// to the appropriate viewer. Falls back to [`create_app_delegate`] for
/// apps that have no "start pointing at this file" concept.
pub(crate) fn create_app_delegate_for_file(
    title: &str,
    path: &str,
    file_path: &str,
    vfs: &dyn Vfs,
) -> Option<Box<dyn super::app_trait::App>> {
    match title {
        "Music Player" => Some(Box::new(oasis_app_media::BrowsingApp::music_player_at(
            path, file_path, vfs,
        ))),
        "Photo Viewer" => Some(Box::new(oasis_app_media::BrowsingApp::photo_viewer_at(
            path, file_path, vfs,
        ))),
        "Text Editor" => Some(Box::new(
            oasis_app_text_editor::TextEditorApp::open_from_vfs(file_path, vfs),
        )),
        _ => create_app_delegate(title, path, vfs),
    }
}
