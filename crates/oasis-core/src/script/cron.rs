//! Interval-scheduled scripts: the `cron` command and [`CronScheduler`].
//!
//! Each job is one file `/etc/cron/<name>` holding `<interval> <script>`
//! (`5m /home/backup.sh`). The command only edits those files; a host runs
//! due jobs by calling [`CronScheduler::tick`] from its frame loop (the
//! desktop shell does, about once a second). Schedules are relative to the
//! host's monotonic clock and are not persisted: after a restart every job
//! first runs one interval after the scheduler first sees it. A job that
//! is overdue (the host was busy or suspended) runs once, not once per
//! missed interval.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::error::{OasisError, Result};
use crate::terminal::{Command, CommandOutput, CommandRegistry, Environment};

use super::{CRON_DIR, run_script};

/// Shortest accepted interval.
const MIN_INTERVAL: Duration = Duration::from_secs(1);
/// Longest accepted interval (30 days).
const MAX_INTERVAL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// One parsed `/etc/cron/<name>` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronEntry {
    /// Time between runs.
    pub interval: Duration,
    /// VFS path of the script to run.
    pub script: String,
}

impl CronEntry {
    /// Parse an entry file's contents (`<interval> <script>`).
    pub fn parse(content: &str) -> Result<Self> {
        let mut words = content.split_whitespace();
        let (Some(interval), Some(script), None) = (words.next(), words.next(), words.next())
        else {
            return Err(cmd_err(format!(
                "malformed cron entry {:?} (expected `<interval> <script>`)",
                content.trim()
            )));
        };
        Ok(Self {
            interval: parse_interval(interval)?,
            script: script.to_string(),
        })
    }

    /// The entry file's contents.
    fn serialize(&self) -> String {
        format!("{} {}\n", format_interval(self.interval), self.script)
    }
}

/// Parse `30`, `30s`, `5m`, `2h` or `1d` into a duration within
/// [`MIN_INTERVAL`]..=[`MAX_INTERVAL`].
pub fn parse_interval(text: &str) -> Result<Duration> {
    let (digits, unit) = match text.find(|c: char| !c.is_ascii_digit()) {
        Some(i) => text.split_at(i),
        None => (text, "s"),
    };
    let scale = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        _ => 0,
    };
    let secs = digits
        .parse::<u64>()
        .ok()
        .filter(|_| scale != 0)
        .and_then(|n| n.checked_mul(scale))
        .map(Duration::from_secs)
        .ok_or_else(|| {
            cmd_err(format!(
                "invalid interval: {text} (use e.g. 30s, 5m, 2h, 1d)"
            ))
        })?;
    if !(MIN_INTERVAL..=MAX_INTERVAL).contains(&secs) {
        return Err(cmd_err(format!(
            "interval out of range: {text} (1s to 30d)"
        )));
    }
    Ok(secs)
}

/// Shortest unit that represents `d` exactly (`300s` -> `5m`).
fn format_interval(d: Duration) -> String {
    let secs = d.as_secs();
    for (unit, scale) in [("d", 86_400), ("h", 3_600), ("m", 60)] {
        if secs % scale == 0 {
            return format!("{}{unit}", secs / scale);
        }
    }
    format!("{secs}s")
}

fn cmd_err(msg: String) -> OasisError {
    OasisError::Command(msg.into())
}

/// VFS path of the cron entry `name`, rejecting names that would escape
/// [`CRON_DIR`] (`cron remove ../startup.sh` used to delete the startup
/// script).
fn cron_entry_path(name: &str) -> Result<String> {
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err(cmd_err(format!("invalid cron job name: {name}")));
    }
    Ok(format!("{CRON_DIR}/{name}"))
}

/// Every entry under [`CRON_DIR`], by name, each parsed or failed.
fn read_entries(env: &mut Environment<'_>) -> Vec<(String, Result<CronEntry>)> {
    if !env.vfs.exists(CRON_DIR) {
        return Vec::new();
    }
    let Ok(mut names) = env.vfs.readdir(CRON_DIR) else {
        return Vec::new();
    };
    names.sort_by(|a, b| a.name.cmp(&b.name));
    names
        .into_iter()
        .filter(|e| matches!(e.kind, crate::vfs::EntryKind::File))
        .map(|e| {
            let parsed = env
                .vfs
                .read(&format!("{CRON_DIR}/{}", e.name))
                .and_then(|data| CronEntry::parse(&String::from_utf8_lossy(&data)));
            (e.name, parsed)
        })
        .collect()
}

/// Run the script of job `name`, its output lines prefixed `[cron name]`.
fn run_job(
    name: &str,
    entry: &CronEntry,
    registry: &CommandRegistry,
    env: &mut Environment<'_>,
) -> Vec<String> {
    let prefix = format!("[cron {name}]");
    match run_script(&entry.script, registry, env) {
        Ok(lines) => lines.into_iter().map(|l| format!("{prefix} {l}")).collect(),
        Err(e) => vec![format!("{prefix} error: {}: {e}", entry.script)],
    }
}

