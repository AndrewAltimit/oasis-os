//! End-to-end System Monitor sessions: the host publishes status snapshots
//! to the VFS and the app is ticked like the desktop frame loop does.
#![allow(clippy::unwrap_used)]

use oasis_app_core::testing::{AppHarness, fuzz_app};
use oasis_app_core::{App, AppAction};
use oasis_app_system_monitor::probe::publish_status;
use oasis_app_system_monitor::{POLL_INTERVAL_MS, STATUS_PATH, SysStatus, SystemMonitorApp};
use oasis_types::input::{Button, Key};
use oasis_vfs::{MemoryVfs, Vfs};

fn open(_vfs: &dyn Vfs) -> Box<dyn App> {
    Box::new(SystemMonitorApp::new(
        "/apps/System Monitor",
        "Desktop (SDL3)",
        "SDL3",
        "MemoryVfs",
        0,
    ))
}

fn monitor() -> AppHarness {
    let vfs = MemoryVfs::new();
    let app = open(&vfs);
    AppHarness::with_vfs(app, vfs)
}

fn snapshot(cpu: f32, used_kb: u64) -> SysStatus {
    SysStatus {
        cpu_percent: Some(cpu),
        cpu_mhz: Some(333),
        mem_used_kb: Some(used_kb),
        mem_total_kb: Some(1_048_576),
        battery_percent: Some(64),
        uptime_secs: Some(3_725),
        ..SysStatus::default()
    }
}

fn app(h: &AppHarness) -> &SystemMonitorApp {
    h.app_as::<SystemMonitorApp>()
}

#[test]
fn shows_na_until_the_host_publishes_then_live_values() {
    let mut h = monitor();
    // First tick reads immediately (nothing published yet).
    assert!(!h.frame(16));
    let text = h.screen_text();
    assert!(text.contains("Waiting for host status"), "{text}");
    assert!(text.contains("Desktop (SDL3)"), "{text}");
    assert!(text.contains("MemoryVfs"), "{text}");

    publish_status(h.vfs_mut(), &snapshot(42.0, 262_144)).unwrap();
    // Within the poll interval nothing is re-read.
    assert!(!h.frame(POLL_INTERVAL_MS / 2));
    assert!(!app(&h).is_live());
    // Once the interval elapses the snapshot shows up.
    assert!(h.frame(POLL_INTERVAL_MS / 2));
    assert!(app(&h).is_live());
    let text = h.screen_text();
    assert!(text.contains("42% @ 333 MHz"), "{text}");
    assert!(text.contains("25%"), "memory 256 MiB of 1 GiB: {text}");
    assert!(text.contains("1:02:05"), "uptime: {text}");
    assert!(text.contains("64%"), "battery: {text}");
    // Unchanged data never asks for a redraw.
    for _ in 0..10 {
        assert!(!h.frame(POLL_INTERVAL_MS));
    }
    // New data does, on the next poll.
    publish_status(h.vfs_mut(), &snapshot(97.0, 1_000_000)).unwrap();
    assert!(h.frame(POLL_INTERVAL_MS));
    let cpu = &app(&h).status().gauges()[0];
    assert_eq!(cpu.text, "97% @ 333 MHz");
    assert!(h.screen_text().contains("97% @ 333 MHz"));
}

#[test]
fn confirm_refreshes_immediately_and_removed_status_goes_back_to_na() {
    let mut h = monitor();
    h.frame(16);
    publish_status(h.vfs_mut(), &snapshot(10.0, 1)).unwrap();
    h.key(Key::Enter);
    assert!(
        app(&h).is_live(),
        "Enter / Confirm refreshes without waiting"
    );
    h.vfs_mut().remove(STATUS_PATH).unwrap();
    h.press(Button::Triangle);
    assert!(!app(&h).is_live());
    assert!(h.screen_text().contains("Waiting for host status"));
    assert_eq!(h.key(Key::Escape), AppAction::Exit);
}

#[test]
fn malformed_and_extreme_status_is_rendered_safely() {
    let mut h = monitor();
    h.vfs_mut().mkdir("/var").unwrap();
    h.vfs_mut().mkdir("/var/sysmon").unwrap();
    h.vfs_mut()
        .write(
            STATUS_PATH,
            b"cpu_percent: NaN\nmem_used_kb: 999999999\nmem_total_kb: 10\n\
              battery_percent: 300\nuptime_secs: 18446744073709551615\n\xff\xfe:::\n",
        )
        .unwrap();
    h.frame(POLL_INTERVAL_MS);
    assert!(app(&h).is_live());
    let gauges = app(&h).status().gauges();
    for g in &gauges {
        if let Some(f) = g.fraction {
            assert!((0.0..=1.0).contains(&f), "{} gauge fraction {f}", g.label);
        }
    }
    assert!(gauges[0].fraction.is_none(), "NaN CPU must be N/A");
    h.draw_all_sizes_and_themes();
}

#[test]
fn draws_inside_window_at_all_sizes_and_themes() {
    let mut h = monitor();
    h.draw_all_sizes_and_themes();
    publish_status(h.vfs_mut(), &snapshot(88.0, 900_000)).unwrap();
    h.frame(POLL_INTERVAL_MS);
    h.draw_all_sizes_and_themes();
}

#[test]
fn fuzz_random_input_never_panics_or_escapes_window() {
    let mut vfs = MemoryVfs::new();
    publish_status(&mut vfs, &snapshot(55.5, 123_456)).unwrap();
    fuzz_app(&open, vfs, 1, 3000);
    fuzz_app(&open, MemoryVfs::new(), 2, 3000);
}
