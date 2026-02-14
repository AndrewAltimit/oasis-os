# AGENTS.md

This file provides guidance to all AI agents working on this repository. It supplements the agent-specific `CLAUDE.md` with shared context that applies regardless of which agent is operating.

## Project Overview

OASIS_OS is an embeddable operating system framework in Rust (edition 2024). It provides a skinnable shell with a scene-graph UI (SDI), command interpreter, virtual file system, plugin system, and remote terminal. It renders anywhere you provide a pixel buffer and an input stream. Originally ported from a PSP homebrew shell (2006-2008).

Native virtual resolution: 480x272 (PSP native) across all backends.

All code changes are authored by AI agents under human direction. No external contributions are accepted (see CONTRIBUTING.md).

## Build and Test Commands

All CI commands run inside Docker containers. Local development works with cargo directly if SDL2 dev libs are installed.

```bash
# Build
cargo build --release -p oasis-app

# Build via Docker (matches CI)
docker compose --profile ci run --rm rust-ci cargo build --workspace --release

# Tests
cargo test --workspace                      # all tests
cargo test --workspace -- test_name         # single test
cargo test -p oasis-core                    # single crate

# Formatting
cargo fmt --all -- --check                  # check only
cargo fmt --all                             # apply

# Linting (CI treats warnings as errors)
cargo clippy --workspace -- -D warnings

# License/advisory audit
cargo deny check

# PSP backend (excluded from workspace, requires nightly + cargo-psp)
cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo +nightly psp --release

# UE5 FFI shared library
cargo build --release -p oasis-ffi

# Screenshots
cargo run -p oasis-app --bin oasis-screenshot
```

## CI Pipeline

**Order:** format check -> clippy -> test -> release build -> cargo-deny -> PSP EBOOT build -> PPSSPP headless test

All steps run via `docker compose --profile ci run --rm rust-ci`. Self-hosted runners only.

**PR Validation** additionally runs: Gemini AI review -> Codex AI review -> agent auto-fix response (max 5 iterations) -> agent failure handler (max 5 iterations).

## Architecture

### Crate Dependency Graph

```
oasis-types     (foundation: Color, Button, InputEvent, backend traits, error types)
├── oasis-vfs        (virtual file system: MemoryVfs, RealVfs, GameAssetVfs)
├── oasis-platform   (platform service traits: Power, Time, USB, Network, OSK)
├── oasis-sdi        (scene display interface: named object registry, z-order)
├── oasis-net        (TCP networking, PSK auth, remote terminal, FTP)
├── oasis-audio      (audio manager, playlist, MP3 ID3 parsing)
├── oasis-ui         (15+ widgets: Button, Card, TabBar, ListView, flex layout)
├── oasis-wm         (window manager: drag/resize, hit testing, decorations)
├── oasis-skin       (TOML skin engine, 8 skins, theme derivation)
├── oasis-terminal   (80+ commands across 14 modules, shell features)
├── oasis-browser    (HTML/CSS/Gemini: DOM, CSS cascade, layout engine)
└── oasis-core       (coordination: apps, dashboard, agent, plugin, script)
    ├── oasis-backend-sdl  (SDL2 desktop/Pi rendering + input + audio)
    │   └── oasis-app      (binary entry points: oasis-app, oasis-screenshot)
    ├── oasis-backend-ue5  (software RGBA framebuffer for Unreal Engine 5)
    │   └── oasis-ffi      (cdylib C-ABI for UE5 integration)
    └── oasis-backend-psp  (excluded from workspace, PSP hardware via sceGu)
```

### Backend Trait Boundary

`oasis-core/src/backend.rs` defines the only abstraction between core and platform:
- `SdiBackend` -- rendering (clear, blit, fill_rect, draw_text, load_texture, swap_buffers, read_pixels)
- `InputBackend` -- input polling (returns `Vec<InputEvent>`)
- `NetworkBackend` -- TCP networking
- `AudioBackend` -- audio playback

