# Adding Terminal Commands

This guide explains how to add new commands to the OASIS_OS terminal interpreter.

---

## Command Architecture

Commands implement the `Command` trait defined in `crates/oasis-terminal/src/types.rs`. The `CommandRegistry` holds all registered commands and dispatches execution by name. Commands receive parsed arguments and a mutable `Environment` providing access to the VFS, platform services, and piped input.

---

## The Command Trait

```rust
pub trait Command {
    /// The command name (what the user types).
    fn name(&self) -> &str;

    /// One-line description for `help`.
    fn description(&self) -> &str;

    /// Usage string (e.g. "ls [path]").
    fn usage(&self) -> &str;

    /// Command category for grouping in `help` output.
    /// Default: "general". Common categories: core, file, system, dev,
    /// fun, security, text, audio, network, skin, ui, plugin, doc.
    fn category(&self) -> &str { "general" }

    /// Execute the command with the given arguments and environment.
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput>;
}
```

Source: `crates/oasis-terminal/src/types.rs`

---

## CommandOutput Variants

Commands return a `CommandOutput` enum:

| Variant | Description |
|---------|-------------|
| `Text(String)` | Plain text output (most common) |
| `Table { headers, rows }` | Tabular data with header row and data rows |
| `None` | No visible output |
| `Clear` | Signal to clear the terminal buffer |
| `Signal(CommandSignal)` | A signal to the app layer (see below) |
| `Multi(Vec<CommandOutput>)` | Multiple outputs from chained commands |
| `Status { code, output }` | `output` plus an explicit exit status; build it with `CommandOutput::status(code, output)`. Use it to fail (non-zero `code`) while still printing, e.g. `test` prints `false` and exits 1. The executor records the code for `&&` / `\|\|` / `$?` / `if` and unwraps it, so hosts never see it. Returning `Err` is still the normal way to fail with an error message (status 1). |

`CommandSignal` variants (intercepted by the app layer to trigger system-level actions):

| Signal | Description |
|--------|-------------|
| `ListenToggle { port }` | Start/stop remote terminal |
| `RemoteConnect { address, port, psk }` | Connect to remote host |
| `SkinSwap { name }` | Swap the active skin |
| `FtpToggle { port, password }` | Start/stop FTP server (optional password auth) |
| `BrowserSandbox { enable }` | Toggle browser sandbox mode |
| `McpToggle { start, port, token }` | Start/stop the optional MCP control server |
| `SdiInspect { name }` | Describe the host's SDI scene (`sdi list` / `sdi get`); hosts resolve it with `terminal_sdi::inspect_sdi` |

Use `Text` for most commands. If the command talks to the host through a
VFS request/status file, take the path (and, where one exists, the typed
request such as `MediaRequest`) from `oasis_types::ipc` instead of a string
literal. Convenience constructors like `CommandOutput::listen_toggle(port)` and `CommandOutput::skin_swap(name)` are provided for creating signal variants.

---

## The Environment Struct

```rust
pub struct Environment<'a> {
    pub cwd: String,                          // current working directory
    pub vfs: &'a mut dyn Vfs,                 // virtual file system
    pub power: Option<&'a dyn PowerService>,  // battery/CPU queries
    pub time: Option<&'a dyn TimeService>,    // clock/uptime queries
    pub usb: Option<&'a dyn UsbService>,      // USB status
    pub network: Option<&'a dyn NetworkService>, // WiFi status
    pub tls: Option<&'a dyn TlsProvider>,     // TLS for HTTPS
    pub stdin: Option<String>,                // piped input from previous command
    pub stderr: String,                       // accumulated error output
}
```

Commands read files and directories through `env.vfs`. Platform services (`power`, `time`, `usb`, `network`) are `Option` because not every backend provides them.

---

## Tutorial: Add a Custom Command

### Step 1: Create the Command Struct

Create a new file or add to an existing command module in `crates/oasis-terminal/src/`:

```rust
// In crates/oasis-terminal/src/my_commands.rs

use oasis_types::error::{OasisError, Result};
use crate::types::{Command, CommandOutput, Environment};

struct WordCountCmd;

impl Command for WordCountCmd {
    fn name(&self) -> &str { "wc" }
    fn description(&self) -> &str { "Count words in a file" }
    fn usage(&self) -> &str { "wc <path>" }
    fn category(&self) -> &str { "text" }

    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        // Handle piped input.
        if let Some(ref input) = env.stdin {
            let count = input.split_whitespace().count();
            return Ok(CommandOutput::Text(format!("{count} words")));
        }

        // Require a file path argument.
        let path = args.first().copied().ok_or_else(|| {
            OasisError::Command("usage: wc <path>".to_string())
        })?;

        // Resolve relative paths against cwd.
        let full_path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("{}/{path}", env.cwd)
        };

        let data = env.vfs.read(&full_path)?;
        let text = String::from_utf8_lossy(&data);
        let words = text.split_whitespace().count();
        let lines = text.lines().count();
        let bytes = data.len();

        Ok(CommandOutput::Text(format!(
            "{lines} lines, {words} words, {bytes} bytes"
        )))
    }
}
```

