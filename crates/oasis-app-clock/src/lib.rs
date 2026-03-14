//! Clock/Timer/Stopwatch application.
//!
//! A multi-mode time utility providing:
//! - Digital clock display with date and day of week
//! - Stopwatch with lap times
//! - Countdown timer with adjustable duration
//! - Alarm list with per-day scheduling
//!
//! All time logic uses an injectable `current_time_secs` field (Unix
//! timestamp) that is set externally by the `AppRunner` refresh cycle.
//! No `SystemTime::now()` or `Instant::now()` calls are used.

use oasis_app_core::{App, AppAction, ContentState, impl_content_app_methods};
use oasis_types::input::Button;
use oasis_vfs::Vfs;

mod alarms;
mod display;

pub use alarms::{Alarm, AlarmDays};
pub use display::{
    date_parts, day_of_week, format_date, format_duration_friendly, format_time_hms,
    format_time_ms, format_weekday,
};

// ---------------------------------------------------------------
// ClockMode
// ---------------------------------------------------------------

/// Active mode of the clock application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockMode {
    /// Digital clock display with date.
    Clock,
    /// Stopwatch with lap times.
    Stopwatch,
    /// Countdown timer.
    Timer,
    /// Alarm list.
    Alarms,
}

impl ClockMode {
    /// Cycle to the next mode (wraps around).
    fn next(self) -> Self {
        match self {
            Self::Clock => Self::Stopwatch,
            Self::Stopwatch => Self::Timer,
            Self::Timer => Self::Alarms,
            Self::Alarms => Self::Clock,
        }
    }

    /// Cycle to the previous mode (wraps around).
    fn prev(self) -> Self {
        match self {
            Self::Clock => Self::Alarms,
            Self::Stopwatch => Self::Clock,
            Self::Timer => Self::Stopwatch,
            Self::Alarms => Self::Timer,
        }
    }

    /// Display label for this mode.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Clock => "Clock",
            Self::Stopwatch => "Stopwatch",
            Self::Timer => "Timer",
            Self::Alarms => "Alarms",
        }
    }
}

// ---------------------------------------------------------------
// Stopwatch
// ---------------------------------------------------------------

/// Stopwatch state with lap support.
#[derive(Debug, Clone)]
pub struct Stopwatch {
    running: bool,
    /// Timestamp when the stopwatch was last started.
    start_time: u64,
    /// Accumulated elapsed seconds from previous runs.
    elapsed_secs: u64,
    /// Fractional milliseconds (not tracked precisely without
    /// sub-second timestamps, kept for display compatibility).
    elapsed_millis: u32,
    /// Recorded lap times.
    pub(crate) laps: Vec<LapTime>,
}

/// A single lap recording.
#[derive(Debug, Clone)]
pub struct LapTime {
    /// 1-based lap number.
    pub lap_number: usize,
    /// Time since the previous lap (or since start for lap 1).
    pub split_secs: u64,
    /// Total elapsed time from the very start.
    pub total_secs: u64,
}

impl Default for Stopwatch {
    fn default() -> Self {
        Self::new()
    }
}

impl Stopwatch {
    /// Create a new stopped stopwatch.
    pub fn new() -> Self {
        Self {
            running: false,
            start_time: 0,
            elapsed_secs: 0,
            elapsed_millis: 0,
            laps: Vec::new(),
        }
    }

    /// Start (or resume) the stopwatch.
    pub fn start(&mut self, now: u64) {
        if !self.running {
            self.start_time = now;
            self.running = true;
        }
    }

    /// Stop the stopwatch, accumulating elapsed time.
    pub fn stop(&mut self, now: u64) {
        if self.running {
            self.elapsed_secs += now.saturating_sub(self.start_time);
            self.running = false;
        }
    }

    /// Reset the stopwatch to zero, clearing laps.
    pub fn reset(&mut self) {
        self.running = false;
        self.start_time = 0;
        self.elapsed_secs = 0;
        self.elapsed_millis = 0;
        self.laps.clear();
    }

    /// Record a lap at the current time.
    pub fn lap(&mut self, now: u64) {
        if !self.running {
            return;
        }
        let (total, _) = self.elapsed(now);
        let prev_total = self.laps.last().map(|l| l.total_secs).unwrap_or(0);
        let split = total.saturating_sub(prev_total);
        self.laps.push(LapTime {
            lap_number: self.laps.len() + 1,
            split_secs: split,
            total_secs: total,
        });
    }

    /// Whether the stopwatch is currently running.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Current elapsed time as `(seconds, millis)`.
    pub fn elapsed(&self, now: u64) -> (u64, u32) {
        let base = self.elapsed_secs;
        let running_extra = if self.running {
            now.saturating_sub(self.start_time)
        } else {
            0
        };
        (base + running_extra, self.elapsed_millis)
    }

