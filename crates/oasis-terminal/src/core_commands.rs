//! Core filesystem and shell commands: ls, cd, pwd, cat, mkdir, rm, echo, clear,
//! status, touch, cp, mv, find.

use oasis_types::error::{OasisError, Result};
use oasis_vfs::EntryKind;

use crate::cmd_helpers::{FlagSpec, parse_flags, require_args};
use crate::interpreter::{CommandOutput, resolve_path};

/// Maximum file size for `cat` display (10 MiB).
const CAT_MAX_SIZE: usize = 10 * 1024 * 1024;

/// Maximum file size for `cp`/`mv` operations (100 MiB).
const COPY_MAX_SIZE: usize = 100 * 1024 * 1024;

register_commands!(
    register_core_commands,
    [
        LsCmd, CdCmd, PwdCmd, CatCmd, MkdirCmd, RmCmd, EchoCmd, ClearCmd, StatusCmd, TouchCmd,
        CpCmd, MvCmd, FindCmd,
    ]
);

// ---------------------------------------------------------------------------
// ls
// ---------------------------------------------------------------------------

const LS_USAGE: &str = "ls [-al] [path...]";

/// `text`, or no output at all when it is empty (so an empty listing adds
/// no blank line to a command list's output).
fn text_or_none(text: String) -> CommandOutput {
    if text.is_empty() {
        CommandOutput::None
    } else {
        CommandOutput::Text(text)
    }
}

/// One `ls` listing of `path`: names (directories as `name/`, in the
/// skin's bright-blue ANSI slot, SGR 94, which the shell UI resolves via
/// its ANSI palette and pipes / `$(...)` / redirects strip), dotfiles only
/// with `all`, `kind size name` rows with `long`.
fn ls_lines(
    vfs: &mut dyn oasis_vfs::Vfs,
    path: &str,
    all: bool,
    long: bool,
) -> Result<Vec<String>> {
    let meta = vfs.stat(path)?;
    if meta.kind != EntryKind::Directory {
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        return Ok(vec![if long {
            format!("-  {:>8}  {name}", meta.size)
        } else {
            name
        }]);
    }
    let mut lines = Vec::new();
    for e in vfs.readdir(path)? {
        if !all && e.name.starts_with('.') {
            continue;
        }
        let shown = if e.kind == EntryKind::Directory {
            crate::pipeline::dir_colored(&format!("{}/", e.name))
        } else {
            e.name.clone()
        };
        lines.push(match (long, e.kind) {
            (false, _) => shown,
            (true, EntryKind::Directory) => format!("d  {:>8}  {shown}", "-"),
            (true, EntryKind::File) => format!("-  {:>8}  {shown}", e.size),
        });
    }
    Ok(lines)
}

define_command!(
    LsCmd,
    "ls",
    "List directory contents",
    LS_USAGE,
    "filesystem",
    |args, env| {
        let flags = parse_flags(args, &FlagSpec::switches("al"), LS_USAGE)?;
        let (all, long) = (flags.has('a'), flags.has('l'));
        let paths: Vec<String> = if flags.positional.is_empty() {
            vec![env.cwd.clone()]
        } else {
            flags
                .positional
                .iter()
                .map(|p| resolve_path(&env.cwd, p))
                .collect()
        };
        if let [path] = &paths[..] {
            let lines = ls_lines(env.vfs, path, all, long)?;
            return Ok(text_or_none(lines.join("\n")));
        }
        let mut sections = Vec::new();
        for path in &paths {
            let lines = ls_lines(env.vfs, path, all, long)?;
            let mut section = format!("{path}:");
            for line in lines {
                section.push('\n');
                section.push_str(&line);
            }
            sections.push(section);
        }
        Ok(CommandOutput::Text(sections.join("\n\n")))
    }
);

// ---------------------------------------------------------------------------
// cd
// ---------------------------------------------------------------------------

// `cd` with no argument (`$HOME`) and `cd -` (`$OLDPWD`) need shell
// variables, which commands cannot see: the executor rewrites those forms
// before dispatch and records `OLDPWD` afterwards
// (`CommandRegistry::run_cd`). This command sees a plain path.
define_command!(
    CdCmd,
    "cd",
    "Change working directory",
    "cd [path|-]",
    "filesystem",
    |args, env| {
        let target = match args {
            [] => "/".to_string(),
            [path] => resolve_path(&env.cwd, path),
            _ => return Err(OasisError::Command("cd: too many arguments".into())),
        };
        let meta = env.vfs.stat(&target)?;
        if meta.kind != EntryKind::Directory {
            return Err(OasisError::Command(
                format!("not a directory: {target}").into(),
            ));
        }
        env.cwd = target;
        Ok(CommandOutput::None)
    }
);

