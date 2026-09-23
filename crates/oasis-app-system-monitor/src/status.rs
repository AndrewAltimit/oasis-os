//! System status snapshot: the data behind the System Monitor gauges.
//!
//! The host publishes a [`SysStatus`] to [`STATUS_PATH`] (see
//! [`crate::probe::HostProbe`]) and the app reads it back, so the app
//! itself never touches platform APIs. The file is plain text, one
//! `key: value` pair per line; a missing key or the value `N/A` means
//! "not available on this platform":
//!
//! ```text
//! platform: Desktop (SDL3)
//! backend: SDL3
//! cpu_percent: 12.5
//! cpu_mhz: 333
//! cpu_max_mhz: 333
//! mem_used_kb: 524288
//! mem_total_kb: 2097152
//! battery_percent: 80
//! battery_state: discharging
//! uptime_secs: 3661
//! ```

use std::fmt::Write as _;

/// VFS path the host publishes the system status to.
pub const STATUS_PATH: &str = "/var/sysmon/status";

/// Battery charge state as published by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryStatus {
    /// Running on battery.
    Discharging,
    /// Plugged in and charging.
    Charging,
    /// Fully charged on external power.
    Full,
    /// No battery present (desktop / wall power).
    NoBattery,
}

impl BatteryStatus {
    /// Wire name used in the status file.
    pub fn as_str(self) -> &'static str {
        match self {
            BatteryStatus::Discharging => "discharging",
            BatteryStatus::Charging => "charging",
            BatteryStatus::Full => "full",
            BatteryStatus::NoBattery => "none",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "discharging" => BatteryStatus::Discharging,
            "charging" => BatteryStatus::Charging,
            "full" => BatteryStatus::Full,
            "none" | "nobattery" | "no battery" => BatteryStatus::NoBattery,
            _ => return None,
        })
    }
}

/// One snapshot of system status. Every field is optional: `None` renders
/// as "N/A".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SysStatus {
    /// Human-readable platform name (e.g. "Desktop (SDL3)", "PSP").
    pub platform: Option<String>,
    /// Active rendering backend (e.g. "SDL3").
    pub backend: Option<String>,
    /// Overall CPU load in percent.
    pub cpu_percent: Option<f32>,
    /// Current CPU clock in MHz.
    pub cpu_mhz: Option<u32>,
    /// Maximum CPU clock in MHz.
    pub cpu_max_mhz: Option<u32>,
    /// Memory in use, in KiB.
    pub mem_used_kb: Option<u64>,
    /// Total memory, in KiB.
    pub mem_total_kb: Option<u64>,
    /// Battery charge in percent.
    pub battery_percent: Option<u8>,
    /// Battery charge state.
    pub battery_state: Option<BatteryStatus>,
    /// Seconds since start-up.
    pub uptime_secs: Option<u64>,
}

/// Parse a value, treating empty / `N/A` / `--` / unparsable as missing.
fn value<T: std::str::FromStr>(v: &str) -> Option<T> {
    let v = v.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("n/a") || v == "--" {
        return None;
    }
    v.trim_end_matches('%').trim().parse().ok()
}

impl SysStatus {
    /// Parse the status file text. Unknown keys and malformed values are
    /// ignored (the field stays `None`).
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let Some((key, v)) = line.split_once(':') else {
                continue;
            };
            let text_value = || {
                let v = v.trim();
                (!v.is_empty() && !v.eq_ignore_ascii_case("n/a")).then(|| v.to_string())
            };
            match key.trim().to_ascii_lowercase().as_str() {
                "platform" => s.platform = text_value(),
                "backend" => s.backend = text_value(),
                "cpu_percent" => s.cpu_percent = value::<f32>(v).filter(|p| p.is_finite()),
                "cpu_mhz" => s.cpu_mhz = value(v),
                "cpu_max_mhz" => s.cpu_max_mhz = value(v),
                "mem_used_kb" => s.mem_used_kb = value(v),
                "mem_total_kb" => s.mem_total_kb = value(v),
                "battery_percent" => s.battery_percent = value(v),
                "battery_state" => s.battery_state = BatteryStatus::parse(v.trim()),
                "uptime_secs" => s.uptime_secs = value(v),
                _ => {},
            }
        }
        s
    }

    /// Serialize to the status file format (`None` fields as `N/A`).
    pub fn to_text(&self) -> String {
        fn put<T: std::fmt::Display>(out: &mut String, key: &str, v: Option<T>) {
            match v {
                Some(v) => {
                    let _ = writeln!(out, "{key}: {v}");
                },
                None => {
                    let _ = writeln!(out, "{key}: N/A");
                },
            }
        }
        let mut out = String::new();
        put(&mut out, "platform", self.platform.as_deref());
        put(&mut out, "backend", self.backend.as_deref());
        put(
            &mut out,
            "cpu_percent",
            self.cpu_percent.map(|p| format!("{p:.1}")),
        );
        put(&mut out, "cpu_mhz", self.cpu_mhz);
        put(&mut out, "cpu_max_mhz", self.cpu_max_mhz);
        put(&mut out, "mem_used_kb", self.mem_used_kb);
        put(&mut out, "mem_total_kb", self.mem_total_kb);
        put(&mut out, "battery_percent", self.battery_percent);
        put(
            &mut out,
            "battery_state",
            self.battery_state.map(BatteryStatus::as_str),
        );
        put(&mut out, "uptime_secs", self.uptime_secs);
        out
    }
}

