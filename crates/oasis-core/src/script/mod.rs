//! Scripting engine for command automation.
//!
//! Reads script files from the VFS and runs them through the terminal's
//! shared script engine (`#` comments, blank lines skipped, full control
//! flow). Provides the `cron` and `startup` commands; `run <path>` is a
//! shell builtin in `oasis-terminal` (`CommandRegistry::execute_run`).
//! Cron jobs run when a host calls [`CronScheduler::tick`] (see [`cron`]).

pub mod cron;

pub use cron::{CronCmd, CronEntry, CronScheduler};

use crate::error::{OasisError, Result};
use crate::terminal::{Command, CommandOutput, CommandRegistry, Environment};

/// VFS paths for scripting configuration.
pub const STARTUP_SCRIPT_PATH: &str = "/etc/startup.sh";
/// Shell profile seeded by `populate_profile`; run first at boot.
pub const PROFILE_PATH: &str = "/home/.profile";
/// Directory of cron job entries, one `<interval> <script>` file per job.
pub const CRON_DIR: &str = "/etc/cron";

/// Execute a script from VFS, returning collected output.
///
/// Runs through the terminal's shared script engine
/// ([`CommandRegistry::run_script_source`]), so startup and cron scripts
/// support the same control flow (`if`/`elif`, loops, `case`) as `run`.
pub fn run_script(
    path: &str,
    registry: &CommandRegistry,
    env: &mut Environment<'_>,
) -> Result<Vec<String>> {
    let data = env.vfs.read(path)?;
    let source = String::from_utf8_lossy(&data);
    registry.run_script_source(&source, env)
}

/// Run the boot scripts once a host's command registry is ready: the shell
/// profile [`PROFILE_PATH`] (variables it `set`s land in `registry`), then
/// the startup script [`STARTUP_SCRIPT_PATH`] managed by `startup`.
///
/// Each script runs only if it exists. A script that fails (unreadable,
/// syntax error) contributes one `error: <path>: ...` line and does not
/// stop the other one. Returns the combined output lines for the host to
/// show in its terminal scrollback.
pub fn run_boot_scripts(registry: &CommandRegistry, env: &mut Environment<'_>) -> Vec<String> {
    let mut lines = Vec::new();
    for path in [PROFILE_PATH, STARTUP_SCRIPT_PATH] {
        if !env.vfs.exists(path) {
            continue;
        }
        match run_script(path, registry, env) {
            Ok(out) => lines.extend(out),
            Err(e) => lines.push(format!("error: {path}: {e}")),
        }
    }
    lines
}

// ---------------------------------------------------------------------------
// Terminal commands
// ---------------------------------------------------------------------------

/// `startup` -- configure the startup script.
pub struct StartupCmd;

impl Command for StartupCmd {
    fn name(&self) -> &str {
        "startup"
    }
    fn description(&self) -> &str {
        "Configure the startup script"
    }
    fn usage(&self) -> &str {
        "startup [show|set <path>|clear]"
    }
    fn category(&self) -> &str {
        "scripting"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("show");

        match subcmd {
            "show" => {
                if env.vfs.exists(STARTUP_SCRIPT_PATH) {
                    let data = env.vfs.read(STARTUP_SCRIPT_PATH)?;
                    let text = String::from_utf8_lossy(&data).into_owned();
                    if text.trim().is_empty() {
                        Ok(CommandOutput::Text("(startup script is empty)".to_string()))
                    } else {
                        Ok(CommandOutput::Text(format!(
                            "Startup script ({STARTUP_SCRIPT_PATH}):\n{text}"
                        )))
                    }
                } else {
                    Ok(CommandOutput::Text(
                        "(no startup script configured)".to_string(),
                    ))
                }
            },
            "set" => {
                let path = args
                    .get(1)
                    .copied()
                    .ok_or_else(|| OasisError::Command("usage: startup set <path>".into()))?;
                // Copy the script to the startup path.
                if !env.vfs.exists(path) {
                    return Err(OasisError::Command(
                        format!("file not found: {path}").into(),
                    ));
                }
                let data = env.vfs.read(path)?;
                env.vfs.write(STARTUP_SCRIPT_PATH, &data)?;
                Ok(CommandOutput::Text(format!(
                    "Startup script set to: {path}"
                )))
            },
            "clear" => {
                if env.vfs.exists(STARTUP_SCRIPT_PATH) {
                    env.vfs.remove(STARTUP_SCRIPT_PATH)?;
                }
                Ok(CommandOutput::Text("Startup script cleared".to_string()))
            },
            _ => Err(OasisError::Command(
                format!("unknown subcommand: {subcmd}\nusage: {}", self.usage()).into(),
            )),
        }
    }
}

