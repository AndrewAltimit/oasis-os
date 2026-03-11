use crate::{date_parts, day_of_week};

// ---------------------------------------------------------------
// AlarmDays
// ---------------------------------------------------------------

/// Bitmask of days on which an alarm is active.
///
/// Bit 0 = Monday, bit 1 = Tuesday, ..., bit 6 = Sunday.
#[derive(Debug, Clone)]
pub struct AlarmDays {
    /// Bitmask of active days.
    pub bits: u8,
}

impl AlarmDays {
    /// All seven days enabled.
    pub fn every_day() -> Self {
        Self { bits: 0b0111_1111 }
    }

    /// Monday through Friday.
    pub fn weekdays() -> Self {
        Self { bits: 0b0001_1111 }
    }

    /// Saturday and Sunday.
    pub fn weekends() -> Self {
        Self { bits: 0b0110_0000 }
    }

    /// No days enabled.
    pub fn none() -> Self {
        Self { bits: 0 }
    }

    /// Check whether a specific day is set (0=Mon..6=Sun).
    pub fn is_set(&self, day: u8) -> bool {
        if day > 6 {
            return false;
        }
        self.bits & (1 << day) != 0
    }

    /// Enable a specific day.
    pub fn set(&mut self, day: u8) {
        if day <= 6 {
            self.bits |= 1 << day;
        }
    }

    /// Disable a specific day.
    pub fn clear(&mut self, day: u8) {
        if day <= 6 {
            self.bits &= !(1 << day);
        }
    }

    /// Toggle a specific day on/off.
    pub fn toggle(&mut self, day: u8) {
        if day <= 6 {
            self.bits ^= 1 << day;
        }
    }

    /// Human-readable format of the active days.
    pub fn format(&self) -> String {
        if self.bits == 0b0111_1111 {
            return "Every day".to_string();
        }
        if self.bits == 0b0001_1111 {
            return "Weekdays".to_string();
        }
        if self.bits == 0b0110_0000 {
            return "Weekends".to_string();
        }
        if self.bits == 0 {
            return "Never".to_string();
        }
        const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let mut parts = Vec::new();
        for (i, name) in NAMES.iter().enumerate() {
            if self.bits & (1 << i) != 0 {
                parts.push(*name);
            }
        }
        parts.join(" ")
    }
}

// ---------------------------------------------------------------
// Alarm
// ---------------------------------------------------------------

/// A scheduled alarm with hour, minute, day filter, and label.
#[derive(Debug, Clone)]
pub struct Alarm {
    /// Hour (0-23).
    pub hour: u8,
    /// Minute (0-59).
    pub minute: u8,
    /// Whether the alarm is enabled.
    pub enabled: bool,
    /// Human-readable label.
    pub label: String,
    /// Which days of the week the alarm fires.
    pub days: AlarmDays,
}

impl Alarm {
    /// Create a new enabled alarm for every day.
    pub fn new(hour: u8, minute: u8, label: &str) -> Self {
        Self {
            hour: hour.min(23),
            minute: minute.min(59),
            enabled: true,
            label: label.to_string(),
            days: AlarmDays::every_day(),
        }
    }

    /// Check whether this alarm should ring at the given timestamp.
    ///
    /// Returns `true` if the alarm is enabled, the hour and minute
    /// match, and the day of week is in the active set.
    pub fn should_ring(&self, timestamp: u64) -> bool {
        if !self.enabled {
            return false;
        }
        let (_, _, _, h, m, _) = date_parts(timestamp);
        if h != self.hour || m != self.minute {
            return false;
        }
        let dow = day_of_week(timestamp);
        self.days.is_set(dow)
    }

    /// Format the alarm time as "HH:MM".
    pub fn format_time(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    /// Toggle the alarm on/off.
    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }
}