// ---------------------------------------------------------------------------
// pwd
// ---------------------------------------------------------------------------

define_command!(
    PwdCmd,
    "pwd",
    "Print working directory",
    "pwd",
    "filesystem",
    |_args, env| { Ok(CommandOutput::Text(env.cwd.clone())) }
);

// ---------------------------------------------------------------------------
// cat
// ---------------------------------------------------------------------------

define_command!(
    CatCmd,
    "cat",
    "Display file contents",
    "cat [file]",
    "filesystem",
    |args, env| {
        // `echo x | cat`, `cat < f`, `cat <<< word`: pass stdin through.
        if args.is_empty()
            && let Some(stdin) = env.stdin.as_ref()
        {
            return Ok(CommandOutput::Text(stdin.clone()));
        }
        require_args(args, 1, "cat [file]")?;
        let path = resolve_path(&env.cwd, args[0]);
        let meta = env.vfs.stat(&path)?;
        if meta.size as usize > CAT_MAX_SIZE {
            return Err(OasisError::Command(
                format!("file too large ({} bytes, max {})", meta.size, CAT_MAX_SIZE).into(),
            ));
        }
        let data = env.vfs.read(&path)?;
        let text = String::from_utf8_lossy(&data).into_owned();
        Ok(CommandOutput::Text(text))
    }
);

// ---------------------------------------------------------------------------
// mkdir
// ---------------------------------------------------------------------------

define_command!(
    MkdirCmd,
    "mkdir",
    "Create a directory",
    MKDIR_USAGE,
    "filesystem",
    |args, env| {
        let flags = parse_flags(args, &FlagSpec::switches("p"), MKDIR_USAGE)?;
        require_args(&flags.positional, 1, MKDIR_USAGE)?;
        for arg in &flags.positional {
            let path = resolve_path(&env.cwd, arg);
            if flags.has('p') {
                mkdir_parents(env.vfs, &path)?;
            } else {
                env.vfs.mkdir(&path)?;
            }
        }
        Ok(CommandOutput::None)
    }
);

const MKDIR_USAGE: &str = "mkdir [-p] <path>...";

