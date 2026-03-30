# AGENTS.md

This file provides guidance to all AI agents working on this repository. It supplements the agent-specific `CLAUDE.md` with shared context that applies regardless of which agent is operating.

## Project Overview

OASIS_OS is an embeddable operating system framework in Rust (edition 2024). It provides a skinnable shell with a scene-graph UI (SDI), command interpreter, virtual file system, plugin system, and remote terminal. It renders anywhere you provide a pixel buffer and an input stream. Built from scratch in Rust starting early 2026, inspired by PSP homebrew shells like PSIX.

Default virtual resolution is 480x272 (PSP native). Skins may override this (e.g. modern=800x600, xp=1024x768); the backend canvas/window scales to match.

All code changes are authored by AI agents under human direction. No external contributions are accepted (see CONTRIBUTING.md).

## Build and Test Commands

All CI commands run inside Docker containers. Local development works with cargo directly (SDL3 is compiled from source; requires cmake, g++, and X11/audio dev headers).

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

# PSP overlay plugin PRX (excluded from workspace, kernel mode)
cd crates/oasis-plugin-psp && RUST_PSP_BUILD_STD=1 cargo +nightly psp --release

# UE5 FFI shared library
cargo build --release -p oasis-ffi

# Build WASM backend (requires wasm-pack)
./scripts/build-wasm.sh          # debug build
./scripts/build-wasm.sh --release # release (smaller + faster)

# Screenshots
cargo run -p oasis-app --bin oasis-screenshot
```

## CI Pipeline

**Order:** format check -> clippy -> nightly clippy (advisory) -> doc build -> markdown link check -> test -> release build -> screenshot regression -> cargo-deny -> benchmarks -> PSP EBOOT build -> PPSSPP headless test -> code coverage -> GitHub Pages deploy (WASM)

**Additional workflows:**
- `memory-ci.yml` -- ASAN + Valgrind massif (non-blocking, `continue-on-error: true`)
- `nightly-streaming.yml` -- Nightly streaming integration tests
- `fuzz.yml` -- Fuzz testing

All steps run via `docker compose --profile ci run --rm rust-ci`. Self-hosted runners only.

**PR Validation** additionally runs: Gemini AI review -> agent auto-fix response (max 5 iterations) -> agent failure handler (max 5 iterations). Codex AI review has been **disabled** (see security notice).

## Architecture

### Crate Dependency Graph

```
oasis-types     (foundation: Color, Button, InputEvent, backend traits, error types, geometry)
├── oasis-vfs        (virtual file system: MemoryVfs, RealVfs, GameAssetVfs)
├── oasis-platform   (platform service traits: Power, Time, USB, Network, OSK)
├── oasis-sdi        (scene display interface: named object registry, z-order)
├── oasis-net        (TCP networking, PSK auth, remote terminal, FTP)
├── oasis-audio      (audio manager, playlist, MP3 ID3 parsing)
├── oasis-ui         (32 widgets: Button, Card, TabBar, ListView, flex layout, etc.)
├── oasis-wm         (window manager: drag/resize, hit testing, decorations)
├── oasis-skin       (TOML skin engine, 20 skins, theme derivation)
├── oasis-terminal   (90+ commands across 17+ modules, shell features)
├── oasis-browser    (HTML/CSS/Gemini: DOM, CSS cascade, flex/grid layout, calc(), transforms, animations, hover-triggered transitions, light compositor with text batching + sticky scroll caching, form elements with select dropdown + label association, soft hyphens, bidi text, @media/@supports, cookies, CSP, JS DOM bindings)
├── oasis-js         (JavaScript engine: QuickJS-NG runtime, console API, document.cookie, history.pushState, persistent localStorage)
├── oasis-video      (MP4/H.264+AAC decode; StreamingBuffer sliding-window; features: h264, no-std-demux, video-decode)
├── oasis-vector     (vector graphics: scene graph, path ops, icons, frame-driven animations)
├── oasis-shader     (animated shader wallpapers: Shadertoy-style fragment shaders)
├── oasis-rasterize  (software rasterizer for CPU-side rendering)
├── oasis-i18n       (internationalization support)
├── oasis-test-backend (mock backend for testing)
├── oasis-app-core   (shared app framework: AppTrait, common utilities)
├── oasis-app-games  (Games app)
├── oasis-app-paint  (Paint app)
├── oasis-app-clock  (Clock app)
├── oasis-app-text-editor (Text Editor app)
├── oasis-app-calculator  (Calculator app)
├── oasis-app-media       (Music Player + Photo Viewer apps)
├── oasis-app-tv-guide    (TV Guide app)
├── oasis-app-radio       (Internet Radio app)
├── oasis-app-settings    (Settings app)
├── oasis-app-file-manager (File Manager app)
└── oasis-core       (coordination: dashboard, agent, plugin, script; apps extracted to oasis-app-* crates)
    ├── oasis-backend-sdl  (SDL3 desktop/Pi rendering + input + audio)
    │   └── oasis-app      (binary entry points: oasis-app, oasis-screenshot; oasis-video[video-decode])
    ├── oasis-backend-wasm (Canvas 2D + DOM input + Web Audio, iframe overlay; feature: wasm-youtube)
    ├── oasis-backend-ue5  (software RGBA framebuffer for Unreal Engine 5)
    │   └── oasis-ffi      (cdylib C-ABI for UE5 integration; oasis-video[video-decode])
    ├── oasis-backend-psp  (excluded from workspace, PSP hardware; oasis-video[no-std-demux])
    └── oasis-plugin-psp   (excluded from workspace, kernel-mode PRX overlay)
