//! Running a command that has I/O redirections (`<`, `<<<`, `>`, `>>`,
//! `2>`, `2>>`, `2>&1`, `>&2`, `&>`).
//!
//! [`CommandRegistry::execute_redirected`] runs in three steps: expand the
//! targets ([`RedirectTargets`]), run the command with any `<` file or
//! `<<<` word as its stdin, then route stdout / stderr to where the
//! redirections, applied left to right, left them ([`route_streams`]).

use oasis_types::error::{OasisError, Result};

use crate::expander::resolve_path;
use crate::pipeline::{Redirections, StdinSource, StreamDest, capture_text, write_redirect};
use crate::registry::CommandRegistry;
use crate::types::{CommandOutput, Environment};

/// The expanded redirect targets of one command.
struct RedirectTargets {
    /// The stdin text (`< path` contents or `<<< word`), if any.
    stdin: Option<String>,
    /// The expanded path of each of [`Redirections::files`], with its
    /// append flag.
    files: Vec<(String, bool)>,
}

/// What a finished command produced, before routing.
struct Captured {
    output: CommandOutput,
    stderr: String,
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
        if let Some(msg) = redirections.unsupported {
            return Err(OasisError::Command(msg.into()));
        }
        let targets = self.redirect_targets(redirections, env)?;
        env.stderr.clear();
        let result = self.run_with_stdin(cmd_part.trim(), targets.stdin, env);

        // Stderr left on the terminal and stdout not sent there: errors
        // propagate normally (the shell shows them and sets `$?`).
        let captures_stderr =
            redirections.stderr != StreamDest::Stderr || redirections.stdout == StreamDest::Stderr;
        if !captures_stderr {
            let output = match result {
                Ok(output) => output,
                Err(e) => {
                    // The files were opened before the command ran.
                    truncate_unused(&targets.files, &[], env)?;
                    return Err(e);
                },
            };
            return route_streams(
                Captured {
                    output,
                    stderr: String::new(),
                },
                redirections,
                &targets.files,
                env,
            );
        }

        let (captured, failed) = match result {
            Ok(output) => (
                Captured {
                    output,
                    stderr: std::mem::take(&mut env.stderr),
                },
                false,
            ),
            Err(e) => (
                Captured {
                    output: CommandOutput::None,
                    stderr: join_lines(std::mem::take(&mut env.stderr), &e.to_string()),
                },
                true,
            ),
        };
        let output = route_streams(captured, redirections, &targets.files, env)?;
        // Keep the failure's exit code even though its text was captured.
        if failed {
            self.last_exit_code.set(1);
            self.set_variable("?", "1");
        }
        Ok(output)
    }

    /// Expand every redirect target like an argument (`> $DIR/out.txt`,
    /// `> "my file"`, `<<< "$X"`) and read any `<` file.
    fn redirect_targets(
        &self,
        redirections: &Redirections<'_>,
        env: &mut Environment<'_>,
    ) -> Result<RedirectTargets> {
        let stdin = match redirections.stdin {
            None => None,
            Some(StdinSource::File(raw)) => {
                let path = self.redirect_target(raw, env)?;
                let resolved = resolve_path(&env.cwd, &path);
                let data = env.vfs.read(&resolved).map_err(|e| {
                    OasisError::Command(format!("cannot redirect stdin from '{path}': {e}").into())
                })?;
                Some(String::from_utf8_lossy(&data).into_owned())
            },
            Some(StdinSource::HereString(raw)) => {
                // No trailing newline, like every other text stream of
                // this shell (`echo hi | cmd` reads "hi").
                Some(self.redirect_target(raw, env)?)
            },
        };
        let files = redirections
            .files
            .iter()
            .map(|r| Ok::<_, OasisError>((self.redirect_target(r.path, env)?, r.append)))
            .collect::<Result<Vec<_>>>()?;
        Ok(RedirectTargets { stdin, files })
    }

    /// Run `cmd` with `stdin` (if any) as its stdin. The outer stdin is
    /// restored afterwards, so it does not leak onward.
    fn run_with_stdin(
        &self,
        cmd: &str,
        stdin: Option<String>,
        env: &mut Environment<'_>,
    ) -> Result<CommandOutput> {
        let Some(stdin) = stdin else {
            return self.execute_single_cmd(cmd, env);
        };
        let outer = env.stdin.replace(stdin);
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

/// Send a command's stdout and stderr where its redirections left them.
/// Text bound for the terminal (original stdout or stderr) is returned,
/// stdout first; each file gets the text of every stream that ends in it.
fn route_streams(
    captured: Captured,
    redirections: &Redirections<'_>,
    files: &[(String, bool)],
    env: &mut Environment<'_>,
) -> Result<CommandOutput> {
    let Captured { output, stderr } = captured;
    let on_terminal = |d: StreamDest| !matches!(d, StreamDest::File(_));

    let mut written: Vec<usize> = Vec::new();
    let to_file = matches!(redirections.stdout, StreamDest::File(_));
    let stdout_text = if to_file || on_terminal(redirections.stderr) {
        capture_text(&output)
    } else {
        String::new()
    };
    for (idx, (path, append)) in files.iter().enumerate() {
        let here = StreamDest::File(idx);
        if redirections.stdout != here && redirections.stderr != here {
            continue;
        }
        let mut text = String::new();
        if redirections.stdout == here {
            text = stdout_text.clone();
        }
        if redirections.stderr == here {
            text = join_lines(text, &stderr);
        }
        write_redirect(&text, path, *append, &env.cwd, env.vfs)?;
        written.push(idx);
    }
    truncate_unused(files, &written, env)?;

    let stdout_shown = on_terminal(redirections.stdout);
    let stderr_shown = on_terminal(redirections.stderr) && !stderr.is_empty();
    Ok(match (stdout_shown, stderr_shown) {
        (true, false) => output,
        (false, false) => CommandOutput::None,
        (false, true) => CommandOutput::Text(stderr),
        (true, true) => match output {
            CommandOutput::Text(_) | CommandOutput::None => {
                CommandOutput::Text(join_lines(stdout_text, &stderr))
            },
            // Structured output (a table, a signal) is kept as is; the
            // stderr text joins it only when it is text.
            other => other,
        },
    })
}

/// Create / truncate the opened files no stream ended up in
/// (`echo x > a > b` empties `a`); `written` are the ones already written.
fn truncate_unused(
    files: &[(String, bool)],
    written: &[usize],
    env: &mut Environment<'_>,
) -> Result<()> {
    for (idx, (path, append)) in files.iter().enumerate() {
        if written.contains(&idx) {
            continue;
        }
        let resolved = resolve_path(&env.cwd, path.trim());
        if *append && env.vfs.exists(&resolved) {
            continue;
        }
        write_redirect("", path, false, &env.cwd, env.vfs)?;
    }
    Ok(())
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
