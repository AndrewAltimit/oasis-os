//! End-to-end shell sessions.
//!
//! These tests drive the terminal the way a host does: keystrokes go through
//! [`ShellSession`] (line editing, completion, history), submitted lines are
//! executed by the [`CommandRegistry`], background jobs are pumped with
//! `poll_jobs`, and history is persisted to / reloaded from the VFS. File
//! workflows run against both `MemoryVfs` and a `RealVfs` rooted in a
//! temporary directory, including path-traversal attempts that must not
//! escape the root.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use oasis_terminal::{
    CommandOutput, CommandRegistry, Environment, SessionEvent, ShellSession, register_builtins,
};
use oasis_types::input::{Button, InputEvent, Key, Modifiers};
use oasis_vfs::{MemoryVfs, RealVfs, Vfs};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Flatten any output into display text (signals are rendered as debug).
fn text_of(out: &CommandOutput) -> String {
    match out {
        CommandOutput::Text(s) => s.clone(),
        CommandOutput::Table { headers, rows } => {
            let mut s = headers.join(" | ");
            for r in rows {
                s.push('\n');
                s.push_str(&r.join(" | "));
            }
            s
        },
        CommandOutput::None | CommandOutput::Clear => String::new(),
        CommandOutput::Signal(sig) => format!("{sig:?}"),
        CommandOutput::Multi(parts) => parts.iter().map(text_of).collect::<Vec<_>>().join("\n"),
    }
}

/// A terminal as a host drives it: registry + VFS + cwd + line session.
struct Shell {
    reg: CommandRegistry,
    vfs: Box<dyn Vfs>,
    cwd: String,
    session: ShellSession,
}

impl Shell {
    fn new(vfs: Box<dyn Vfs>) -> Self {
        let mut reg = CommandRegistry::new();
        register_builtins(&mut reg);
        Self {
            reg,
            vfs,
            cwd: "/".to_string(),
            session: ShellSession::with_history_path("/home/user/.oasis_history"),
        }
    }

    fn memory() -> Self {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home/user").unwrap();
        vfs.mkdir("/tmp").unwrap();
        Self::new(Box::new(vfs))
    }

    /// Run a line directly (no keystrokes).
    fn run(&mut self, line: &str) -> Result<String, String> {
        let mut env = Environment {
            cwd: self.cwd.clone(),
            vfs: self.vfs.as_mut(),
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        let res = self.reg.execute(line, &mut env);
        self.cwd = env.cwd;
        res.map(|o| text_of(&o)).map_err(|e| e.to_string())
    }

    /// Run a line that must succeed.
    fn ok(&mut self, line: &str) -> String {
        self.run(line)
            .unwrap_or_else(|e| panic!("`{line}` failed: {e}"))
    }

    /// Pump background jobs like a host does once per frame.
    fn tick(&mut self) -> Option<String> {
        let mut env = Environment {
            cwd: self.cwd.clone(),
            vfs: self.vfs.as_mut(),
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        let out = self.reg.poll_jobs(&mut env);
        self.cwd = env.cwd;
        out.map(|o| text_of(&o))
    }

    fn event(&mut self, ev: InputEvent) -> Option<SessionEvent> {
        self.session
            .handle_event(&ev, &self.reg, &self.cwd, self.vfs.as_ref())
    }

    fn key(&mut self, key: Key, mods: Modifiers) -> Option<SessionEvent> {
        self.session
            .handle_key(key, mods, &self.reg, &self.cwd, self.vfs.as_ref())
    }

    fn type_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.event(InputEvent::TextInput(ch));
        }
    }

    fn tab(&mut self) -> Option<SessionEvent> {
        self.event(InputEvent::Tab)
    }

    /// Press Enter; execute the submitted line like the host would.
    fn enter(&mut self) -> Result<String, String> {
        match self.event(InputEvent::ButtonPress(Button::Confirm)) {
            Some(SessionEvent::Submit(line)) => self.run(&line),
            other => panic!("Enter did not submit: {other:?}"),
        }
    }

    fn save_history(&mut self) {
        self.session
            .save_history(&self.reg, self.vfs.as_mut())
            .unwrap();
    }
}

/// A unique scratch directory under the OS temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "oasis-term-e2e-{tag}-{}-{nanos}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Sandbox layout: `<tmp>/outside/secret.txt` next to `<tmp>/root/` (the VFS
/// root), so any `..` escape from the root would be observable.
fn real_sandbox(tag: &str) -> (TempDir, Shell) {
    let tmp = TempDir::new(tag);
    let root = tmp.path().join("root");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(root.join("home/user")).unwrap();
    std::fs::create_dir_all(root.join("tmp")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), b"TOP SECRET").unwrap();
    let vfs = RealVfs::new(&root).unwrap();
    (tmp, Shell::new(Box::new(vfs)))
}