/// Per-job schedule state kept by [`CronScheduler`].
#[derive(Debug)]
struct Slot {
    entry: CronEntry,
    due: Duration,
}

/// Runs the jobs under [`CRON_DIR`] when they come due.
///
/// Hold one per host and call [`tick`](Self::tick) regularly with a
/// monotonic clock reading; the job files are re-read on every call, so
/// `cron add` / `cron remove` take effect on the next tick.
#[derive(Debug, Default)]
pub struct CronScheduler {
    slots: BTreeMap<String, Slot>,
    /// Malformed entries already reported (name -> contents' error), so a
    /// bad file is reported once instead of every tick.
    reported: BTreeMap<String, String>,
}

impl CronScheduler {
    /// A scheduler with no jobs seen yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Run every job that is due at `now` (any monotonic clock, e.g. the
    /// time since host start) and return their output lines.
    ///
    /// A job seen for the first time, or whose entry changed, is scheduled
    /// one interval after `now`. After running, a job is next due one
    /// interval after `now`. A malformed entry yields one `[cron name]
    /// error: ...` line the first time it is seen and is otherwise skipped.
    pub fn tick(
        &mut self,
        now: Duration,
        registry: &CommandRegistry,
        env: &mut Environment<'_>,
    ) -> Vec<String> {
        let entries = read_entries(env);
        self.slots
            .retain(|name, _| entries.iter().any(|(n, e)| n == name && e.is_ok()));
        self.reported
            .retain(|name, _| entries.iter().any(|(n, e)| n == name && e.is_err()));

        let mut out = Vec::new();
        for (name, parsed) in entries {
            let entry = match parsed {
                Ok(entry) => entry,
                Err(e) => {
                    let msg = e.to_string();
                    if self.reported.get(&name) != Some(&msg) {
                        out.push(format!("[cron {name}] error: {msg}"));
                        self.reported.insert(name, msg);
                    }
                    continue;
                },
            };
            match self.slots.get_mut(&name) {
                Some(slot) if slot.entry == entry => {
                    if now >= slot.due {
                        slot.due = now + entry.interval;
                        out.extend(run_job(&name, &entry, registry, env));
                    }
                },
                _ => {
                    let due = now + entry.interval;
                    self.slots.insert(name, Slot { entry, due });
                },
            }
        }
        out
    }

    /// Number of well-formed jobs currently scheduled.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// True when no job is scheduled.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// `cron` -- manage interval-scheduled scripts.
pub struct CronCmd;

impl Command for CronCmd {
    fn name(&self) -> &str {
        "cron"
    }
    fn description(&self) -> &str {
        "Run scripts on an interval"
    }
    fn usage(&self) -> &str {
        "cron [list|add <name> <interval> <script>|remove <name>]"
    }
    fn category(&self) -> &str {
        "scripting"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("list");
        match subcmd {
            "list" => Ok(CommandOutput::Text(list(env))),
            "add" => {
                let [_, name, interval, script] = args else {
                    return Err(cmd_err(
                        "usage: cron add <name> <interval> <script>".to_string(),
                    ));
                };
                let entry_path = cron_entry_path(name)?;
                let entry = CronEntry {
                    interval: parse_interval(interval)?,
                    script: (*script).to_string(),
                };
                if !env.vfs.exists(CRON_DIR) {
                    env.vfs.mkdir(CRON_DIR)?;
                }
                env.vfs.write(&entry_path, entry.serialize().as_bytes())?;
                Ok(CommandOutput::Text(format!(
                    "Cron job '{name}' added: {script} every {}",
                    format_interval(entry.interval)
                )))
            },
            "remove" => {
                let name = one_name(args, "remove")?;
                let entry_path = cron_entry_path(name)?;
                if !env.vfs.exists(&entry_path) {
                    return Err(cmd_err(format!("cron job not found: {name}")));
                }
                env.vfs.remove(&entry_path)?;
                Ok(CommandOutput::Text(format!("Cron job '{name}' removed")))
            },
            _ => Err(cmd_err(format!(
                "unknown subcommand: {subcmd}\nusage: {}",
                self.usage()
            ))),
        }
    }
}

fn one_name<'a>(args: &[&'a str], sub: &str) -> Result<&'a str> {
    match args {
        [_, name] => Ok(name),
        _ => Err(cmd_err(format!("usage: cron {sub} <name>"))),
    }
}

/// `cron list` output.
fn list(env: &mut Environment<'_>) -> String {
    let entries = read_entries(env);
    if entries.is_empty() {
        return "(no cron jobs configured)".to_string();
    }
    let mut out = String::from("Scheduled scripts:\n");
    for (name, parsed) in entries {
        match parsed {
            Ok(e) => out.push_str(&format!(
                "  {name}  every {}  {}\n",
                format_interval(e.interval),
                e.script
            )),
            Err(e) => out.push_str(&format!("  {name}  (invalid: {e})\n")),
        }
    }
    out
}
