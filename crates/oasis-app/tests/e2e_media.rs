#![allow(clippy::unwrap_used)] // Test code -- unwrap is acceptable.
//! End-to-end media scenarios: TV Guide playback volume reaching the
//! audio output, and the boot splash rendering.

use oasis_app::boot_splash::{BootSplash, SplashTheme};
use oasis_app::harness::{Harness, HarnessOptions};
use oasis_app::headless::HeadlessBackend;
use oasis_app::{BootObserver, Mode};
use oasis_core::backend::{SdiCore, SdiTextures};

// ---------------------------------------------------------------------------
// TV Guide volume -> audio output
// ---------------------------------------------------------------------------

#[cfg(feature = "_video")]
mod tv {
    use super::*;
    use oasis_core::apps::tv_guide::catalog::ChannelCatalog;
    use oasis_core::apps::tv_guide::{TvGuideState, VideoEpisode};
    use oasis_core::input::Key;

    fn mock_episodes(channel: u32, count: usize) -> Vec<VideoEpisode> {
        (0..count)
            .map(|i| VideoEpisode {
                item_id: format!("mock-{channel}-{i}"),
                filename: format!("ep{i:02}.mp4"),
                title: format!("Episode {}", i + 1),
                duration_secs: 1800.0,
                width: 640,
                height: 480,
                size_bytes: 50_000_000,
                format: "MPEG4".into(),
                original: None,
            })
            .collect()
    }

    fn guide(h: &mut Harness) -> &mut TvGuideState {
        h.app_runner("TV Guide")
            .expect("TV Guide runner")
            .tv_guide_state()
            .expect("TV Guide state")
    }

    /// Open the TV Guide from the dashboard, give it offline catalogs and
    /// tune the selected channel with Enter.
    fn open_and_tune() -> Harness {
        let mut h = Harness::new("classic");
        h.settle();
        assert!(h.click_app_icon("TV Guide"), "{:?}", h.dashboard_apps());
        h.settle();
        assert!(h.find_window("TV Guide").is_some(), "{:?}", h.windows());

        let g = guide(&mut h);
        for (i, ch) in g.channels.clone().iter().enumerate() {
            let mut catalog = ChannelCatalog::new(ch.number);
            catalog.add_episodes(mock_episodes(ch.number, 4));
            g.catalogs[i] = Some(catalog);
            g.rebuild_cached_schedule(i);
        }
        g.fetch_attempted = true;
        h.app_runner("TV Guide").unwrap().refresh_tv_text();

        h.key(Key::Enter);
        h.run_frames(3);
        assert!(
            guide(&mut h).tuned_channel.is_some(),
            "Enter tunes the channel"
        );
        let st = h.state();
        assert!(
            st.video_player.is_active(),
            "tune started (injected) playback"
        );
        assert!(
            st.tv_audio_track.is_some(),
            "tune opened a streaming audio track"
        );
        assert_eq!(st.mode, Mode::Desktop);
        h
    }

    /// Feed one chunk of full-scale PCM through the video player and return
    /// the samples that reached the audio output.
    fn feed_unit_chunk(h: &mut Harness) -> Vec<f32> {
        let before = h.audio().pcm_chunks.len();
        assert!(h.inject_video_audio(vec![1.0; 960], 2, 48_000));
        h.run_frames(1);
        let log = h.audio();
        assert_eq!(log.pcm_chunks.len(), before + 1, "chunk reached the output");
        let chunk = log.pcm_chunks.last().unwrap();
        assert_eq!((chunk.channels, chunk.sample_rate), (2, 48_000));
        assert_eq!(Some(chunk.track), h.state().tv_audio_track);
        chunk.samples.clone()
    }

    fn assert_gain(samples: &[f32], volume: u8) {
        let want = f32::from(volume) / 100.0;
        assert_eq!(samples.len(), 960);
        for s in samples {
            assert!(
                (s - want).abs() < 1e-4,
                "sample {s} != gain {want} (volume {volume})"
            );
        }
    }

    /// Find the painted volume bar: returns `(x0, width, y)`.
    ///
    /// Located from pixels, the way a user finds it: the "VOL…" label is
    /// drawn in the fill color on the bar's row, left of the bar. The bar
    /// is the first long run of fill-colored pixels right of the label
    /// start (glyph strokes are only a few pixels wide), framed by 1px
    /// border columns and followed by the track color.
    fn locate_volume_bar(h: &Harness) -> (i32, u32, i32) {
        let label = h
            .frame_text_calls()
            .iter()
            .find(|t| t.text.starts_with("VOL"))
            .unwrap_or_else(|| panic!("volume label drawn: {:?}", h.frame_text()))
            .clone();
        let (w, _) = h.size();
        let w = w as i32;
        let y = label.y + i32::from(label.font_size) / 2;
        let fill = [label.color.r, label.color.g, label.color.b];
        let rgb = |x: i32| {
            let p = h.pixel(x as u32, y as u32);
            [p[0], p[1], p[2]]
        };
        let run_end = |from: i32, color: [u8; 3]| {
            let mut x = from;
            while x < w && rgb(x) == color {
                x += 1;
            }
            x
        };
        let mut x = label.x;
        let fill_start = loop {
            assert!(x < w, "no volume bar fill right of the label on row {y}");
            if rgb(x) == fill && run_end(x, fill) - x >= 6 {
                break x;
            }
            x += 1;
        };
        let track_start = run_end(fill_start, fill);
        let right_border = run_end(track_start, rgb(track_start));
        // `fill_start - 1` and `right_border` are the 1px border columns.
        let x0 = fill_start - 1;
        (x0, (right_border - x0 + 1) as u32, y)
    }

