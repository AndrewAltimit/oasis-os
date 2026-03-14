//! File and archive utility commands: write, append, tree, du, stat, xxd, checksum.

use oasis_types::error::{OasisError, Result};
use oasis_vfs::EntryKind;

use crate::cmd_helpers::require_args;
use crate::interpreter::{Command, CommandOutput, Environment, resolve_path};

// ---------------------------------------------------------------------------
// write
// ---------------------------------------------------------------------------

struct WriteCmd;
impl Command for WriteCmd {
    fn name(&self) -> &str {
        "write"
    }
    fn description(&self) -> &str {
        "Write text to a file"
    }
    fn usage(&self) -> &str {
        "write <file> <text...>"
    }
    fn category(&self) -> &str {
        "filesystem"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        require_args(args, 2, "write <file> <text...>")?;
        let path = resolve_path(&env.cwd, args[0]);
        let text = args[1..].join(" ");
        env.vfs.write(&path, text.as_bytes())?;
        Ok(CommandOutput::Text(format!(
            "Wrote {} bytes to {path}",
            text.len()
        )))
    }
}

// ---------------------------------------------------------------------------
// append
// ---------------------------------------------------------------------------

struct AppendCmd;
impl Command for AppendCmd {
    fn name(&self) -> &str {
        "append"
    }
    fn description(&self) -> &str {
        "Append text to a file"
    }
    fn usage(&self) -> &str {
        "append <file> <text...>"
    }
    fn category(&self) -> &str {
        "filesystem"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        require_args(args, 2, "append <file> <text...>")?;
        let path = resolve_path(&env.cwd, args[0]);
        let text = args[1..].join(" ");
        let mut data = if env.vfs.exists(&path) {
            env.vfs.read(&path)?
        } else {
            Vec::new()
        };
        data.extend_from_slice(b"\n");
        data.extend_from_slice(text.as_bytes());
        env.vfs.write(&path, &data)?;
        Ok(CommandOutput::Text(format!(
            "Appended {} bytes to {path}",
            text.len()
        )))
    }
}

// ---------------------------------------------------------------------------
// tree
// ---------------------------------------------------------------------------

