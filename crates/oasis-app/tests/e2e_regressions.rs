#![allow(clippy::unwrap_used)] // Test code -- unwrap is acceptable.
//! Regression scenarios for bugs found by the e2e harness.

use oasis_app::Mode;
use oasis_app::harness::Harness;
use oasis_apps::TvGuideRunnerExt as _;
use oasis_core::input::Key;

/// Escape (Cancel) in a windowed app must reach the app first. The Text
/// Editor answers Cancel with its unsaved-changes prompt; the desktop host
/// used to close the window outright, silently discarding the edits.
#[test]
fn escape_in_modified_text_editor_prompts_instead_of_discarding() {
    let mut h = Harness::new("classic");
    h.settle();
    assert!(h.click_app_icon("Text Editor"), "{:?}", h.dashboard_apps());
    h.settle();
    assert!(h.find_window("Text Editor").is_some());
    h.type_text("unsaved words");
    h.settle();
    h.render_now();
    assert!(
        h.text_drawn_contains("unsaved words"),
        "typing reached the editor: {:?}",
        h.frame_text()
    );

    h.key(Key::Escape);
    h.settle();
    assert!(
        h.find_window("Text Editor").is_some(),
        "Escape with unsaved changes must not close the editor"
    );
    assert_eq!(h.mode(), Mode::Desktop);
    h.render_now();
    assert!(
        h.text_drawn_contains("unsaved words"),
        "the text is still there: {:?}",
        h.frame_text()
    );
}

/// Unmodified documents still close on Escape (the app answers Exit).
#[test]
fn escape_closes_an_unmodified_app_window() {
    let mut h = Harness::new("classic");
    h.settle();
    assert!(h.click_app_icon("Text Editor"));
    h.settle();
    h.key(Key::Escape);
    h.settle();
    assert!(h.find_window("Text Editor").is_none(), "{:?}", h.windows());
    assert_eq!(h.mode(), Mode::Dashboard);
}

/// The titlebar close button on a Text Editor with unsaved changes must
/// raise the unsaved-changes prompt like Escape does. The window manager
/// used to drop the window (and the edits) on the spot.
#[test]
fn close_button_on_modified_text_editor_prompts_instead_of_discarding() {
    let mut h = Harness::new("classic");
    h.settle();
    assert!(h.click_app_icon("Text Editor"));
    h.settle();
    h.type_text("keep me");
    h.settle();
    assert!(h.close_window("Text Editor"));
    h.settle();
    assert!(
        h.find_window("Text Editor").is_some(),
        "close button with unsaved changes must not discard the edits"
    );
    h.render_now();
    assert!(h.text_drawn_contains("keep me"), "{:?}", h.frame_text());

    // "Discard" in the prompt closes it.
    h.key(Key::Char('d'));
    h.settle();
    assert!(h.find_window("Text Editor").is_none(), "{:?}", h.windows());
    assert_eq!(h.mode(), Mode::Dashboard);
}

/// The unsaved-changes prompt on the editor's status strip used to be
/// drawn underneath the right-aligned "Ln / Col" readout at 480x272. The
/// two texts must not overlap on screen.
#[test]
fn unsaved_changes_prompt_does_not_overlap_line_column_readout() {
    let mut opts = oasis_app::harness::HarnessOptions::new("classic");
    opts.resolution = Some((480, 272));
    let mut h = Harness::with_options(opts).unwrap();
    h.settle();
    assert!(h.click_app_icon("Text Editor"));
    h.settle();
    h.type_text("draft");
    h.settle();
    assert!(h.close_window("Text Editor"));
    h.settle();
    h.render_now();
    let texts = h.frame_text_calls().to_vec();
    let prompt = texts
        .iter()
        .find(|t| t.text.starts_with("Unsaved changes"))
        .unwrap_or_else(|| panic!("prompt drawn: {:?}", h.frame_text()));
    let width = |t: &oasis_app::headless::DrawnText| {
        oasis_core::backend::bitmap_measure_text(&t.text, t.font_size) as i32
    };
    let line_h = i32::from(prompt.font_size);
    for pos in texts.iter().filter(|t| t.text.starts_with("Ln ")) {
        let overlaps = (pos.y - prompt.y).abs() < line_h
            && pos.x < prompt.x + width(prompt)
            && prompt.x < pos.x + width(pos);
        assert!(!overlaps, "{pos:?} overlaps {prompt:?}");
    }
}