Core code never calls platform APIs directly.

### Core Modules

The framework is split into 16 workspace crates. Each module below is its own crate:

- **oasis-types** -- Foundation types: `Color`, `Button`, `InputEvent`, backend traits, error types, TLS, bitmap font metrics
- **oasis-sdi** -- Scene Display Interface: named objects with position, size, color, texture, text, z-order, gradients, rounded corners, shadows
- **oasis-skin** -- Data-driven TOML skin system with 8 skins (2 external in `skins/`, 7 built-in; xp exists in both forms). Theme derivation from 9 base colors to ~30 UI element colors.
- **oasis-browser** -- Embeddable HTML/CSS/Gemini rendering engine: DOM parser, CSS cascade, block/inline/table layout, link navigation, reader mode, bookmarks
- **oasis-ui** -- 15+ reusable widgets: Button, Card, TabBar, Panel, TextField, ListView, ScrollView, ProgressBar, Toggle, NinePatch, flex layout
- **oasis-vfs** -- Virtual file system: `MemoryVfs` (in-RAM), `RealVfs` (disk), `GameAssetVfs` (UE5 with overlay writes)
- **oasis-terminal** -- Command interpreter with 80+ commands across 14 modules (core, text, file, system, dev, fun, security, doc, audio, network, skin, UI, plus agent/plugin/script/transfer/update registered by oasis-core). Shell features: variable expansion, glob expansion, aliases, history, piping
- **oasis-wm** -- Window manager (window configs, hit testing, drag/resize, minimize/maximize/close)
- **oasis-net** -- TCP networking with PSK authentication, remote terminal, FTP transfer
- **oasis-audio** -- Audio manager with playlist, shuffle/repeat modes, MP3 ID3 tag parsing
- **oasis-platform** -- Platform service traits: PowerService, TimeService, UsbService, NetworkService, OskService
- **oasis-core** -- Coordination layer: app runner with 8 apps (File Manager with dual-panel, Settings, Network, Music Player, Photo Viewer, Package Manager, Browser, System Monitor), dashboard, agent/MCP, plugin, scripting, status/bottom bars

### FFI Boundary (oasis-ffi)

Exports C-ABI functions: `oasis_create`, `oasis_destroy`, `oasis_tick`, `oasis_send_input`, `oasis_get_buffer`, `oasis_get_dirty`, `oasis_send_command`, `oasis_free_string`, `oasis_set_vfs_root`, `oasis_register_callback`, `oasis_add_vfs_file`.

### Font Rendering

Proportional bitmap font rendering from glyph ink bounds. `oasis-types` provides `glyph_advance()` with variable per-character widths (3-8px). Each backend has its own glyph table in `font.rs`. The PSP backend additionally uses system TrueType fonts via `psp::font` with a VRAM glyph atlas. No external font dependencies for desktop/UE5.

## Code Conventions

- **MSRV:** 1.91.0 (uses `str::floor_char_boundary`)
- **Edition:** 2024 (let-chains, `if let ... &&` syntax is used throughout)
- **Max line width:** 100 characters
- **Formatting:** `cargo fmt` with `rustfmt.toml` -- 4-space indent, Unix newlines, `fn_params_layout = "Tall"`, `match_block_trailing_comma = true`
- **Linting:** Clippy warnings are CI errors (`-D warnings`). Workspace lints: `clone_on_ref_ptr`, `dbg_macro`, `todo`, `unimplemented` = warn; `unsafe_op_in_unsafe_fn` = warn
- **Unsafe:** All unsafe blocks require `// SAFETY:` comments. Unsafe is limited to FFI boundary, SDL texture lifetime erasure, and PSP system calls.
- **Tests:** In-module (`#[cfg(test)] mod tests`), not in a separate `tests/` directory. Dev dependency: `tempfile = "3"` for oasis-core.
- **License:** Dual-licensed Unlicense + MIT

## Multi-Agent System

### Enabled Agents

