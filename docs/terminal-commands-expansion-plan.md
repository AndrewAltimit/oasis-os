# Terminal Commands Expansion Plan

**Branch**: `feat/terminal-commands-expansion`
**Status**: Complete
**Approach**: Multi-phase, multi-session incremental delivery

---

## Current State Assessment

### Inventory
- **45 commands defined** across 10 modules
- **26 commands registered** in production (`register_builtins()`)
- **19 commands orphaned** -- defined, tested, but never wired into the running binary
  - 6 agent commands, 5 browser commands, 3 script commands, 3 transfer commands, 1 plugin command, 1 update command

### Key Gaps in Shell Infrastructure
1. **No argument quoting** -- `split_whitespace()` parser, no way to pass spaces
2. **No piping or redirection** -- commands cannot chain or write to files
3. **No command history** -- no up-arrow recall
4. **No aliases or variables** -- no `$HOME`, no `alias ll='ls -l'`
5. **Broken help** -- `help` returns static text; no `help <command>` for per-command docs
6. **Broken `run`** -- lists script contents but does not execute
7. **12-line output cap** -- `MAX_OUTPUT_LINES = 12`, large output is silently lost
8. **Case-sensitive dispatch** -- `HELP` does not match `help`

### Underexposed Platform Capabilities
- Window manager (no terminal control)
- SDI scene graph (no direct manipulation)
- Browser (bookmarks, cache, reader mode not exposed)
- Skin theme (no runtime color queries/overrides)
- Audio playlist management (add/remove tracks, queue)
- MCP tool invocation
- Agent availability management

---

## Phase Overview

| Phase | Title | New Cmds | Focus |
|-------|-------|----------|-------|
| 0 | Foundation & Fixes | 0 | Wire orphaned commands, fix bugs, register everything |
| 1 | Shell Infrastructure | 0 | Quoting, pipes, redirection, history, variables, aliases |
| 2 | Help & Discovery | 1 | Per-command help, command categories, tab-completion data |
| 3 | Text Processing | 10 | head, tail, wc, grep, sort, uniq, tee, tr, cut, diff |
| 4 | File & Archive Utilities | 7 | write, append, tree, du, stat, xxd, checksum |
| 5 | Environment & Config | 5 | env/set/unset, alias/unalias |
| 6 | System & Process | 6 | uptime, df, whoami, hostname, date, sleep |
| 7 | Developer Tools | 7 | base64, json, uuid, seq, expr, test, xargs |
| 8 | Window & UI Control | 5 | wm, sdi, theme, notify, screenshot |
| 9 | Enhanced Networking | 5 | ifconfig, dns, netstat, wget, ssh |
| 10 | Enhanced Browser | 4 | bookmark, history, reader, cache |
| 11 | Enhanced Audio | 3 | playlist, eq, visualize |
| 12 | Advanced Scripting | 4 | if/else, while, for, function |
| 13 | Fun & Utilities | 8 | cal, fortune, banner, figlet, matrix, yes, watch, time |
| 14 | Security & Permissions | 4 | chmod, chown, passwd, audit |
| 15 | Polish & Integration | 0 | Man pages, tutorials, shell profiles, RC files |

**Total new commands**: ~69 (plus the 19 orphaned commands wired in Phase 0)

---

## Phase 0: Foundation & Fixes

**Goal**: Make all existing commands available and fix known bugs before adding anything new.

### Step 0.1: Wire All Orphaned Command Modules

Register all existing but unregistered command modules in production. In `oasis-app/src/main.rs` (or the appropriate registration site), call:
- `register_script_commands()`
- `register_transfer_commands()`
- `register_update_commands()`
- `register_plugin_commands()`
- `register_agent_commands()`
- `register_browser_commands()`

This brings the production count from 26 to 45 commands with zero new code.

**Files**: `oasis-app/src/main.rs`, possibly `oasis-app/src/commands.rs`

### Step 0.2: Fix the `help` Command

The current `help` command returns a static string and cannot see the registry. Fix:
- Pass the registry's command list into the environment or make `help` a special-case dispatch in `CommandRegistry::execute()`
- Support `help <command>` to show `usage()` + `description()` for a specific command
- Group commands by category in the `help` listing

**Files**: `oasis-terminal/src/interpreter.rs`, `oasis-terminal/src/commands.rs`

