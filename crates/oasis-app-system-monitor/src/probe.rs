//! Host-side probe that fills a [`SysStatus`] and publishes it to the VFS.
//!
//! Hosts own the platform services, so they sample them (plus, on Linux,
//! `/proc/stat` and `/proc/meminfo`) about once a second and write the
//! result to [`STATUS_PATH`], where the System Monitor app picks it up:
//!
//! ```ignore
//! let mut probe = HostProbe::new("Desktop (SDL3)", "SDL3");
//! // once a second:
//! probe.publish(&mut vfs, Some(&platform), Some(&platform))?;
//! ```

use oasis_platform::{BatteryState, PowerService, TimeService};
use oasis_vfs::Vfs;

use crate::status::{BatteryStatus, STATUS_PATH, SysStatus};

/// Samples platform services and OS counters into [`SysStatus`] snapshots.
///
/// Keeps the previous CPU counters so load is reported over the interval
/// between two samples (the first sample has no CPU load yet).
#[derive(Debug, Clone)]
pub struct HostProbe {
    platform: String,
    backend: String,
    prev_cpu: Option<CpuTimes>,
}

/// Aggregate CPU time counters (`busy`, `total`) in clock ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    /// Ticks spent doing work (everything except idle + iowait).
    pub busy: u64,
    /// All ticks.
    pub total: u64,
}

impl HostProbe {
    /// Create a probe that reports `platform` / `backend` names.
    pub fn new(platform: &str, backend: &str) -> Self {
        Self {
            platform: platform.to_string(),
            backend: backend.to_string(),
            prev_cpu: None,
        }
    }

    /// Take one snapshot from the given services and the OS counters.
    pub fn sample(
        &mut self,
        power: Option<&dyn PowerService>,
        time: Option<&dyn TimeService>,
    ) -> SysStatus {
        let mut s = SysStatus {
            platform: Some(self.platform.clone()),
            backend: Some(self.backend.clone()),
            ..SysStatus::default()
        };
        if let Some(info) = power.and_then(|p| p.power_info().ok()) {
            s.battery_state = Some(match info.state {
                BatteryState::Discharging => BatteryStatus::Discharging,
                BatteryState::Charging => BatteryStatus::Charging,
                BatteryState::Full => BatteryStatus::Full,
                BatteryState::NoBattery => BatteryStatus::NoBattery,
            });
            s.battery_percent = info.battery_percent;
            s.cpu_mhz = (info.cpu.current_mhz > 0).then_some(info.cpu.current_mhz);
            s.cpu_max_mhz = (info.cpu.max_mhz > 0).then_some(info.cpu.max_mhz);
        }
        s.uptime_secs = time.and_then(|t| t.uptime_secs().ok());

        if let Some(now) = os::cpu_times() {
            s.cpu_percent = self.prev_cpu.and_then(|prev| cpu_load(prev, now));
            self.prev_cpu = Some(now);
        }
        if let Some((used, total)) = os::memory_kb() {
            s.mem_used_kb = Some(used);
            s.mem_total_kb = Some(total);
        }
        s
    }

    /// Sample and write the snapshot to [`STATUS_PATH`] (creating
    /// `/var/sysmon` if needed).
    pub fn publish(
        &mut self,
        vfs: &mut dyn Vfs,
        power: Option<&dyn PowerService>,
        time: Option<&dyn TimeService>,
    ) -> oasis_types::error::Result<()> {
        let status = self.sample(power, time);
        publish_status(vfs, &status)
    }
}

/// Write `status` to [`STATUS_PATH`], creating its folder if needed.
pub fn publish_status(vfs: &mut dyn Vfs, status: &SysStatus) -> oasis_types::error::Result<()> {
    let mut cur = String::new();
    let dir = STATUS_PATH.rsplit_once('/').map_or("", |(d, _)| d);
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        cur.push('/');
        cur.push_str(part);
        if !vfs.exists(&cur) {
            vfs.mkdir(&cur)?;
        }
    }
    vfs.write(STATUS_PATH, status.to_text().as_bytes())
}

/// CPU load in percent between two counter samples.
pub fn cpu_load(prev: CpuTimes, now: CpuTimes) -> Option<f32> {
    let total = now.total.checked_sub(prev.total)?;
    let busy = now.busy.checked_sub(prev.busy)?;
    (total > 0).then(|| (busy as f64 * 100.0 / total as f64).clamp(0.0, 100.0) as f32)
}

/// Parse the aggregate `cpu` line of Linux `/proc/stat`.
pub fn parse_proc_stat(text: &str) -> Option<CpuTimes> {
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map_while(|f| f.parse().ok())
        .collect();
    if fields.len() < 4 {
        return None;
    }
    // user nice system idle iowait irq softirq steal (guest* are already
    // counted in user/nice).
    let total: u64 = fields.iter().take(8).sum();
    let idle = fields[3] + fields.get(4).copied().unwrap_or(0);
    Some(CpuTimes {
        busy: total.saturating_sub(idle),
        total,
    })
}