```

### Backend Trait Boundary

`oasis-types/src/backend.rs` defines the only abstraction between core and platform (re-exported by `oasis-core`):
- `SdiCore` -- required rendering (13 methods: init, clear, blit, fill_rect, draw_text, swap_buffers, load_texture, destroy_texture, set_clip_rect, reset_clip_rect, measure_text, read_pixels, shutdown)
- `SdiBackend` -- extends `SdiCore` with 39 optional accelerated primitives (shapes, gradients, text styling, vector graphics, batching)
- `InputBackend` -- input polling (returns `Vec<InputEvent>`)
- `NetworkBackend` -- TCP networking
- `AudioBackend` -- audio playback

Core code never calls platform APIs directly.

### Core Modules

The repository contains 37 crates (35 in the workspace, 2 excluded PSP crates). Each module below is its own crate:

- **oasis-types** -- Foundation types: `Color`, `Button`, `InputEvent`, backend traits (`SdiCore`, `SdiBackend`, `InputBackend`, `NetworkBackend`, `AudioBackend`), error types, TLS, bitmap font metrics, `geometry.rs` (shared shape algorithms)
- **oasis-sdi** -- Scene Display Interface: named objects with position, size, color, texture, text, z-order, gradients, rounded corners, shadows
- **oasis-skin** -- Data-driven TOML skin system with 20 skins (14 external TOML in `skins/`, 20 built-in). Theme derivation from 9 base colors to ~30 UI element colors.
- **oasis-browser** -- Embeddable HTML/CSS/Gemini rendering engine: DOM parser, CSS cascade with `@media`/`@supports` queries, flex/grid/table layout, `calc()`, transforms, animations, hover-triggered CSS transitions, light compositor (batched rect+text, occlusion culling, animation dirty tracking, sticky scroll caching), form elements (select dropdown, label for="" association), soft hyphens, bidi text detection, cookies, gzip, CSP (img-src relaxed), link navigation, reader mode, JavaScript DOM bindings
- **oasis-js** -- JavaScript engine wrapping QuickJS-NG via rquickjs: `console` API, inline `<script>` execution, DOM manipulation, `document.cookie`, `history.pushState`/`replaceState`, persistent `localStorage`. Feature-gated (`javascript`)
- **oasis-ui** -- 32 reusable widgets: Button, Card, TabBar, Panel, InputField, ListView, ScrollView, ProgressBar, Toggle, NinePatch, flex layout, Accordion, Avatar, Badge, Checkbox, ColorPicker, ContextMenu, DatePicker, Divider, Dropdown, Icon, Modal, Radio, RichText, Slider, SpinBox, Spinner, SplitPane, Table, Toast, Tooltip, TreeView
- **oasis-vfs** -- Virtual file system: `MemoryVfs` (in-RAM), `RealVfs` (disk), `GameAssetVfs` (UE5 with overlay writes)
- **oasis-terminal** -- Command interpreter with 90+ commands across 17 modules (core, text, file, system, dev, fun, security, doc, audio, network, skin, UI, plus agent/plugin/script/transfer/update registered by oasis-core). Shell features: variable expansion, glob expansion, aliases, history, piping
- **oasis-wm** -- Window manager (window configs, hit testing, drag/resize, minimize/maximize/close)
- **oasis-net** -- TCP networking with PSK authentication, remote terminal, FTP transfer
- **oasis-audio** -- Audio manager with playlist, shuffle/repeat modes, MP3 ID3 tag parsing
- **oasis-platform** -- Platform service traits: PowerService, TimeService, UsbService, NetworkService, OskService
- **oasis-video** -- MP4/H.264+AAC decode pipeline. Feature flags: `h264` (openh264 video decode + symphonia demux/AAC), `no-std-demux` (lightweight `demux_lite::Mp4Lite` parser, no symphonia/no std::sync::Once — PSP-safe), `video-decode` (re-exports `SoftwareVideoDecoder` for desktop/UE5). Streaming pipelines: desktop uses `StreamingBuffer` sliding-window for progressive playback with deferred tail probe, CDN failover, and PTS-based A/V sync; PSP streams in-memory via `demux_lite` + `sceAudiocodec` AAC hardware decode + `sceVideocodec` H.264 (real HW only, audio-only on PPSSPP) with backpressure-throttled I/O
- **oasis-vector** -- Resolution-independent vector graphics: scene graph with path-based drawing operations (fill, stroke, arcs, beziers), dashboard icons, and frame-driven animations
- **oasis-shader** -- Animated shader wallpapers: Shadertoy-style fragment shaders (voronoi, city lights, ocean waves, calm waves, Balatro)
- **oasis-app-core** -- Shared app framework: `AppTrait`, common utilities for extracted app crates
- **oasis-app-*** -- 11 extracted app crates: `oasis-app-games`, `oasis-app-paint`, `oasis-app-clock`, `oasis-app-text-editor`, `oasis-app-calculator`, `oasis-app-media` (Music Player + Photo Viewer), `oasis-app-tv-guide`, `oasis-app-radio`, `oasis-app-settings`, `oasis-app-file-manager`
- **oasis-core** -- Coordination layer: dashboard, agent/MCP, plugin, scripting, status/bottom bars, desktop taskbar. Apps extracted to `oasis-app-*` crates (remaining in-core: Browser, Network, Package Manager, System Monitor)

### FFI Boundary (oasis-ffi)

Exports C-ABI functions: `oasis_create`, `oasis_destroy`, `oasis_tick`, `oasis_send_input`, `oasis_get_buffer`, `oasis_get_dirty`, `oasis_send_command`, `oasis_free_string`, `oasis_set_vfs_root`, `oasis_register_callback`, `oasis_add_vfs_file`. Video playback (feature-gated): `oasis_video_play`, `oasis_video_stop`, `oasis_video_is_playing`.

### Font Rendering

Proportional bitmap font rendering from glyph ink bounds. `oasis-types` provides `glyph_advance()` with variable per-character widths (3-8px). Each backend has its own glyph table in `font.rs`. The PSP backend additionally uses system TrueType fonts via `psp::font` with a VRAM glyph atlas. No external font dependencies for desktop/UE5.

## Code Conventions

- **MSRV:** 1.91.0 (uses `str::floor_char_boundary`)
- **Edition:** 2024 (let-chains, `if let ... &&` syntax is used throughout)
- **Max line width:** 100 characters
- **Formatting:** `cargo fmt` with `rustfmt.toml` -- 4-space indent, Unix newlines, `fn_params_layout = "Tall"`, `match_block_trailing_comma = true`
- **Linting:** Clippy warnings are CI errors (`-D warnings`). Workspace lints: `clone_on_ref_ptr`, `dbg_macro`, `todo`, `unimplemented` = warn; `unsafe_op_in_unsafe_fn` = warn; `unwrap_used` = **deny**
- **Unsafe:** All unsafe blocks require `// SAFETY:` comments. Unsafe is limited to FFI boundary, SDL texture lifetime erasure, and PSP system calls.
- **Tests:** In-module (`#[cfg(test)] mod tests`), not in a separate `tests/` directory. 6,600+ tests across the workspace. Dev dependency: `tempfile = "3"` for oasis-core.
- **License:** Dual-licensed Unlicense + MIT