### Step 0.3: Fix the `run` Command

The `run` command currently just lists script contents. Fix it to actually execute scripts by calling `run_script()` internally and returning collected output.

**Files**: `oasis-core/src/script/mod.rs`

### Step 0.4: Increase Output Buffer

Increase `MAX_OUTPUT_LINES` from 12 to a more practical value (e.g., 200) with scrollback. The terminal already has scroll support via the SDI scene graph -- this just needs the buffer to retain more lines.

**Files**: `oasis-app/src/commands.rs` or wherever `MAX_OUTPUT_LINES` is defined, `oasis-app/src/terminal_sdi.rs`

### Step 0.5: Case-Insensitive Command Lookup

Normalize command input to lowercase before dispatch. A one-line change in `CommandRegistry::execute()`.

**Files**: `oasis-terminal/src/interpreter.rs`

### Step 0.6: Handle `CommandOutput` Variants from Newly-Registered Modules

Ensure `process_command_output()` in the app layer handles all `CommandOutput` variants that the newly-registered commands may produce (browser sandbox, skin swap signals are already handled; verify script/transfer/update/plugin/agent output handling).

**Files**: `oasis-app/src/commands.rs`

---

## Phase 1: Shell Infrastructure

**Goal**: Transform the terminal from a simple command dispatcher into a proper interactive shell.

### Step 1.1: Quoted Argument Parsing

Replace `split_whitespace()` with a proper tokenizer supporting:
- Single quotes: `echo 'hello world'`
- Double quotes: `echo "hello world"`
- Backslash escaping: `echo hello\ world`
- Preserve existing behavior for unquoted input

**Files**: `oasis-terminal/src/interpreter.rs` (new `tokenize()` function)

### Step 1.2: Environment Variables

Add a `HashMap<String, String>` to `Environment` for shell variables:
- `$VAR` expansion in arguments before dispatch
- Built-in variables: `$CWD`, `$USER`, `$SHELL`, `$HOME`, `$?` (last exit status)
- Variable assignment: `set VAR=value` (implemented in Phase 5)

**Files**: `oasis-terminal/src/interpreter.rs`

### Step 1.3: Command History

Add a history ring buffer to `CommandRegistry` or `Environment`:
- Store last N commands (configurable, default 100)
- `history` command to list recent commands
- `!!` to repeat last command
- `!n` to repeat command number n
- History persists to VFS (`/home/.shell_history`) across sessions

**Files**: `oasis-terminal/src/interpreter.rs`, new history command in `commands.rs`

### Step 1.4: Pipes

Support `cmd1 | cmd2` syntax:
- `cmd1`'s `Text` output becomes `cmd2`'s stdin (passed as final args or via a new `stdin` field on `Environment`)
- Only `Text` and `Table` output can be piped
- Signal outputs (`SkinSwap`, `Clear`, etc.) cannot be piped

**Files**: `oasis-terminal/src/interpreter.rs`

### Step 1.5: Output Redirection

Support `>` and `>>` operators:
- `cmd > /path/file` -- write `Text` output to VFS file (overwrite)
- `cmd >> /path/file` -- append to VFS file
- Works with pipes: `cmd1 | cmd2 > file`

**Files**: `oasis-terminal/src/interpreter.rs`

### Step 1.6: Command Chaining

Support `;` and `&&` and `||` operators:
- `cmd1 ; cmd2` -- run both regardless
- `cmd1 && cmd2` -- run cmd2 only if cmd1 succeeds
- `cmd1 || cmd2` -- run cmd2 only if cmd1 fails

**Files**: `oasis-terminal/src/interpreter.rs`

### Step 1.7: Globbing

Expand `*` and `?` patterns against VFS directory listings before command dispatch:
- `ls *.txt` expands to `ls file1.txt file2.txt`
- `cat /home/*.md` expands paths

**Files**: `oasis-terminal/src/interpreter.rs`

---

## Phase 2: Help & Discovery

**Goal**: Make the terminal self-documenting and discoverable.

### Step 2.1: Enhanced Help Command (rework)

- `help` -- list all commands grouped by category
- `help <command>` -- show detailed usage, description, and examples
- `help --all` -- show every command with full details

**Files**: `oasis-terminal/src/commands.rs`

### Step 2.2: Command Categories

