# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

OASIS_OS is an embeddable operating system framework in Rust (edition 2024). It provides a skinnable shell with a scene-graph UI, command interpreter, virtual file system, browser engine (HTML/CSS/Gemini), plugin system, and remote terminal. It renders to any pixel buffer + input stream. Built from scratch in Rust starting early 2026, inspired by PSP homebrew shells like PSIX. Seventeen built-in skins are implemented; 14 of them also ship as external TOML files in `skins/` (the other 3 — `corrupted`, `desktop`, `modern` — are built-in only).

Default virtual resolution is 480x272 (PSP native). Skins may override this (e.g. modern=800x600, xp=1024x768); the backend canvas/window scales to match.

## Build Commands

All CI commands run inside Docker containers. For local development you can run cargo directly (SDL3 is compiled from source automatically via the `build-from-source` feature), or use the Docker wrapper.

```bash
# Build (desktop). Default features need system ffmpeg dev libs (video-decode-ffmpeg);
# without them (e.g. local Windows) use the openh264 fallback:
cargo build --release -p oasis-app
cargo build -p oasis-app --no-default-features --features javascript,video-decode

# Build via Docker (matches CI exactly)
docker compose --profile ci run --rm rust-ci cargo build --workspace --release

# Run tests
cargo test --workspace

# Run a single test
cargo test --workspace -- test_name

# Run tests in a specific crate
cargo test -p oasis-core

# Format check
cargo fmt --all -- --check

# Apply formatting
cargo fmt --all

# Lint (CI treats warnings as errors)
cargo clippy --workspace -- -D warnings

# Lint exactly as CI does — ALWAYS run this before opening a PR. The CI image
# pins a specific Rust version (rust:1.93-slim), so a locally newer/older
# toolchain can pass clippy while CI fails (lints change between releases).
docker compose --profile ci run --rm rust-ci cargo clippy --workspace -- -D warnings

# License/advisory audit
cargo deny check

# Build PSP backend (excluded from workspace, requires nightly + cargo-psp)
cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo psp --release

# Build PSP overlay plugin PRX (excluded from workspace, kernel mode)
cd crates/oasis-plugin-psp && RUST_PSP_BUILD_STD=1 cargo psp --release

# Build UE5 FFI shared library
cargo build --release -p oasis-ffi

# Build WASM backend (requires wasm-pack)
./scripts/build-wasm.sh          # debug build
./scripts/build-wasm.sh --release # release (smaller + faster)
# Serve: python3 -m http.server 8080 → http://localhost:8080/www/

# Take screenshots
cargo run -p oasis-app --bin oasis-screenshot
```

## CI Pipeline Order

format check -> clippy -> nightly clippy (advisory) -> doc build -> markdown link check -> test -> release build -> screenshot regression -> cargo-deny -> benchmarks -> PSP EBOOT build -> PPSSPP headless test -> code coverage -> GitHub Pages deploy (WASM)

All steps run via `docker compose --profile ci run --rm rust-ci`.

**Memory analysis** (`memory-ci.yml`, separate non-blocking workflow):
- **ASAN** (AddressSanitizer) -- catches use-after-free, buffer overflow, leaks (~2x slowdown)
- **Valgrind massif** -- heap profiling with peak memory assertions for video decode

Both jobs use `continue-on-error: true` and run on pushes to main, PRs touching `crates/oasis-video/**` or `crates/oasis-core/**`, and `workflow_dispatch`.

**Nightly streaming** (`nightly-streaming.yml`, separate workflow):
- End-to-end streaming validation for TV Guide video playback

## Architecture

### Crate Dependency Graph