/// `wm close <id>` (the terminal command's VFS request) asks a modified
/// Text Editor first, like the close button; `wm close --force <id>`
/// closes it without the prompt.
#[test]
fn wm_close_force_skips_the_unsaved_changes_prompt() {
    use oasis_core::vfs::Vfs;
    let mut h = Harness::new("classic");
    h.settle();
    assert!(h.click_app_icon("Text Editor"));
    h.settle();
    let id = h.find_window("Text Editor").expect("editor window").id;
    h.type_text("unsaved words");
    h.settle();

    h.vfs_mut()
        .write("/var/wm/request", format!("close {id}").as_bytes())
        .unwrap();
    h.settle();
    assert!(
        h.find_window("Text Editor").is_some(),
        "plain wm close must prompt, not discard"
    );

    h.vfs_mut()
        .write("/var/wm/request", format!("close --force {id}").as_bytes())
        .unwrap();
    h.settle();
    assert!(h.find_window("Text Editor").is_none(), "{:?}", h.windows());
}

/// Unmodified apps still close on the first click of the close button.
#[test]
fn close_button_closes_unmodified_apps_immediately() {
    let mut h = Harness::new("classic");
    h.settle();
    for app in ["Text Editor", "Calculator", "Paint", "File Manager"] {
        assert!(h.open_app(app));
        h.settle();
        assert!(h.close_window(app), "{app}");
        h.settle();
        assert!(h.find_window(app).is_none(), "{app}: {:?}", h.windows());
    }
    assert_eq!(h.mode(), Mode::Dashboard);
}

#[cfg(feature = "_video")]
mod tv {
    use super::*;
    use oasis_apps::tv_guide::VideoEpisode;
    use oasis_apps::tv_guide::catalog::ChannelCatalog;

    /// Escape while watching returns to the guide (the TV Guide's Cancel
    /// = untune); a second Escape closes it. The host used to close the
    /// whole app on the first press.
    #[test]
    fn escape_while_watching_tv_returns_to_the_guide() {
        let mut h = Harness::new("classic");
        h.settle();
        assert!(h.click_app_icon("TV Guide"));
        h.settle();
        {
            let g = h.app_runner("TV Guide").unwrap().tv_guide_state().unwrap();
            for (i, ch) in g.channels.clone().iter().enumerate() {
                let mut c = ChannelCatalog::new(ch.number);
                c.add_episodes(
                    (0..3)
                        .map(|k| VideoEpisode {
                            item_id: format!("m{}-{k}", ch.number),
                            filename: format!("e{k}.mp4"),
                            title: format!("Episode {k}"),
                            duration_secs: 1800.0,
                            width: 640,
                            height: 480,
                            size_bytes: 1,
                            format: "MPEG4".into(),
                            original: None,
                        })
                        .collect(),
                );
                g.catalogs[i] = Some(c);
                g.rebuild_cached_schedule(i);
            }
            g.fetch_attempted = true;
        }
        h.key(Key::Enter);
        h.run_frames(3);
        let track = h.state().tv.audio_track.expect("tuned");

        h.key(Key::Escape);
        h.settle();
        assert!(
            h.find_window("TV Guide").is_some(),
            "first Escape keeps the guide open: {:?}",
            h.windows()
        );
        let g = h.app_runner("TV Guide").unwrap().tv_guide_state().unwrap();
        assert!(g.tuned_channel.is_none(), "first Escape untunes");
        assert!(!h.state().tv.video_player.is_active(), "playback stopped");
        assert!(h.audio().tracks_unloaded.contains(&track), "audio released");

        h.key(Key::Escape);
        h.settle();
        assert!(h.find_window("TV Guide").is_none(), "second Escape closes");
        assert_eq!(h.mode(), Mode::Dashboard);
    }
}

/// Leaving the fullscreen terminal with windows open must return to the
/// desktop. On skins without a window manager F1 opens the fullscreen
/// terminal even over windows, and Escape used to land on the dashboard,
/// which draws the windows' chrome but routes no input to them.
#[test]
fn leaving_the_terminal_with_windows_open_returns_to_the_desktop() {
    let mut h = Harness::new("vaporwave");
    h.settle();
    assert!(!h.state().skin.features.window_manager);
    assert!(h.open_app("Calculator"));
    h.settle();
    assert_eq!(h.mode(), Mode::Desktop);
    h.key(Key::F(1));
    h.settle();
    assert_eq!(h.mode(), Mode::Terminal);
    h.key(Key::Escape);
    h.settle();
    assert_eq!(h.mode(), Mode::Desktop, "back to the open windows");
    // The window is usable again: Escape reaches and closes it.
    h.key(Key::Escape);
    h.settle();
    assert!(h.find_window("Calculator").is_none(), "{:?}", h.windows());
    assert_eq!(h.mode(), Mode::Dashboard);
}