## Multi-Agent System

### Enabled Agents

| Agent | Runtime | Role | Status |
|-------|---------|------|--------|
| Claude | Host (Claude Code CLI) | Primary development, PR creation, complex tasks | Active |
| Gemini | Host (Gemini CLI) | PR code review (primary reviewer) | Active |
| Codex | Containerized | PR code review (secondary reviewer) | **DISABLED** -- see security notice below |
| OpenCode | Containerized | Code generation, issue implementation | Active |
| Crush | Containerized | Quick code generation, conversion | Active |

### Agent Priorities

- **Issue creation / PR authoring:** Claude > OpenCode
- **PR reviews:** Gemini (primary). Codex secondary review is **disabled** (see security notice).
- **Code fixes:** Claude > Crush > OpenCode

### CI Agent Workflow (PR Validation)

1. CI pipeline runs (format, clippy, test, build, cargo-deny, PSP)
2. Gemini reviews the PR diff and posts feedback
3. ~~Codex reviews after Gemini completes~~ **(DISABLED -- see security notice)**
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
| codex | Codex AI consultation **(DISABLED)** |
| opencode | OpenCode AI consultation |
| crush | Crush AI consultation |
| github-board | GitHub Projects board management (issue claiming, status tracking) |
| agentcore-memory | Persistent agent memory (AWS/ChromaDB) |
| reaction-search | Reaction image search |

