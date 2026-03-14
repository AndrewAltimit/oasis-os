//! Platform service commands: power, clock, memory, usb.

#[cfg(test)]
use oasis_types::error::Result;

use crate::interpreter::CommandOutput;

register_commands!(
    register_platform_commands,
    [PowerCmd, ClockCmd, MemoryCmd, UsbCmd]
);

// ---------------------------------------------------------------------------
// power
// ---------------------------------------------------------------------------

define_command!(
    PowerCmd,
    "power",
    "Show power/battery status",
    "power",
    "system",
    |_args, env| {
        let Some(power) = env.power else {
            return Ok(CommandOutput::Text(
                "power: no platform service available".to_string(),
            ));
        };
        let info = power.power_info()?;
        let mut lines = Vec::new();
        lines.push(format!("State: {:?}", info.state));
        match info.battery_percent {
            Some(pct) => lines.push(format!("Battery: {pct}%")),
            None => lines.push("Battery: N/A".to_string()),
        }
        if let Some(mins) = info.battery_minutes {
            lines.push(format!("Remaining: {mins} min"));
        }
        if info.cpu.current_mhz > 0 {
            lines.push(format!(
                "CPU: {} MHz (max {} MHz)",
                info.cpu.current_mhz, info.cpu.max_mhz
            ));
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }
);

// ---------------------------------------------------------------------------
// clock
// ---------------------------------------------------------------------------

define_command!(
    ClockCmd,
    "clock",
    "Show current time and uptime",
    "clock",
    "system",
    |_args, env| {
        let Some(time) = env.time else {
            return Ok(CommandOutput::Text(
                "clock: no platform service available".to_string(),
            ));
        };
        let now = time.now()?;
        let uptime = time.uptime_secs()?;
        let hours = uptime / 3600;
        let mins = (uptime % 3600) / 60;
        let secs = uptime % 60;
        let mut lines = Vec::new();
        lines.push(format!("Time: {now}"));
        lines.push(format!("Uptime: {hours}h {mins}m {secs}s"));
        Ok(CommandOutput::Text(lines.join("\n")))
    }
);

// ---------------------------------------------------------------------------
// memory
// ---------------------------------------------------------------------------

define_command!(
    MemoryCmd,
    "memory",
    "Show memory usage",
    "memory",
    "system",
    |_args, _env| {
        // On desktop/Pi, report process RSS if /proc/self/status is readable.
        // On PSP, this would query sceKernelTotalFreeMemSize().
        let mut lines = Vec::new();
        lines.push("OASIS_OS memory info".to_string());
        #[cfg(target_os = "linux")]
        {
            if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
                for line in status.lines() {
                    if line.starts_with("VmRSS:") || line.starts_with("VmSize:") {
                        lines.push(line.trim().to_string());
                    }
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            lines.push("(detailed memory info not available on this platform)".to_string());
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }
);

// ---------------------------------------------------------------------------
// usb
// ---------------------------------------------------------------------------

define_command!(
    UsbCmd,
    "usb",
    "Show USB status",
    "usb",
    "system",
    |_args, env| {
        let Some(usb) = env.usb else {
            return Ok(CommandOutput::Text(
                "usb: no platform service available".to_string(),
            ));
        };
        let state = usb.usb_state()?;
        Ok(CommandOutput::Text(format!("USB: {state}")))
    }
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::assert_text;
    use crate::{CommandOutput, CommandRegistry, Environment};
    use oasis_vfs::MemoryVfs;

    fn exec(reg: &CommandRegistry, vfs: &mut MemoryVfs, line: &str) -> Result<CommandOutput> {
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        reg.execute(line, &mut env)
    }

    #[test]
    fn power_no_platform() {
        let mut reg = CommandRegistry::new();
        register_platform_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        assert!(assert_text!(exec(&reg, &mut vfs, "power").unwrap()).contains("no platform"));
    }

    #[test]
    fn power_with_platform() {
        let mut reg = CommandRegistry::new();
        register_platform_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let platform = oasis_platform::DesktopPlatform::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: Some(&platform),
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        assert!(assert_text!(reg.execute("power", &mut env).unwrap()).contains("NoBattery"));
    }

    #[test]
    fn clock_no_platform() {
        let mut reg = CommandRegistry::new();
        register_platform_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        assert!(assert_text!(exec(&reg, &mut vfs, "clock").unwrap()).contains("no platform"));
    }

    #[test]
    fn clock_with_platform() {
        let mut reg = CommandRegistry::new();
        register_platform_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let platform = oasis_platform::DesktopPlatform::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: Some(&platform),
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        let s = assert_text!(reg.execute("clock", &mut env).unwrap());
        assert!(s.contains("Time:"));
        assert!(s.contains("Uptime:"));
    }

    #[test]
    fn memory_shows_info() {
        let mut reg = CommandRegistry::new();
        register_platform_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        assert!(assert_text!(exec(&reg, &mut vfs, "memory").unwrap()).contains("OASIS_OS memory"));
    }

    #[test]
    fn usb_no_platform() {
        let mut reg = CommandRegistry::new();
        register_platform_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        assert!(assert_text!(exec(&reg, &mut vfs, "usb").unwrap()).contains("no platform"));
    }

    #[test]
    fn usb_with_platform() {
        let mut reg = CommandRegistry::new();
        register_platform_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let platform = oasis_platform::DesktopPlatform::new();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: Some(&platform),
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        assert!(assert_text!(reg.execute("usb", &mut env).unwrap()).contains("unsupported"));
    }
}
