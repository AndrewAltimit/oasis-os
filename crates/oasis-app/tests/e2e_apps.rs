//! App registry flows through the real shell: the `open` terminal command,
//! the Network app's live service state, and resolution changes keeping
//! the running skin.
#![allow(clippy::unwrap_used)]

use std::net::TcpListener;
use std::time::Duration;

use oasis_app::Mode;
use oasis_app::harness::Harness;
use oasis_core::input::Key;
use oasis_core::vfs::Vfs;

/// A currently free loopback port (the terminal treats port 0 as "stop").
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Run `command` in the terminal (opened, or focused, through the same
/// path as `OASIS_APP`, so it works with windows on screen too).
fn run(h: &mut Harness, command: &str) {
    assert!(h.open_app("Terminal"));
    h.settle();
    h.type_text(command);
    h.key(Key::Enter);
    h.settle();
}

fn output_contains(h: &Harness, needle: &str) -> bool {
    h.state()
        .terminal
        .output_lines
        .iter()
        .any(|l| l.contains(needle))
}

/// `open <app> [file]` used to be documented but did not exist: apps
/// without a dashboard tile were reachable only through MCP.
#[test]
fn open_command_launches_apps_and_files() {
    let mut h = Harness::new("classic");
    h.settle();

    // Any registered app, case-insensitive, with or without a tile.
    run(&mut h, "open system monitor");
    assert_eq!(h.mode(), Mode::Desktop);
    assert!(
        h.find_window("System Monitor").is_some(),
        "{:?}",
        h.windows()
    );

    // With a file: the app opens it.
    run(
        &mut h,
        "open Photo Viewer /home/user/photos/oasis_sample.png",
    );
    let viewer = h.app_runner("Photo Viewer").expect("photo viewer window");
    assert_eq!(
        viewer.viewing_file(),
        Some("/home/user/photos/oasis_sample.png")
    );

    // Errors are reported in the terminal and open nothing.
    run(&mut h, "open Calculator /home/user/readme.txt");
    let before = h.windows().len();
    assert!(output_contains(&h, "Calculator does not open files"));
    run(&mut h, "open No Such App");
    assert!(output_contains(&h, "unknown app 'No Such App'"));
    assert_eq!(h.windows().len(), before);
}

/// `open <app> <file>` on an app that declines to be replaced (a Text
/// Editor with unsaved changes) reports it in one readable line: the
/// message used to carry a run of source indentation mid-sentence.
#[test]
fn open_command_reports_a_declined_replacement_readably() {
    let mut h = Harness::new("classic");
    h.settle();
    assert!(h.open_app("Text Editor"));
    h.settle();
    h.type_text("unsaved words");
    h.settle();
    run(&mut h, "open Text Editor /home/user/readme.txt");
    assert!(
        output_contains(
            &h,
            "Text Editor is asking for confirmation (e.g. unsaved changes); \
             /home/user/readme.txt was not opened"
        ),
        "{:?}",
        h.state().terminal.output_lines
    );
}

/// `open <app> <file>` resolves a relative file against the terminal's
/// working directory: `open Text Editor notes.txt` used to read
/// "notes.txt" as part of the app title ("unknown app").
#[test]
fn open_command_resolves_relative_files_against_cwd() {
    let mut h = Harness::new("classic");
    h.settle();
    run(&mut h, "cd /home/user/photos");
    run(&mut h, "open Photo Viewer oasis_sample.png");
    let viewer = h.app_runner("Photo Viewer").expect("photo viewer window");
    assert_eq!(
        viewer.viewing_file(),
        Some("/home/user/photos/oasis_sample.png")
    );

    run(&mut h, "cd ..");
    run(&mut h, "open Text Editor readme.txt");
    assert!(
        !output_contains(&h, "unknown app"),
        "{:?}",
        h.state().terminal.output_lines
    );
    assert!(h.find_window("Text Editor").is_some(), "{:?}", h.windows());
    h.render_now();
    assert!(h.text_drawn_contains("readme.txt"), "{:?}", h.frame_text());
}

/// The Network app used to show hard-coded "Not running / Not connected"
/// whatever the shell was doing. It now follows `/system/state/net`.
#[test]
fn network_app_follows_the_listener() {
    let mut h = Harness::new("classic");
    h.settle();
    let port = free_port();
    run(&mut h, &format!("listen {port}"));
    assert!(h.open_app("Network"));
    h.settle();
    h.render_now();
    let running = format!("Running (port {port})");
    assert!(h.text_drawn_contains(&running), "{:?}", h.frame_text());

    // The open window picks up the change within its poll interval.
    run(&mut h, "listen stop");
    assert!(
        output_contains(&h, "listener stopped"),
        "{:?}",
        h.state().terminal.output_lines
    );
    assert!(h.open_app("Network"));
    h.advance(Duration::from_millis(600));
    h.settle();
    h.render_now();
    assert!(
        h.text_drawn_contains("Listener:   Not running"),
        "{:?}",
        h.frame_text()
    );
}

mod plugin {
    use oasis_core::apps::SimpleApp;
    use oasis_core::error::Result;
    use oasis_core::plugin::{AppCategory, Plugin, PluginAppRegistration, PluginHost, PluginInfo};

    pub const TITLE: &str = "Plugin Notes";
    pub const CONTENT: &str = "notes from a plugin";

    struct NotesPlugin;

    impl Plugin for NotesPlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo::new("notes-plugin", "1.0.0")
        }
        fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
            host.register_app(PluginAppRegistration::new(
                TITLE,
                AppCategory::Utility,
                |path, _vfs| Box::new(SimpleApp::new(TITLE, path, vec![CONTENT.to_string()])),
            ))
        }
        fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
        fn shutdown(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
            Ok(())
        }
    }

    pub fn make() -> Box<dyn Plugin> {
        Box::new(NotesPlugin)
    }
}