    /// Toggle start/stop.
    pub fn toggle(&mut self, now: u64) {
        if self.running {
            self.stop(now);
        } else {
            self.start(now);
        }
    }
}

// ---------------------------------------------------------------
// CountdownTimer
// ---------------------------------------------------------------

/// Countdown timer with pause/resume support.
#[derive(Debug, Clone)]
pub struct CountdownTimer {
    /// Total countdown duration in seconds.
    pub(crate) duration_secs: u64,
    /// Remaining seconds (snapshot at last pause/start).
    remaining_secs: u64,
    /// Whether the timer is currently counting down.
    running: bool,
    /// Timestamp when the timer was last started/resumed.
    start_time: u64,
    /// True once the countdown has reached zero.
    finished: bool,
}

impl CountdownTimer {
    /// Create a new timer with the given duration (seconds).
    pub fn new(duration_secs: u64) -> Self {
        Self {
            duration_secs,
            remaining_secs: duration_secs,
            running: false,
            start_time: 0,
            finished: false,
        }
    }

    /// Start or resume the countdown.
    pub fn start(&mut self, now: u64) {
        if !self.running && !self.finished {
            self.start_time = now;
            self.running = true;
        }
    }

    /// Pause the countdown, snapshotting the remaining time.
    pub fn pause(&mut self, now: u64) {
        if self.running {
            let elapsed = now.saturating_sub(self.start_time);
            self.remaining_secs = self.remaining_secs.saturating_sub(elapsed);
            self.running = false;
            if self.remaining_secs == 0 {
                self.finished = true;
            }
        }
    }

    /// Reset to the original duration.
    pub fn reset(&mut self) {
        self.remaining_secs = self.duration_secs;
        self.running = false;
        self.finished = false;
        self.start_time = 0;
    }

    /// Whether the timer is actively counting down.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Whether the countdown has reached zero.
    pub fn is_finished(&self) -> bool {
        if self.running {
            // Computed on the fly.
            return false;
        }
        self.finished
    }

    /// Remaining seconds (computed from current time if running).
    pub fn remaining(&self, now: u64) -> u64 {
        if self.running {
            let elapsed = now.saturating_sub(self.start_time);
            self.remaining_secs.saturating_sub(elapsed)
        } else {
            self.remaining_secs
        }
    }

    /// Set the total duration (only when not running).
    pub fn set_duration(&mut self, secs: u64) {
        if !self.running {
            self.duration_secs = secs;
            self.remaining_secs = secs;
            self.finished = false;
        }
    }

    /// Toggle start/pause.
    pub fn toggle(&mut self, now: u64) {
        if self.running {
            self.pause(now);
        } else {
            self.start(now);
        }
    }

    /// Adjust the duration by a signed delta (only when stopped).
    ///
    /// Clamps the result to `[0, 359_999]` (99h 59m 59s).
    pub fn adjust_duration(&mut self, delta: i64) {
        if self.running {
            return;
        }
        let new_dur = if delta >= 0 {
            self.duration_secs.saturating_add(delta as u64)
        } else {
            self.duration_secs.saturating_sub(delta.unsigned_abs())
        };
        let clamped = new_dur.min(359_999);
        self.duration_secs = clamped;
        self.remaining_secs = clamped;
        self.finished = false;
    }
}

// ---------------------------------------------------------------
// TimerEditField
// ---------------------------------------------------------------

/// Which field the user is adjusting in the timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerEditField {
    Hours,
    Minutes,
    Seconds,
}

impl TimerEditField {
    fn next(self) -> Self {
        match self {
            Self::Hours => Self::Minutes,
            Self::Minutes => Self::Seconds,
            Self::Seconds => Self::Hours,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Hours => Self::Seconds,
            Self::Minutes => Self::Hours,
            Self::Seconds => Self::Minutes,
        }
    }

    /// The increment/decrement delta in seconds for this field.
    fn delta(self) -> i64 {
        match self {
            Self::Hours => 3600,
            Self::Minutes => 60,
            Self::Seconds => 1,
        }
    }
}

// ---------------------------------------------------------------
// ClockApp
// ---------------------------------------------------------------

/// Clock/Timer/Stopwatch application.
#[derive(Debug)]
pub struct ClockApp {
    pub(crate) content: ContentState,
    pub(crate) mode: ClockMode,
    /// Current Unix timestamp, set externally each frame.
    pub current_time_secs: u64,
    pub(crate) stopwatch: Stopwatch,
    pub(crate) timer: CountdownTimer,
    pub(crate) alarms: Vec<Alarm>,
    pub(crate) alarm_cursor: usize,
    pub(crate) timer_edit_field: TimerEditField,
    /// Index of an alarm currently ringing (if any).
    pub(crate) ringing_alarm: Option<usize>,
}

