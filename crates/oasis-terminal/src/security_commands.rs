//! Security and permissions commands: chmod, chown, passwd, audit.

use oasis_types::error::{OasisError, Result};

use crate::interpreter::{Command, CommandOutput, Environment, resolve_path};

/// Append an entry to the VFS audit log.
fn audit_log(vfs: &mut dyn oasis_vfs::Vfs, entry: &str) {
    let log_path = "/var/log/audit.log";
    // Ensure parent dirs exist.
    let _ = vfs.mkdir("/var");
    let _ = vfs.mkdir("/var/log");
    let existing = vfs.read(log_path).unwrap_or_default();
    let mut content = String::from_utf8_lossy(&existing).into_owned();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(entry);
    content.push('\n');
    let _ = vfs.write(log_path, content.as_bytes());
}

// ---------------------------------------------------------------------------
// chmod
// ---------------------------------------------------------------------------

struct ChmodCmd;
impl Command for ChmodCmd {
    fn name(&self) -> &str {
        "chmod"
    }
    fn description(&self) -> &str {
        "Set file permissions (VFS metadata)"
    }
    fn usage(&self) -> &str {
        "chmod <mode> <file>"
    }
    fn category(&self) -> &str {
        "security"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.len() < 2 {
            return Err(OasisError::Command("usage: chmod <mode> <file>".into()));
        }
        let mode_str = args[0];
        let path = resolve_path(&env.cwd, args[1]);
        let mode = u16::from_str_radix(mode_str, 8)
            .map_err(|_| OasisError::Command(format!("invalid octal mode: {mode_str}").into()))?;
        let mut perms = env.vfs.get_permissions(&path)?;
        perms.mode = mode;
        env.vfs.set_permissions(&path, perms)?;
        audit_log(env.vfs, &format!("chmod {mode_str} {path}"));
        Ok(CommandOutput::Text(format!(
            "Set permissions on {path}: {mode_str}"
        )))
    }
}

// ---------------------------------------------------------------------------
// chown
// ---------------------------------------------------------------------------

struct ChownCmd;
impl Command for ChownCmd {
    fn name(&self) -> &str {
        "chown"
    }
    fn description(&self) -> &str {
        "Set file owner (VFS metadata)"
    }
    fn usage(&self) -> &str {
        "chown <owner> <file>"
    }
    fn category(&self) -> &str {
        "security"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.len() < 2 {
            return Err(OasisError::Command("usage: chown <owner> <file>".into()));
        }
        let owner = args[0];
        let path = resolve_path(&env.cwd, args[1]);
        let mut perms = env.vfs.get_permissions(&path)?;
        perms.owner = owner.to_string();
        env.vfs.set_permissions(&path, perms)?;
        audit_log(env.vfs, &format!("chown {owner} {path}"));
        Ok(CommandOutput::Text(format!("Set owner of {path}: {owner}")))
    }
}

// ---------------------------------------------------------------------------
// passwd
// ---------------------------------------------------------------------------

struct PasswdCmd;
impl Command for PasswdCmd {
    fn name(&self) -> &str {
        "passwd"
    }
    fn description(&self) -> &str {
        "Change user password (simulated)"
    }
    fn usage(&self) -> &str {
        "passwd [user]"
    }
    fn category(&self) -> &str {
        "security"
    }
    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        let user = args.first().copied().unwrap_or("oasis");
        Ok(CommandOutput::Text(format!(
            "Password change for user '{user}' -- \
             (simulated: single-user system, no real password store)"
        )))
    }
}

// ---------------------------------------------------------------------------
// audit
// ---------------------------------------------------------------------------

struct AuditCmd;
impl Command for AuditCmd {
    fn name(&self) -> &str {
        "audit"
    }
    fn description(&self) -> &str {
        "Show security audit log"
    }
    fn usage(&self) -> &str {
        "audit [show|clear]"
    }
    fn category(&self) -> &str {
        "security"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("show");
        let log_path = "/var/log/audit.log";

        match subcmd {
            "show" => {
                if env.vfs.exists(log_path) {
                    let data = env.vfs.read(log_path)?;
                    let text = String::from_utf8_lossy(&data).into_owned();
                    if text.trim().is_empty() {
                        Ok(CommandOutput::Text("(audit log is empty)".to_string()))
                    } else {
                        Ok(CommandOutput::Text(text))
                    }
                } else {
                    Ok(CommandOutput::Text("(no audit log found)".to_string()))
                }
            },
            "clear" => {
                if env.vfs.exists(log_path) {
                    env.vfs.write(log_path, &[])?;
                    Ok(CommandOutput::Text("Audit log cleared.".to_string()))
                } else {
                    Ok(CommandOutput::Text("(no audit log to clear)".to_string()))
                }
            },
            _ => Err(OasisError::Command(
                format!("unknown subcommand: {subcmd}").into(),
            )),
        }
    }
}