```
oasis-types     (foundation: Color, Button, InputEvent, backend traits, error types, geometry)
├── oasis-vfs        (virtual file system: MemoryVfs, RealVfs, GameAssetVfs)
├── oasis-platform   (platform service traits: Power, Time, USB, Network, OSK)
├── oasis-sdi        (scene display interface: named object registry, z-order)
├── oasis-net        (TCP networking, PSK auth, remote terminal, FTP)
├── oasis-mcp        (optional MCP control server: Streamable HTTP + JSON-RPC, agent tool dispatch)
├── oasis-audio      (audio manager, playlist, MP3 ID3 parsing)
├── oasis-ui         (29 Widget impls + ListView/MenuBar/NinePatch/icons/flex+grid layout)
├── oasis-wm         (window manager: drag/resize, hit testing, decorations, snapping, tiling, animations)
├── oasis-skin       (TOML skin engine, 17 built-in skins, theme derivation)
├── oasis-terminal   (~95 commands + 18 shell builtins, line editing, completion, jobs, scripting)
├── oasis-browser    (HTML/CSS/Gemini: DOM, CSS cascade+@media, layout engine, @font-face web fonts (fontdue TTF/OTF rasterizer, font registry, glyph texture cache), full 2D CSS transforms + 3D transform functions (rotateX/Y/Z, translate3d, scale3d, rotate3d, matrix3d, perspective()) with screen-space perspective projection from ancestor `perspective:`, transform-style: preserve-3d propagation, transform-origin Z, perspective-origin, and backface-visibility culling, Canvas 2D path API, SVG paths/groups, light compositor, z-index stacking contexts, nested scroll containers, form elements with select dropdown + label association, hover-triggered CSS transitions, soft hyphens, bidi text, JS DOM bindings)
├── oasis-js         (JavaScript engine: QuickJS-NG via rquickjs on all backends incl. PSP)
├── oasis-video      (MP4/H.264+AAC decode; StreamingBuffer sliding-window; features: h264, ffmpeg, no-std-demux)
├── oasis-vector     (vector graphics: scene graph, path ops, icons, frame-driven animations)
├── oasis-shader     (animated shader wallpapers: Shadertoy-style fragment shaders)
├── oasis-rasterize  (software rasterizer for CPU-side rendering)
├── oasis-i18n       (internationalization support)
├── oasis-test-backend (mock/recording backends for testing)
├── oasis-app-core   (shared app framework: `App` trait, `AppAction`, render helpers)
├── oasis-app-games  (Games app)
├── oasis-app-paint  (Paint app)
├── oasis-app-text-editor (Text Editor app)
├── oasis-app-calculator  (Calculator app)
├── oasis-app-media       (Music Player + Photo Viewer apps)
├── oasis-app-tv-guide    (TV Guide app)
├── oasis-app-radio       (Internet Radio app)
├── oasis-app-settings    (Settings app)
├── oasis-app-file-manager (File Manager app)
├── oasis-app-browser     (Browser launcher placeholder)
├── oasis-app-network     (Network status app)
├── oasis-app-package-manager (Package Manager app)
├── oasis-app-system-monitor  (System Monitor app)
└── oasis-core       (coordination only: dashboard, agent, plugin, script, terminal; all apps live in oasis-app-* crates)
    ├── oasis-backend-sdl  (SDL3 desktop/Pi rendering + input + audio)
    │   └── oasis-app      (binaries oasis-app, oasis-screenshot; features video-decode-ffmpeg (default) / video-decode (openh264), skin-dev, mcp)
    ├── oasis-backend-wasm (Canvas 2D + DOM input + Web Audio; iframe overlay drives a Browser pane and a YouTube embed; YouTube search + thumbnail grid via Invidious; feature: wasm-youtube)
    ├── oasis-backend-ue5  (software RGBA framebuffer for Unreal Engine 5)
    │   └── oasis-ffi      (cdylib C-ABI for UE5 integration; features video-decode / video-decode-ffmpeg)
    ├── oasis-backend-psp  (in the workspace `exclude` list, PSP hardware; oasis-video[no-std-demux])
    └── oasis-plugin-psp   (standalone, not in the workspace; kernel-mode PRX overlay)
oasis-usb-host  (standalone binary, rusb only: host side of the PSP USB thin client)
```

### PSP (target-specific architecture)

PSP has meaningful enough constraints that the detail lives in
[`docs/psp-architecture.md`](docs/psp-architecture.md). Summary:

- **Two binaries** — `oasis-backend-psp` (EBOOT.PBP, the shell) and
  `oasis-plugin-psp` (PRX overlay loaded by CFW, <64 KB, no
  `oasis-core` dependency). See [ADR 004](docs/adr/004-psp-two-binary-architecture.md).
- **GU command buffer (1 MB)** overflows with dense pages. Don't
  double-start GU frames after `swap_buffers_inner()`.
- **QuickJS-NG** built through pspdev's `psp-gcc` with `-msingle-float`,
  linked via `psp-ld`, libc provided by
  `oasis-backend-psp/src/quickjs_shim.rs` — see
  [`docs/javascript-engine.md`](docs/javascript-engine.md).
- **TLS 1.3** via `embedded-tls` + `UnsecureProvider` because PSP
  firmware ships 2008-vintage CAs.
