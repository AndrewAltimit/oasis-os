//! End-to-end Internet Radio sessions through the `App` trait, with the
//! radio subsystem emulated the way `oasis-app` drives it: requests are
//! taken from the app, and playback status is published to
//! `RADIO_STATUS_PATH`.
#![allow(clippy::unwrap_used)]

use oasis_app_core::testing::{AppHarness, fuzz_app};
use oasis_app_core::{App, AppAction};
use oasis_app_radio::{PlayState, RadioApp};
use oasis_audio::radio::station::StationRegistry;
use oasis_audio::{RADIO_REQUEST_PATH, RADIO_STATUS_PATH};
use oasis_types::input::{Button, Key};
use oasis_vfs::{MemoryVfs, Vfs};

const STATIONS: &str = "/etc/radio/stations.toml";

fn station_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    for d in ["/etc", "/etc/radio", "/var", "/var/radio"] {
        vfs.mkdir(d).unwrap();
    }
    let toml = StationRegistry::defaults().to_toml().unwrap();
    vfs.write(STATIONS, toml.as_bytes()).unwrap();
    vfs
}

fn open(vfs: &dyn Vfs) -> Box<dyn App> {
    Box::new(RadioApp::new("/apps/Internet Radio", vfs))
}

fn radio() -> AppHarness {
    let vfs = station_vfs();
    let app = open(&vfs);
    AppHarness::with_vfs(app, vfs)
}

fn request(h: &mut AppHarness) -> Option<String> {
    h.app_mut().take_pending_request().map(|(path, data)| {
        assert_eq!(path, RADIO_REQUEST_PATH);
        data
    })
}

fn publish(h: &mut AppHarness, status: &str) {
    h.vfs_mut()
        .write(RADIO_STATUS_PATH, status.as_bytes())
        .unwrap();
    // `oasis-app`'s radio controller calls `refresh` every frame while the
    // radio is visible (the harness has no bare refresh, so swap the VFS
    // out to lend it to the app).
    let vfs = std::mem::replace(h.vfs_mut(), MemoryVfs::new());
    h.app_mut().refresh(&vfs);
    *h.vfs_mut() = vfs;
}

fn defaults() -> Vec<String> {
    StationRegistry::defaults()
        .stations
        .into_iter()
        .map(|s| s.name)
        .collect()
}

#[test]
fn clicking_a_station_tunes_it_and_published_status_is_shown() {
    let mut h = radio();
    let names = defaults();
    assert!(
        h.screen_text()
            .contains(&format!("Stations ({})", names.len()))
    );
    assert!(h.screen_text().contains("STOPPED"));
    h.click_text(&names[2]);
    assert_eq!(request(&mut h).as_deref(), Some("tune 2"));
    // The subsystem connects, then plays.
    publish(
        &mut h,
        &format!("State: buffering\nStation: {}\nVolume: 80%\n", names[2]),
    );
    assert_eq!(h.app_as::<RadioApp>().status().state, PlayState::Loading);
    assert!(h.screen_text().contains("LOADING"));
    publish(
        &mut h,
        &format!(
            "State: playing\nStation: {}\nNow Playing: Episode 7\nVolume: 80%\n",
            names[2]
        ),
    );
    let text = h.screen_text();
    assert!(text.contains("PLAYING"), "{text}");
    assert!(text.contains("Episode 7"), "{text}");
    // Stop via the drawn button.
    h.click_text("Stop");
    assert_eq!(request(&mut h).as_deref(), Some("stop"));
}

#[test]
fn keyboard_navigation_tune_stop_and_escape() {
    let mut h = radio();
    let names = defaults();
    h.key(Key::Down);
    h.key(Key::Down);
    h.key(Key::Up);
    h.key(Key::Enter);
    assert_eq!(request(&mut h).as_deref(), Some("tune 1"));
    // Past the end clamps to the last station.
    for _ in 0..50 {
        h.key(Key::Down);
    }
    h.key(Key::Enter);
    assert_eq!(request(&mut h), Some(format!("tune {}", names.len() - 1)));
    // The selected (last) station is scrolled into the drawn list.
    assert!(h.screen_text().contains(names.last().unwrap().as_str()));
    // Escape while playing stops instead of closing.
    publish(&mut h, "State: playing\nStation: X\nVolume: 80%\n");
    assert_eq!(h.key(Key::Escape), AppAction::None);
    assert_eq!(request(&mut h).as_deref(), Some("stop"));
    assert!(!h.closed());
    publish(&mut h, "State: stopped\nVolume: 80%\n");
    h.key(Key::Escape);
    assert!(h.closed(), "Escape on a stopped radio closes the app");
}

