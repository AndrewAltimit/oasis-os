//! `test` and `[`: file, string and integer conditions.

use oasis_types::error::{OasisError, Result};

use crate::interpreter::{CommandOutput, Environment};

// ---------------------------------------------------------------------------
// test / [
// ---------------------------------------------------------------------------

/// Evaluate a `test` expression: `! EXPR`, a unary file / string test, a
/// binary comparison, or a single non-empty string.
fn eval_test(args: &[&str], env: &mut Environment<'_>) -> Result<bool> {
    if let Some((&"!", rest)) = args.split_first() {
        return eval_test(rest, env).map(|b| !b);
    }
    let kind = |path: &str, env: &mut Environment<'_>| {
        let full = crate::interpreter::resolve_path(&env.cwd, path);
        env.vfs.stat(&full).ok().map(|m| m.kind)
    };
    let num = |s: &str| -> Result<i64> {
        s.trim()
            .parse()
            .map_err(|_| OasisError::Command(format!("test: {s}: integer expected").into()))
    };
    Ok(match args {
        [] => false,
        [s] => !s.is_empty(),
        ["-e", path] => kind(path, env).is_some(),
        ["-f", path] => kind(path, env) == Some(oasis_vfs::EntryKind::File),
        ["-d", path] => kind(path, env) == Some(oasis_vfs::EntryKind::Directory),
        ["-n", s] => !s.is_empty(),
        ["-z", s] => s.is_empty(),
        [a, op, b] => match *op {
            "=" | "==" => a == b,
            "!=" => a != b,
            "-eq" => num(a)? == num(b)?,
            "-ne" => num(a)? != num(b)?,
            "-lt" => num(a)? < num(b)?,
            "-le" => num(a)? <= num(b)?,
            "-gt" => num(a)? > num(b)?,
            "-ge" => num(a)? >= num(b)?,
            _ => {
                return Err(OasisError::Command(
                    format!("test: {op}: unknown operator").into(),
                ));
            },
        },
        _ => {
            return Err(OasisError::Command(
                format!("test: too many arguments: {}", args.join(" ")).into(),
            ));
        },
    })
}

/// Run `test` on `args`: no output, exit status 0 (true) or 1 (false).
/// A malformed expression is an error (status 1, message on the terminal).
fn run_test(args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
    let truth = eval_test(args, env)?;
    Ok(CommandOutput::status(
        i32::from(!truth),
        CommandOutput::None,
    ))
}

define_command!(
    TestCmd,
    "test",
    "Evaluate a condition; exit status 0 if true, 1 if false",
    "test [!] <-e|-f|-d path | -n|-z str | a =|!=|-eq|-ne|-lt|-le|-gt|-ge b | str>",
    "dev",
    |args, env| { run_test(args, env) }
);

define_command!(
    BracketCmd,
    "[",
    "Same as test; the last argument must be ]",
    "[ <expression> ]",
    "dev",
    |args, env| {
        match args.split_last() {
            Some((&"]", expr)) => run_test(expr, env),
            _ => Err(OasisError::Command("[: missing ']'".into())),
        }
    }
);

register_commands!(register, [TestCmd, BracketCmd]);