- **Video streaming** via ME hardware decode (`sceMpeg` NAL direct
  path) + `sceAudiocodec`, ≤480p only. See
  [`docs/video-streaming.md`](docs/video-streaming.md).

### Desktop Video Streaming

Progressive streaming via `StreamingBuffer` (sliding-window over an
`Arc<StreamingInner>` while symphonia decodes from the same buffer
via `Read + Seek`). Probe-mode reads return zeros, deferred tail
probe for moov-at-end files, CDN failover through the archive.org
URL for fresh 302 redirects. Full details in
[`docs/video-streaming.md`](docs/video-streaming.md).

### Key Abstraction: Backend Traits

`oasis-types/src/backend/` defines the only abstraction boundary between core and platform (re-exported by `oasis-core`):
- `SdiCore` -- required rendering (13 methods: init, clear, blit, fill_rect, draw_text, swap_buffers, load_texture, destroy_texture, set_clip_rect, reset_clip_rect, measure_text, read_pixels, shutdown)
- `SdiBackend` -- a marker super-trait with no methods of its own. A blanket impl satisfies it for any type that implements `SdiCore` plus all nine extension traits: `SdiShapes`, `SdiGradients`, `SdiAlpha`, `SdiText`, `SdiTextures`, `SdiClipTransform`, `SdiVector`, `SdiBatch`, `SdiRenderTarget`. The extensions live under `oasis-types/src/backend/extensions/` (one file per trait, re-exported via `extensions/mod.rs`) and total 53 default-impl methods (shapes, gradients, alpha/blend, text styling, texture management, clipping/transforms, vector path ops, batched submission, offscreen render targets); two further optional traits, `SdiBlendMode` and `SdiGeometry`, are not part of `SdiBackend`. `SdiBatch` in particular provides `begin_batch`/`flush_batch` plus `submit_rect_batch`/`submit_text_batch` for batched rect and text geometry submission (backends can override with GPU geometry calls).
- `InputBackend` -- input polling (returns `Vec<InputEvent>`)
- `NetworkBackend` -- TCP networking
- `AudioBackend` -- audio playback

Core code never calls platform APIs directly. All platform interaction goes through these traits.

### Core Modules

The framework lives in 50 crate directories under `crates/`:
- **40 workspace members** (the desktop / WASM / UE5 build surface — what `cargo build --workspace` compiles)
- **1 explicitly excluded crate** (`oasis-backend-psp`, on the `mipsel-sony-psp` target)
- **9 standalone PSP-target crates** that are not mentioned in the workspace `Cargo.toml` at all and are built individually via `cargo psp`: `oasis-plugin-psp`, `oasis-devloop-psp`, `oasis-me-boot`, `oasis-prx-decrypt-psp`, `oasis-recovery-psp`, `oasis-usb-client-psp`, `oasis-usb-debug-psp`, `oasis-usb-trace-psp`, `oasis-usb-vbus-psp`

Each module below is its own crate (previously all in oasis-core):

