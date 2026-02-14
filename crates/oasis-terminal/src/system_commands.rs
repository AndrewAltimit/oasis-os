//! System and process commands: uptime, df, whoami, hostname, date, sleep.

use oasis_types::error::{OasisError, Result};

use crate::interpreter::{Command, CommandOutput, Environment};

// ---------------------------------------------------------------------------
// uptime
// ---------------------------------------------------------------------------

struct UptimeCmd;
impl Command for UptimeCmd {
    fn name(&self) -> &str {
        "uptime"
    }
    fn description(&self) -> &str {
        "Show system uptime"
    }
    fn usage(&self) -> &str {
        "uptime"
    }
    fn category(&self) -> &str {
        "system"
    }
    fn execute(&self, _args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        if let Some(time) = env.time {
            let secs = time.uptime_secs()?;
            let days = secs / 86400;
            let hours = (secs % 86400) / 3600;
            let mins = (secs % 3600) / 60;
            let s = secs % 60;
            if days > 0 {
                Ok(CommandOutput::Text(format!(
                    "up {days} day(s), {hours:02}:{mins:02}:{s:02}"
                )))
            } else {
                Ok(CommandOutput::Text(format!(
                    "up {hours:02}:{mins:02}:{s:02}"
                )))
            }
        } else {
            Ok(CommandOutput::Text(
                "uptime: no time service available".to_string(),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// df
// ---------------------------------------------------------------------------

struct DfCmd;
impl Command for DfCmd {
    fn name(&self) -> &str {
        "df"
    }
    fn description(&self) -> &str {
        "Show VFS filesystem usage"
    }
    fn usage(&self) -> &str {
        "df"
    }
    fn category(&self) -> &str {
        "system"
    }
    fn execute(&self, _args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        // Count entries recursively to approximate usage.
        let (dirs, files, total_bytes) = count_vfs_recursive(env, "/", 0)?;
        let mut lines = Vec::new();
        lines.push("Filesystem      Files  Dirs  Size".to_string());
        lines.push(format!(
            "vfs             {files:>5}  {dirs:>4}  {total_bytes}B"
        ));
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

/// Maximum recursion depth for VFS traversal to prevent stack overflow.
const MAX_DEPTH: usize = 64;

fn count_vfs_recursive(
    env: &mut Environment<'_>,
    dir: &str,
    depth: usize,
) -> Result<(u32, u32, u64)> {
    if depth >= MAX_DEPTH {
        return Ok((0, 0, 0));
    }
    let entries = env.vfs.readdir(dir)?;
    let mut dirs = 0u32;
    let mut files = 0u32;
    let mut bytes = 0u64;
    for entry in &entries {
        let path = if dir == "/" {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", dir, entry.name)
        };
        if entry.kind == oasis_vfs::EntryKind::Directory {
            dirs += 1;
            let (d, f, b) = count_vfs_recursive(env, &path, depth + 1)?;
            dirs += d;
            files += f;
            bytes += b;
        } else {
            files += 1;
            bytes += entry.size;
        }
    }
    Ok((dirs, files, bytes))
}

// ---------------------------------------------------------------------------
// whoami
// ---------------------------------------------------------------------------

struct WhoamiCmd;
impl Command for WhoamiCmd {
    fn name(&self) -> &str {
        "whoami"
    }
    fn description(&self) -> &str {
        "Print current user name"
    }
    fn usage(&self) -> &str {
        "whoami"
    }
    fn category(&self) -> &str {
        "system"
    }
    fn execute(&self, _args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        Ok(CommandOutput::Text("oasis".to_string()))
    }
}

// ---------------------------------------------------------------------------
// hostname
// ---------------------------------------------------------------------------

struct HostnameCmd;
impl Command for HostnameCmd {
    fn name(&self) -> &str {
        "hostname"
    }
    fn description(&self) -> &str {
        "Print system hostname"
    }
    fn usage(&self) -> &str {
        "hostname"
    }
    fn category(&self) -> &str {
        "system"
    }
    fn execute(&self, _args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        Ok(CommandOutput::Text("oasis-os".to_string()))
    }
}

// ---------------------------------------------------------------------------
// date
// ---------------------------------------------------------------------------

struct DateCmd;
impl Command for DateCmd {
    fn name(&self) -> &str {
        "date"
    }
    fn description(&self) -> &str {
        "Print current date and time"
    }
    fn usage(&self) -> &str {
        "date"
    }
    fn category(&self) -> &str {
        "system"
    }
    fn execute(&self, _args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        if let Some(time) = env.time {
            let now = time.now()?;
            Ok(CommandOutput::Text(now.to_string()))
        } else {
            Ok(CommandOutput::Text(
                "date: no time service available".to_string(),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// sleep
// ---------------------------------------------------------------------------

struct SleepCmd;
impl Command for SleepCmd {
    fn name(&self) -> &str {
        "sleep"
    }
    fn description(&self) -> &str {
        "Pause execution (simulated)"
    }
    fn usage(&self) -> &str {
        "sleep <seconds>"
    }
    fn category(&self) -> &str {
        "system"
    }
    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: sleep <seconds>".to_string()));
        }
        let secs: f64 = args[0]
            .parse()
            .map_err(|_| OasisError::Command("invalid number".to_string()))?;
        // In a real system this would block; in VFS-only mode we just report.
        Ok(CommandOutput::Text(format!(
            "(slept {secs:.1}s -- simulated)"
        )))
    }
}

/// Register system commands.
pub fn register_system_commands(reg: &mut crate::CommandRegistry) {
    reg.register(Box::new(UptimeCmd));
    reg.register(Box::new(DfCmd));
    reg.register(Box::new(WhoamiCmd));
    reg.register(Box::new(HostnameCmd));
    reg.register(Box::new(DateCmd));
    reg.register(Box::new(SleepCmd));
}

#[cfg(test)]
mod tests {
    use super::*;
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
        };
        reg.execute(line, &mut env)
    }

    #[test]
    fn whoami_returns_oasis() {
        let mut reg = CommandRegistry::new();
        register_system_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "whoami").unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "oasis"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn hostname_returns_oasis_os() {
        let mut reg = CommandRegistry::new();
        register_system_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "hostname").unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "oasis-os"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn uptime_no_service() {
        let mut reg = CommandRegistry::new();
        register_system_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "uptime").unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("no time service")),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn df_basic() {
        let mut reg = CommandRegistry::new();
        register_system_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "df").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("Filesystem"));
                assert!(s.contains("vfs"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn date_no_service() {
        let mut reg = CommandRegistry::new();
        register_system_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "date").unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("no time service")),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn sleep_simulated() {
        let mut reg = CommandRegistry::new();
        register_system_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "sleep 1").unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("simulated")),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn sleep_no_args() {
        let mut reg = CommandRegistry::new();
        register_system_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        assert!(exec(&reg, &mut vfs, "sleep").is_err());
    }
}