/// A click on the bare desktop unfocuses every window. Escape used to
/// switch to the dashboard mode anyway, stranding the still-visible
/// windows (painted, but dead to input). It now focuses the top window.
#[test]
fn escape_with_nothing_focused_keeps_open_windows_usable() {
    let mut h = Harness::new("classic");
    h.settle();
    assert!(h.open_app("Calculator"));
    h.settle();
    let (w, hh) = h.size();
    h.click(w as i32 - 20, hh as i32 / 2); // bare desktop, right edge
    h.settle();
    assert_eq!(
        h.state().wm.active_window(),
        None,
        "desktop click unfocuses"
    );
    h.key(Key::Escape);
    h.settle();
    assert_eq!(h.mode(), Mode::Desktop);
    assert_eq!(h.state().wm.active_window(), Some("calculator"));
    h.key(Key::Escape);
    h.settle();
    assert!(h.find_window("Calculator").is_none());
    assert_eq!(h.mode(), Mode::Dashboard);
}

/// "Save as custom skin" in a harness session used to write
/// `skins/<name>/` into the working directory (the repository). Hermetic
/// sessions now save under a temp dir, and the saved skin is applied.
#[test]
fn custom_skin_save_stays_out_of_the_working_directory() {
    use oasis_core::vfs::Vfs;
    let mut h = Harness::new("classic");
    h.settle();
    let name = "e2e-hermetic-custom";
    let theme = h.state().skin.theme.to_toml_string().unwrap();
    h.vfs_mut()
        .write(
            oasis_apps::settings::SKIN_SAVE_CUSTOM_REQUEST_PATH,
            format!("{name}\n{theme}").as_bytes(),
        )
        .unwrap();
    h.settle();
    assert_eq!(
        h.state().skin.manifest.name,
        name,
        "{:?}",
        h.state().terminal.output_lines
    );
    let root = h.state().custom_skin_root.clone();
    assert!(root.starts_with(std::env::temp_dir()), "{root:?}");
    assert!(root.join(name).join("skin.toml").is_file());
    assert!(!std::path::Path::new("skins").join(name).exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// Saving a custom skin under a built-in's name (`classic`) overwrote the
/// built-in's source directory when run from the repository, and the copy
/// could never be loaded again (the built-in wins name resolution). Such
/// saves are refused with a message.
#[test]
fn custom_skin_save_refuses_builtin_names() {
    use oasis_core::vfs::Vfs;
    let mut h = Harness::new("xp");
    h.settle();
    let theme = h.state().skin.theme.to_toml_string().unwrap();
    h.vfs_mut()
        .write(
            oasis_apps::settings::SKIN_SAVE_CUSTOM_REQUEST_PATH,
            format!("classic\n{theme}").as_bytes(),
        )
        .unwrap();
    h.settle();
    assert_eq!(h.state().skin.manifest.name, "xp", "no swap");
    let root = h.state().custom_skin_root.clone();
    assert!(!root.join("classic").exists());
    assert!(
        h.state()
            .terminal
            .output_lines
            .iter()
            .any(|l| l.contains("'classic' is a built-in skin")),
        "{:?}",
        h.state().terminal.output_lines
    );
}

// ---------------------------------------------------------------------------
// Programmatic close honours the app's close prompt
// ---------------------------------------------------------------------------

/// Type `command` into the windowed terminal (opened or focused first)
/// and run it.
fn run_in_windowed_terminal(h: &mut Harness, command: &str) {
    assert!(h.open_app("Terminal"));
    h.settle();
    assert_eq!(h.active_window().as_deref(), Some("terminal"));
    h.type_text(command);
    h.key(Key::Enter);
    h.settle();
}

/// `wm close <id>` on a Text Editor with unsaved changes used to drop the
/// window and the edits without asking. It now behaves like the titlebar
/// close button: the editor raises its prompt, the shell reports that the
/// window did not close, and the edits survive.
#[test]
fn wm_close_on_modified_text_editor_raises_the_prompt() {
    let mut h = Harness::new("classic");
    h.settle();
    assert!(h.open_app("Text Editor"));
    h.settle();
    h.type_text("precious draft");
    h.settle();

    run_in_windowed_terminal(&mut h, "wm close text_editor");
    assert!(
        h.find_window("Text Editor").is_some(),
        "wm close must not discard unsaved edits: {:?}",
        h.windows()
    );
    assert!(
        h.state()
            .terminal
            .output_lines
            .iter()
            .any(|l| l.contains("text_editor did not close")),
        "{:#?}",
        h.state().terminal.output_lines
    );
    // The editor is focused, showing its prompt; "Discard" closes it.
    assert_eq!(h.active_window().as_deref(), Some("text_editor"));
    h.render_now();
    assert!(
        h.text_drawn_contains("precious draft"),
        "{:?}",
        h.frame_text()
    );
    h.key(Key::Char('d'));
    h.settle();
    assert!(h.find_window("Text Editor").is_none(), "{:?}", h.windows());
    assert!(h.app_runner("Text Editor").is_none());
}

/// `wm close` on an app without unsaved state closes it and drops its
/// runner, like the titlebar button.
#[test]
fn wm_close_closes_an_unmodified_app() {
    let mut h = Harness::new("classic");
    h.settle();
    assert!(h.open_app("Calculator"));
    h.settle();
    run_in_windowed_terminal(&mut h, "wm close calculator");
    assert!(h.find_window("Calculator").is_none(), "{:?}", h.windows());
    assert!(h.app_runner("Calculator").is_none(), "runner leaked");
    // The terminal window is still open, so the desktop stays.
    assert_eq!(h.mode(), Mode::Desktop);
}

// ---------------------------------------------------------------------------
// Plugin apps survive dashboard rebuilds
// ---------------------------------------------------------------------------

mod plugin_tile {
    use oasis_core::apps::simple_app::SimpleApp;
    use oasis_core::error::Result;
    use oasis_core::plugin::{AppCategory, Plugin, PluginAppRegistration, PluginHost, PluginInfo};

    pub const TITLE: &str = "Plugin Tile";
    pub const CONTENT: &str = "hello from a plugin";

    /// A plugin that registers one dashboard app.
    struct TilePlugin;

    impl Plugin for TilePlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo::new("tile-plugin", "1.0.0")
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
        Box::new(TilePlugin)
    }
}

/// The plugin app has a dashboard tile, and the tile opens a window
/// showing the plugin's content.
fn assert_plugin_app_launches(h: &mut Harness, what: &str) {
    let has_tile = h
        .state()
        .ui
        .dashboard
        .apps
        .iter()
        .any(|a| a.title == plugin_tile::TITLE);
    assert!(has_tile, "{what}: tile missing");
    assert!(h.open_app(plugin_tile::TITLE), "{what}");
    h.settle();
    h.render_now();
    assert!(
        h.text_drawn_contains(plugin_tile::CONTENT),
        "{what}: plugin app not drawn: {:?}",
        h.frame_text()
    );
    assert!(h.close_window(plugin_tile::TITLE), "{what}");
    h.settle();
    assert!(h.find_window(plugin_tile::TITLE).is_none(), "{what}");
}

/// Plugin-registered apps used to vanish from the dashboard after a skin
/// switch or a resolution change: only the boot path merged them in.
#[test]
fn plugin_app_survives_skin_switch_and_resolution_change() {
    use oasis_app::harness::HarnessOptions;
    use oasis_core::vfs::Vfs;

    let mut opts = HarnessOptions::new("classic");
    opts.plugins = vec![plugin_tile::make];
    let mut h = Harness::with_options(opts).unwrap();
    h.settle();
    assert_plugin_app_launches(&mut h, "boot");

    h.vfs_mut()
        .write(oasis_apps::settings::SKIN_CHANGE_REQUEST_PATH, b"xp")
        .unwrap();
    h.settle();
    assert_eq!(h.state().skin.manifest.name, "xp");
    assert_plugin_app_launches(&mut h, "after skin switch");

    h.vfs_mut()
        .write(
            oasis_apps::settings::RESOLUTION_CHANGE_REQUEST_PATH,
            b"800x600",
        )
        .unwrap();
    h.settle();
    assert_eq!(h.size(), (800, 600));
    assert_plugin_app_launches(&mut h, "after resolution change");
}

// ---------------------------------------------------------------------------
// IPC request files queue instead of overwriting
// ---------------------------------------------------------------------------

/// `open Calculator; open Paint` used to open only Paint: each `open`
/// overwrote the single-request `/var/wm/request` before the shell polled
/// it. Requests now queue one per line and the shell applies them all.
#[test]
fn one_command_line_opens_two_apps() {
    let mut h = Harness::new("classic");
    h.settle();
    h.terminal("open Calculator; open Paint");
    h.settle();
    assert!(h.find_window("Calculator").is_some(), "{:?}", h.windows());
    assert!(h.find_window("Paint").is_some(), "{:?}", h.windows());
    assert_eq!(h.mode(), Mode::Desktop);

    // Same for `wm`: both closes apply.
    run_in_windowed_terminal(&mut h, "wm close calculator; wm close paint");
    assert!(h.find_window("Calculator").is_none(), "{:?}", h.windows());
    assert!(h.find_window("Paint").is_none(), "{:?}", h.windows());
}

/// A music request queued earlier in the frame (the `music` command)
/// used to be overwritten when the shell wrote an app's request (Music
/// Player `play_file`) to the same line-queued file.
#[test]
fn app_music_request_keeps_requests_queued_in_the_same_frame() {
    use oasis_core::vfs::Vfs;
    const TRACK: &str = "/home/user/music/ambient_dawn.mp3";
    let mut h = Harness::new("classic");
    h.settle();
    h.vfs_mut()
        .write(oasis_audio::AUDIO_REQUEST_PATH, b"vol 30")
        .unwrap();
    // Opening the track queues `play_file` on the same frame.
    h.open_app_with_file("Music Player", TRACK);
    h.settle();
    assert!(h.state().media_track.is_some(), "play_file applied");
    assert_eq!(h.audio().volume, 30, "the earlier `vol 30` survived");
}

/// An app's last requests before it closes (picked a skin, then Escape,
/// both within one frame) used to be dropped with its runner: the
/// shell only drained the requests of apps still open.
#[test]
fn requests_queued_right_before_a_close_are_carried_out() {
    use oasis_core::input::{Button, InputEvent};
    let mut h = Harness::new("classic");
    h.settle();
    assert!(h.open_app("Settings"));
    h.settle();
    let before = h.state().skin.manifest.name.clone();
    h.step(&[
        InputEvent::ButtonPress(Button::Down),
        InputEvent::ButtonPress(Button::Confirm),
        InputEvent::ButtonPress(Button::Cancel),
    ]);
    h.settle();
    assert!(h.find_window("Settings").is_none(), "{:?}", h.windows());
    assert_ne!(
        h.state().skin.manifest.name,
        before,
        "the skin picked just before closing Settings was applied"
    );
}

// ---------------------------------------------------------------------------
// Default window geometry
// ---------------------------------------------------------------------------

/// Default-size windows (380x220 content on small screens) cascaded down
/// past the bottom edge at 480x272: even the first one lost a few rows,
/// later ones most of their content. Every newly opened window now fits
/// the screen, on every skin's chrome and at the larger resolutions.
#[test]
fn default_windows_fit_the_screen() {
    use oasis_app::harness::HarnessOptions;

    let cases = [
        ("classic", Some((480, 272))),
        ("xp", Some((480, 272))),
        ("gnome", Some((480, 272))),
        ("win95", Some((480, 272))),
        ("macos", Some((800, 600))),
        ("xp", None),
        ("classic", None),
    ];
    for (skin, res) in cases {
        let mut opts = HarnessOptions::new(skin);
        opts.resolution = res;
        let mut h = Harness::with_options(opts).unwrap();
        h.settle();
        let (sw, sh) = h.size();
        for app in [
            "Calculator",
            "Paint",
            "Text Editor",
            "File Manager",
            "Settings",
            "Games",
        ] {
            assert!(h.open_app(app), "{skin}: {app}");
        }
        h.settle();
        assert_eq!(h.windows().len(), 6, "{skin}");
        for w in h.windows() {
            let (x, y, fw, fh) = w.frame;
            assert!(
                x >= 0 && y >= 0 && x + fw as i32 <= sw as i32 && y + fh as i32 <= sh as i32,
                "{skin} {sw}x{sh}: {} frame {:?} off screen",
                w.id,
                w.frame
            );
            let (_, _, cw, ch) = w.content;
            assert!(
                cw >= 380 && ch >= 220,
                "{skin}: {} content {:?}",
                w.id,
                w.content
            );
        }
        // The topmost window's close button is on screen and works.
        assert!(h.close_window("Games"), "{skin}");
        h.settle();
        assert!(
            h.find_window("Games").is_none(),
            "{skin}: {:?}",
            h.windows()
        );
    }
}
