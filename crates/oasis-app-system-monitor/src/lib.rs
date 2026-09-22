//! System Monitor app for OASIS_OS.
//!
//! Shows platform identification, active backend and VFS type, plus
//! themed gauges for CPU, memory, battery and uptime. Live values come
//! from a [`SysStatus`] snapshot the host publishes to
//! [`status::STATUS_PATH`] (see [`probe::HostProbe`]); anything the host
//! cannot measure renders as an explicit "N/A" gauge.
//!
//! The app re-reads the status file from its per-frame hooks
//! ([`App::apply_vfs_ops`] / [`App::refresh`]) through
//! [`SystemMonitorApp::poll`], which throttles itself to one read every
//! [`POLL_INTERVAL_FRAMES`] calls, so a future per-frame tick can drive
//! it the same way.

use oasis_app_core::render::{hide_app_sdi, render_app_chrome, render_content_sdi};
use oasis_app_core::{App, AppAction, ContentState};
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_types::input::Button;
use oasis_vfs::Vfs;

pub mod probe;
mod render;
pub mod status;

pub use render::SysmonColors;
pub use status::{Gauge, Level, STATUS_PATH, SysStatus};

/// Status file re-read interval, in poll calls (~0.5 s at 60 fps).
pub const POLL_INTERVAL_FRAMES: u32 = 30;

/// Width of the text-mode gauge bars (full-screen SDI view).
const TEXT_BAR_W: usize = 16;

/// System Monitor app.
#[derive(Debug)]
pub struct SystemMonitorApp {
    content: ContentState,
    /// VFS implementation name.
    vfs_type: String,
    /// Values known at construction (platform / backend / uptime).
    base: SysStatus,
    /// Current snapshot: host-published values over `base`.
    status: SysStatus,
    /// Whether the host has published a status file.
    live: bool,
    /// Poll calls left before the status file is read again.
    poll_countdown: u32,
    /// Raw bytes of the last status file read (skip re-parsing unchanged
    /// data).
    last_raw: Option<Vec<u8>>,
}

impl SystemMonitorApp {
    /// Create a new System Monitor app for `platform` running on `backend`,
    /// using `vfs_type` (e.g. `"MemoryVfs"`, `"RealVfs"`, `"GameAssetVfs"`)
    /// and the given uptime in seconds (`0` = not known yet).
    ///
    /// Host-published values in [`STATUS_PATH`] take precedence once read.
    pub fn new(
        path: &str,
        platform: &str,
        backend: &str,
        vfs_type: &str,
        uptime_secs: u64,
    ) -> Self {
        let base = SysStatus {
            platform: Some(platform.to_string()),
            backend: Some(backend.to_string()),
            uptime_secs: (uptime_secs > 0).then_some(uptime_secs),
            ..SysStatus::default()
        };
        let mut app = Self {
            content: ContentState::new("System Monitor", path),
            vfs_type: vfs_type.to_string(),
            status: base.clone(),
            base,
            live: false,
            poll_countdown: 0,
            last_raw: None,
        };
        app.rebuild_lines();
        app
    }

    /// The snapshot currently displayed.
    pub fn status(&self) -> &SysStatus {
        &self.status
    }

    /// Whether the host has published live data.
    pub fn is_live(&self) -> bool {
        self.live
    }

    /// Re-read [`STATUS_PATH`] if the poll interval elapsed. Returns `true`
    /// when the displayed data changed.
    pub fn poll(&mut self, vfs: &dyn Vfs) -> bool {
        if self.poll_countdown > 0 {
            self.poll_countdown -= 1;
            return false;
        }
        self.poll_countdown = POLL_INTERVAL_FRAMES - 1;
        self.reload(vfs)
    }

    /// Read [`STATUS_PATH`] now. Returns `true` when the data changed.
    pub fn reload(&mut self, vfs: &dyn Vfs) -> bool {
        let raw = if vfs.exists(STATUS_PATH) {
            vfs.read(STATUS_PATH).ok()
        } else {
            None
        };
        if raw == self.last_raw {
            return false;
        }
        match &raw {
            Some(data) => {
                let live = SysStatus::parse(&String::from_utf8_lossy(data));
                self.status = merge(&self.base, live);
                self.live = true;
            },
            None => {
                self.status = self.base.clone();
                self.live = false;
            },
        }
        self.last_raw = raw;
        self.rebuild_lines();
        true
    }

    /// Info rows shown above the gauges: `(label, value)`.
    fn info_rows(&self) -> [(&'static str, String); 3] {
        let or_na = |v: &Option<String>| v.clone().unwrap_or_else(|| "N/A".to_string());
        [
            ("Platform", or_na(&self.status.platform)),
            ("Backend", or_na(&self.status.backend)),
            ("VFS", self.vfs_type.clone()),
        ]
    }

    /// Status line under the gauges.
    fn footer(&self) -> &'static str {
        if self.live {
            "Live - updates every second"
        } else {
            "Waiting for host status (live data N/A)"
        }
    }

