#![allow(clippy::unwrap_used)] // Test code -- unwrap is acceptable.
//! Regression scenarios for bugs found by the e2e harness.

use oasis_app::Mode;
use oasis_app::harness::Harness;
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
    use oasis_core::apps::tv_guide::VideoEpisode;
    use oasis_core::apps::tv_guide::catalog::ChannelCatalog;

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
        let track = h.state().tv_audio_track.expect("tuned");

        h.key(Key::Escape);
        h.settle();
        assert!(
            h.find_window("TV Guide").is_some(),
            "first Escape keeps the guide open: {:?}",
            h.windows()
        );
        let g = h.app_runner("TV Guide").unwrap().tv_guide_state().unwrap();
        assert!(g.tuned_channel.is_none(), "first Escape untunes");
        assert!(!h.state().video_player.is_active(), "playback stopped");
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
