//! Job control for the shell.
//!
//! Since OASIS_OS targets single-threaded environments (PSP, WASM), there are
//! no real OS processes or threads for background execution. Instead, jobs
//! represent deferred command execution that can be polled, paused, resumed,
//! and completed cooperatively.

use std::fmt;

use oasis_types::error::{OasisError, Result};

use crate::interpreter::{CommandOutput, CommandRegistry, Environment};

/// Current state of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    /// Currently running (or ready to run).
    Running,
    /// Paused/stopped.
    Stopped,
    /// Finished with exit code.
    Done(i32),
}

impl fmt::Display for JobState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => write!(f, "Running"),
            Self::Stopped => write!(f, "Stopped"),
            Self::Done(code) => write!(f, "Done({code})"),
        }
    }
}

/// A shell job (background or stopped command).
#[derive(Debug, Clone)]
pub struct Job {
    /// Job number (1-indexed, displayed as `%N`).
    pub id: usize,
    /// The original command string.
    pub command: String,
    /// Current state.
    pub state: JobState,
    /// Output accumulated while running in background.
    pub output: Vec<String>,
    /// Whether the user has been notified of completion.
    pub notified: bool,
}

impl fmt::Display for Job {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}]  {:<10}{}",
            self.id,
            self.state.to_string(),
            self.command,
        )
    }
}

/// Manages shell jobs (background and stopped commands).
pub struct JobManager {
    /// All tracked jobs (may include completed ones awaiting cleanup).
    jobs: Vec<Job>,
    /// Next job id to assign (monotonically increasing, 1-indexed).
    next_id: usize,
}

impl JobManager {
    /// Create an empty job manager.
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
            next_id: 1,
        }
    }

    /// Add a new running job and return its id.
    pub fn add_job(&mut self, command: String) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.jobs.push(Job {
            id,
            command,
            state: JobState::Running,
            output: Vec::new(),
            notified: false,
        });
        id
    }

    /// Get a job by id.
    pub fn get(&self, id: usize) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    /// Get a mutable reference to a job by id.
    pub fn get_mut(&mut self, id: usize) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    /// Pause a running job. Returns `true` if the state changed.
    pub fn stop_job(&mut self, id: usize) -> bool {
        if let Some(job) = self.get_mut(id)
            && job.state == JobState::Running
        {
            job.state = JobState::Stopped;
            return true;
        }
        false
    }

    /// Resume a stopped job. Returns `true` if the state changed.
    pub fn resume_job(&mut self, id: usize) -> bool {
        if let Some(job) = self.get_mut(id)
            && job.state == JobState::Stopped
        {
            job.state = JobState::Running;
            return true;
        }
        false
    }

    /// Mark a job as completed with the given exit code.
    /// Returns `true` if the job existed and was not already done.
    pub fn complete_job(&mut self, id: usize, exit_code: i32) -> bool {
        if let Some(job) = self.get_mut(id)
            && !matches!(job.state, JobState::Done(_))
        {
            job.state = JobState::Done(exit_code);
            return true;
        }
        false
    }

    /// Remove a job by id and return it. Typically used after the job
    /// is done and the user has been notified.
    pub fn remove_job(&mut self, id: usize) -> Option<Job> {
        if let Some(pos) = self.jobs.iter().position(|j| j.id == id) {
            Some(self.jobs.remove(pos))
        } else {
            None
        }
    }

    /// Append a line of output to the given job's output buffer.
    pub fn append_output(&mut self, id: usize, line: &str) {
        if let Some(job) = self.get_mut(id) {
            job.output.push(line.to_string());
        }
    }

    /// List all active (non-removed) jobs.
    pub fn list_jobs(&self) -> Vec<&Job> {
        self.jobs.iter().collect()
    }

    /// Count jobs that are currently in the `Running` state.
    pub fn running_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| j.state == JobState::Running)
            .count()
    }

    /// Check whether any completed jobs have not yet been reported
    /// to the user.
    pub fn has_unnotified(&self) -> bool {
        self.jobs
            .iter()
            .any(|j| matches!(j.state, JobState::Done(_)) && !j.notified)
    }

    /// Drain completed-but-unnotified jobs. Returns a list of
    /// `(job_id, command, exit_code)` tuples and marks each as notified.
    pub fn drain_notifications(&mut self) -> Vec<(usize, String, i32)> {
        let mut result = Vec::new();
        for job in &mut self.jobs {
            if let JobState::Done(code) = job.state
                && !job.notified
            {
                result.push((job.id, job.command.clone(), code));
                job.notified = true;
            }
        }
        result
    }

    /// Return the id of the most recently added job, if any.
    pub fn most_recent(&self) -> Option<usize> {
        self.jobs.last().map(|j| j.id)
    }

    /// Remove all jobs that are done and have been notified.
    pub fn cleanup_done(&mut self) {
        self.jobs
            .retain(|j| !matches!(j.state, JobState::Done(_)) || !j.notified);
    }

    /// Check whether the job list is empty.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }
}