/// Parse Linux `/proc/meminfo` into `(used_kb, total_kb)`.
///
/// "Used" is `MemTotal - MemAvailable` (falling back to `MemFree` on
/// kernels without `MemAvailable`).
pub fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let field = |name: &str| {
        text.lines().find_map(|l| {
            l.strip_prefix(name)?
                .strip_prefix(':')?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
    };
    let total = field("MemTotal")?;
    let available = field("MemAvailable").or_else(|| field("MemFree"))?;
    Some((total.saturating_sub(available), total))
}

#[cfg(target_os = "linux")]
mod os {
    use super::{CpuTimes, parse_meminfo, parse_proc_stat};

    pub(super) fn cpu_times() -> Option<CpuTimes> {
        parse_proc_stat(&std::fs::read_to_string("/proc/stat").ok()?)
    }

    pub(super) fn memory_kb() -> Option<(u64, u64)> {
        parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
    }
}

/// Non-Linux hosts have no OS counters here: CPU load and memory stay N/A
/// unless the platform services provide them.
#[cfg(not(target_os = "linux"))]
mod os {
    use super::CpuTimes;

    pub(super) fn cpu_times() -> Option<CpuTimes> {
        None
    }

    pub(super) fn memory_kb() -> Option<(u64, u64)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_platform::{CpuClock, PowerInfo};
    use oasis_vfs::MemoryVfs;

    struct Power(PowerInfo);
    impl PowerService for Power {
        fn power_info(&self) -> oasis_types::error::Result<PowerInfo> {
            Ok(self.0.clone())
        }
    }

    struct Clock(u64);
    impl TimeService for Clock {
        fn now(&self) -> oasis_types::error::Result<oasis_platform::SystemTime> {
            Err(oasis_types::error::OasisError::Platform("no clock".into()))
        }
        fn uptime_secs(&self) -> oasis_types::error::Result<u64> {
            Ok(self.0)
        }
    }

    #[test]
    fn proc_stat_parsing_and_load() {
        let a =
            parse_proc_stat("cpu  100 0 100 700 100 0 0 0 0 0\ncpu0 1 2 3 4\n").expect("cpu line");
        assert_eq!(a.total, 1000);
        assert_eq!(a.busy, 200);
        let b = parse_proc_stat("cpu  200 0 200 1300 100 0 0 0 0 0\n").expect("cpu line");
        // 200 busy ticks out of 800.
        assert_eq!(cpu_load(a, b), Some(25.0));
        // Counters going backwards (reset) give no reading.
        assert_eq!(cpu_load(b, a), None);
        assert_eq!(cpu_load(a, a), None);
        assert!(parse_proc_stat("intr 1 2 3\n").is_none());
        assert!(parse_proc_stat("cpu  1 2\n").is_none());
    }

    #[test]
    fn meminfo_parsing() {
        let text = "MemTotal:       2048000 kB\nMemFree:  100 kB\nMemAvailable:   512000 kB\n";
        assert_eq!(parse_meminfo(text), Some((1_536_000, 2_048_000)));
        let old = "MemTotal: 1000 kB\nMemFree: 250 kB\n";
        assert_eq!(parse_meminfo(old), Some((750, 1000)));
        assert_eq!(parse_meminfo("MemFree: 5 kB\n"), None);
    }

    #[test]
    fn sample_maps_platform_services() {
        let power = Power(PowerInfo {
            battery_percent: Some(64),
            battery_minutes: None,
            state: BatteryState::Discharging,
            cpu: CpuClock {
                current_mhz: 222,
                max_mhz: 333,
            },
        });
        let mut probe = HostProbe::new("PSP", "GU");
        let s = probe.sample(Some(&power), Some(&Clock(42)));
        assert_eq!(s.platform.as_deref(), Some("PSP"));
        assert_eq!(s.backend.as_deref(), Some("GU"));
        assert_eq!(s.battery_percent, Some(64));
        assert_eq!(s.battery_state, Some(BatteryStatus::Discharging));
        assert_eq!(s.cpu_mhz, Some(222));
        assert_eq!(s.cpu_max_mhz, Some(333));
        assert_eq!(s.uptime_secs, Some(42));
        // First sample never has a CPU load (needs two counter readings).
        assert_eq!(s.cpu_percent, None);
    }

    #[test]
    fn sample_without_services_leaves_na() {
        let mut probe = HostProbe::new("Test", "Test");
        let s = probe.sample(None, None);
        assert_eq!(s.uptime_secs, None);
        assert_eq!(s.battery_state, None);
        assert_eq!(s.cpu_mhz, None);
    }

    #[test]
    fn publish_writes_parseable_status() {
        let mut vfs = MemoryVfs::new();
        let mut probe = HostProbe::new("Desktop (SDL3)", "SDL3");
        probe
            .publish(&mut vfs, None, Some(&Clock(7)))
            .expect("publish");
        let data = vfs.read(STATUS_PATH).expect("status file");
        let s = SysStatus::parse(&String::from_utf8_lossy(&data));
        assert_eq!(s.platform.as_deref(), Some("Desktop (SDL3)"));
        assert_eq!(s.uptime_secs, Some(7));
        // Republishing overwrites in place.
        probe
            .publish(&mut vfs, None, Some(&Clock(8)))
            .expect("republish");
        let data = vfs.read(STATUS_PATH).expect("status file");
        assert_eq!(
            SysStatus::parse(&String::from_utf8_lossy(&data)).uptime_secs,
            Some(8)
        );
    }
}