    #[test]
    fn tv_guide_volume_slider_scales_audio_reaching_the_output() {
        let mut h = open_and_tune();

        // Default guide volume already applies to the decoded audio.
        let volume = guide(&mut h).volume;
        assert!(volume < 100, "default guide volume below full scale");
        assert_gain(&feed_unit_chunk(&mut h), volume);

        // Click 80% of the way along the painted bar.
        h.render_now();
        let (x0, bw, y) = locate_volume_bar(&h);
        let x = x0 + ((bw - 1) as f32 * 0.8).round() as i32;
        h.click(x, y);
        let volume = guide(&mut h).volume;
        assert!(
            (78..=82).contains(&volume),
            "click at 80% of the bar set volume {volume}"
        );
        assert_gain(&feed_unit_chunk(&mut h), volume);

        // The new level is what the bar shows.
        h.render_now();
        assert!(
            h.text_drawn_contains(&format!("{volume}%")),
            "label shows {volume}%: {:?}",
            h.frame_text()
        );

        // And muting (left edge) silences the output.
        let (x0, _, y) = locate_volume_bar(&h);
        h.click(x0, y);
        assert_eq!(guide(&mut h).volume, 0);
        assert_gain(&feed_unit_chunk(&mut h), 0);
    }

    #[test]
    fn closing_the_tv_guide_stops_playback_and_releases_audio() {
        let mut h = open_and_tune();
        let track = h.state().tv_audio_track.unwrap();
        // Tuning enters kiosk fullscreen; leave it (F11) so the titlebar
        // close button exists, then close the window with the mouse.
        assert!(
            h.find_window("TV Guide").unwrap().fullscreen,
            "tune -> kiosk"
        );
        h.send(&[oasis_core::input::InputEvent::ToggleFullscreen]);
        h.settle();
        assert!(!h.find_window("TV Guide").unwrap().fullscreen);
        assert!(h.state().video_player.is_active(), "still playing windowed");
        assert!(
            h.close_window("TV Guide"),
            "close button: {:?}",
            h.find_window("TV Guide")
        );
        h.settle();
        assert!(h.find_window("TV Guide").is_none());
        assert!(!h.state().video_player.is_active(), "playback stopped");
        assert!(h.state().tv_audio_track.is_none(), "track released");
        assert!(h.audio().tracks_unloaded.contains(&track));
        assert_eq!(h.mode(), Mode::Dashboard);
    }
}

// ---------------------------------------------------------------------------
// Boot splash
// ---------------------------------------------------------------------------

/// Largest per-channel difference between pixel `(x, y)` and its mirror
/// `(w-1-x, y)` over one row.
fn row_mirror_error(px: &[u8], w: u32, y: u32) -> u8 {
    let mut worst = 0u8;
    for x in 0..w / 2 {
        let a = ((y * w + x) * 4) as usize;
        let b = ((y * w + (w - 1 - x)) * 4) as usize;
        for c in 0..3 {
            worst = worst.max(px[a + c].abs_diff(px[b + c]));
        }
    }
    worst
}

/// The splash phase just before the logo appears is left/right mirror
/// symmetric by construction: row-uniform gradients, a row-uniform horizon
/// glow texture and a radially symmetric vignette texture. A texture
/// uploaded or sampled with the wrong row stride smears into diagonal
/// stripes, which breaks the symmetry.
fn assert_splash_symmetric(w: u32, h: u32) {
    let mut be = HeadlessBackend::new(w, h);
    be.init(w, h).unwrap();
    let splash = BootSplash::start_themed(&mut be, w, h, SplashTheme::default()).unwrap();
    // 3.9s: flicker over, horizon glow on, logo not yet (4.0s).
    splash.render_at(&mut be, 3.9).unwrap();
    be.swap_buffers().unwrap();
    let px = be.pixels().to_vec();
    let mut distinct = std::collections::HashSet::new();
    for p in px.chunks_exact(4) {
        distinct.insert([p[0], p[1], p[2]]);
    }
    assert!(distinct.len() > 16, "{w}x{h}: splash painted a picture");
    for y in 0..h {
        let err = row_mirror_error(&px, w, y);
        assert!(
            err <= 2,
            "{w}x{h}: row {y} not mirror-symmetric (max channel diff {err}) -- \
             texture stride artifact?"
        );
    }
    splash.finish(&mut be).unwrap();
}

