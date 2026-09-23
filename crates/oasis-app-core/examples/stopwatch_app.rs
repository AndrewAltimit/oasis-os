//! Minimal OASIS_OS app: a stopwatch that exercises every `App` hook a
//! typical app needs.
//!
//! This is the example walked through in `docs/writing-apps.md`. It uses
//! the shared `ContentState` + `impl_content_app_methods!` helpers for
//! rendering, so it only implements behavior:
//!
//! - `handle_input`: gamepad-style buttons (Confirm = start/stop,
//!   Square = lap, Cancel = exit). Works on every host, including PSP.
//! - `handle_key`: keyboard-only accelerators (Ctrl+R reset, Ctrl+S save).
//! - `tick`: wall-clock time; returns `true` only when the display changes.
//! - `apply_vfs_ops`: writes the lap list with mutable VFS access.
//!
//! Run it headless with:
//!
//! ```text
//! cargo run -p oasis-app-core --example stopwatch_app
//! ```

use oasis_app_core::{App, AppAction, ContentState, impl_content_app_methods};
use oasis_types::input::{Button, Key, Modifiers};
use oasis_vfs::{MemoryVfs, Vfs};

/// Where `Ctrl+S` saves the recorded laps.
const LAPS_PATH: &str = "/home/user/laps.txt";

/// Stopwatch app state.
#[derive(Debug)]
pub struct StopwatchApp {
    /// Title, path and the lines drawn by the shared renderers.
    content: ContentState,
    /// Whether the clock is running.
    running: bool,
    /// Elapsed wall time in milliseconds.
    elapsed_ms: u64,
    /// Recorded lap times in milliseconds.
    laps: Vec<u64>,
    /// A save queued from an input handler, applied in `apply_vfs_ops`.
    save_pending: bool,
    /// Status message shown under the clock.
    status: String,
}

impl StopwatchApp {
    /// Create the app. `path` is the app's VFS path (e.g. `/apps/stopwatch`).
    pub fn new(path: &str) -> Self {
        let mut app = Self {
            content: ContentState::new("Stopwatch", path),
            running: false,
            elapsed_ms: 0,
            laps: Vec::new(),
            save_pending: false,
            status: "Confirm: start/stop  Square: lap  Ctrl+S: save".to_string(),
        };
        app.rebuild_lines();
        app
    }

    /// Format milliseconds as `MM:SS.t`.
    fn format(ms: u64) -> String {
        let tenths = (ms / 100) % 10;
        let secs = (ms / 1000) % 60;
        let mins = ms / 60_000;
        format!("{mins:02}:{secs:02}.{tenths}")
    }

    /// Regenerate the lines the shared renderers draw.
    fn rebuild_lines(&mut self) {
        let mut lines = vec![
            format!("  {}", Self::format(self.elapsed_ms)),
            String::new(),
            self.status.clone(),
            String::new(),
        ];
        for (i, lap) in self.laps.iter().enumerate() {
            lines.push(format!("Lap {:>2}: {}", i + 1, Self::format(*lap)));
        }
        self.content.lines = lines;
    }

    fn reset(&mut self) {
        self.running = false;
        self.elapsed_ms = 0;
        self.laps.clear();
        self.status = "Reset".to_string();
        self.rebuild_lines();
    }
}

impl App for StopwatchApp {
    // title, path, lines, update_sdi, draw_windowed, hide_sdi,
    // take/peek_pending_request, as_any, as_any_mut.
    impl_content_app_methods!(content);

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Cancel => return AppAction::Exit,
            Button::Confirm => {
                self.running = !self.running;
                self.status = if self.running { "Running" } else { "Stopped" }.to_string();
            },
            Button::Square => self.laps.push(self.elapsed_ms),
            Button::Up => self.content.navigate_up(),
            Button::Down => self.content.navigate_down(),
            _ => return AppAction::None,
        }
        self.rebuild_lines();
        AppAction::None
    }

    fn handle_key(&mut self, key: &Key, mods: Modifiers, _vfs: &dyn Vfs) -> Option<AppAction> {
        // Claim only exact Ctrl+<letter> combos; everything else falls
        // through to `handle_input` via the key's gamepad-style twin.
        if !mods.only(Modifiers::CTRL) {
            return None;
        }
        match key {
            Key::Char('r') => self.reset(),
            Key::Char('s') => {
                // Input hooks only get `&dyn Vfs`: queue the write.
                self.save_pending = true;
                self.status = "Saving...".to_string();
                self.rebuild_lines();
            },
            _ => return None,
        }
        Some(AppAction::None)
    }

    fn tick(&mut self, dt_ms: u32, _vfs: &dyn Vfs) -> bool {
        if !self.running {
            return false;
        }
        let before = self.elapsed_ms / 100;
        self.elapsed_ms += u64::from(dt_ms);
        // Only ask for a redraw when the displayed tenth changes.
        if self.elapsed_ms / 100 != before {
            self.rebuild_lines();
            return true;
        }
        false
    }

    fn apply_vfs_ops(&mut self, vfs: &mut dyn Vfs) -> bool {
        if !self.save_pending {
            return false;
        }
        self.save_pending = false;
        let text: String = self
            .laps
            .iter()
            .map(|lap| format!("{}\n", Self::format(*lap)))
            .collect();
        let result = vfs
            .mkdir("/home/user")
            .and_then(|()| vfs.write(LAPS_PATH, text.as_bytes()));
        self.status = match result {
            Ok(()) => format!("Saved {} laps to {LAPS_PATH}", self.laps.len()),
            Err(e) => format!("Save failed: {e}"),
        };
        self.rebuild_lines();
        true
    }
}

/// Drive the app the way a host does: input, per-frame ticks, then the
/// once-per-frame `apply_vfs_ops` drain.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut vfs = MemoryVfs::new();
    let mut app = StopwatchApp::new("/apps/stopwatch");

    app.handle_input(&Button::Confirm, &vfs); // start
    let mut redraws = 0;
    for _ in 0..90 {
        // ~1.5 s at 60 fps
        if app.tick(16, &vfs) {
            redraws += 1;
        }
    }
    app.handle_input(&Button::Square, &vfs); // lap
    app.handle_input(&Button::Confirm, &vfs); // stop

    let consumed = app.handle_key(&Key::Char('s'), Modifiers::CTRL, &vfs);
    assert_eq!(consumed, Some(AppAction::None));
    assert!(app.apply_vfs_ops(&mut vfs));

    println!(
        "{} ({} redraws requested over 90 ticks)",
        app.title(),
        redraws
    );
    for line in app.lines() {
        println!("  {line}");
    }
    let saved = String::from_utf8(vfs.read(LAPS_PATH)?)?;
    println!("{LAPS_PATH}: {}", saved.trim_end());
    Ok(())
}