| Agent | Runtime | Role |
|-------|---------|------|
| Claude | Host (Claude Code CLI) | Primary development, PR creation, complex tasks |
| Gemini | Host (Gemini CLI) | PR code review (primary reviewer) |
| Codex | Containerized | PR code review (secondary reviewer) |
| OpenCode | Containerized | Code generation, issue implementation |
| Crush | Containerized | Quick code generation, conversion |

### Agent Priorities

- **Issue creation / PR authoring:** Claude > OpenCode
- **PR reviews:** Gemini (primary) > Codex (secondary, runs after Gemini)
- **Code fixes:** Claude > Crush > OpenCode

### CI Agent Workflow (PR Validation)

1. CI pipeline runs (format, clippy, test, build, cargo-deny, PSP)
2. Gemini reviews the PR diff and posts feedback
3. Codex reviews after Gemini completes
4. **Review Response Agent** reads review artifacts and auto-fixes issues (via `automation-cli`), max 5 iterations
5. **Failure Handler Agent** auto-fixes CI failures, max 5 iterations
6. Iteration tracking via `.github/actions/agent-iteration-check/` -- admins can comment `[CONTINUE]` to extend limits
7. `no-auto-fix` label disables automated fix agents

### MCP Servers

Configured in `.mcp.json`, all run as Docker containers via `docker compose --profile services`:

| Server | Purpose |
|--------|---------|
| code-quality | Linting, formatting, testing, security scanning, type checking |
| content-creation | LaTeX, TikZ, Manim rendering |
| gemini | Gemini AI consultation (second opinion) |
| codex | Codex AI consultation |
| opencode | OpenCode AI consultation |
| crush | Crush AI consultation |
| github-board | GitHub Projects board management (issue claiming, status tracking) |
| agentcore-memory | Persistent agent memory (AWS/ChromaDB) |
| reaction-search | Reaction image search |

### Agent Tooling

- `tools/cli/agents/` -- Run scripts for each agent (claude, gemini, codex, crush, opencode)
- `tools/cli/containers/` -- Containerized agent run scripts
- `.agents.yaml` -- Multi-agent configuration (copy `.agents.yaml.example` to set up)
- `.env` -- Environment variables (copy `.env.example` to set up)

### Security

- `agent_admins` in `.agents.yaml` controls who can trigger agent actions via `[Approved][Agent]` keywords
- `trusted_sources` controls whose comments are marked trusted for AI context
- All agents run in sandboxed environments (`autonomous_mode: true`, `require_sandbox: true`)
- Agent commit authors: "AI Review Agent", "AI Pipeline Agent", "AI Agent Bot"
- Fork PRs are blocked from self-hosted runners (fork guard in `pr-validation.yml`)

## Docker Services

`docker-compose.yml` profiles:
- **`ci`** -- rust-ci container (rust:1.93-slim + SDL2 dev libs + nightly + cargo-deny)
- **`psp`** -- PPSSPP emulator (multi-stage build, NVIDIA GPU passthrough, X11 forwarding)
- **`services`** -- All MCP server containers
- **`memory`** -- AgentCore memory service (also included in `services`)

## Key Files

- `docs/design.md` -- Technical design document v2.3 (~1300 lines)
- `docs/skin-authoring.md` -- Skin creation guide with full TOML reference
- `docs/psp-modernization-plan.md` -- PSP backend modernization roadmap (9 phases, 40 steps)
- `skins/classic/` -- Classic skin TOML configs (skin.toml, layout.toml, features.toml, theme.toml)
- `skins/xp/` -- XP skin TOML configs (Windows XP Luna-inspired theme with start menu)
- `clippy.toml` -- Clippy lint thresholds (cognitive complexity 25, too-many-lines 100, too-many-args 7)
- `rustfmt.toml` -- Formatting rules
- `deny.toml` -- License and advisory policy
- `.pre-commit-config.yaml` -- Pre-commit hooks (trailing whitespace, yaml check, large files, actionlint, shellcheck, containerized rustfmt + clippy)