impl ClockApp {
    /// Create a new clock app at the given path.
    pub fn new(path: &str) -> Self {
        Self {
            content: ContentState::new("Clock", path),
            mode: ClockMode::Clock,
            current_time_secs: 0,
            stopwatch: Stopwatch::new(),
            timer: CountdownTimer::new(300), // default 5 minutes
            alarms: Vec::new(),
            alarm_cursor: 0,
            timer_edit_field: TimerEditField::Minutes,
            ringing_alarm: None,
        }
    }

    /// Check alarms and set ringing state if any match.
    fn check_alarms(&mut self) {
        let now = self.current_time_secs;
        self.ringing_alarm = None;
        for (i, alarm) in self.alarms.iter().enumerate() {
            if alarm.should_ring(now) {
                self.ringing_alarm = Some(i);
                break;
            }
        }
    }

    /// Handle input in Clock mode.
    fn handle_clock_input(&mut self, _button: &Button) -> AppAction {
        // Clock mode is display-only; no special actions.
        AppAction::None
    }

    /// Handle input in Stopwatch mode.
    fn handle_stopwatch_input(&mut self, button: &Button) -> AppAction {
        let now = self.current_time_secs;
        match button {
            Button::Confirm => {
                self.stopwatch.toggle(now);
                AppAction::None
            },
            Button::Triangle => {
                self.stopwatch.lap(now);
                AppAction::None
            },
            Button::Square => {
                self.stopwatch.reset();
                AppAction::None
            },
            _ => AppAction::None,
        }
    }

    /// Handle input in Timer mode.
    fn handle_timer_input(&mut self, button: &Button) -> AppAction {
        let now = self.current_time_secs;
        match button {
            Button::Up => {
                if !self.timer.is_running() {
                    let delta = self.timer_edit_field.delta();
                    self.timer.adjust_duration(delta);
                }
                AppAction::None
            },
            Button::Down => {
                if !self.timer.is_running() {
                    let delta = -self.timer_edit_field.delta();
                    self.timer.adjust_duration(delta);
                }
                AppAction::None
            },
            Button::Left => {
                self.timer_edit_field = self.timer_edit_field.prev();
                AppAction::None
            },
            Button::Right => {
                self.timer_edit_field = self.timer_edit_field.next();
                AppAction::None
            },
            Button::Confirm => {
                self.timer.toggle(now);
                AppAction::None
            },
            Button::Square => {
                self.timer.reset();
                AppAction::None
            },
            _ => AppAction::None,
        }
    }

    /// Handle input in Alarms mode.
    fn handle_alarms_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Up => {
                if self.alarm_cursor > 0 {
                    self.alarm_cursor -= 1;
                }
                AppAction::None
            },
            Button::Down => {
                if !self.alarms.is_empty() && self.alarm_cursor + 1 < self.alarms.len() {
                    self.alarm_cursor += 1;
                }
                AppAction::None
            },
            Button::Confirm => {
                if let Some(alarm) = self.alarms.get_mut(self.alarm_cursor) {
                    alarm.toggle();
                }
                AppAction::None
            },
            Button::Triangle => {
                // Add a default alarm at 08:00.
                self.alarms.push(Alarm::new(8, 0, "Alarm"));
                self.alarm_cursor = self.alarms.len() - 1;
                AppAction::None
            },
            Button::Square => {
                if !self.alarms.is_empty() {
                    self.alarms.remove(self.alarm_cursor);
                    if self.alarm_cursor > 0 && self.alarm_cursor >= self.alarms.len() {
                        self.alarm_cursor = self.alarms.len().saturating_sub(1);
                    }
                }
                AppAction::None
            },
            _ => AppAction::None,
        }
    }
}

