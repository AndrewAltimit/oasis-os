//! Job control for the shell.
//!
//! Since OASIS_OS targets single-threaded environments (PSP, WASM), there are
//! no real OS processes or threads for background execution. Instead, jobs
//! represent deferred command execution that can be polled, paused, resumed,
//! and completed cooperatively.
//!
//! This module holds the job table ([`Job`], [`JobState`], [`JobManager`],
//! [`parse_job_spec`]); `shell` wires it into [`CommandRegistry`] (`cmd &`,
//! `jobs`, `fg`, `bg`, `kill %N`, [`CommandRegistry::poll_jobs`]).
//!
//! [`CommandRegistry`]: crate::interpreter::CommandRegistry
//! [`CommandRegistry::poll_jobs`]: crate::interpreter::CommandRegistry::poll_jobs

mod shell;
#[cfg(test)]
mod tests;

use std::fmt;

/// Maximum background jobs waiting to run (running or stopped).
pub(crate) const MAX_PENDING_JOBS: usize = 64;

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
