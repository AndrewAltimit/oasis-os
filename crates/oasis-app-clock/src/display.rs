use crate::{ClockApp, ClockMode, TimerEditField};

// ---------------------------------------------------------------
// Time display helpers (pure functions)
// ---------------------------------------------------------------

/// Format seconds as "HH:MM:SS".
pub fn format_time_hms(total_secs: u64) -> String {
    let h = (total_secs / 3600) % 24;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

/// Format seconds and milliseconds as "HH:MM:SS.mmm".
pub fn format_time_ms(total_secs: u64, millis: u32) -> String {
    let h = (total_secs / 3600) % 24;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    let ms = millis % 1000;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

/// Format a duration in a friendly human-readable form.
///
/// Examples: "2h 15m 30s", "45s", "1m 5s", "0s".
pub fn format_duration_friendly(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 && m > 0 && s > 0 {
        format!("{h}h {m}m {s}s")
    } else if h > 0 && m > 0 {
        format!("{h}h {m}m")
    } else if h > 0 && s > 0 {
        format!("{h}h {s}s")
    } else if h > 0 {
        format!("{h}h")
    } else if m > 0 && s > 0 {
        format!("{m}m {s}s")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{s}s")
    }
}

/// Extract date parts from a Unix timestamp (UTC).
///
/// Returns `(year, month, day, hour, minute, second)`.
pub fn date_parts(timestamp: u64) -> (u32, u8, u8, u8, u8, u8) {
    let secs = timestamp;
    let hour = ((secs % 86400) / 3600) as u8;
    let minute = ((secs % 3600) / 60) as u8;
    let second = (secs % 60) as u8;

    // Days since Unix epoch (1970-01-01).
    let mut days = (secs / 86400) as i64;

    // Civil calendar from day count (algorithm from Howard Hinnant).
    days += 719_468; // shift epoch to 0000-03-01
    let era = if days >= 0 {
        days / 146_097
    } else {
        (days - 146_096) / 146_097
    };
    let doe = (days - era * 146_097) as u32; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    (year as u32, m as u8, d as u8, hour, minute, second)
}

/// Format a Unix timestamp as "YYYY-MM-DD".
pub fn format_date(timestamp: u64) -> String {
    let (y, m, d, _, _, _) = date_parts(timestamp);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Return the day of week for a Unix timestamp.
///
/// 0 = Monday, 1 = Tuesday, ..., 6 = Sunday.
pub fn day_of_week(timestamp: u64) -> u8 {
    // 1970-01-01 was a Thursday (index 3 if Mon=0).
    let days = timestamp / 86400;
    ((days + 3) % 7) as u8
}

/// Return the day-of-week name for a Unix timestamp.
pub fn format_weekday(timestamp: u64) -> &'static str {
    match day_of_week(timestamp) {
        0 => "Monday",
        1 => "Tuesday",
        2 => "Wednesday",
        3 => "Thursday",
        4 => "Friday",
        5 => "Saturday",
        _ => "Sunday",
    }
}

// ---------------------------------------------------------------
// ClockApp display methods
// ---------------------------------------------------------------

impl ClockApp {
    /// Build the display lines for the current mode.
    pub(crate) fn build_lines(&mut self) {
        let now = self.current_time_secs;
        let mut lines = Vec::new();

        match self.mode {
            ClockMode::Clock => {
                self.build_clock_lines(now, &mut lines);
            },
            ClockMode::Stopwatch => {
                self.build_stopwatch_lines(now, &mut lines);
            },
            ClockMode::Timer => {
                self.build_timer_lines(now, &mut lines);
            },
            ClockMode::Alarms => {
                self.build_alarm_lines(&mut lines);
            },
        }

        // Footer: mode tabs.
        lines.push(String::new());
        lines.push(Self::mode_tabs(self.mode));

        self.content.lines = lines;
    }

    fn build_clock_lines(&self, now: u64, lines: &mut Vec<String>) {
        let (_, _, _, h, m, s) = date_parts(now);
        let time_str = format!("{h:02}:{m:02}:{s:02}");
        let date_str = format_date(now);
        let weekday = format_weekday(now);

        lines.push("Clock".to_string());
        lines.push("\u{2500}".repeat(25));
        lines.push(format!("  {time_str}"));
        lines.push(format!("  {weekday}, {date_str}"));
        lines.push("\u{2500}".repeat(25));
    }

    fn build_stopwatch_lines(&self, now: u64, lines: &mut Vec<String>) {
        let (elapsed_s, elapsed_ms) = self.stopwatch.elapsed(now);
        let status = if self.stopwatch.is_running() {
            "Running"
        } else if elapsed_s > 0 || elapsed_ms > 0 {
            "Paused"
        } else {
            "Stopped"
        };

        lines.push("Stopwatch".to_string());
        lines.push("\u{2500}".repeat(25));
        lines.push(format!("  {}", format_time_ms(elapsed_s, elapsed_ms)));
        lines.push(format!("  Status: {status}"));
        lines.push("\u{2500}".repeat(25));
        lines.push("  X=Start/Stop  /\\=Lap  []=Reset".to_string());

        if !self.stopwatch.laps.is_empty() {
            lines.push(String::new());
            lines.push("  Laps:".to_string());
            for lap in self.stopwatch.laps.iter().rev() {
                lines.push(format!(
                    "  #{:<3} Split: {}  Total: {}",
                    lap.lap_number,
                    format_duration_friendly(lap.split_secs),
                    format_duration_friendly(lap.total_secs),
                ));
            }
        }
    }

    fn build_timer_lines(&self, now: u64, lines: &mut Vec<String>) {
        let remaining = self.timer.remaining(now);
        let status = if self.timer.is_running() {
            "Running"
        } else if self.timer.is_finished() {
            "Finished!"
        } else {
            "Paused"
        };

        let h = remaining / 3600;
        let m = (remaining % 3600) / 60;
        let s = remaining % 60;

        // Indicate which field is selected with brackets.
        let h_str = match self.timer_edit_field {
            TimerEditField::Hours => format!("[{h:02}]"),
            _ => format!(" {h:02} "),
        };
        let m_str = match self.timer_edit_field {
            TimerEditField::Minutes => format!("[{m:02}]"),
            _ => format!(" {m:02} "),
        };
        let s_str = match self.timer_edit_field {
            TimerEditField::Seconds => format!("[{s:02}]"),
            _ => format!(" {s:02} "),
        };

        lines.push("Timer".to_string());
        lines.push("\u{2500}".repeat(25));
        lines.push(format!("  {h_str}:{m_str}:{s_str}"));
        lines.push(format!("  Status: {status}"));
        lines.push(format!(
            "  Duration: {}",
            format_duration_friendly(self.timer.duration_secs)
        ));
        lines.push("\u{2500}".repeat(25));
        lines.push("  ^v=Adjust  <>=Field  X=Go  []=Reset".to_string());
    }

    fn build_alarm_lines(&self, lines: &mut Vec<String>) {
        lines.push("Alarms".to_string());
        lines.push("\u{2500}".repeat(25));

        if self.alarms.is_empty() {
            lines.push("  (No alarms set)".to_string());
        } else {
            for (i, alarm) in self.alarms.iter().enumerate() {
                let marker = if i == self.alarm_cursor { ">" } else { " " };
                let state = if alarm.enabled { "ON " } else { "OFF" };
                lines.push(format!(
                    " {marker} {} [{state}] {} ({})",
                    alarm.format_time(),
                    alarm.label,
                    alarm.days.format(),
                ));
            }
        }

        lines.push("\u{2500}".repeat(25));
        lines.push("  X=Toggle  /\\=Add  []=Delete".to_string());

        if let Some(idx) = self.ringing_alarm
            && let Some(alarm) = self.alarms.get(idx)
        {
            lines.push(String::new());
            lines.push(format!(
                "  ** ALARM: {} - {} **",
                alarm.format_time(),
                alarm.label,
            ));
        }
    }

    /// Build the mode tab bar line.
    pub(crate) fn mode_tabs(active: ClockMode) -> String {
        let modes = [
            ClockMode::Clock,
            ClockMode::Stopwatch,
            ClockMode::Timer,
            ClockMode::Alarms,
        ];
        let tabs: Vec<String> = modes
            .iter()
            .map(|m| {
                if *m == active {
                    format!("[{}]", m.label())
                } else {
                    m.label().to_string()
                }
            })
            .collect();
        format!(" {}", tabs.join("  "))
    }
}