- **oasis-types** -- Foundation types: `Color`, `Button`, `InputEvent` (incl. raw `Key` + `Modifiers` events, `Key::legacy_press` gamepad-twin table, `KeyTwinFilter`), backend traits (`SdiCore`, `SdiBackend`, `InputBackend`, `NetworkBackend`, `AudioBackend`), error types, TLS, bitmap font metrics, `geometry.rs` (shared shape algorithms)
- **oasis-sdi** -- Scene Display Interface: named objects with position, size, color, texture, text, z-order, gradients, rounded corners, shadows
- **oasis-skin** -- Data-driven TOML skin system with 17 built-in skins (14 of which also ship as external TOML in `skins/`; the other 3 — `corrupted`, `desktop`, `modern` — are built-in only). Theme derivation from 9 base colors.
- **oasis-browser** -- Embeddable HTML/CSS/Gemini rendering engine: WHATWG HTML, full CSS cascade with `@media` / `@container` / `@layer` / `@supports` / `@scope` / CSS Nesting / `:has()`, viewport-aware stylesheet parsing, CSS Grid, `var()` substituted at computed-value time, 2D + 3D transforms, `@font-face` web fonts, canvas + SVG, HTTP/1.1 + HTTP/2 over rustls (8 MB decompressed-body cap), cookies, CSP, origin-checked `fetch` + per-origin quota'd Web Storage (JS glue in `src/js_dom/`), reader mode, forms, bookmarks. **Feature catalogue:** [`docs/browser-engine.md`](docs/browser-engine.md). **Backlog:** [`docs/browser-backlog.md`](docs/browser-backlog.md).
- **oasis-js** -- QuickJS-NG via `rquickjs` on every target (desktop / WASM / UE5 / PSP). DOM bindings (`getElementById`, `querySelector`, `fetch`, `setTimeout`, `localStorage`, event bubbling, …) feature-gated as `javascript`. An execution watchdog (`JsEngine::with_context_guarded`, bounded microtask drains, timer limits) keeps runaway scripts from freezing the shell. PSP cross-compile is non-trivial (pspdev toolchain, `-msingle-float`, `psp-ld`, hand-rolled libc shim) — see [`docs/javascript-engine.md`](docs/javascript-engine.md).
- **oasis-ui** -- 29 types implementing `Widget` (Button, Card, TabBar, Panel, InputField, ScrollView, ProgressBar, Toggle, Accordion, Avatar, Badge, Checkbox, ColorPicker, ContextMenu, DatePicker, Divider, Dropdown, Modal, RadioGroup, RichText, Slider, SpinBox, Spinner, SplitPane, Table, TextBlock, ToastStack, Tooltip, TreeView) plus ListView, MenuBar, NinePatch, IconAtlas, flex/grid layout, focus and animation helpers. Catalogue: [`docs/ui-widgets.md`](docs/ui-widgets.md)
- **oasis-vfs** -- Virtual file system: `MemoryVfs` (in-RAM), `RealVfs` (disk), `GameAssetVfs` (UE5 with overlay writes)
- **oasis-terminal** -- Command interpreter: ~95 registered commands (core, platform, remote, network, audio, radio, skin, text, file, system, dev, fun, UI, security, doc; plus agent/plugin/script/transfer/update/tv/browser registered by oasis-core) and 18 shell builtins. Shell features: variables, globs, aliases, piping, one script engine (`script.rs`: if/while/until/for/case) for inline and `run` scripts, `cmd &` jobs, and a `ShellSession` (`session.rs`) with readline editing, tab completion and persistent history
- **oasis-wm** -- Window manager (window configs, hit testing, drag/resize, minimize/maximize/close, edge snapping, tiling, open/close/minimize animations). See [`docs/window-manager.md`](docs/window-manager.md)
- **oasis-net** -- TCP networking with PSK authentication, remote terminal, FTP transfer
- **oasis-mcp** -- Optional MCP control server (off by default). Minimal Streamable-HTTP transport + JSON-RPC 2.0 / MCP dispatch over a non-blocking `NetworkStream`; app-agnostic (depends only on `oasis-types`) with a `ToolDispatcher` callback the app implements in `crates/oasis-app/src/mcp_tools.rs`. Lets an on-device agent drive the shell. See [`docs/mcp-server.md`](docs/mcp-server.md)
- **oasis-audio** -- Audio manager with playlist, shuffle/repeat modes, MP3 ID3 tag parsing
- **oasis-platform** -- Platform service traits: PowerService, TimeService, UsbService, NetworkService, OskService
- **oasis-video** -- MP4 / H.264 + AAC decode pipeline. Feature flags: `h264` (openh264 + symphonia demux/AAC), `ffmpeg` (statically linked ffmpeg; mutually exclusive with `h264`), `no-std-demux` (PSP-safe `demux_lite::Mp4Lite`). `video-decode` / `video-decode-ffmpeg` are features of `oasis-app` / `oasis-ffi` that select `h264` / `ffmpeg`. Desktop progressive streaming + PSP in-memory ME-hardware pipeline are documented in [`docs/video-streaming.md`](docs/video-streaming.md) and [`docs/psp-architecture.md`](docs/psp-architecture.md) §Video Streaming.
- **oasis-vector** -- Resolution-independent vector graphics: scene graph with path-based drawing operations (fill, stroke, arcs, beziers), Altimit-style dashboard icons, and frame-driven animations. Integrates via `SdiBackend` vector graphics trait extensions
- **oasis-shader** -- Animated shader wallpapers: Shadertoy-style fragment shaders (voronoi, city lights, ocean waves, calm waves, Balatro)
- **oasis-app-core** -- Shared app framework: the `App` trait (input, `handle_key`, `accepts_text`, `tick`, `wants_frame`, `apply_vfs_ops`, SDI + windowed rendering), `AppAction`, render helpers. Guide: [`docs/writing-apps.md`](docs/writing-apps.md)
- **oasis-app-*** -- 13 extracted app crates: `oasis-app-games`, `oasis-app-paint`, `oasis-app-text-editor`, `oasis-app-calculator`, `oasis-app-media` (Music Player + Photo Viewer), `oasis-app-tv-guide`, `oasis-app-radio`, `oasis-app-settings`, `oasis-app-file-manager`, `oasis-app-browser`, `oasis-app-network`, `oasis-app-package-manager`, `oasis-app-system-monitor`
- **oasis-rasterize** -- Shared software rasterizer (span/row fast paths) used by the UE5, SDL and WASM backends
- **oasis-i18n** -- Internationalization framework
- **oasis-test-backend** -- Mock and recording `SdiBackend` implementations for tests
- **oasis-usb-host** -- Standalone host binary (rusb) that streams frames to / reads input from the PSP USB thin client (`oasis-usb-client-psp`)
- **oasis-core** -- Coordination layer: dashboard, agent/MCP, plugin, scripting, status/bottom bars, desktop taskbar, in-core Terminal app. All other apps live in `oasis-app-*` crates; `oasis-core` no longer ships any app implementations beyond the Terminal delegate (which the desktop input pipeline pumps `set_lines` into) and the `SimpleApp` placeholder used by the plugin system for dynamically registered apps.

