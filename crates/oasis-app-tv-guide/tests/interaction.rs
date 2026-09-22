//! Interaction sequences through the TV Guide's public `App` / input API:
//! grid navigation, tuning and channel changes, the volume bar, catalog
//! errors and retry, empty channels, and schedules that cross midnight or
//! have fractional episode durations.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use oasis_app_core::{App, AppAction};
use oasis_app_tv_guide::schedule::{
    CachedSchedule, ScheduleSlot, format_time, schedule_at, schedule_range,
};
use oasis_app_tv_guide::{ChannelCatalog, TV_REQUEST_PATH, TvGuideApp, VideoEpisode};
use oasis_skin::active_theme::ActiveTheme;
use oasis_types::input::Button;
use oasis_vfs::{MemoryVfs, Vfs};

const CW: u32 = 800;
const CH: u32 = 600;

fn episode(item: &str, title: &str, duration_secs: f64) -> VideoEpisode {
    VideoEpisode {
        item_id: item.into(),
        filename: format!("{title}.mp4"),
        title: title.into(),
        duration_secs,
        width: 640,
        height: 480,
        size_bytes: 1000,
        format: "h.264".into(),
        original: None,
    }
}

fn catalog(channel_number: u32, episodes: Vec<VideoEpisode>) -> ChannelCatalog {
    let mut c = ChannelCatalog::new(channel_number);
    c.add_episodes(episodes);
    c
}

/// A guide with the default 5 channels, catalogs loaded for the ones in
/// `loaded` (by index), each with a single long episode.
fn guide_with(loaded: &[usize]) -> (TvGuideApp, MemoryVfs) {
    let vfs = MemoryVfs::new();
    let mut app = TvGuideApp::new("/apps/TV Guide", &vfs, &ActiveTheme::default());
    app.guide.fetch_attempted = true;
    for &i in loaded {
        let number = app.guide.channels[i].number;
        app.guide.catalogs[i] = Some(catalog(
            number,
            vec![episode(
                &format!("item-{i}"),
                &format!("Show {i}"),
                3600.0 * 24.0,
            )],
        ));
        app.guide.rebuild_cached_schedule(i);
    }
    app.refresh_text();
    (app, vfs)
}

fn press(app: &mut TvGuideApp, vfs: &dyn Vfs, b: Button) -> AppAction {
    app.handle_input(&b, vfs)
}

/// Content-local y of the middle of visible grid row `row` (fullscreen
/// layout, mirrors `TvGuideState::handle_click`).
fn row_y(rows: usize, row: usize) -> i32 {
    let header_h = (CH * 20 / 100).max(60);
    let time_h = (CH * 4 / 100).max(20);
    let footer_h = (CH * 5 / 100).max(18);
    let grid_h = CH - header_h - time_h - footer_h;
    let row_h = (grid_h / rows.clamp(1, 5) as u32).max(20);
    (header_h + time_h + row_h * row as u32 + row_h / 2) as i32
}

/// The fullscreen volume bar, `(x, y, w)` (mirrors `volume_bar_rect`).
fn volume_bar(expanded: bool) -> (i32, i32, u32) {
    let w = (CW / 3).clamp(80, 220);
    let x = CW as i32 - w as i32 - 12;
    let y = if expanded {
        let overlay_y = CH as i32 - 20;
        overlay_y + (20 - 12) / 2
    } else {
        let footer_h = (CH * 5 / 100).max(14);
        CH as i32 - footer_h as i32 + (footer_h as i32 - 12) / 2
    };
    (x, y, w)
}

fn tune_request(app: &mut TvGuideApp) -> Option<(String, String)> {
    app.take_pending_request()
}

// ---------------------------------------------------------------------------
// Navigation and tuning
// ---------------------------------------------------------------------------