impl App for ClockApp {
    impl_content_app_methods!(content);

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Cancel => return AppAction::Exit,
            Button::Start => {
                self.mode = self.mode.next();
                return AppAction::None;
            },
            Button::Select => {
                self.mode = self.mode.prev();
                return AppAction::None;
            },
            _ => {},
        }

        match self.mode {
            ClockMode::Clock => self.handle_clock_input(button),
            ClockMode::Stopwatch => self.handle_stopwatch_input(button),
            ClockMode::Timer => self.handle_timer_input(button),
            ClockMode::Alarms => self.handle_alarms_input(button),
        }
    }

    fn refresh(&mut self, _vfs: &dyn Vfs) {
        self.check_alarms();
        self.build_lines();
    }
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vfs::MemoryVfs;

    fn make_vfs() -> MemoryVfs {
        MemoryVfs::new()
    }

    // -- Time formatting tests --

    #[test]
    fn test_format_time_hms_zero() {
        assert_eq!(format_time_hms(0), "00:00:00");
    }

    #[test]
    fn test_format_time_hms_various() {
        assert_eq!(format_time_hms(3661), "01:01:01");
        assert_eq!(format_time_hms(86399), "23:59:59");
        assert_eq!(format_time_hms(43200), "12:00:00");
    }

    #[test]
    fn test_format_time_hms_wraps_at_24h() {
        // 90000 = 25 hours -> wraps to 01:00:00
        assert_eq!(format_time_hms(90000), "01:00:00");
    }

    #[test]
    fn test_format_time_ms() {
        assert_eq!(format_time_ms(0, 0), "00:00:00.000");
        assert_eq!(format_time_ms(3661, 500), "01:01:01.500");
        assert_eq!(format_time_ms(0, 42), "00:00:00.042");
    }

    #[test]
    fn test_format_time_ms_millis_clamped() {
        // 1234 millis -> 234
        assert_eq!(format_time_ms(0, 1234), "00:00:00.234");
    }

    #[test]
    fn test_format_duration_friendly_zero() {
        assert_eq!(format_duration_friendly(0), "0s");
    }

    #[test]
    fn test_format_duration_friendly_seconds() {
        assert_eq!(format_duration_friendly(45), "45s");
    }

    #[test]
    fn test_format_duration_friendly_minutes_seconds() {
        assert_eq!(format_duration_friendly(65), "1m 5s");
    }

    #[test]
    fn test_format_duration_friendly_hours() {
        assert_eq!(format_duration_friendly(8130), "2h 15m 30s");
    }

    #[test]
    fn test_format_duration_friendly_exact_hour() {
        assert_eq!(format_duration_friendly(3600), "1h");
    }

    #[test]
    fn test_format_duration_friendly_hours_seconds() {
        assert_eq!(format_duration_friendly(3605), "1h 5s");
    }

    // -- Date parts tests --

    #[test]
    fn test_date_parts_epoch() {
        let (y, mo, d, h, mi, s) = date_parts(0);
        assert_eq!((y, mo, d, h, mi, s), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn test_date_parts_known_date() {
        // 2026-03-02 12:34:56 UTC
        // Verified: 1772276096 is Mon 2026-03-02 12:34:56 UTC
        // Actually let's compute: days from 1970-01-01 to 2026-03-02
        // Use a known timestamp: 1_000_000_000 = 2001-09-09T01:46:40
        let (y, mo, d, h, mi, s) = date_parts(1_000_000_000);
        assert_eq!(y, 2001);
        assert_eq!(mo, 9);
        assert_eq!(d, 9);
        assert_eq!(h, 1);
        assert_eq!(mi, 46);
        assert_eq!(s, 40);
    }

    #[test]
    fn test_date_parts_leap_year() {
        // 2000-02-29 00:00:00 UTC = 951782400
        let (y, mo, d, _, _, _) = date_parts(951_782_400);
        assert_eq!((y, mo, d), (2000, 2, 29));
    }

    #[test]
    fn test_format_date() {
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(format_date(1_000_000_000), "2001-09-09");
    }

    #[test]
    fn test_day_of_week_epoch() {
        // 1970-01-01 was a Thursday = 3.
        assert_eq!(day_of_week(0), 3);
    }

    #[test]
    fn test_format_weekday_epoch() {
        assert_eq!(format_weekday(0), "Thursday");
    }

    #[test]
    fn test_day_of_week_known() {
        // 1_000_000_000 = 2001-09-09 was a Sunday = 6.
        assert_eq!(day_of_week(1_000_000_000), 6);
        assert_eq!(format_weekday(1_000_000_000), "Sunday");
    }

    #[test]
    fn test_day_of_week_full_cycle() {
        // Check a full week starting from a known Monday.
        // 2024-01-01 was a Monday. Timestamp: 1704067200.
        let mon = 1_704_067_200u64;
        assert_eq!(day_of_week(mon), 0); // Monday
        assert_eq!(day_of_week(mon + 86400), 1); // Tuesday
        assert_eq!(day_of_week(mon + 86400 * 2), 2); // Wednesday
        assert_eq!(day_of_week(mon + 86400 * 3), 3); // Thursday
        assert_eq!(day_of_week(mon + 86400 * 4), 4); // Friday
        assert_eq!(day_of_week(mon + 86400 * 5), 5); // Saturday
        assert_eq!(day_of_week(mon + 86400 * 6), 6); // Sunday
    }

    // -- Stopwatch tests --

    #[test]
    fn test_stopwatch_new() {
        let sw = Stopwatch::new();
        assert!(!sw.is_running());
        assert_eq!(sw.elapsed(0), (0, 0));
        assert!(sw.laps.is_empty());
    }

    #[test]
    fn test_stopwatch_start_stop() {
        let mut sw = Stopwatch::new();
        sw.start(100);
        assert!(sw.is_running());
        assert_eq!(sw.elapsed(110), (10, 0));
        sw.stop(115);
        assert!(!sw.is_running());
        assert_eq!(sw.elapsed(200), (15, 0));
    }

    #[test]
    fn test_stopwatch_resume() {
        let mut sw = Stopwatch::new();
        sw.start(100);
        sw.stop(110); // 10s elapsed
        sw.start(200);
        assert_eq!(sw.elapsed(205), (15, 0));
        sw.stop(210); // +10s = 20s total
        assert_eq!(sw.elapsed(999), (20, 0));
    }

    #[test]
    fn test_stopwatch_reset() {
        let mut sw = Stopwatch::new();
        sw.start(100);
        sw.stop(200);
        sw.lap(200); // won't record (not running)
        sw.start(200);
        sw.lap(210);
        sw.stop(220);
        sw.reset();
        assert!(!sw.is_running());
        assert_eq!(sw.elapsed(999), (0, 0));
        assert!(sw.laps.is_empty());
    }

    #[test]
    fn test_stopwatch_lap() {
        let mut sw = Stopwatch::new();
        sw.start(100);
        sw.lap(110); // lap 1: split=10, total=10
        sw.lap(125); // lap 2: split=15, total=25
        assert_eq!(sw.laps.len(), 2);
        assert_eq!(sw.laps[0].lap_number, 1);
        assert_eq!(sw.laps[0].split_secs, 10);
        assert_eq!(sw.laps[0].total_secs, 10);
        assert_eq!(sw.laps[1].lap_number, 2);
        assert_eq!(sw.laps[1].split_secs, 15);
        assert_eq!(sw.laps[1].total_secs, 25);
    }

    #[test]
    fn test_stopwatch_lap_when_stopped() {
        let mut sw = Stopwatch::new();
        sw.lap(100); // should be ignored
        assert!(sw.laps.is_empty());
    }

    #[test]
    fn test_stopwatch_toggle() {
        let mut sw = Stopwatch::new();
        sw.toggle(100);
        assert!(sw.is_running());
        sw.toggle(110);
        assert!(!sw.is_running());
        assert_eq!(sw.elapsed(999), (10, 0));
    }

    #[test]
    fn test_stopwatch_double_start() {
        let mut sw = Stopwatch::new();
        sw.start(100);
        sw.start(200); // should be ignored
        assert_eq!(sw.elapsed(210), (110, 0));
    }

    #[test]
    fn test_stopwatch_double_stop() {
        let mut sw = Stopwatch::new();
        sw.start(100);
        sw.stop(110);
        sw.stop(120); // should be ignored
        assert_eq!(sw.elapsed(999), (10, 0));
    }

    // -- CountdownTimer tests --

    #[test]
    fn test_timer_new() {
        let t = CountdownTimer::new(300);
        assert_eq!(t.remaining(0), 300);
        assert!(!t.is_running());
        assert!(!t.is_finished());
    }

    #[test]
    fn test_timer_countdown() {
        let mut t = CountdownTimer::new(60);
        t.start(100);
        assert!(t.is_running());
        assert_eq!(t.remaining(130), 30);
        assert_eq!(t.remaining(160), 0);
    }

    #[test]
    fn test_timer_pause_resume() {
        let mut t = CountdownTimer::new(100);
        t.start(0);
        t.pause(30); // 70 remaining
        assert!(!t.is_running());
        assert_eq!(t.remaining(999), 70);
        t.start(500);
        assert_eq!(t.remaining(520), 50);
    }

    #[test]
    fn test_timer_finished() {
        let mut t = CountdownTimer::new(10);
        t.start(0);
        t.pause(15); // elapsed > duration, remaining=0
        assert!(t.is_finished());
        assert_eq!(t.remaining(999), 0);
    }

    #[test]
    fn test_timer_reset() {
        let mut t = CountdownTimer::new(60);
        t.start(0);
        t.pause(30);
        t.reset();
        assert_eq!(t.remaining(0), 60);
        assert!(!t.is_running());
        assert!(!t.is_finished());
    }

    #[test]
    fn test_timer_set_duration() {
        let mut t = CountdownTimer::new(60);
        t.set_duration(120);
        assert_eq!(t.remaining(0), 120);
    }

    #[test]
    fn test_timer_set_duration_while_running() {
        let mut t = CountdownTimer::new(60);
        t.start(0);
        t.set_duration(120); // ignored while running
        t.pause(10);
        assert_eq!(t.remaining(0), 50); // original duration
    }

    #[test]
    fn test_timer_toggle() {
        let mut t = CountdownTimer::new(60);
        t.toggle(0);
        assert!(t.is_running());
        t.toggle(20);
        assert!(!t.is_running());
        assert_eq!(t.remaining(999), 40);
    }

    #[test]
    fn test_timer_adjust_duration() {
        let mut t = CountdownTimer::new(300);
        t.adjust_duration(60);
        assert_eq!(t.remaining(0), 360);
        t.adjust_duration(-120);
        assert_eq!(t.remaining(0), 240);
    }

    #[test]
    fn test_timer_adjust_duration_clamp_zero() {
        let mut t = CountdownTimer::new(10);
        t.adjust_duration(-100);
        assert_eq!(t.remaining(0), 0);
    }

    #[test]
    fn test_timer_adjust_while_running() {
        let mut t = CountdownTimer::new(60);
        t.start(0);
        t.adjust_duration(100); // ignored
        t.pause(10);
        assert_eq!(t.remaining(0), 50);
    }

    #[test]
    fn test_timer_zero_duration() {
        let t = CountdownTimer::new(0);
        assert_eq!(t.remaining(0), 0);
        assert!(!t.is_finished()); // not started, so not finished
    }

    #[test]
    fn test_timer_cannot_start_after_finished() {
        let mut t = CountdownTimer::new(5);
        t.start(0);
        t.pause(10); // finished
        assert!(t.is_finished());
        t.start(20); // should be ignored
        assert!(!t.is_running());
    }

    // -- Alarm tests --

    #[test]
    fn test_alarm_new() {
        let a = Alarm::new(8, 30, "Wake up");
        assert_eq!(a.hour, 8);
        assert_eq!(a.minute, 30);
        assert!(a.enabled);
        assert_eq!(a.label, "Wake up");
        assert_eq!(a.days.format(), "Every day");
    }

    #[test]
    fn test_alarm_format_time() {
        let a = Alarm::new(14, 5, "Meeting");
        assert_eq!(a.format_time(), "14:05");
    }

    #[test]
    fn test_alarm_toggle() {
        let mut a = Alarm::new(8, 0, "Test");
        assert!(a.enabled);
        a.toggle();
        assert!(!a.enabled);
        a.toggle();
        assert!(a.enabled);
    }

    #[test]
    fn test_alarm_should_ring_matching() {
        let a = Alarm::new(1, 46, "Test");
        // 1_000_000_000 = 2001-09-09 01:46:40 (Sunday=6)
        assert!(a.should_ring(1_000_000_000));
    }

    #[test]
    fn test_alarm_should_ring_disabled() {
        let mut a = Alarm::new(1, 46, "Test");
        a.enabled = false;
        assert!(!a.should_ring(1_000_000_000));
    }

    #[test]
    fn test_alarm_should_ring_wrong_time() {
        let a = Alarm::new(12, 0, "Noon");
        assert!(!a.should_ring(1_000_000_000)); // 01:46
    }

    #[test]
    fn test_alarm_day_filter() {
        let mut a = Alarm::new(1, 46, "Test");
        a.days = AlarmDays::weekdays(); // Mon-Fri
        // 1_000_000_000 is Sunday -> should NOT ring.
        assert!(!a.should_ring(1_000_000_000));
    }

    #[test]
    fn test_alarm_clamps_hour_minute() {
        let a = Alarm::new(25, 99, "Bad");
        assert_eq!(a.hour, 23);
        assert_eq!(a.minute, 59);
    }

    // -- AlarmDays tests --

    #[test]
    fn test_alarm_days_every_day() {
        let d = AlarmDays::every_day();
        for i in 0..7 {
            assert!(d.is_set(i), "day {i} should be set");
        }
        assert_eq!(d.format(), "Every day");
    }

    #[test]
    fn test_alarm_days_weekdays() {
        let d = AlarmDays::weekdays();
        for i in 0..5 {
            assert!(d.is_set(i), "day {i} should be set");
        }
        assert!(!d.is_set(5)); // Saturday
        assert!(!d.is_set(6)); // Sunday
        assert_eq!(d.format(), "Weekdays");
    }

    #[test]
    fn test_alarm_days_weekends() {
        let d = AlarmDays::weekends();
        for i in 0..5 {
            assert!(!d.is_set(i), "day {i} should not be set");
        }
        assert!(d.is_set(5));
        assert!(d.is_set(6));
        assert_eq!(d.format(), "Weekends");
    }

    #[test]
    fn test_alarm_days_none() {
        let d = AlarmDays::none();
        for i in 0..7 {
            assert!(!d.is_set(i));
        }
        assert_eq!(d.format(), "Never");
    }

    #[test]
    fn test_alarm_days_toggle() {
        let mut d = AlarmDays::none();
        d.toggle(0); // enable Monday
        assert!(d.is_set(0));
        d.toggle(2); // enable Wednesday
        assert!(d.is_set(2));
        d.toggle(0); // disable Monday
        assert!(!d.is_set(0));
        assert_eq!(d.format(), "Wed");
    }

    #[test]
    fn test_alarm_days_set_clear() {
        let mut d = AlarmDays::none();
        d.set(4); // Friday
        d.set(5); // Saturday
        assert_eq!(d.format(), "Fri Sat");
        d.clear(4);
        assert_eq!(d.format(), "Sat");
    }

    #[test]
    fn test_alarm_days_out_of_range() {
        let mut d = AlarmDays::none();
        d.set(7); // out of range
        assert!(!d.is_set(7));
        d.toggle(8);
        d.clear(9);
        assert_eq!(d.bits, 0);
    }

    // -- ClockApp tests --

    #[test]
    fn test_clock_app_new() {
        let app = ClockApp::new("/apps/clock");
        assert_eq!(app.title(), "Clock");
        assert_eq!(app.path(), "/apps/clock");
        assert_eq!(app.mode, ClockMode::Clock);
    }

    #[test]
    fn test_clock_app_mode_switching_rtrigger() {
        let vfs = make_vfs();
        let mut app = ClockApp::new("/apps/clock");
        assert_eq!(app.mode, ClockMode::Clock);
        app.handle_input(&Button::Start, &vfs);
        assert_eq!(app.mode, ClockMode::Stopwatch);
        app.handle_input(&Button::Start, &vfs);
        assert_eq!(app.mode, ClockMode::Timer);
        app.handle_input(&Button::Start, &vfs);
        assert_eq!(app.mode, ClockMode::Alarms);
        app.handle_input(&Button::Start, &vfs);
        assert_eq!(app.mode, ClockMode::Clock);
    }

    #[test]
    fn test_clock_app_mode_switching_ltrigger() {
        let vfs = make_vfs();
        let mut app = ClockApp::new("/apps/clock");
        app.handle_input(&Button::Select, &vfs);
        assert_eq!(app.mode, ClockMode::Alarms);
        app.handle_input(&Button::Select, &vfs);
        assert_eq!(app.mode, ClockMode::Timer);
    }

    #[test]
    fn test_clock_app_cancel_exits() {
        let vfs = make_vfs();
        let mut app = ClockApp::new("/apps/clock");
        let action = app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::Exit);
    }

    #[test]
    fn test_clock_mode_display_lines() {
        let mut app = ClockApp::new("/apps/clock");
        app.current_time_secs = 1_000_000_000; // 2001-09-09
        app.build_lines();
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("Clock")));
        assert!(lines.iter().any(|l| l.contains("01:46:40")));
        assert!(lines.iter().any(|l| l.contains("Sunday")));
        assert!(lines.iter().any(|l| l.contains("2001-09-09")));
    }

    #[test]
    fn test_stopwatch_mode_input() {
        let vfs = make_vfs();
        let mut app = ClockApp::new("/apps/clock");
        app.current_time_secs = 100;
        app.mode = ClockMode::Stopwatch;

        // Start
        app.handle_input(&Button::Confirm, &vfs);
        assert!(app.stopwatch.is_running());

        // Lap
        app.current_time_secs = 110;
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.stopwatch.laps.len(), 1);

        // Stop
        app.handle_input(&Button::Confirm, &vfs);
        assert!(!app.stopwatch.is_running());

        // Reset
        app.handle_input(&Button::Square, &vfs);
        assert_eq!(app.stopwatch.elapsed(999), (0, 0));
        assert!(app.stopwatch.laps.is_empty());
    }

    #[test]
    fn test_timer_mode_input() {
        let vfs = make_vfs();
        let mut app = ClockApp::new("/apps/clock");
        app.current_time_secs = 0;
        app.mode = ClockMode::Timer;

        // Default edit field is Minutes.
        assert_eq!(app.timer_edit_field, TimerEditField::Minutes);

        // Adjust up (add 60s).
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.timer.remaining(0), 360);

        // Switch to seconds field.
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(app.timer_edit_field, TimerEditField::Seconds);

        // Adjust down (subtract 1s).
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.timer.remaining(0), 359);

        // Start.
        app.handle_input(&Button::Confirm, &vfs);
        assert!(app.timer.is_running());

        // Pause.
        app.current_time_secs = 10;
        app.handle_input(&Button::Confirm, &vfs);
        assert!(!app.timer.is_running());
        assert_eq!(app.timer.remaining(10), 349);

        // Reset.
        app.handle_input(&Button::Square, &vfs);
        // After reset, remaining = duration (359).
        assert_eq!(app.timer.remaining(0), 359);
    }

    #[test]
    fn test_alarms_mode_input() {
        let vfs = make_vfs();
        let mut app = ClockApp::new("/apps/clock");
        app.mode = ClockMode::Alarms;

        // Add alarm.
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.alarms.len(), 1);
        assert_eq!(app.alarm_cursor, 0);
        assert!(app.alarms[0].enabled);

        // Toggle off.
        app.handle_input(&Button::Confirm, &vfs);
        assert!(!app.alarms[0].enabled);

        // Add another.
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.alarms.len(), 2);
        assert_eq!(app.alarm_cursor, 1);

        // Navigate up.
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.alarm_cursor, 0);

        // Navigate down.
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.alarm_cursor, 1);

        // Delete.
        app.handle_input(&Button::Square, &vfs);
        assert_eq!(app.alarms.len(), 1);
        assert_eq!(app.alarm_cursor, 0);

        // Delete last one.
        app.handle_input(&Button::Square, &vfs);
        assert!(app.alarms.is_empty());
        assert_eq!(app.alarm_cursor, 0);
    }

    #[test]
    fn test_stopwatch_display_lines() {
        let mut app = ClockApp::new("/apps/clock");
        app.mode = ClockMode::Stopwatch;
        app.current_time_secs = 100;
        app.stopwatch.start(90);
        app.build_lines();
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("Stopwatch")));
        assert!(lines.iter().any(|l| l.contains("Running")));
        assert!(lines.iter().any(|l| l.contains("00:00:10.000")));
    }

    #[test]
    fn test_timer_display_lines() {
        let mut app = ClockApp::new("/apps/clock");
        app.mode = ClockMode::Timer;
        app.current_time_secs = 0;
        app.build_lines();
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("Timer")));
        assert!(lines.iter().any(|l| l.contains("Paused")));
        assert!(lines.iter().any(|l| l.contains("5m")));
    }

    #[test]
    fn test_alarm_display_lines() {
        let mut app = ClockApp::new("/apps/clock");
        app.mode = ClockMode::Alarms;
        app.alarms.push(Alarm::new(7, 30, "Morning"));
        app.build_lines();
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("Alarms")));
        assert!(lines.iter().any(|l| l.contains("07:30")));
        assert!(lines.iter().any(|l| l.contains("Morning")));
    }

    #[test]
    fn test_alarm_display_empty() {
        let mut app = ClockApp::new("/apps/clock");
        app.mode = ClockMode::Alarms;
        app.build_lines();
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("No alarms set")));
    }

    #[test]
    fn test_mode_tabs_format() {
        let tabs = ClockApp::mode_tabs(ClockMode::Clock);
        assert!(tabs.contains("[Clock]"));
        assert!(tabs.contains("Stopwatch"));
        assert!(!tabs.contains("[Stopwatch]"));

        let tabs2 = ClockApp::mode_tabs(ClockMode::Timer);
        assert!(tabs2.contains("[Timer]"));
        assert!(!tabs2.contains("[Clock]"));
    }

    #[test]
    fn test_check_alarms_ringing() {
        let mut app = ClockApp::new("/apps/clock");
        // 1_000_000_000 = 2001-09-09 01:46:40 (Sunday)
        app.current_time_secs = 1_000_000_000;
        app.alarms.push(Alarm::new(1, 46, "Match"));
        app.check_alarms();
        assert_eq!(app.ringing_alarm, Some(0));
    }

    #[test]
    fn test_check_alarms_not_ringing() {
        let mut app = ClockApp::new("/apps/clock");
        app.current_time_secs = 1_000_000_000;
        app.alarms.push(Alarm::new(12, 0, "Noon"));
        app.check_alarms();
        assert!(app.ringing_alarm.is_none());
    }

    #[test]
    fn test_midnight_rollover_display() {
        let mut app = ClockApp::new("/apps/clock");
        // Midnight exactly: 86400 * N.
        app.current_time_secs = 86400;
        app.build_lines();
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("00:00:00")));
        assert!(lines.iter().any(|l| l.contains("1970-01-02")));
    }

    #[test]
    fn test_timer_edit_field_cycling() {
        assert_eq!(TimerEditField::Hours.next(), TimerEditField::Minutes);
        assert_eq!(TimerEditField::Minutes.next(), TimerEditField::Seconds);
        assert_eq!(TimerEditField::Seconds.next(), TimerEditField::Hours);
        assert_eq!(TimerEditField::Hours.prev(), TimerEditField::Seconds);
        assert_eq!(TimerEditField::Seconds.prev(), TimerEditField::Minutes);
    }

    #[test]
    fn test_downcast() {
        let app = ClockApp::new("/apps/clock");
        let any = app.as_any();
        assert!(any.downcast_ref::<ClockApp>().is_some());
    }

    #[test]
    fn test_refresh_builds_lines() {
        let vfs = make_vfs();
        let mut app = ClockApp::new("/apps/clock");
        app.current_time_secs = 1_000_000_000;
        app.refresh(&vfs);
        assert!(!app.lines().is_empty());
    }

    #[test]
    fn test_empty_lap_list_display() {
        let mut app = ClockApp::new("/apps/clock");
        app.mode = ClockMode::Stopwatch;
        app.current_time_secs = 0;
        app.build_lines();
        let lines = app.lines();
        // Should not contain "Laps:" header when empty.
        assert!(!lines.iter().any(|l| l.contains("Laps:")));
    }

    #[test]
    fn test_stopwatch_elapsed_empty_laps() {
        let sw = Stopwatch::new();
        assert_eq!(sw.elapsed(0), (0, 0));
        assert!(sw.laps.is_empty());
    }
}