register_commands!(
    register_security_commands,
    [ChmodCmd, ChownCmd, PasswdCmd, AuditCmd]
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::assert_text;
    use crate::{CommandRegistry, Environment};
    use oasis_vfs::{MemoryVfs, Vfs};

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
    fn chmod_sets_permissions() {
        let mut reg = CommandRegistry::new();
        register_security_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.write("/test.txt", b"data").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "chmod 755 /test.txt").unwrap());
        assert!(s.contains("755"));
        let perms = vfs.get_permissions("/test.txt").unwrap();
        assert_eq!(perms.mode, 0o755);
    }

    #[test]
    fn chmod_enforces_readonly() {
        let mut reg = CommandRegistry::new();
        register_security_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.write("/test.txt", b"data").unwrap();
        exec(&reg, &mut vfs, "chmod 444 /test.txt").unwrap();
        assert!(vfs.write("/test.txt", b"new data").is_err());
        assert_eq!(vfs.read("/test.txt").unwrap(), b"data");
    }

    #[test]
    fn chown_sets_owner() {
        let mut reg = CommandRegistry::new();
        register_security_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.write("/test.txt", b"data").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "chown root /test.txt").unwrap());
        assert!(s.contains("root"));
        let perms = vfs.get_permissions("/test.txt").unwrap();
        assert_eq!(perms.owner, "root");
    }

    #[test]
    fn passwd_simulated() {
        let mut reg = CommandRegistry::new();
        register_security_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let s = assert_text!(exec(&reg, &mut vfs, "passwd").unwrap());
        assert!(s.contains("simulated"));
    }

    #[test]
    fn audit_no_log() {
        let mut reg = CommandRegistry::new();
        register_security_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let s = assert_text!(exec(&reg, &mut vfs, "audit").unwrap());
        assert!(s.contains("no audit log"));
    }

    #[test]
    fn audit_show_log() {
        let mut reg = CommandRegistry::new();
        register_security_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/var").unwrap();
        vfs.mkdir("/var/log").unwrap();
        vfs.write("/var/log/audit.log", b"event: login at 12:00")
            .unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "audit show").unwrap());
        assert!(s.contains("login"));
    }

    #[test]
    fn audit_clear() {
        let mut reg = CommandRegistry::new();
        register_security_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/var").unwrap();
        vfs.mkdir("/var/log").unwrap();
        vfs.write("/var/log/audit.log", b"old data").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "audit clear").unwrap());
        assert!(s.contains("cleared"));
        let data = vfs.read("/var/log/audit.log").unwrap();
        assert!(data.is_empty());
    }

    #[test]
    fn chmod_chown_create_audit_entries() {
        let mut reg = CommandRegistry::new();
        register_security_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.write("/a.txt", b"hello").unwrap();
        vfs.write("/b.txt", b"world").unwrap();

        // No audit log yet.
        assert!(!vfs.exists("/var/log/audit.log"));

        // chmod should create an audit entry.
        exec(&reg, &mut vfs, "chmod 755 /a.txt").unwrap();
        let log = String::from_utf8(vfs.read("/var/log/audit.log").unwrap()).unwrap();
        assert!(
            log.contains("chmod 755 /a.txt"),
            "missing chmod entry: {log}"
        );

        // chown should append an audit entry.
        exec(&reg, &mut vfs, "chown root /b.txt").unwrap();
        let log = String::from_utf8(vfs.read("/var/log/audit.log").unwrap()).unwrap();
        assert!(log.contains("chmod 755 /a.txt"), "chmod entry lost: {log}");
        assert!(
            log.contains("chown root /b.txt"),
            "missing chown entry: {log}"
        );
    }
}
