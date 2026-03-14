# OASIS_OS Comprehensive Codebase Review

**Date:** 2026-03-14
**Branch:** `review/comprehensive-codebase-analysis`
**Scope:** Full codebase deep dive — architecture, quality, PSP homebrew context, and actionable improvements

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [What's Working Well](#whats-working-well)
3. [What's Not Working Well](#whats-not-working-well)
4. [What's Normal vs Abnormal](#whats-normal-vs-abnormal)
5. [PSP Homebrew Context](#psp-homebrew-context)
6. [Actionable Improvements](#actionable-improvements)
   - [DRY](#dry-dont-repeat-yourself)
   - [Modularity](#modularity)
   - [Robustness](#robustness)
   - [Features](#features)

---

## Executive Summary

| Metric | Value |
|--------|-------|
| Total crates | 32 workspace + 2 excluded (PSP backend/plugin) |
| Total Rust files | ~298 |
| Total LOC | ~166,000 |
| Largest crate | oasis-ui (16,442 LOC) |
| Test coverage | 30/32 crates have tests |
| Unsafe blocks | 556 (all with `// SAFETY:` comments) |
| Terminal commands | 90+ |
| Apps | 10 functional + 6 stubs |
| Skins | 18 (12 external TOML, 18 built-in) |
| UI widgets | 32 |
| Browser engine | Full HTML/CSS pipeline + Gemini + JS DOM bindings |

**Overall grade: A-** — Production-grade embedded OS framework with excellent trait design, comprehensive testing, and sophisticated streaming infrastructure. Main gaps are in app-layer completeness and some code duplication across backends.

---

## What's Working Well

### 1. Trait-Based Platform Abstraction (A+)

The `SdiCore` (13 required methods) + `SdiBackend` (30+ optional methods) + 8 focused extension traits design is excellent. Default implementations on `SdiBackend` mean backends only override what they can accelerate. Four backends (SDL, WASM, UE5, PSP) prove the abstraction works across radically different platforms.

- `SdiCore`: init, clear, blit, fill_rect, draw_text, swap_buffers, load_texture, destroy_texture, set_clip_rect, reset_clip_rect, measure_text, read_pixels, shutdown
- Extension traits: `SdiShapes`, `SdiGradients`, `SdiAlpha`, `SdiText`, `SdiTextures`, `SdiClipTransform`, `SdiVector`, `SdiBatch`
- Progressive override: any backend can opt into accelerated paths without implementing everything

### 2. Error Handling (A+)

Structured domain-specific error enums (`SdiError`, `BackendError`, `ConfigError`, `VfsError`, `CommandError`, `WmError`, `PluginError`, `PlatformError`) with a top-level `OasisError` wrapper. Uses `thiserror` for `Display`/`From` derives. No `anyhow` in production code — all errors are typed and recoverable. 70+ test functions validate error conversions and pattern matching.

### 3. Skin/Theme System (A)

Data-driven TOML skins with hot-swapping, inheritance (`merge_theme_from`), and 9 base colors for automatic theme derivation. 18 skins compiled into binary for instant availability. Skins control layout, colors, features, and strings — genuinely data-driven, not just color swaps.

### 4. Browser Engine (A-)

Complete pipeline: HTML tokenizer → DOM (arena-allocated) → CSS cascade (specificity, selectors, shorthand expansion) → block/inline/table layout → paint → JavaScript DOM bindings. ~31K LOC. Reader mode, Gemini protocol, image caching, form handling. Pragmatically scoped — no flexbox/grid, but complete for embedded content rendering.

### 5. Video Streaming Architecture (A)

Desktop: `StreamingBuffer` sliding-window with probe_mode (zeros during symphonia probe), deferred tail probe (prevents CDN throttling), CDN failover (fresh 302 redirects), PTS-based A/V sync, and linear seek estimation. PSP: in-memory streaming via `demux_lite` + `sceAudiocodec` hardware AAC. Both paths handle moov-at-start and moov-at-end MP4 files. All 5 TV Guide channels work.

### 6. Window Manager (A)

Full-featured: drag/resize, tiling layout, virtual desktops, window snapping, smooth animations, hit testing, decorations. 8,079 LOC across 9 well-separated modules. Production-grade for a 2D compositing environment.

### 7. Widget Library (A)

32 widgets with a minimal 2-method `Widget` trait (`measure` + `draw`). Stateless design (most widgets are `&self`). Flex layout system. Focus management with keyboard navigation. Accessibility attributes (ARIA). Theme integration through `DrawContext`.

### 8. Testing & Safety Culture (A+)

- 30/32 crates tested (only PSP plugin lacks tests — kernel-mode code)
- `unsafe_op_in_unsafe_fn = "warn"` enforces SAFETY comments
- All 556 unsafe blocks documented
- Clippy warnings are CI errors (`-D warnings`)
- Screenshot regression tests, browser rendering tests (2,321 LOC), e2e tests
- ASAN + Valgrind massif in separate CI workflow

### 9. PSP TLS 1.3 Implementation

Pure Rust TLS 1.3 on a 2004 MIPS CPU via `embedded-tls` is remarkable. Handles the `mfc0 $9` privilege issue, DNS endianness fix, RSA scheme advertisement via `alloc` feature, HTTP→HTTPS redirect detection with fallback, and CDN node variability (some HTTP, some HTTPS-only). This works end-to-end for streaming from archive.org.

---

## What's Not Working Well

### 1. Large Monolithic Files

20+ files exceed 1,000 lines. The worst offenders:

| File | Lines | Issue |
|------|-------|-------|
| `oasis-wm/manager.rs` | 2,177 | Window state machine + drag + animations + snap — too many concerns |
| `oasis-ffi/lib.rs` | 1,848 | 20+ C-ABI functions + marshaling in one file |
| `oasis-skin/builtin.rs` | 1,802 | 18 skins as hardcoded Rust structs |
| `oasis-video/demux_lite.rs` | 1,834 | Entire MP4 parser in one file |
| `oasis-skin/theme.rs` | 1,721 | Theme derivation + color math + animation curves |
| `oasis-browser/paint.rs` | 1,504 | Full paint pipeline |
| `oasis-backend-ue5/renderer.rs` | 1,490 | Entire UE5 renderer |
| `oasis-terminal/commands.rs` | 1,486 | All core commands in one file |
| `oasis-app-games/lib.rs` | 1,438 | All games (Snake, Puzzle, Memory) in one file |

### 2. Backend Code Duplication

SDL, WASM, and UE5 backends independently re-implement:
- **Clip stack**: `Vec<ClipRect>` with intersection logic — nearly identical across 3 backends
- **Translate stack**: `Vec<(i32, i32)>` with cumulative offset — identical across 3 backends
- **Gradient interpolation math**: Same linear color interpolation
- **Shader bridge**: `shader_bridge.rs` in both SDL (80 lines) and WASM (75 lines) — nearly identical core logic

### 3. Stub Apps

6 of 16 apps are stubs via `SimpleApp`:
- Settings — only shows TOML data
- Network — placeholder
- Package Manager — placeholder
- Browser (as app) — stub; the real browser is a subsystem
- System Monitor — shows backend name only
- Terminal (as app) — stub; real terminal is in oasis-core

### 4. Duplicate Type Definitions

- `VideoPlayer` — 2 definitions (oasis-app, oasis-backend-wasm)
- `DecodedFrame` — 2 definitions (oasis-video internals)
- `ShaderParams` / `ShaderLayerInfo` — 2 definitions (separate shader modules)
- `HistoryEntry` — 2 definitions (browser, terminal)

### 5. PSP Backend Missing Extended Methods

PSP backend does not implement `push/pop_clip_rect`, `push/pop_translate`, `fill_triangle`, `blit_sub`, `blit_tinted`, or `blit_flipped`. This means PSP apps get software fallbacks for these operations, which may be slower than necessary. The clip/translate stack omission forces app-layer workarounds.

### 6. No Plugin Sandboxing

Plugins get `&mut dyn Vfs` (full filesystem), `&mut CommandRegistry` (can register any command), and optional audio/network access. No capability-based restrictions. Intentional for the embedded context, but limits trust model.

### 7. Permissions Are Decorative

`chmod`, `chown`, `passwd` commands exist, but permissions are not enforced at the VFS layer. `whoami` always returns "oasis". The security commands create an illusion of access control without substance.

---

## What's Normal vs Abnormal

### Normal for an Embedded OS Framework

- **No process isolation**: Apps are cooperative delegates, not preemptive processes. Standard for embedded/homebrew systems.
- **Blocking VFS**: All file operations are synchronous. Normal for single-threaded game loops; mitigated by background threads where needed.
- **No CSS3 flexbox/grid**: The browser focuses on content rendering, not web app compatibility. Typical for embedded browsers (NetSurf, Dillo).
- **Monolithic app binaries**: Single binary with all apps compiled in. Normal for embedded targets.
- **Feature-gated dependencies**: JavaScript, video decode, TLS are all optional. Standard for cross-platform Rust crates.

### Abnormal (In a Good Way)

- **Rust on PSP**: Virtually all PSP homebrew is C/C++ with pspsdk. A full OS framework in Rust with 15,600+ LOC PSP backend, TLS 1.3, and video streaming is unprecedented.
- **4 rendering backends**: SDL, WASM, UE5, and PSP from one codebase. Most projects support 1-2 platforms.
- **Kernel-mode PRX plugin**: A Rust-based PSP kernel plugin that hooks `sceDisplaySetFrameBuf` for in-game overlay, survives game launches, and does background audio — no other Rust project does this.
- **166K LOC workspace**: 32 crates with clear dependency hierarchy. PSP homebrew projects are typically 1K-10K lines.
- **Screenshot regression testing**: CI captures screenshots and diffs them. Unusual for any project, extraordinary for homebrew.
- **Arena-allocated DOM**: The browser uses arena allocation for DOM nodes, which is an advanced optimization typically seen in production browser engines.

### Abnormal (Concerning)

- **850KB built-in skins**: `skin_builtin::load_builtin()` adds ~850KB of compiled TOML data. On PSP (32MB RAM), this is significant. Already documented as a known issue.
- **No async anywhere**: Even networking and streaming are synchronous with manual threading. Not inherently wrong, but limits composability.
- **FFI file is 1,848 lines**: The C-ABI boundary for UE5 integration is a single file with 20+ extern functions and all marshaling logic. High blast radius for changes.

---

## PSP Homebrew Context

### How OASIS_OS Compares to Typical PSP Homebrew

| Aspect | Typical PSP Homebrew | OASIS_OS |
|--------|---------------------|----------|
| Language | C/C++ (pspsdk/GCC) | Rust (custom rust-psp/LLVM) |
| Scale | 1K-10K LOC, 1-5 files | 166K LOC, 298 files, 32 crates |
| Threading | Single-thread or basic | Lock-free queues, kernel threads, media engine offloading |
| Networking | Basic HTTP downloads | TLS 1.3, CDN failover, streaming with backpressure |
| GPU usage | Textured quads only | Full GU pipeline with vertex colors, sprite batching |
| Plugin support | N/A | Kernel-mode PRX with display/audio hooks |
| Cross-platform | PSP-only | PSP + SDL + WASM + UE5 |
| Testing | Manual | CI with screenshot regression, ASAN, Valgrind |

### Similar PSP Homebrew Projects (for reference)

- **PSP Custom Firmware (ARK-4, PRO)**: Kernel-mode plugins with display hooks — OASIS_OS's PRX plugin follows this pattern but in Rust
- **PSPKVM**: Java ME runtime on PSP — similar ambition level (full runtime on embedded hardware)
- **PSP Media Center / LuaPlayer**: Media playback + scripting — OASIS_OS covers this plus more
- **Daedalus (N64 emulator)**: Similar complexity, GU usage, and threading model
- **NetFront Browser**: Sony's built-in PSP browser — OASIS_OS's browser engine is more capable (CSS cascade, JavaScript)

### PSP-Specific Engineering Achievements

1. **TLS 1.3 on Allegrex MIPS**: Pure Rust, no C/asm, handles RSA certs
2. **60 FPS with 108+ SDI objects**: Split render path (base + overlay layers)
3. **Streaming audio from archive.org CDN**: HTTP/HTTPS fallback, DNS endianness fix, backpressure throttling
4. **System TrueType fonts**: VRAM glyph atlas via `psp::font`
5. **Weak import stubs**: Fixed `sceVideocodec` import flag from strong (0x4001) to weak (0x4009) to prevent module load failure on real hardware

---

## Actionable Improvements

### DRY (Don't Repeat Yourself)

#### D1. Extract Shared Clip/Translate Stack Helper
**Priority: High** | **Effort: Medium** | **Impact: 3 backends**

SDL, WASM, and UE5 all implement nearly identical `Vec<ClipRect>` clip stacks and `Vec<(i32, i32)>` translate stacks. Extract to a shared helper in `oasis-types` or a new `oasis-backend-common` crate.

```
// New: oasis-types/src/backend/clip_stack.rs
pub struct ClipStack { rects: Vec<(i32, i32, u32, u32)> }
impl ClipStack {
    pub fn push(&mut self, x, y, w, h) -> (i32, i32, u32, u32) // returns intersection
    pub fn pop(&mut self) -> Option<(i32, i32, u32, u32)>
    pub fn current(&self) -> Option<(i32, i32, u32, u32)>
}
```

#### D2. Unify Shader Bridge
**Priority: Medium** | **Effort: Low** | **Impact: 2 backends**

`oasis-backend-sdl/src/shader_bridge.rs` (80 lines) and `oasis-backend-wasm/src/shader_bridge.rs` (75 lines) share identical CPU shader rendering logic. Extract the core computation into `oasis-shader` and leave only the backend-specific texture upload in each backend.

#### D3. Consolidate Gradient Math
**Priority: Medium** | **Effort: Low** | **Impact: 3 backends**

Linear color interpolation for gradient fills is duplicated in SDL, UE5, and WASM backends. Extract to a `gradient_interpolate(color_a, color_b, t: f32) -> Color` function in `oasis-types::Color`.

#### D4. Eliminate Duplicate Type Definitions
**Priority: Medium** | **Effort: Medium** | **Impact: Clarity**

- `DecodedFrame`: Consolidate into single definition in `oasis-video`
- `ShaderParams` / `ShaderLayerInfo`: Consolidate into `oasis-shader`
- `HistoryEntry`: If semantically different (browser history vs terminal history), rename for clarity. If same, extract to shared module.

#### D5. Centralize Color Manipulation Utilities
**Priority: Low** | **Effort: Low** | **Impact: 4+ crates**

`with_alpha()`, `lighten()`, `darken()`, `blend()` are reimplemented in multiple crates. Add these as methods on `oasis-types::Color` if not already there, and remove crate-local copies.

---

### Modularity

#### M1. Split `oasis-wm/manager.rs` (2,177 lines)
**Priority: High** | **Effort: Medium**

Split into:
- `lifecycle.rs` — Window creation, destruction, state transitions
- `geometry.rs` — Position/size calculations, layout
- `input.rs` — Focus routing, click dispatch
- `render.rs` — Decoration drawing, SDI object management
- `manager.rs` — Coordination only (thin orchestrator)

#### M2. Split `oasis-ffi/lib.rs` (1,848 lines)
**Priority: High** | **Effort: Medium**

Split into:
- `handle.rs` — Instance lifecycle (`oasis_create`, `oasis_destroy`)
- `input.rs` — Input marshaling (`oasis_send_input`)
- `render.rs` — Buffer access (`oasis_get_buffer`, `oasis_get_dirty`)
- `commands.rs` — Command dispatch (`oasis_send_command`)
- `vfs.rs` — VFS operations (`oasis_set_vfs_root`, `oasis_add_vfs_file`)
- `callbacks.rs` — Callback registration
- `lib.rs` — Re-exports only

#### M3. Split `oasis-app-games/lib.rs` (1,438 lines)
**Priority: Medium** | **Effort: Low**

Each game should be its own module:
- `games/snake.rs`
- `games/puzzle.rs`
- `games/memory.rs`
- `games/mod.rs` — Game selector UI

#### M4. Split `oasis-skin/builtin.rs` (1,802 lines)
**Priority: Medium** | **Effort: Medium**

Group built-in skins by category:
- `builtin/classic.rs` — classic, retro, terminal
- `builtin/nature.rs` — beach, sky, forest
- `builtin/modern.rs` — modern, xp, flat
- Or generate at build time from the TOML files via `build.rs`

#### M5. Extract App Crates for Remaining In-Core Apps
**Priority: Medium** | **Effort: High**

`file_manager.rs` (856 lines) is still in `oasis-core/src/apps/`. Follow the pattern established by `oasis-app-calculator`, `oasis-app-clock`, etc. and extract it to `oasis-app-file-manager`.

#### M6. Split `oasis-terminal/commands.rs` (1,486 lines)
**Priority: Medium** | **Effort: Medium**

Core commands like `ls`, `cd`, `cat`, `echo` are all in one file. Split into logical groups following the pattern of `dev_commands.rs`, `text_commands.rs`:
- `fs_commands.rs` — ls, cd, pwd, mkdir, rm, cp, mv, find, touch
- `io_commands.rs` — cat, echo, write, append
- `system_commands.rs` — clear, status, uptime, df

#### M7. Consider `oasis-backend-common` Crate
**Priority: Low** | **Effort: Medium**

A new crate for shared backend logic:
- Clip/translate stacks (D1)
- Gradient math (D3)
- Shader bridge (D2)
- Common input event mapping
- Software rasterization primitives (used by UE5 and as fallbacks)

---

### Robustness

#### R1. Enforce VFS Permissions
**Priority: High** | **Effort: Medium**

Currently `chmod`/`chown` set metadata but nothing checks it. Either:
- (a) Add permission checks to `Vfs::read()`, `Vfs::write()`, etc. with a `VfsContext` carrying the current user
- (b) Remove the permission commands and metadata to avoid false security promises

Option (a) is better for the OS metaphor. Option (b) is simpler.

#### R2. Add Integration Tests for Video Streaming
**Priority: High** | **Effort: Medium**

The video streaming pipeline (`StreamingBuffer`, probe_mode, CDN failover) is 929+ LOC on PSP and similar on desktop with minimal test coverage. Add integration tests that:
- Test probe_mode behavior (zeros returned, decoder_pos not updated)
- Test `should_throttle()` edge cases
- Test moov-at-end detection and deferred tail probe
- Mock HTTP responses for CDN failover scenarios

#### R3. Reduce Unsafe Surface in FFI
**Priority: Medium** | **Effort: Medium**

215 unsafe blocks in `oasis-ffi/lib.rs`. Many follow the same pattern: `let Some(instance) = (unsafe { handle.as_mut() }) else { return; }`. Extract a safe wrapper:

```rust
fn with_instance<F, R>(handle: *mut OasisInstance, default: R, f: F) -> R
where F: FnOnce(&mut OasisInstance) -> R
```

This would eliminate ~100 unsafe blocks.

#### R4. Plugin Capability System
**Priority: Medium** | **Effort: High**

Add a capability declaration to `PluginInfo`:
```rust
pub struct PluginInfo {
    // existing fields...
    pub capabilities: PluginCapabilities,
}

pub struct PluginCapabilities {
    pub vfs_read: bool,
    pub vfs_write: bool,
    pub commands: bool,
    pub audio: bool,
    pub network: bool,
}
```

`PluginHost` checks these before granting access. Doesn't need to be a security boundary — just a "principle of least surprise" guard.

#### R5. Validate TOML Skin Schema
**Priority: Low** | **Effort: Medium**

External TOML skins are parsed but not validated against a schema. A malformed skin can cause confusing errors deep in the rendering pipeline. Add a `validate()` step to `Skin` after loading that checks:
- Required fields present
- Color values are valid hex
- Layout coordinates are within virtual resolution
- Feature flags are recognized names

#### R6. Add Tests for PSP-Specific Code Paths
**Priority: Medium** | **Effort: High**

PSP backend has 15,600+ LOC but limited unit tests. Key areas to test (using mock types, not real PSP hardware):
- Input dispatch logic (1,027 lines)
- Theme derivation
- SDI object rendering decisions
- Network TLS fallback logic

#### R7. Error Recovery in Browser Paint
**Priority: Low** | **Effort: Low**

`paint.rs` (1,504 lines) has several `.unwrap_or_default()` calls that silently swallow errors. Add logging for paint failures to aid debugging layout issues.

---

### Features

#### F1. Flesh Out Stub Apps
**Priority: High** | **Effort: High**

The 6 stub apps hurt the "OS" feel. Priority order:
1. **Settings** — Make it functional: skin selection, audio volume, display resolution, network config
2. **System Monitor** — Show CPU/memory usage, running apps, backend info
3. **Network** — WiFi scanner (PSP), interface list (desktop), connection status
4. **Package Manager** — App discovery from VFS, install/uninstall, version tracking
5. **Browser (app)** — Wire the oasis-browser subsystem into a proper windowed app
6. **Terminal (app)** — Wire the oasis-terminal subsystem into a proper windowed app

#### F2. Add PSP Clip/Translate Stack Support
**Priority: Medium** | **Effort: Medium**

The PSP backend lacks `push/pop_clip_rect` and `push/pop_translate`. These could be implemented using GU scissor test (`sceGuScissor`) for clipping and transform matrix manipulation for translation. This would enable more widgets to render correctly on PSP without app-layer workarounds.

#### F3. Clipboard Support
**Priority: Medium** | **Effort: Low**

No clipboard exists. Add:
- `ClipboardBackend` trait with `copy(text)` and `paste() -> Option<String>`
- SDL: use `SDL_SetClipboardText` / `SDL_GetClipboardText`
- WASM: use `navigator.clipboard` API
- PSP/UE5: in-memory clipboard
- Wire into Text Editor, Terminal, Browser

#### F4. Notification System
**Priority: Medium** | **Effort: Medium**

The `Toast` widget exists in `oasis-ui` but there's no system-level notification queue. Add:
- `NotificationManager` in `oasis-core` with priority levels
- Toast rendering in the status bar area
- Terminal `notify` command to push notifications
- App API to emit notifications

#### F5. Text Editor Syntax Highlighting
**Priority: Low** | **Effort: Medium**

The Text Editor (955 lines) is functional but bare. The terminal already has `highlight.rs` (1,042 lines) for syntax highlighting. Wire this into the Text Editor for `.rs`, `.toml`, `.html`, `.css`, `.js` files.

#### F6. i18n / Localization Framework
**Priority: Low** | **Effort: High**

All strings are hardcoded English. The skin system already has `SkinStrings` — extend it to a proper i18n system:
- Locale files in `locales/{lang}.toml`
- `t!("key")` macro for string lookup
- Fallback chain: skin strings → locale → English default

#### F7. UDP / Multicast Networking
**Priority: Low** | **Effort: Medium**

Only TCP is supported. Add UDP for:
- Local device discovery (mDNS)
- Lightweight telemetry
- Game multiplayer (PSP ad-hoc)

#### F8. Additional Media Codecs
**Priority: Low** | **Effort: High**

Currently limited to MP4/H.264/AAC/MP3/WAV. Consider:
- WebP image support (via `image` crate)
- FLAC audio (symphonia already supports it)
- OGG/Vorbis (symphonia supports it)

#### F9. Virtual Desktop Exposure in Shell
**Priority: Low** | **Effort: Low**

The window manager supports virtual desktops (`desktops.rs`, 849 lines) but the shell UI doesn't expose desktop switching. Add a desktop switcher to the taskbar.

#### F10. Build-Time Skin Compilation
**Priority: Low** | **Effort: Medium**

Instead of hardcoding 18 skins in `builtin.rs` (1,802 lines), use a `build.rs` script to:
1. Read TOML files from `skins/`
2. Parse and validate at compile time
3. Generate Rust code (or embed as `include_bytes!`)

This would eliminate the maintenance burden of keeping TOML and Rust skin definitions in sync and reduce `builtin.rs` to a thin loader.

---

## Priority Matrix

| ID | Category | Priority | Effort | Summary |
|----|----------|----------|--------|---------|
| D1 | DRY | High | Medium | Extract shared clip/translate stack |
| M1 | Modularity | High | Medium | Split `manager.rs` (2,177 lines) |
| M2 | Modularity | High | Medium | Split `ffi/lib.rs` (1,848 lines) |
| R1 | Robustness | High | Medium | Enforce or remove VFS permissions |
| R2 | Robustness | High | Medium | Integration tests for video streaming |
| F1 | Features | High | High | Flesh out 6 stub apps |
| D2 | DRY | Medium | Low | Unify shader bridge |
| D3 | DRY | Medium | Low | Consolidate gradient math |
| D4 | DRY | Medium | Medium | Eliminate duplicate types |
| M3 | Modularity | Medium | Low | Split games into modules |
| M4 | Modularity | Medium | Medium | Split/generate builtin skins |
| M5 | Modularity | Medium | High | Extract remaining in-core apps |
| M6 | Modularity | Medium | Medium | Split terminal commands.rs |
| R3 | Robustness | Medium | Medium | Reduce FFI unsafe surface |
| R4 | Robustness | Medium | High | Plugin capability system |
| R6 | Robustness | Medium | High | PSP code path tests |
| F2 | Features | Medium | Medium | PSP clip/translate stack |
| F3 | Features | Medium | Low | Clipboard support |
| F4 | Features | Medium | Medium | Notification system |
| D5 | DRY | Low | Low | Centralize color utilities |
| M7 | Modularity | Low | Medium | `oasis-backend-common` crate |
| R5 | Robustness | Low | Medium | TOML skin schema validation |
| R7 | Robustness | Low | Low | Browser paint error logging |
| F5 | Features | Low | Medium | Text editor syntax highlighting |
| F6 | Features | Low | High | i18n / localization |
| F7 | Features | Low | Medium | UDP / multicast networking |
| F8 | Features | Low | High | Additional media codecs |
| F9 | Features | Low | Low | Virtual desktop UI exposure |
| F10 | Features | Low | Medium | Build-time skin compilation |

---

## Appendix: Crate Size Reference

| Crate | LOC | Files | Largest File |
|-------|-----|-------|--------------|
| oasis-ui | 16,442 | 42 | focus.rs (977) |
| oasis-terminal | 14,485 | 30 | commands.rs (1,486) |
| oasis-backend-psp | 15,635 | 28 | main.rs (1,178) |
| oasis-browser | 11,822 | 22 | browser_tests.rs (2,321) |
| oasis-wm | 8,739 | 11 | manager.rs (2,177) |
| oasis-app | 8,453 | 12 | input.rs (1,347) |
| oasis-skin | 6,311 | 8 | builtin.rs (1,802) |
| oasis-backend-wasm | 5,477 | 14 | lib.rs (1,396) |
| oasis-core | 5,122 | 11 | startmenu.rs (825) |
| oasis-video | 4,467 | 8 | demux_lite.rs (1,834) |
| oasis-app-tv-guide | 4,177 | 10 | grid_render.rs (1,270) |
| oasis-backend-sdl | 3,458 | 8 | lib.rs (1,265) |
| oasis-types | 3,255 | 11 | — |
| oasis-vector | 3,209 | 8 | icons.rs (834) |
| oasis-net | 3,082 | 8 | tls_rustls.rs (965) |
| oasis-vfs | 2,661 | 5 | memory.rs (1,016) |
| oasis-plugin-psp | 2,458 | 12 | video.rs (777) |
| oasis-backend-ue5 | 2,065 | 4 | renderer.rs (1,490) |
| oasis-audio | 2,011 | 7 | — |
| oasis-sdi | 1,947 | 4 | — |
| oasis-ffi | 1,848 | 1 | lib.rs (1,848) |
| oasis-app-games | 1,438 | 5 | lib.rs (1,438) |
| oasis-app-clock | 1,365 | 4 | lib.rs (1,365) |
| oasis-app-paint | 1,317 | 4 | lib.rs (1,317) |
| oasis-app-calculator | 1,273 | 2 | lib.rs (1,273) |
| oasis-app-text-editor | 955 | 4 | lib.rs (955) |
| oasis-app-media | 666 | 2 | lib.rs (666) |
| oasis-app-radio | 338 | 2 | lib.rs (338) |