Add a `category(&self) -> &str` method to the `Command` trait (with a default of `"general"`). Categories:
- `filesystem` -- ls, cd, pwd, cat, mkdir, rm, touch, cp, mv, find, tree, etc.
- `system` -- status, power, clock, memory, usb, uptime, df, hostname, etc.
- `network` -- wifi, ping, http, ifconfig, dns, netstat, wget, etc.
- `audio` -- music, playlist, eq, visualize
- `text` -- head, tail, wc, grep, sort, uniq, tr, cut, diff, tee
- `browser` -- browse, fetch, gemini, curl, bookmark, history, reader, cache
- `scripting` -- run, cron, startup, if, while, for, function
- `developer` -- base64, json, uuid, hexdump, checksum, expr, test
- `ui` -- skin, theme, wm, sdi, notify, screenshot
- `agent` -- agent, mcp, tamper, board, ci, health
- `transfer` -- ftp, push, pull
- `config` -- env, set, unset, alias, unalias
- `fun` -- cal, fortune, banner, figlet, matrix, yes
- `security` -- chmod, chown, passwd, audit

### Step 2.3: `which` Command

`which <name>` -- show whether a command exists and its category/description.

**Files**: `oasis-terminal/src/commands.rs`

---

## Phase 3: Text Processing

**Goal**: Provide Unix-like text manipulation tools that work on VFS files and piped input.

All text commands accept either a file path argument or piped input from Phase 1.4.

### Commands

| Command | Description | Usage |
|---------|-------------|-------|
| `head` | Show first N lines of a file | `head [-n N] <file>` |
| `tail` | Show last N lines of a file | `tail [-n N] <file>` |
| `wc` | Count lines, words, bytes | `wc [-l\|-w\|-c] <file>` |
| `grep` | Search for pattern in files | `grep <pattern> <file> [-i] [-n] [-c]` |
| `sort` | Sort lines | `sort <file> [-r] [-n] [-u]` |
| `uniq` | Remove duplicate adjacent lines | `uniq <file> [-c] [-d]` |
| `tee` | Write to file and pass through | `cmd \| tee <file>` |
| `tr` | Translate/delete characters | `tr <from> <to>` (piped input) |
| `cut` | Extract fields from lines | `cut -d <delim> -f <fields> <file>` |
| `diff` | Compare two files | `diff <file1> <file2>` |

**Files**: New `oasis-terminal/src/text_commands.rs`

---

## Phase 4: File & Archive Utilities

**Goal**: Richer file inspection and manipulation.

### Commands

| Command | Description | Usage |
|---------|-------------|-------|
| `write` | Write text content to a file | `write <file> <content...>` |
| `append` | Append text to a file | `append <file> <content...>` |
| `tree` | Display directory tree | `tree [path] [-d] [--depth N]` |
| `du` | Show file/directory sizes | `du [path] [-h] [-s]` |
| `stat` | Show detailed file metadata | `stat <path>` |
| `xxd` | Hex dump of file contents | `xxd <file> [-l N] [-s offset]` |
| `checksum` | Compute hash of file | `checksum <file> [md5\|sha1\|sha256]` |

**Files**: New `oasis-terminal/src/file_commands.rs`

---

## Phase 5: Environment & Configuration

**Goal**: Shell customization and persistent configuration.

### Commands

| Command | Description | Usage |
|---------|-------------|-------|
| `env` | List all environment variables | `env` |
| `set` | Set an environment variable | `set <VAR>=<value>` |
| `unset` | Remove an environment variable | `unset <VAR>` |
| `alias` | Create a command alias | `alias <name>=<command...>` |
| `unalias` | Remove a command alias | `unalias <name>` |

### Persistence
- Variables saved to `/home/.shellrc` on `set`
- Aliases saved to `/home/.aliases`
- Both loaded on terminal startup via the scripting system

**Files**: New `oasis-terminal/src/env_commands.rs`, updates to `interpreter.rs`

---

## Phase 6: System & Process Commands

**Goal**: System information and control utilities.

### Commands

| Command | Description | Usage |
|---------|-------------|-------|
| `uptime` | Show system uptime in human-readable format | `uptime` |
| `df` | Show VFS filesystem usage | `df [-h]` |
| `whoami` | Show current user | `whoami` |
| `hostname` | Show/set system hostname | `hostname [new_name]` |
| `date` | Show/format current date and time | `date [+format]` |
| `sleep` | Pause execution for N seconds | `sleep <seconds>` |

