//! Running a command that has I/O redirections (`<`, `>`, `>>`, `2>`,
//! `2>>`, `2>&1`, `>&2`, `&>`).
//!
//! [`CommandRegistry::execute_redirected`] runs in three steps: expand the
//! targets ([`RedirectTargets`]), run the command with any `<` file as its
//! stdin, then route stdout / stderr to their targets ([`route_streams`]).

use oasis_types::error::{OasisError, Result};

use crate::expander::resolve_path;
use crate::pipeline::{Redirections, capture_text, write_redirect};
use crate::registry::CommandRegistry;
use crate::types::{CommandOutput, Environment};

/// The expanded redirect targets of one command.
struct RedirectTargets {
    /// `< path`.
    stdin: Option<String>,
    /// `> path` / `>> path` (`true` = append).
    stdout: Option<(String, bool)>,
    /// `2> path` / `2>> path` (`true` = append).
    stderr: Option<(String, bool)>,
}

/// What a finished command produced, before routing.
struct Captured {
    output: CommandOutput,
    stderr: String,
    /// The command failed; its error text is the tail of `stderr`.
    failed: bool,
}

impl CommandRegistry {
    /// [`execute_with_redirect`](Self::execute_with_redirect) for a command
    /// that has at least one redirection.
    #[inline(never)]
    pub(crate) fn execute_redirected(
        &self,
        cmd_part: &str,
        redirections: &Redirections<'_>,
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let targets = self.redirect_targets(redirections, env)?;
        env.stderr.clear();
        let result = self.run_with_stdin(cmd_part.trim(), targets.stdin.as_deref(), env);

        let handles_stderr = redirections.stderr.is_some()
            || redirections.stderr_to_stdout
            || redirections.stdout_to_stderr;
        if !handles_stderr {
            // Errors propagate normally; only stdout can be redirected.
            let output = result?;
            return match targets.stdout {
                Some((path, append)) => {
                    write_redirect(&capture_text(&output), &path, append, &env.cwd, env.vfs)?;
                    Ok(CommandOutput::None)
                },
                None => Ok(output),
            };
        }

        let captured = match result {
            Ok(output) => Captured {
                output,
                stderr: std::mem::take(&mut env.stderr),
                failed: false,
            },
            Err(e) => Captured {
                output: CommandOutput::None,
                stderr: join_lines(std::mem::take(&mut env.stderr), &e.to_string()),
                failed: true,
            },
        };
        let failed = captured.failed;
        let output = route_streams(captured, redirections, targets, env)?;
        // Keep the failure's exit code even though its text was captured.
        if failed {
            self.last_exit_code.set(1);
            self.set_variable("?", "1");
        }
        Ok(output)
    }

    /// Expand every redirect target like an argument (`> $DIR/out.txt`,
    /// `> "my file"`).
    fn redirect_targets(
        &self,
        redirections: &Redirections<'_>,
        env: &mut Environment<'_>,
    ) -> Result<RedirectTargets> {
        let stdin = redirections
            .stdin
            .map(|p| self.redirect_target(p, env))
            .transpose()?;
        let stdout = redirections
            .stdout
            .as_ref()
            .map(|r| Ok::<_, OasisError>((self.redirect_target(r.path, env)?, r.append)))
            .transpose()?;
        let stderr = redirections
            .stderr
            .as_ref()
            .map(|r| Ok::<_, OasisError>((self.redirect_target(r.path, env)?, r.append)))
            .transpose()?;
        Ok(RedirectTargets {
            stdin,
            stdout,
            stderr,
        })
    }

    /// Run `cmd` with the contents of `stdin_path` (if any) as its stdin.
    /// The outer stdin is restored afterwards, so it does not leak onward.
    fn run_with_stdin(
        &self,
        cmd: &str,
        stdin_path: Option<&str>,
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let Some(stdin_path) = stdin_path else {
            return self.execute_single_cmd(cmd, env);
        };
        let resolved = resolve_path(&env.cwd, stdin_path);
        let data = env.vfs.read(&resolved).map_err(|e| {
            OasisError::Command(format!("cannot redirect stdin from '{stdin_path}': {e}").into())
        })?;
        let outer = env
            .stdin
            .replace(String::from_utf8_lossy(&data).into_owned());
        let result = self.execute_single_cmd(cmd, env);
        env.stdin = outer;
        result
    }

    /// Expand a raw redirect target like an argument (quote removal,
    /// `$VAR`, `$(...)`; no globbing). It must name exactly one word.
    pub(crate) fn redirect_target(&self, raw: &str, env: &mut Environment<'_>) -> Result<String> {
        if raw.trim().is_empty() {
            return Err(OasisError::Command(
                "syntax error: missing file name after redirection".into(),
            ));
        }
        let mut words = self.expand_words_of(raw.trim(), env)?;
        match words.len() {
            1 => Ok(words.pop().map(|w| w.text).unwrap_or_default()),
            _ => Err(OasisError::Command(
                format!("{}: ambiguous redirect", raw.trim()).into(),
            )),
        }
    }
}

/// Send a command's stdout and stderr where its redirections say: `>&2`
/// moves stdout into stderr; `2>&1` (or stderr with no target of its own)
/// joins stderr to stdout; then each stream is written to its target file.
/// Returns what is left to show.
fn route_streams(
    captured: Captured,
    redirections: &Redirections<'_>,
    targets: RedirectTargets,
    env: &mut Environment<'_>,
) -> Result<CommandOutput> {
    let Captured {
        mut output,
        mut stderr,
        ..
    } = captured;

    if redirections.stdout_to_stderr {
        let text = capture_text(&output);
        if !text.is_empty() {
            stderr = join_lines(text, &stderr);
        }
        output = CommandOutput::None;
    }

    if (redirections.stderr_to_stdout || targets.stderr.is_none()) && !stderr.is_empty() {
        let stderr = std::mem::take(&mut stderr);
        output = match output {
            CommandOutput::Text(t) if !t.is_empty() => CommandOutput::Text(join_lines(t, &stderr)),
            CommandOutput::Text(_) | CommandOutput::None => CommandOutput::Text(stderr),
            other => other,
        };
    }

    if let Some((path, append)) = targets.stdout {
        write_redirect(&capture_text(&output), &path, append, &env.cwd, env.vfs)?;
        output = CommandOutput::None;
    }
    if let Some((path, append)) = targets.stderr {
        write_redirect(&stderr, &path, append, &env.cwd, env.vfs)?;
    }
    Ok(output)
}

/// `first`, a newline, then `second`.
pub(crate) fn join_lines(mut first: String, second: &str) -> String {
    if !second.is_empty() {
        if !first.is_empty() {
            first.push('\n');
        }
        first.push_str(second);
    }
    first
}
