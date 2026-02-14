# OASIS_OS

An embeddable operating system framework in Rust. Renders a skinnable shell interface -- scene-graph UI, command interpreter, virtual file system, browser engine, plugin system, remote terminal -- anywhere you can provide a pixel buffer and an input stream.

### Classic Skin

| Dashboard | Terminal |
|:---:|:---:|
| ![Dashboard](screenshots/classic/01_dashboard.png) | ![Terminal](screenshots/classic/04_terminal.png) |

### XP Skin

| Dashboard | Terminal |
|:---:|:---:|
| ![Dashboard](screenshots/xp/01_dashboard.png) | ![Terminal](screenshots/xp/04_terminal.png) |

### Modern Skin

| Dashboard | Terminal |
|:---:|:---:|
| ![Dashboard](screenshots/modern/01_dashboard.png) | ![Terminal](screenshots/modern/04_terminal.png) |

### Desktop Skin

| Dashboard | Terminal |
|:---:|:---:|
| ![Dashboard](screenshots/desktop/01_dashboard.png) | ![Terminal](screenshots/desktop/04_terminal.png) |

### Terminal Skins

| Terminal | Tactical | Corrupted | Agent Terminal |
|:---:|:---:|:---:|:---:|
| ![Terminal](screenshots/terminal/04_terminal.png) | ![Tactical](screenshots/tactical/04_terminal.png) | ![Corrupted](screenshots/corrupted/04_terminal.png) | ![Agent](screenshots/agent-terminal/04_terminal.png) |

## Overview

OASIS_OS originated as a Rust port of a PSP homebrew shell OS written in C circa 2006-2008. The trait-based backend system designed for cross-platform portability extends to four rendering targets:

| Target | Backend | Renderer | Input | Status |
|--------|---------|----------|-------|--------|
| Desktop / Raspberry Pi | `oasis-backend-sdl` | SDL2 window | Keyboard, mouse, gamepad | Implemented |
| PSP / PPSSPP | `oasis-backend-psp` | sceGu hardware sprites | PSP controller | Implemented |
| Unreal Engine 5 | `oasis-backend-ue5` | Software RGBA framebuffer | FFI input queue | Implemented |
| Framebuffer (headless Pi) | Planned | `/dev/fb0` direct writes | evdev | Planned |

### Skins

The framework supports a data-driven **skin system** that controls visual layout, color themes, feature gating, and behavioral personality. Skins are defined in TOML configuration files -- no code changes required. Eight skins are implemented:

| Skin | Style | Key Features | Source |
|------|-------|-------------|--------|
| **classic** | PSIX-style icon grid dashboard | Dashboard + terminal, tabbed navigation (OSS/APPS/MODS/NET), status bar, chrome bezels | External (`skins/classic/`) |
| **xp** | Windows XP Luna-inspired blue theme | Dashboard + terminal + start menu, gradient titlebars, taskbar | External (`skins/xp/`) |
| **terminal** | Green-on-black CRT | Terminal only, full-screen command line | Built-in |
| **tactical** | Military command console | Terminal + restricted commands, stripped-down UI | Built-in |
| **corrupted** | Glitched terminal | Terminal + corruption effects (jitter, flicker, garbling) | Built-in |
| **desktop** | Windowed desktop | Window manager + terminal, resizable/draggable windows | Built-in |
| **agent-terminal** | AI agent console | Terminal + agent/MCP commands, system health | Built-in |
| **modern** | Purple accent, rounded corners | Dashboard + WM + browser, gradient fills, shadows | Built-in |

All skins share the same core: scene graph, command interpreter, virtual file system, networking, and plugin infrastructure. See the [Skin Authoring Guide](docs/skin-authoring.md) for creating custom skins.

Native virtual resolution is 480x272 (PSP native) across all backends.

### Key Features

- **Scene Graph (SDI)** -- Named object registry with position, size, color, texture, text, z-order, alpha, gradients, rounded corners, shadows
- **Browser Engine** -- Embedded HTML/CSS/Gemini renderer with DOM parsing, CSS cascade, block/inline/table layout, link navigation, reader mode, bookmarks
- **Window Manager** -- Movable, resizable, overlapping windows with titlebars, minimize/maximize/close, hit testing, and themed decorations
- **UI Widget Toolkit** -- 15+ reusable widgets: Button, Card, TabBar, Panel, TextField, ListView, ScrollView, ProgressBar, Toggle, NinePatch, flex layout, and more
- **Proportional Bitmap Font** -- Variable-width glyph rendering from ink bounds with per-character advance values (not fixed-width 8x8)
- **80+ Terminal Commands** -- 14 command modules: core (fs/system), text processing (head, tail, grep, sort, uniq, tr, cut, diff), file utilities (write, tree, du, stat, xxd, checksum), dev tools (base64, json, uuid, seq, expr), fun (cal, fortune, banner, matrix), security (chmod, chown, passwd, audit), documentation (man, tutorial, motd), networking (wifi, ping, http), audio, UI, skin switching, scripting, transfer (FTP), system updates. Shell features include variable expansion, glob expansion, aliases, history (!!/!n), piping, and command chaining
- **Audio System** -- Playlist management, MP3/WAV playback, ID3 tag parsing, shuffle/repeat modes, volume control
- **Plugin System** -- Runtime-extensible via `Plugin` trait, VFS-based IPC, manifest-driven discovery
- **Virtual File System** -- `MemoryVfs` (in-RAM), `RealVfs` (disk), `GameAssetVfs` (UE5 with overlay writes)
- **Remote Terminal** -- TCP listener with PSK authentication for headless device management
- **Agent/MCP Integration** -- Agent status tracking, MCP tool browsing/invocation, tamper detection, system health dashboard
- **Scripting** -- Line-based command scripts, startup scripts, cron-like scheduling
- **8 Built-in Apps** -- File Manager (dual-panel Norton Commander-style), Settings, Network, Music Player, Photo Viewer, Package Manager, Browser, System Monitor