**Notes**:
- `uptime` leverages `TimeService::uptime_secs()`
- `df` reports VFS stats (total entries, total bytes, per-directory breakdown)
- `whoami` returns the configured username (default: `user`)
- `date` supports strftime-like format strings using `TimeService::now()`
- `sleep` is primarily useful in scripts

**Files**: New `oasis-terminal/src/system_commands.rs`

---

## Phase 7: Developer Tools

**Goal**: Utility commands for data manipulation and scripting support.

### Commands

| Command | Description | Usage |
|---------|-------------|-------|
| `base64` | Encode/decode base64 | `base64 [encode\|decode] <input>` |
| `json` | Parse, query, and format JSON | `json <file> [path.to.key]` or piped |
| `uuid` | Generate a UUID | `uuid` |
| `seq` | Generate number sequences | `seq <start> [step] <end>` |
| `expr` | Evaluate arithmetic expressions | `expr <expression>` |
| `test` | Evaluate conditional expressions | `test <expression>` / `test -f <path>` / `test -d <path>` |
| `xargs` | Build commands from piped input | `cmd \| xargs <command>` |

**Notes**:
- `json` uses a minimal JSON parser (no serde dependency -- keep it lightweight or use existing if available)
- `expr` supports `+`, `-`, `*`, `/`, `%`, `(`, `)` with integer arithmetic
- `test` returns success/failure (exit code) for use with `&&`/`||` from Phase 1.6

**Files**: New `oasis-terminal/src/dev_commands.rs`

---

## Phase 8: Window & UI Control

**Goal**: Terminal-driven UI manipulation for power users and scripting.

### Commands

| Command | Description | Usage |
|---------|-------------|-------|
| `wm` | Window manager control | `wm list` / `wm close <id>` / `wm focus <id>` / `wm move <id> <x> <y>` / `wm resize <id> <w> <h>` / `wm minimize <id>` / `wm maximize <id>` |
| `sdi` | Inspect/manipulate SDI scene objects | `sdi list` / `sdi get <name>` / `sdi set <name> <prop> <value>` |
| `theme` | Query/override skin theme properties | `theme` / `theme get <property>` / `theme set <property> <value>` |
| `notify` | Display a notification/toast message | `notify <message> [--duration N]` |
| `screenshot` | Capture screen to VFS file | `screenshot [path]` |

**Notes**:
- `wm` requires adding window manager access to `Environment` (new optional field)
- `sdi` enables visual scripting: `sdi set status_text text "Hello"` changes an SDI object
- `theme` operates on the active skin's theme colors
- `notify` writes to a VFS notification path for the app layer to display
- `screenshot` leverages `SdiBackend::read_pixels()` via a new `CommandOutput::Screenshot` variant

**Files**: New `oasis-terminal/src/ui_commands.rs`, updates to `interpreter.rs` (Environment), `oasis-app/src/commands.rs` (output handling)

---

## Phase 9: Enhanced Networking

**Goal**: Richer network diagnostics and transfer tools.

### Commands

| Command | Description | Usage |
|---------|-------------|-------|
| `ifconfig` | Show network interface details | `ifconfig` |
| `dns` | DNS lookup | `dns <hostname>` |
| `netstat` | Show active connections | `netstat` |
| `wget` | Download file from URL to VFS | `wget <url> [output_path]` |
| `ssh` | Connect to remote OASIS terminal (enhanced) | `ssh <host> [-p port] [-k psk]` |

**Notes**:
- `ifconfig` expands `wifi` to show IP, MAC, gateway, subnet
- `dns` does a DNS resolution and shows all returned addresses
- `netstat` shows active remote terminal connections and FTP sessions
- `wget` combines `http` GET with file write -- saves response body to VFS
- `ssh` is an enhanced `remote` with better UX (prompt for PSK, auto-save to hosts)

**Files**: Updates to `oasis-terminal/src/network_commands.rs`

---

## Phase 10: Enhanced Browser Commands

**Goal**: Full browser control from the terminal.

### Commands