struct TreeCmd;
impl Command for TreeCmd {
    fn name(&self) -> &str {
        "tree"
    }
    fn description(&self) -> &str {
        "Display directory tree"
    }
    fn usage(&self) -> &str {
        "tree [path]"
    }
    fn category(&self) -> &str {
        "filesystem"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let root = if args.is_empty() {
            env.cwd.clone()
        } else {
            resolve_path(&env.cwd, args[0])
        };
        let mut lines = vec![root.clone()];
        let mut dirs = 0u32;
        let mut files = 0u32;
        tree_recursive(env, &root, "", &mut lines, &mut dirs, &mut files, 0)?;
        lines.push(format!("\n{dirs} directories, {files} files"));
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

/// Maximum recursion depth for tree/du to prevent stack overflow.
const MAX_DEPTH: usize = 64;

fn tree_recursive(
    env: &mut Environment<'_>,
    dir: &str,
    prefix: &str,
    lines: &mut Vec<String>,
    dirs: &mut u32,
    files: &mut u32,
    depth: usize,
) -> Result<()> {
    if depth >= MAX_DEPTH {
        lines.push(format!("{prefix}... (max depth reached)"));
        return Ok(());
    }
    let entries = env.vfs.readdir(dir)?;
    let count = entries.len();
    for (i, entry) in entries.iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let suffix = if entry.kind == EntryKind::Directory {
            "/"
        } else {
            ""
        };
        lines.push(format!("{prefix}{connector}{}{suffix}", entry.name));

        if entry.kind == EntryKind::Directory {
            *dirs += 1;
            let child_path = if dir == "/" {
                format!("/{}", entry.name)
            } else {
                format!("{}/{}", dir, entry.name)
            };
            let child_prefix = if is_last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}│   ")
            };
            tree_recursive(
                env,
                &child_path,
                &child_prefix,
                lines,
                dirs,
                files,
                depth + 1,
            )?;
        } else {
            *files += 1;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// du
// ---------------------------------------------------------------------------

struct DuCmd;
impl Command for DuCmd {
    fn name(&self) -> &str {
        "du"
    }
    fn description(&self) -> &str {
        "Show disk usage of files/directories"
    }
    fn usage(&self) -> &str {
        "du [path]"
    }
    fn category(&self) -> &str {
        "filesystem"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let root = if args.is_empty() {
            env.cwd.clone()
        } else {
            resolve_path(&env.cwd, args[0])
        };
        let meta = env.vfs.stat(&root)?;
        if meta.kind == EntryKind::File {
            return Ok(CommandOutput::Text(format!(
                "{:>8}  {root}",
                format_size(meta.size)
            )));
        }
        let mut lines = Vec::new();
        let total = du_recursive(env, &root, &mut lines, 0)?;
        lines.push(format!("{:>8}  {root} (total)", format_size(total)));
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

fn du_recursive(
    env: &mut Environment<'_>,
    dir: &str,
    lines: &mut Vec<String>,
    depth: usize,
) -> Result<u64> {
    if depth >= MAX_DEPTH {
        return Ok(0);
    }
    let entries = env.vfs.readdir(dir)?;
    let mut total = 0u64;
    for entry in &entries {
        let path = if dir == "/" {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", dir, entry.name)
        };
        if entry.kind == EntryKind::Directory {
            let sub_total = du_recursive(env, &path, lines, depth + 1)?;
            total += sub_total;
        } else {
            total += entry.size;
        }
    }
    lines.push(format!("{:>8}  {dir}", format_size(total)));
    Ok(total)
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ---------------------------------------------------------------------------
// stat
// ---------------------------------------------------------------------------

struct StatCmd;
impl Command for StatCmd {
    fn name(&self) -> &str {
        "stat"
    }
    fn description(&self) -> &str {
        "Show file metadata"
    }
    fn usage(&self) -> &str {
        "stat <path>"
    }
    fn category(&self) -> &str {
        "filesystem"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        require_args(args, 1, "stat <path>")?;
        let path = resolve_path(&env.cwd, args[0]);
        let meta = env.vfs.stat(&path)?;
        let kind = match meta.kind {
            EntryKind::File => "regular file",
            EntryKind::Directory => "directory",
        };
        let mut lines = Vec::new();
        lines.push(format!("  File: {path}"));
        lines.push(format!("  Type: {kind}"));
        lines.push(format!(
            "  Size: {} ({})",
            meta.size,
            format_size(meta.size)
        ));
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

// ---------------------------------------------------------------------------
// xxd
// ---------------------------------------------------------------------------

struct XxdCmd;
impl Command for XxdCmd {
    fn name(&self) -> &str {
        "xxd"
    }
    fn description(&self) -> &str {
        "Hex dump a file"
    }
    fn usage(&self) -> &str {
        "xxd [-l N] <file>"
    }
    fn category(&self) -> &str {
        "filesystem"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let mut limit = 256usize;
        let mut file_arg = None;
        let mut i = 0;
        while i < args.len() {
            if args[i] == "-l" {
                i += 1;
                if i < args.len() {
                    limit = args[i]
                        .parse()
                        .map_err(|_| OasisError::Command("invalid length".into()))?;
                }
            } else {
                file_arg = Some(args[i]);
            }
            i += 1;
        }
        let file =
            file_arg.ok_or_else(|| OasisError::Command("usage: xxd [-l N] <file>".into()))?;
        let path = resolve_path(&env.cwd, file);
        let data = env.vfs.read(&path)?;
        let data = &data[..data.len().min(limit)];

        let mut lines = Vec::new();
        for (offset, chunk) in data.chunks(16).enumerate() {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
            let hex_str = hex.join(" ");
            let ascii: String = chunk
                .iter()
                .map(|&b| {
                    if (0x20..0x7f).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            lines.push(format!("{:08x}: {hex_str:<48} {ascii}", offset * 16));
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

// ---------------------------------------------------------------------------
// checksum
// ---------------------------------------------------------------------------

struct ChecksumCmd;
impl Command for ChecksumCmd {
    fn name(&self) -> &str {
        "checksum"
    }
    fn description(&self) -> &str {
        "Compute simple checksum of a file"
    }
    fn usage(&self) -> &str {
        "checksum <file>"
    }
    fn category(&self) -> &str {
        "filesystem"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        require_args(args, 1, "checksum <file>")?;
        let path = resolve_path(&env.cwd, args[0]);
        let data = env.vfs.read(&path)?;
        // Simple FNV-1a 32-bit hash (no external dependencies).
        let mut hash: u32 = 0x811c_9dc5;
        for &byte in &data {
            hash ^= byte as u32;
            hash = hash.wrapping_mul(0x0100_0193);
        }
        Ok(CommandOutput::Text(format!(
            "{hash:08x}  {path} ({} bytes)",
            data.len()
        )))
    }
}

register_commands!(
    register_file_commands,
    [
        WriteCmd,
        AppendCmd,
        TreeCmd,
        DuCmd,
        StatCmd,
        XxdCmd,
        ChecksumCmd,
    ]
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::assert_text;
    use crate::{CommandOutput, CommandRegistry, Environment};
    use oasis_vfs::{MemoryVfs, Vfs};

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_file_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        (reg, vfs)
    }

    fn exec(reg: &CommandRegistry, vfs: &mut MemoryVfs, line: &str) -> Result<CommandOutput> {
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        reg.execute(line, &mut env)
    }

    #[test]
    fn write_creates_file() {
        let (reg, mut vfs) = setup();
        exec(&reg, &mut vfs, "write /tmp/out.txt hello world").unwrap();
        let data = vfs.read("/tmp/out.txt").unwrap();
        assert_eq!(data, b"hello world");
    }

    #[test]
    fn append_to_file() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/log.txt", b"line1").unwrap();
        exec(&reg, &mut vfs, "append /tmp/log.txt line2").unwrap();
        let data = vfs.read("/tmp/log.txt").unwrap();
        let text = String::from_utf8_lossy(&data);
        assert!(text.contains("line1"));
        assert!(text.contains("line2"));
    }

    #[test]
    fn tree_basic() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/a.txt", b"data").unwrap();
        vfs.mkdir("/tmp/sub").unwrap();
        vfs.write("/tmp/sub/b.txt", b"data2").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "tree /tmp").unwrap());
        assert!(s.contains("a.txt"));
        assert!(s.contains("sub"));
        assert!(s.contains("b.txt"));
        assert!(s.contains("directories"));
        assert!(s.contains("files"));
    }

    #[test]
    fn du_basic() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/file.txt", b"12345").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "du /tmp/file.txt").unwrap());
        assert!(s.contains("5B"));
    }

    #[test]
    fn stat_file() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/x.txt", b"hello").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "stat /tmp/x.txt").unwrap());
        assert!(s.contains("regular file"));
        assert!(s.contains("5"));
    }

    #[test]
    fn stat_dir() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "stat /tmp").unwrap());
        assert!(s.contains("directory"));
    }

    #[test]
    fn xxd_hex_dump() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/bin.dat", b"ABCD").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "xxd /tmp/bin.dat").unwrap());
        assert!(s.contains("41 42 43 44"));
        assert!(s.contains("ABCD"));
    }

    #[test]
    fn checksum_deterministic() {
        let (reg, mut vfs) = setup();
        vfs.write("/tmp/ck.txt", b"test data").unwrap();
        let a = assert_text!(exec(&reg, &mut vfs, "checksum /tmp/ck.txt").unwrap());
        let b = assert_text!(exec(&reg, &mut vfs, "checksum /tmp/ck.txt").unwrap());
        assert_eq!(a, b);
        assert!(a.contains("9 bytes"));
    }
}