#[test]
fn navigate_tune_change_channel_and_back_out() {
    let (mut app, vfs) = guide_with(&[0, 1, 2, 3, 4]);

    // Walk down to channel index 2 and tune.
    press(&mut app, &vfs, Button::Down);
    press(&mut app, &vfs, Button::Down);
    assert_eq!(app.guide.selected_channel, 2);
    assert_eq!(
        press(&mut app, &vfs, Button::Confirm),
        AppAction::RequestFullscreen
    );
    assert_eq!(app.guide.tuned_channel, Some(2));
    let (path, data) = tune_request(&mut app).expect("tune request");
    assert_eq!(path, TV_REQUEST_PATH);
    assert!(data.starts_with("tune_url "), "{data}");
    assert!(data.contains("item-2"), "{data}");
    assert!(
        app.lines().iter().any(|l| l.starts_with(" > CH")),
        "tuned marker"
    );

    // Confirm again on the playing channel: no duplicate request.
    assert_eq!(press(&mut app, &vfs, Button::Confirm), AppAction::None);
    assert!(tune_request(&mut app).is_none());

    // Channel change while watching.
    press(&mut app, &vfs, Button::Up);
    assert_eq!(
        press(&mut app, &vfs, Button::Confirm),
        AppAction::RequestFullscreen
    );
    assert_eq!(app.guide.tuned_channel, Some(1));
    assert!(tune_request(&mut app).unwrap().1.contains("item-1"));

    // Cancel untunes first, then exits.
    app.guide.preview_texture = Some(oasis_types::backend::TextureId(7));
    assert_eq!(press(&mut app, &vfs, Button::Cancel), AppAction::None);
    assert_eq!(app.guide.tuned_channel, None);
    assert!(app.guide.preview_texture.is_none());
    assert_eq!(press(&mut app, &vfs, Button::Cancel), AppAction::Exit);
}

#[test]
fn channel_selection_stops_at_both_ends() {
    // Up at the first channel / Down at the last stays put (no wraparound:
    // the guide has no channel-wrap behaviour).
    let (mut app, vfs) = guide_with(&[]);
    press(&mut app, &vfs, Button::Up);
    assert_eq!(app.guide.selected_channel, 0);
    for _ in 0..10 {
        press(&mut app, &vfs, Button::Down);
    }
    assert_eq!(app.guide.selected_channel, app.guide.channels.len() - 1);
}

#[test]
fn time_window_scrolls_left_and_right() {
    let (mut app, vfs) = guide_with(&[0]);
    for _ in 0..3 {
        press(&mut app, &vfs, Button::Right);
    }
    assert_eq!(app.guide.time_offset, 3);
    for _ in 0..5 {
        press(&mut app, &vfs, Button::Left);
    }
    assert_eq!(app.guide.time_offset, -2);
}

#[test]
fn many_channels_page_with_selection() {
    let mut toml = String::new();
    for i in 0..12 {
        toml.push_str(&format!(
            "[[channel]]\nnumber = {}\ncall_sign = \"C{i}\"\nname = \"Ch {i}\"\n\
             genre = \"t\"\n[[channel.source]]\nitem_id = \"t-{i}\"\n\n",
            i + 1
        ));
    }
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/etc").unwrap();
    vfs.mkdir("/etc/tv").unwrap();
    vfs.write("/etc/tv/channels.toml", toml.as_bytes()).unwrap();
    let mut app = TvGuideApp::new("/apps/TV Guide", &vfs, &ActiveTheme::default());
    assert_eq!(app.guide.channels.len(), 12);
    for _ in 0..11 {
        press(&mut app, &vfs, Button::Down);
    }
    assert_eq!(app.guide.selected_channel, 11);
    assert_eq!(app.guide.current_page(), app.guide.total_pages());
    // Clicking the first visible row on the last page selects channel 7.
    let first_visible = app.guide.scroll_offset;
    app.handle_click(100, row_y(12, 0), CW, CH, true);
    assert_eq!(app.guide.selected_channel, first_visible);
}

#[test]
fn click_selects_then_second_click_tunes() {
    let (mut app, _vfs) = guide_with(&[0, 1, 2, 3, 4]);
    assert_eq!(
        app.handle_click(100, row_y(5, 3), CW, CH, true),
        AppAction::None
    );
    assert_eq!(app.guide.selected_channel, 3);
    assert!(app.guide.tuned_channel.is_none());
    assert_eq!(
        app.handle_click(100, row_y(5, 3), CW, CH, true),
        AppAction::RequestFullscreen
    );
    assert_eq!(app.guide.tuned_channel, Some(3));
    assert!(tune_request(&mut app).unwrap().1.contains("item-3"));
}

// ---------------------------------------------------------------------------
// Volume
// ---------------------------------------------------------------------------