| Command | Description | Usage |
|---------|-------------|-------|
| `bookmark` | Manage browser bookmarks | `bookmark list` / `bookmark add <url> [title]` / `bookmark remove <url>` |
| `history` | Browse/search navigation history | `history [list\|search <term>\|clear]` |
| `reader` | Toggle reader mode | `reader [on\|off]` |
| `cache` | Manage browser cache | `cache [status\|clear\|size]` |

**Notes**:
- These extend the existing `browse` command with dedicated subcommands
- Bookmark data persists in VFS at `/home/.bookmarks`
- History stored in VFS at `/home/.browse_history`

**Files**: Updates to `oasis-browser/src/commands.rs`

---

## Phase 11: Enhanced Audio

**Goal**: Advanced audio control beyond basic playback.

### Commands

| Command | Description | Usage |
|---------|-------------|-------|
| `playlist` | Manage playlists | `playlist list` / `playlist add <file>` / `playlist remove <index>` / `playlist save <name>` / `playlist load <name>` |
| `eq` | Equalizer presets | `eq [flat\|bass\|treble\|vocal\|custom <bands...>]` |
| `visualize` | ASCII audio visualization mode | `visualize [spectrum\|waveform\|off]` |

**Notes**:
- `playlist` extends `music` with playlist persistence (save/load to VFS `/home/.playlists/`)
- `eq` is a stretch goal -- depends on audio backend capabilities
- `visualize` writes ASCII art spectrum to terminal output lines on a timer (needs a new periodic output mechanism or `CommandOutput` variant)

**Files**: Updates to `oasis-terminal/src/audio_commands.rs`

---

## Phase 12: Advanced Scripting

**Goal**: Make the scripting system Turing-complete with control flow.

### Step 12.1: Conditional Execution (`if`/`else`/`fi`)

```
if test -f /home/config.txt
  cat /home/config.txt
else
  echo "No config found"
fi
```

### Step 12.2: While Loops (`while`/`done`)

```
set I=0
while test $I -lt 10
  echo $I
  expr $I + 1 | set I
done
```

### Step 12.3: For Loops (`for`/`done`)

```
for f in /home/*.txt
  echo "File: $f"
  wc -l $f
done
```

### Step 12.4: Functions (`function`/`end`)

```
function greet
  echo "Hello, $1!"
end
greet World
```

**Notes**:
- Control flow state tracked in a `ScriptContext` stack in the interpreter
- `if`/`while`/`for`/`function` are registered as commands but trigger interpreter-level state changes
- Nesting supported via stack depth
- Max loop iterations capped (e.g., 1000) to prevent infinite loops
- Functions stored in `Environment` and callable like regular commands

**Files**: Major updates to `oasis-terminal/src/interpreter.rs`, new `oasis-terminal/src/control_flow.rs`, updates to `oasis-core/src/script/mod.rs`

---

## Phase 13: Fun & Utilities

**Goal**: Personality and delight -- commands that make the terminal fun to use.

### Commands

| Command | Description | Usage |
|---------|-------------|-------|
| `cal` | Display a calendar | `cal [month] [year]` |
| `fortune` | Display a random quote/tip | `fortune` |
| `banner` | Display large ASCII text | `banner <text>` |
| `figlet` | Display stylized ASCII art text | `figlet <text> [-f font]` |
| `matrix` | Matrix-style rain animation | `matrix [--duration N]` |
| `yes` | Repeatedly output a string | `yes [string]` (useful in pipes) |
| `watch` | Re-run a command periodically | `watch [-n secs] <command...>` |
| `time` | Measure command execution time | `time <command...>` |

**Notes**:
- `fortune` reads from a fortune file in VFS (`/usr/share/fortunes`)
- `banner`/`figlet` use built-in bitmap patterns (no external deps)
- `matrix` uses a new `CommandOutput::Animated` variant for periodic updates, or writes frames to a VFS path for the app layer to render
- `watch` and `time` require timing infrastructure (use `TimeService`)
- `yes` has a built-in iteration limit (e.g., 1000) to prevent lockups

**Files**: New `oasis-terminal/src/fun_commands.rs`

---

## Phase 14: Security & Permissions

**Goal**: Simulated multi-user permissions and security auditing.

### Commands

| Command | Description | Usage |
|---------|-------------|-------|
| `chmod` | Set file permissions (simulated) | `chmod <mode> <path>` |
| `chown` | Set file owner (simulated) | `chown <user> <path>` |
| `passwd` | Set/change user password | `passwd [user]` |
| `audit` | View security audit log | `audit [list\|clear\|tail N]` |