#[test]
fn volume_controls_clamp_and_survive_until_confirmed() {
    let mut h = radio();
    assert!(h.screen_text().contains("Vol 80%"));
    h.click_text("Vol+");
    assert_eq!(request(&mut h).as_deref(), Some("vol 90"));
    h.type_text("+");
    h.type_text("+");
    assert_eq!(request(&mut h).as_deref(), Some("vol 100"));
    // A stale published volume does not snap the control back.
    publish(&mut h, "State: stopped\nVolume: 80%\n");
    assert_eq!(h.app_as::<RadioApp>().volume(), 100);
    publish(&mut h, "State: stopped\nVolume: 100%\n");
    assert_eq!(h.app_as::<RadioApp>().volume(), 100);
    for _ in 0..12 {
        h.press(Button::Left);
    }
    assert_eq!(request(&mut h).as_deref(), Some("vol 0"));
    assert!(h.screen_text().contains("Vol 0%"));
    // Clicking the volume bar sets the value under the pointer: the bar
    // starts right of the "Vol" label, just left of the Vol- button.
    let b = h.draw();
    let label = b.find_text("Vol 0%").unwrap().clone();
    let minus = b.find_text("Vol-").unwrap().clone();
    let (ox, oy) = (8, 20); // harness content origin
    let y = label.center().1 - oy + 1;
    h.click(minus.x - ox - 20, y);
    let v = request(&mut h).unwrap();
    let pct: u8 = v.strip_prefix("vol ").unwrap().parse().unwrap();
    assert!(pct >= 85, "click near the bar's right end gave {v}");
}

#[test]
fn favorite_toggle_is_requested_and_survives_reopening_the_app() {
    let mut h = radio();
    let names = defaults();
    h.key(Key::Down);
    h.press(Button::Triangle);
    assert_eq!(request(&mut h).as_deref(), Some("fav 1"));
    assert!(
        h.app()
            .lines()
            .iter()
            .any(|l| l.contains("[*]") && l.contains(&names[1]))
    );
    // Host frame: VFS work is applied.
    h.frame(16);
    // Close and reopen the app on the same VFS: the favorite is kept.
    let app = open(h.vfs());
    h.replace_app(app);
    assert!(
        h.app()
            .lines()
            .iter()
            .any(|l| l.contains("[*]") && l.contains(&names[1])),
        "favorite lost on reopen:\n{}",
        h.app().lines().join("\n")
    );
    // Un-favorite round-trips too.
    h.key(Key::Down);
    h.press(Button::Triangle);
    h.frame(16);
    let app = open(h.vfs());
    h.replace_app(app);
    assert!(
        !h.app()
            .lines()
            .iter()
            .any(|l| l.contains("[*]") && l.contains(&names[1])),
        "{}",
        h.app().lines().join("\n")
    );
}

#[test]
fn error_status_and_broken_config_are_handled() {
    let mut vfs = station_vfs();
    vfs.write(STATIONS, b"this is [not toml").unwrap();
    vfs.write(
        RADIO_STATUS_PATH,
        b"State: error\nError: connection refused\n",
    )
    .unwrap();
    let app = open(&vfs);
    let mut h = AppHarness::with_vfs(app, vfs);
    // Falls back to the built-in stations.
    assert!(h.screen_text().contains(&defaults()[0]));
    let text = h.screen_text();
    assert!(text.contains("ERROR"), "{text}");
    assert!(text.contains("connection refused"), "{text}");
    h.draw_all_sizes_and_themes();
    // An empty station list still works.
    let empty = StationRegistry { stations: vec![] }.to_toml().unwrap();
    h.vfs_mut().write(STATIONS, empty.as_bytes()).unwrap();
    let app = open(h.vfs());
    h.replace_app(app);
    h.key(Key::Enter);
    h.key(Key::Down);
    h.press(Button::Triangle);
    assert!(
        h.screen_text().contains("Stations (0)"),
        "{}",
        h.screen_text()
    );
    assert!(request(&mut h).is_none());
    h.draw_all_sizes_and_themes();
}

#[test]
fn draws_inside_window_at_all_sizes_and_themes() {
    let mut h = radio();
    publish(
        &mut h,
        "State: playing\nStation: A very long station name that will not fit\n\
         Now Playing: An even longer track title to overflow the panel width\nVolume: 55%\n",
    );
    h.draw_all_sizes_and_themes();
}

#[test]
fn fuzz_random_input_never_panics_or_escapes_window() {
    for seed in 1..=3 {
        fuzz_app(&|vfs: &dyn Vfs| open(vfs), station_vfs(), seed, 3000);
    }
}