impl Default for JobManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a job specifier string into a job id.
///
/// Supported formats:
/// - `%N` -- job number N (e.g. `%1`, `%23`)
/// - `%` or `%%` or `%+` -- the most recently added job
/// - `%-` -- the second most recent job
///
/// Returns `None` if the specifier is invalid or refers to a
/// nonexistent job.
pub fn parse_job_spec(spec: &str, manager: &JobManager) -> Option<usize> {
    let trimmed = spec.trim();
    if !trimmed.starts_with('%') {
        return None;
    }

    let suffix = &trimmed[1..];

    match suffix {
        "" | "%" | "+" => manager.most_recent(),
        "-" => {
            let len = manager.jobs.len();
            if len >= 2 {
                Some(manager.jobs[len - 2].id)
            } else {
                None
            }
        },
        digits => {
            let id: usize = digits.parse().ok()?;
            // Verify the job actually exists.
            if manager.get(id).is_some() {
                Some(id)
            } else {
                None
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Shell integration: `cmd &`, `jobs`, `fg`, `bg`, `kill %N`
// ---------------------------------------------------------------------------
//
// The interpreter is synchronous, so a background job is a *deferred*
// command: `cmd &` queues it (state `Running` = "runnable") and returns
// immediately; the host calls `CommandRegistry::poll_jobs` once per tick
// (frame), which runs the oldest runnable job to completion and reports
// its output followed by a `[N]+  Done  cmd` line. `kill -STOP %N`
// parks a queued job (`Stopped`), `bg %N` / `kill -CONT %N` makes it
// runnable again, `fg %N` runs it right away, and `kill %N` drops it.

impl CommandRegistry {
    /// Queue `cmd` as a background job (trailing `&`).
    pub(crate) fn queue_job(&self, cmd: &str) -> CommandOutput {
        let id = self.jobs.borrow_mut().add_job(cmd.to_string());
        CommandOutput::Text(format!("[{id}] {cmd}"))
    }

    /// Run the oldest runnable background job, if any.
    ///
    /// Hosts call this once per tick. Returns the job's output followed
    /// by a completion notice (`[N]+  Done  cmd`), or `None` when no job
    /// is runnable. Does nothing while a command is executing.
    pub fn poll_jobs(&self, env: &mut Environment<'_>) -> Option<CommandOutput> {
        if self.exec_depth.get() != 0 {
            return None;
        }
        let (id, cmd) = {
            let jobs = self.jobs.borrow();
            let job = jobs
                .list_jobs()
                .into_iter()
                .find(|j| j.state == JobState::Running)?;
            (job.id, job.command.clone())
        };
        Some(self.run_job(id, &cmd, env, true))
    }

    /// Number of jobs that are queued and runnable.
    pub fn pending_jobs(&self) -> usize {
        self.jobs.borrow().running_count()
    }

    /// Remove job `id` and run it synchronously.
    fn run_job(
        &self,
        id: usize,
        cmd: &str,
        env: &mut Environment<'_>,
        notify: bool,
    ) -> CommandOutput {
        self.jobs.borrow_mut().remove_job(id);
        let mut outputs = Vec::new();
        if !notify {
            // `fg` echoes the command like an interactive shell.
            outputs.push(CommandOutput::Text(cmd.to_string()));
        }
        let code = match self.execute_nested(cmd, env) {
            Ok(output) => {
                outputs.push(output);
                self.last_exit_code.get()
            },
            Err(e) => {
                outputs.push(CommandOutput::Text(format!("error: {e}")));
                1
            },
        };
        if notify {
            let status = if code == 0 {
                "Done".to_string()
            } else {
                format!("Exit {code}")
            };
            outputs.push(CommandOutput::Text(format!("[{id}]+  {status:<10}{cmd}")));
        }
        crate::executor::merge_outputs(outputs)
    }

    /// Resolve a job spec argument (`%N`, `%%`, `%+`, `%-`; default `%%`).
    fn resolve_job(&self, spec: Option<&&str>, builtin: &str) -> Result<(usize, String)> {
        let spec = spec.copied().unwrap_or("%%");
        let jobs = self.jobs.borrow();
        parse_job_spec(spec, &jobs)
            .and_then(|id| jobs.get(id).map(|j| (id, j.command.clone())))
            .ok_or_else(|| OasisError::Command(format!("{builtin}: {spec}: no such job").into()))
    }

    /// Built-in `jobs`: list background jobs.
    pub(crate) fn execute_jobs(&self) -> Result<CommandOutput> {
        let jobs = self.jobs.borrow();
        if jobs.is_empty() {
            return Ok(CommandOutput::None);
        }
        let recent = jobs.most_recent();
        let lines: Vec<String> = jobs
            .list_jobs()
            .iter()
            .map(|j| {
                let mark = if Some(j.id) == recent { '+' } else { ' ' };
                format!("[{}]{mark} {:<10}{}", j.id, j.state.to_string(), j.command)
            })
            .collect();
        Ok(CommandOutput::Text(lines.join("\n")))
    }

    /// Built-in `fg [%N]`: run a job now, in the foreground.
    pub(crate) fn execute_fg(
        &self,
        args: &[&str],
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let (id, cmd) = self.resolve_job(args.first(), "fg")?;
        Ok(self.run_job(id, &cmd, env, false))
    }

    /// Built-in `bg [%N]`: make a stopped job runnable again.
    pub(crate) fn execute_bg(&self, args: &[&str]) -> Result<CommandOutput> {
        let (id, cmd) = self.resolve_job(args.first(), "bg")?;
        if self.jobs.borrow_mut().resume_job(id) {
            Ok(CommandOutput::Text(format!("[{id}]+ {cmd} &")))
        } else {
            Ok(CommandOutput::Text(format!(
                "bg: job {id} already in background"
            )))
        }
    }

    /// Built-in `kill [-STOP|-CONT|-TERM|-9] %N...`.
    pub(crate) fn execute_kill(&self, args: &[&str]) -> Result<CommandOutput> {
        let mut signal = "TERM";
        let mut lines = Vec::new();
        for arg in args {
            if let Some(sig) = arg.strip_prefix('-') {
                signal = match sig.trim_start_matches("SIG") {
                    "STOP" | "TSTP" | "19" | "20" => "STOP",
                    "CONT" | "18" => "CONT",
                    "TERM" | "KILL" | "INT" | "HUP" | "9" | "15" | "2" | "1" => "TERM",
                    other => {
                        return Err(OasisError::Command(
                            format!("kill: unknown signal: {other}").into(),
                        ));
                    },
                };
                continue;
            }
            let (id, cmd) = self.resolve_job(Some(arg), "kill")?;
            let mut jobs = self.jobs.borrow_mut();
            let state = match signal {
                "STOP" => {
                    jobs.stop_job(id);
                    "Stopped"
                },
                "CONT" => {
                    jobs.resume_job(id);
                    "Running"
                },
                _ => {
                    jobs.remove_job(id);
                    "Terminated"
                },
            };
            lines.push(format!("[{id}]+  {state:<10}{cmd}"));
        }
        if lines.is_empty() {
            return Err(OasisError::Command(
                "usage: kill [-STOP|-CONT|-TERM] %job...".into(),
            ));
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_job_creates_running_job() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("echo hello &".to_string());
        assert_eq!(id, 1);

        let job = mgr.get(id).unwrap();
        assert_eq!(job.id, 1);
        assert_eq!(job.command, "echo hello &");
        assert_eq!(job.state, JobState::Running);
        assert!(job.output.is_empty());
        assert!(!job.notified);
    }

    #[test]
    fn add_multiple_jobs_increments_id() {
        let mut mgr = JobManager::new();
        let id1 = mgr.add_job("sleep 10 &".to_string());
        let id2 = mgr.add_job("find / &".to_string());
        let id3 = mgr.add_job("ls &".to_string());

        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
    }

    #[test]
    fn stop_job_transitions_running_to_stopped() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("sleep 10 &".to_string());

        assert!(mgr.stop_job(id));
        assert_eq!(mgr.get(id).unwrap().state, JobState::Stopped);
    }

    #[test]
    fn stop_job_returns_false_for_non_running() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("sleep 10 &".to_string());
        mgr.complete_job(id, 0);

        assert!(!mgr.stop_job(id));
    }

    #[test]
    fn stop_job_returns_false_for_invalid_id() {
        let mut mgr = JobManager::new();
        assert!(!mgr.stop_job(42));
    }

    #[test]
    fn resume_job_transitions_stopped_to_running() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("find / &".to_string());
        mgr.stop_job(id);

        assert!(mgr.resume_job(id));
        assert_eq!(mgr.get(id).unwrap().state, JobState::Running);
    }

    #[test]
    fn resume_job_returns_false_for_running() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("echo hi &".to_string());

        assert!(!mgr.resume_job(id));
    }

    #[test]
    fn complete_job_transitions_to_done() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("echo hello &".to_string());

        assert!(mgr.complete_job(id, 0));
        assert_eq!(mgr.get(id).unwrap().state, JobState::Done(0));
    }

    #[test]
    fn complete_job_with_nonzero_exit_code() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("false &".to_string());

        assert!(mgr.complete_job(id, 1));
        assert_eq!(mgr.get(id).unwrap().state, JobState::Done(1));
    }

    #[test]
    fn complete_job_returns_false_if_already_done() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("echo x &".to_string());
        mgr.complete_job(id, 0);

        assert!(!mgr.complete_job(id, 1));
        // State unchanged.
        assert_eq!(mgr.get(id).unwrap().state, JobState::Done(0));
    }

    #[test]
    fn complete_stopped_job() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("sleep 5 &".to_string());
        mgr.stop_job(id);

        assert!(mgr.complete_job(id, 130));
        assert_eq!(mgr.get(id).unwrap().state, JobState::Done(130));
    }

    #[test]
    fn list_jobs_returns_all_active_jobs() {
        let mut mgr = JobManager::new();
        mgr.add_job("a".to_string());
        mgr.add_job("b".to_string());
        mgr.add_job("c".to_string());

        let jobs = mgr.list_jobs();
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[0].command, "a");
        assert_eq!(jobs[1].command, "b");
        assert_eq!(jobs[2].command, "c");
    }

    #[test]
    fn running_count_accuracy() {
        let mut mgr = JobManager::new();
        let id1 = mgr.add_job("a".to_string());
        let id2 = mgr.add_job("b".to_string());
        mgr.add_job("c".to_string());

        assert_eq!(mgr.running_count(), 3);

        mgr.stop_job(id1);
        assert_eq!(mgr.running_count(), 2);

        mgr.complete_job(id2, 0);
        assert_eq!(mgr.running_count(), 1);
    }

    #[test]
    fn drain_notifications_returns_done_jobs() {
        let mut mgr = JobManager::new();
        let id1 = mgr.add_job("echo a &".to_string());
        let id2 = mgr.add_job("echo b &".to_string());
        mgr.add_job("sleep 10 &".to_string());

        mgr.complete_job(id1, 0);
        mgr.complete_job(id2, 42);

        let notifs = mgr.drain_notifications();
        assert_eq!(notifs.len(), 2);
        assert_eq!(notifs[0], (1, "echo a &".to_string(), 0));
        assert_eq!(notifs[1], (2, "echo b &".to_string(), 42));

        // Second drain returns empty -- already notified.
        let notifs2 = mgr.drain_notifications();
        assert!(notifs2.is_empty());
    }

    #[test]
    fn has_unnotified_tracking() {
        let mut mgr = JobManager::new();
        assert!(!mgr.has_unnotified());

        let id = mgr.add_job("echo hi &".to_string());
        assert!(!mgr.has_unnotified());

        mgr.complete_job(id, 0);
        assert!(mgr.has_unnotified());

        mgr.drain_notifications();
        assert!(!mgr.has_unnotified());
    }

    #[test]
    fn cleanup_done_removes_notified_completed_jobs() {
        let mut mgr = JobManager::new();
        let id1 = mgr.add_job("a".to_string());
        let id2 = mgr.add_job("b".to_string());
        let _id3 = mgr.add_job("c".to_string());

        mgr.complete_job(id1, 0);
        mgr.complete_job(id2, 0);
        mgr.drain_notifications();

        mgr.cleanup_done();
        // Only "c" (still running) remains.
        assert_eq!(mgr.list_jobs().len(), 1);
        assert_eq!(mgr.list_jobs()[0].command, "c");
    }

    #[test]
    fn cleanup_done_keeps_unnotified_done_jobs() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("a".to_string());
        mgr.complete_job(id, 0);

        mgr.cleanup_done();
        // Not yet notified, so it stays.
        assert_eq!(mgr.list_jobs().len(), 1);
    }

    #[test]
    fn append_output_accumulates() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("find / &".to_string());

        mgr.append_output(id, "line1");
        mgr.append_output(id, "line2");
        mgr.append_output(id, "line3");

        let job = mgr.get(id).unwrap();
        assert_eq!(job.output.len(), 3);
        assert_eq!(job.output[0], "line1");
        assert_eq!(job.output[1], "line2");
        assert_eq!(job.output[2], "line3");
    }

    #[test]
    fn append_output_to_invalid_id_is_noop() {
        let mut mgr = JobManager::new();
        mgr.append_output(99, "ghost");
        assert!(mgr.is_empty());
    }

    #[test]
    fn remove_job_returns_and_removes() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("echo done &".to_string());
        mgr.complete_job(id, 0);

        let removed = mgr.remove_job(id);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().command, "echo done &");
        assert!(mgr.is_empty());
    }

    #[test]
    fn remove_job_returns_none_for_invalid_id() {
        let mut mgr = JobManager::new();
        assert!(mgr.remove_job(99).is_none());
    }

    #[test]
    fn most_recent_returns_latest() {
        let mut mgr = JobManager::new();
        assert!(mgr.most_recent().is_none());

        mgr.add_job("a".to_string());
        assert_eq!(mgr.most_recent(), Some(1));

        mgr.add_job("b".to_string());
        assert_eq!(mgr.most_recent(), Some(2));
    }

    #[test]
    fn is_empty_on_new_manager() {
        let mgr = JobManager::new();
        assert!(mgr.is_empty());
    }

    #[test]
    fn is_empty_after_removal() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("x".to_string());
        assert!(!mgr.is_empty());

        mgr.remove_job(id);
        assert!(mgr.is_empty());
    }

    #[test]
    fn display_formatting_running() {
        let mut mgr = JobManager::new();
        mgr.add_job("sleep 10 &".to_string());

        let job = mgr.get(1).unwrap();
        let display = format!("{job}");
        assert_eq!(display, "[1]  Running   sleep 10 &");
    }

    #[test]
    fn display_formatting_stopped() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("find / -name \"*.rs\"".to_string());
        mgr.stop_job(id);

        let job = mgr.get(id).unwrap();
        let display = format!("{job}");
        assert_eq!(display, "[1]  Stopped   find / -name \"*.rs\"");
    }

    #[test]
    fn display_formatting_done() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("echo hello &".to_string());
        mgr.complete_job(id, 0);

        let job = mgr.get(id).unwrap();
        let display = format!("{job}");
        assert_eq!(display, "[1]  Done(0)   echo hello &");
    }

    #[test]
    fn parse_job_spec_percent_n() {
        let mut mgr = JobManager::new();
        mgr.add_job("a".to_string());
        mgr.add_job("b".to_string());

        assert_eq!(parse_job_spec("%1", &mgr), Some(1));
        assert_eq!(parse_job_spec("%2", &mgr), Some(2));
        assert_eq!(parse_job_spec("%3", &mgr), None);
    }

    #[test]
    fn parse_job_spec_percent_most_recent() {
        let mut mgr = JobManager::new();
        mgr.add_job("a".to_string());
        mgr.add_job("b".to_string());

        assert_eq!(parse_job_spec("%", &mgr), Some(2));
        assert_eq!(parse_job_spec("%%", &mgr), Some(2));
        assert_eq!(parse_job_spec("%+", &mgr), Some(2));
    }

    #[test]
    fn parse_job_spec_percent_minus() {
        let mut mgr = JobManager::new();
        mgr.add_job("a".to_string());
        mgr.add_job("b".to_string());
        mgr.add_job("c".to_string());

        assert_eq!(parse_job_spec("%-", &mgr), Some(2));
    }

    #[test]
    fn parse_job_spec_minus_with_one_job() {
        let mut mgr = JobManager::new();
        mgr.add_job("a".to_string());

        assert_eq!(parse_job_spec("%-", &mgr), None);
    }

    #[test]
    fn parse_job_spec_empty_manager() {
        let mgr = JobManager::new();
        assert_eq!(parse_job_spec("%", &mgr), None);
        assert_eq!(parse_job_spec("%1", &mgr), None);
        assert_eq!(parse_job_spec("%-", &mgr), None);
    }

    #[test]
    fn parse_job_spec_invalid_format() {
        let mgr = JobManager::new();
        assert_eq!(parse_job_spec("1", &mgr), None);
        assert_eq!(parse_job_spec("", &mgr), None);
        assert_eq!(parse_job_spec("abc", &mgr), None);
        assert_eq!(parse_job_spec("%abc", &mgr), None);
    }

    #[test]
    fn parse_job_spec_whitespace() {
        let mut mgr = JobManager::new();
        mgr.add_job("a".to_string());

        assert_eq!(parse_job_spec(" %1 ", &mgr), Some(1));
        assert_eq!(parse_job_spec("  %  ", &mgr), Some(1));
    }

    #[test]
    fn default_trait() {
        let mgr = JobManager::default();
        assert!(mgr.is_empty());
        assert_eq!(mgr.running_count(), 0);
        assert!(!mgr.has_unnotified());
    }

    #[test]
    fn job_state_display() {
        assert_eq!(format!("{}", JobState::Running), "Running");
        assert_eq!(format!("{}", JobState::Stopped), "Stopped");
        assert_eq!(format!("{}", JobState::Done(0)), "Done(0)");
        assert_eq!(format!("{}", JobState::Done(127)), "Done(127)");
    }

    #[test]
    fn get_returns_none_for_missing() {
        let mgr = JobManager::new();
        assert!(mgr.get(1).is_none());
        assert!(mgr.get(0).is_none());
    }

    #[test]
    fn get_mut_allows_modification() {
        let mut mgr = JobManager::new();
        let id = mgr.add_job("test".to_string());

        if let Some(job) = mgr.get_mut(id) {
            job.command = "modified".to_string();
        }

        assert_eq!(mgr.get(id).unwrap().command, "modified");
    }

    #[test]
    fn ids_do_not_recycle_after_removal() {
        let mut mgr = JobManager::new();
        let id1 = mgr.add_job("a".to_string());
        mgr.remove_job(id1);

        let id2 = mgr.add_job("b".to_string());
        assert_eq!(id2, 2); // Not 1 again.
    }

    #[test]
    fn full_lifecycle() {
        let mut mgr = JobManager::new();

        // Create and run.
        let id = mgr.add_job("sleep 5 &".to_string());
        assert_eq!(mgr.running_count(), 1);
        assert!(!mgr.has_unnotified());

        // Accumulate output.
        mgr.append_output(id, "tick 1");
        mgr.append_output(id, "tick 2");

        // Stop.
        assert!(mgr.stop_job(id));
        assert_eq!(mgr.running_count(), 0);
        assert_eq!(mgr.get(id).unwrap().state, JobState::Stopped);

        // Resume.
        assert!(mgr.resume_job(id));
        assert_eq!(mgr.running_count(), 1);

        // Complete.
        assert!(mgr.complete_job(id, 0));
        assert_eq!(mgr.running_count(), 0);
        assert!(mgr.has_unnotified());

        // Drain notifications.
        let notifs = mgr.drain_notifications();
        assert_eq!(notifs.len(), 1);
        assert_eq!(notifs[0].0, id);
        assert!(!mgr.has_unnotified());

        // Cleanup.
        mgr.cleanup_done();
        assert!(mgr.is_empty());
    }
}