**Notes**:
- Permissions are stored as VFS metadata (requires extending `VfsMetadata`)
- Not enforced (simulated) -- OASIS is single-user, but commands teach Unix concepts
- `passwd` manages a PSK for remote terminal authentication
- `audit` logs command execution, remote connections, file modifications to `/var/log/audit`

**Files**: New `oasis-terminal/src/security_commands.rs`, updates to `oasis-vfs/src/lib.rs` (metadata), updates to `oasis-terminal/src/interpreter.rs` (audit logging)

---

## Phase 15: Polish & Integration

**Goal**: Documentation, tutorials, and shell startup experience.

### Step 15.1: Man Pages

- Add a `man` command that reads `/usr/share/man/<command>.txt` from VFS
- Pre-populate man pages for all commands during VFS initialization
- Format: plain text with sections (NAME, SYNOPSIS, DESCRIPTION, EXAMPLES)

### Step 15.2: Interactive Tutorial

- Add a `tutorial` command that walks through terminal basics
- Progressive lessons: navigation, file ops, pipes, scripting
- Track progress in `/home/.tutorial_progress`

### Step 15.3: Shell Profile

- Load `/home/.profile` on terminal startup (via scripting system)
- Set default aliases, variables, prompt customization
- Provide a default `.profile` with useful defaults

### Step 15.4: MOTD (Message of the Day)

- Display `/etc/motd` contents when terminal opens
- Default MOTD includes version, tip of the day, system status summary

### Step 15.5: Tab Completion Data

- Expose command names and VFS paths for tab completion
- Add `CommandRegistry::completions(partial: &str) -> Vec<String>` method
- UI layer can call this when tab is pressed (future UI work, but the data layer is ready)

**Files**: New `oasis-terminal/src/doc_commands.rs`, updates to `oasis-core/src/script/mod.rs`, VFS initialization updates

---

## Implementation Guidelines

### Code Organization
- Each phase gets its own PR(s) for clean review
- New command modules follow existing patterns: one file per category, `register_*_commands()` function
- All commands implement the `Command` trait
- All commands have tests (`#[cfg(test)] mod tests`)

### Quality Gates (per PR)
- `cargo fmt --all -- --check`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace`
- No increase in unsafe code
- Max 100-char line width
- All commands have `description()` and `usage()` strings

### Dependencies
- Phase 0 is prerequisite for everything
- Phase 1 (shell infrastructure) is prerequisite for Phases 3, 7, 12 (pipes, variables, control flow)
- Phase 2 (help/categories) should come early to maintain discoverability
- Phases 3-11 are largely independent and can be done in any order
- Phase 12 (advanced scripting) depends on Phase 1 (variables) and Phase 7 (`test`, `expr`)
- Phase 13-14 are independent
- Phase 15 should come last

### Suggested Order of Implementation
```
Phase 0 (Foundation) ──► Phase 1 (Shell Infra) ──► Phase 2 (Help)
                                 │
                    ┌────────────┼─────────────┐
                    ▼            ▼              ▼
              Phase 3       Phase 4-6      Phase 8
              (Text)        (File/Env/Sys) (UI)
                    │            │              │
                    └────────────┼─────────────┘
                                 ▼
                    Phase 7 (Dev Tools) ──► Phase 12 (Scripting)
                                 │
                    ┌────────────┼─────────────┐
                    ▼            ▼              ▼
              Phase 9       Phase 10-11    Phase 13
              (Network)    (Browser/Audio) (Fun)
                                 │
                                 ▼
                         Phase 14 (Security)
                                 │
                                 ▼
                         Phase 15 (Polish)
```

### Estimated Scope
- **~69 new commands** across 12 new command files
- **~19 orphaned commands** wired into production
- **Major interpreter upgrades**: quoting, pipes, redirection, variables, aliases, history, control flow
- **Final total**: ~88 production commands + full scripting language

---

## Notes

- Each phase is designed to be independently shippable and testable
- Commands should fail gracefully when optional platform services are unavailable
- All output should respect the terminal's line width (~60 chars for 480px at 8px font)
- Commands operating on VFS should work identically across MemoryVfs, RealVfs, and GameAssetVfs
- Performance matters on PSP: avoid allocations in hot paths, prefer stack buffers