/// Register scripting commands (`cron`, `startup`).
pub fn register_script_commands(reg: &mut CommandRegistry) {
    reg.register(Box::new(CronCmd));
    reg.register(Box::new(StartupCmd));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::CommandRegistry;
    use crate::vfs::{MemoryVfs, Vfs};

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        crate::terminal::register_builtins(&mut reg);
        register_script_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/etc").unwrap();
        vfs.mkdir("/tmp").unwrap();
        vfs.mkdir("/home").unwrap();
        (reg, vfs)
    }

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
    fn run_script_executes_commands() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/test.sh", b"# test script\necho hello\necho world")
            .unwrap();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,

            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        let output = run_script("/tmp/test.sh", &reg, &mut env).unwrap();
        assert_eq!(output, vec!["hello", "world"]);
    }

    #[test]
    fn run_script_reports_errors() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/bad.sh", b"nonexistent_command\necho ok")
            .unwrap();
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,

            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        let output = run_script("/tmp/bad.sh", &reg, &mut env).unwrap();
        assert!(output[0].contains("error at line 1"));
        assert_eq!(output[1], "ok");
    }

    #[test]
    fn run_cmd_missing_path() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "run").is_err());
    }

    #[test]
    fn run_cmd_file_not_found() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "run /tmp/nope.sh").is_err());
    }

    #[test]
    fn run_cmd_executes_script() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/demo.sh", b"echo hello\necho world\n")
            .unwrap();
        let __out = exec(&reg, &mut vfs, "run /tmp/demo.sh").unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected CommandOutput::Text, got {__out:?}");
        };
        assert!(s.contains("hello"));
        assert!(s.contains("world"));
    }

    fn text(out: Result<CommandOutput>) -> String {
        match out {
            Ok(CommandOutput::Text(s)) => s,
            other => panic!("expected CommandOutput::Text, got {other:?}"),
        }
    }

    #[test]
    fn cron_list_empty() {
        let (reg, mut vfs) = setup();
        assert!(text(exec(&reg, &mut vfs, "cron list")).contains("no cron"));
    }

    #[test]
    fn cron_add_and_list() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/job.sh", b"echo cron").unwrap();
        let added = text(exec(&reg, &mut vfs, "cron add myjob 300 /tmp/job.sh"));
        assert!(added.contains("every 5m"), "{added}");
        assert_eq!(vfs.read("/etc/cron/myjob").unwrap(), b"5m /tmp/job.sh\n");
        let s = text(exec(&reg, &mut vfs, "cron list"));
        assert!(s.contains("myjob  every 5m  /tmp/job.sh"), "{s}");
    }

    #[test]
    fn cron_add_requires_an_interval() {
        let (reg, mut vfs) = setup();
        // The pre-scheduler form `cron add <name> <script>` is rejected
        // rather than stored as a job that never runs.
        assert!(exec(&reg, &mut vfs, "cron add myjob /tmp/job.sh").is_err());
        assert!(exec(&reg, &mut vfs, "cron add myjob 0 /tmp/job.sh").is_err());
        assert!(exec(&reg, &mut vfs, "cron add myjob 5m /a.sh extra").is_err());
        assert!(!vfs.exists("/etc/cron/myjob"));
    }

    #[test]
    fn cron_list_flags_malformed_entries() {
        let (reg, mut vfs) = setup();
        vfs.mkdir(CRON_DIR).unwrap();
        vfs.write("/etc/cron/legacy", b"/tmp/job.sh").unwrap();
        let s = text(exec(&reg, &mut vfs, "cron list"));
        assert!(s.contains("legacy  (invalid:"), "{s}");
    }

    #[test]
    fn cron_remove() {
        let (reg, mut vfs) = setup();
        exec(&reg, &mut vfs, "cron add myjob 1m /tmp/job.sh").unwrap();
        exec(&reg, &mut vfs, "cron remove myjob").unwrap();
        assert!(text(exec(&reg, &mut vfs, "cron list")).contains("no cron"));
    }

    #[test]
    fn cron_remove_nonexistent() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "cron remove nope").is_err());
    }

    #[test]
    fn cron_rejects_names_escaping_cron_dir() {
        // `RealVfs` resolves `..` lexically, so `/etc/cron/../startup.sh`
        // is `/etc/startup.sh`.
        let dir = tempfile::tempdir().unwrap();
        let mut vfs = crate::vfs::RealVfs::new(dir.path()).unwrap();
        vfs.mkdir("/etc/cron").unwrap();
        vfs.write(STARTUP_SCRIPT_PATH, b"echo boot").unwrap();
        let mut reg = CommandRegistry::new();
        register_script_commands(&mut reg);
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        assert!(reg.execute("cron remove ../startup.sh", &mut env).is_err());
        assert!(
            env.vfs.exists(STARTUP_SCRIPT_PATH),
            "startup script deleted"
        );
        assert!(
            reg.execute("cron add ../startup.sh 5m /tmp/x.sh", &mut env)
                .is_err()
        );
        assert_eq!(env.vfs.read(STARTUP_SCRIPT_PATH).unwrap(), b"echo boot");
        assert!(reg.execute("cron add .. 5m /tmp/x.sh", &mut env).is_err());
    }

    #[test]
    fn cron_interval_parsing() {
        use cron::parse_interval;
        let secs = |t: &str| parse_interval(t).map(|d| d.as_secs()).ok();
        assert_eq!(secs("30"), Some(30));
        assert_eq!(secs("30s"), Some(30));
        assert_eq!(secs("5m"), Some(300));
        assert_eq!(secs("2h"), Some(7200));
        assert_eq!(secs("30d"), Some(30 * 86_400));
        for bad in [
            "",
            "0",
            "0s",
            "31d",
            "5x",
            "m",
            "-5",
            "1.5m",
            "99999999999999999999",
        ] {
            assert_eq!(secs(bad), None, "{bad:?} accepted");
        }
    }

    fn cron_tick(
        sched: &mut CronScheduler,
        secs: u64,
        reg: &CommandRegistry,
        vfs: &mut MemoryVfs,
    ) -> Vec<String> {
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
        sched.tick(std::time::Duration::from_secs(secs), reg, &mut env)
    }

    #[test]
    fn cron_scheduler_runs_jobs_every_interval() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/job.sh", b"echo tick").unwrap();
        exec(&reg, &mut vfs, "cron add beat 1m /tmp/job.sh").unwrap();
        let mut sched = CronScheduler::new();
        assert!(cron_tick(&mut sched, 0, &reg, &mut vfs).is_empty());
        assert_eq!(sched.len(), 1);
        assert!(cron_tick(&mut sched, 59, &reg, &mut vfs).is_empty());
        assert_eq!(
            cron_tick(&mut sched, 60, &reg, &mut vfs),
            vec!["[cron beat] tick"]
        );
        assert!(cron_tick(&mut sched, 61, &reg, &mut vfs).is_empty());
        // Overdue by several intervals: one run, then one interval later.
        assert_eq!(cron_tick(&mut sched, 400, &reg, &mut vfs).len(), 1);
        assert!(cron_tick(&mut sched, 459, &reg, &mut vfs).is_empty());
        assert_eq!(cron_tick(&mut sched, 460, &reg, &mut vfs).len(), 1);
    }

    #[test]
    fn cron_add_resolves_a_relative_script_against_the_cwd() {
        // Jobs run from `/`, so `cd /home; cron add j 1s j.sh` must store
        // `/home/j.sh` (it stored `j.sh` and failed every tick).
        let (reg, mut vfs) = setup();
        vfs.write("/home/j.sh", b"echo hi").unwrap();
        let mut env = Environment {
            cwd: "/home".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        reg.execute("cron add j 1s j.sh", &mut env).unwrap();
        let mut sched = CronScheduler::new();
        assert!(cron_tick(&mut sched, 0, &reg, &mut vfs).is_empty());
        assert_eq!(
            cron_tick(&mut sched, 1, &reg, &mut vfs),
            vec!["[cron j] hi"]
        );
    }

    #[test]
    fn cron_scheduler_follows_entry_changes() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/job.sh", b"echo tick").unwrap();
        exec(&reg, &mut vfs, "cron add beat 1m /tmp/job.sh").unwrap();
        let mut sched = CronScheduler::new();
        cron_tick(&mut sched, 0, &reg, &mut vfs);
        // A changed interval reschedules from the tick that sees it.
        exec(&reg, &mut vfs, "cron add beat 10s /tmp/job.sh").unwrap();
        assert!(cron_tick(&mut sched, 30, &reg, &mut vfs).is_empty());
        assert_eq!(cron_tick(&mut sched, 40, &reg, &mut vfs).len(), 1);
        // Removed jobs stop.
        exec(&reg, &mut vfs, "cron remove beat").unwrap();
        assert!(cron_tick(&mut sched, 1000, &reg, &mut vfs).is_empty());
        assert!(sched.is_empty());
    }

    #[test]
    fn cron_scheduler_reports_problems_once() {
        let (reg, mut vfs) = setup();
        vfs.mkdir(CRON_DIR).unwrap();
        vfs.write("/etc/cron/legacy", b"/tmp/job.sh").unwrap();
        exec(&reg, &mut vfs, "cron add gone 1s /tmp/missing.sh").unwrap();
        let mut sched = CronScheduler::new();
        let first = cron_tick(&mut sched, 0, &reg, &mut vfs);
        assert_eq!(first.len(), 1, "{first:?}");
        assert!(
            first[0].starts_with("[cron legacy] error: malformed"),
            "{first:?}"
        );
        // The malformed entry is not reported again; a missing script is
        // an error each time the job comes due.
        let later = cron_tick(&mut sched, 1, &reg, &mut vfs);
        assert_eq!(later.len(), 1, "{later:?}");
        assert!(
            later[0].starts_with("[cron gone] error: /tmp/missing.sh"),
            "{later:?}"
        );
    }

    #[test]
    fn cron_jobs_share_shell_variables() {
        let (reg, mut vfs) = setup();
        exec(&reg, &mut vfs, "set WHO=cron").unwrap();
        vfs.write("/tmp/job.sh", b"if true; then echo hi $WHO; fi")
            .unwrap();
        exec(&reg, &mut vfs, "cron add greet 1s /tmp/job.sh").unwrap();
        let mut sched = CronScheduler::new();
        cron_tick(&mut sched, 0, &reg, &mut vfs);
        assert_eq!(
            cron_tick(&mut sched, 1, &reg, &mut vfs),
            vec!["[cron greet] hi cron"]
        );
    }
    #[test]
    fn startup_show_empty() {
        let (reg, mut vfs) = setup();
        let __out = exec(&reg, &mut vfs, "startup show").unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected CommandOutput::Text, got {__out:?}");
        };
        assert!(s.contains("no startup"));
    }

    #[test]
    fn startup_set_and_show() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/init.sh", b"echo booting\nstatus").unwrap();
        exec(&reg, &mut vfs, "startup set /tmp/init.sh").unwrap();
        let __out = exec(&reg, &mut vfs, "startup show").unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected CommandOutput::Text, got {__out:?}");
        };
        assert!(s.contains("echo booting"));
        assert!(s.contains("status"));
    }

    #[test]
    fn startup_clear() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/init.sh", b"echo boot").unwrap();
        exec(&reg, &mut vfs, "startup set /tmp/init.sh").unwrap();
        exec(&reg, &mut vfs, "startup clear").unwrap();
        let __out = exec(&reg, &mut vfs, "startup show").unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected CommandOutput::Text, got {__out:?}");
        };
        assert!(s.contains("no startup"));
    }

    #[test]
    fn startup_set_missing_file() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "startup set /tmp/nope.sh").is_err());
    }

    fn boot(reg: &CommandRegistry, vfs: &mut MemoryVfs) -> Vec<String> {
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
        run_boot_scripts(reg, &mut env)
    }

    #[test]
    fn boot_scripts_absent_is_silent() {
        let (reg, mut vfs) = setup();
        assert!(boot(&reg, &mut vfs).is_empty());
    }

    #[test]
    fn boot_runs_profile_then_startup() {
        let (reg, mut vfs) = setup();
        vfs.write(PROFILE_PATH, b"set GREETING=hi").unwrap();
        // The startup script sees the profile's variables.
        vfs.write(STARTUP_SCRIPT_PATH, b"echo $GREETING booted")
            .unwrap();
        assert_eq!(boot(&reg, &mut vfs), vec!["hi booted"]);
        assert_eq!(reg.get_variable("GREETING").as_deref(), Some("hi"));
    }

    #[test]
    fn boot_failing_profile_still_runs_startup() {
        let (reg, mut vfs) = setup();
        vfs.write(PROFILE_PATH, b"if true; then echo unterminated")
            .unwrap();
        vfs.write(STARTUP_SCRIPT_PATH, b"echo booted").unwrap();
        let out = boot(&reg, &mut vfs);
        assert!(out[0].starts_with("error: /home/.profile"), "{out:?}");
        assert_eq!(out.last().map(String::as_str), Some("booted"));
    }

    #[test]
    fn seeded_profile_matches_registry_defaults() {
        let (reg, mut vfs) = setup();
        let before: Vec<_> = ["USER", "HOME", "SHELL"]
            .iter()
            .map(|k| reg.get_variable(k))
            .collect();
        crate::terminal::populate_profile(&mut vfs);
        assert!(boot(&reg, &mut vfs).is_empty());
        let after: Vec<_> = ["USER", "HOME", "SHELL"]
            .iter()
            .map(|k| reg.get_variable(k))
            .collect();
        assert_eq!(
            before, after,
            "the default profile must not change identity vars"
        );
    }
}