### Security Notice: OpenAI / Codex / GPT Phase-Out

**OpenAI Codex has been disabled across all pipelines effective immediately.** OpenAI has entered partnerships with government agencies that enable mass surveillance and autonomous weapons decision-making. The mass surveillance risk alone -- where code, prompts, and context sent through OpenAI APIs may be accessible to nation-state actors -- is unacceptable for this project and its users.

We strongly recommend **Anthropic (Claude)** models for all AI-assisted workflows. See the [README security notice](README.md#security-notice) for full details.

To re-enable at your own risk: set `CODEX_ENABLED=true` in your environment.

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
- **`ci`** -- rust-ci container (rust:1.93-slim + cmake + X11/audio dev libs + nightly + cargo-deny)
- **`psp`** -- PPSSPP emulator (multi-stage build, NVIDIA GPU passthrough, X11 forwarding)
- **`services`** -- All MCP server containers
- **`memory`** -- AgentCore memory service (also included in `services`)

## Key Files

- `docs/design.md` -- Technical design document v2.4 (~1500 lines)
- `docs/skin-authoring.md` -- Skin creation guide with full TOML reference
- `skins/classic/` -- Classic skin TOML configs (skin.toml, layout.toml, features.toml, theme.toml)
- `skins/xp/` -- XP skin TOML configs (Windows XP Luna-inspired theme with start menu)
- `skins/macos/` -- macOS-inspired desktop skin
- `skins/gnome/` -- GNOME desktop style skin
- `skins/cyberpunk/` -- Neon cyberpunk aesthetic skin
- `skins/retro-cga/` -- CGA 4-color retro skin
- `skins/paper/` -- Minimalist paper/ink style skin
- `clippy.toml` -- Clippy lint thresholds (cognitive complexity 25, too-many-lines 100, too-many-args 7)
- `rustfmt.toml` -- Formatting rules
- `deny.toml` -- License and advisory policy
- `.pre-commit-config.yaml` -- Pre-commit hooks (trailing whitespace, yaml check, large files, actionlint, shellcheck, containerized rustfmt + clippy)

## Document Index

Key documentation for deeper context on specific topics. Read as needed rather than loading everything.

### Architecture & Design
- [`docs/design.md`](docs/design.md) -- Technical design document v2.4 (~1500 lines, comprehensive architecture)
- [`docs/adr/001-arena-based-dom.md`](docs/adr/001-arena-based-dom.md) -- ADR: Arena-based DOM allocation
- [`docs/adr/002-vfs-abstraction.md`](docs/adr/002-vfs-abstraction.md) -- ADR: Virtual file system design
- [`docs/adr/003-backend-trait-design.md`](docs/adr/003-backend-trait-design.md) -- ADR: Backend trait hierarchy
- [`docs/adr/004-psp-two-binary-architecture.md`](docs/adr/004-psp-two-binary-architecture.md) -- ADR: PSP EBOOT + PRX split
- [`docs/adr/005-toml-skin-system.md`](docs/adr/005-toml-skin-system.md) -- ADR: TOML skin engine

### Guides
- [`docs/getting-started.md`](docs/getting-started.md) -- Getting started guide
- [`docs/adding-commands.md`](docs/adding-commands.md) -- How to add terminal commands
- [`docs/skin-authoring.md`](docs/skin-authoring.md) -- Skin creation with full TOML reference
- [`docs/plugin-development.md`](docs/plugin-development.md) -- Plugin development guide
- [`docs/ffi-integration.md`](docs/ffi-integration.md) -- UE5 / C-ABI integration guide
- [`docs/psp-plugin.md`](docs/psp-plugin.md) -- PSP kernel plugin (PRX) documentation

### Operations
- [`docs/troubleshooting.md`](docs/troubleshooting.md) -- Troubleshooting common issues
- [`docs/security.md`](docs/security.md) -- Security policy and advisories
- [`CLAUDE.md`](CLAUDE.md) -- Claude Code agent instructions and build commands
- [`CONTRIBUTING.md`](CONTRIBUTING.md) -- Contribution policy (AI-authored only)
- [`scripts/psp-scenarios.md`](scripts/psp-scenarios.md) -- PSP test scenario documentation