#[test]
fn boot_splash_frames_have_no_stride_artifacts() {
    // Even + odd half-widths exercise the half-resolution vignette.
    for (w, h) in [
        (1280, 720),
        (1366, 768),
        (1024, 768),
        (800, 600),
        (480, 272),
    ] {
        assert_splash_symmetric(w, h);
    }
}

#[test]
fn boot_splash_renders_every_phase() {
    let (w, h) = (1280, 720);
    let mut be = HeadlessBackend::new(w, h);
    let mut splash = BootSplash::start_themed(&mut be, w, h, SplashTheme::default()).unwrap();
    splash.set_bios_line(0, "E2E BIOS LINE");
    splash.set_status("E2E status");
    for t in [0.1, 0.5, 1.0, 2.0, 3.0, 3.45, 3.7, 4.2, 5.0, 6.0, 6.49] {
        splash.render_at(&mut be, t).unwrap();
        be.swap_buffers().unwrap();
        let px = be.pixels();
        let first = &px[..3];
        assert!(
            px.chunks_exact(4).any(|p| &p[..3] != first),
            "t={t}: splash frame is a single flat color"
        );
        if t > 0.5 && t < 3.4 {
            // BIOS lines are typed out glyph by glyph.
            let typed: String = be
                .frame_text()
                .iter()
                .filter(|d| d.font_size == 22)
                .map(|d| d.text.as_str())
                .collect();
            assert!(
                typed.contains("E2E BIOS LINE"),
                "t={t}: BIOS line painted: {typed}"
            );
        }
    }
    splash.finish(&mut be).unwrap();
}

/// Boot the shell with a splash observer, like `main.rs` does, and check
/// the BIOS lines it reports carry real boot data.
#[test]
fn boot_reports_real_progress_to_the_splash() {
    #[derive(Default)]
    struct Recorder {
        lines: Vec<(usize, String)>,
        waits: Vec<f32>,
        statuses: Vec<String>,
    }
    impl BootObserver<HeadlessBackend> for Recorder {
        fn status(&mut self, text: &str) {
            self.statuses.push(text.to_string());
        }
        fn bios_line(&mut self, idx: usize, text: String) {
            self.lines.push((idx, text));
        }
        fn wait_until(&mut self, backend: &mut HeadlessBackend, secs: f32) {
            // The backend must be usable mid-boot (the splash draws on it).
            backend
                .clear(oasis_core::backend::Color::rgb(1, 2, 3))
                .unwrap();
            backend.swap_buffers().unwrap();
            self.waits.push(secs);
        }
    }
    let mut rec = Recorder::default();
    let mut h = Harness::with_observer(HarnessOptions::new("xp"), &mut rec).unwrap();
    let idx: Vec<usize> = rec.lines.iter().map(|(i, _)| *i).collect();
    assert_eq!(
        idx,
        vec![0, 1, 2, 3, 4, 5, 6],
        "all seven BIOS lines, in order"
    );
    assert!(
        rec.waits.windows(2).all(|p| p[0] < p[1]),
        "waits ascend: {:?}",
        rec.waits
    );
    let line = |i: usize| rec.lines[i].1.clone();
    assert!(line(3).contains("FILES"), "VFS line: {}", line(3));
    assert!(line(4).contains("\"XP\""), "skin line: {}", line(4));
    assert!(
        line(6).contains("BACKEND: Headless"),
        "display line: {}",
        line(6)
    );
    assert_eq!(
        rec.statuses.last().map(String::as_str),
        Some(""),
        "status cleared"
    );
    h.settle();
    assert_eq!(h.mode(), Mode::Dashboard);
}

/// Textures survive a round trip through the headless backend's resize
/// (the Settings resolution path relies on ids staying valid).
#[test]
fn headless_resize_preserves_texture_ids() {
    use oasis_app::ShellBackend;
    let mut be = HeadlessBackend::new(64, 64);
    let a = be.load_texture(2, 2, &[255; 16]).unwrap();
    let b = be.load_texture(1, 1, &[0, 255, 0, 255]).unwrap();
    be.destroy_texture(a).unwrap();
    be.set_window_size(32, 16).unwrap();
    assert_eq!(be.dimensions(), (32, 16));
    be.clear(oasis_core::backend::Color::rgb(0, 0, 0)).unwrap();
    be.blit(b, 0, 0, 4, 4).unwrap();
    assert!(
        be.blit(a, 0, 0, 4, 4).is_err(),
        "destroyed id stays destroyed"
    );
    be.blit_sub(b, 0, 0, 1, 1, 8, 0, 2, 2).unwrap();
    assert_eq!(&be.pixels()[..4], &[0, 255, 0, 255]);
}