#[test]
fn volume_bar_click_sets_volume_while_tuned() {
    let (mut app, vfs) = guide_with(&[0]);
    assert_eq!(app.guide.volume, 25, "default guide volume");
    press(&mut app, &vfs, Button::Confirm);
    let _ = tune_request(&mut app);

    for expanded in [false, true] {
        app.guide.video_expanded = expanded;
        let (x, y, w) = volume_bar(expanded);
        // Left edge, middle, right edge.
        for (px, want) in [
            (x, 0u8),
            (x + (w as i32 - 1) / 2, 50),
            (x + w as i32 - 1, 100),
        ] {
            app.guide.volume_changed = false;
            assert_eq!(app.handle_click(px, y + 6, CW, CH, true), AppAction::None);
            assert!(app.guide.volume_changed);
            assert!(
                app.guide.volume.abs_diff(want) <= 1,
                "expanded={expanded}: click at {px} -> {} (want ~{want})",
                app.guide.volume
            );
            // A volume click never retunes or toggles the view.
            assert!(tune_request(&mut app).is_none());
            assert_eq!(app.guide.video_expanded, expanded);
        }
    }
}

#[test]
fn volume_bar_is_inert_when_not_tuned() {
    let (mut app, _vfs) = guide_with(&[0]);
    let (x, y, w) = volume_bar(false);
    app.handle_click(x + w as i32 / 2, y + 6, CW, CH, true);
    assert_eq!(app.guide.volume, 25);
    assert!(!app.guide.volume_changed);
}

// ---------------------------------------------------------------------------
// Catalog errors and empty channels
// ---------------------------------------------------------------------------

#[test]
fn catalog_error_is_shown_and_select_retries() {
    let (mut app, vfs) = guide_with(&[]);
    app.guide.fetch_in_progress = false;
    app.guide.fetch_error = Some("HTTP 503".into());
    app.refresh_text();
    assert!(app.lines().iter().any(|l| l.contains("Error: HTTP 503")));
    // Nothing to tune while catalogs are missing.
    assert_eq!(press(&mut app, &vfs, Button::Confirm), AppAction::None);
    assert!(tune_request(&mut app).is_none());

    press(&mut app, &vfs, Button::Select);
    assert!(!app.guide.fetch_attempted, "retry re-arms the fetch");
    assert!(app.guide.fetch_error.is_none());
    assert!(app.lines().iter().any(|l| l.contains("Loading")));
}

#[test]
fn partial_catalog_load_tunes_only_loaded_channels() {
    let (mut app, vfs) = guide_with(&[1]);
    // Channel 0 has no catalog.
    assert_eq!(press(&mut app, &vfs, Button::Confirm), AppAction::None);
    assert!(app.guide.tuned_channel.is_none());
    press(&mut app, &vfs, Button::Down);
    assert_eq!(
        press(&mut app, &vfs, Button::Confirm),
        AppAction::RequestFullscreen
    );
    assert_eq!(app.guide.tuned_channel, Some(1));
}

#[test]
fn channel_with_empty_catalog_cannot_be_tuned() {
    let (mut app, vfs) = guide_with(&[]);
    let number = app.guide.channels[0].number;
    app.guide.catalogs[0] = Some(ChannelCatalog::new(number));
    app.guide.rebuild_cached_schedule(0);
    app.refresh_text();
    assert!(app.lines().iter().any(|l| l.contains("(empty schedule)")));
    assert_eq!(press(&mut app, &vfs, Button::Confirm), AppAction::None);
    assert!(app.guide.tuned_channel.is_none());
    assert!(tune_request(&mut app).is_none());
}

#[test]
fn guide_without_channels_handles_all_input() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/etc").unwrap();
    vfs.mkdir("/etc/tv").unwrap();
    vfs.write("/etc/tv/channels.toml", b"channel = []").unwrap();
    let mut app = TvGuideApp::new("/apps/TV Guide", &vfs, &ActiveTheme::default());
    assert!(app.guide.channels.is_empty());
    for b in [
        Button::Up,
        Button::Down,
        Button::Left,
        Button::Right,
        Button::Confirm,
        Button::Select,
    ] {
        assert_eq!(press(&mut app, &vfs, b), AppAction::None);
    }
    assert_eq!(app.handle_click(100, 300, CW, CH, true), AppAction::None);
    assert!(app.lines().iter().any(|l| l.contains("No channels")));
    assert_eq!(press(&mut app, &vfs, Button::Cancel), AppAction::Exit);
}

#[test]
fn invalid_channel_config_falls_back_to_defaults() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/etc").unwrap();
    vfs.mkdir("/etc/tv").unwrap();
    vfs.write("/etc/tv/channels.toml", b"this is [not toml")
        .unwrap();
    let app = TvGuideApp::new("/apps/TV Guide", &vfs, &ActiveTheme::default());
    assert_eq!(app.guide.channels.len(), 5);
}

// ---------------------------------------------------------------------------
// Schedules
// ---------------------------------------------------------------------------

/// 2026-03-14 23:50:00 UTC.
const BEFORE_MIDNIGHT: u64 = 1_773_532_200;