/// `mkdir -p`: create `path` and any missing ancestors; an existing
/// directory is fine, an existing file is an error.
fn mkdir_parents(vfs: &mut dyn oasis_vfs::Vfs, path: &str) -> Result<()> {
    let mut prefix = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        prefix.push('/');
        prefix.push_str(part);
        match vfs.stat(&prefix) {
            Ok(meta) if meta.kind == EntryKind::Directory => {},
            Ok(_) => {
                return Err(OasisError::Command(
                    format!("mkdir: not a directory: {prefix}").into(),
                ));
            },
            Err(_) => vfs.mkdir(&prefix)?,
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// rm
// ---------------------------------------------------------------------------

define_command!(
    RmCmd,
    "rm",
    "Remove files or directories",
    RM_USAGE,
    "filesystem",
    |args, env| {
        let flags = parse_flags(args, &FlagSpec::switches("rRf"), RM_USAGE)?;
        let (recursive, force) = (flags.has('r') || flags.has('R'), flags.has('f'));
        if !force {
            require_args(&flags.positional, 1, RM_USAGE)?;
        }
        for arg in &flags.positional {
            let path = resolve_path(&env.cwd, arg);
            if path == "/" {
                return Err(OasisError::Command("rm: refusing to remove '/'".into()));
            }
            let meta = match env.vfs.stat(&path) {
                Ok(meta) => meta,
                Err(_) if force => continue,
                Err(e) => return Err(e),
            };
            if recursive && meta.kind == EntryKind::Directory && !env.vfs.is_symlink(&path) {
                remove_tree(env.vfs, &path, 0)?;
            } else {
                env.vfs.remove(&path)?;
            }
        }
        Ok(CommandOutput::None)
    }
);

const RM_USAGE: &str = "rm [-rf] <path>...";

/// Directory depth `rm -r` descends before giving up.
const MAX_REMOVE_DEPTH: usize = 64;

/// Remove `dir` and everything under it, deepest first.
fn remove_tree(vfs: &mut dyn oasis_vfs::Vfs, dir: &str, depth: usize) -> Result<()> {
    if depth >= MAX_REMOVE_DEPTH {
        return Err(OasisError::Command(
            format!("rm: {dir}: nested deeper than {MAX_REMOVE_DEPTH}").into(),
        ));
    }
    for entry in vfs.readdir(dir)? {
        let child = format!("{}/{}", dir.trim_end_matches('/'), entry.name);
        // A link to a directory lists as a directory: remove the link,
        // never the files of its target (as GNU `rm -r` does).
        if entry.kind == EntryKind::Directory && !vfs.is_symlink(&child) {
            remove_tree(vfs, &child, depth + 1)?;
        } else {
            vfs.remove(&child)?;
        }
    }
    vfs.remove(dir)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// echo
// ---------------------------------------------------------------------------

define_command!(
    EchoCmd,
    "echo",
    "Print text",
    "echo [text...]",
    "general",
    |args, _env| { Ok(CommandOutput::Text(args.join(" "))) }
);

// ---------------------------------------------------------------------------
// clear
// ---------------------------------------------------------------------------

define_command!(
    ClearCmd,
    "clear",
    "Clear terminal output",
    "clear",
    "general",
    |_args, _env| { Ok(CommandOutput::Clear) }
);

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

define_command!(
    StatusCmd,
    "status",
    "Show system status",
    "status",
    "general",
    |_args, env| {
        let mut lines = Vec::new();
        lines.push(concat!("OASIS_OS v", env!("CARGO_PKG_VERSION")).to_string());
        lines.push(format!("cwd: {}", env.cwd));
        match env.vfs.readdir("/") {
            Ok(entries) => lines.push(format!("root entries: {}", entries.len())),
            Err(_) => lines.push("root entries: (error reading)".to_string()),
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }
);

// ---------------------------------------------------------------------------
// touch
// ---------------------------------------------------------------------------

define_command!(
    TouchCmd,
    "touch",
    "Create an empty file",
    "touch <file>...",
    "filesystem",
    |args, env| {
        require_args(args, 1, "touch <file>...")?;
        for arg in args {
            let path = resolve_path(&env.cwd, arg);
            if !env.vfs.exists(&path) {
                env.vfs.write(&path, &[])?;
            }
        }
        Ok(CommandOutput::None)
    }
);

// ---------------------------------------------------------------------------
// cp
// ---------------------------------------------------------------------------

/// Resolve the destination of `cp`/`mv`: an existing directory receives the
/// source under its own base name (`mv a.txt dir` -> `dir/a.txt`).
fn transfer_target(env: &crate::interpreter::Environment<'_>, src: &str, dst: String) -> String {
    let dst_is_dir = env
        .vfs
        .stat(&dst)
        .is_ok_and(|m| m.kind == EntryKind::Directory);
    if !dst_is_dir || dst == src {
        return dst;
    }
    let base = src.rsplit('/').next().unwrap_or(src);
    if dst == "/" {
        format!("/{base}")
    } else {
        format!("{dst}/{base}")
    }
}

fn same_file_error(src: &str, dst: &str) -> OasisError {
    OasisError::Command(format!("'{src}' and '{dst}' are the same file").into())
}

define_command!(
    CpCmd,
    "cp",
    "Copy a file",
    "cp <src> <dst>",
    "filesystem",
    |args, env| {
        require_args(args, 2, "cp <src> <dst>")?;
        let src = resolve_path(&env.cwd, args[0]);
        let meta = env.vfs.stat(&src)?;
        if meta.kind == EntryKind::Directory {
            return Err(OasisError::Command(
                format!("{src} is a directory (not copied)").into(),
            ));
        }
        let dst = transfer_target(env, &src, resolve_path(&env.cwd, args[1]));
        if dst == src {
            return Err(same_file_error(&src, &dst));
        }
        if meta.size as usize > COPY_MAX_SIZE {
            return Err(OasisError::Command(
                format!(
                    "file too large ({} bytes, max {})",
                    meta.size, COPY_MAX_SIZE
                )
                .into(),
            ));
        }
        let data = env.vfs.read(&src)?;
        env.vfs.write(&dst, &data)?;
        Ok(CommandOutput::None)
    }
);

// ---------------------------------------------------------------------------
// mv
// ---------------------------------------------------------------------------

define_command!(
    MvCmd,
    "mv",
    "Move/rename a file or directory",
    "mv <src> <dst>",
    "filesystem",
    |args, env| {
        require_args(args, 2, "mv <src> <dst>")?;
        let src = resolve_path(&env.cwd, args[0]);
        // Fails (without side effects) when the source does not exist.
        env.vfs.stat(&src)?;
        if src == "/" {
            return Err(OasisError::Command("cannot move the root directory".into()));
        }
        let dst = transfer_target(env, &src, resolve_path(&env.cwd, args[1]));
        // Moving a path onto itself used to copy it and then delete the
        // source -- i.e. the only copy.
        if dst == src {
            return Err(same_file_error(&src, &dst));
        }
        if dst.starts_with(&format!("{src}/")) {
            return Err(OasisError::Command(
                format!("cannot move '{src}' into itself ('{dst}')").into(),
            ));
        }
        if env
            .vfs
            .stat(&dst)
            .is_ok_and(|m| m.kind == EntryKind::Directory)
        {
            return Err(OasisError::Command(
                format!("destination exists and is a directory: {dst}").into(),
            ));
        }
        // A rename moves files and whole directories without copying.
        env.vfs.rename(&src, &dst)?;
        Ok(CommandOutput::None)
    }
);

// ---------------------------------------------------------------------------
// find
// ---------------------------------------------------------------------------

define_command!(
    FindCmd,
    "find",
    "Find files by name pattern",
    "find [path] <pattern>",
    "filesystem",
    |args, env| {
        let (root, pattern) = match args.len() {
            0 => {
                return Err(OasisError::Command("usage: find [path] <pattern>".into()));
            },
            1 => (env.cwd.clone(), args[0]),
            _ => (resolve_path(&env.cwd, args[0]), args[1]),
        };
        // A pattern without glob characters keeps the historical
        // substring search (`find / log` finds `syslog.txt`).
        let pattern = if pattern.contains(['*', '?', '[']) {
            pattern.to_string()
        } else {
            format!("*{pattern}*")
        };
        let mut results = Vec::new();
        find_recursive(env.vfs, &root, &pattern, &mut results, 0)?;
        Ok(text_or_none(results.join("\n")))
    }
);

/// Recursively search for entries whose name matches glob `pattern`.
fn find_recursive(
    vfs: &mut dyn oasis_vfs::Vfs,
    dir: &str,
    pattern: &str,
    results: &mut Vec<String>,
    depth: usize,
) -> Result<()> {
    if depth >= MAX_REMOVE_DEPTH {
        return Ok(());
    }
    let entries = vfs.readdir(dir)?;
    for entry in &entries {
        let full = if dir == "/" {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", dir, entry.name)
        };
        if crate::expander::glob_match(pattern, &entry.name) {
            results.push(full.clone());
        }
        if entry.kind == EntryKind::Directory {
            find_recursive(vfs, &full, pattern, results, depth + 1)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{assert_clear, assert_text};
    use crate::{CommandOutput, CommandRegistry, Environment};
    use oasis_vfs::{MemoryVfs, Vfs};

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_core_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home").unwrap();
        vfs.mkdir("/home/user").unwrap();
        vfs.write("/home/user/readme.txt", b"Hello OASIS").unwrap();
        (reg, vfs)
    }

    fn exec(
        reg: &CommandRegistry,
        vfs: &mut MemoryVfs,
        cwd: &mut String,
        line: &str,
    ) -> Result<CommandOutput> {
        let mut env = Environment {
            cwd: cwd.clone(),
            vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        let result = reg.execute(line, &mut env);
        *cwd = env.cwd;
        result
    }

    #[test]
    fn pwd_shows_cwd() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/home".to_string();
        assert_eq!(
            assert_text!(exec(&reg, &mut vfs, &mut cwd, "pwd").unwrap()),
            "/home"
        );
    }

    #[test]
    fn ls_root() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        assert!(assert_text!(exec(&reg, &mut vfs, &mut cwd, "ls").unwrap()).contains("home"));
    }

    #[test]
    fn ls_with_path() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        assert!(
            assert_text!(exec(&reg, &mut vfs, &mut cwd, "ls /home/user").unwrap())
                .contains("readme.txt")
        );
    }

    #[test]
    fn cd_and_pwd() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        exec(&reg, &mut vfs, &mut cwd, "cd /home/user").unwrap();
        assert_eq!(cwd, "/home/user");
    }

    #[test]
    fn cd_relative() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/home".to_string();
        exec(&reg, &mut vfs, &mut cwd, "cd user").unwrap();
        assert_eq!(cwd, "/home/user");
    }

    #[test]
    fn cd_dotdot() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/home/user".to_string();
        exec(&reg, &mut vfs, &mut cwd, "cd ..").unwrap();
        assert_eq!(cwd, "/home");
    }

    #[test]
    fn cat_file() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        assert_eq!(
            assert_text!(exec(&reg, &mut vfs, &mut cwd, "cat /home/user/readme.txt").unwrap()),
            "Hello OASIS"
        );
    }

    #[test]
    fn cat_no_args() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        assert!(exec(&reg, &mut vfs, &mut cwd, "cat").is_err());
    }

    #[test]
    fn mkdir_creates_dir() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        exec(&reg, &mut vfs, &mut cwd, "mkdir /tmp").unwrap();
        assert!(vfs.exists("/tmp"));
    }

    #[test]
    fn rm_removes_file() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        exec(&reg, &mut vfs, &mut cwd, "rm /home/user/readme.txt").unwrap();
        assert!(!vfs.exists("/home/user/readme.txt"));
    }

    #[test]
    fn echo_output() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        assert_eq!(
            assert_text!(exec(&reg, &mut vfs, &mut cwd, "echo hello world").unwrap()),
            "hello world"
        );
    }

    #[test]
    fn clear_returns_clear() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        assert_clear!(exec(&reg, &mut vfs, &mut cwd, "clear").unwrap());
    }

    #[test]
    fn status_shows_info() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        let s = assert_text!(exec(&reg, &mut vfs, &mut cwd, "status").unwrap());
        assert!(s.contains("OASIS_OS"));
        assert!(s.contains("cwd: /"));
    }

    #[test]
    fn touch_creates_file() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        exec(&reg, &mut vfs, &mut cwd, "mkdir /tmp").unwrap();
        exec(&reg, &mut vfs, &mut cwd, "touch /tmp/new.txt").unwrap();
        assert!(vfs.exists("/tmp/new.txt"));
        assert_eq!(vfs.read("/tmp/new.txt").unwrap(), b"");
    }

    #[test]
    fn cp_copies_file() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        exec(
            &reg,
            &mut vfs,
            &mut cwd,
            "cp /home/user/readme.txt /home/user/copy.txt",
        )
        .unwrap();
        assert!(vfs.exists("/home/user/copy.txt"));
        assert_eq!(vfs.read("/home/user/copy.txt").unwrap(), b"Hello OASIS");
        // Original still exists.
        assert!(vfs.exists("/home/user/readme.txt"));
    }

    #[test]
    fn cp_no_args() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        assert!(exec(&reg, &mut vfs, &mut cwd, "cp").is_err());
    }

    #[test]
    fn mv_moves_file() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        exec(
            &reg,
            &mut vfs,
            &mut cwd,
            "mv /home/user/readme.txt /home/moved.txt",
        )
        .unwrap();
        assert!(!vfs.exists("/home/user/readme.txt"));
        assert!(vfs.exists("/home/moved.txt"));
        assert_eq!(vfs.read("/home/moved.txt").unwrap(), b"Hello OASIS");
    }

    #[test]
    fn mv_no_args() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        assert!(exec(&reg, &mut vfs, &mut cwd, "mv").is_err());
    }

    #[test]
    fn find_by_name() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        assert!(
            assert_text!(exec(&reg, &mut vfs, &mut cwd, "find / readme").unwrap())
                .contains("/home/user/readme.txt")
        );
    }

    #[test]
    fn find_no_matches() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        // No placeholder text: `find` output feeds pipes and loops.
        let out = exec(&reg, &mut vfs, &mut cwd, "find / zzzzz").unwrap();
        assert!(matches!(out, CommandOutput::None), "{out:?}");
    }

    #[test]
    fn find_no_args() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        assert!(exec(&reg, &mut vfs, &mut cwd, "find").is_err());
    }
}