/// Fill level of a gauge in `0.0..=1.0` from a percentage, clamping
/// out-of-range values; non-finite input is "not available".
pub fn percent_fraction(percent: f32) -> Option<f32> {
    percent
        .is_finite()
        .then(|| (percent / 100.0).clamp(0.0, 1.0))
}

/// Fill level of `used / total` (`None` when the total is unknown / zero).
pub fn ratio_fraction(used: u64, total: u64) -> Option<f32> {
    (total > 0).then(|| (used as f64 / total as f64).clamp(0.0, 1.0) as f32)
}

/// Format seconds as `H:MM:SS` (hours unbounded).
pub fn format_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{h}:{m:02}:{s:02}")
}

/// Format KiB as MiB with no decimals (or KiB below 1 MiB).
pub fn format_kb(kb: u64) -> String {
    if kb >= 1024 {
        format!("{} MB", kb / 1024)
    } else {
        format!("{kb} KB")
    }
}

/// How serious a gauge reading is (drives its value color).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Nothing to flag.
    Normal,
    /// Getting high / low.
    Warning,
    /// Critical.
    Critical,
    /// No data.
    Unavailable,
}

/// One gauge row: label, fill level (`None` = N/A) and value text.
#[derive(Debug, Clone, PartialEq)]
pub struct Gauge {
    /// Row label ("CPU", "Memory", ...).
    pub label: &'static str,
    /// Fill level in `0.0..=1.0`, or `None` when not available.
    pub fraction: Option<f32>,
    /// Value text drawn beside the bar ("N/A ..." when unavailable).
    pub text: String,
    /// Severity of the reading.
    pub level: Level,
}

impl Gauge {
    fn unavailable(label: &'static str, why: &str) -> Self {
        let text = if why.is_empty() {
            "N/A".to_string()
        } else {
            format!("N/A ({why})")
        };
        Self {
            label,
            fraction: None,
            text,
            level: Level::Unavailable,
        }
    }

    /// Whether this gauge has data.
    pub fn is_available(&self) -> bool {
        self.fraction.is_some()
    }
}

/// Length of the uptime gauge's scale: it fills over one day.
const UPTIME_SCALE_SECS: u64 = 24 * 3600;

impl SysStatus {
    /// CPU gauge: load percentage, else clock relative to its maximum.
    pub fn cpu_gauge(&self) -> Gauge {
        if let Some(p) = self.cpu_percent
            && let Some(fraction) = percent_fraction(p)
        {
            let mut text = format!("{:.0}%", fraction * 100.0);
            if let Some(mhz) = self.cpu_mhz.filter(|m| *m > 0) {
                let _ = write!(text, " @ {mhz} MHz");
            }
            let level = match fraction {
                f if f >= 0.9 => Level::Critical,
                f if f >= 0.7 => Level::Warning,
                _ => Level::Normal,
            };
            return Gauge {
                label: "CPU",
                fraction: Some(fraction),
                text,
                level,
            };
        }
        match (self.cpu_mhz, self.cpu_max_mhz) {
            (Some(cur), Some(max)) if cur > 0 && max > 0 => Gauge {
                label: "CPU",
                fraction: ratio_fraction(cur as u64, max as u64),
                text: format!("{cur} / {max} MHz"),
                level: Level::Normal,
            },
            (Some(cur), _) if cur > 0 => Gauge {
                label: "CPU",
                fraction: Some(1.0),
                text: format!("{cur} MHz"),
                level: Level::Normal,
            },
            _ => Gauge::unavailable("CPU", ""),
        }
    }