## Crates

The framework is split into 16 workspace crates plus 1 excluded PSP backend:

```
oasis-os/
+-- Cargo.toml                        # Workspace root (resolver="2", edition 2024)
+-- crates/
|   +-- oasis-types/                  # Foundation types: Color, Button, InputEvent, backend traits, error types
|   +-- oasis-vfs/                    # Virtual file system: MemoryVfs, RealVfs, GameAssetVfs
|   +-- oasis-platform/              # Platform service traits: Power, Time, USB, Network, OSK
|   +-- oasis-sdi/                    # Scene Display Interface: named object registry, z-order, rendering
|   +-- oasis-net/                    # TCP networking, PSK authentication, remote terminal, FTP transfer
|   +-- oasis-audio/                  # Audio manager, playlist, shuffle/repeat, MP3 ID3 parsing
|   +-- oasis-ui/                     # 15+ widgets: Button, Card, TabBar, Panel, TextField, ListView, etc.
|   +-- oasis-wm/                     # Window manager: drag/resize, hit testing, minimize/maximize/close
|   +-- oasis-skin/                   # TOML skin engine, 8 skins, theme derivation from 9 base colors
|   +-- oasis-terminal/              # Command interpreter: 80+ commands across 14 modules, shell features
|   +-- oasis-browser/               # HTML/CSS/Gemini browser: DOM, CSS cascade, block/inline/table layout
|   +-- oasis-core/                   # Coordination layer: apps, dashboard, agent, plugin, script, etc.
|   +-- oasis-backend-sdl/            # SDL2 rendering and input (desktop + Pi)
|   +-- oasis-backend-ue5/            # UE5 software framebuffer + FFI input queue
|   +-- oasis-backend-psp/            # [excluded from workspace] sceGu hardware rendering, PSP controller, UMD browsing
|   +-- oasis-ffi/                    # C FFI boundary for UE5 integration
|   +-- oasis-app/                    # Binary entry points: desktop app + screenshot tool
+-- skins/
|   +-- classic/                      # PSIX-style icon grid dashboard
|   +-- xp/                           # Windows XP Luna-inspired theme with start menu
+-- docs/
    +-- design.md                     # Technical design document (v2.4)
    +-- skin-authoring.md             # Skin creation guide with full TOML reference
    +-- psp-modernization-plan.md     # PSP backend modernization roadmap (9 phases, 40 steps)
```

