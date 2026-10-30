//! Shell integration of job control: `cmd &`, `jobs`, `fg`, `bg`,
//! `kill %N` and [`CommandRegistry::poll_jobs`].

use oasis_types::error::{OasisError, Result};

use super::{JobState, MAX_PENDING_JOBS, parse_job_spec};
use crate::interpreter::{CommandOutput, CommandRegistry, Environment};

// The interpreter is synchronous, so a background job is a *deferred*
// command: `cmd &` queues it (state `Running` = "runnable") and returns
// immediately; the host calls `CommandRegistry::poll_jobs` once per tick
// (frame), which runs the oldest runnable job to completion and reports
// its output followed by a `[N]+  Done  cmd` line. `kill -STOP %N`
// parks a queued job (`Stopped`), `bg %N` / `kill -CONT %N` makes it
// runnable again, `fg %N` runs it right away, and `kill %N` drops it.

impl CommandRegistry {
    /// Queue `cmd` as a background job (trailing `&`).
    /// Queue `cmd` as a background job. At most [`MAX_PENDING_JOBS`] jobs
    /// may be waiting (running or stopped), so a loop of `cmd &` cannot grow
    /// the job table without bound.
    pub(crate) fn queue_job(&self, cmd: &str) -> Result<CommandOutput> {
        let mut jobs = self.jobs.borrow_mut();
        let pending = jobs
            .list_jobs()
            .iter()
            .filter(|j| !matches!(j.state, JobState::Done(_)))
            .count();
        if pending >= MAX_PENDING_JOBS {
            return Err(OasisError::Command(
                format!("jobs: too many background jobs (max {MAX_PENDING_JOBS})").into(),
            ));
        }
        let id = jobs.add_job(cmd.to_string());
        Ok(CommandOutput::Text(format!("[{id}] {cmd}")))
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
        self.begin_top_level();
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
    /// A bare job number (`kill 1`, `fg 2`) means `%N`: the shell has no
    /// process ids, so there is nothing else it could refer to.
    fn resolve_job(&self, spec: Option<&&str>, builtin: &str) -> Result<(usize, String)> {
        let spec = spec.copied().unwrap_or("%%");
        let jobs = self.jobs.borrow();
        let numbered;
        let lookup = if !spec.is_empty() && spec.bytes().all(|b| b.is_ascii_digit()) {
            numbered = format!("%{spec}");
            numbered.as_str()
        } else {
            spec
        };
        parse_job_spec(lookup, &jobs)
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