#[test]
fn schedule_is_continuous_across_midnight() {
    let cat = catalog(
        5,
        (0..7)
            .map(|i| episode("m", &format!("Ep {i}"), 600.0 + 60.0 * f64::from(i)))
            .collect(),
    );
    // Walk second by second through midnight: every instant is covered by
    // exactly one slot, and slots are contiguous.
    let mut prev = schedule_at(&cat, BEFORE_MIDNIGHT).unwrap();
    for t in BEFORE_MIDNIGHT + 1..BEFORE_MIDNIGHT + 1800 {
        let slot = schedule_at(&cat, t).unwrap();
        assert_eq!(slot.elapsed_secs, t - slot.start_time);
        assert_eq!(
            slot.elapsed_secs + slot.remaining_secs,
            slot.episode.duration_secs as u64
        );
        if slot.start_time != prev.start_time {
            assert_eq!(
                slot.start_time,
                prev.start_time + prev.episode.duration_secs as u64,
                "gap/overlap at {t}"
            );
        }
        prev = slot;
    }
    assert_schedule_tiles(
        |t| schedule_at(&cat, t),
        BEFORE_MIDNIGHT,
        BEFORE_MIDNIGHT + 1800,
    );
    // The grid range across midnight tiles the window without gaps.
    let slots = schedule_range(&cat, BEFORE_MIDNIGHT, BEFORE_MIDNIGHT + 3600);
    for w in slots.windows(2) {
        assert_eq!(
            w[1].start_time,
            w[0].start_time + w[0].episode.duration_secs as u64
        );
    }
    assert!(slots.last().unwrap().start_time + slots.last().unwrap().remaining_secs > 0);
    // Midnight renders as 12:00 AM (times are UTC).
    assert_eq!(format_time(BEFORE_MIDNIGHT), "11:50 PM");
    assert_eq!(format_time(BEFORE_MIDNIGHT + 600), "12:00 AM");
}

/// Every instant in `[from, to)` is covered by a slot consistent with its
/// episode, and consecutive slots tile time without gaps or overlaps.
fn assert_schedule_tiles(slot_at: impl Fn(u64) -> Option<ScheduleSlot>, from: u64, to: u64) {
    let mut prev = slot_at(from).unwrap();
    for t in from..to {
        let slot = slot_at(t).unwrap();
        assert!(slot.remaining_secs > 0, "t={t}: zero-length slot");
        assert_eq!(slot.elapsed_secs, t - slot.start_time, "t={t}");
        assert_eq!(
            slot.elapsed_secs + slot.remaining_secs,
            slot.episode.duration_secs as u64,
            "t={t}: slot does not match its episode"
        );
        if slot.start_time != prev.start_time {
            assert_eq!(
                slot.start_time,
                prev.start_time + prev.episode.duration_secs as u64,
                "t={t}: {:?} starts before {:?} ended (phantom slot)",
                slot.episode.title,
                prev.episode.title
            );
        }
        prev = slot;
    }
}

#[test]
fn fractional_episode_durations_keep_schedule_consistent() {
    // Archive.org durations are fractional. The schedule must still tile
    // time exactly: no phantom slot at the end of each cycle.
    for channel in 1..=8 {
        let cat = catalog(
            channel,
            vec![
                episode("a", "A", 100.6),
                episode("b", "B", 200.7),
                episode("c", "C", 50.9),
            ],
        );
        let cached = CachedSchedule::new(&cat).unwrap();
        let from = 1_000_000;
        let to = from + 3 * 353;
        assert_schedule_tiles(|t| schedule_at(&cat, t), from, to);
        assert_schedule_tiles(|t| cached.at(t), from, to);
    }
}

#[test]
fn sub_second_episode_does_not_hang_the_grid() {
    // An episode shorter than a second (bad metadata) must not stall the
    // grid's range walk -- whichever position the shuffle gives it. Runs
    // on a thread with a hard timeout.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut total = 0;
        for channel in 1..=16 {
            let cat = catalog(
                channel,
                vec![episode("a", "A", 100.6), episode("b", "Blip", 0.6)],
            );
            total += schedule_range(&cat, 1_000_000, 1_000_000 + 3 * 3600).len();
            if let Some(c) = CachedSchedule::new(&cat) {
                total += c.range(1_000_000, 1_000_000 + 3 * 3600).len();
            }
        }
        let _ = tx.send(total);
    });
    let n = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("schedule range hung on a sub-second episode");
    assert!(n > 0);
}
