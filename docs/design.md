# OASIS_OS

## An Embeddable Operating System Framework in Rust

**Technical Design Document**
Version 3.1 -- October 2026
Author: Andrew
Classification: Personal Project -- Open Source

---

This document is the architectural overview: goals, layering, the contracts
between layers, and the platform-specific design decisions. It deliberately
does not repeat the reference material that has its own document; each
section links to the doc that owns the details. The full index is
[docs/README.md](README.md).

**Revision history**

| Version | Date | Changes |
|---------|------|---------|
| 3.0 | 2026-10 | Backend drawing-state contract and span-exact defaults (§5); slimmed `App` trait (§4.3); every skin generated from `skins/` with strict resolution (§6); one close / launch path in the desktop shell (§4.6); music IPC (§14); VFS rename invariants and symlink jail (§11); browser network hardening and script budgets (§16). Reference tables that duplicated the subsystem docs (command catalogue, FFI signatures, WM shortcuts, planned PPSSPP MCP patch set) were replaced by links. |
| 3.1 | 2026-10 | Remote terminal PSK challenge-response and the unencrypted-session caveat (§4.4, §16); eight extension traits (§5.1); `oasis_types::ipc` as the one IPC home (§11.2); music IPC on WASM (§14); PRX size gate and Python tooling tests in CI (§17.2). |
| 2.6 | 2026-09 | App VFS access, ticking and idle-frame elision; window snapping, tiling and animations |

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

OASIS_OS is an embeddable operating system framework written in Rust. It provides a fully functional, skinnable shell interface -- complete with a scene-graph UI, window manager, command interpreter, virtual file system, browser engine, plugin system, and remote terminal -- that can render anywhere you can provide a pixel buffer and an input stream.

The project was built from scratch in Rust starting in early 2026, inspired by PSP homebrew shells like PSIX. The architecture -- a themed dashboard driven by a custom scene-graph engine called SDI (Simple Display Interface), with platform abstraction via trait-based backends -- turned out to be a natural foundation for something more general. The trait-based backend system designed for PSP / SDL portability extends cleanly to a browser canvas (WASM) and to a texture inside Unreal Engine 5, where in-game computer props become fully interactive systems rather than scripted UI sequences.

The framework supports multiple "skins" -- visual and behavioral personalities that determine what the OS looks like and what it exposes to the user. Seventeen skins are built in, each a TOML directory under `skins/` that is compiled into the binary (§6). All skins share the same core: scene graph, command interpreter, virtual file system, browser engine, networking, and plugin infrastructure. The skin defines layout, theme, feature gating, and visual style.

Primary deployment targets are: in-game computers in UE5 projects (rendered as interactive props), real PSP hardware running modern custom firmware (6.60/6.61 with ARK-4), the web (a WASM build with a Canvas 2D backend), and a tamper-responsive briefcase (a separate hardware project) where a Raspberry Pi 5 boots directly into OASIS_OS as the field-deployable shell. On the briefcase, OASIS_OS is the operator-facing interface for managing AI agents in untrusted environments -- the tamper detection, LUKS encryption, and cryptographic wipe services run alongside it as systemd units. On a PSP connected to infrastructure WiFi, OASIS_OS's remote terminal enables direct command sessions to machines running AI agents, making a 2004 handheld a viable field controller for an AI agent ecosystem. The original C codebase (~15,000 lines) provides the proven UI design; the Rust rewrite provides memory safety, cross-platform backends, and the extensibility to support all targets from a single codebase.

---

## 2. Project Goals and Non-Goals

### 2.1 Goals

- Build an embeddable OS framework in Rust that renders to any pixel buffer and consumes any input stream
- Implement a skin system that separates visual/behavioral personality from core OS logic
- Integrate as a native library in Unreal Engine 5 for interactive in-game computer props
- Port the original C shell OS as the Classic skin, preserving its icon-grid dashboard and theming
- Achieve cross-platform execution on PSP hardware (via rust-psp), PPSSPP emulator, Raspberry Pi and desktop (via SDL3), the web (via WASM), and UE5 (via render target)
- Provide a virtual file system abstraction backed by real files, game assets, or procedural content depending on platform
- Add remote terminal access for headless device management over TCP/IP, doubling as the primary interface for controlling remote AI agents from portable devices
- Implement a scriptable command layer for automation and gameplay scripting
- Establish a plugin/module system for runtime-extensible functionality
- Serve as the user-facing OS for a tamper-responsive briefcase (Raspberry Pi), replacing bare TTY login with a themed, interactive shell
- Leverage the existing AI agent pipelines and MCP server ecosystem for code translation, build orchestration, and automated testing

### 2.2 Non-Goals

- Building a general-purpose operating system kernel (the framework runs atop existing kernels, firmware, game engines, or a browser)
- WASM-based plugin sandboxing or binary compatibility (unnecessary complexity for the use case; the WASM *build* is a host, not a sandbox for plugins)
- Porting the vendored ffmpeg codebase (replaced by modern alternatives or external linkage)
- Supporting commercial game piracy or circumventing DRM protections
- Achieving feature parity with full Linux desktop environments
- Real-time or safety-critical certification

---

## 3. System Architecture

### 3.1 High-Level Architecture

OASIS_OS follows a layered architecture with strict separation between the platform-agnostic core framework, platform-specific backends, skin definitions, and host applications. The core framework never calls platform-specific APIs directly -- all I/O passes through trait boundaries defined in `oasis-types` (§5).

| Layer | Responsibility | Scope |
|-------|---------------|-------|
| Core Framework | Scene graph (SDI), window manager, command interpreter, virtual file system, skin/theming engine, browser engine, plugin interface, input event pipeline | All platforms |
| Apps | One crate per app (`oasis-app-*`) against the `App` trait in `oasis-app-core`; never depend on `oasis-core` | All hosts that run the shell |
| Backends | Rendering (GU, SDL3, Canvas 2D, software RGBA buffer), input (PSP pad, keyboard/mouse, DOM events, FFI queue), networking (pspnet, std::net), audio | Platform-specific impl |
| Skins | Layout definitions, theme assets, feature gating, visual style, "personality" (what the file tree looks like, what apps are discoverable) | Content layer |
| Host Application | Desktop shell (`oasis-app`), WASM page, UE5 game via `oasis-ffi`, PSP EBOOT, Linux init system -- owns the OS instance, ticks it, provides render target and input | Integration layer |

The backends depend only on the foundation crates: `oasis-backend-sdl` needs `oasis-types`, `oasis-net`, `oasis-audio`, `oasis-rasterize` and `oasis-shader`, and `oasis-backend-ue5` needs `oasis-types`, `oasis-rasterize` and `oasis-audio`. Neither depends on `oasis-core`, so a desktop build without JavaScript links no QuickJS at all. The hosts (`oasis-app`, `oasis-ffi`, `oasis-backend-wasm`, `oasis-backend-psp`) are the only crates that combine a backend with `oasis-core`.

### 3.2 Repository Structure