    /// Rebuild the text lines (full-screen SDI view).
    fn rebuild_lines(&mut self) {
        let mut lines = Vec::new();
        for (label, value) in self.info_rows() {
            lines.push(format!("  {label:<10}{value}"));
        }
        lines.push(String::new());
        for g in self.status.gauges() {
            lines.push(format!("  {:<10}{}  {}", g.label, text_bar(&g), g.text));
        }
        lines.push(String::new());
        lines.push(format!("  {}", self.footer()));
        self.content.lines = lines;
    }
}

/// Live values take precedence; names fall back to the construction-time
/// values when the host leaves them out.
fn merge(base: &SysStatus, live: SysStatus) -> SysStatus {
    SysStatus {
        platform: live.platform.or_else(|| base.platform.clone()),
        backend: live.backend.or_else(|| base.backend.clone()),
        uptime_secs: live.uptime_secs.or(base.uptime_secs),
        ..live
    }
}

/// ASCII gauge for text mode: `[#####-----]`, or a blank track for N/A.
fn text_bar(g: &Gauge) -> String {
    let filled = g
        .fraction
        .map_or(0, |f| (f * TEXT_BAR_W as f32).round() as usize)
        .min(TEXT_BAR_W);
    let fill = if g.is_available() { '-' } else { ' ' };
    let mut bar = String::with_capacity(TEXT_BAR_W + 2);
    bar.push('[');
    bar.extend(std::iter::repeat_n('#', filled));
    bar.extend(std::iter::repeat_n(fill, TEXT_BAR_W - filled));
    bar.push(']');
    bar
}

