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