### Step 2: Create a Registration Function

Follow the existing pattern -- each command module exports a `register_*` function:

```rust
pub fn register_my_commands(reg: &mut crate::CommandRegistry) {
    reg.register(Box::new(WordCountCmd));
}
```

### Step 3: Wire It Into the Module System

Add your module to `crates/oasis-terminal/src/lib.rs`:

```rust
mod my_commands;
pub use my_commands::register_my_commands;
```

Then call it from `crates/oasis-terminal/src/commands.rs` in `register_builtins()`:

```rust
pub fn register_builtins(reg: &mut CommandRegistry) {
    // ... existing registrations ...
    crate::register_my_commands(reg);
}
```

---

## Existing Command Modules

Commands are organized into modules by category:

| Module | Category | Examples |
|--------|----------|----------|
| `commands.rs` | - | `register_builtins` (registers every module below) |
| `core_commands.rs` | core | ls, cd, pwd, cat, mkdir, rm, cp, mv, find |
| `text_commands.rs` | text | grep, sort, head, tail, cut, tr, tee, diff |
| `file_commands.rs` | file | write, append, tree, du, stat, xxd, checksum |
| `system_commands.rs` | system | uptime, hostname, whoami, df, date, sleep |
| `dev_commands.rs` | dev | base64, json, uuid, seq, expr, test, xargs |
| `fun_commands.rs` | fun | cal, fortune, banner, matrix, yes, watch, time |
| `security_commands.rs` | security | chmod, chown, passwd, audit |
| `doc_commands.rs` | doc | man, tutorial, motd |
| `audio_commands.rs` | audio | music (play/pause/next/vol/repeat/shuffle via `/var/audio/request`) |
| `network_commands.rs` | network | wifi, ping, http |
| `ui_commands.rs` | ui | wm, sdi, theme, notify, screenshot |
| `radio_commands.rs` | audio | radio |
| `platform_commands.rs` | system | power, clock, memory, usb |
| `remote_commands.rs` | network | listen, remote, hosts |
| `control_flow.rs` / `script.rs` | core | `if`/`while`/`until`/`for`/`case` parser + `run` (shell syntax, not commands) |
| `builtins.rs` / `jobs.rs` / `script.rs` | - | the 20 shell builtins (help, which, history, set, unset, env, alias, unalias, run, function, return, local, break, continue, true, false, jobs, fg, bg, kill) |

Additional commands (agent, browser, plugin, script, skin, transfer, tv, update) are registered by `oasis-core`; `oasis-terminal` must not depend on the UI / skin stack, so a command that needs `oasis-skin` (like `skin`, in `oasis-core/src/terminal/skin_commands.rs`) belongs there.

### Shell builtins

Commands that need the registry's own state (variables, aliases, history,
functions, jobs) are shell builtins rather than `Command` impls. They live
in one table, `BUILTINS` in `crates/oasis-terminal/src/builtins.rs`: a
`BuiltinSpec { name, category, usage, description, run }` per builtin. The
executor dispatches from it before the command map, and `help`, `which`
and tab completion read it, so adding a builtin is one table entry plus
its `CommandRegistry` method. Builtins always win: registering a command
with a builtin's name has no effect.

---

## Testing Commands with Mock VFS

Use `MemoryVfs` for fast, isolated command tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vfs::MemoryVfs;

    fn make_env(vfs: &mut MemoryVfs) -> Environment<'_> {
        Environment {
            cwd: "/".to_string(),
            vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        }
    }

    #[test]
    fn wc_counts_words() {
        let mut vfs = MemoryVfs::new();
        vfs.write("/test.txt", b"hello world foo").unwrap();
        let mut env = make_env(&mut vfs);

        let cmd = WordCountCmd;
        match cmd.execute(&["test.txt"], &mut env).unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("3 words"));
            }
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn wc_piped_input() {
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        env.stdin = Some("one two three four".to_string());

        let cmd = WordCountCmd;
        match cmd.execute(&[], &mut env).unwrap() {
            CommandOutput::Text(s) => assert!(s.contains("4 words")),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn wc_missing_file() {
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);

        let cmd = WordCountCmd;
        assert!(cmd.execute(&["/nonexistent"], &mut env).is_err());
    }
}
```

---

## Shell Features Your Command Gets for Free

The interpreter handles these before your `execute()` is called:

- **Argument parsing** -- quoted strings, escape sequences
- **Variable expansion** -- `$HOME`, `$USER`, `$?` (last exit code)
- **Glob expansion** -- `*.txt` expanded against VFS
- **Alias resolution** -- user-defined aliases
- **Piping** -- `cat file.txt | wc` sets `env.stdin`
- **Command chaining** -- `cmd1 ; cmd2` and `cmd1 && cmd2`
- **History** -- all commands are recorded for up-arrow recall