    /// Memory gauge: used / total.
    pub fn memory_gauge(&self) -> Gauge {
        match (self.mem_used_kb, self.mem_total_kb) {
            (Some(used), Some(total)) if total > 0 => {
                let fraction = ratio_fraction(used, total);
                let pct = fraction.unwrap_or(0.0) * 100.0;
                let level = match pct {
                    p if p >= 90.0 => Level::Critical,
                    p if p >= 75.0 => Level::Warning,
                    _ => Level::Normal,
                };
                Gauge {
                    label: "Memory",
                    fraction,
                    text: format!(
                        "{} / {} ({pct:.0}%)",
                        format_kb(used.min(total)),
                        format_kb(total)
                    ),
                    level,
                }
            },
            _ => Gauge::unavailable("Memory", ""),
        }
    }

    /// Battery gauge: charge percentage plus state.
    pub fn battery_gauge(&self) -> Gauge {
        if self.battery_state == Some(BatteryStatus::NoBattery) {
            return Gauge::unavailable("Battery", "no battery");
        }
        let Some(pct) = self.battery_percent else {
            return Gauge::unavailable("Battery", "");
        };
        let pct = pct.min(100);
        let state = match self.battery_state {
            Some(BatteryStatus::Charging) => " charging",
            Some(BatteryStatus::Full) => " full",
            _ => "",
        };
        let charging = matches!(
            self.battery_state,
            Some(BatteryStatus::Charging | BatteryStatus::Full)
        );
        let level = match pct {
            _ if charging => Level::Normal,
            p if p <= 10 => Level::Critical,
            p if p <= 25 => Level::Warning,
            _ => Level::Normal,
        };
        Gauge {
            label: "Battery",
            fraction: percent_fraction(pct as f32),
            text: format!("{pct}%{state}"),
            level,
        }
    }

    /// Uptime gauge: fills over one day, text is `H:MM:SS`.
    pub fn uptime_gauge(&self) -> Gauge {
        match self.uptime_secs {
            Some(secs) => Gauge {
                label: "Uptime",
                fraction: ratio_fraction(secs.min(UPTIME_SCALE_SECS), UPTIME_SCALE_SECS),
                text: format_uptime(secs),
                level: Level::Normal,
            },
            None => Gauge::unavailable("Uptime", ""),
        }
    }