impl App for SystemMonitorApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Cancel => AppAction::Exit,
            Button::Up => {
                self.content.navigate_up();
                AppAction::None
            },
            Button::Down => {
                self.content.navigate_down();
                AppAction::None
            },
            // Refresh now.
            Button::Confirm | Button::Triangle => {
                self.reload(vfs);
                AppAction::None
            },
            _ => AppAction::None,
        }
    }

    fn refresh(&mut self, vfs: &dyn Vfs) {
        self.poll(vfs);
    }

    fn apply_vfs_ops(&mut self, vfs: &mut dyn Vfs) -> bool {
        // Read-only: this is simply the host's per-frame hook with VFS
        // access.
        self.poll(vfs)
    }

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.content.update_layout(at);
        self.content.animate_selection(0.3);
        render_app_chrome(sdi, at);
        render_content_sdi(&self.content, sdi, at);
    }

    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        self.draw_monitor(cx, cy, cw, ch, backend, at)
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        hide_app_sdi(sdi);
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_test_backend::{DrawCommand, RecordingBackend};
    use oasis_vfs::MemoryVfs;

    fn app() -> SystemMonitorApp {
        SystemMonitorApp::new("/apps/sysmon", "Desktop (SDL3)", "SDL3", "MemoryVfs", 0)
    }

    fn publish(vfs: &mut MemoryVfs, status: &SysStatus) {
        probe::publish_status(vfs, status).expect("publish");
    }

    fn drawn_texts(app: &SystemMonitorApp) -> Vec<String> {
        let mut backend = RecordingBackend::new(400, 260);
        let at = ActiveTheme::default();
        app.draw_windowed(0, 0, 400, 260, &mut backend, &at)
            .expect("draw");
        backend
            .commands()
            .iter()
            .filter_map(|c| match c {
                DrawCommand::DrawText { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn title_and_path() {
        let app = app();
        assert_eq!(app.title(), "System Monitor");
        assert_eq!(app.path(), "/apps/sysmon");
    }

    #[test]
    fn lines_contain_platform_and_backend() {
        let app = app();
        assert!(app.lines().iter().any(|l| l.contains("Desktop (SDL3)")));
        assert!(app.lines().iter().any(|l| l.contains("SDL3")));
    }

    #[test]
    fn lines_show_passed_vfs_type() {
        let app = SystemMonitorApp::new("/apps/sysmon", "UE5", "UE5", "GameAssetVfs", 0);
        assert!(app.lines().iter().any(|l| l.contains("GameAssetVfs")));
    }

    #[test]
    fn uptime_formatted_as_hms() {
        let app = SystemMonitorApp::new("/apps/sysmon", "Desktop", "SDL3", "MemoryVfs", 3661);
        // 3661s = 1h 1m 1s
        assert!(app.lines().iter().any(|l| l.contains("1:01:01")));
    }

    #[test]
    fn without_host_data_every_gauge_is_na() {
        let app = app();
        assert!(!app.is_live());
        for label in ["CPU", "Memory", "Battery", "Uptime"] {
            let line = app
                .lines()
                .iter()
                .find(|l| l.trim_start().starts_with(label))
                .expect("gauge line");
            assert!(line.contains("N/A"), "{line}");
        }
        let texts = drawn_texts(&app);
        assert_eq!(texts.iter().filter(|t| t.starts_with("N/A")).count(), 4);
        assert!(texts.iter().any(|t| t.contains("Waiting for host status")));
    }

    #[test]
    fn na_gauges_draw_no_fill() {
        // An unavailable gauge draws only its track: no accent fill.
        let app = app();
        let mut backend = RecordingBackend::new(400, 260);
        let at = ActiveTheme::default();
        app.draw_windowed(0, 0, 400, 260, &mut backend, &at)
            .expect("draw");
        let accent = at.ui_theme.accent;
        assert!(!backend.commands().iter().any(|c| matches!(
            c,
            DrawCommand::FillRoundedRect { color, .. } | DrawCommand::FillRect { color, .. }
                if *color == accent
        )));
    }

    #[test]
    fn reads_published_status() {
        let mut vfs = MemoryVfs::new();
        let mut app = app();
        publish(
            &mut vfs,
            &SysStatus {
                cpu_percent: Some(50.0),
                mem_used_kb: Some(1024 * 1024),
                mem_total_kb: Some(4 * 1024 * 1024),
                battery_percent: Some(77),
                battery_state: Some(status::BatteryStatus::Discharging),
                uptime_secs: Some(3661),
                ..SysStatus::default()
            },
        );
        assert!(app.apply_vfs_ops(&mut vfs));
        assert!(app.is_live());
        // Names fall back to the constructor values.
        assert_eq!(app.status().platform.as_deref(), Some("Desktop (SDL3)"));
        let texts = drawn_texts(&app);
        for want in ["50%", "1024 MB / 4096 MB (25%)", "77%", "1:01:01"] {
            assert!(texts.iter().any(|t| t == want), "missing {want}: {texts:?}");
        }
        assert!(
            app.lines()
                .iter()
                .any(|l| l.contains("[########--------]  50%"))
        );
        // Gauges with data draw an accent fill.
        let mut backend = RecordingBackend::new(400, 260);
        let at = ActiveTheme::default();
        app.draw_windowed(0, 0, 400, 260, &mut backend, &at)
            .expect("draw");
        let fills = backend
            .commands()
            .iter()
            .filter(|c| {
                matches!(c,
                    DrawCommand::FillRoundedRect { color, .. } | DrawCommand::FillRect { color, .. }
                    if *color == at.ui_theme.accent)
            })
            .count();
        assert_eq!(fills, 4);
    }

    #[test]
    fn poll_is_throttled_and_skips_unchanged_data() {
        let mut vfs = MemoryVfs::new();
        let mut app = app();
        publish(
            &mut vfs,
            &SysStatus {
                uptime_secs: Some(1),
                ..SysStatus::default()
            },
        );
        assert!(app.poll(&vfs));
        publish(
            &mut vfs,
            &SysStatus {
                uptime_secs: Some(2),
                ..SysStatus::default()
            },
        );
        // Not re-read until the interval elapses.
        for _ in 0..POLL_INTERVAL_FRAMES - 1 {
            assert!(!app.poll(&vfs));
        }
        assert!(app.poll(&vfs));
        assert_eq!(app.status().uptime_secs, Some(2));
        // Same bytes again: no change reported.
        assert!(!app.reload(&vfs));
    }

    #[test]
    fn status_file_removed_falls_back_to_na() {
        let mut vfs = MemoryVfs::new();
        let mut app = app();
        publish(
            &mut vfs,
            &SysStatus {
                cpu_percent: Some(10.0),
                ..SysStatus::default()
            },
        );
        assert!(app.reload(&vfs));
        vfs.remove(STATUS_PATH).expect("remove");
        assert!(app.reload(&vfs));
        assert!(!app.is_live());
        assert!(!app.status().cpu_gauge().is_available());
    }

    #[test]
    fn confirm_refreshes_immediately() {
        let mut vfs = MemoryVfs::new();
        let mut app = app();
        app.poll(&vfs);
        publish(
            &mut vfs,
            &SysStatus {
                battery_percent: Some(12),
                ..SysStatus::default()
            },
        );
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.status().battery_percent, Some(12));
    }

    #[test]
    fn text_bar_clamps() {
        let full = Gauge {
            label: "X",
            fraction: Some(1.0),
            text: String::new(),
            level: Level::Normal,
        };
        assert_eq!(text_bar(&full), format!("[{}]", "#".repeat(TEXT_BAR_W)));
        let na = Gauge {
            fraction: None,
            ..full
        };
        assert_eq!(text_bar(&na), format!("[{}]", " ".repeat(TEXT_BAR_W)));
    }

    #[test]
    fn tiny_window_does_not_panic() {
        let app = app();
        let mut backend = RecordingBackend::new(20, 20);
        let at = ActiveTheme::default();
        app.draw_windowed(0, 0, 20, 20, &mut backend, &at)
            .expect("draw");
    }

    #[test]
    fn cancel_exits() {
        let vfs = MemoryVfs::new();
        let mut app = app();
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn downcast_works() {
        let app = app();
        let any = app.as_any();
        assert!(any.downcast_ref::<SystemMonitorApp>().is_some());
    }
}
