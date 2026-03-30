# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

OASIS_OS is an embeddable operating system framework in Rust (edition 2024). It provides a skinnable shell with a scene-graph UI, command interpreter, virtual file system, browser engine (HTML/CSS/Gemini), plugin system, and remote terminal. It renders to any pixel buffer + input stream. Built from scratch in Rust starting early 2026, inspired by PSP homebrew shells like PSIX. Eighteen skins are implemented (14 external TOML skins, 18 built-in; external skins also have built-in equivalents).

Default virtual resolution is 480x272 (PSP native). Skins may override this (e.g. modern=800x600, xp=1024x768); the backend canvas/window scales to match.

## Build Commands

All CI commands run inside Docker containers. For local development you can run cargo directly (SDL3 is compiled from source automatically via the `build-from-source` feature), or use the Docker wrapper.

```bash
# Build (desktop)
cargo build --release -p oasis-app

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

# License/advisory audit
cargo deny check

# Build PSP backend (excluded from workspace, requires nightly + cargo-psp)
cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo +nightly psp --release

# Build PSP overlay plugin PRX (excluded from workspace, kernel mode)
cd crates/oasis-plugin-psp && RUST_PSP_BUILD_STD=1 cargo +nightly psp --release

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
├── oasis-audio      (audio manager, playlist, MP3 ID3 parsing)
├── oasis-ui         (32 widgets: Button, Card, TabBar, ListView, flex layout, etc.)
├── oasis-wm         (window manager: drag/resize, hit testing, decorations)
├── oasis-skin       (TOML skin engine, 18 skins, theme derivation)
├── oasis-terminal   (90+ commands across 17+ modules, shell features)
├── oasis-browser    (HTML/CSS/Gemini: DOM, CSS cascade+@media, layout engine, full 2D CSS transforms, Canvas 2D path API, SVG paths/groups, light compositor, z-index stacking contexts, nested scroll containers, form elements with select dropdown + label association, hover-triggered CSS transitions, soft hyphens, bidi text, JS DOM bindings)
├── oasis-js         (JavaScript engine: QuickJS-NG runtime, console API)
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

### PSP Two-Binary Architecture

The PSP deployment uses two binaries:
- **`oasis-backend-psp`** (EBOOT.PBP) -- the full shell application, runs standalone
- **`oasis-plugin-psp`** (PRX) -- lightweight companion module loaded by CFW (ARK-4/PRO) via `PLUGINS.TXT`, stays resident in kernel memory alongside games

The PRX hooks `sceDisplaySetFrameBuf` to draw overlay UI into the game's framebuffer and claims a PSP audio channel for background MP3 playback. No dependency on oasis-core -- direct framebuffer rendering only (<64KB binary).

### PSP GU Rendering Constraints

The PSP GU (Graphics Unit) has a fixed-size command buffer (`DISPLAY_LIST`, 1 MB in BSS). Each `fill_rect`, glyph blit, clip push/pop, and blend mode change appends commands. Browser pages with many elements can generate hundreds of KB of GU commands per frame.

**Critical rules:**
- **Never call `reinit_gu_frame()` after `swap_buffers_inner()`** — `swap_buffers_inner` already starts a new GU frame via `sceGuStart`. A second `sceGuStart` without `sceGuFinish` corrupts the command buffer and hangs `sceGuSync` on the next frame. Only use `reinit_gu_frame()` after utility dialogs (OSK, `psp::dialog`) which run their own GU frames.
- **`std::time::Instant` crashes on PSP Allegrex** (confirmed by testing) — browser `tick()` is not called on PSP. Image loading happens synchronously during `navigate_vfs` instead of progressively per-frame.

### PSP TLS 1.3

The PSP firmware's built-in SSL uses root CAs from 2008 and SSL 3.0, which cannot connect to modern HTTPS servers. The PSP backend implements native TLS 1.3 via `embedded-tls` (pure Rust, no C/asm) with `UnsecureProvider` (no certificate validation). The `alloc` feature is required to advertise RSA signature schemes (archive.org uses RSA certs). Raw TCP sockets (`sceNetInet*`) are wrapped with `embedded_io::Read + Write` adapters. RNG seeded from `sceKernelGetSystemTimeLow` (not `mfc0 $9` which is privileged on PSP Allegrex). DNS resolution via `psp::net::resolve_hostname` with `to_ne_bytes()` (network byte order fix for little-endian MIPS). HTTP→HTTPS redirect loops are detected automatically, triggering TLS fallback; HTTPS redirects (archive.org → CDN node) are followed within the TLS path. This enables HTTPS downloads for TV Guide video streaming from servers that enforce TLS.

### PSP Video Streaming

TV Guide on PSP uses in-memory streaming (no disk I/O). The I/O thread downloads HTTP(S) data, buffers the MP4 `moov` atom (~1-3MB), parses track tables via `demux_lite::Mp4Lite`, then extracts interleaved audio/video samples from the `mdat` stream in file-offset order. Video samples are skipped (H.264 decode requires real ME hardware, not available on PPSSPP). Audio AAC frames are decoded via `sceAudiocodec` hardware and output through `AudioChannel::output_blocking`. Backpressure is applied via retry-with-sleep when the audio command queue is full, naturally throttling the download to real-time playback speed.

### Desktop Video Streaming

TV Guide on desktop uses in-process progressive streaming via `StreamingBuffer` (in `tv_controller.rs`). A background download thread feeds an `Arc<StreamingInner>` sliding-window buffer while symphonia decodes from the same buffer via `Read + Seek`. Key mechanisms:

- **`probe_mode`** — During symphonia's probe phase, reads return zeros so mdat body is skipped instantly. `decoder_pos` is NOT updated during probe to prevent a throttle deadlock.
- **Deferred tail probe** — A separate thread fetches the last 8MB for moov-at-end files, but only launches after >8MB body data received without finding moov. Prevents CDN connection throttling.
- **`should_throttle()`** — Backpressure: `decoder_pos > 0 ? received > decoder_pos + 16MB : has_moov && buf_size > 16MB`
- **CDN failover** — Range requests route through the original archive.org URL (not cached CDN) to get a fresh 302 redirect, avoiding 401 errors from stale CDN nodes. `open_range_connection()` follows redirect chains.
- **Prebuffer gate** — Decoder waits for MIN_PREBUFFER (2MB) of body data before seeking, preventing reads into empty buffer regions.
- **Seek restart** — After probe discovers moov, the download restarts from the estimated byte offset via a Range request. Linear interpolation: `(seek_secs / duration) * file_size`.

### Key Abstraction: Backend Traits

`oasis-types/src/backend.rs` defines the only abstraction boundary between core and platform (re-exported by `oasis-core`):
- `SdiCore` -- required rendering (13 methods: init, clear, blit, fill_rect, draw_text, swap_buffers, load_texture, destroy_texture, set_clip_rect, reset_clip_rect, measure_text, read_pixels, shutdown)
- `SdiBackend` -- extends `SdiCore` with 39 optional accelerated primitives (shapes, gradients, text styling, batching, vector graphics path operations). Also split into 8 focused extension traits: `SdiShapes`, `SdiGradients`, `SdiAlpha`, `SdiText`, `SdiTextures`, `SdiClipTransform`, `SdiVector`, `SdiBatch`. `SdiBatch` provides `begin_batch`/`flush_batch` plus `submit_rect_batch`/`submit_text_batch` for batched rect and text geometry submission (backends can override with GPU geometry calls)
- `InputBackend` -- input polling (returns `Vec<InputEvent>`)
- `NetworkBackend` -- TCP networking
- `AudioBackend` -- audio playback

Core code never calls platform APIs directly. All platform interaction goes through these traits.

### Core Modules

The framework is split into 37 crates (35 workspace members + 2 excluded PSP crates). Each module below is its own crate (previously all in oasis-core):

- **oasis-types** -- Foundation types: `Color`, `Button`, `InputEvent`, backend traits (`SdiCore`, `SdiBackend`, `InputBackend`, `NetworkBackend`, `AudioBackend`), error types, TLS, bitmap font metrics, `geometry.rs` (shared shape algorithms)
- **oasis-sdi** -- Scene Display Interface: named objects with position, size, color, texture, text, z-order, gradients, rounded corners, shadows
- **oasis-skin** -- Data-driven TOML skin system with 18 skins (14 external TOML in `skins/`, 18 built-in). Theme derivation from 9 base colors.
- **oasis-browser** -- Embeddable HTML/CSS/Gemini rendering engine: DOM parser, CSS cascade with `@media`/`@supports` queries and `var()` custom properties, flex/grid/table layout, `calc()`, full 2D CSS transforms (rotate/scale/skew rendered as transformed quads via `AffineTransform2D`), animations, hover-triggered CSS transitions (27 numeric properties auto-interpolate on state change), `::before`/`::after` pseudo-elements, `text-overflow: ellipsis`, z-index stacking contexts (CSS 2.1 appendix E paint order), Canvas 2D path API (`beginPath`/`bezierCurveTo`/`quadraticCurveTo`/`fill`/`stroke`/`save`/`restore`), SVG paths with fill-rule/linecap/linejoin and `<g>` group transform composition, light compositor (display list with batched rect+text submission via `SdiBatch::submit_rect_batch`/`submit_text_batch`, vertical/horizontal strip merging, occluded rect elimination, clip intersection optimization, granular animation dirty tracking, sticky element scroll caching via `PushSticky`/`PopSticky` display items), nested scroll containers (`overflow: auto/scroll` with per-element scroll offsets), form elements (text input, password masking, checkbox, radio button, textarea, select with dropdown overlay, submit/reset buttons, `<label for="">` click association, Tab focus navigation, form GET/POST submission), soft hyphen (U+00AD) line breaking with visible hyphens, bidi text direction detection (Hebrew/Arabic/Syriac), cookies, gzip, CSP (scripts/styles/connect enforced; img-src relaxed), link navigation, reader mode, JavaScript DOM bindings
- **oasis-js** -- JavaScript engine wrapping QuickJS-NG via rquickjs: `console` API (log/warn/error/info), inline `<script>` execution, DOM manipulation (`document.getElementById`, `createElement`, `textContent`, attributes, `innerHTML`, `classList`, `style` property, `querySelector`/`querySelectorAll`), `fetch()` API, `setTimeout`/`setInterval`, `localStorage` (persistent across navigations)/`sessionStorage`, `document.cookie` getter/setter, `history.pushState`/`replaceState`, `window.location` with assign/replace/reload, retained engine with event dispatch (click/keydown/keyup/mousedown/mouseup/mousemove bubbling via `__oasis_dispatch_with_bubbling` with three-phase capture/target/bubble dispatch, `addEventListener` options `{once, capture, passive}`, detail properties `clientX`/`clientY`/`key`/`code`, `stopPropagation`/`preventDefault`). Feature-gated (`javascript`)
- **oasis-ui** -- 32 reusable widgets: Button, Card, TabBar, Panel, InputField, ListView, ScrollView, ProgressBar, Toggle, NinePatch, flex layout, Accordion, Avatar, Badge, Checkbox, ColorPicker, ContextMenu, DatePicker, Divider, Dropdown, Icon, Modal, Radio, RichText, Slider, SpinBox, Spinner, SplitPane, Table, Toast, Tooltip, TreeView
- **oasis-vfs** -- Virtual file system: `MemoryVfs` (in-RAM), `RealVfs` (disk), `GameAssetVfs` (UE5 with overlay writes)
- **oasis-terminal** -- Command interpreter with 90+ commands across 17 modules (core, text, file, system, dev, fun, security, doc, audio, network, skin, UI, plus agent/plugin/script/transfer/update registered by oasis-core). Shell features: variable expansion, glob expansion, aliases, history, piping
- **oasis-wm** -- Window manager (window configs, hit testing, drag/resize, minimize/maximize/close)
- **oasis-net** -- TCP networking with PSK authentication, remote terminal, FTP transfer
- **oasis-audio** -- Audio manager with playlist, shuffle/repeat modes, MP3 ID3 tag parsing
- **oasis-platform** -- Platform service traits: PowerService, TimeService, UsbService, NetworkService, OskService
- **oasis-video** -- MP4/H.264+AAC decode pipeline. Feature flags: `h264` (openh264 video decode + symphonia demux/AAC), `no-std-demux` (lightweight `demux_lite::Mp4Lite` parser, no symphonia/no std::sync::Once — PSP-safe), `video-decode` (re-exports `SoftwareVideoDecoder` for desktop/UE5). Streaming pipelines: desktop uses `StreamingBuffer` sliding-window for progressive playback with deferred tail probe, CDN failover, and PTS-based A/V sync; PSP streams in-memory via `demux_lite` + `sceAudiocodec` AAC hardware decode + `sceVideocodec` H.264 (real HW only, audio-only on PPSSPP) with backpressure-throttled I/O
- **oasis-vector** -- Resolution-independent vector graphics: scene graph with path-based drawing operations (fill, stroke, arcs, beziers), Altimit-style dashboard icons, and frame-driven animations. Integrates via `SdiBackend` vector graphics trait extensions
- **oasis-shader** -- Animated shader wallpapers: Shadertoy-style fragment shaders (voronoi, city lights, ocean waves, calm waves, Balatro)
- **oasis-app-core** -- Shared app framework: `AppTrait`, common utilities for extracted app crates
- **oasis-app-*** -- 11 extracted app crates: `oasis-app-games`, `oasis-app-paint`, `oasis-app-clock`, `oasis-app-text-editor`, `oasis-app-calculator`, `oasis-app-media` (Music Player + Photo Viewer), `oasis-app-tv-guide`, `oasis-app-radio`, `oasis-app-settings`, `oasis-app-file-manager`
- **oasis-core** -- Coordination layer: dashboard, agent/MCP, plugin, scripting, status/bottom bars, desktop taskbar. Apps extracted to `oasis-app-*` crates (remaining in-core: Browser, Network, Package Manager, System Monitor)

### Font Rendering

Proportional bitmap font rendering from glyph ink bounds. `oasis-types` provides `glyph_advance()` with variable per-character widths (3-8px). Each backend has its own glyph table in `font.rs`. The PSP backend additionally uses system TrueType fonts via `psp::font` with a VRAM glyph atlas. No external font dependencies for desktop/UE5.

### FFI Boundary (oasis-ffi)

Exports C-ABI functions: `oasis_create`, `oasis_destroy`, `oasis_tick`, `oasis_send_input`, `oasis_get_buffer`, `oasis_get_dirty`, `oasis_send_command`, `oasis_free_string`, `oasis_set_vfs_root`, `oasis_register_callback`, `oasis_add_vfs_file`. This is how UE5 (or any C-compatible host) embeds OASIS_OS.

## Code Conventions

- MSRV: 1.91.0 (uses `str::floor_char_boundary`)
- Max line width: 100 characters
- Clippy warnings are CI errors (`-D warnings`)
- Workspace lints: `clone_on_ref_ptr`, `dbg_macro`, `todo`, `unimplemented` = warn; `unsafe_op_in_unsafe_fn` = warn; `unwrap_used` = deny
- All unsafe blocks require `// SAFETY:` comments
- Tests are in-module (`#[cfg(test)] mod tests`), not in a separate `tests/` directory
- Dual-licensed: Unlicense + MIT

## Docker Services

`docker-compose.yml` profiles:
- `ci` -- rust-ci container (rust:1.93-slim + cmake + X11/audio dev libs + nightly + cargo-deny)
- `psp` -- PPSSPP emulator (multi-stage build, NVIDIA GPU passthrough)
- `services` -- MCP server containers (code-quality, content-creation, gemini, etc.)

## Document Index

Key documentation files for agents and contributors. Read these for deeper context on specific topics rather than loading everything into every conversation.

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
- [`AGENTS.md`](AGENTS.md) -- Multi-agent system configuration and CI workflow
- [`CONTRIBUTING.md`](CONTRIBUTING.md) -- Contribution policy (AI-authored only)
- [`scripts/psp-scenarios.md`](scripts/psp-scenarios.md) -- PSP test scenario documentation