### Font Rendering

Proportional bitmap font rendering from glyph ink bounds. `oasis-types` provides `glyph_advance()` with variable per-character widths (3-8px). Each backend has its own glyph table in `font.rs`. The PSP backend additionally uses system TrueType fonts via `psp::font` with a VRAM glyph atlas. No external font dependencies for desktop/UE5.

### FFI Boundary (oasis-ffi)

Exports C-ABI functions: `oasis_create`, `oasis_create_full`, `oasis_destroy`, `oasis_tick`, `oasis_send_input`, `oasis_get_buffer`, `oasis_get_dirty`, `oasis_send_command`, `oasis_free_string`, `oasis_set_vfs_root`, `oasis_register_callback`, `oasis_add_vfs_file`; audio: `oasis_audio_load`, `oasis_audio_play`, `oasis_audio_pause`, `oasis_audio_resume`, `oasis_audio_stop`, `oasis_audio_set_volume`, `oasis_audio_get_volume`, `oasis_audio_is_playing`, `oasis_set_audio_callback`; video (feature `video-decode`/`video-decode-ffmpeg`): `oasis_video_play`, `oasis_video_stop`, `oasis_video_next_frame`, `oasis_video_get_audio`, `oasis_video_is_playing`. This is how UE5 (or any C-compatible host) embeds OASIS_OS. Reference: [`docs/ffi-integration.md`](docs/ffi-integration.md).

## Code Conventions

- MSRV: 1.91.0 (uses `str::floor_char_boundary`)
- Max line width: 100 characters
- Clippy warnings are CI errors (`-D warnings`)
- Workspace lints: `clone_on_ref_ptr`, `dbg_macro`, `todo`, `unimplemented` = warn; `unsafe_op_in_unsafe_fn` = warn; `unwrap_used` = deny
- All unsafe blocks require `// SAFETY:` comments
- Unit tests are in-module (`#[cfg(test)] mod tests`); add new tests there. A few crates also have `tests/` integration suites (oasis-browser, oasis-terminal, oasis-test-backend, oasis-vfs, oasis-wm) and criterion `benches/`
- Dual-licensed: Unlicense + MIT

## Docker Services

`docker-compose.yml` profiles:
- `ci` -- rust-ci container (rust:1.93-slim + cmake + X11/audio dev libs + nightly + cargo-deny)
- `psp` -- PPSSPP emulator (multi-stage build, NVIDIA GPU passthrough)
- `services` -- MCP server containers (code-quality, content-creation, gemini, etc.)

## Document Index

Key documentation files for agents and contributors. Read these for deeper context on specific topics rather than loading everything into every conversation.

