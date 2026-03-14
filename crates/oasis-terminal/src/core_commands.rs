//! Core filesystem and shell commands: ls, cd, pwd, cat, mkdir, rm, echo, clear,
//! status, touch, cp, mv, find.

use oasis_types::error::{OasisError, Result};
use oasis_vfs::EntryKind;

use crate::cmd_helpers::require_args;
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

define_command!(
    LsCmd,
    "ls",
    "List directory contents",
    "ls [path]",
    "filesystem",
    |args, env| {
        let path = if args.is_empty() {
            env.cwd.clone()
        } else {
            resolve_path(&env.cwd, args[0])
        };
        let entries = env.vfs.readdir(&path)?;
        if entries.is_empty() {
            return Ok(CommandOutput::Text("(empty)".to_string()));
        }
        let mut lines = Vec::new();
        for e in &entries {
            let suffix = if e.kind == EntryKind::Directory {
                "/"
            } else {
                ""
            };
            lines.push(format!("{}{suffix}", e.name));
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }
);

// ---------------------------------------------------------------------------
// cd
// ---------------------------------------------------------------------------

define_command!(
    CdCmd,
    "cd",
    "Change working directory",
    "cd <path>",
    "filesystem",
    |args, env| {
        let target = if args.is_empty() {
            "/".to_string()
        } else {
            resolve_path(&env.cwd, args[0])
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
    "cat <file>",
    "filesystem",
    |args, env| {
        require_args(args, 1, "cat <file>")?;
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
    "mkdir <path>",
    "filesystem",
    |args, env| {
        require_args(args, 1, "mkdir <path>")?;
        let path = resolve_path(&env.cwd, args[0]);
        env.vfs.mkdir(&path)?;
        Ok(CommandOutput::None)
    }
);

// ---------------------------------------------------------------------------
// rm
// ---------------------------------------------------------------------------

define_command!(
    RmCmd,
    "rm",
    "Remove a file or empty directory",
    "rm <path>",
    "filesystem",
    |args, env| {
        require_args(args, 1, "rm <path>")?;
        let path = resolve_path(&env.cwd, args[0]);
        env.vfs.remove(&path)?;
        Ok(CommandOutput::None)
    }
);

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
        lines.push("OASIS_OS v0.1.0".to_string());
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
    "touch <file>",
    "filesystem",
    |args, env| {
        require_args(args, 1, "touch <file>")?;
        let path = resolve_path(&env.cwd, args[0]);
        if !env.vfs.exists(&path) {
            env.vfs.write(&path, &[])?;
        }
        Ok(CommandOutput::None)
    }
);

// ---------------------------------------------------------------------------
// cp
// ---------------------------------------------------------------------------

define_command!(
    CpCmd,
    "cp",
    "Copy a file",
    "cp <src> <dst>",
    "filesystem",
    |args, env| {
        require_args(args, 2, "cp <src> <dst>")?;
        let src = resolve_path(&env.cwd, args[0]);
        let dst = resolve_path(&env.cwd, args[1]);
        let meta = env.vfs.stat(&src)?;
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
    "Move/rename a file",
    "mv <src> <dst>",
    "filesystem",
    |args, env| {
        require_args(args, 2, "mv <src> <dst>")?;
        let src = resolve_path(&env.cwd, args[0]);
        let dst = resolve_path(&env.cwd, args[1]);
        let meta = env.vfs.stat(&src)?;
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
        env.vfs.remove(&src)?;
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
        let mut results = Vec::new();
        find_recursive(env.vfs, &root, pattern, &mut results)?;
        if results.is_empty() {
            Ok(CommandOutput::Text("(no matches)".to_string()))
        } else {
            Ok(CommandOutput::Text(results.join("\n")))
        }
    }
);

/// Recursively search for files matching a simple substring pattern.
fn find_recursive(
    vfs: &mut dyn oasis_vfs::Vfs,
    dir: &str,
    pattern: &str,
    results: &mut Vec<String>,
) -> Result<()> {
    let entries = vfs.readdir(dir)?;
    for entry in &entries {
        let full = if dir == "/" {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", dir, entry.name)
        };
        if entry.name.contains(pattern) {
            results.push(full.clone());
        }
        if entry.kind == EntryKind::Directory {
            find_recursive(vfs, &full, pattern, results)?;
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
        assert!(
            assert_text!(exec(&reg, &mut vfs, &mut cwd, "find / zzzzz").unwrap())
                .contains("no matches")
        );
    }

    #[test]
    fn find_no_args() {
        let (reg, mut vfs) = setup();
        let mut cwd = "/".to_string();
        assert!(exec(&reg, &mut vfs, &mut cwd, "find").is_err());
    }
}