/// Every file (relative path) under `dir`, recursively.
fn snapshot(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            out.push(p.strip_prefix(base).unwrap().display().to_string());
            if p.is_dir() {
                walk(base, &p, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Interactive sessions: editing + execution + history
// ---------------------------------------------------------------------------

#[test]
fn typed_session_edits_executes_and_records_history() {
    let mut sh = Shell::memory();
    sh.cwd = "/home/user".into();

    // Type with a typo, fix it with cursor movement, then submit.
    sh.type_str("ecoh hello > greet.txt");
    sh.key(Key::Home, Modifiers::NONE);
    for _ in 0..3 {
        sh.event(InputEvent::ButtonPress(Button::Right));
    }
    // Readline transpose: swaps the char before the cursor with the one at it.
    sh.key(Key::Char('t'), Modifiers::CTRL); // "eco|h" -> "echo"
    assert_eq!(sh.session.buffer(), "echo hello > greet.txt");
    assert_eq!(sh.enter().unwrap(), "");
    assert_eq!(sh.vfs.read("/home/user/greet.txt").unwrap(), b"hello");

    // Kill / yank round trip, Ctrl+C abandons without executing.
    sh.type_str("rm greet.txt");
    assert_eq!(
        sh.key(Key::Char('c'), Modifiers::CTRL),
        Some(SessionEvent::Interrupted("rm greet.txt".into()))
    );
    assert!(sh.vfs.exists("/home/user/greet.txt"));

    sh.type_str("cat greet.txt");
    assert_eq!(sh.enter().unwrap().trim_end(), "hello");

    // Up recalls the most recent line; editing it produces a new command.
    sh.event(InputEvent::ButtonPress(Button::Up));
    assert_eq!(sh.session.buffer(), "cat greet.txt");
    // Ctrl+W stops at punctuation (readline backward-kill-word).
    sh.key(Key::Char('w'), Modifiers::CTRL);
    assert_eq!(sh.session.buffer(), "cat greet.");
    sh.key(Key::Char('u'), Modifiers::CTRL);
    sh.type_str("cat missing.txt");
    assert!(sh.enter().is_err(), "cat of a missing file must fail");

    // Reverse search finds the redirect line and it can be re-run.
    sh.key(Key::Char('r'), Modifiers::CTRL);
    sh.type_str("hello >");
    assert_eq!(sh.session.buffer(), "echo hello > greet.txt");
    sh.enter().unwrap();

    let hist = sh.reg.history();
    assert_eq!(
        hist,
        vec![
            "echo hello > greet.txt",
            "cat greet.txt",
            "cat missing.txt",
            "echo hello > greet.txt",
        ],
        "interrupted lines are not recorded"
    );
}

#[test]
fn unicode_line_editing_and_completion_do_not_panic() {
    let mut sh = Shell::memory();
    sh.vfs
        .write("/tmp/caf\u{e9}.txt", "\u{2615}".as_bytes())
        .unwrap();
    sh.cwd = "/tmp".into();

    sh.type_str("echo h\u{e9}llo \u{1F600} w\u{f6}rld");
    // Move around multi-byte characters, delete, word-kill, transpose.
    for _ in 0..3 {
        sh.event(InputEvent::ButtonPress(Button::Left));
    }
    sh.event(InputEvent::Backspace);
    sh.key(Key::Char('t'), Modifiers::CTRL);
    sh.key(Key::Char('w'), Modifiers::CTRL);
    sh.key(Key::Left, Modifiers::CTRL);
    sh.key(Key::Delete, Modifiers::NONE);
    // Tab with multi-byte text before the cursor.
    sh.tab();
    let (display, col) = sh.session.display();
    assert!(col <= display.chars().count());
    sh.key(Key::Char('u'), Modifiers::CTRL);
    sh.key(Key::Char('k'), Modifiers::CTRL);
    assert_eq!(sh.session.buffer(), "");

    // Completing a file with a multi-byte name.
    sh.type_str("cat caf");
    sh.tab();
    assert_eq!(sh.session.buffer(), "cat caf\u{e9}.txt ");
    assert_eq!(sh.enter().unwrap(), "\u{2615}");

    // Completion in the middle of a multi-byte line.
    sh.type_str("echo \u{e9}\u{e9} | c");
    sh.tab();
    sh.type_str("\u{1F600}");
    sh.key(Key::Home, Modifiers::NONE);
    sh.tab();
    sh.key(Key::End, Modifiers::NONE);
    sh.tab();
}

#[test]
fn history_persists_across_sessions_memory_vfs() {
    let mut first = Shell::memory();
    for line in [
        "mkdir /home/user/proj",
        "cd /home/user/proj",
        "pwd",
        "echo done",
    ] {
        first.type_str(line);
        first.enter().unwrap();
    }
    // An interactive `history` also records itself.
    first.type_str("history");
    let listing = first.enter().unwrap();
    assert!(listing.contains("mkdir /home/user/proj"), "{listing}");
    first.save_history();

    let data = first.vfs.read("/home/user/.oasis_history").unwrap();
    let text = String::from_utf8(data).unwrap();
    assert_eq!(
        text,
        "mkdir /home/user/proj\ncd /home/user/proj\npwd\necho done\nhistory\n"
    );

    // A brand-new registry + session over the same VFS picks it up.
    let vfs = std::mem::replace(&mut first.vfs, Box::new(MemoryVfs::new()));
    let mut second = Shell::new(vfs);
    assert_eq!(
        second
            .session
            .load_history(&second.reg, second.vfs.as_ref()),
        5
    );
    second.event(InputEvent::ButtonPress(Button::Up));
    second.event(InputEvent::ButtonPress(Button::Up));
    assert_eq!(second.session.buffer(), "echo done");
    assert_eq!(second.enter().unwrap(), "done");
    // `!1` expands against the reloaded history.
    assert_eq!(second.ok("!3").trim(), "/");
}

#[test]
fn history_persists_across_sessions_real_vfs() {
    let (tmp, mut sh) = real_sandbox("hist");
    sh.type_str("echo persisted");
    sh.enter().unwrap();
    sh.type_str("ls /");
    sh.enter().unwrap();
    sh.save_history();
    drop(sh);

    let on_disk = tmp.path().join("root/home/user/.oasis_history");
    assert_eq!(
        std::fs::read_to_string(&on_disk).unwrap(),
        "echo persisted\nls /\n"
    );

    // Reopen the VFS from disk: a new process would see the same history.
    let vfs = RealVfs::new(tmp.path().join("root")).unwrap();
    let mut again = Shell::new(Box::new(vfs));
    assert_eq!(
        again.session.load_history(&again.reg, again.vfs.as_ref()),
        2
    );
    again.key(Key::Char('p'), Modifiers::CTRL);
    again.key(Key::Char('p'), Modifiers::CTRL);
    assert_eq!(again.session.buffer(), "echo persisted");
}

#[test]
fn history_file_with_junk_is_sanitised_on_load() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home/user").unwrap();
    let mut junk = String::new();
    for i in 0..700 {
        junk.push_str(&format!("  echo {i}  \r\n\n"));
    }
    junk.push_str("\u{0}\u{7f}\u{fffd}\n");
    vfs.write("/home/user/.oasis_history", junk.as_bytes())
        .unwrap();
    let sh = Shell::new(Box::new(vfs));
    let n = sh.session.load_history(&sh.reg, sh.vfs.as_ref());
    assert_eq!(n, 500, "history is capped");
    let hist = sh.reg.history();
    assert!(hist.iter().all(|h| !h.is_empty() && h == h.trim()));
    assert_eq!(hist[hist.len() - 2], "echo 699");
}

#[test]
fn history_save_creates_missing_directory() {
    let mut sh = Shell::new(Box::new(MemoryVfs::new()));
    sh.type_str("echo x");
    sh.enter().unwrap();
    sh.save_history();
    assert!(sh.vfs.exists("/home/user/.oasis_history"));
}

// ---------------------------------------------------------------------------
// Tab completion
// ---------------------------------------------------------------------------

#[test]
fn tab_completes_commands_paths_and_variables() {
    let mut sh = Shell::memory();
    sh.vfs.mkdir("/home/user/documents").unwrap();
    sh.vfs.mkdir("/home/user/downloads").unwrap();
    sh.vfs.write("/home/user/notes.txt", b"n").unwrap();
    sh.cwd = "/home/user".into();

    // Unique command prefix -> full name + space.
    sh.type_str("pw");
    sh.tab();
    assert_eq!(sh.session.buffer(), "pwd ");
    assert_eq!(sh.enter().unwrap(), "/home/user");

    // Absolute path through several levels.
    sh.type_str("ls /ho");
    sh.tab();
    assert_eq!(sh.session.buffer(), "ls /home/");
    sh.type_str("u");
    sh.tab();
    assert_eq!(sh.session.buffer(), "ls /home/user/");
    sh.type_str("no");
    sh.tab();
    assert_eq!(sh.session.buffer(), "ls /home/user/notes.txt ");
    sh.key(Key::Char('u'), Modifiers::CTRL);

    // Ambiguous relative path: extend to the common prefix, then list.
    sh.type_str("cd d");
    sh.tab();
    assert_eq!(sh.session.buffer(), "cd do");
    match sh.tab() {
        Some(SessionEvent::Candidates(c)) => {
            assert!(c.contains(&"documents/".to_string()), "{c:?}");
            assert!(c.contains(&"downloads/".to_string()), "{c:?}");
        },
        other => panic!("expected candidates, got {other:?}"),
    }
    sh.type_str("w");
    sh.tab();
    assert_eq!(sh.session.buffer(), "cd downloads/");
    sh.enter().unwrap();
    assert_eq!(sh.cwd, "/home/user/downloads");

    // `..` relative completion.
    sh.type_str("cat ../no");
    sh.tab();
    assert_eq!(sh.session.buffer(), "cat ../notes.txt ");
    assert_eq!(sh.enter().unwrap(), "n");

    // Command position after a pipe.
    sh.type_str("echo a | hea");
    sh.tab();
    assert_eq!(sh.session.buffer(), "echo a | head ");
    sh.key(Key::Char('u'), Modifiers::CTRL);

    // Variables, including user-defined ones.
    sh.ok("set PROJECT_DIR=/home/user");
    sh.type_str("echo $PROJECT_D");
    sh.tab();
    assert!(
        sh.session.buffer().starts_with("echo $PROJECT_DIR"),
        "{}",
        sh.session.buffer()
    );
    assert_eq!(sh.enter().unwrap(), "/home/user");

    // Aliases and functions complete as commands.
    sh.ok("alias zzlist=ls");
    sh.type_str("zzl");
    sh.tab();
    assert_eq!(sh.session.buffer(), "zzlist ");
    sh.key(Key::Char('u'), Modifiers::CTRL);

    // No match: buffer untouched, no panic.
    sh.type_str("cat /nope/nothing");
    sh.tab();
    assert_eq!(sh.session.buffer(), "cat /nope/nothing");
}

#[test]
fn tab_completion_against_real_vfs() {
    let (_tmp, mut sh) = real_sandbox("tab");
    sh.ok("write /home/user/report.md hi");
    sh.ok("mkdir /home/user/reports");
    sh.cwd = "/home/user".into();
    sh.type_str("cat rep");
    sh.tab();
    assert_eq!(sh.session.buffer(), "cat report");
    // Completion must not list anything outside the VFS root.
    sh.key(Key::Char('u'), Modifiers::CTRL);
    sh.type_str("cat ../../../out");
    sh.tab();
    assert_eq!(sh.session.buffer(), "cat ../../../out");
}

// ---------------------------------------------------------------------------
// Scripting
// ---------------------------------------------------------------------------

const DEPLOY_SCRIPT: &str = r#"#!/bin/oasis
# Build a small project tree and report on it.
set PROJECT=/home/user/app
mkdir $PROJECT
for name in alpha beta gamma; do
  echo "module $name" > $PROJECT/$name.rs
done
set count=0
while test $count -lt 3; do
  append $PROJECT/log.txt tick-$count
  set count=$(expr $count + 1)
done
alias lsp="ls $PROJECT"
for f in $PROJECT/*.rs; do
  case $f in
    *alpha*) echo "first: $f" ;;
    *gamma*) echo "last: $f" ;;
    *) echo "other: $f" ;;
  esac
done
if test -f $PROJECT/beta.rs; then
  echo beta present
elif test -d $PROJECT; then
  echo only dir
else
  echo nothing
fi
cat $PROJECT/log.txt | grep -c tick
echo done > /tmp/deploy.status
"#;

#[test]
fn run_script_file_with_control_flow() {
    for vfs_kind in ["memory", "real"] {
        let (_tmp, mut sh) = match vfs_kind {
            "memory" => (None, Shell::memory()),
            _ => {
                let (t, s) = real_sandbox("script");
                (Some(t), s)
            },
        };
        sh.vfs
            .write("/home/user/deploy.sh", DEPLOY_SCRIPT.as_bytes())
            .unwrap();
        sh.cwd = "/home/user".into();
        let out = sh.ok("run deploy.sh");

        assert!(
            out.contains("first: /home/user/app/alpha.rs"),
            "{vfs_kind}: {out}"
        );
        assert!(
            out.contains("other: /home/user/app/beta.rs"),
            "{vfs_kind}: {out}"
        );
        assert!(
            out.contains("last: /home/user/app/gamma.rs"),
            "{vfs_kind}: {out}"
        );
        assert!(out.contains("beta present"), "{vfs_kind}: {out}");
        assert!(!out.contains("only dir"), "{vfs_kind}: {out}");
        assert!(
            out.lines().any(|l| l.trim() == "3"),
            "grep -c over the loop log: {vfs_kind}: {out}"
        );
        assert_eq!(
            sh.vfs.read("/home/user/app/beta.rs").unwrap(),
            b"module beta",
            "{vfs_kind}"
        );
        assert_eq!(
            sh.vfs.read("/tmp/deploy.status").unwrap(),
            b"done",
            "{vfs_kind}"
        );
        // Variables and aliases set by the script persist in the shell.
        assert_eq!(sh.ok("echo $PROJECT"), "/home/user/app");
        let listing = sh.ok("lsp");
        assert!(listing.contains("gamma.rs"), "{vfs_kind}: {listing}");
    }
}

#[test]
fn script_errors_are_reported_and_execution_continues() {
    let mut sh = Shell::memory();
    sh.vfs
        .write(
            "/tmp/s.sh",
            b"echo before\ncat /does/not/exist\necho after\nnosuchcommand arg\necho end\n",
        )
        .unwrap();
    let out = sh.ok("run /tmp/s.sh");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.first(), Some(&"before"), "{out}");
    assert!(out.contains("error at line 2"), "{out}");
    assert!(out.contains("after"), "{out}");
    assert!(out.contains("error at line 4"), "{out}");
    assert_eq!(lines.last(), Some(&"end"), "{out}");
}

#[test]
fn script_syntax_errors_abort_cleanly() {
    let mut sh = Shell::memory();
    for (i, src) in [
        "if true; then echo x",
        "for x in a b; do echo $x",
        "while true; do echo y",
        "case a in a) echo a ;;",
        "fi",
        "done",
        "esac",
        "then echo",
        "if; then; fi",
    ]
    .iter()
    .enumerate()
    {
        let path = format!("/tmp/bad{i}.sh");
        sh.vfs.write(&path, src.as_bytes()).unwrap();
        let res = sh.run(&format!("run {path}"));
        assert!(
            res.is_err(),
            "`{src}` should be a syntax error, got {res:?}"
        );
    }
    // The shell is still usable.
    assert_eq!(sh.ok("echo ok"), "ok");
}

#[test]
fn runaway_loops_are_bounded() {
    let mut sh = Shell::memory();
    let started = std::time::Instant::now();
    // `while true` is capped by the interpreter; must terminate.
    let _ = sh.run("while true; do echo spin > /tmp/spin; done");
    let _ = sh.run("until false; do set X=1; done");
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    assert_eq!(sh.ok("echo alive"), "alive");
}

#[test]
fn self_recursive_script_is_bounded() {
    // Run on a 1 MiB stack: the Windows default for a host's main thread.
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(self_recursive_script_is_bounded_inner)
        .unwrap()
        .join()
        .unwrap();
}

fn self_recursive_script_is_bounded_inner() {
    // A script that runs itself must hit a depth limit, not overflow the
    // stack (which would abort the whole host process).
    let mut sh = Shell::memory();
    sh.vfs
        .write("/tmp/loop.sh", b"echo level\nrun /tmp/loop.sh\n")
        .unwrap();
    let res = sh.run("run /tmp/loop.sh");
    let text = match res {
        Ok(t) | Err(t) => t,
    };
    assert!(
        text.to_lowercase().contains("depth"),
        "expected a depth-limit error, got: {}",
        &text[..text.len().min(300)]
    );
    // Mutually recursive scripts too.
    sh.vfs.write("/tmp/a.sh", b"run /tmp/b.sh\n").unwrap();
    sh.vfs.write("/tmp/b.sh", b"run /tmp/a.sh\n").unwrap();
    let _ = sh.run("run /tmp/a.sh");
    assert_eq!(sh.ok("echo alive"), "alive");
}

#[test]
fn recursive_aliases_and_functions_are_bounded() {
    // Same 1 MiB stack as a Windows host main thread.
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(recursive_aliases_and_functions_are_bounded_inner)
        .unwrap()
        .join()
        .unwrap();
}

fn recursive_aliases_and_functions_are_bounded_inner() {
    let mut sh = Shell::memory();
    sh.ok("alias loop1=loop2");
    sh.ok("alias loop2=loop1");
    let _ = sh.run("loop1");
    sh.ok("alias ls='ls /'");
    assert!(sh.ok("ls").contains("home"));
    sh.ok("function rec() { rec; }");
    let res = sh.run("rec");
    assert!(res.is_err() || res.as_ref().is_ok_and(|t| t.contains("depth")));
    // Command substitution nesting.
    let _ = sh.run("echo $(echo $(echo $(echo $(echo deep))))");
    assert_eq!(sh.ok("echo alive"), "alive");
}

#[test]
fn inline_pipelines_redirection_and_globs() {
    let mut sh = Shell::memory();
    sh.cwd = "/home/user".into();
    sh.ok("write fruits.txt banana");
    sh.ok("append fruits.txt apple");
    sh.ok("append fruits.txt cherry");
    sh.ok("append fruits.txt apple");

    assert_eq!(sh.ok("cat fruits.txt | sort | uniq | head -n 1"), "apple");
    assert_eq!(
        sh.ok("sort fruits.txt | uniq -c | grep -c apple").trim(),
        "1"
    );
    sh.ok("cat fruits.txt | grep an > matches.txt");
    assert_eq!(sh.ok("cat matches.txt").trim(), "banana");
    sh.ok("echo extra >> matches.txt");
    assert_eq!(sh.ok("wc -l < matches.txt").trim(), "2");

    for i in 0..5 {
        sh.ok(&format!("touch log{i}.txt"));
    }
    sh.ok("touch other.md");
    let out = sh.ok("echo log*.txt");
    assert_eq!(out.split_whitespace().count(), 5, "{out}");
    let out = sh.ok("echo log?.txt other.*");
    assert_eq!(out.split_whitespace().count(), 6, "{out}");
    // Redirect targets expand variables and drop quotes.
    sh.ok("set OUT=/tmp");
    sh.ok("echo quoted > \"$OUT/my file.txt\"");
    assert_eq!(sh.ok("cat \"/tmp/my file.txt\""), "quoted");
    sh.ok("echo more >> ${OUT}/my\\ file2.txt");
    assert_eq!(sh.ok("wc -l < $OUT/my\\ file2.txt").trim(), "1");
    // `case` supports `?` and character classes, not just a single `*`.
    let out = sh.ok("case log3.txt in log[0-2].txt) echo low ;; log?.txt) echo other ;; esac");
    assert_eq!(out, "other");
    // A glob with no match is passed through literally.
    assert_eq!(sh.ok("echo *.nomatch"), "*.nomatch");
    // Brace expansion + chaining.
    sh.ok("mkdir /tmp/b && touch /tmp/b/{x,y,z}.dat");
    assert_eq!(sh.ok("ls /tmp/b").split_whitespace().count(), 3);
    let res = sh.run("false && echo no");
    assert!(res.as_ref().is_ok_and(|t| !t.contains("no")), "{res:?}");
    assert_eq!(sh.ok("false; echo $?"), "1");
    assert_eq!(sh.ok("true && echo yes"), "yes");
    // `test` verdicts drive && / || (not just `if`).
    let out = sh.ok("test -f /nope/missing && echo yes");
    assert!(!out.contains("yes"), "{out}");
    let out = sh.ok("test -f /nope/missing || echo fallback");
    assert!(out.ends_with("fallback"), "{out}");
    let out = sh.ok("test -d /tmp && echo isdir");
    assert!(out.ends_with("isdir"), "{out}");
    sh.ok("while true; do append /tmp/w.txt x; break; done");
    assert_eq!(sh.ok("cat /tmp/w.txt"), "x", "loop body ran exactly once");
    // The failure is reported inline, then the `||` branch runs.
    let out = sh.ok("cat /nope || echo recovered");
    assert!(out.ends_with("recovered"), "{out}");
    assert_eq!(sh.ok("cat /nope; echo $?").lines().last(), Some("1"));
}

// ---------------------------------------------------------------------------
// Background jobs
// ---------------------------------------------------------------------------

#[test]
fn background_jobs_lifecycle() {
    let mut sh = Shell::memory();
    let queued = sh.ok("echo bg-one > /tmp/one.txt &");
    assert!(queued.starts_with("[1]"), "{queued}");
    sh.ok("echo bg-two > /tmp/two.txt &");
    sh.ok("echo bg-three > /tmp/three.txt &");
    // Nothing has run yet.
    assert!(!sh.vfs.exists("/tmp/one.txt"));
    let jobs = sh.ok("jobs");
    assert_eq!(jobs.lines().count(), 3, "{jobs}");

    // Stop job 2, kill job 3.
    sh.ok("kill -STOP %2");
    sh.ok("kill %3");
    let jobs = sh.ok("jobs");
    assert!(jobs.contains("Stopped"), "{jobs}");
    assert!(!jobs.contains("three"), "{jobs}");

    // One tick runs the oldest runnable job only.
    let done = sh.tick().expect("job 1 runs");
    assert!(done.contains("Done"), "{done}");
    assert_eq!(sh.vfs.read("/tmp/one.txt").unwrap(), b"bg-one");
    assert!(sh.tick().is_none(), "job 2 is stopped");
    assert!(!sh.vfs.exists("/tmp/two.txt"));

    // bg makes it runnable again; fg would run it synchronously.
    sh.ok("bg %2");
    assert!(sh.tick().is_some());
    assert_eq!(sh.vfs.read("/tmp/two.txt").unwrap(), b"bg-two");
    assert!(!sh.vfs.exists("/tmp/three.txt"), "killed job never runs");
    assert!(sh.tick().is_none());

    // fg on a queued job runs it now and echoes the command.
    sh.ok("echo now > /tmp/fg.txt &");
    let fg = sh.ok("fg %%");
    assert!(fg.contains("echo now"), "{fg}");
    assert!(sh.vfs.exists("/tmp/fg.txt"));

    // Error handling for bad specs.
    assert!(sh.run("fg %99").is_err());
    assert!(sh.run("bg %nope").is_err());
    assert!(sh.run("kill %42").is_err());
    // A failing background job reports its failure without panicking.
    sh.ok("cat /missing &");
    let out = sh.tick().unwrap();
    assert!(out.contains("Exit") || out.contains("error"), "{out}");
}

#[test]
fn many_background_jobs_drain_in_order() {
    let mut sh = Shell::memory();
    for i in 0..50 {
        sh.ok(&format!("append /tmp/order.txt {i} &"));
    }
    let mut ticks = 0;
    while sh.tick().is_some() {
        ticks += 1;
        assert!(ticks <= 50);
    }
    assert_eq!(ticks, 50);
    let expected: Vec<String> = (0..50).map(|i| i.to_string()).collect();
    let data = String::from_utf8(sh.vfs.read("/tmp/order.txt").unwrap()).unwrap();
    assert_eq!(data.lines().collect::<Vec<_>>(), expected);
}

// ---------------------------------------------------------------------------
// File workflows
// ---------------------------------------------------------------------------

fn file_workflow(sh: &mut Shell) {
    sh.cwd = "/home/user".into();
    sh.ok("mkdir work");
    sh.ok("cd work");
    assert_eq!(sh.ok("pwd"), "/home/user/work");
    sh.ok("write a.txt hello world");
    sh.ok("append a.txt second line");
    sh.ok("cp a.txt b.txt");
    assert_eq!(sh.ok("cat b.txt"), sh.ok("cat a.txt"));
    assert_eq!(
        sh.ok("checksum b.txt"),
        sh.ok("checksum a.txt").replace("a.txt", "b.txt")
    );
    sh.ok("mkdir sub");
    sh.ok("mv b.txt sub/c.txt");
    assert!(sh.run("cat b.txt").is_err());
    assert!(sh.ok("cat sub/c.txt").contains("second line"));

    let found = sh.ok("find /home/user c.txt");
    assert_eq!(found.trim(), "/home/user/work/sub/c.txt");
    assert_eq!(sh.ok("grep -c line sub/c.txt").trim(), "1");
    assert_eq!(sh.ok("grep -n hello a.txt").trim(), "1:hello world");
    assert!(sh.ok("tree /home/user").contains("c.txt"));
    assert!(sh.ok("du /home/user").contains("total"));

    // rm refuses non-empty directories, then succeeds once emptied.
    assert!(sh.run("rm sub").is_err());
    sh.ok("rm sub/c.txt");
    sh.ok("rm sub");
    assert!(!sh.vfs.exists("/home/user/work/sub"));

    // `cd ..` / `cd` edge cases.
    sh.ok("cd ../../..");
    assert_eq!(sh.cwd, "/");
    sh.ok("cd ../../../../..");
    assert_eq!(sh.cwd, "/");
    assert!(sh.run("cd /home/user/work/a.txt").is_err());
    assert_eq!(sh.cwd, "/");
}

#[test]
fn file_workflow_memory_vfs() {
    let mut sh = Shell::memory();
    file_workflow(&mut sh);
}

#[test]
fn file_workflow_real_vfs() {
    let (tmp, mut sh) = real_sandbox("files");
    file_workflow(&mut sh);
    // The same bytes are visible on disk.
    let on_disk = std::fs::read(tmp.path().join("root/home/user/work/a.txt")).unwrap();
    assert_eq!(on_disk, b"hello world\nsecond line");
}

fn same_path_ops(sh: &mut Shell) {
    sh.ok("write /tmp/keep.txt precious");
    // mv / cp onto itself must not lose data.
    let _ = sh.run("mv /tmp/keep.txt /tmp/keep.txt");
    assert_eq!(sh.ok("cat /tmp/keep.txt"), "precious");
    let _ = sh.run("mv /tmp/keep.txt /tmp/../tmp/./keep.txt");
    assert_eq!(sh.ok("cat /tmp/keep.txt"), "precious");
    let _ = sh.run("cp /tmp/keep.txt /tmp/keep.txt");
    assert_eq!(sh.ok("cat /tmp/keep.txt"), "precious");
}

#[test]
fn mv_onto_itself_keeps_file_memory_vfs() {
    same_path_ops(&mut Shell::memory());
}

#[test]
fn mv_onto_itself_keeps_file_real_vfs() {
    let (_tmp, mut sh) = real_sandbox("mvself");
    same_path_ops(&mut sh);
}

fn move_into_directory(sh: &mut Shell) {
    sh.ok("write /tmp/m.txt payload");
    sh.ok("mkdir /tmp/dest");
    sh.ok("mv /tmp/m.txt /tmp/dest");
    assert_eq!(sh.ok("cat /tmp/dest/m.txt"), "payload");
    assert!(!sh.vfs.exists("/tmp/m.txt"));
    sh.ok("cp /tmp/dest/m.txt /tmp");
    assert_eq!(sh.ok("cat /tmp/m.txt"), "payload");
    // Renaming a directory.
    sh.ok("mv /tmp/dest /tmp/renamed");
    assert_eq!(sh.ok("cat /tmp/renamed/m.txt"), "payload");
    assert!(!sh.vfs.exists("/tmp/dest"));
}

#[test]
fn mv_and_cp_into_directory_memory_vfs() {
    move_into_directory(&mut Shell::memory());
}

#[test]
fn mv_and_cp_into_directory_real_vfs() {
    let (_tmp, mut sh) = real_sandbox("mvdir");
    move_into_directory(&mut sh);
}

#[test]
fn real_vfs_path_traversal_is_refused() {
    let (tmp, mut sh) = real_sandbox("traversal");
    let outside = tmp.path().join("outside");
    let before = snapshot(tmp.path());
    let secret_abs = outside.join("secret.txt").display().to_string();
    let secret_fwd = secret_abs.replace('\\', "/");

    let reads = [
        "cat ../outside/secret.txt".to_string(),
        "cat ../../outside/secret.txt".to_string(),
        "cat /../outside/secret.txt".to_string(),
        "cat /home/../../outside/secret.txt".to_string(),
        "cat ..\\outside\\secret.txt".to_string(),
        "cat /..\\..\\outside\\secret.txt".to_string(),
        format!("cat {secret_abs}"),
        format!("cat {secret_fwd}"),
        "head ../../outside/secret.txt".to_string(),
        "grep SECRET ../../outside/secret.txt".to_string(),
        "xxd ../../outside/secret.txt".to_string(),
        "checksum ../../outside/secret.txt".to_string(),
        "cp ../../outside/secret.txt /tmp/stolen.txt".to_string(),
        "cat < ../../outside/secret.txt".to_string(),
        "ls ../../outside".to_string(),
        "ls ..\\..\\outside".to_string(),
        "find ../.. secret".to_string(),
        "tree ../..".to_string(),
        "stat ../../outside/secret.txt".to_string(),
        "run ../../outside/secret.txt".to_string(),
    ];
    for line in &reads {
        for cwd in ["/", "/home/user"] {
            sh.cwd = cwd.into();
            let out = match sh.run(line) {
                Ok(t) | Err(t) => t,
            };
            assert!(
                !out.contains("TOP SECRET"),
                "`{line}` (cwd {cwd}) leaked outside data: {out}"
            );
        }
    }
    assert!(
        !sh.vfs.exists("/tmp/stolen.txt")
            || !String::from_utf8_lossy(&sh.vfs.read("/tmp/stolen.txt").unwrap())
                .contains("TOP SECRET")
    );

    let writes = [
        "write ../../outside/evil.txt pwned".to_string(),
        "write ..\\..\\outside\\evil.txt pwned".to_string(),
        "echo pwned > ../../outside/evil.txt".to_string(),
        "echo pwned >> ../../outside/secret.txt".to_string(),
        "touch /../../outside/evil.txt".to_string(),
        "mkdir ../../outside/evil_dir".to_string(),
        "rm ../../outside/secret.txt".to_string(),
        "mv /tmp ../../outside/moved".to_string(),
        "cp /home ../../outside/copied".to_string(),
        format!("write {}/evil.txt pwned", outside.display()),
        format!(
            "write {}/evil.txt pwned",
            outside.display().to_string().replace('\\', "/")
        ),
        "echo pwned | tee ../../outside/evil.txt".to_string(),
    ];
    for line in &writes {
        sh.cwd = "/home/user".into();
        let _ = sh.run(line);
    }
    assert_eq!(
        std::fs::read(outside.join("secret.txt")).unwrap(),
        b"TOP SECRET",
        "secret modified"
    );
    let after = snapshot(tmp.path());
    let escaped: Vec<&String> = after
        .iter()
        .filter(|p| !p.starts_with("root") && !before.contains(p))
        .collect();
    assert!(
        escaped.is_empty(),
        "files created outside the root: {escaped:?}"
    );
    // `..` is clamped at the VFS root, so the "outside" writes landed inside
    // the root instead (e.g. `/outside/evil_dir`, `/tmp` moved to
    // `/outside/moved`); the tree is otherwise intact.
    assert!(sh.vfs.exists("/tmp") || sh.vfs.exists("/outside/moved"));
    assert!(sh.vfs.exists("/home/user"));
}

#[test]
fn memory_vfs_dotdot_clamps_at_root() {
    let mut sh = Shell::memory();
    sh.ok("write /../../../../tmp/x.txt clamp");
    assert_eq!(sh.ok("cat /tmp/x.txt"), "clamp");
    sh.cwd = "/home/user".into();
    assert_eq!(sh.ok("cat ../../../../../tmp/x.txt"), "clamp");
    sh.ok("cd ../../../../..");
    assert_eq!(sh.ok("pwd"), "/");
}