The project is a Cargo workspace at the repository root (edition 2024, workspace version 1.4.0, 41 members). The authoritative crate list, with one line per crate, is the dependency graph in [`CLAUDE.md`](../CLAUDE.md#crate-dependency-graph); the README has the [crate table by layer](../README.md#crates).

```
oasis-os/
+-- Cargo.toml        # workspace root: 41 members, oasis-backend-psp excluded
+-- crates/           # 51 crate directories:
|                     #   41 workspace members (foundation, rendering, content, apps,
|                     #   backends, oasis-app desktop shell, tools)
|                     #   oasis-backend-psp (workspace `exclude`, mipsel-sony-psp)
|                     #   9 standalone PSP crates (overlay PRX, recovery EBOOT, devloop,
|                     #   ME boot, PRX decrypt, USB client/debug/trace/vbus)
+-- skins/            # 17 skin directories (every built-in skin, single source of truth)
+-- docs/             # this document, guides, subsystem docs, ADRs (index: docs/README.md)
+-- examples/         # runnable examples registered as [[example]] targets
+-- docker/           # rust-ci, psp-build and PPSSPP Dockerfiles
+-- scripts/          # WASM build/serve, PSP devloop and test scenarios
+-- site/, www/       # GitHub Pages site and the WASM page shell
```

`oasis-app` is the desktop shell: a library (`oasis_app::Shell`, plus the headless `harness` used by the e2e suites) and four binaries -- `oasis-app` (the shell), `oasis-screenshot` (per-skin captures), `screenshot-tests` (the visual regression run in CI) and `e2e-tests` (a TV Guide streaming pass/fail runner, not run in CI).

The rust-psp SDK is an external dependency hosted at [github.com/AndrewAltimit/rust-psp](https://github.com/AndrewAltimit/rust-psp) (MIT), a modernized fork with edition 2024, safety fixes and kernel mode support, referenced as a git dependency pinned to one revision across every PSP crate (CI checks the pins). The PSP backend is excluded from the workspace because it requires the `mipsel-sony-psp` target and is built separately with `cargo psp`; it depends on `oasis-core` (without default features), `oasis-browser` (`psp` + `javascript`), `oasis-js` (`psp-bindgen`), `oasis-video` (`demux-lite`) and `oasis-shader`.

**Planned crates (not yet created):**

- `oasis-backend-framebuffer` -- Direct Linux framebuffer rendering (headless Pi)
- `oasis-platform-psp` -- PSP hardware services: USB, UMD, power, Media Engine, kernel services
- `oasis-platform-linux` -- Pi-specific: GPIO (rppal), systemd integration, sysfs queries

### 3.3 Platform Targeting

| Target | Build | Render Backend | Input Backend | VFS Backend | Status |
|--------|-------|----------------|---------------|-------------|--------|
| Desktop (dev) | `cargo build --release -p oasis-app` | SDL3 | Keyboard/mouse/gamepad | `MemoryVfs` seeded at boot | Implemented |
| Web (WASM) | `./scripts/build-wasm.sh --release` | Canvas 2D | DOM keyboard/mouse/touch | `MemoryVfs` | Implemented ([wasm-backend.md](wasm-backend.md)) |
| PSP / PPSSPP | `cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo psp --release` | sceGu hardware (Sprites) | PSP controller | `ms0:/` real FS | Implemented ([psp-architecture.md](psp-architecture.md)) |
| UE5 (in-game) | `cargo build --profile release-ffi -p oasis-ffi` (cdylib) | Software RGBA buffer read by UE5 | FFI event queue | `GameAssetVfs` | Implemented ([ffi-integration.md](ffi-integration.md)) |
| Raspberry Pi (briefcase) | `cargo build --release -p oasis-app --target aarch64-unknown-linux-gnu` | SDL3 | Keyboard/gamepad | `MemoryVfs` | Planned (SDL3 backend works, cross-compile not yet tested) |

Std support on the PSP target is provided by the rust-psp SDK with `RUST_PSP_BUILD_STD=1`, which builds a custom sysroot with PSP-specific PAL implementations. The PSP backend uses sceGu hardware-accelerated 2D rendering with `Sprites` primitives; ABGR conversion uses the inherent `Color::to_abgr` / `Color::from_abgr` from `oasis-types`, and the bitmap glyph atlas is built from the shared `oasis_types::bitmap_font` table.

---

## 4. Core Framework

### 4.1 SDI -- Simple Display Interface

SDI is the scene-graph engine at the center of the framework. The original C implementation (~1,574 lines in `sdi.c`) manages named UI objects with position, size, alpha, z-order, and image data through a flat registry. The Rust SDI (`oasis-sdi`) keeps that model: a registry of named `SdiObject`s with position, size, color, texture, text, z-order, gradients, rounded corners, shadows and alpha, with exact per-object dirty tracking so idle frames can skip the redraw.

SDI is deliberately simple. It is not a DOM, not a layout engine, and not a retained-mode GUI framework. It is a flat collection of named, positionable, blittable objects with z-ordering and alpha blending. This simplicity is a feature -- it maps directly to what every backend can provide efficiently, from PSP GU hardware to a UE5 texture blit.

| SDI Operation | Original C | Rust Equivalent |
|--------------|-----------|-----------------|
| Create object | `sdi_new("name")` | `sdi.create("name")` |
| Set position | `sdi_set_x("name", 100)` | `sdi.get_mut("name")?.x = 100` |
| Draw frame | `sdi_draw()` | `sdi.draw(&mut backend)?` |
| Z-ordering | `sdi_move_top("name")` | `sdi.move_to_top("name")` |

Lookups of a missing object return the typed `SdiError::NotFound`. An object's alpha scales everything it draws (fill, text, text shadow, stroke and box shadow), which is what window open/close fades rely on. The overlay draw passes re-sort by z-order before drawing, so a create/destroy between passes cannot walk stale handles.

### 4.2 Command Interpreter

The command interpreter is a registry-based dispatch system in the `oasis-terminal` crate, which depends only on `oasis-types`, `oasis-vfs`, `oasis-platform` and `oasis-net`. Commands implement a `Command` trait with an `execute()` method returning structured output (`CommandOutput`). The interpreter includes full shell features: variable expansion (`$VAR`, `${VAR}`), command substitution (`$(...)`), glob expansion, aliases, history (`!!`, `!n`), piping, redirects, and command chaining (`&&`, `||`, `;`). Inline control flow and `run` scripts share one parser (`script/`) with nested `if`/`elif`/`else`, `while`/`until`, `for` and `case` blocks; a trailing `&` queues a background job managed with `jobs`/`fg`/`bg`/`kill`. Hosts drive interactive terminals through a `ShellSession` (`session.rs`) that adds readline-style editing, tab completion (commands, `$VARS`, VFS paths), Ctrl+R history search and history persisted to `/home/user/.oasis_history`.

- **Builtins.** The 21 shell builtins (`help`, `run`, `xargs`, `history`, `set`, `alias`, `function`, `jobs`, `kill`, ...) live in one table, `BUILTINS` in `builtins.rs`, which the executor dispatches from before the command map; `help`, `which` and completion read the same table. A builtin always wins over a registered command of the same name.
- **Exit status.** `CommandOutput::Status { code, output }` lets a command print and fail without an `Err`; the executor records the code for `&&`, `||`, `$?` and `if`.
- **Layering.** Commands that need higher layers are registered by `oasis-core`: `skin` (it needs `oasis-skin`), agent/MCP, browser, plugin, script (`cron`, `startup`), file transfer, update and TV. `oasis_core::terminal::register_builtins` is the entry point every host calls. The desktop shell adds `mcp-server` when built with the `mcp` feature.

Counted from the source: 75 commands in `oasis-terminal`, 19 in `oasis-core`, 1 (`tv`) in `oasis-apps` and 1 in `oasis-app` (feature `mcp`) -- 95 on the default desktop build -- plus the 21 builtins. The catalogue is [terminal-commands.md](terminal-commands.md) and the how-to is [adding-commands.md](adding-commands.md).

### 4.3 Input Pipeline

The input system translates platform-specific events into a platform-agnostic `InputEvent` enum. Every backend maps its native input to this enum. The core framework never sees raw platform input.

| Event Type | PSP Source | Desktop/Pi/Web Source | UE5 Source |
|-----------|-----------|-------------------|------------|
| `CursorMove(x, y)` | Analog stick | Mouse motion / gamepad stick | Line trace UV -> screen coords |
| `ButtonPress(Button)` | D-pad, face buttons | Keyboard / gamepad | Mapped interact keys |
| `TriggerPress(Trigger)` | L/R shoulder | Q/E keys, gamepad triggers | Shoulder buttons on gamepad |
| `TextInput(char)` | On-screen keyboard | Physical keyboard | Captured keyboard when "seated" |
| `PointerClick(x, y)` | N/A | Mouse click / tap | Line trace hit on monitor mesh |
| `FocusGained` / `FocusLost` | N/A (always focused) | Window focus / blur | Player enters / leaves interaction range |
| `Key { key, mods }` | N/A (never emitted) | Every physical key-down + held modifiers | `OASIS_EVENT_KEY` via FFI |

The hook-by-hook `App` reference (signatures, defaults, when each is called) is [writing-apps.md](writing-apps.md); the subsections below explain the design.

#### 4.3.1 Keyboard Model: Gamepad Events + Raw Keys

Keyboard input is delivered in two layers so the PSP / gamepad path is untouched while desktop apps get real shortcuts:

1. **Gamepad-style events** (`ButtonPress`, `TriggerPress`, `Backspace`, `Tab`, ...). Keyboard backends synthesize these from keys with one canonical table, `Key::legacy_press` / `Key::legacy_release` in `oasis-types/src/input.rs` (arrows -> d-pad, Enter -> Confirm, Esc -> Cancel, Space -> Triangle, F1/F2 -> Start/Select, Q/E -> L/R triggers, F11 -> fullscreen).
2. **Raw keys**: `InputEvent::Key { key: Key, mods: Modifiers }`. `Key` covers `Char(c)` (letters lowercase), `Space`, `Enter`, `Escape`, `Tab`, `Backspace`, `Delete`, `Insert`, `Home`, `End`, `PageUp`, `PageDown`, arrows and `F(1..=12)`. `Modifiers` is a bitflags-style set (`SHIFT`, `CTRL`, `ALT`, `SUPER`) with helpers such as `ctrl()`, `has_command()` and `only(Modifiers::CTRL)` for exact shortcut matching.

For each key-down the SDL and WASM backends emit, in order: `Key`, then its gamepad-style twin (if any), then `TextInput` for printable characters. The host routes `Key` to the focused app's `App::handle_key` (default `None`). A `KeyTwinFilter` then drops the twin that immediately follows when either:

- the app returned `Some(action)` (it consumed the key as a shortcut, so an app claiming `Key::Enter` does not also receive `Button::Confirm`), or
- the focused target accepts text (`App::accepts_text()`, the terminal, or the browser URL bar / a focused form field) and the key merely types a character (`Key::produces_text`). This is what stops "q"/"e" from switching virtual desktops and Space from firing Triangle while typing.

Gamepad events that do not immediately follow a matching `Key` are never filtered. Apps type from `handle_text_input`, keep every feature reachable through `handle_input` (PSP has no raw keys), and use `handle_key` for keyboard-only accelerators. Triggers are consumed by the host (virtual desktops, dashboard tabs) and never reach apps; the mouse wheel over a window reaches `App::handle_scroll`.

#### 4.3.2 App VFS Access

Every input / render hook on `App` sees at most a shared `&dyn Vfs`. Read-only follow-ups to an input (navigating, listing a folder, opening the clicked file) happen inside the handler, which receives the VFS. Apps that need to change the file system queue the work and implement `App::apply_vfs_ops(&mut dyn Vfs) -> bool`, the only hook with mutable access, which the hosts call once per frame for every open app runner. Apps that need the *host* to do something (play a file, tune the TV, open a URL) queue `AppRequest { path, data }` VFS IPC writes and hand them over in `App::drain_requests`; the host performs every queued request each frame, in order.

#### 4.3.3 App Ticking and Layout

`App::tick(dt_ms, &dyn Vfs) -> bool` advances time-driven state by wall time. The hosts call it once per frame for every open runner, focused or not, including frames whose redraw is elided, so app time never depends on the frame rate or on input. Apps accumulate `dt_ms` instead of counting calls, return `true` when something they draw changed, and poll mirrored VFS state here (the Internet Radio's published status, Video Embed's search results). `App::layout(cw, ch, theme)` runs right before every `draw_windowed` and `handle_click(lx, ly, vfs)` with the same size, so apps record their size and hit-test geometry in plain fields instead of smuggling it out of `draw_windowed` through `Cell`s; `handle_click` takes no size of its own.

The per-frame order for every open runner is: drain requests, `apply_vfs_ops`, `tick`, `take_close_request`; runners retired by a close get `release_resources` before they are dropped.

#### 4.3.4 Frame Requests and Idle Elision

The SDL host skips clear/draw/present on frames where nothing visible changed. Window content is painted through the WM draw callback, outside the SDI dirty tracking, so an open window does not force a redraw by itself; instead the host ORs over the visible (non-minimized, active-desktop) windows:

- `AppRunner::wants_frame()` -- true until the next drawn frame after any input (a wheel event only when `handle_scroll` reports a change), a `tick` returning `true`, applied VFS ops, a terminal sync that changed the lines or scroll, or mutable access via `delegate_as_mut`; and always true while `App::wants_frame()` is (continuous content such as the video embed).
- `BrowserWidget::wants_frame()` -- pending layout / repaint / dirty rects / unpainted scroll, a page or its resources still loading, and active CSS animations or transitions. Fired JS timers dirty the layout during `tick`, which runs on elided frames too.
- WM window animations and drags.

Video/audio playback, kiosk apps, the input grace window and the 1 s heartbeat still force redraws (`OASIS_FRAME_STATS=1` reports the elided share).

### 4.4 Remote Terminal

On platforms with networking (PSP via infrastructure WiFi, Linux, desktop), the framework runs a TCP listener that accepts remote terminal connections. The remote terminal feeds lines into the same command interpreter as local input. This is functional on real hardware and in PPSSPP (1.19+ maps `sceNetInet` to host sockets). In UE5, the terminal is available for debugging.

On the briefcase Pi, the remote terminal is the primary access path for headless operation. OASIS_OS can also act as a client, establishing outbound TCP sessions to remote machines with the `remote` command (see Section 10). The listener binds loopback by default and refuses to listen on all interfaces without a pre-shared key. The PSK never crosses the wire: the listener sends `AUTH_REQUIRED <nonce>` and expects `AUTH <hex HMAC-SHA256(psk, nonce)>` (`oasis_net::psk_response`); unauthenticated connections close after 20 s and failed logins feed a server-wide lockout. The session itself is not encrypted, so use it on trusted networks or tunnel it. Protocol, auth handshake and limits: [networking.md](networking.md).

### 4.5 Window Manager

The window manager (WM, `oasis-wm`) enables skins that present multiple movable, resizable, overlapping windows -- producing the feel of a real desktop operating system. The WM is a consumer of the SDI API, not a modification to it. SDI remains a flat scene graph; the WM is the smart layer that creates, positions, and manipulates groups of SDI objects. The API (window types, events, snapping, keyboard shortcuts, tiling, virtual desktops, animations) is documented in [window-manager.md](window-manager.md).

#### 4.5.1 Design Principle: SDI Stays Flat

SDI has no concept of grouping, containment, or parent-child relationships. The WM introduces the illusion of hierarchy by a naming convention: a window named `"editor_01"` owns SDI objects `"editor_01.frame"`, `"editor_01.titlebar"`, `"editor_01.title_text"`, `"editor_01.btn_close"`, `"editor_01.btn_minimize"`, `"editor_01.btn_maximize"` and `"editor_01.content"`. Moving a window moves every object sharing that prefix; bringing it to front moves every object in the group to the top of the z-order.

The WM is active when the skin's `features.toml` sets `window_manager = true` (12 of the 17 skins, including `classic`, so apps open as windows over the dashboard). `paper`, `retro-cga`, `vaporwave`, `highcontrast` and `corrupted` run without it. The desktop, WASM and PSP hosts all drive the same `WindowManager`; the PSP backend uses a PSP-tuned `WmTheme` (`psp_wm_theme()`).

#### 4.5.2 Window Anatomy

Each window is a `Window` struct in the WM that tracks its SDI child objects, state, and geometry. The visual composition is defined by the active skin's theme (`WmTheme`, built from the skin) -- the WM handles behavior, the skin handles appearance.

| SDI Object | Role | Skin Provides |
|-----------|------|---------------|
| `{id}.frame` | Background rectangle, receives drop shadow or border | Frame color, corner radius, border width |
| `{id}.titlebar` | Draggable title bar region | Titlebar height, background color/gradient, active vs inactive colors |
| `{id}.title_text` | Window title label | Font size, color, alignment |
| `{id}.btn_close` / `btn_minimize` / `btn_maximize` | Titlebar buttons | Glyphs and colors, `button_side` |
| `{id}.content` | Content area where the host paints the app | Content background color |

#### 4.5.3 Hit Testing and Content Clipping

A `PointerClick` walks the window list topmost first and checks titlebar buttons, then the titlebar body (drag; a second click within 500 ms and 6 px toggles maximize), then resize edges, then the content area (forwarded content-local to the app as `WmEvent::ContentClick`). A click on no window is a desktop click. Clicking a window that is not topmost focuses it first.

Window content is painted by the host inside a clip rect the WM pushes for that window, through the backend clip stack (§5.2), so each window's content is isolated and nested panels intersect correctly.

#### 4.5.4 Window Types

| Window Type | Behavior | Use Case |
|------------|----------|----------|
| **App Window** | Draggable, resizable, closable, minimizable, maximizable | Text editor, file browser, any app |
| **Dialog** | Modal, centered; blocks input to other windows until dismissed | Confirmation prompts, unsaved-changes prompts |
| **Panel** | Docked to a screen edge | Taskbar, sidebar, status bar |
| **Floating Widget** | Always on top, no minimize/maximize | Clocks, toasts, quick-launch |
| **Fullscreen** | No chrome, covers the work area (a kiosk window on desktop) | Terminal, games, video |

#### 4.5.5 Platform Applicability

| Platform | WM Enabled | Rationale |
|----------|-----------|-----------|
| PSP | Yes (compact) | 480x272. Apps open as windows over the icon-grid dashboard and can go fullscreen. |
| Raspberry Pi (briefcase) | Optional | Depends on display; headless operation uses the remote terminal. |
| Desktop / Web | Yes | The desktop shell always hosts apps in windows ("fullscreen" is a kiosk window); the WASM host also has a fullscreen mode. |
| UE5 (in-game) | Yes | The WM resolution matches the virtual screen texture (e.g. 1024x768 for a high-res in-game monitor, 480x272 for a handheld prop). |

#### 4.5.6 Skin-Driven Appearance and the Corrupted Skin

The WM handles behavior uniformly; the skin defines every visual detail. Titlebar layout is skin data too: `button_side = "left"` gives macOS order, `"right"` gives minimize, maximize, close; `title_align = "center"` mirrors the reserved button space and titles that do not fit are truncated with `...`.

The Corrupted skin demonstrates behavioral hooks: jittered positions, garbled text, alpha flicker, horizontal tear bands and RGB-split text shadows, configured in its `corrupted.toml` and parsed into `oasis_skin::CorruptedModifiers`. The desktop shell applies them every drawn frame between layout and the SDI pass: `apply(&mut SdiRegistry, &mut SimpleRng)` distorts the scene and returns a `CorruptionUndo` that `restore`s it right after drawing, so the glitch never accumulates and the SDI dirty tracker sees no spurious change. The shell keeps redrawing while the skin is active, and skips the effect under `reduced_motion`. Other hosts (WASM, UE5, PSP) do not apply it yet.

### 4.6 Desktop Shell

The desktop shell (`oasis-app`, `oasis_app::Shell`) is the reference host. Its design rules, established in the 2026-10 overhaul:

- **One launch path.** Every launch site (dashboard tile, MCP `open_app`, `OASIS_APP`, file associations, plugin tiles) goes through `launch::launch` with `LaunchOpts`. Titles without a dashboard tile (`AppEntry::for_title`) launch the same way.
- **One close path.** `windows::close_app_window(state, sdi, vfs, id, CloseHow)` is the only way a window closes. It asks the app first (`App::on_close_requested`, so unsaved edits raise a prompt) unless the close is forced, retires the runner so `release_resources` runs, stops media the window owned, and falls back to the dashboard. `wm close [--force]` and MCP `close_window { force }` use it.
- **Windows only.** There is no separate fullscreen app mode on desktop; kiosk windows are windows without chrome.
- **Feature layers.** TV, radio, network and content state live in their own layers (`TvLayer`, `RadioLayer`, ...) of `AppState`, and the skin's backend resources (wallpaper, layout textures, decal layers, cursor) in `SkinAssets`; skin swaps and resolution changes rebuild the dashboard from `dashboard_apps()`, which keeps plugin-registered apps, and keep the status bar and taskbar. Shell-only state (the `settings` store and `UserPrefs`, the UI sound queue) lives in `oasis-app`, not `oasis-core`.
- **One command-output path per caller.** `commands::apply_command_output` runs a local terminal result (server toggles allowed); `commands::run_remote_command` serves the remote-terminal listener and MCP `run_command` alike (server toggles refused). Both resolve `skin` requests in order, so `skin xp; skin @variant:dark` derives from xp.
- **Per-frame services.** `Shell::step` polls the services in `commands/` (`services`: listener / FTP / MCP / remote client; `ipc`: `wm` and `plugin` request files; `settings_ipc`: the Settings app's `/system/ipc`) plus `ui_ipc` and the media controllers. Every request file is read and cleared through one `take_request` (missing, empty and whitespace-only files are no-ops), and status files are rewritten only when they change (`publish_if_changed`). Input routing is `input/` (`handle_event`, then the `desktop` or `dashboard` mode handler).

Every user-flow change gets an end-to-end scenario in `crates/oasis-app/tests/e2e_*.rs`, driven through the headless harness ([testing.md](testing.md)).

---

## 5. Backend Abstraction Layer

The backend traits live in `oasis-types/src/backend/` and are re-exported by `oasis-core`. The rationale for splitting them is [ADR-003](adr/003-backend-trait-design.md).

### 5.1 Rendering Backend Traits

Every rendering operation passes through `SdiCore` (13 required methods: `init`, `clear`, `blit`, `fill_rect`, `draw_text`, `swap_buffers`, `load_texture`, `destroy_texture`, `set_clip_rect`, `reset_clip_rect`, `measure_text`, `read_pixels`, `shutdown`; plus the provided `viewport_size`) and eight extension traits with 46 default-implemented methods: `SdiShapes` (8), `SdiGradients` (2), `SdiText` (7), `SdiTextures` (4), `SdiClipTransform` (8), `SdiVector` (6), `SdiBatch` (2) and `SdiRenderTarget` (9). `SdiBackend` is a marker super-trait with a blanket impl for any type implementing `SdiCore` plus all eight extensions, and every consumer takes `&mut dyn SdiBackend`.

- **Span-exact defaults.** The shape defaults (circles, rings, rounded rects, thick lines, even-odd polygons) emit `fill_rect` spans from the shared generators in `oasis_types::rasterize`, the same generators the SDL and UE5 rasterizers use, so a backend that only implements `SdiCore` draws the same pixels; overriding is for speed, not correctness. PSP and every test mock rely on the defaults.
- **Boilerplate.** `oasis_types::impl_sdi_defaults!(T)` writes the eight empty extension impls for a mock or null backend (`impl_sdi_defaults!(T; except SdiShapes, ..)` skips the ones a type writes itself).
- **Typed errors.** `BackendError::{TextureNotFound, Unsupported, RenderTargetNotFound}`; the render-target defaults return `Unsupported`.

| Backend | Crate | Surface | Notes |
|---------|-------|---------|-------|
| SDL3 | `oasis-backend-sdl` | GPU renderer (window) | Span generators + glyph masks from `oasis-rasterize`; skin TTF/OTF fonts via fontdue |
| WASM | `oasis-backend-wasm` | Canvas 2D | Native Canvas paths for shapes; bitmap glyph masks from `oasis-rasterize` |
| UE5 | `oasis-backend-ue5` | `SoftwareBuffer` RGBA framebuffer | UE5 reads it through `oasis_get_buffer` (`Ue5Backend::buffer()`); there is no `get_buffer` on `SdiCore` |
| PSP | `oasis-backend-psp` | sceGu display lists (Sprites) | Relies on the trait defaults for shapes; VRAM glyph atlas |
| Test | `oasis-test-backend` | Mock / recording / clip-audit | Records `DrawCommand`s for assertions |

### 5.2 Drawing-State Contract

Every backend follows one contract (rustdoc on `SdiCore`, `SdiClipTransform` and `SdiRenderTarget`):

- `set_clip_rect` is absolute (untranslated) and replaces the clip; `push_clip_rect` is translated and intersects with the current clip; `current_clip_rect` is the push stack's top.
- `clear` ignores the clip and does not blend.
- A bound render target starts with no clip and no translate, and unbinding restores the parent's state (`stacks::DrawState`); `viewport_size` reports the bound surface.
- Composites scale the target to the destination rect; `read_render_target` requires an exact-size buffer.

`oasis_test_backend::conformance` pins the contract on SDL and UE5 (`cargo test -p oasis-backend-sdl --test backend_parity`).

### 5.3 Input, Network, Audio and Clipboard Traits

- `InputBackend::poll_events() -> Vec<InputEvent>` (events in §4.3).
- `NetworkBackend` (`listen`, `listen_loopback`, `accept`, `connect`, optional `tls_provider`) hands out boxed non-blocking `NetworkStream`s; `OasisError::is_would_block()` is the one way to tell "no data yet" from a failure.
- `AudioBackend` covers track playback plus a streaming path (`load_streaming`, `feed_data`, `streaming_can_accept`, `finalize_streaming`, `feed_pcm_f32`); see [audio-engine.md](audio-engine.md).
- `ClipboardBackend` handles copy/paste, with an in-memory fallback.

| Operation | PSP (sceNetInet) | Desktop / Pi (std::net) | UE5 (debug only) |
|-----------|-----------------|------------------|-------------------|
| TCP listen / accept | sceNetInetSocket + Bind + Listen / Accept | `TcpListener` | `TcpListener` on localhost |
| TCP send/recv | sceNetInetSend / Recv | `TcpStream` | `TcpStream` |
| TCP connect (outbound) | sceNetInetConnect | `TcpStream::connect` | `TcpStream::connect` |
| WiFi connect | sceNetApctlConnect | N/A (OS manages) | N/A |
| DNS resolve | sceNetResolverStartNtoA | `ToSocketAddrs` | `ToSocketAddrs` |

Outbound TCP connect is essential for the PSP remote agent control use case (Section 10).

---

## 6. Skin System

### 6.1 Skin Architecture

A skin is a data-driven configuration that defines the visual and behavioral personality of an OS instance. Skins do not contain code -- they are TOML files referencing layout definitions, theme assets, and feature flags. The decision record is [ADR-005](adr/005-toml-skin-system.md); the full TOML reference is [skin-authoring.md](skin-authoring.md).

| Skin Component | Description | Format |
|---------------|-------------|--------|
| Manifest | Name, version, author, description, target resolution, optional parent skin (`inherits = "classic"`) | `skin.toml` |
| Layout | SDI object definitions: names, positions, sizes, z-order, visibility defaults | `layout.toml` |
| Theme | 9 base colors from which ~30 UI element colors are derived, plus overrides, WM theme, effects, wallpapers (incl. shader wallpapers) | `theme.toml` + assets |
| Feature gates | Dashboard, terminal, window manager, browser, ... | `features.toml` |
| Strings | User-facing text: menu labels, boot text | `strings.toml` |
| Corrupted modifiers | Optional glitch parameters (§4.5.6) | `corrupted.toml` |

### 6.2 Skins

All 17 built-in skins are generated at build time from their `skins/<name>/` directories (`oasis-skin/build.rs` embeds the TOML and assets and emits the name list); no skin is defined in Rust, and loading `skins/<name>` from disk gives exactly the built-in. A snapshot test pins every built-in's parsed form.

| Resolution | Skins |
|------------|-------|
| 480x272 (PSP native, default) | classic, psix-tribute, psix-hifi, altimit, modern, paper, retro-cga, vaporwave, highcontrast, corrupted |
| 640x480 | win95 |
| 800x600 | desktop, macos, gnome, balatro, solarized |
| 1024x768 | xp |

Skins with distinct behavior:

| Skin | Description | Primary Target |
|------|-------------|---------------|
| Classic | PSIX-style icon grid dashboard with document icons, tabbed status/bottom bars, chrome bezels, wave arc wallpaper | PSP / Pi / Desktop |
| PSIX Tribute / PSIX HiFi | PSIX tributes layered on Classic (`inherits` manifest inheritance): shaped chrome, baked wallpaper, media dock | PSP / Desktop |
| XP | Windows XP Luna-inspired theme with gradient titlebars, taskbar and start menu | Desktop / UE5 |
| Desktop | Window-like panels, taskbar, start menu analog, multiple apps visible simultaneously | UE5 / Pi |
| Corrupted | Glitch aesthetic; designed for randomized positions, garbled output and "repair" puzzles (§4.5.6) | UE5 |

### 6.3 Skin Loading and Switching

Resolution is strict: `resolve_skin` returns an error for an unknown or broken name or path (so `skin typo` does not silently act on Classic), while `resolve_skin_or_classic` is used only on boot paths (CLI argument, `OASIS_SKIN`, the WASM `?skin=` parameter). `skin lint` validates a skin directory, including unknown shader names.

Skins can be hot-swapped at runtime. The host loads the new skin, tears down and rebuilds the SDI scene from the new layout, reapplies the theme and WM theme, and reconfigures feature gates. The VFS is preserved across skin swaps, as are plugin-registered apps, the status bar and the taskbar.

---

## 7. Unreal Engine 5 Integration

### 7.1 Integration Architecture

OASIS_OS compiles as a C-compatible dynamic library (`oasis-ffi`, cdylib) that UE5 links against. The FFI boundary exports an opaque handle API; the UE5 side wraps it in an Actor Component that owns an OS instance, ticks it per frame, and bridges rendering and input. The full C API, event codes, header and a UE5 outline are in [ffi-integration.md](ffi-integration.md).

```
+-----------------------------------------------------+
|  Unreal Engine 5                                    |
|  AComputerTerminalActor                             |
|    +-- UStaticMeshComponent (monitor prop)          |
|    +-- UOasisOSComponent                            |
|    |     +-- Owns: OasisInstance* (opaque FFI)      |
|    |     +-- Owns: UTexture2D (render target)       |
|    |     +-- Tick(): oasis_tick(handle, dt)         |
|    |     +-- Render(): oasis_get_buffer(handle)     |
|    |     |            -> UpdateTextureRegions()     |
|    |     +-- Input(): oasis_send_input(handle, evt) |
|    +-- UMaterialInstanceDynamic (texture -> mesh)   |
+-----------------------------------------------------+
        | FFI boundary (C ABI)
+-------+---------------------------------------------+
|  OASIS_OS (oasis-ffi)                               |
|  Ue5Backend      (SdiBackend, SoftwareBuffer RGBA)  |
|  FfiInputBackend (InputBackend, event queue)        |
|  Ue5AudioBackend (AudioBackend, host callbacks)     |
|  GameAssetVfs    (Vfs, read-only base + overlay)    |
+-----------------------------------------------------+
```

### 7.2 FFI Boundary

The FFI layer exports 26 C-ABI functions: lifecycle and rendering (`oasis_create`, `oasis_create_full`, `oasis_destroy`, `oasis_tick`, `oasis_send_input`, `oasis_get_buffer`, `oasis_get_dirty`), commands and VFS (`oasis_send_command`, `oasis_free_string`, `oasis_set_vfs_root`, `oasis_add_vfs_file`, `oasis_register_callback`), audio (`oasis_audio_*`, `oasis_set_audio_callback`) and video (`oasis_video_*`, with the `video-decode` / `video-decode-ffmpeg` feature). All internal Rust state is behind the opaque pointer; null handles and strings are checked at every entry point and no panic crosses the boundary.

### 7.3 Render Pipeline

1. `oasis_tick()` advances the OS state and renders: the shader or vector wallpaper (static vector layers are baked once and composited from a cache; animated layers render at a 12 FPS cadence), the SDI scene and the windows, into `Ue5Backend`'s RGBA buffer. Render-stage errors are logged, not discarded.
2. The backend sets the dirty flag.
3. UE5's `TickComponent()` checks `oasis_get_dirty()` -- if true, calls `oasis_get_buffer()` and `UpdateTextureRegions()` to push pixels to the `UTexture2D`.
4. The texture is sampled by a material on the monitor mesh; optional CRT effects (scanlines, curvature, bloom) are applied UE5-side.

### 7.4 Input Pipeline

1. Player aims at in-game monitor, presses interact key; UE5 enters "computer interaction mode" (HUD suppressed, camera locked, input rerouted).
2. Line trace from crosshair hits the monitor mesh; the UV at the hit point is mapped to virtual screen coordinates (e.g. UV(0.5, 0.5) -> (240, 136) on 480x272).
3. Mouse movement -> `CursorMove`, click -> `PointerClick`, keyboard -> `TextInput` / `ButtonPress` / raw `OASIS_EVENT_KEY`.
4. Escape / interact again exits interaction mode.

### 7.5 Gameplay Integration via Callbacks

`oasis_register_callback` lets UE5 react to OS-level events: `OASIS_CB_COMMAND_EXEC` (detail = the command) and `OASIS_CB_APP_LAUNCH` (detail = app title). Other codes are ignored with a warning; the never-fired placeholders `FILE_ACCESS` (1), `LOGIN` (4), `NETWORK_SEND` (5) and `PLUGIN_LOAD` (6) were retired, and their numbers stay unused.

| Callback Event | Trigger | Gameplay Example |
|---------------|---------|-----------------|
| `COMMAND_EXEC` | Player executes a terminal command | Hacking puzzle: run the right sequence to bypass "security" |
| `APP_LAUNCH` | Player launches an app from the dashboard | Open a game-specific interface, launch a mini-game |

Other gameplay hooks (a lore file opened, a key entered) can be built on these two: watch `COMMAND_EXEC` for `cat lore.txt` or `login <key>`, or seed files with `oasis_add_vfs_file`.

### 7.6 Multiple Instances

Each in-game computer is an independent OS instance with its own skin, VFS root, plugin set, and state. A single UE5 level might contain a dozen terminals, each running a different skin with different content. Instances share no state unless explicitly connected (e.g. an in-game "network" where sending a message on one terminal delivers it to another's VFS inbox).

---

## 8. PSP Platform Implementation

The current PSP architecture -- the two-binary split, the GU command buffer, the QuickJS cross-compile, TLS 1.3 and the Media Engine video pipeline -- is documented in [psp-architecture.md](psp-architecture.md), with the overlay PRX in [psp-plugin.md](psp-plugin.md) and [ADR-004](adr/004-psp-two-binary-architecture.md). This section keeps the design constraints and the firmware rationale.

### 8.1 Memory and Resource Constraints

The PSP has 32MB of main RAM (64MB on PSP-2000+), a 333MHz MIPS R4000 CPU, and 2MB of dedicated VRAM.

| Constraint | Impact on Rust Design | Mitigation |
|-----------|----------------------|------------|
| 32MB RAM | Unbounded growth is fatal | Size caps on every network and media buffer (2 MB HTTP bodies, ≤480p video, 1 MB GU command buffer), 8 MiB JS heap |
| No MMU isolation | All threads share address space | Rust ownership prevents data races; unsafe blocks minimized and documented |
| 333MHz CPU | No room for abstraction overhead | Hardware GU sprites, ME hardware video decode, 500 ms JS budget per tick |
| 2MB VRAM | Textures must be carefully managed | VRAM glyph atlas, texture cache |
| No ASLR/stack canaries | Network-facing code is a high-value target | Rust memory safety; bounded parsers (§16) |

### 8.2 Kernel Mode and Privileges

**rust-psp SDK:** OASIS_OS depends on a standalone fork of the rust-psp SDK at [github.com/AndrewAltimit/rust-psp](https://github.com/AndrewAltimit/rust-psp). The upstream project (`github.com/overdrivenpotato/rust-psp`, MIT) lacks kernel mode support; the fork adds kernel-mode modules (`psp::module_kernel!`), std support via a sysroot overlay, edition 2024 modernization, safety fixes, and additional syscall bindings.

The EBOOT runs in user mode, which is sufficient for rendering, input, networking, and file I/O on modern custom firmware. Kernel-mode work (Media Engine boot, syscall hooks, the in-game overlay) lives in separate PRX modules: `oasis-me-boot` (a clean Rust replacement for cooleyesBridge that resolves `sceMeBootStart660`) and the `oasis-plugin-psp` overlay. The original C codebase's hand-written ME and audio-stub assembly (`me.S`, `audiolib.S`) is not needed: audio uses the firmware's `sceMp3*` / `sceAudiocodec`.

### 8.3 Firmware Modernization: 1.50 to 6.60/6.61

The original C codebase was architecturally coupled to PSP firmware 1.50, the only firmware that ran unsigned homebrew without custom firmware. OASIS_OS targets firmware 6.60/6.61 with modern custom firmware (ARK-4, PRO CFW, or ME CFW, made persistent via Infinity 2). This eliminated every firmware-specific hack in the original codebase:

| Original Mechanism (FW 1.50) | OASIS_OS Disposition (FW 6.60/6.61) |
|------------------------|-------------------------------------|
| KXploit directory pairs (`GAMEFOLDER` + `GAMEFOLDER%`) and title watermark stripping | Eliminated: CFW runs unsigned EBOOTs directly from `ms0:/PSP/GAME/` |
| `__attribute__((constructor))` kernel escalation, `pspSdkInstallNoPlainModuleCheckPatch()`, `pspSdkInstallNoDeviceCheckPatch()` | Replaced by CFW-provided kernel access |
| DevHook flash emulation (`devhook.c`, `ms0:/dh/`) | Eliminated: 6.6x has every game API; ISO loading is a CFW feature (Inferno/NP9660) |
| `sceKernelLoadModuleBufferUsbWlan()` | Replaced by `sceKernelLoadModule()`, which loads unsigned PRX on CFW |
| Commented-out WiFi error handling | Restored: 6.6x has a stable WPA2 stack, which is what makes PSP-based remote agent control viable (Section 10) |

Modern firmware also enables 64MB RAM on PSP-2000+ and PSP Go support (`ef0:/` instead of `ms0:/`). Target stack: firmware 6.61, ARK-4 (supports every model incl. Street/E-1000), Infinity 2 or cIPL for persistence. Any CFW that provides kernel-mode access and unsigned code execution on 6.60/6.61 is sufficient.

### 8.4 Overlay Plugin PRX Architecture

OASIS uses a two-binary design on PSP, inspired by IRShell and similar shells that shipped companion PRX modules:

| Binary | Crate | Mode | Purpose |
|--------|-------|------|---------|
| EBOOT.PBP | `oasis-backend-psp` | User | Full shell (dashboard, terminal, browser, apps) |
| PRX | `oasis-plugin-psp` | Kernel | Lightweight overlay + background music, ~153 KB stripped PRX |

The PRX is loaded by custom firmware at boot via a `PLUGINS.TXT` entry and stays resident when games launch. It patches `sceDisplaySetFrameBuf` via `sctrlHENPatchSyscall` and draws its overlay into the game's framebuffer after each frame. It uses static buffers (no heap on the overlay path) and shares state between the display hook and its audio / config threads through atomics. Background audio streams MP3 files from `ms0:/MUSIC/` through the hardware decoder. A kernel watchdog in the PRX also recovers the EBOOT's ME video decode from a firmware deadlock.

---

## 9. Linux / Raspberry Pi Platform

### 9.1 Behavioral Differences

| Behavior | PSP | Raspberry Pi / Linux |
|----------|-----|---------------------|
| App launching | `sceKernelLoadExec` (replaces process) | In-process apps (returns to shell) |
| Multitasking | Single process, multiple threads | Full process isolation |
| File system root | `ms0:/` (Memory Stick) | VFS seeded at boot |
| Power management | `scePowerGetBatteryLifePercent` | `/sys/class/power_supply/` sysfs reads |
| Display | 480x272 fixed, 32-bit color via GU | Configurable resolution via SDL3 |
| Networking | sceNetInet (WiFi only, infrastructure mode) | std::net (Ethernet or WiFi, full stack) |
| Boot flow | Custom firmware loads EBOOT.PBP | systemd service, auto-login to kiosk mode |

### 9.2 Raspberry Pi Deployment

On the Raspberry Pi, OASIS_OS operates as a kiosk-mode application booting directly into the shell interface. Recommended deployment uses Raspberry Pi OS Lite with auto-login to launch the OASIS_OS binary on TTY1.

- **Boot-to-shell time:** target 3-5 seconds on a Pi 5, from power-on to rendered dashboard.
- **Display options:** Official Raspberry Pi touchscreen (800x480), HDMI at configurable resolution, or headless with remote terminal only.
- **Input options:** USB keyboard/mouse, USB gamepad, GPIO-wired buttons, touchscreen (via SDL3 touch events), or remote terminal.
- **Systemd integration:** A `oasis-os.service` unit manages lifecycle, restart-on-crash, and dependency on `network-online.target`.
- **Coexistence with tamper services:** On the briefcase Pi, `oasis-os.service` runs alongside `tamper-sensor.service` and `tamper-gate.service`. OASIS_OS is the user-facing interface; the tamper services are the physical security layer and operate at the systemd level independent of the shell.

---

## 10. PSP Remote Agent Control

> **Status:** Partially implemented. The `listen`, `remote` and `hosts` commands exist (`oasis-terminal/src/remote_commands.rs`), backed by `oasis-net`'s `RemoteListener`, outbound `RemoteClient` (non-blocking, answers the PSK challenge-response) and saved-hosts parser (`hosts.rs`); the desktop and WASM hosts wire `remote` sessions into the terminal. Not yet implemented: VT100 emulation (terminal output only understands SGR foreground colors, `oasis-core/src/ansi.rs`), the PSP backend's `remote` session wiring, and the PSP command palette / d-pad shortcuts described in §10.4.

### 10.1 Concept

A PSP running OASIS_OS on infrastructure WiFi can establish outbound TCP connections to machines running AI agent infrastructure. The remote terminal works bidirectionally -- the PSP is both a server (accepting inbound sessions for its own OS) and a client (initiating outbound sessions to remote hosts). PPSSPP 1.19+ maps `sceNetInet` to real host sockets, so the whole session stack can be developed in the emulator against real network services before deploying to hardware.

```
+----------------------------+          +----------------------------+
|  PSP (OASIS_OS)            |  WiFi    |  Agent Host Machine        |
|  remote dev-server         | -------> |  remote terminal port      |
|    +-- outbound TCP        |  TCP     |    +-- Claude Code CLI     |
|    +-- SGR colors          |          |    +-- CI tooling          |
|    +-- command passthrough |          |    +-- MCP servers         |
+----------------------------+          +----------------------------+
```

### 10.2 Use Cases

| Scenario | How It Works |
|----------|-------------|
| Check agent status from a PSP | `remote briefcase` -> `agent status` |
| Claim board work | `remote briefcase` -> `board query` -> `board claim 142` |
| Trigger CI | `remote dev-server` -> `ci run <stage>` |
| Emergency wipe trigger | `remote briefcase` -> `tamper arm` |

### 10.3 Saved Hosts Configuration

Hosts are read from `/etc/hosts.toml` in the VFS (`oasis_net::parse_hosts`; `port` defaults to 9000, `protocol` to `oasis-terminal`, and an optional `psk` enables authentication):

```toml
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
```

### 10.4 PSP Input Considerations [PLANNED]

| Input Method | Availability | Speed | Use Case |
|-------------|-------------|-------|----------|
| On-screen keyboard (sceUtilityOsk) | All PSP models | Slow | Short commands, passwords |
| USB keyboard | PSP-2000+ only | Full typing speed | Extended terminal sessions |
| D-pad shortcuts | All models | Instant | Mapped to common commands (L+Up = `agent status`) |
| Analog stick cursor | All models | Moderate | Scrolling output, selecting from menus |

A command palette on Triangle -- a scrollable list of frequently used agent commands executed with one press -- is the planned answer to slow text entry.

---

## 11. Virtual File System

### 11.1 VFS Architecture

The virtual file system is the abstraction that makes the same command interpreter work across all platforms. On PSP, `ls` lists Memory Stick contents; in UE5 it lists game-authored content. The decision record, including the shared invariants, is [ADR-002](adr/002-vfs-abstraction.md).

The `Vfs` trait (`oasis-vfs/src/lib.rs`) has `readdir`, `read`, `write`, `stat`, `mkdir`, `remove`, `rename`, `exists`, `get_permissions` / `set_permissions`, an optional `VfsContext` (user identity for permission checks), and two provided methods every implementation shares:

- `check_permission(path, mode)` for Unix-style mode bits.
- `validate_rename(from, to)` rejects renaming `/`, a missing source, a destination inside the source (`/a -> /a/b` used to orphan the subtree), a missing or non-directory destination parent, an existing directory destination, and a directory onto a file; `from == to` is a no-op.

`crates/oasis-vfs/tests/vfs_conformance.rs` runs one suite against all three implementations, plus permission, layering and (unix) symlink tests.

| Backend | Backing Store | Use Case |
|---------|--------------|----------|
| `MemoryVfs` | In-memory tree (`BTreeMap`) | Desktop shell, WASM, tests, ephemeral terminals |
| `GameAssetVfs` | Read-only base layer + writable overlay; deletes hide base entries | UE5 in-game computers (`oasis-ffi`) |
| `RealVfs` | A host directory, jailed: the deepest existing ancestor of every path is canonicalized (lstat-based) and checked against the root, so symlinks cannot escape | Tools and tests; no shipped host mounts it |
| OverlayVFS [PLANNED] | Read-only skin defaults + writable user layer | All platforms |

### 11.2 VFS as IPC

Hosts and apps also use well-known VFS paths as IPC: `oasis_types::ipc` is the one home for the path constants (`/var/audio`, `/var/radio`, `/var/wm`, `/var/tv`, `/var/plugin`, `/var/browser/request`, the Settings `/system/ipc/*` requests and `/system/state/*` mirrors, including `/system/state/net`) and the typed requests (`MediaRequest` (§14), `WmRequest`, `PluginRequest`, `NetState`); apps hand host requests over through `App::drain_requests` (§4.3.2). Plugins use the same mechanism ([plugin-development.md](plugin-development.md)).

### 11.3 Game Asset VFS

The `GameAssetVfs` presents game-authored content as a filesystem that players navigate with standard commands. Each in-game terminal has its own VFS root configured by level designers, for example:

- `/home/user/inbox/message_from_rival.txt` -- lore-delivering text file
- `/var/log/system.log` -- procedurally generated log file with clues for a puzzle
- `/etc/security/access.conf` -- file the player must edit (`echo >>`) to solve a hacking puzzle
- `/tmp/upload/card_data.json` -- file deposited by another player via the in-game network

---

## 12. Development Workflow

### 12.1 Three-Tier Testing Strategy

| Tier | Environment | Tests | Cycle Time |
|------|------------|-------|------------|
| 1 -- Desktop | Native build + headless shell harness | Unit tests, e2e scenarios against the real shell (`oasis_app::harness`), backend conformance, screenshot regression | Seconds |
| 2 -- PPSSPP (container) | PSP build in the PPSSPP container (headless in CI) | GU rendering, PSP input, networking, memory constraints, scripted scenarios via [psp-autorun](psp-autorun.md) | ~5 seconds |
| 3 -- Hardware/UE5 | Real PSP + Raspberry Pi + UE5 editor | WiFi, USB, Media Engine, boot-to-shell, in-game render target | ~30 seconds |

Test layers and the harness API: [testing.md](testing.md). PSP scenarios: [scripts/psp-scenarios.md](../scripts/psp-scenarios.md).

### 12.2 PSP Debugging

Today PSP debugging uses the WiFi TCP command server inside the EBOOT (`scripts/psp_remote.py`: deploy, reboot, remote input injection, live framebuffer capture, file upload, scripted sequences), the network recovery EBOOT (hold R on boot) and PPSSPP headless runs in the `psp` Docker profile.

**Planned: PPSSPP with an embedded MCP server.** A patched PPSSPP container would expose emulator internals as MCP tools (`psp.memory.read/write/search`, `psp.gpu.state/vram_dump/texture_list`, `psp.debug.breakpoint/step/registers/threads/stack_trace`, `psp.net.sockets/capture/inject_latency`, `psp.emu.screenshot/save_state/frame_advance`), built from a pinned PPSSPP commit plus a small patch series under `docker/ppsspp-patches/` (currently a placeholder). It would let an agent inspect GU state, threads and memory of a running EBOOT the way it inspects desktop code. None of it is built yet.

### 12.3 UE5 Development Workflow

1. Build OASIS_OS as a cdylib (`cargo build --profile release-ffi -p oasis-ffi`)
2. Copy the `.dll`/`.so` to the UE5 project's `Binaries/ThirdParty/` directory
3. UE5 Build.cs references the library; a C++ actor component calls the FFI functions
4. Place `AComputerTerminalActor` in a level, assign skin and VFS root via editor properties
5. Play-in-Editor and interact with the in-game terminal

### 12.4 AI Agent Integration

All code in the repository is authored by AI agents under human direction (see [`AGENTS.md`](../AGENTS.md) for the agent roster and CI workflow, and [`CONTRIBUTING.md`](../CONTRIBUTING.md)):

| Agent / tool | Task |
|-------|------|
| Claude Code | Architecture, refactoring, PSP debugging, PR authoring |
| PR review (Claude security + quality review, OpenRouter general review) | Automated review of every PR, with review-response and failure-handler agents that push fixes |
| CI (GitHub Actions + Docker) | fmt, clippy, docs, link check, tests, builds, screenshot regression, PSP EBOOT + PPSSPP smoke test |
| In-app MCP server (`oasis-mcp`) | Lets a local agent drive a running desktop instance ([mcp-server.md](mcp-server.md)) |

---

## 13. Plugin System

OASIS_OS supports runtime-extensible functionality through plugins: Rust types implementing the `Plugin` trait (`oasis-core/src/plugin/traits.rs`), registered with a `PluginManager` and given a `PluginHost` with access to the SDI scene graph, command registry, VFS, and optionally audio, network and the rendering backend. The guide is [plugin-development.md](plugin-development.md).

- **Lifecycle.** Register -> `init` (register commands, create SDI objects) -> `update` once per frame -> `shutdown` on unload.
- **Discovery.** `PluginManager::discover_manifests` reads `plugin.toml` manifests from `/etc/oasis-os/plugins/` in the VFS (each with a typed `[config]` section of `PluginConfigValue` bool / int / float / string entries).
- **Apps.** A plugin can register a dashboard app (`PluginAppRegistration`, needs the app capability; `register_app` returns `Result`). Its tile survives skin swaps.
- **IPC.** VFS paths (the shared `PluginHost::vfs`).
- **Not implemented.** Loading plugins from binaries at runtime (PRX via `sceKernelLoadModule`, `.so` via `libloading`) remains a design goal; today every plugin is compiled in. The PSP overlay PRX (§8.4) is a separate binary, not an `oasis-core` plugin.

---

## 14. Audio System

`oasis-audio` holds the audio logic; backends implement `AudioBackend` (§5.3). The manager, playlists, radio sources, decoders and streaming back-pressure are described in [audio-engine.md](audio-engine.md).

- **Music IPC.** `AudioManager` is a backend-free state machine (playlist, repeat / shuffle, playback state): it turns each `MediaRequest` into a `MediaAction` for the host. The `music` command and the Music Player write typed `MediaRequest` lines to `/var/audio/request`; `oasis_audio::MusicController` carries out the actions on the host's audio backend and publishes `/var/audio/status` (write-on-change) and `/var/audio/position`. The desktop shell (SDL audio) and the WASM backend (`<audio>` element) both run it; the PSP, UE5 / FFI hosts do not service this IPC yet.
- **Internet radio.** `RadioManager` streams Icecast and Internet Archive sources over non-blocking sockets (`is_would_block()` for "no data yet", non-2xx responses rejected).
- **Decoders.** WAV (PCM 8/16-bit) and Ogg Vorbis (symphonia) for sound effects and assets; MP3 on the backends.

---

## 15. UI Toolkit and Accessibility

### 15.1 Widgets

`oasis-ui` provides 27 types implementing the `Widget` trait (`measure` + `draw` against a theme-aware `DrawContext`) plus non-`Widget` helpers (ListView, MenuBar, NinePatch, flex/grid layout, focus management, tweens; easing curves come from `oasis_types::easing`). The catalogue with source paths is [ui-widgets.md](ui-widgets.md).

### 15.2 Accessibility

WCAG contrast math lives in `oasis_types::color` (`relative_luminance()`, `contrast_ratio()`, `meets_wcag_aa()` / `meets_wcag_aa_large()`); `skin lint` and the skin tests use it to check every built-in skin's text/background pairs, and the High Contrast skin is designed to meet WCAG AAA (7:1). Keyboard-only operation goes through `oasis_ui::focus` (Tab / Shift-Tab cycling and arrow-key spatial navigation with a theme-styled focus ring).

---

## 16. Security Considerations

The security model, per-subsystem defenses and known gaps are in [security.md](security.md). Summary:

| Threat | Mitigation |
|--------|-----------|
| Buffer overflow in network input parsing | Rust memory safety; bounded line lengths (1024 B listener, 16 KiB client), bounded MP4 tables |
| Unauthorized remote access | PSK challenge-response (HMAC-SHA256 over a per-connection nonce; the key never crosses the wire); the remote listener binds loopback by default and refuses all-interfaces binds without a PSK; constant-time compare, a 20 s unauthenticated timeout and a server-wide lockout after repeated failures (FTP `PASS` is throttled the same way); the MCP server is loopback-only with Origin/Host checks and an optional bearer token |
| Hostile web content | CSP `script-src` / `style-src` (nonces, sha256 hashes) and `connect-src` enforced; `fetch` blocks more-private targets and enforces CORS; RFC 6265 cookie jar with cookies attached and stored per redirect hop, credentials and `Cookie` / `Authorization` dropped off-origin; web-origin pages cannot load `vfs://`; HTTP/2 frame / header-block / HPACK caps; decompressed bodies capped (8 MB desktop, 2 MB PSP); DOM depth cap and selector-match budget; JS time budget and heap cap per tick |
| VFS escape | Path canonicalization, permission bits, shared rename invariants, `RealVfs` symlink jail, `GameAssetVfs` overlay writes never touch base assets |
| Plugin loading malicious code | Plugins are compiled in; no remote plugin installation |
| Memory corruption via unsafe blocks | Unsafe limited to platform FFI wrappers and documented with `// SAFETY:` comments |
| Man-in-the-middle / eavesdropping | Remote terminal and FTP-like transfer sessions are **not** encrypted (the PSK challenge-response only authenticates): use them on trusted networks or tunnel them over SSH. Outbound HTTPS / Gemini use TLS (rustls on desktop; `embedded-tls` TLS 1.3 on PSP, with optional host pinning because PSP has no usable CA store) |
| UE5 FFI boundary safety | Null checks on every entry point; opaque handles; no panics cross the boundary |
| Briefcase physical compromise | Delegated to the separate tamper-briefcase services (dual-sensor detection, LUKS2, cryptographic wipe) |

---

## 17. Build System and CI/CD

### 17.1 Build Targets

| Target Triple | Toolchain | Output | Deploy Target |
|--------------|-----------|--------|--------------|
| mipsel-sony-psp (custom) | rust-psp + cargo-psp (nightly) | EBOOT.PBP / PRX | PPSSPP / Memory Stick |
| wasm32-unknown-unknown | wasm-pack | `.wasm` + JS glue | GitHub Pages ([demo](https://andrewaltimit.github.io/oasis-os/demo/)) |
| aarch64-unknown-linux-gnu | cross (Docker) or native Pi | ELF binary | Raspberry Pi 5 (briefcase) |
| x86_64-unknown-linux-gnu | Native Rust toolchain | ELF binary / .so | Desktop / UE5 Linux |
| x86_64-pc-windows-msvc | Native Rust toolchain | .exe / .dll | Desktop / UE5 Windows |

Build commands and feature flags are in [getting-started.md](getting-started.md).

### 17.2 CI Pipeline

CI runs on self-hosted runners; every Rust step runs in the `rust-ci` Docker image (pinned to `rust:1.99-slim`), and the PSP steps use the rust-psp revision every PSP crate pins (CI checks the pins). `main-ci.yml` (pushes to `main`) gates on format, clippy with `-D warnings` over all targets and over the supported non-default feature sets, rustdoc with `-D warnings`, the markdown link check, tests (cargo-nextest + doc-tests), the release build, screenshot regression, cargo-deny, benchmarks, the `scripts/test_*.py` PSP tooling unit tests, warning-free PSP EBOOT / overlay PRX / recovery EBOOT builds (the overlay PRX is stripped with `prxmin` and held to a 192 KiB size budget) and a PPSSPP headless smoke test, and then deploys the WASM build to GitHub Pages. `pr-validation.yml` runs a subset on pull requests plus the Claude reviews and auto-fix agents. The step-by-step order is in [CLAUDE.md](../CLAUDE.md#ci-pipeline-order) and the agent workflow in [AGENTS.md](../AGENTS.md#ci-pipeline). Separate workflows run memory analysis (`memory-ci.yml`: ASAN, Valgrind massif), nightly streaming tests (`nightly-streaming.yml`) and fuzzing (`fuzz.yml`).

---

## 18. Migration Strategy from Original C Codebase

The original C source (`psixpsp.7z`, not checked in) contains ~15,000 lines of C. The port followed a phased approach, each phase producing a working, testable binary:

| Phase | Deliverable | Status |
|-------|-----------|--------|
| 1 -- Framework scaffold | SDI core + backend trait + blank screen on SDL3 + UE5 render target stub | **Complete** |
| 2 -- SDI + VFS + Commands | Full scene graph, VFS trait + implementations, command interpreter | **Complete** |
| 3 -- Classic skin | Icon grid dashboard, PBP scanning, app discovery, cursor navigation | **Complete** |
| 4 -- PSP subsystems | Input, power, time, USB, file browser, on-screen keyboard | **Complete** |
| 5 -- Remote terminal | TCP listener + outbound client, authentication, full command suite | **Complete** |
| 6 -- UE5 integration | FFI boundary, UE5 render target backend, interaction input, `GameAssetVfs` | **Complete** |
| 7 -- Window manager | Window lifecycle, drag/resize/focus, hit testing, clipping, snapping, tiling, animations | **Complete** |
| 8 -- Skin system | Skin loading, hot-swap, 17 skins from TOML | **Complete** |
| 9 -- Plugins | Plugin trait, host API, manifests, example plugins | **Complete** (compiled-in plugins) |
| 10 -- Audio | MP3 playback with ME offloading (PSP) or SDL3 (desktop), music IPC, internet radio | **Complete** |
| 11 -- Polish | Transitions, update checker, scripting, file transfer server | **Complete** |
| 12 -- PSP GU backend | Hardware-accelerated sceGu rendering, PSIX-style UI matching desktop layout | **Complete** |

The Rust codebase is about 400,000 lines in 858 source files across 51 crate directories (counted with `git ls-files 'crates/*.rs' | xargs cat | wc -l`, tests included), far beyond the original C because of the framework abstraction, 10 app crates, the browser engine with JS bindings, video decode, vector graphics, shader wallpapers, the window manager, the VFS and UE5 integration. The vendored ffmpeg (~176,000 lines) is gone.

---

## 19. Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| rust-psp SDK fork lacks needed PSP syscall bindings | Medium | Medium | Add bindings directly to the fork; fall back to raw FFI |
| PPSSPP infrastructure networking breaks on edge cases | Medium | Medium | Test on latest PPSSPP; fall back to real hardware |
| PPSSPP headless mode inaccurate vs real PSP | Low | Medium | Validate rendering on real hardware at milestones |
| UE5 texture update latency causes visible lag | Medium | Medium | Only update on dirty frames; profile `UpdateTextureRegions` |
| FFI boundary introduces undefined behavior | Low | High | Fuzz the FFI boundary; validate all pointers; miri for unsafe auditing |
| Memory budget exceeded on PSP | Low | High | Profile in PPSSPP; size caps on every buffer; lazy-load textures |
| Skin hot-swap causes state corruption | Medium | Medium | One rebuild path that keeps VFS, plugin apps and chrome; e2e skin-swap scenarios |
| Backends drift apart in clip / render-target semantics | Medium | Medium | One drawing-state contract (§5.2) pinned by conformance tests |
| UE5 version upgrade breaks FFI linkage | Low | Medium | C ABI only; no C++ mangled symbols; versioned header |
| Scope creep into a general-purpose OS | Medium | Medium | Strict adherence to non-goals |
| WM hit testing slow under high window counts | Medium | Medium | Spatial index if a linear scan exceeds the frame budget |
| Conflict between OASIS_OS and tamper services on Pi | Low | Low | Strict separation via systemd unit ordering |

---

## 20. Success Criteria

| Criterion | Verification Method |
|----------|-------------------|
| OASIS_OS boots to a themed dashboard on real PSP hardware | Visual verification on PSP-2000/3000 with custom firmware |
| The same dashboard runs in a browser | WASM build deployed to GitHub Pages |
| In-game computer in UE5 renders a functional OS with player interaction | Play-in-Editor: interact with terminal, execute commands, browse files |
| Distinct skins render correctly from the same core framework | Screenshot regression over all 17 skins in CI |
| Dashboard discovers and launches apps on each platform | e2e scenarios (desktop), PPSSPP scenarios (PSP), FFI tests |
| Remote terminal accessible via TCP from another machine | e2e remote-terminal scenario; netcat session |
| PSP establishes an outbound remote session to an agent host | `remote dev-server` from OASIS_OS on PSP (or PPSSPP) |
| VFS abstraction behaves identically across implementations | `vfs_conformance` suite against all three |
| Backends render the same pixels for the same calls | `backend_parity` conformance tests (SDL, UE5) |
| UE5 callbacks fire on command execution and app launch | FFI tests |
| FFI boundary passes fuzz testing with no undefined behavior | cargo-fuzz on FFI entry points |
| Window manager supports drag, resize, focus, minimize, maximize, and close | e2e scenarios + `oasis-wm` integration tests |
| Content clipping prevents rendering outside window boundaries | Clip-audit backend in `oasis-test-backend` |
| CI pipeline runs in Docker | All `main-ci.yml` stages pass |

---

## 21. References and Resources

| Resource | URL / Location |
|----------|---------------|
| Original C source (v1.90) | `psixpsp.7z` (not checked in) |
| rust-psp upstream | github.com/overdrivenpotato/rust-psp |
| rust-psp SDK fork | github.com/AndrewAltimit/rust-psp |
| PPSSPP emulator (GPL-2.0+) | github.com/hrydgard/ppsspp |
| PPSSPP infrastructure networking PR | github.com/hrydgard/ppsspp/pull/19827 |
| MCP specification | modelcontextprotocol.io |
| PSP SDK documentation | psp-archive.github.io/pspsdk-docs/ |
| PSP homebrew wiki | pspdev.github.io/ |
| Unreal Engine 5 C++ API | docs.unrealengine.com |
| Rust FFI guide | doc.rust-lang.org/nomicon/ffi.html |
| SDL3 Rust bindings | github.com/revmischa/sdl3-rs |
| ARK-4 custom firmware | github.com/PSP-Archive/ARK-4 |
| Documentation index | [`docs/README.md`](README.md) |
| Agent workflow and CI | [`AGENTS.md`](../AGENTS.md) |
| Developer's Journal | [andrewaltimit.github.io/oasis-os/journal](https://andrewaltimit.github.io/oasis-os/journal/) |

---

*End of Document*
