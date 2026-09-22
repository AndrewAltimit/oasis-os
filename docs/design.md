# OASIS_OS

## An Embeddable Operating System Framework in Rust

**Technical Design Document**
Version 2.6 -- September 2026
Author: Andrew
Classification: Personal Project -- Open Source

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Project Goals and Non-Goals](#2-project-goals-and-non-goals)
3. [System Architecture](#3-system-architecture)
4. [Core Framework](#4-core-framework)
5. [Backend Abstraction Layer](#5-backend-abstraction-layer)
6. [Skin System](#6-skin-system)
7. [Unreal Engine 5 Integration](#7-unreal-engine-5-integration)
8. [PSP Platform Implementation](#8-psp-platform-implementation)
9. [Linux / Raspberry Pi Platform](#9-linux--raspberry-pi-platform)
10. [PSP Remote Agent Control](#10-psp-remote-agent-control)
11. [Virtual File System](#11-virtual-file-system)
12. [Development Workflow](#12-development-workflow)
13. [Plugin System](#13-plugin-system)
14. [Audio System](#14-audio-system)
15. [UI Toolkit and Accessibility](#15-ui-toolkit-and-accessibility)
16. [Security Considerations](#16-security-considerations)
17. [Build System and CI/CD](#17-build-system-and-cicd)
18. [Migration Strategy from Original C Codebase](#18-migration-strategy-from-original-c-codebase)
19. [Risk Assessment](#19-risk-assessment)
20. [Success Criteria](#20-success-criteria)
21. [References and Resources](#21-references-and-resources)

---

## 1. Executive Summary

OASIS_OS is an embeddable operating system framework written in Rust. It provides a fully functional, skinnable shell interface -- complete with a scene-graph UI, command interpreter, virtual file system, plugin system, and remote terminal -- that can render anywhere you can provide a pixel buffer and an input stream.

The project was built from scratch in Rust starting in early 2026, inspired by PSP homebrew shells like PSIX. The architecture -- a themed dashboard driven by a custom scene-graph engine called SDI (Simple Display Interface), with platform abstraction via trait-based backends -- turned out to be a natural foundation for something more general. The trait-based backend system designed for cross-platform PSP/SDL/framebuffer portability extends cleanly to a fourth target: rendering onto a texture inside Unreal Engine 5, where in-game computer props become fully interactive systems rather than scripted UI sequences.

The framework supports multiple "skins" -- visual and behavioral personalities that determine what the OS looks like and what it exposes to the user. Seventeen skins are built in: Classic (PSIX-style icon grid dashboard), PSIX Tribute and PSIX HiFi (PSIX homebrew tributes layered on Classic), XP (Windows XP Luna-inspired blue theme with start menu), macOS (macOS-inspired desktop), GNOME (GNOME desktop style), Balatro (neon balatro aesthetic), Retro-CGA (CGA 4-color retro), Paper (minimalist paper/ink), Win95 (Windows 95/98 classic 3D borders), Solarized (Solarized Dark developer palette), Vaporwave (aesthetic purple/pink/cyan), High Contrast (accessibility: black/white/yellow), Altimit (Altimit OS inspired), Corrupted (glitched visual effects), Desktop (windowed environment), and Modern (purple accent with rounded corners). Fourteen of them are TOML directories in `skins/` embedded at build time (`oasis-skin/build.rs` generates the loaders); `corrupted`, `desktop` and `modern` are defined directly in `oasis-skin/src/builtin/`. All skins share the same core: scene graph, command interpreter, virtual file system, browser engine, networking, and plugin infrastructure. The skin defines layout, theme, feature gating, and visual style.

Primary deployment targets are: in-game computers in UE5 projects (rendered as interactive props), real PSP hardware running modern custom firmware (6.60/6.61 with ARK-4), and a tamper-responsive briefcase (a separate hardware project) where a Raspberry Pi 5 boots directly into OASIS_OS as the field-deployable shell. On the briefcase, OASIS_OS is the operator-facing interface for managing AI agents in untrusted environments -- the tamper detection, LUKS encryption, and cryptographic wipe services run alongside it as systemd units. On a PSP connected to infrastructure WiFi, OASIS_OS's remote terminal enables direct command sessions to machines running AI agents, making a 2005 handheld a viable field controller for an AI agent ecosystem. The original C codebase (~15,000 lines) provides the proven UI design; the Rust rewrite provides memory safety, cross-platform backends, and the extensibility to support all targets from a single codebase.

---

## 2. Project Goals and Non-Goals

### 2.1 Goals

- Build an embeddable OS framework in Rust that renders to any pixel buffer and consumes any input stream
- Implement a skin system that separates visual/behavioral personality from core OS logic
- Integrate as a native library in Unreal Engine 5 for interactive in-game computer props
- Port the original C shell OS as the Classic skin, preserving its icon-grid dashboard and theming
- Achieve cross-platform execution on PSP hardware (via rust-psp), PPSSPP emulator, Raspberry Pi (via SDL3/framebuffer), and UE5 (via render target)
- Provide a virtual file system abstraction backed by real files, game assets, or procedural content depending on platform
- Add remote terminal access for headless device management over TCP/IP, doubling as the primary interface for controlling remote AI agents from portable devices
- Implement a scriptable command layer for automation and gameplay scripting
- Establish a plugin/module system for runtime-extensible functionality
- Serve as the user-facing OS for a tamper-responsive briefcase (Raspberry Pi), replacing bare TTY login with a themed, interactive shell
- Leverage the existing AI agent pipelines (Claude, Gemini, Codex, OpenCode, Crush) and MCP server ecosystem for code translation, build orchestration, and automated testing

### 2.2 Non-Goals

- Building a general-purpose operating system kernel (the framework runs atop existing kernels, firmware, or game engines)
- WASM-based sandboxing or binary compatibility (unnecessary complexity for the use case; pure Rust provides better performance and simpler integration)
- Porting the vendored ffmpeg codebase (replaced by modern alternatives or external linkage)
- Supporting commercial game piracy or circumventing DRM protections
- Achieving feature parity with full Linux desktop environments
- Real-time or safety-critical certification

---

## 3. System Architecture

### 3.1 High-Level Architecture

OASIS_OS follows a layered architecture with strict separation between the platform-agnostic core framework, platform-specific backends, skin definitions, and host applications. The core framework never calls platform-specific APIs directly -- all I/O passes through trait boundaries.

| Layer | Responsibility | Scope |
|-------|---------------|-------|
| Core Framework | Scene graph (SDI), command interpreter, virtual file system, config/theming engine, plugin interface, input event pipeline | All platforms |
| Backends | Rendering (GU, SDL3, framebuffer, UE5 render target), input (PSP pad, keyboard/mouse, gamepad, UE5 interaction), networking (pspnet, std::net), file I/O (real FS, game assets) | Platform-specific impl |
| Skins | Layout definitions, theme assets, feature gating, visual style, "personality" (which commands exist, what the file tree looks like, what apps are discoverable) | Content layer |
| Host Application | UE5 game, PSP firmware, Linux init system, briefcase tamper services, or desktop window manager -- owns the OS instance, ticks it, provides render target and input | Integration layer |

### 3.2 Repository Structure

The project is a standalone Cargo workspace at the repository root (edition 2024, workspace version 1.4.0). Library crates depend on `std`. Skins are data-driven configurations loaded at runtime.

The rust-psp SDK is an external dependency hosted at [github.com/AndrewAltimit/rust-psp](https://github.com/AndrewAltimit/rust-psp) and referenced as a git dependency. The PSP backend crate is excluded from the workspace because it requires the `mipsel-sony-psp` target and is built separately with `cargo psp`; the other PSP-target crates are standalone packages with their own `[workspace]` table.

```
oasis-os/
+-- Cargo.toml                       # Workspace root (resolver="2", edition 2024, 40 members)
+-- crates/                          # 50 crate directories
|   +-- oasis-types/                 # Foundation: Color, Button, InputEvent (incl. Key/Modifiers), geometry, backend traits, errors
|   +-- oasis-vfs/                   # Virtual file system: MemoryVfs, RealVfs, GameAssetVfs
|   +-- oasis-platform/              # Platform service traits: Power, Time, USB, Network, OSK
|   +-- oasis-sdi/                   # Scene graph: named registry, z-order, alpha, exact dirty tracking
|   +-- oasis-net/                   # TCP networking, PSK auth, remote terminal (listener + client), saved hosts, FTP transfer
|   +-- oasis-mcp/                   # Optional MCP control server (Streamable HTTP + JSON-RPC)
|   +-- oasis-audio/                 # Audio manager, playlist, shuffle/repeat, MP3/WAV decode, ID3 parsing
|   +-- oasis-ui/                    # 29 Widget impls + ListView, MenuBar, NinePatch, icons, flex/grid layout
|   +-- oasis-wm/                    # Window manager: lifecycle, drag/resize, hit testing, snapping, tiling, animations
|   +-- oasis-skin/                  # TOML skin engine, 17 built-in skins, theme derivation from 9 base colors
|   +-- oasis-terminal/              # ~95 commands + 18 shell builtins, script engine, jobs, line editing
|   +-- oasis-browser/               # HTML/CSS/Gemini engine: DOM, cascade, flex/grid/table layout, transforms, compositor, JS DOM bindings
|   +-- oasis-js/                    # QuickJS-NG: console, DOM glue, Promise fetch, per-origin storage, watchdog
|   +-- oasis-video/                 # MP4/H.264+AAC decode (openh264 + symphonia, or ffmpeg; PSP-safe demux_lite)
|   +-- oasis-vector/                # Vector graphics: scene graph, path ops, icons, frame-driven animations
|   +-- oasis-shader/                # Animated shader wallpapers (voronoi, city lights, ocean waves, calm waves, balatro)
|   +-- oasis-rasterize/             # Shared software rasterizer used by the SDL, WASM and UE5 backends
|   +-- oasis-i18n/                  # Internationalization framework
|   +-- oasis-test-backend/          # Mock / recording backends for tests
|   +-- oasis-app-core/              # App trait, AppAction, layout + render helpers
|   +-- oasis-app-*/                 # 13 app crates: file-manager, settings, media, tv-guide, radio, games, paint,
|   |                                #   text-editor, calculator, browser, network, package-manager, system-monitor
|   +-- oasis-core/                  # Coordination: dashboard, app runner/registry, agent, plugin, script, terminal glue
|   +-- oasis-backend-sdl/           # SDL3 rendering and input (desktop dev + Raspberry Pi)
|   +-- oasis-backend-ue5/           # UE5 render target, software RGBA framebuffer, FFI input queue
|   +-- oasis-backend-wasm/          # Canvas 2D + DOM input + Web Audio, iframe overlay
|   +-- oasis-ffi/                   # C FFI boundary for UE5: exported functions, opaque handles
|   +-- oasis-app/                   # Binary entry points: desktop app + screenshot tool
|   +-- oasis-usb-host/              # Host side of the PSP USB thin client (rusb)
|   +-- oasis-backend-psp/           # [workspace exclude] sceGu rendering, PSP controller, ME video decode
|   +-- oasis-plugin-psp/            # [standalone] kernel-mode PRX: in-game overlay + background music
|   +-- oasis-recovery-psp/          # [standalone] network recovery EBOOT
|   +-- oasis-devloop-psp/           # [standalone] WiFi TCP dev-automation PRX
|   +-- oasis-me-boot/               # [standalone] Media Engine boot PRX
|   +-- oasis-prx-decrypt-psp/       # [standalone] Kirk-based PRX decryptor
|   +-- oasis-usb-{client,debug,trace,vbus}-psp/  # [standalone] USB thin client + host-mode research
+-- skins/                           # 14 external skins (TOML directories)
|   +-- classic/  psix-tribute/  psix-hifi/  altimit/  xp/  macos/  gnome/  win95/
|   +-- balatro/  retro-cga/  paper/  solarized/  vaporwave/  highcontrast/
+-- docs/                            # This document, guides, subsystem docs, ADRs (index: docs/README.md)
+-- examples/                        # Runnable examples registered as [[example]] targets
+-- docker/                          # rust-ci and PPSSPP Dockerfiles
+-- scripts/                         # WASM build/serve, PSP devloop and test scenarios
+-- www/                             # WASM page shell
```

**External dependencies:**

- [rust-psp SDK](https://github.com/AndrewAltimit/rust-psp) (MIT) -- modernized fork with edition 2024, safety fixes, kernel mode support. Referenced as a git dependency from the PSP crates.

**Built-in-only skins (defined in `oasis-skin/src/builtin/`):**

- `corrupted` -- Glitched OS: randomized layouts, visual artifacts
- `desktop` -- Window-like panels, taskbar, start menu analog
- `modern` -- Purple accent, rounded corners, gradient fills

**External skins (TOML directories in `skins/`, embedded at build time):**

- `classic` -- PSIX-style icon grid dashboard, status bar, chrome bezels
- `psix-tribute` -- PSIX tribute: shaped orange chrome, free desktop icons (inherits `classic`)
- `psix-hifi` -- High-fidelity PSIX tribute: baked ribbon wallpaper, left-column icons, media dock (inherits `classic`)
- `altimit` -- Altimit-style vector icon dashboard
- `xp` -- Windows XP Luna theme with start menu (the built-in variant runs at 1024x768)
- `macos` -- macOS-inspired desktop with traffic-light buttons and dock
- `gnome` -- GNOME desktop style with Activities bar
- `balatro` -- Neon balatro aesthetic with glowing accents
- `retro-cga` -- CGA 4-color retro palette
- `paper` -- Minimalist paper/ink style
- `win95` -- Windows 95/98 classic look with raised 3D borders, system gray, blue titlebars
- `solarized` -- Solarized Dark color palette, clean and developer-focused
- `vaporwave` -- Aesthetic vaporwave palette with purple, pink, and cyan gradients
- `highcontrast` -- High contrast dark theme for accessibility: black background, white text, yellow accents

**Implemented infrastructure (in `docker/`):**

- `rust-ci.Dockerfile` -- CI toolchain image (rust:1.93-slim + cmake + X11/audio dev libs + nightly + cargo-deny)
- `ppsspp.Dockerfile` + `ppsspp-entrypoint.sh` -- PPSSPP Docker container for automated PSP testing
- `ppsspp-patches/` -- placeholder for PPSSPP patches (see §12.2)

**Planned crates (not yet created):**

- `oasis-backend-framebuffer` -- Direct Linux framebuffer rendering (headless Pi)
- `oasis-platform-psp` -- PSP hardware services: USB, UMD, power, Media Engine, kernel services
- `oasis-platform-linux` -- Pi-specific: GPIO (rppal), systemd integration, sysfs queries

**Workspace Cargo.toml (abridged):**

```toml
[workspace]
resolver = "2"
members = [
    "crates/oasis-types", "crates/oasis-vfs", "crates/oasis-platform", "crates/oasis-sdi",
    "crates/oasis-net", "crates/oasis-mcp", "crates/oasis-audio", "crates/oasis-ui",
    "crates/oasis-wm", "crates/oasis-skin", "crates/oasis-terminal", "crates/oasis-browser",
    "crates/oasis-js",
    # oasis-app-core + 13 oasis-app-* crates
    "crates/oasis-app-core", "crates/oasis-app-games", "crates/oasis-app-paint", # ...
    "crates/oasis-vector", "crates/oasis-shader", "crates/oasis-core",
    "crates/oasis-backend-sdl", "crates/oasis-backend-ue5", "crates/oasis-backend-wasm",
    "crates/oasis-ffi", "crates/oasis-app", "crates/oasis-video", "crates/oasis-rasterize",
    "crates/oasis-i18n", "crates/oasis-test-backend", "crates/oasis-usb-host",
]   # 40 members in total
# PSP crate excluded from workspace -- it requires mipsel-sony-psp target
# and the rust-psp SDK. Build separately with cargo-psp:
#   cd crates/oasis-backend-psp && cargo psp --release
exclude = [
    "crates/oasis-backend-psp",
]

[workspace.package]
version = "1.4.0"
edition = "2024"
license = "MIT OR Unlicense"
repository = "https://github.com/AndrewAltimit/oasis-os"
authors = ["AndrewAltimit"]

[workspace.lints.clippy]
clone_on_ref_ptr = "warn"
dbg_macro = "warn"
todo = "warn"
unimplemented = "warn"
unwrap_used = "deny"

[workspace.lints.rust]
unsafe_op_in_unsafe_fn = "warn"
```

See the root `Cargo.toml` for the full member list, `[workspace.dependencies]` and release profile.

The PSP backend crate has its own standalone `Cargo.toml` (not workspace-inherited) because it targets `mipsel-sony-psp` and depends on the standalone rust-psp SDK:

```toml
# crates/oasis-backend-psp/Cargo.toml (abridged)
[workspace]  # standalone workspace

[package]
name = "oasis-backend-psp"
version = "1.4.0"
edition = "2024"
license = "MIT OR Unlicense"

[dependencies]
psp = { git = "https://github.com/AndrewAltimit/rust-psp", features = ["std"] }
oasis-core = { path = "../oasis-core", default-features = false }
oasis-video = { path = "../oasis-video", default-features = false, features = ["no-std-demux"] }
```

### 3.3 Platform Targeting

| Target | Cargo Command | Render Backend | Input Backend | VFS Backend | Status |
|--------|--------------|----------------|---------------|-------------|--------|
| Desktop (dev) | `cargo build --release -p oasis-app` | SDL3 | Keyboard/mouse | Real Linux FS | Implemented |
| PSP / PPSSPP | `cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo psp --release` | sceGu hardware (Sprites) | PSP controller | ms0:/ real FS | Implemented |
| UE5 (in-game) | `cargo build --release -p oasis-ffi` (cdylib) | UE5 render target | UE5 interaction | Game asset VFS | Implemented (FFI ready) |
| Raspberry Pi (briefcase) | `cargo build --release -p oasis-app --target aarch64-unknown-linux-gnu` | SDL3 | Keyboard/gamepad | Real Linux FS | Planned (SDL3 backend works, cross-compile not yet tested) |

Note: The PSP backend is a standalone crate excluded from the workspace. It depends on `oasis-core` directly and re-exports shared types (`Color`, `Button`, `Trigger`, `TextureId`, `InputEvent`). Std support on the PSP target is provided by the rust-psp SDK's `main` branch with `RUST_PSP_BUILD_STD=1`, which builds a custom sysroot with PSP-specific PAL implementations. A `ColorExt` extension trait provides PSP-specific ABGR conversion without modifying oasis-core. The PSP backend uses sceGu hardware-accelerated 2D rendering with `Sprites` primitives and renders a PSIX-style UI (document icons, tabbed bars, chrome bezels, wave arc wallpaper) matching the desktop layout.

---

## 4. Core Framework

### 4.1 SDI -- Simple Display Interface

SDI is the scene-graph engine at the center of the framework. The original C implementation (~1,574 lines in `sdi.c`) manages named UI objects with position, size, alpha, z-order, and image data through a flat registry. The Rust SDI replaces string-keyed C arrays with a `HashMap<String, SdiObject>` registry, with the borrow checker enforcing exclusive mutation.

SDI is deliberately simple. It is not a DOM, not a layout engine, and not a retained-mode GUI framework. It is a flat collection of named, positionable, blittable objects with z-ordering and alpha blending. This simplicity is a feature -- it maps directly to what every backend can provide efficiently, from PSP GU hardware to a UE5 texture blit.

| SDI Operation | Original C | Rust Equivalent |
|--------------|-----------|-----------------|
| Create object | `sdi_new("name")` | `sdi.create("name")` |
| Set position | `sdi_set_x("name", 100)` | `sdi.get_mut("name")?.x = 100` |
| Load image | `sdi_load_png("name", path)` | `sdi.load_png("name", path)?` |
| Draw frame | `sdi_draw()` | `sdi.draw(&mut backend)?` |
| Z-ordering | `sdi_move_top("name")` | `sdi.move_to_top("name")` |
| Theme loading | `sdi_load_state(path)` | `sdi.load_theme(path)?` |

**Key traits:** `SdiCore` defines 13 required rendering methods and `SdiBackend` adds 53 optional accelerated primitives with default implementations, spread over nine extension traits. The scene graph calls `backend.blit()`, `backend.clear()`, `backend.swap_buffers()` -- it never knows what surface it's drawing to.

### 4.2 Command Interpreter

The command interpreter is a registry-based dispatch system in the `oasis-terminal` crate. Commands implement a `Command` trait with an `execute()` method returning structured output. The interpreter includes full shell features: variable expansion (`$VAR`, `${VAR}`), glob expansion, aliases, history (`!!`, `!n`), piping, redirects, and command chaining (`&&`, `||`, `;`). Inline control flow and `run` scripts share one parser (`script.rs`) with nested `if`/`elif`/`else`, `while`/`until`, `for` and `case` blocks; a trailing `&` queues a background job managed with `jobs`/`fg`/`bg`/`kill %N`. Hosts drive interactive terminals through a `ShellSession` (`session.rs`) that adds readline-style editing, tab completion (commands, `$VARS`, VFS paths), Ctrl+R history search and history persisted to `/home/user/.oasis_history`. Skins control which commands are registered -- a terminal-focused skin exposes everything, a locked-down kiosk skin exposes only approved commands, a corrupted skin registers broken versions of standard commands that produce garbled output. The shell ships with agent-management commands for remote agent interaction (see Section 10).

About 95 registered commands (76 from `oasis-terminal`, 19 registered by `oasis-core`) plus 18 shell builtins. The catalogue lives in [`terminal-commands.md`](terminal-commands.md).

| Command Module | Commands | Description |
|----------------|----------|-------------|
| Core (13) | ls, cd, pwd, cat, mkdir, rm, echo, clear, status, touch, cp, mv, find | File system operations and shell basics |
| Platform (4) | power, clock, memory, usb | Platform service queries |
| Remote (3) | listen, remote, hosts | Remote terminal listener, outbound sessions, saved hosts |
| Text Processing (10) | head, tail, wc, grep, sort, uniq, tee, tr, cut, diff | Text filtering and transformation |
| File Utilities (7) | write, append, tree, du, stat, xxd, checksum | File creation, disk usage, hex dump, hashing |
| System (6) | uptime, df, whoami, hostname, date, sleep | System information and timing |
| Dev Tools (7) | base64, json, uuid, seq, expr, test, xargs | Encoding, parsing, testing, scripting utilities |
| Fun/Utility (7) | cal, fortune, banner, matrix, yes, watch, time | Calendars, ASCII art, command timing |
| Security (4) | chmod, chown, passwd, audit | Permission management and auditing |
| Documentation (3) | man, tutorial, motd | Manual pages, tutorials, message of the day |
| Network (3) | wifi, ping, http | WiFi control, connectivity, HTTP requests |
| Audio / Radio (2) | music, radio | Playlist control, internet radio |
| Skin (1) | skin | Skin switching and listing |
| UI (5) | wm, sdi, theme, notify, screenshot | Window management, SDI inspection (`sdi list` / `sdi get`), notifications |
| Registered by oasis-core (19) | agent, board, ci, health, mcp, tamper, plugin, cron, startup, ftp, push, pull, update, tv, browse, curl, fetch, gemini, sandbox | Agent/MCP, plugins, scripting, file transfer, updates, TV, browser |
| Shell builtins (18) | help, run, history, set, unset, env, alias, unalias, which, return, break, continue, local, function, jobs, fg, bg, kill | Handled by the executor |

The desktop app additionally registers `mcp-server` when built with the `mcp` feature.

### 4.3 Input Pipeline

The input system translates platform-specific events into a platform-agnostic `InputEvent` enum. Every backend maps its native input to this enum. The core framework never sees raw platform input.

| Event Type | PSP Source | Desktop/Pi Source | UE5 Source |
|-----------|-----------|-------------------|------------|
| `CursorMove(x, y)` | Analog stick | Mouse motion / gamepad stick | Line trace UV -> screen coords |
| `ButtonPress(Button)` | D-pad, face buttons | Keyboard / gamepad | Mapped interact keys |
| `Trigger(Trigger)` | L/R shoulder | Keyboard / gamepad triggers | Shoulder buttons on gamepad |
| `TextInput(char)` | On-screen keyboard | Physical keyboard | Captured keyboard when "seated" |
| `PointerClick(x, y)` | N/A | Mouse click | Line trace hit on monitor mesh |
| `FocusGained` | N/A (always focused) | Window focus event | Player enters interaction range |
| `FocusLost` | N/A | Window blur event | Player walks away from terminal |
| `Key { key, mods }` | N/A (never emitted) | Every physical key-down + held modifiers | `OASIS_EVENT_KEY` via FFI |

#### 4.3.1 Keyboard Model: Gamepad Events + Raw Keys

Keyboard input is delivered in two layers so the PSP / gamepad path is untouched while desktop apps get real shortcuts:

1. **Gamepad-style events** (`ButtonPress`, `TriggerPress`, `Backspace`, `Tab`, ...). Keyboard backends synthesize these from keys with one canonical table, `Key::legacy_press` / `Key::legacy_release` in `oasis-types/src/input.rs` (arrows -> d-pad, Enter -> Confirm, Esc -> Cancel, Space -> Triangle, F1/F2 -> Start/Select, Q/E -> L/R triggers, F11 -> fullscreen).
2. **Raw keys**: `InputEvent::Key { key: Key, mods: Modifiers }`. `Key` covers `Char(c)` (letters lowercase), `Space`, `Enter`, `Escape`, `Tab`, `Backspace`, `Delete`, `Insert`, `Home`, `End`, `PageUp`, `PageDown`, arrows and `F(1..=12)`. `Modifiers` is a bitflags-style set (`SHIFT`, `CTRL`, `ALT`, `SUPER`) with helpers such as `ctrl()`, `has_command()` and `only(Modifiers::CTRL)` for exact shortcut matching.

For each key-down the SDL and WASM backends emit, in order: `Key`, then its gamepad-style twin (if any), then `TextInput` for printable characters. The host routes `Key` to the focused app's `App::handle_key(&Key, Modifiers, &dyn Vfs) -> Option<AppAction>` (default `None`). A `KeyTwinFilter` then drops the twin that immediately follows when either:

- the app returned `Some(action)` (it consumed the key as a shortcut, so e.g. an app claiming `Key::Enter` does not also receive `Button::Confirm`), or
- the focused target accepts text (`App::accepts_text()`, the terminal, or the browser URL bar / a focused form field) and the key merely types a character (`Key::produces_text`). This is what stops "q"/"e" from switching virtual desktops and Space from firing Triangle while typing in the text editor.

Gamepad events that do not immediately follow a matching `Key` are never filtered. Apps should type from `handle_text_input`, keep every feature reachable through `handle_input` (PSP has no raw keys), and use `handle_key` for keyboard-only accelerators (Ctrl+S/Z/C/V, Delete, Home/End, PageUp/PageDown). FFI hosts send raw keys as `OASIS_EVENT_KEY` (see [`ffi-integration.md`](ffi-integration.md)).

#### 4.3.2 App VFS Access

Every input / render hook on `App` sees at most a shared `&dyn Vfs`, and `handle_click` sees none. Apps that need to change the file system queue the work and implement `App::apply_vfs_ops(&mut dyn Vfs) -> bool` (default no-op), which the SDL and WASM hosts call once per frame for every open app runner (`AppRunner::apply_vfs_ops`). The File Manager drains its delete / mkdir / rename / copy / move queue there, and Paint writes its binary BMP saves there (the older `take_pending_request` path only carries UTF-8 strings). Read-only follow-ups to a click (navigation, listing a folder) still go through `App::refresh(&dyn Vfs)`, which the host calls right after forwarding a click or a consumed key. Hosts forward only discrete clicks to apps — there are no drag / pointer-move events at the app level — so pointer-driven tools (e.g. Paint strokes) work click-by-click.

#### 4.3.3 App Ticking

`App::tick(dt_ms: u32, &dyn Vfs) -> bool` (default no-op returning `false`) advances time-driven app state. The SDL and WASM hosts call it once per frame for every open runner (`AppRunner::tick`), windowed or fullscreen, focused or not, with the wall time since the previous call — on the SDL host this includes frames whose redraw is elided, so app time never depends on the frame rate or on input. Apps accumulate `dt_ms` instead of counting calls, and return `true` when the tick changed something they draw (the runner then schedules a redraw, see §4.3.4). `refresh` is not a clock: it only runs after clicks and consumed keys.

- **Games** run Snake and Memory Match on a fixed 60 Hz simulation step (wall time is accumulated and at most 250 ms is simulated per tick, so a stalled host doesn't replay seconds of game time at once). The Snake high score is persisted to `/home/user/.games.toml` (`[snake] high_score = N`), loaded lazily on the first tick and written through `apply_vfs_ops` when beaten.
- **Photo Viewer** slideshows advance every `SLIDESHOW_INTERVAL_MS` (5 s). **Music Player** shuffle picks a random other playlist track (`oasis_skin::SimpleRng`) and Previous retraces the shuffled order.

#### 4.3.4 Frame Requests and Idle Elision

The SDL host skips clear/draw/present on frames where nothing visible changed. Window content is painted through the WM draw callback, outside the SDI dirty tracking, so an open window no longer forces a redraw by itself; instead `render::windows_want_frame` ORs over the visible (non-minimized, active-desktop) windows:

- `AppRunner::wants_frame()` — true until the next drawn frame (`mark_drawn`) after any input, a `tick` returning `true`, applied VFS ops, a change to the mirrored lines / browse dir / viewing file, or mutable access via `delegate_as_mut`; and always true while `App::wants_frame()` is (continuous content such as the video embed).
- `BrowserWidget::wants_frame()` — pending layout / repaint / dirty rects / unpainted scroll, a page or its resources still loading, and active CSS animations or transitions. Fired JS timers dirty the layout during `tick`, which runs on elided frames too.
- WM window animations and drags.

Video/audio playback, fullscreen kiosk apps, the input grace window and the 1 s heartbeat still force redraws as before (`OASIS_FRAME_STATS=1` reports the elided share). A running Snake game requests a frame only on the ~10 steps per second it actually moves.

### 4.4 Remote Terminal

On platforms with networking (PSP via infrastructure WiFi, Linux, desktop), the framework runs a TCP listener that accepts remote terminal connections. The remote terminal feeds keystrokes into the same command interpreter as local input. This is functional on real hardware and in PPSSPP (1.19+ maps `sceNetInet` to host sockets). In UE5, the terminal is available for debugging -- connect to `localhost:9000` while the game is running to interact with any in-game computer's OS instance directly.

On the briefcase Pi, the remote terminal is the primary access path for headless operation -- SSH into the Pi, then connect to the OASIS_OS terminal port for a full interactive shell over the network. OASIS_OS can also act as a client, establishing outbound TCP sessions to remote machines with the `remote` command (see Section 10). The terminal `listen` command binds to loopback only; `RemoteListener` refuses to listen on all interfaces without a pre-shared key.

### 4.5 Window Manager

The window manager (WM) enables skins that present multiple movable, resizable, overlapping windows -- producing the feel of a real desktop operating system. The WM is a consumer of the SDI API, not a modification to it. SDI remains a flat, dumb scene graph; the WM is the smart layer that creates, positions, and manipulates groups of SDI objects to simulate windowed interfaces.

#### 4.5.1 Design Principle: SDI Stays Flat

SDI has no concept of grouping, containment, or parent-child relationships. Every object is independent. The WM introduces the illusion of hierarchy by adopting a naming convention: a window named `"editor_01"` owns SDI objects `"editor_01.frame"`, `"editor_01.titlebar"`, `"editor_01.title_text"`, `"editor_01.btn_close"`, `"editor_01.btn_minimize"`, and `"editor_01.content"`. When the WM moves a window, it updates the position of every object sharing that prefix by the same delta. When it brings a window to front, it calls `sdi.move_to_top()` for every object in the group.

This design preserves SDI's simplicity and portability. The PSP target (which has no use for overlapping windows on a 480x272 screen) never instantiates a WM. Skins that don't need windows (Classic, Corrupted) skip it entirely. The WM only exists when a skin's `features.toml` enables it.

#### 4.5.2 Window Anatomy

Each window is a `Window` struct in the WM that tracks its SDI child objects, state, and geometry. The visual composition of a window is defined by the active skin's theme -- the WM handles behavior, the skin handles appearance.

| SDI Object | Role | Skin Provides |
|-----------|------|---------------|
| `{id}.frame` | Background rectangle, receives drop shadow or border | Frame image/color, corner radius, border width |
| `{id}.titlebar` | Draggable title bar region, colored band at top | Titlebar height, background color/gradient, active vs inactive colors |
| `{id}.title_text` | Window title label | Font, size, color, alignment within titlebar |
| `{id}.btn_close` | Close button, top-right of titlebar | Button icon (normal, hover, pressed states) |
| `{id}.btn_minimize` | Minimize button, left of close | Button icon (normal, hover, pressed states) |
| `{id}.btn_maximize` | Maximize/restore toggle | Button icon (normal, restore states) |
| `{id}.resize_n/s/e/w/ne/nw/se/sw` | Invisible resize handles at edges and corners | Handle hit area size (typically 4-8 pixels) |
| `{id}.content` | Content area where the "app" renders | Content background color, padding, scrollbar style |

#### 4.5.3 Window Operations

| Operation | WM Behavior | SDI Calls |
|-----------|------------|-----------|
| **Create** | Allocate Window struct, create all child SDI objects at initial position, register in window list | `sdi.create()` for each child object, `sdi.load_png()` for frame/button assets |
| **Move (drag)** | On titlebar drag: calculate cursor delta per frame, apply to all child object positions | `sdi.get_mut("{id}.*")?.x += dx` for every child |
| **Resize** | On resize handle drag: update window dimensions, reposition edges and corners, notify content renderer of new available area | Reposition/resize affected child objects, `backend.set_clip_rect()` updated |
| **Focus (bring to front)** | Move all child objects to top of z-order, mark as active, update titlebar to active color | `sdi.move_to_top("{id}.*")` for every child |
| **Minimize** | Hide all child objects; taskbar button remains (click to restore) | `sdi.set_visible("{id}.*", false)`, taskbar `update_sdi()` shows minimized state |
| **Maximize** | Snap window to full content area, store previous geometry for restore | Update all child positions/sizes to fill screen |
| **Close** | Call app's shutdown hook, destroy all child SDI objects, remove from window list | `sdi.destroy("{id}.*")` for every child |
| **Cascade** | Position newly created windows offset from the last, wrapping when reaching screen edge | Initial position = last window position + (24, 24) offset |

#### 4.5.4 Hit Testing

When a `PointerClick(x, y)` event arrives, the WM performs hit testing to determine the target. The test walks the window list in reverse z-order (topmost first) and checks regions in priority order:

1. **Titlebar buttons** -- minimize, maximize, close (left to right, regardless of `button_side`). Triggers the corresponding window operation.
2. **Titlebar body** -- initiates a drag operation. Subsequent `CursorMove` events move the window until pointer release. A second click within 500 ms and 6 px of the first is treated as a double-click and toggles maximize/restore.
3. **Resize handles** -- initiates a resize operation on the corresponding edge/corner.
4. **Content area** -- forwards the click (translated to content-local coordinates) to the app running inside the window.
5. **No hit** -- click landed on the desktop background. Deselects all windows, optionally triggers a desktop context menu.

If the clicked window is not already the topmost, the WM brings it to front before dispatching the event. This matches the standard desktop behavior of click-to-focus.

#### 4.5.5 Content Clipping

When a window's content exceeds its content area (scrolling text, tall file listings, large images), the content must be clipped at the window boundary. This requires a `set_clip_rect(x, y, w, h)` method on the `SdiCore` trait -- one of the 13 required backend methods.

| Backend | Clip Implementation |
|---------|-------------------|
| PSP (GU) | `sceGuScissor(x, y, w, h)` -- hardware scissor rectangle |
| SDL3 | `SDL_SetRenderClipRect(&rect)` -- renderer-level clip |
| Framebuffer | Software clip during blit -- skip pixels outside rect |
| UE5 Render Target | Software clip during blit into RGBA buffer |

Clip state is pushed before rendering a window's content and popped afterward, so each window's content is isolated. The clip stack supports nesting for windows that contain sub-panels.

#### 4.5.6 Window Types

Skins define which window types are available. The WM provides the behavioral templates; the skin provides the visual treatment.

| Window Type | Behavior | Use Case |
|------------|----------|----------|
| **App Window** | Draggable, resizable, closable, minimizable, maximizable. Content rendered by an app module. Has titlebar, frame, and all controls. | Card database viewer, deck builder, text editor, file browser, email client |
| **Dialog** | Modal. Centered on screen, not draggable (or constrained to screen center). Blocks input to other windows until dismissed. Dim overlay behind. | Confirmation prompts, error messages, login screens, save dialogs |
| **Panel** | Docked to a screen edge. Not draggable (or constrained to its edge). Auto-hides optionally. | Taskbar (bottom), sidebar (left), notification tray (top-right), status bar |
| **Floating Widget** | Small, always-on-top, draggable. No minimize/maximize. Minimal or no frame. | Clock, system monitor, notification toasts, quick-launch shortcuts |
| **Fullscreen** | No frame, no titlebar, covers entire content area. App controls its own exit mechanism. | Terminal emulator, game within a game, video player, boot splash |

#### 4.5.7 Platform Applicability

Not every platform needs or benefits from the window manager. The skin's feature gates control availability.

| Platform | WM Enabled | Rationale |
|----------|-----------|-----------|
| PSP | No | 480x272 at 30-60fps. Screen too small for overlapping windows. Classic skin uses fixed-position SDI objects for its icon grid. |
| Raspberry Pi (briefcase) | Optional | Depends on display. Pi 5 with HDMI to a monitor inside or attached to the briefcase lid can run the Desktop or Modern skin with windowing. Headless operation uses the Classic skin via remote terminal. |
| Desktop (dev) | Yes | Full resolution display. Desktop skin uses windowed interface. Also useful for debugging -- inspect multiple OS subsystems simultaneously. |
| UE5 (in-game) | Yes | In-game computers with Desktop skin present windowed interfaces on the virtual monitor. The WM resolution matches the virtual screen texture (e.g., 1024x768 for a high-res in-game monitor, 480x272 for a handheld prop). |

#### 4.5.8 Skin-Driven Appearance

The WM handles behavior uniformly; the skin defines every visual detail. Two skins using the same WM can look completely different:

| Visual Property | Desktop Skin | Corrupted Skin |
|----------------|-------------|---------------|
| Titlebar | Clean gradient, centered text, rounded top corners | Jagged edges, flickering title text, random color shifts per frame |
| Frame | 1px subtle border, soft drop shadow | Torn border with missing segments, no shadow, occasional frame duplication offset by a few pixels |
| Close button | Standard x icon, red on hover | Glitched icon that sometimes shows x, sometimes shows random glyph |
| Drag behavior | Smooth 1:1 cursor tracking | Jittery -- WM adds random +/-2px noise to position each frame |
| Resize | Smooth, snaps to grid optionally | Fights back -- window occasionally resizes itself randomly |
| Window creation | Clean fade-in animation | Stuttered appearance with screen tear artifact |

The corrupted skin demonstrates that the WM's behavioral hooks (position update, resize calculation, animation tick) can be intercepted by skin-defined modifiers. The WM exposes optional callback points where the skin can inject visual distortion, and the skin's `features.toml` declares which modifiers to apply.

Titlebar layout is skin-driven too: `button_side = "left"` gives macOS order (close, minimize, maximize from the corner inward), `"right"` gives minimize, maximize, close. `Window::title_layout` reserves the button group on its side (mirrored on the other side for `title_align = "center"`), centers titles on their measured glyph width, and truncates titles that do not fit with `...`.

#### 4.5.9 Snapping, Keyboard Management and Animation

- **Drag-to-edge snapping.** While a titlebar drag is in a screen-edge zone (16 px edges, 64 px corners) the WM publishes `WindowManager::snap_preview()`, which the desktop host draws as a translucent rectangle tinted with the titlebar color. Releasing snaps the window to the left/right half, a corner quarter, or maximizes it (top edge), all within the work area (screen minus `maximize_top_inset` / `maximize_bottom_inset`). Dragging a snapped window away restores its pre-snap size; resizing by hand clears the snap.
- **Keyboard shortcuts** (desktop mode, handled before keys reach the focused app; every combo includes Ctrl, Alt or Super, so plain Tab/arrows still reach text-entry windows):

| Shortcut | Action |
|----------|--------|
| Alt+Tab / Alt+Shift+Tab | Cycle focus forward / backward, skipping minimized windows (disabled while a modal is open) |
| Super+Left/Right or Ctrl+Alt+Left/Right | Snap the active window to that half; the opposite direction unsnaps |
| Super+Up or Ctrl+Alt+Up | Maximize |
| Super+Down or Ctrl+Alt+Down | Restore a maximized/snapped window, otherwise minimize |
| Super+T or Ctrl+Alt+T | Cycle tiling: master/stack, grid, columns, rows, monocle, then back to floating (pre-tiling geometry restored) |

- **Animations.** `WindowManager::set_motion_enabled(true)` + a per-frame `tick_animations(sdi)` animate open (grow + fade in), close (shrink + fade out; SDI objects are destroyed only when it ends), minimize (shrink toward the bottom edge) and restore-from-minimized. Motion is off by default so headless hosts and the screenshot tool stay instant; the desktop app enables it unless the skin sets `features.reduced_motion`. Animations are visual only — logical geometry and state change immediately.

---

## 5. Backend Abstraction Layer

### 5.1 Rendering Backend Trait

Every rendering operation passes through `SdiCore` (13 required methods) and `SdiBackend` (53 optional accelerated primitives across nine extension traits). Four implementations exist: PSP (GU), SDL3, WASM (Canvas 2D), and UE5 (software RGBA buffer).

| Method | PSP (GU) | SDL3 | Framebuffer [PLANNED] | UE5 Render Target |
|--------|---------|------|-------------|-------------------|
| `init()` | sceGuInit + display list | SDL_CreateWindow + Renderer | open /dev/fb0 + mmap | Allocate RGBA buffer |
| `blit(tex, x, y, w, h)` | sceGuDrawArray (Sprites) | SDL_RenderTexture | memcpy to mapped buf | memcpy to RGBA buffer |
| `swap_buffers()` | sceGuSwapBuffers + VBlank | SDL_RenderPresent | ioctl WAITFORVSYNC | Set dirty flag (UE5 reads) |
| `load_texture(data)` | RAM alloc (16-byte aligned) | SDL_CreateTexture | Decode to RGB buf | Decode to RGBA buf |
| `clear(color)` | sceGuClear | SDL_RenderClear | memset framebuffer | memset RGBA buffer |
| `shutdown()` | sceGuTerm | SDL_Destroy* | munmap + close | Free buffer |
| `get_buffer()` | N/A | N/A | N/A | Return raw RGBA ptr |
| `set_clip_rect(x,y,w,h)` | sceGuScissor | SDL_SetRenderClipRect | Software clip bounds | Software clip bounds |
| `reset_clip_rect()` | sceGuScissor(full) | SDL_SetRenderClipRect(NULL) | Clear clip bounds | Clear clip bounds |

The UE5 backend is unique in that it doesn't own a display. It renders to a shared memory buffer that UE5 reads via `get_buffer()`. This is the zero-copy bridge: Rust writes pixels, UE5's `UpdateTextureRegions()` reads them directly.

### 5.2 Input Backend Trait

| Event Type | PSP Source | Desktop/Pi Source | UE5 Source |
|-----------|-----------|-------------------|------------|
| `CursorMove(x, y)` | Analog stick via sceCtrlReadBufferPositive | Mouse motion or gamepad right stick | Line trace UV mapped to virtual screen |
| `ButtonPress(Button)` | D-pad, Cross, Circle, Triangle, Square | Keyboard keys or gamepad buttons | Interact key bindings |
| `Trigger(Trigger)` | L/R shoulder buttons | Keyboard L/R or gamepad triggers | Gamepad triggers |
| `TextInput(char)` | On-screen keyboard (sceUtilityOsk) | Physical keyboard input | Keyboard captured during interaction |

### 5.3 Network Backend Trait

| Operation | PSP (sceNetInet) | Linux (std::net) | UE5 (debug only) |
|-----------|-----------------|------------------|-------------------|
| TCP listen | sceNetInetSocket + Bind + Listen | TcpListener::bind() | TcpListener::bind() on localhost |
| TCP accept | sceNetInetAccept | listener.accept() | listener.accept() |
| TCP send/recv | sceNetInetSend / Recv | stream.read() / write() | stream.read() / write() |
| TCP connect (outbound) | sceNetInetConnect | TcpStream::connect() | TcpStream::connect() |
| WiFi connect | sceNetApctlConnect | N/A (OS manages) | N/A |
| DNS resolve | sceNetResolverStartNtoA | std::net::ToSocketAddrs | std::net::ToSocketAddrs |

Note: outbound TCP connect is essential for the PSP remote agent control use case (Section 10).

---

## 6. Skin System

### 6.1 Skin Architecture

A skin is a data-driven configuration that defines the visual and behavioral personality of an OS instance. Skins do not contain code -- they are TOML manifests referencing layout definitions, theme assets, and feature flags. The core framework interprets skins at runtime.

| Skin Component | Description | Format |
|---------------|-------------|--------|
| Manifest | Name, version, author, description, target resolution, required plugins | `skin.toml` |
| Layout | SDI object definitions: names, positions, sizes, z-order, visibility defaults | `layout.toml` |
| Theme | Colors, fonts, images, backgrounds, icon sets, animations | Asset directory + `theme.toml` |
| Feature gates | Which command categories are available, which dashboard pages exist, whether file browser is enabled | `features.toml` |
| VFS overlay | What the virtual file system root looks like -- which directories exist, what files are pre-populated | `filesystem.toml` + content directory |
| Strings | All user-facing text: menu labels, command help text, error messages, boot sequence text | `strings.toml` |

### 6.2 Skins

The table below covers the skins with distinct behavior; the remaining TOML skins (§3.2) are visual variations of the dashboard or desktop layouts.

| Skin | Description | Primary Target | Use Case | Source |
|------|-------------|---------------|----------|--------|
| Classic | PSIX-style icon grid dashboard with document icons, tabbed status/bottom bars, chrome bezels, wave arc wallpaper | PSP / Pi / Desktop | Homebrew shell OS, the original C codebase experience modernized with PSIX styling | External (`skins/classic/`) |
| XP | Windows XP Luna-inspired blue theme with gradient titlebars, taskbar, and start menu | Desktop / UE5 | Nostalgic desktop experience, in-game civilian computer props | External (`skins/xp/`) + Built-in |
| Corrupted | Randomized SDI object positions/alpha, garbled command output, visual glitch artifacts, "repair" puzzle hooks | UE5 | Damaged terminals the player must fix, environmental storytelling | Built-in |
| Desktop | Window-like panels, taskbar, start menu analog, multiple "apps" visible simultaneously | UE5 / Pi | Civilian in-game computers, player home terminals | Built-in |
| Modern | Purple accent theme with rounded corners, gradient fills, shadows, and full feature set | Desktop / UE5 | Contemporary aesthetic, feature showcase | Built-in |

### 6.3 Skin Loading and Switching

Each OS instance is initialized with a skin. Skins can be hot-swapped at runtime (e.g., a "corrupted" terminal that the player "repairs" transitions to a clean variant). The core framework loads the new skin manifest, tears down the current SDI object tree, rebuilds it from the new layout, reloads theme assets, and reconfigures feature gates. The VFS overlay is preserved across skin swaps -- file state persists.

---

## 7. Unreal Engine 5 Integration

### 7.1 Integration Architecture

OASIS_OS compiles as a static library (`.lib` / `.a`) or C-compatible dynamic library (`.dll` / `.so`) that UE5 links against. A C FFI boundary in `crates/oasis-ffi/` exports an opaque handle API. The UE5 side wraps this in an Actor Component that owns an OS instance, ticks it per frame, and bridges rendering and input.

```
+-----------------------------------------------------+
|  Unreal Engine 5                                    |
|                                                     |
|  AComputerTerminalActor                             |
|    +-- UStaticMeshComponent (monitor prop)          |
|    +-- UOasisOSComponent                             |
|    |     +-- Owns: oasis_instance_t* (opaque FFI)    |
|    |     +-- Owns: UTexture2D (render target)       |
|    |     +-- Tick(): oasis_tick(handle, dt)           |
|    |     +-- Render(): oasis_get_buffer(handle)       |
|    |     |            -> UpdateTextureRegions()      |
|    |     +-- Input(): oasis_send_input(handle, evt)   |
|    +-- UMaterialInstanceDynamic (maps texture to mesh)|
|                                                     |
|  On player interaction:                             |
|    Line trace -> hit monitor mesh                   |
|    Calculate UV at hit point                        |
|    Map UV to virtual screen coordinates             |
|    Send CursorMove/PointerClick to oasis_send_input  |
|    Capture keyboard -> send TextInput events        |
+-----------------------------------------------------+
        | FFI boundary (C ABI)
+-------+---------------------------------------------+
|  OASIS_OS (Rust static lib)                          |
|                                                     |
|  oasis_create(w, h, skin, layout, features) -> handle|
|  oasis_tick(handle, delta_time)                      |
|  oasis_send_input(handle, event_type, x, y, key)    |
|  oasis_get_buffer(handle) -> *const u8, width, height|
|  oasis_get_dirty(handle) -> bool                     |
|  oasis_destroy(handle)                               |
|                                                     |
|  Internal: SdiBackend::UE5RenderTarget              |
|            InputBackend::FFIInput                   |
|            VfsBackend::GameAssetVFS                 |
+-----------------------------------------------------+
```

### 7.2 FFI Boundary

The FFI layer exports a minimal C-ABI surface. All internal Rust state is behind an opaque pointer. UE5 never sees Rust types.

| FFI Function | Signature | Description |
|-------------|-----------|-------------|
| `oasis_create` | `(width: u32, height: u32, skin_toml, layout_toml, features_toml: *const c_char) -> *mut OasisInstance` | Create an OS instance from skin TOML strings (default theme) |
| `oasis_create_full` | `(width, height, manifest_toml, layout_toml, features_toml, theme_toml, strings_toml) -> *mut OasisInstance` | Same, also taking the skin's theme and strings |
| `oasis_destroy` | `(handle: *mut OasisInstance)` | Tear down and free an OS instance |
| `oasis_tick` | `(handle: *mut OasisInstance, delta_seconds: f32)` | Advance OS state by one frame |
| `oasis_send_input` | `(handle: *mut OasisInstance, event: *const OasisInputEvent)` | Deliver an input event (incl. raw keys, `OASIS_EVENT_KEY`) |
| `oasis_get_buffer` | `(handle: *mut OasisInstance, out_width: *mut u32, out_height: *mut u32) -> *const u8` | Get pointer to RGBA framebuffer |
| `oasis_get_dirty` | `(handle: *mut OasisInstance) -> bool` | Check if framebuffer changed since last read |
| `oasis_send_command` | `(handle: *mut OasisInstance, cmd: *const c_char) -> *mut c_char` | Execute a terminal command, return output (free with `oasis_free_string`) |
| `oasis_free_string` | `(ptr: *mut c_char)` | Free a string returned by the library |
| `oasis_set_vfs_root` | `(handle: *mut OasisInstance, path: *const c_char)` | Reset the VFS (e.g., per-terminal game content) |
| `oasis_add_vfs_file` | `(handle: *mut OasisInstance, path: *const c_char, data: *const u8, data_len: u32)` | Add a file to the instance's VFS |
| `oasis_register_callback` | `(handle: *mut OasisInstance, event: u32, cb: OasisCallback)` | Register callback for OS events (app launch, file access, etc.) |
| `oasis_audio_*`, `oasis_set_audio_callback` | load / play / pause / resume / stop / set_volume / get_volume / is_playing | Audio playback bridged to the host |
| `oasis_video_*` | play / stop / next_frame / get_audio / is_playing | Video decode (only with the `video-decode` or `video-decode-ffmpeg` feature) |

The full C API reference, event codes and header are in [`ffi-integration.md`](ffi-integration.md).

### 7.3 Render Pipeline

1. `oasis_tick()` advances the OS state (cursor animation, blinking cursors, scrolling text, pending command output)
2. SDI scene graph draws all visible objects to the UE5RenderTarget backend's internal RGBA buffer
3. Backend sets dirty flag
4. UE5's `TickComponent()` checks `oasis_get_dirty()` -- if true, calls `oasis_get_buffer()` and `UpdateTextureRegions()` to push pixels to the `UTexture2D`
5. The `UTexture2D` is sampled by a material applied to the monitor mesh
6. Optional: the material applies CRT shader effects (scanlines, curvature, phosphor bloom) based on skin metadata -- the Rust side renders clean pixels, UE5 applies the aesthetic

This is effectively zero-copy when Rust and UE5 share the same memory page for the buffer. On platforms where that's not possible, it's a single memcpy per dirty frame.

### 7.4 Input Pipeline

1. Player aims at in-game monitor, presses interact key
2. UE5 enters "computer interaction mode": HUD suppressed, camera locks, input rerouted
3. Line trace from crosshair hits monitor mesh -> compute UV at hit point
4. UV coordinates mapped to virtual screen resolution (e.g., UV(0.5, 0.5) -> screen(240, 136) on a 480x272 virtual display)
5. Mouse movement -> `CursorMove` events; mouse click -> `PointerClick` events
6. Keyboard input captured -> `TextInput` events (for terminal skins) or `ButtonPress` events (for dashboard skins)
7. Player presses escape/interact key again -> exit interaction mode, restore normal camera and input

### 7.5 Gameplay Integration via Callbacks

The `oasis_register_callback` function lets UE5 react to OS-level events. This enables gameplay mechanics driven by in-game computer interaction:

| Callback Event | Trigger | Gameplay Example |
|---------------|---------|-----------------|
| `ON_FILE_ACCESS` | Player opens a file via `cat` or file browser | Unlock a lore entry, trigger a quest objective |
| `ON_COMMAND_EXEC` | Player executes a terminal command | Hacking puzzle: execute the right sequence of commands to bypass "security" |
| `ON_APP_LAUNCH` | Player launches an "app" from the dashboard | Open a game-specific interface, launch a mini-game |
| `ON_LOGIN` | Player enters the correct pre-shared key | Gain access to a restricted terminal with new commands |
| `ON_NETWORK_SEND` | Player sends a message via in-game email | Deliver message to another player's in-game computer, NPC responds |
| `ON_PLUGIN_LOAD` | Player installs a "plugin" found elsewhere in the game world | Unlock new terminal capabilities as a progression mechanic |

### 7.6 Multiple Instances

Each in-game computer is an independent OS instance with its own skin, VFS root, plugin set, and state. A single UE5 level might contain a dozen terminals, each running a different skin with different content. Instances share no state unless explicitly connected (e.g., an in-game "network" where sending a message on one terminal delivers it to another's VFS inbox).

---

## 8. PSP Platform Implementation

### 8.1 Memory and Resource Constraints

The PSP has 32MB of main RAM (64MB on PSP-2000+), a 333MHz MIPS R4000 CPU, and 2MB of dedicated VRAM. The original C codebase allocates a 20MB heap (`PSP_HEAP_SIZE_KB(1024 * 20)`) and runs 5-6 concurrent threads.

| Constraint | Impact on Rust Design | Mitigation |
|-----------|----------------------|------------|
| 32MB RAM | No std collections with unbounded growth; fixed-capacity containers preferred | Use heapless or tinyvec for bounded collections; arena allocators for scene graph |
| No MMU isolation | All threads share address space; corruption in one affects all | Rust ownership prevents data races; unsafe blocks minimized and audited |
| 333MHz CPU | No room for abstraction overhead; zero-cost abstractions critical | Monomorphization over dynamic dispatch; inline hot paths; profile with PPSSPP |
| 2MB VRAM | Textures must be carefully managed; power-of-two dimensions required | Texture atlas for UI elements; lazy loading; VRAM allocation tracker |
| No ASLR/stack canaries | Network-facing code is high-value exploit target | Rust memory safety eliminates buffer overflows at compile time |

### 8.2 Kernel Mode and Privileges

The original C codebase runs in kernel mode (`PSP_MODULE_INFO` flag `0x1000`), granting access to all hardware registers, the ability to load arbitrary PRX modules, and direct Memory Stick I/O.

**rust-psp SDK:** OASIS_OS depends on a standalone fork of the rust-psp SDK at [github.com/AndrewAltimit/rust-psp](https://github.com/AndrewAltimit/rust-psp), referenced as a git dependency. The upstream project (`github.com/overdrivenpotato/rust-psp`, MIT license) is maintained at a low cadence (~3-4 commits/year) and lacks kernel mode support (issue #48, open since June 2020). The fork extends the SDK with kernel mode module support (`PSP_MODULE_INFO` flag `0x1000`), std support via sysroot overlay (`main` branch), edition 2024 modernization, safety fixes, and additional syscall bindings -- without waiting on upstream. The fork tracks upstream for bug fixes but diverges for kernel-mode and feature additions.

OASIS_OS initially targets user mode, which is sufficient for rendering, input, networking, and file I/O on modern custom firmware. Kernel-mode features (Media Engine coprocessor, raw hardware register access) are the primary motivation for extending the SDK fork. Until kernel mode is fully implemented, these features are accessed through pre-compiled PRX modules loaded at runtime. The `#![feature(restricted_std)]` + `#![no_main]` entry point uses `psp::module!()` macro with `psp_main()`. Std support is enabled by `RUST_PSP_BUILD_STD=1` which triggers cargo-psp to build a custom sysroot with PSP-specific PAL implementations for threads, filesystem, random, and networking.

### 8.3 Assembly Preservation [SUPERSEDED]

> **Status:** Not needed. Media Engine boot is handled by the `oasis-me-boot` kernel PRX (a clean Rust replacement for cooleyesBridge that resolves `sceMeBootStart660`), and audio uses firmware `sceMp3*` / `sceAudiocodec` instead of the original PRX stubs. The original plan is kept below for history.

Three assembly files from the original C codebase must be preserved rather than ported:

- **me.S:** Media Engine coprocessor initialization. Directly manipulates CP0 registers, performs cache invalidation, and boots the ME. ~60 lines of hand-tuned MIPS assembly.
- **audiolib.S:** PRX module stub definitions for the audio library (MP3_Init, MP3_Load, MP3_Play, etc.).
- **pspstub.s:** Macro definitions for STUB_START/STUB_FUNC/STUB_END used by the audio stubs.

Included in the Rust build via `global_asm!` or linked as pre-assembled object files in `build.rs`.

### 8.4 Firmware Modernization: 1.50 to 6.60/6.61

The original C codebase was architecturally coupled to PSP firmware 1.50, the only firmware that ran unsigned homebrew without custom firmware. OASIS_OS targets firmware 6.60/6.61 with modern custom firmware (ARK-4, PRO CFW, or ME CFW, made persistent via Infinity 2). This eliminates every firmware-specific hack in the original codebase.

#### 8.4.1 Eliminated Legacy Mechanisms

| Original Mechanism | Why It Existed (FW 1.50) | OASIS_OS Disposition (FW 6.60/6.61) |
|------------------------|-------------------------|-------------------------------------|
| KXploit directory scanning (`GAMEFOLDER` + `GAMEFOLDER%`) | Firmware 1.50 required a paired directory exploit to trick the kernel into running unsigned code. The '%' suffix exploited a directory validation bug. | Eliminated entirely. Modern CFW runs unsigned EBOOTs directly from `ms0:/PSP/GAME/`. Dashboard scanner reduced to simple recursive directory traversal. |
| KXploit title stripping ("KXPloit Boot by PSP-DEV Team") | KXploit injected a watermark string into PBP titles that cluttered the dashboard. | Eliminated. No watermarks exist in modern homebrew EBOOTs. |
| Dual default icons (`icon_default_15` vs `icon_default_10`) | Distinguished between homebrew targeting FW 1.50 vs FW 1.00 based on directory structure. | Eliminated. Single default icon for all homebrew. Icons extracted from PBP ICON0 section when available. |
| `__attribute__((constructor))` kernel escalation | `pspKernelSetKernelPC()` and `pspSdkInstallNoPlainModuleCheckPatch()` patched the 1.50 kernel at load time to gain kernel-mode access and disable module signature checks. | Replaced by CFW-provided kernel access. rust-psp targets the CFW's documented entry points. Privilege escalation is the CFW's responsibility. |
| `pspSdkInstallNoDeviceCheckPatch()` | Disabled firmware 1.50's device check that prevented certain I/O operations on unsigned code. | Eliminated. Modern CFW disables all such checks globally. |
| DevHook integration (`devhook.c`, `ms0:/dh/`) | Firmware 1.50 lacked APIs required by newer commercial games. DevHook emulated a higher firmware's flash filesystem from the Memory Stick. | Eliminated entirely. Firmware 6.60/6.61 natively includes all game APIs. ISO loading is a built-in CFW feature via Inferno/NP9660/Prometheus drivers. |
| DevHook config structure (DH04 magic, flash path, reboot path) | Stored emulated firmware version, CPU/bus speeds, ISO path, and flash redirect path for the DevHook kernel module. | Eliminated. No equivalent needed. ISO path stored in standard OASIS_OS config. |
| `sceKernelLoadModuleBufferUsbWlan()` | Firmware 1.50-specific kernel function for loading modules from memory buffers, used to bypass signature checks on PRX modules. | Replaced by standard `sceKernelLoadModule()` which loads unsigned PRX files natively on modern CFW. |
| Commented-out error handling in network init | Firmware 1.50's WiFi stack was unreliable (limited WPA support, buggy AP connection). | Proper error handling restored. Firmware 6.60/6.61 has a mature, stable WiFi stack with full WPA2 support. |

#### 8.4.2 New Capabilities Enabled by Modern Firmware

| Capability | Details |
|-----------|---------|
| 64MB RAM (PSP-2000+) | Firmware 1.50 only ran on the PSP-1000 (32MB). Modern CFW on PSP-2000/3000/Go exposes the full 64MB. ARK-4 provides an "Auto" mode that enables extra RAM for homebrew that requests it. OASIS_OS detects the model at runtime via `sceKernelDevkitVersion()` and adjusts heap allocation, texture cache size, and plugin memory budget accordingly. |
| Stable kernel API surface | Modern CFW provides a consistent, well-documented kernel API across all PSP models. The fragile 1.50-specific patches are replaced by a stable interface maintained by the CFW community. |
| Built-in ISO/CSO driver | CFW's Inferno driver handles ISO mounting transparently. The dashboard can launch ISOs via standard `sceKernelLoadExec` -- no kernel patching, no DevHook, no flash emulation. |
| Plugin (PRX) hot-loading | `sceKernelLoadModule` works reliably for unsigned PRX on modern CFW. The plugin system can dynamically load/unload modules without signature bypass hacks. |
| WPA2 WiFi | Full WPA2-PSK support with stable AP connection handling. The remote terminal module can rely on infrastructure mode networking being functional and robust. This is what makes PSP-based remote agent control viable (Section 10). |
| PSP Go support | The original codebase never supported the PSP Go. Modern CFW abstracts storage access, allowing OASIS_OS to run on all PSP models including the Go (`ef0:/` instead of `ms0:/`). |

#### 8.4.3 Target Custom Firmware Stack

- **Base firmware:** 6.61 (Sony's final official firmware, with all security and stability patches)
- **Custom firmware:** ARK-4 (actively maintained, supports all PSP models including Street/E-1000, provides Inferno ISO driver, extra RAM management, and comprehensive plugin support)
- **Persistence:** Infinity 2 for semi-permanent CFW on PSP-2000 (TA-088v3+), PSP-3000, and PSP Go; cIPL flasher for permanent CFW on PSP-1000 and early PSP-2000 (TA-088v2 and older)
- **Storage paths:** `ms0:/PSP/GAME/OASISOS/` for Memory Stick models; `ef0:/PSP/GAME/OASISOS/` for PSP Go. Runtime detection via `sceIoDevctl()` determines available storage.

OASIS_OS does not require or depend on any specific custom firmware choice. Any CFW that provides kernel-mode access and unsigned code execution on firmware 6.60 or 6.61 is sufficient.

#### 8.4.4 Simplified Application Discovery

The firmware modernization dramatically simplifies the dashboard's application discovery logic. The original `scan_for_exec` was 128 lines of C with complex KXploit handling. The OASIS_OS equivalent:

- Recursively scan `ms0:/PSP/GAME/` (or `ef0:/` on PSP Go) for directories containing EBOOT.PBP
- Parse each PBP header to extract title (SFO section) and icon (ICON0 section)
- Scan configured ISO directories for .ISO/.CSO files (CFW handles mounting)
- Skip the OASIS_OS directory itself
- Register discovered applications with the dashboard grid

No KXploit handling, no '%' directory pairing, no title watermark stripping, no firmware-version-specific icon selection. The code reduction is approximately 60% in the discovery module alone.

#### 8.4.5 Overlay Plugin PRX Architecture

OASIS uses a two-binary design on PSP, inspired by IRShell and similar PSP shells that shipped companion PRX modules for in-game functionality:

| Binary | Crate | Format | Mode | Purpose |
|--------|-------|--------|------|---------|
| EBOOT.PBP | `oasis-backend-psp` | Standalone executable | User | Full shell (dashboard, terminal, browser, apps) |
| PRX | `oasis-plugin-psp` | Relocatable module | Kernel | Lightweight overlay + background music |

The PRX is loaded by custom firmware at boot time via a `PLUGINS.TXT` entry and stays resident in kernel memory when games launch. The EBOOT is replaced by the game, but the PRX persists.

**Hook mechanism:** The PRX patches `sceDisplaySetFrameBuf` via `sctrlHENPatchSyscall` (a CFW-provided API for syscall table manipulation). After the game renders each frame, the hook draws overlay UI elements directly into the framebuffer using alpha-blended pixel writes.

**Memory budget:** <64KB total (code + static data). No heap allocator for the core overlay path. The plugin uses static buffers for MP3 decode, PCM output, playlist storage, and overlay rendering. Atomics provide thread-safe state sharing between the display hook (game thread) and audio thread.

**Background audio:** A dedicated kernel thread streams MP3 files from `ms0:/MUSIC/` through the PSP's hardware MP3 decoder (`sceMp3*`) to a reserved audio channel. File data is streamed in chunks rather than loaded entirely into memory.

**Integration:** The EBOOT includes terminal commands (`plugin install`, `plugin remove`, `plugin status`) to manage the PRX installation, PLUGINS.TXT registration, and configuration file (`ms0:/seplugins/oasis.ini`).

---

## 9. Linux / Raspberry Pi Platform

### 9.1 Behavioral Differences

| Behavior | PSP | Raspberry Pi / Linux |
|----------|-----|---------------------|
| App launching | `sceKernelLoadExec` (replaces process) | fork/exec (returns to shell) |
| Multitasking | Single process, multiple threads | Full process isolation |
| Plugin loading | PRX modules via `sceKernelLoadModule` | Dynamic .so via dlopen/libloading |
| File system root | `ms0:/` (Memory Stick) | Configurable path (e.g., `/home/pi/apps/`) |
| Power management | `scePowerGetBatteryLifePercent` | `/sys/class/power_supply/` sysfs reads |
| Display | 480x272 fixed, 32-bit color via GU | Configurable resolution via SDL3 or fbdev |
| Networking | sceNetInet (WiFi only, infrastructure mode) | std::net (Ethernet or WiFi, full stack) |
| Boot flow | Custom firmware loads EBOOT.PBP | systemd service, auto-login to kiosk mode |

### 9.2 Raspberry Pi Deployment

On the Raspberry Pi, OASIS_OS operates as a kiosk-mode application booting directly into the shell interface. Recommended deployment uses Raspberry Pi OS Lite with auto-login to launch the OASIS_OS binary on TTY1.

- **Boot-to-shell time:** Approximately 3-5 seconds on a Pi 5, from power-on to rendered dashboard.
- **Display options:** Official Raspberry Pi touchscreen (800x480), HDMI at configurable resolution, or headless with remote terminal only.
- **Input options:** USB keyboard/mouse, USB gamepad, GPIO-wired buttons, touchscreen (via SDL3 touch events), or remote terminal.
- **Systemd integration:** A `oasis-os.service` unit file manages lifecycle, restart-on-crash, and dependency on `network-online.target`.
- **Coexistence with tamper services:** On the briefcase Pi, `oasis-os.service` runs alongside `tamper-sensor.service` and `tamper-gate.service`. OASIS_OS is the user-facing interface; the tamper services are the physical security layer. They share the Pi but do not interact directly -- the tamper system operates at the systemd level independent of whatever user-space application is running.

---

## 10. PSP Remote Agent Control

> **Status:** Partially implemented. The `listen`, `remote` and `hosts` commands exist (`oasis-terminal/src/remote_commands.rs`), backed by `oasis-net`'s `RemoteListener`, outbound `RemoteClient` (non-blocking, PSK authentication) and saved-hosts parser (`hosts.rs`); the desktop and WASM hosts wire `remote` sessions into the terminal. Not yet implemented: VT100 emulation (terminal output only understands SGR foreground colors, `oasis-core/src/ansi.rs`), the PSP backend's `remote` session wiring, and the PSP command palette / d-pad shortcuts described in §10.5 [PLANNED].

### 10.1 Concept

A PSP running OASIS_OS on infrastructure WiFi can establish outbound TCP connections to machines running AI agent infrastructure. The remote terminal works bidirectionally -- the PSP is both a server (accepting inbound sessions for its own OS) and a client (initiating outbound sessions to remote hosts). This makes a PSP with modern custom firmware (and its reliable WPA2 WiFi stack) a viable portable controller for the agent ecosystem.

This is not a theoretical capability. PPSSPP runs locally on the development machine and version 1.19+ maps `sceNetInet` to real host sockets, meaning the entire remote session stack can be developed and tested in the emulator against real network services before deploying to hardware.

### 10.2 Architecture

```
+----------------------------+          +----------------------------+
|  PSP (OASIS_OS)             |  WiFi    |  Agent Host Machine        |
|                            | -------> |                            |
|  remote dev-server         |  TCP     |  SSH / remote terminal port |
|    +-- outbound TCP        |          |    +-- Claude Code CLI     |
|    +-- SGR colors          |          |    +-- CI tooling          |
|    +-- command passthrough  |          |    +-- board-manager       |
|                            |          |    +-- MCP servers         |
+----------------------------+          +----------------------------+
```

The `remote` command in the OASIS_OS interpreter opens an outbound TCP session to a saved host. The session runs inside the OASIS_OS terminal UI -- keystrokes from the PSP's on-screen keyboard or USB keyboard (on PSP-2000+) are sent over the wire, and received text is rendered in the terminal view. SGR foreground colors are rendered today; a VT100 parser for cursor positioning and clearing is planned.

### 10.3 Use Cases

| Scenario | How It Works |
|----------|-------------|
| Check agent status from a PSP | `remote briefcase` -> `agent status` -- see which agents are online |
| Claim board work | `remote briefcase` -> `board query` -> `board claim 142` -- claim an issue for implementation |
| Trigger CI | `remote dev-server` -> run the CI entry point -- kick off CI from anywhere on the network |
| Monitor PR feedback | `remote dev-server` -> `pr-monitor 48` -- watch for review comments |
| Emergency wipe trigger | `remote briefcase` -> `tamper arm` -- re-arm the briefcase tamper system remotely |

### 10.4 Saved Hosts Configuration

Hosts are read from `/etc/hosts.toml` in the VFS (`oasis_net::parse_hosts`; `port` defaults to 9000, `protocol` to `oasis-terminal`, and an optional `psk` enables authentication):

```toml
# /etc/hosts.toml (VFS)

[[host]]
name = "briefcase"
address = "192.168.0.50"
port = 9000
protocol = "oasis-terminal"

[[host]]
name = "dev-server"
address = "192.168.0.100"
port = 22
protocol = "raw-tcp"

[[host]]
name = "gpu-box"
address = "192.168.0.222"
port = 9000
protocol = "oasis-terminal"
```

### 10.5 PSP Input Considerations [PLANNED]

The PSP's input limitations shape the remote terminal experience:

| Input Method | Availability | Speed | Use Case |
|-------------|-------------|-------|----------|
| On-screen keyboard (sceUtilityOsk) | All PSP models | Slow (d-pad character selection) | Short commands, passwords |
| USB keyboard | PSP-2000+ only | Full typing speed | Extended terminal sessions |
| D-pad shortcuts | All models | Instant | Mapped to common commands (L+Up = `agent status`, R+Triangle = `board query`) |
| Analog stick cursor | All models | Moderate | Scrolling terminal output, selecting from menus |

PSP adapts to these limitations by offering a command palette accessible via the Triangle button -- a scrollable list of frequently used agent commands that can be executed with a single button press rather than typed out character by character.

---

## 11. Virtual File System

### 11.1 VFS Architecture

The virtual file system is the abstraction that makes the same command interpreter work across all platforms. On PSP, `ls` lists Memory Stick contents. On Pi, it lists real Linux directories. In UE5, it lists game-authored content. The VFS trait provides a uniform interface over fundamentally different storage backends.

| VFS Operation | Trait Method | Description |
|--------------|-------------|-------------|
| List directory | `readdir(path) -> Vec<Entry>` | Return entries at path |
| Read file | `read(path) -> Vec<u8>` | Return file contents |
| Write file | `write(path, data)` | Write data to path |
| File info | `stat(path) -> Metadata` | Size, modified time, type |
| Create directory | `mkdir(path)` | Create directory |
| Delete | `remove(path)` | Delete file or directory |
| Rename | `rename(from, to)` | Rename/move file or directory |
| Exists | `exists(path) -> bool` | Check existence |

### 11.2 VFS Backends

| Backend | Backing Store | Use Case |
|---------|--------------|----------|
| `RealVfs` | Native filesystem (ms0:/, Linux paths) | PSP, Pi, desktop |
| `GameAssetVfs` | UE5 data assets, with writes going to an in-memory overlay | In-game computers in UE5 projects |
| `MemoryVfs` | In-memory tree, no persistence | Unit tests, ephemeral terminals, WASM |
| OverlayVFS [PLANNED] | Layered: read-only base (skin defaults) + writable layer (user changes) | All platforms -- skin-provided files with user modifications |

### 11.3 Game Asset VFS

The GameAssetVFS is the most novel backend. It presents game-authored content as a filesystem that players navigate with standard commands. Each in-game terminal has its own VFS root configured by level designers.

Examples of in-game file content:

- `/home/user/inbox/message_from_rival.txt` -- lore-delivering text file, triggers `ON_FILE_ACCESS` callback when read
- `/var/log/system.log` -- procedurally generated log file with clues for a puzzle
- `/opt/trading/deck_analyzer` -- "executable" that, when launched, opens a game-specific UI
- `/etc/security/access.conf` -- file the player must edit (via a `nano`-like command or `echo >>`) to solve a hacking puzzle
- `/tmp/upload/card_data.json` -- file deposited by another player via the in-game network

The VFS root, pre-populated content, and write permissions are all defined per-terminal in the skin's `filesystem.toml`.

---

## 12. Development Workflow

### 12.1 Three-Tier Testing Strategy

| Tier | Environment | Tests | Cycle Time |
|------|------------|-------|------------|
| 1 -- Desktop | Native SDL3 build on dev machine | UI layout, theming, scene graph, command interpreter, plugin loading, VFS, skins | < 1 second (hot rebuild) |
| 2 -- PPSSPP (container) | PSP build running in the PPSSPP container (headless in CI) | GU rendering, PSP input, networking (infra mode), memory constraints, thread behavior, scripted scenarios via psp-autorun | ~5 seconds (cross-compile + launch) |
| 3 -- Hardware/UE5 | Real PSP + Raspberry Pi + UE5 editor | WiFi, USB, Media Engine, GPIO, boot-to-shell, in-game render target, interaction flow | ~30 seconds (deploy + reboot/PIE) |

### 12.2 Containerized PPSSPP with MCP Integration [PLANNED]

> **Status:** This section describes planned infrastructure that has not yet been built. Currently, PPSSPP is used for testing via the project's existing `ppsspp` Docker Compose service (a stock PPSSPP build with X11 forwarding). The MCP-patched container, patch files, and MCP schema directory described below are future work.

PPSSPP runs inside a Docker container built from source with a set of patches that embed an MCP server directly into the emulator. This gives AI agents deep introspection into the running PSP environment -- memory state, GPU pipeline, thread scheduling, network sockets -- through the same MCP tool interface used by every other tool in the repository. The container follows the project's container-first philosophy: no local PPSSPP installation required, reproducible builds, and CI-ready headless mode.

#### 12.2.1 Container Architecture

```
+----------------------------------------------------------+
|  Docker: ppsspp-mcp                                      |
|                                                          |
|  PPSSPP (patched)                                        |
|    +-- PSP emulation core                                |
|    +-- MCP server (STDIO or TCP :8808)                   |
|    |     +-- psp.memory.read / write / search            |
|    |     +-- psp.gpu.state / vram_dump / texture_list    |
|    |     +-- psp.debug.breakpoint / step / registers     |
|    |     +-- psp.debug.threads / stack_trace             |
|    |     +-- psp.net.sockets / capture / inject_latency  |
|    |     +-- psp.emu.screenshot / save_state / load_state|
|    |     +-- psp.emu.frame_advance / set_speed           |
|    +-- Headless renderer (no X11 required for CI)        |
|    +-- Infrastructure networking (sceNetInet -> host)    |
|                                                          |
|  Mounts:                                                 |
|    /eboot  <- target/mipsel-sony-psp/                    |
|    /states <- ppsspp/states/ (save states, snapshots)    |
+----------------------------------------------------------+
        |  MCP (TCP :8808)        |  PSP net (TCP :9000)
        v                         v
  AI agents (Claude,        telnet / OASIS_OS
  CI tooling, etc.)         remote terminal
```

#### 12.2.2 MCP Tool Surface

The patches add an MCP server to PPSSPP that exposes the emulator's internal state as tools. The tool schemas would live in `docker/ppsspp-patches/mcp-schema/` and follow the same conventions as the project's other MCP servers.

| Tool | Description | Agent Use Case |
|------|-------------|---------------|
| `psp.memory.read` | Read bytes at a PSP address (user or kernel space) | Inspect SDI object registry in memory, verify scene graph state |
| `psp.memory.write` | Write bytes at a PSP address | Hot-patch values during debugging, inject test data |
| `psp.memory.search` | Scan memory for byte pattern or string | Find leaked allocations, locate specific SDI objects by name string |
| `psp.memory.heap_info` | Return heap allocation map (block list, free list, fragmentation) | Diagnose memory budget issues on 32MB PSP |
| `psp.gpu.state` | Current GU state: display list position, matrix stack, texture bindings, blend mode | Debug rendering issues -- agent can compare expected vs actual GU state |
| `psp.gpu.vram_dump` | Dump VRAM contents as PNG | Visual regression testing -- agent compares VRAM snapshot against reference |
| `psp.gpu.texture_list` | List all loaded textures with dimensions, format, VRAM address | Track VRAM budget, find texture leaks |
| `psp.gpu.draw_call_trace` | Log the next N draw calls with parameters | Profile rendering pipeline, identify redundant draws |
| `psp.debug.breakpoint` | Set/clear/list breakpoints at MIPS addresses or symbol names | Agent-driven debugging -- set breakpoint on `sdi_draw`, inspect state when hit |
| `psp.debug.step` | Single-step or continue execution | Step through problematic code paths |
| `psp.debug.registers` | Read MIPS register file (GPR, FPR, HI/LO, PC, CP0) | Low-level debugging of crashes or hangs |
| `psp.debug.threads` | List PSP threads with state, priority, stack pointer, wait reason | Diagnose deadlocks or scheduling issues in the 5-6 thread OASIS_OS runtime |
| `psp.debug.stack_trace` | Walk stack frames for a given thread | Identify crash location, trace call chain |
| `psp.debug.symbols` | Query symbol table (from ELF before PBP packing) | Resolve addresses to function names for readable debugging |
| `psp.net.sockets` | List open sceNetInet sockets with state, addresses, buffer contents | Debug remote terminal connection issues |
| `psp.net.capture` | Capture next N packets on a socket | Inspect the OASIS_OS terminal protocol wire format |
| `psp.net.inject_latency` | Add artificial latency to socket operations | Test remote terminal behavior under poor WiFi conditions |
| `psp.emu.screenshot` | Capture current frame as PNG | Visual verification in CI, skin rendering checks |
| `psp.emu.save_state` | Save emulator state to file | Reproducible debugging -- save state at crash, reload to investigate |
| `psp.emu.load_state` | Restore emulator state from file | Reproduce exact conditions from a bug report |
| `psp.emu.frame_advance` | Advance exactly N frames | Frame-precise testing of animations and transitions |
| `psp.emu.set_speed` | Set emulation speed (0.1x to unlimited) | Fast-forward through boot sequence in CI; slow down for debugging |

#### 12.2.3 Docker Compose Service

```yaml
# In project root docker-compose.yml
ppsspp-mcp:
  build:
    context: docker
    dockerfile: ppsspp-mcp.Dockerfile
  ports:
    - "8808:8808"   # MCP server (TCP transport)
    - "9000:9000"   # OASIS_OS remote terminal (forwarded from emulated PSP)
  volumes:
    - ./crates/oasis-backend-psp/target/mipsel-sony-psp/release:/eboot:ro
    - ./ppsspp-states:/states
  environment:
    - PPSSPP_HEADLESS=1           # No display required (CI mode)
    - PPSSPP_MCP_TRANSPORT=tcp    # tcp or stdio
    - PPSSPP_MCP_PORT=8808
    - PPSSPP_NET_INFRA=1          # Enable infrastructure networking
    - PPSSPP_EBOOT=/eboot/EBOOT.PBP
```

#### 12.2.4 Dockerfile Build Strategy

The Dockerfile clones PPSSPP from source at a pinned commit, applies the patch series in order, and builds. PPSSPP is GPL-2.0+, so the patches (which are derivative works distributed as part of the container build, not as a modified PPSSPP binary in the repository) comply with the license -- the patches themselves and the Dockerfile are the source.

```dockerfile
FROM ubuntu:24.04 AS builder
# Install PPSSPP build deps (CMake, SDL3-dev, etc.)
RUN apt-get update && apt-get install -y ...

# Clone at pinned commit for reproducibility
ARG PPSSPP_COMMIT=<pinned-hash>
RUN git clone --recursive https://github.com/hrydgard/ppsspp.git /ppsspp \
    && cd /ppsspp && git checkout $PPSSPP_COMMIT

# Apply MCP patches
COPY patches/ /patches/
RUN cd /ppsspp && for p in /patches/*.patch; do git apply "$p"; done

# Build (headless + SDL for optional display forwarding)
RUN cd /ppsspp && cmake -B build -DHEADLESS=ON -DUSING_X11_VULKAN=OFF ... \
    && cmake --build build -j$(nproc)

FROM ubuntu:24.04
COPY --from=builder /ppsspp/build/PPSSPPSDL /usr/local/bin/ppsspp
COPY --from=builder /ppsspp/build/assets /usr/local/share/ppsspp/assets
ENTRYPOINT ["ppsspp"]
```

#### 12.2.5 Patch Architecture

The patches are minimal, focused modifications to PPSSPP's C++ source. Each patch is self-contained and applies cleanly to the pinned commit.

| Patch | Files Modified | Approach |
|-------|---------------|----------|
| `0001-mcp-server.patch` | New `Core/MCP/` directory, hooks into `Core/System.cpp` startup | Adds a lightweight MCP server (JSON-RPC over TCP or STDIO). The server runs on a dedicated thread, accepts tool calls, and dispatches to handler functions. Uses PPSSPP's existing `Core/Debugger/` infrastructure for memory/CPU access. |
| `0002-memory-tools.patch` | `Core/MCP/MemoryTools.cpp` | Wraps `Memory::Read_U8/U16/U32`, `Memory::Write_*`, and `Memory::GetPointer` as MCP tools. Heap info reads the PSP kernel's internal allocation structures. |
| `0003-gpu-tools.patch` | `Core/MCP/GPUTools.cpp`, minor hooks in `GPU/GPUCommonHW.cpp` | Reads GU state from the GPU emulation layer. VRAM dump uses `GPU::GetFramebuffer()`. Draw call trace hooks into the display list interpreter. |
| `0004-debug-tools.patch` | `Core/MCP/DebugTools.cpp`, hooks in `Core/MIPS/MIPSDebugInterface.cpp` | Wraps PPSSPP's existing breakpoint, stepping, and register read APIs. Symbol lookup uses the ELF symbol table loaded at boot. Thread list reads from the emulated kernel's thread manager. |
| `0005-network-tools.patch` | `Core/MCP/NetTools.cpp`, hooks in `Core/HLE/sceNetInet.cpp` | Intercepts socket operations at the HLE layer. Packet capture records data flowing through `sceNetInetSend`/`Recv`. Latency injection adds `usleep()` before forwarding to host sockets. |
| `0006-headless-mode.patch` | `headless/Headless.cpp` | Extends PPSSPP's existing headless mode to support the MCP server and long-running execution (original headless mode is designed for screenshot comparison tests that exit immediately). |

#### 12.2.6 Agent Debugging Workflow

With the MCP-patched PPSSPP container running, AI agents can debug OASIS_OS on the PSP target the same way they debug desktop code -- but with hardware-accurate emulation:

```bash
# 1. Build OASIS_OS for PSP
cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo psp --release

# 2. Launch PPSSPP container (headless, MCP on :8808)
docker compose up -d ppsspp-mcp

# 3. Agent connects via MCP and inspects running state
# (via mcp-code-quality or direct TCP to 8808)
```

**Example: Agent diagnosing a rendering bug**

```
Agent -> psp.emu.screenshot
  <- PNG showing garbled dashboard icons

Agent -> psp.gpu.texture_list
  <- 12 textures loaded, texture #7 "icon_browser" has wrong dimensions (128x128, expected 64x64)

Agent -> psp.memory.search { pattern: "icon_browser" }
  <- Found at 0x08A03400 (SDI registry entry)

Agent -> psp.memory.read { address: 0x08A03400, length: 64 }
  <- SDI object struct: width=128, height=128, tex_ptr=0x04110000

Agent -> psp.gpu.state
  <- Current scissor rect: (0,0,480,272), blend mode: SRC_ALPHA

# Agent now has enough information to identify the bug:
# icon_browser loaded at 2x resolution, SDI blit stretches it
# Fix: check texture loading code for dimension handling
```

**Example: Agent diagnosing a deadlock**

```
Agent -> psp.debug.threads
  <- Thread 0 "main": WAIT (sema 0x00000003)
     Thread 1 "render": WAIT (sema 0x00000002)
     Thread 2 "network": RUNNING
     Thread 3 "terminal": WAIT (sema 0x00000003)

Agent -> psp.debug.stack_trace { thread: 0 }
  <- #0 sceKernelWaitSema at 0x08801234
     #1 sdi_draw at 0x08805678
     #2 main_loop at 0x08800ABC

# Agent identifies: main and terminal both waiting on sema 3,
# render waiting on sema 2 -- classic lock ordering inversion
```

#### 12.2.7 PPSSPP Networking (Passthrough)

PPSSPP 1.19+ maps `sceNetInet` socket calls to real host sockets. With the container's port forwarding, the OASIS_OS remote terminal running inside the emulated PSP is accessible from the host:

1. Cross-compile OASIS_OS for MIPS, producing EBOOT.PBP
2. Container launches PPSSPP with infrastructure networking enabled
3. OASIS_OS starts TCP listener on port 9000 (forwarded to host)
4. From another terminal: `telnet localhost 9000`
5. Full interactive command session with the running instance

This also works for outbound connections -- the `remote` command from the containerized PPSSPP instance connects to real services on the Docker network or host network, enabling full testing of the PSP agent control flow without physical hardware.

### 12.3 UE5 Development Workflow

1. Build OASIS_OS as a cdylib (`cargo build --release -p oasis-ffi`)
2. Copy `.dll`/`.so` to UE5 project's `Binaries/ThirdParty/` directory
3. UE5 Build.cs references the library; C++ actor component calls FFI functions
4. Place `AComputerTerminalActor` in level, assign skin and VFS root via editor properties
5. Press Play-in-Editor; interact with in-game terminal
6. Optional: `telnet localhost 9000` for direct terminal access while PIE is running

### 12.4 AI Agent Integration

All code in the repository is authored by AI agents under human direction (see [`AGENTS.md`](../AGENTS.md) for the agent roster and CI workflow):

| Agent | Task | Automation Level |
|-------|------|-----------------|
| Claude Code | Architecture design, complex refactoring, C-to-Rust translation of SDI patterns, PSP debugging | Primary -- deep codebase understanding |
| OpenCode / Crush | Repetitive code translation, test generation | High -- patterns are mechanical |
| Gemini CLI | Automated PR review of OASIS_OS changes | Automatic -- runs on every PR |
| CI (GitHub Actions + Docker) | fmt, clippy, tests, builds, screenshot regression, PSP EBOOT + PPSSPP | High -- deterministic pipeline |
| In-app MCP server (`oasis-mcp`) | Lets a local agent drive a running desktop instance (see [`mcp-server.md`](mcp-server.md)) | Optional, off by default |
| PPSSPP MCP container [PLANNED] | Live PSP debugging: memory inspection, GPU state, breakpoints, screenshots, network capture | Planned (§12.2) |

The original C source (`psixpsp.7z`, not checked in) served as the reference for AI-assisted translation. Agents can diff the original C against the Rust port to verify behavioral equivalence. Once built, the PPSSPP MCP container (Section 12.2) would let agents debug the PSP target with the same depth as desktop code. Today, PSP debugging uses the TCP devloop server and PPSSPP headless runs (see [`psp-autorun.md`](psp-autorun.md)).

---

## 13. Plugin System

OASIS_OS supports runtime-extensible functionality through a platform-appropriate plugin system. On PSP, plugins are PRX modules loaded via `sceKernelLoadModule`. On Linux, shared libraries (`.so`) loaded via `libloading`. In UE5, plugins can also be authored as UE5 Blueprint-callable FFI extensions.

### 13.1 Plugin Lifecycle

| Phase | Description | PSP | Linux | UE5 |
|-------|------------|-----|-------|-----|
| Discovery | Scan plugin directory | `ms0:/.../plugins/` for .prx | `/etc/oasis-os/plugins/` for .so | Game asset directory for plugin manifests |
| Load | Load binary, resolve symbols | `sceKernelLoadModule` | `libloading::Library::new` | `libloading` or static registration |
| Initialize | Call init with host API handle | `oasis_plugin_init()` | `oasis_plugin_init()` | `oasis_plugin_init()` |
| Register | Plugin registers commands, UI, handlers | Function pointer table | `Box<dyn Plugin>` | `Box<dyn Plugin>` |
| Update | Per-frame tick | Main loop call | Main loop call | Main loop call |
| Unload | Shutdown, free memory | `sceKernelUnloadModule` | Drop Library | Drop Library |

### 13.2 Host API Surface

Plugins interact with OASIS_OS through a stable, versioned API providing access to: SDI scene graph (create/modify UI elements), command registry (register new commands), VFS (read/write files), network sockets, configuration storage, and event bus (subscribe to OS events).

### 13.3 Event Bus IPC

The plugin event bus (`crates/oasis-core/src/plugin/event_bus.rs`) provides a publish/subscribe communication channel between plugins and core components. Plugins subscribe to topics and publish `PluginEvent` messages containing a topic string, source identifier, and data payload. This supplements VFS-based IPC with a more immediate, event-driven pattern suitable for real-time notifications. Events are string-based for cross-language compatibility.

### 13.4 Plugin Configuration

Plugin manifests support a `config` field containing typed key-value pairs via `PluginConfigValue`:

| Variant | Rust Type | TOML Example |
|---------|-----------|-------------|
| `Bool` | `bool` | `auto_start = true` |
| `Int` | `i64` | `refresh_interval = 30` |
| `Float` | `f64` | `opacity = 0.8` |
| `Str` | `String` | `theme = "dark"` |

Configuration values are parsed from the `[config]` section of `plugin.toml` and available to plugins at init time.

---

## 14. Audio System

The audio manager, playlists, radio sources and streaming back-pressure are described in [`audio-engine.md`](audio-engine.md).

### 14.1 WAV Decoder

The `oasis-audio` crate (`crates/oasis-audio/src/wav.rs`) includes a software WAV decoder supporting uncompressed PCM audio (format tag 1) in 8-bit and 16-bit, mono and stereo. The decoder validates RIFF/WAVE headers and returns interleaved i16 samples at the source sample rate. WAV support supplements the existing MP3 decode pipeline for short sound effects, UI feedback tones, and game audio assets.

---

## 15. UI Toolkit and Accessibility

### 15.1 Widgets

`oasis-ui` provides 29 types implementing the `Widget` trait (`measure` + `draw` against a theme-aware `DrawContext`) plus non-`Widget` helpers (ListView, MenuBar, NinePatch, IconAtlas, flex/grid layout, focus management, tweens). The catalogue with source paths is [`ui-widgets.md`](ui-widgets.md).

### 15.2 Accessibility

The `oasis-ui` crate (`crates/oasis-ui/src/accessibility.rs`) provides accessibility utilities:

- **`AccessibilityLabel`** -- Semantic label struct with `label`, `description`, and `role` fields for UI elements
- **`relative_luminance()`** -- Calculates relative luminance per WCAG 2.0 specification
- **`contrast_ratio()`** -- Computes contrast ratio between two colors
- **`meets_wcag_aa()`** / **`meets_wcag_aaa()`** -- Checks minimum contrast thresholds for WCAG AA (4.5:1) and AAA (7:1) compliance

The High Contrast skin leverages these utilities to ensure all text meets WCAG AAA contrast requirements.

---

## 16. Security Considerations

| Threat | Mitigation |
|--------|-----------|
| Buffer overflow in network input parsing | Rust ownership model eliminates buffer overflows at compile time; no unsafe in terminal module |
| Unauthorized remote access | Pre-shared key authentication; the remote listener binds loopback by default and refuses all-interfaces binds without a PSK; the MCP server is loopback-only with Origin/Host checks and an optional bearer token |
| Plugin loading malicious code | Plugins explicitly installed by user; no remote plugin installation; signed manifests optional |
| Memory corruption via unsafe blocks | Unsafe limited to platform FFI wrappers; all unsafe blocks documented with safety invariants |
| Denial of service via resource exhaustion | Connection limit on terminal listener; per-command timeout; bounded input buffer size |
| Man-in-the-middle on terminal connection | Optional TLS (rustls on desktop; `embedded-tls` TLS 1.3 on PSP) |
| Hostile web content | Browser `fetch` blocks loopback/private targets from less-private origins and enforces CORS; per-origin storage quotas; decompressed bodies capped at 8 MB; JS execution watchdog (see [`security.md`](security.md)) |
| UE5 FFI boundary safety | All pointers validated on Rust side; null checks on every FFI entry point; opaque handles prevent UE5 from accessing Rust internals |
| Game save tampering via VFS | Write permissions per-path in VFS config; integrity checks on game-critical files; read-only base layer in the planned OverlayVFS |
| Briefcase physical compromise | Handled by the separate tamper-briefcase services -- dual-sensor detection, LUKS2 encryption, cryptographic wipe. OASIS_OS does not implement physical security; it delegates to the tamper services. |
| PSP remote session interception | Pre-shared key per host; connection limited to local network (no internet-facing listener). Defense in depth: sensitive operations on the remote host require their own authentication. |

---

## 17. Build System and CI/CD

### 17.1 Build Targets

| Target Triple | Toolchain | Output | Deploy Target |
|--------------|-----------|--------|--------------|
| mipsel-sony-psp (custom) | rust-psp + psp-gcc | EBOOT.PBP | PPSSPP / Memory Stick |
| aarch64-unknown-linux-gnu | cross (Docker) or native Pi | ELF binary | Raspberry Pi 5 (briefcase) via SSH/SCP |
| x86_64-unknown-linux-gnu | Native Rust toolchain | ELF binary / .so | Desktop dev / UE5 Linux |
| x86_64-pc-windows-msvc | Native Rust toolchain | .dll / .lib | UE5 Windows |

### 17.2 CI Pipeline Integration

OASIS_OS uses Docker-based CI execution (container-first philosophy). All CI stages run automatically on push to `main` and on pull requests via GitHub Actions:

```bash
# Docker-based execution
docker compose --profile ci run --rm rust-ci cargo fmt --all -- --check
docker compose --profile ci run --rm rust-ci cargo clippy --workspace -- -D warnings
docker compose --profile ci run --rm rust-ci cargo test --workspace
docker compose --profile ci run --rm rust-ci cargo build --workspace --release
docker compose --profile ci run --rm rust-ci cargo deny check
```

Full order: format check -> clippy -> nightly clippy (advisory) -> doc build -> markdown link check -> test -> release build -> screenshot regression -> cargo-deny -> benchmarks -> PSP EBOOT build -> PPSSPP headless test -> code coverage -> GitHub Pages deploy (WASM). Separate workflows run memory analysis (ASAN, Valgrind massif), nightly streaming tests and fuzzing. PR validation adds Gemini AI review and automated fix agents (Codex review is disabled).

**Planned CI stages:**

| Stage | Tool | Purpose |
|-------|------|---------|
| PSP integration (PPSSPP MCP) [PLANNED] | docker compose up ppsspp-mcp + MCP test sequence | Boot EBOOT in container, inspect state over MCP |

### 17.3 Context Protection

Verbose CI output is redirected to prevent context window pollution:

```bash
docker compose --profile ci run --rm rust-ci cargo test --workspace > /tmp/ci-output.log 2>&1 \
    && echo "CI passed" \
    || (echo "CI failed - check /tmp/ci-output.log"; exit 1)
```

---

## 18. Migration Strategy from Original C Codebase

The original C source (`psixpsp.7z`, not checked in) contains ~15,000 lines of C. The port follows a phased approach. Each phase produces a working, testable binary. The framework refactoring (core/backend/skin separation) happens in Phase 1-2, with original codebase features migrating in Phase 3-4 as the Classic skin.

| Phase | Deliverable | Source | Status |
|-------|-----------|--------|--------|
| 1 -- Framework scaffold | SDI core + backend trait + blank screen on SDL3 + UE5 render target stub | `sdi/sdi.c`, `sdi/backends/psp/gu.c` (architecture only) | **Complete** |
| 2 -- SDI + VFS + Commands | Full scene graph, VFS trait + RealFS + MemoryVFS, command interpreter with basic commands | `sdi/sdi.c`, `sdi/png.c`, `font.c`, new VFS code | **Complete** |
| 3 -- Classic skin | Icon grid dashboard, PBP scanning, app discovery, cursor navigation | `dashboard.c`, `pbp.c`, `image.c`, skin config | **Complete** |
| 4 -- PSP subsystems | Input, power, time, USB, file browser, on-screen keyboard, GU backend | `input.c`, `power.c`, `time.c`, `usb.c`, `file.c`, `osk.c` | **Complete** |
| 5 -- Remote terminal | TCP listener + outbound client, authentication, full command suite, remote access on all platforms | `net.c` (partial), new code | **Complete** |
| 6 -- UE5 integration | FFI boundary, UE5 render target backend, interaction input, GameAssetVFS | All new code | **Complete** |
| 7 -- Window manager | WM core: window lifecycle, grouping, drag/resize/focus, hit testing, clipping, window types | All new code | **Complete** |
| 8 -- Skin system | Skin loading, hot-swap (17 skins implemented across TOML + built-in) | All new code + skin configs | **Complete** |
| 9 -- Plugins | Plugin system, host API, 2-3 example plugins | All new code | **Complete** (framework; example plugins planned) |
| 10 -- Audio | MP3 playback with ME offloading (PSP) or rodio (Linux) | `audio.c`, `me.S`, `modules/audio/*` | **Complete** (framework) |
| 11 -- Polish | Transitions, update checker, scripting, FTP server | `transition.c`, `update.c` + new | **Complete** |
| 12 -- PSP GU backend | Hardware-accelerated sceGu rendering, PSIX-style UI matching desktop layout | New code | **Complete** |

Phase 12 was added after the original plan. The PSP backend was initially software-rendered, then switched to sceGu hardware acceleration with `Sprites` primitives for all 2D drawing. The PSP UI now renders the full PSIX-style layout: document icons with 6 layers, tabbed status/bottom bars, chrome bezels, procedural wave arc wallpaper, and paginated grid navigation.

Total Rust codebase: approximately 360,000 lines across 50 crate directories, substantially exceeding the original C codebase (~15,000 lines) due to the framework abstraction, 13 extracted app crates, browser engine with JS bindings, video decode pipeline, vector graphics, shader wallpapers, window manager, VFS, UE5 integration, and multiple skins. The vendored ffmpeg (~176,000 lines) is eliminated entirely.

---

## 19. Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| rust-psp SDK fork lacks needed PSP syscall bindings | Medium | Medium | Add bindings directly to the fork; fallback to raw unsafe FFI via C headers |
| Kernel mode support in SDK fork not yet implemented | High | Medium | Design for user mode first; kernel-mode features (ME, raw hardware) accessed via pre-compiled PRX loaded at runtime; extend SDK fork incrementally |
| PPSSPP infrastructure networking breaks on edge cases | Medium | Medium | Test on latest PPSSPP builds; report issues upstream; fallback to real hardware |
| PPSSPP MCP patches break on upstream update | Medium | Low | Pin to specific PPSSPP commit in Dockerfile; re-apply and fix patches when bumping version; patches are small and isolated to new files |
| PPSSPP internal APIs change (debugger, memory, GPU) | Medium | Medium | Patches primarily hook into stable internal APIs (Memory::Read, MIPSDebugInterface); GPU tools are more fragile -- pin to known-good PPSSPP version |
| PPSSPP headless mode inaccurate vs real PSP | Low | Medium | Validate critical rendering against real hardware at phase milestones; use VRAM dumps from MCP for automated visual regression |
| UE5 texture update latency causes visible lag on in-game monitors | Medium | Medium | Double-buffer the RGBA surface; only update on dirty frames; profile UpdateTextureRegions cost |
| FFI boundary introduces undefined behavior | Low | High | Fuzz the FFI boundary; validate all pointers on Rust side; use miri for unsafe auditing |
| GU rendering differences between PPSSPP and real PSP | Low | Medium | Validate rendering on real hardware at each phase milestone |
| Memory budget exceeded on PSP (32MB) | Low | High | Profile in PPSSPP; use bounded collections; lazy-load textures |
| Skin hot-swap causes state corruption | Medium | Medium | Snapshot VFS and command state before swap; validate SDI tree consistency after rebuild |
| UE5 version upgrade breaks FFI linkage | Low | Medium | Pin to C ABI only; no C++ mangled symbols; version the FFI header |
| Scope creep beyond embeddable OS into general-purpose OS | Medium | Medium | Strict adherence to non-goals; resist adding package management or virtual memory |
| SDL3 dependency issues on Pi (wayland vs X11 vs kms) | Low | Low | Provide framebuffer backend as fallback; document tested Pi OS versions |
| Window manager hit testing incorrect under high object count | Medium | Medium | Spatial index (grid-based) for hit testing if linear scan exceeds frame budget; benchmark with 20+ open windows |
| WM drag/resize latency from updating many SDI objects per frame | Low | Medium | Batch position updates; defer non-visible object updates; profile with Desktop skin at max window count |
| Conflict between OASIS_OS and tamper services on Pi | Low | Low | Strict separation via systemd unit ordering; OASIS_OS reads tamper state files but never writes to them |

---

## 20. Success Criteria

| Criterion | Verification Method |
|----------|-------------------|
| OASIS_OS boots to a themed dashboard on real PSP hardware | Visual verification on PSP-2000 with custom firmware |
| OASIS_OS boots to the same dashboard on Raspberry Pi | Visual verification on Pi 5 with SDL3 or framebuffer backend |
| In-game computer in UE5 renders a functional OS with player interaction | Play-in-Editor: interact with terminal, execute commands, browse files |
| At least 4 distinct skins render correctly from the same core framework | Load Classic, Modern, XP, and Desktop skins; verify layout, features, and visuals differ |
| Dashboard discovers and launches homebrew (PSP) / executables (Pi) / game UI (UE5) | Launch 3+ apps from dashboard on each platform |
| Remote terminal accessible via TCP from another machine | Telnet/netcat session with successful command execution |
| PSP establishes outbound remote session to agent host | From OASIS_OS on PSP (or PPSSPP), `remote dev-server` connects and allows command execution |
| VFS abstraction works across RealFS, GameAssetVFS, and MemoryVFS | Automated test: same command sequence produces correct results on all VFS backends |
| Command interpreter correctly gates features per skin | Dashboard skins expose dashboard + launcher commands; terminal-focused skins restrict to approved set; verify gating |
| UE5 callback system fires on file access and command execution | Automated test: access file -> verify ON_FILE_ACCESS callback fires in UE5 |
| Full development cycle runs in PPSSPP container without real hardware | End-to-end: build EBOOT, launch container, terminal connect, command test, MCP memory read -- all in Docker |
| PPSSPP MCP tools accessible by AI agents | Agent connects to container MCP port, reads PSP memory, takes screenshot, lists threads -- verified in CI |
| FFI boundary passes fuzz testing with no undefined behavior | Run cargo-fuzz on all FFI entry points; miri audit on unsafe blocks |
| Boot-to-dashboard time under 5 seconds on Raspberry Pi | Timed measurement from power-on to rendered dashboard |
| Zero unsafe blocks outside of platform FFI wrappers | `cargo geiger` audit of core and backend crates |
| Window manager supports drag, resize, focus, minimize, maximize, and close on Desktop skin | Manual test: open 5+ windows, perform all operations, verify correct z-ordering and clipping |
| Content clipping prevents rendering outside window boundaries on all backends | Automated test: render overflowing content in a window, verify no pixel bleed outside clip rect |
| Corrupted skin modifiers produce visible distortion without crashing WM | Load Corrupted skin, open windows, verify drag jitter, glitched frames, and visual artifacts render without panics |
| CI pipeline runs in Docker (`docker compose --profile ci run --rm rust-ci`) | All stages pass: fmt, clippy, test, build, deny |

---

## 21. References and Resources

| Resource | URL / Location |
|----------|---------------|
| Original C source (v1.90) | `psixpsp.7z` (not checked in) |
| rust-psp upstream | github.com/overdrivenpotato/rust-psp |
| rust-psp SDK fork | github.com/AndrewAltimit/rust-psp |
| PPSSPP emulator (GPL-2.0+) | github.com/hrydgard/ppsspp |
| PPSSPP infrastructure networking PR | github.com/hrydgard/ppsspp/pull/19827 |
| PPSSPP MCP patches [PLANNED] | `docker/ppsspp-patches/` (placeholder) |
| MCP specification | modelcontextprotocol.io |
| PSP SDK documentation | psp-archive.github.io/pspsdk-docs/ |
| PSP homebrew wiki | pspdev.github.io/ |
| Unreal Engine 5 C++ API | docs.unrealengine.com |
| UE5 UTexture2D / UpdateTextureRegions | docs.unrealengine.com/API/Runtime/Engine/Engine/UTexture2D/ |
| Rust FFI guide | doc.rust-lang.org/nomicon/ffi.html |
| libloading crate | crates.io/crates/libloading |
| Raspberry Pi OS Lite | raspberrypi.com/software/operating-systems/ |
| SDL3 Rust bindings | github.com/revmischa/sdl3-rs |
| ARK-4 custom firmware | github.com/PSP-Archive/ARK-4 |
| Infinity 2 persistent CFW | infinity.lolhax.org |
| cargo-fuzz | github.com/rust-fuzz/cargo-fuzz |
| This design document | `docs/design.md` |
| Documentation index | [`docs/README.md`](README.md) |
| Agent workflow and CI | [`AGENTS.md`](../AGENTS.md) |
| MCP control server | [`docs/mcp-server.md`](mcp-server.md) |

---

*End of Document*