| Crate | Description |
|-------|-------------|
| `oasis-types` | Foundation types and traits: `Color`, `Button`, `InputEvent`, `SdiBackend`, `InputBackend`, `NetworkBackend`, `AudioBackend`, error types, TLS |
| `oasis-vfs` | Virtual file system: `MemoryVfs` (in-RAM), `RealVfs` (disk), `GameAssetVfs` (UE5 with overlay writes) |
| `oasis-platform` | Platform service traits: `PowerService`, `TimeService`, `UsbService`, `NetworkService`, `OskService` |
| `oasis-sdi` | Scene Display Interface: named object registry with position, size, color, texture, text, z-order, gradients, shadows |
| `oasis-net` | TCP networking with PSK authentication, remote terminal, FTP transfer |
| `oasis-audio` | Audio manager with playlist, shuffle/repeat modes, MP3 ID3 tag parsing |
| `oasis-ui` | 15+ reusable widgets: Button, Card, TabBar, Panel, TextField, ListView, ScrollView, ProgressBar, Toggle, NinePatch, flex layout |
| `oasis-wm` | Window manager: movable/resizable windows, titlebar buttons, hit testing, themed decorations |
| `oasis-skin` | Data-driven TOML skin system with 8 skins, theme derivation from 9 base colors to ~30 UI element colors |
| `oasis-terminal` | Command interpreter with 80+ commands across 14 modules, shell features (variables, globs, aliases, history, piping) |
| `oasis-browser` | Embeddable HTML/CSS/Gemini rendering engine: DOM parser, CSS cascade, block/inline/table layout, reader mode |
| `oasis-core` | Coordination layer: app runner (dual-panel file manager), dashboard, agent/MCP, plugin, scripting, status/bottom bars |
| `oasis-backend-sdl` | SDL2 rendering and input backend for desktop and Raspberry Pi |
| `oasis-backend-ue5` | UE5 render target backend -- software RGBA framebuffer and FFI input queue |
| `oasis-backend-psp` | PSP hardware backend -- sceGu sprite rendering, PSP controller input, dual-panel file manager, UMD disc browsing, std via [rust-psp](https://github.com/AndrewAltimit/rust-psp) SDK |
| `oasis-ffi` | C-ABI FFI boundary (`cdylib`) for UE5 and external integrations |
| `oasis-app` | Desktop entry point (SDL2) and screenshot capture tool |

The PSP backend is excluded from the workspace (requires `mipsel-sony-psp` target) and depends on the standalone [rust-psp SDK](https://github.com/AndrewAltimit/rust-psp) via git dependency.

## Building

### Desktop (SDL2)

```bash
# Via Docker (container-first)
docker compose --profile ci run --rm rust-ci cargo build --release -p oasis-app

# Or natively (requires libsdl2-dev, libsdl2-mixer-dev)
cargo build --release -p oasis-app
```

### PSP (EBOOT.PBP)

Requires the nightly Rust toolchain with `rust-src` and `cargo-psp`:

```bash
cd crates/oasis-backend-psp
RUST_PSP_BUILD_STD=1 cargo +nightly psp --release
# Output: target/mipsel-sony-psp-std/release/EBOOT.PBP
```

### UE5 (FFI Library)

```bash
cargo build --release -p oasis-ffi
# Output: target/release/liboasis_ffi.so (or .dll on Windows)
```

### Screenshots

Capture screenshots for all skins:

```bash
# Classic skin (default)
cargo run -p oasis-app --bin oasis-screenshot

# Any skin by name
cargo run -p oasis-app --bin oasis-screenshot xp
cargo run -p oasis-app --bin oasis-screenshot modern
cargo run -p oasis-app --bin oasis-screenshot desktop
cargo run -p oasis-app --bin oasis-screenshot terminal
cargo run -p oasis-app --bin oasis-screenshot tactical
cargo run -p oasis-app --bin oasis-screenshot corrupted
cargo run -p oasis-app --bin oasis-screenshot agent-terminal
```

## PSP Testing (PPSSPP)

The repo includes a containerized PPSSPP emulator with NVIDIA GPU passthrough for testing PSP EBOOTs:

```bash
# Build the PPSSPP Docker image (first time only)
docker compose --profile psp build ppsspp

# Run with GUI (requires X11 display)
docker compose --profile psp run --rm ppsspp /roms/release/EBOOT.PBP

# Run headless (CI / no display -- exits TIMEOUT on success)
docker compose --profile psp run --rm -e PPSSPP_HEADLESS=1 ppsspp /roms/release/EBOOT.PBP --timeout=5

# Run with interpreter (more stable for some MIPS code paths)
docker compose --profile psp run --rm -e PPSSPP_HEADLESS=1 ppsspp /roms/release/EBOOT.PBP -i --timeout=5
```

The `/roms/` mount maps to `crates/oasis-backend-psp/target/mipsel-sony-psp-std/` so both `release/` and `debug/` EBOOTs are available. Headless mode exits with `TIMEOUT` on success (OASIS_OS runs an infinite render loop). Any crash produces a non-zero exit code.

## CI

All CI stages run in Docker containers on a self-hosted runner:

```bash
# Build the CI container
docker compose --profile ci build

# Format check
docker compose --profile ci run --rm rust-ci cargo fmt --all -- --check

# Clippy
docker compose --profile ci run --rm rust-ci cargo clippy --workspace -- -D warnings

# Tests
docker compose --profile ci run --rm rust-ci cargo test --workspace

# License/advisory check
docker compose --profile ci run --rm rust-ci cargo deny check
```

GitHub Actions workflows run the full pipeline automatically on push to `main` and on pull requests, including PSP EBOOT build + PPSSPP headless testing, AI code review (Gemini + Codex), and automated fix agents.

## Documentation

- [Technical Design Document](docs/design.md) -- architecture, backends, skins, UE5 integration, PSP implementation, VFS, plugin system, security considerations, migration strategy (v2.4, 1300+ lines)
- [Skin Authoring Guide](docs/skin-authoring.md) -- creating custom skins, TOML file reference, theme derivation, effect system, runtime switching
- [PSP Modernization Plan](docs/psp-modernization-plan.md) -- 9-phase, 40-step roadmap for PSP backend modernization using the rust-psp SDK

## License

Dual-licensed under [MIT](LICENSE-MIT) and [Unlicense](LICENSE).
