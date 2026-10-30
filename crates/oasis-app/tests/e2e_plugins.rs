#![allow(clippy::unwrap_used)] // Test code -- unwrap is acceptable.
//! Plugin lifecycle through the real shell: plugins draw into the shell's
//! scene, `update` runs every frame and `shutdown` at exit, and the
//! `plugin` / `ftp status` terminal commands are serviced by the host.

use std::sync::atomic::{AtomicUsize, Ordering};

use oasis_app::harness::{Harness, HarnessOptions};
use oasis_core::error::Result;
use oasis_core::plugin::{Plugin, PluginHost, PluginInfo};

/// The frozen harness clock is 12:00:00; the status bar shows "12:00",
/// only the clock-widget plugin draws the seconds.
const PLUGIN_CLOCK_TEXT: &str = "12:00:00";

fn output_contains(h: &Harness, needle: &str) -> bool {
    h.state()
        .terminal
        .output_lines
        .iter()
        .any(|l| l.contains(needle))
}

fn run(h: &mut Harness, command: &str) {
    h.terminal(command);
    h.settle();
}

fn clock_drawn(h: &mut Harness) -> bool {
    h.render_now();
    h.text_drawn_contains(PLUGIN_CLOCK_TEXT)
}

/// The clock-widget example plugin used to draw into a throwaway scene
/// and never got `update`: nothing appeared. It now shows the live time,
/// right-aligned, on the dashboard.
#[test]
fn clock_widget_plugin_is_drawn_with_the_live_time() {
    let mut opts = HarnessOptions::new("classic");
    opts.resolution = Some((480, 272));
    let mut h = Harness::with_options(opts).unwrap();
    h.settle();
    assert!(clock_drawn(&mut h), "{:?}", h.frame_text());
    let (x, _, w, _) = h.sdi_rect("plugin_clock_bg").expect("clock background");
    assert_eq!(x + w as i32, 480 - 6, "right-aligned on the 480px screen");

    // `pclock hide` / `show` reach the plugin through its update.
    run(&mut h, "pclock hide");
    assert!(output_contains(&h, "Clock widget: hidden"));
    assert!(!clock_drawn(&mut h), "{:?}", h.frame_text());
    run(&mut h, "pclock show");
    assert!(clock_drawn(&mut h), "{:?}", h.frame_text());
}

/// `plugin list/load/unload` used to print "(plugin status not available)"
/// and queue requests no host read. The shell now publishes the list and
/// applies the requests: unloading takes the plugin's widget and commands
/// away, loading brings them back.
#[test]
fn plugin_command_lists_unloads_and_reloads_plugins() {
    let mut h = Harness::new("classic");
    h.settle();

    run(&mut h, "plugin list");
    assert!(
        output_contains(&h, "clock-widget (1.0.0) [active]"),
        "{:?}",
        h.state().terminal.output_lines
    );
    assert!(output_contains(&h, "hello (1.0.0) [active]"));

    run(&mut h, "plugin unload clock-widget");
    assert!(output_contains(&h, "Plugin clock-widget unloaded."));
    assert!(h.sdi_rect("plugin_clock").is_none(), "widget removed");
    assert!(!clock_drawn(&mut h));
    run(&mut h, "pclock");
    assert!(
        output_contains(&h, "pclock: command not found") || output_contains(&h, "unknown command"),
        "the plugin's command left with it: {:?}",
        h.state().terminal.output_lines
    );
    run(&mut h, "plugin list");
    assert!(output_contains(&h, "clock-widget (1.0.0) [stopped]"));

    run(&mut h, "plugin load clock-widget");
    assert!(output_contains(&h, "Plugin clock-widget loaded."));
    assert!(clock_drawn(&mut h), "{:?}", h.frame_text());

    // Honest errors for what cannot work.
    run(&mut h, "plugin load no-such-plugin");
    assert!(output_contains(&h, "there is no dynamic loader"));
    run(&mut h, "plugin load hello");
    assert!(output_contains(&h, "already active"));
}

static UPDATES: AtomicUsize = AtomicUsize::new(0);
static SHUTDOWNS: AtomicUsize = AtomicUsize::new(0);

struct CountingPlugin;

impl Plugin for CountingPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo::new("counting", "1.0.0")
    }
    fn init(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
        Ok(())
    }
    fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
        UPDATES.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn shutdown(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
        SHUTDOWNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn counting() -> Box<dyn Plugin> {
    Box::new(CountingPlugin)
}

/// The shell never called `update` or `shutdown` on plugins. `update` now
/// runs once per shell step and `shutdown` once when the shell exits.
#[test]
fn plugins_are_updated_every_frame_and_shut_down_at_exit() {
    let mut opts = HarnessOptions::new("classic");
    opts.plugins = vec![counting];
    let mut h = Harness::with_options(opts).unwrap();
    let before = UPDATES.load(Ordering::SeqCst);
    h.run_frames(5);
    assert_eq!(UPDATES.load(Ordering::SeqCst) - before, 5);
    assert_eq!(SHUTDOWNS.load(Ordering::SeqCst), 0);
    h.shutdown().unwrap();
    assert_eq!(SHUTDOWNS.load(Ordering::SeqCst), 1);
}

/// `ftp status` used to read a file no host wrote ("inactive" while the
/// server ran); it now reports the shell's published network state.
#[test]
fn ftp_status_reports_the_running_server() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut h = Harness::new("classic");
    h.settle();
    run(&mut h, "ftp status");
    assert!(output_contains(&h, "FTP: inactive"));
    run(&mut h, &format!("ftp start {port}"));
    run(&mut h, "ftp status");
    assert!(
        output_contains(&h, &format!("FTP: listening on port {port}")),
        "{:?}",
        h.state().terminal.output_lines
    );
    run(&mut h, "ftp stop");
}