### Architecture & Design
- [`docs/README.md`](docs/README.md) -- Index of every document under `docs/`
- [`docs/design.md`](docs/design.md) -- Technical design document v2.6 (~1400 lines, comprehensive architecture)
- [`docs/browser-engine.md`](docs/browser-engine.md) -- Browser feature catalogue (HTTP, HTML, CSS, layout, fonts, chrome, JS bindings)
- [`docs/javascript-engine.md`](docs/javascript-engine.md) -- QuickJS-NG integration and PSP cross-compile
- [`docs/psp-architecture.md`](docs/psp-architecture.md) -- PSP two-binary split, GU, TLS 1.3, ME video decode
- [`docs/video-streaming.md`](docs/video-streaming.md) -- Desktop `StreamingBuffer` progressive playback
- [`docs/boot-splash.md`](docs/boot-splash.md) -- Functional boot splash: BIOS phase probes, splash-phase warm-up, `BootSplash` API
- [`docs/adr/001-arena-based-dom.md`](docs/adr/001-arena-based-dom.md) -- ADR: Arena-based DOM allocation
- [`docs/adr/002-vfs-abstraction.md`](docs/adr/002-vfs-abstraction.md) -- ADR: Virtual file system design
- [`docs/adr/003-backend-trait-design.md`](docs/adr/003-backend-trait-design.md) -- ADR: Backend trait hierarchy
- [`docs/adr/004-psp-two-binary-architecture.md`](docs/adr/004-psp-two-binary-architecture.md) -- ADR: PSP EBOOT + PRX split
- [`docs/adr/005-toml-skin-system.md`](docs/adr/005-toml-skin-system.md) -- ADR: TOML skin engine

### Guides
- [`docs/getting-started.md`](docs/getting-started.md) -- Getting started guide
- [`docs/writing-apps.md`](docs/writing-apps.md) -- `App` trait lifecycle, hooks, idle-frame rules, minimal example app
- [`docs/ui-widgets.md`](docs/ui-widgets.md) -- `oasis-ui` widget catalogue
- [`docs/window-manager.md`](docs/window-manager.md) -- `WindowManager` API, snapping, shortcuts, animations
- [`docs/wasm-backend.md`](docs/wasm-backend.md) -- WASM build, serving, input mapping, iframe overlay
- [`docs/adding-commands.md`](docs/adding-commands.md) -- How to add terminal commands
- [`docs/terminal-commands.md`](docs/terminal-commands.md) -- Catalog of built-in terminal commands grouped by module
- [`docs/skin-authoring.md`](docs/skin-authoring.md) -- Skin creation with full TOML reference
- [`docs/plugin-development.md`](docs/plugin-development.md) -- Plugin development guide
- [`docs/ffi-integration.md`](docs/ffi-integration.md) -- UE5 / C-ABI integration guide
- [`docs/psp-plugin.md`](docs/psp-plugin.md) -- PSP kernel plugin (PRX) documentation
- [`docs/browser-backlog.md`](docs/browser-backlog.md) -- Browser engine backlog and roadmap
- [`docs/theming-desktop-plan.md`](docs/theming-desktop-plan.md) -- Advanced theming + PSIX-style desktop metaphor plan (asset pipeline, free icons, shaped chrome, perf)

### Subsystems
- [`docs/oasis-js.md`](docs/oasis-js.md) -- JavaScript engine desktop API + DOM bindings catalog (PSP cross-compile lives in `javascript-engine.md`)
- [`docs/networking.md`](docs/networking.md) -- TCP transport, PSK auth, remote terminal protocol, TLS, FTP-like file transfer
- [`docs/mcp-server.md`](docs/mcp-server.md) -- Optional MCP control server: enabling, transport, tool catalog, security, agent (Claude Code) integration
- [`docs/audio-engine.md`](docs/audio-engine.md) -- AudioManager, playlist/repeat/shuffle, radio sources, streaming back-pressure
- [`docs/vector-graphics.md`](docs/vector-graphics.md) -- VectorOp/VectorScene, icon catalog, AnimClock, SdiVector integration
- [`docs/shaders.md`](docs/shaders.md) -- Built-in shader wallpapers, GPU + software backends, extension guide

### Operations
- [`docs/psp-autorun.md`](docs/psp-autorun.md) -- Boot-time script runner for deterministic PSP/PPSSPP integration tests
- [`docs/troubleshooting.md`](docs/troubleshooting.md) -- Troubleshooting common issues
- [`docs/security.md`](docs/security.md) -- Security policy and advisories
- [`AGENTS.md`](AGENTS.md) -- Multi-agent system configuration and CI workflow
- [`CONTRIBUTING.md`](CONTRIBUTING.md) -- Contribution policy (AI-authored only)
- [`scripts/psp-scenarios.md`](scripts/psp-scenarios.md) -- PSP test scenario documentation