/// Plugin apps are ordinary registry entries: `open` (and MCP, and
/// `OASIS_APP`) reach them like built-ins, at their `/plugins/` path.
#[test]
fn plugin_apps_open_by_title() {
    use oasis_app::harness::HarnessOptions;
    let mut opts = HarnessOptions::new("classic");
    opts.plugins = vec![plugin::make];
    let mut h = Harness::with_options(opts).unwrap();
    h.settle();
    let reg = h.state().content.apps.get(plugin::TITLE).unwrap();
    assert!(reg.from_plugin && reg.on_dashboard);

    run(&mut h, "open plugin notes");
    let runner = h.app_runner(plugin::TITLE).expect("plugin window");
    assert_eq!(runner.path, "/plugins/plugin-notes");
    h.render_now();
    assert!(
        h.text_drawn_contains(plugin::CONTENT),
        "{:?}",
        h.frame_text()
    );
}

/// A resolution change used to re-resolve the running skin by name, which
/// fails for external skins and dropped an applied theme preview. It now
/// re-applies the running skin itself.
#[test]
fn resolution_change_keeps_the_previewed_theme() {
    let mut h = Harness::new("classic");
    h.settle();
    let mut theme = h.state().skin.theme.clone();
    theme.background = "#123456".to_string();
    h.vfs_mut()
        .write(
            oasis_core::ipc::SKIN_APPLY_THEME_REQUEST_PATH,
            theme.to_toml_string().unwrap().as_bytes(),
        )
        .unwrap();
    h.settle();
    assert_eq!(h.state().skin.theme.background, "#123456");

    h.vfs_mut()
        .write(oasis_core::ipc::RESOLUTION_CHANGE_REQUEST_PATH, b"800x600")
        .unwrap();
    h.settle();
    assert_eq!(h.size(), (800, 600));
    assert_eq!(h.state().skin.manifest.name, "classic");
    assert_eq!(h.state().skin.theme.background, "#123456");
}

/// Undoing back to the opened text left the Text Editor "[Modified]" (and
/// closing it prompted to save); reopening Find appended to the previous
/// query instead of replacing it.
#[test]
fn text_editor_undo_to_saved_is_clean_and_find_replaces_the_query() {
    use oasis_core::input::Modifiers;
    let mut h = Harness::new("classic");
    h.settle();
    run(&mut h, "open Text Editor /home/user/readme.txt");
    h.settle();
    h.type_text("x");
    h.settle();
    h.render_now();
    assert!(h.text_drawn_contains("[Modified]"), "{:?}", h.frame_text());
    h.key_with(Key::Char('z'), Modifiers::CTRL);
    h.settle();
    h.render_now();
    assert!(!h.text_drawn_contains("[Modified]"), "{:?}", h.frame_text());

    h.key_with(Key::Char('f'), Modifiers::CTRL);
    h.type_text("first");
    h.key(Key::Escape);
    h.key_with(Key::Char('f'), Modifiers::CTRL);
    h.settle();
    h.render_now();
    assert!(
        h.text_drawn_contains("Find: [first]"),
        "{:?}",
        h.frame_text()
    );
    h.type_text("second");
    h.settle();
    h.render_now();
    assert!(
        h.text_drawn_contains("Find: second_"),
        "{:?}",
        h.frame_text()
    );
    assert!(
        !h.text_drawn_contains("firstsecond"),
        "{:?}",
        h.frame_text()
    );
}

/// `tv tune` used to answer "Tuning to CH ..." with no TV Guide open, while
/// the shell dropped the request. The shell now publishes the guide's
/// status and the command reports that nothing can tune.
#[test]
fn tv_tune_without_an_open_guide_says_so() {
    let mut h = Harness::new("classic");
    h.settle();
    run(&mut h, "tv tune 2");
    assert!(
        output_contains(&h, "the TV Guide is not open"),
        "{:?}",
        h.state().terminal.output_lines
    );
    assert!(!output_contains(&h, "Tuning to CH"));
    assert!(h.open_app("TV Guide"));
    h.settle();
    let status = h.vfs().read(oasis_core::ipc::TV_STATUS_PATH).unwrap();
    assert_eq!(
        oasis_core::ipc::TvStatus::parse(&String::from_utf8_lossy(&status)),
        Some(oasis_core::ipc::TvStatus::GuideOpen)
    );
}