    /// All gauges in display order.
    pub fn gauges(&self) -> [Gauge; 4] {
        [
            self.cpu_gauge(),
            self.memory_gauge(),
            self.battery_gauge(),
            self.uptime_gauge(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_status() {
        let s = SysStatus::parse(
            "platform: PSP\nbackend: GU\ncpu_percent: 42.5\ncpu_mhz: 333\ncpu_max_mhz: 333\n\
             mem_used_kb: 1024\nmem_total_kb: 4096\nbattery_percent: 80%\n\
             battery_state: charging\nuptime_secs: 3661\n",
        );
        assert_eq!(s.platform.as_deref(), Some("PSP"));
        assert_eq!(s.backend.as_deref(), Some("GU"));
        assert_eq!(s.cpu_percent, Some(42.5));
        assert_eq!(s.cpu_mhz, Some(333));
        assert_eq!(s.mem_used_kb, Some(1024));
        assert_eq!(s.mem_total_kb, Some(4096));
        assert_eq!(s.battery_percent, Some(80));
        assert_eq!(s.battery_state, Some(BatteryStatus::Charging));
        assert_eq!(s.uptime_secs, Some(3661));
    }

    #[test]
    fn parse_tolerates_na_garbage_and_unknown_keys() {
        let s = SysStatus::parse(
            "cpu_percent: N/A\nmem_used_kb: lots\nbattery_percent: --\nwhatever: 1\n\
             no colon here\ncpu_mhz: 300\nplatform: n/a\nbattery_state: exploding\n\
             cpu_percent_typo: 5\n",
        );
        assert_eq!(s.cpu_percent, None);
        assert_eq!(s.mem_used_kb, None);
        assert_eq!(s.battery_percent, None);
        assert_eq!(s.platform, None);
        assert_eq!(s.battery_state, None);
        assert_eq!(s.cpu_mhz, Some(300));
        assert_eq!(SysStatus::parse(""), SysStatus::default());
    }

    #[test]
    fn parse_rejects_non_finite_cpu() {
        assert_eq!(SysStatus::parse("cpu_percent: NaN").cpu_percent, None);
        assert_eq!(SysStatus::parse("cpu_percent: inf").cpu_percent, None);
    }

    #[test]
    fn to_text_round_trips() {
        let s = SysStatus {
            platform: Some("Desktop (SDL3)".into()),
            backend: Some("SDL3".into()),
            cpu_percent: Some(12.5),
            cpu_mhz: None,
            cpu_max_mhz: None,
            mem_used_kb: Some(2048),
            mem_total_kb: Some(8192),
            battery_percent: None,
            battery_state: Some(BatteryStatus::NoBattery),
            uptime_secs: Some(5),
        };
        let text = s.to_text();
        assert!(text.contains("battery_percent: N/A"));
        assert_eq!(SysStatus::parse(&text), s);
    }

    #[test]
    fn fractions_clamp_to_unit_range() {
        assert_eq!(percent_fraction(-20.0), Some(0.0));
        assert_eq!(percent_fraction(50.0), Some(0.5));
        assert_eq!(percent_fraction(250.0), Some(1.0));
        assert_eq!(percent_fraction(f32::NAN), None);
        assert_eq!(ratio_fraction(10, 0), None);
        assert_eq!(ratio_fraction(300, 100), Some(1.0));
        assert_eq!(ratio_fraction(25, 100), Some(0.25));
    }

    #[test]
    fn gauges_clamp_out_of_range_readings() {
        let s = SysStatus {
            cpu_percent: Some(180.0),
            mem_used_kb: Some(9000),
            mem_total_kb: Some(4096),
            battery_percent: Some(250),
            uptime_secs: Some(10 * 24 * 3600),
            ..SysStatus::default()
        };
        for g in s.gauges() {
            let f = g.fraction.expect("available");
            assert!((0.0..=1.0).contains(&f), "{} = {f}", g.label);
        }
        assert_eq!(s.cpu_gauge().text, "100%");
        assert_eq!(s.battery_gauge().text, "100%");
        assert!(s.memory_gauge().text.contains("(100%)"));
        let neg = SysStatus {
            cpu_percent: Some(-5.0),
            ..SysStatus::default()
        };
        assert_eq!(neg.cpu_gauge().fraction, Some(0.0));
    }

    #[test]
    fn empty_status_is_all_na() {
        let s = SysStatus::default();
        for g in s.gauges() {
            assert!(!g.is_available(), "{}", g.label);
            assert!(g.text.starts_with("N/A"), "{}", g.text);
            assert_eq!(g.level, Level::Unavailable);
        }
        let no_batt = SysStatus {
            battery_state: Some(BatteryStatus::NoBattery),
            battery_percent: Some(50),
            ..SysStatus::default()
        };
        assert_eq!(no_batt.battery_gauge().text, "N/A (no battery)");
    }

    #[test]
    fn cpu_falls_back_to_clock() {
        let s = SysStatus {
            cpu_mhz: Some(222),
            cpu_max_mhz: Some(333),
            ..SysStatus::default()
        };
        let g = s.cpu_gauge();
        assert_eq!(g.text, "222 / 333 MHz");
        assert!((g.fraction.expect("clock") - 222.0 / 333.0).abs() < 1e-6);
        // A zero clock means "unknown", not "idle".
        let zero = SysStatus {
            cpu_mhz: Some(0),
            cpu_max_mhz: Some(0),
            ..SysStatus::default()
        };
        assert!(!zero.cpu_gauge().is_available());
    }

    #[test]
    fn levels_flag_high_load_and_low_battery() {
        let s = SysStatus {
            cpu_percent: Some(95.0),
            mem_used_kb: Some(80),
            mem_total_kb: Some(100),
            battery_percent: Some(5),
            battery_state: Some(BatteryStatus::Discharging),
            ..SysStatus::default()
        };
        assert_eq!(s.cpu_gauge().level, Level::Critical);
        assert_eq!(s.memory_gauge().level, Level::Warning);
        assert_eq!(s.battery_gauge().level, Level::Critical);
        let charging = SysStatus {
            battery_state: Some(BatteryStatus::Charging),
            ..s
        };
        assert_eq!(charging.battery_gauge().level, Level::Normal);
        assert_eq!(charging.battery_gauge().text, "5% charging");
    }

    #[test]
    fn uptime_formats_hms() {
        assert_eq!(format_uptime(3661), "1:01:01");
        assert_eq!(format_uptime(0), "0:00:00");
        assert_eq!(format_uptime(100 * 3600), "100:00:00");
    }
}
