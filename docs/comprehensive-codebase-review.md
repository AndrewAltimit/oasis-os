# Comprehensive Codebase Review: OASIS_OS

**Date**: 2026-03-14
**Scope**: Full codebase audit across 34 crates, ~215K LOC, 468 Rust source files
**Branch**: `review/comprehensive-codebase-audit`

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [What's Working Well](#2-whats-working-well)
3. [What's Not Working Well](#3-whats-not-working-well)
4. [Normal vs. Abnormal Patterns](#4-normal-vs-abnormal-patterns)
5. [PSP Homebrew Comparison](#5-psp-homebrew-comparison)
6. [Actionable Improvements](#6-actionable-improvements)
   - [DRY](#61-dry-dont-repeat-yourself)
   - [Modularity](#62-modularity)
   - [Robustness](#63-robustness)
   - [Features](#64-features)

---

## 1. Executive Summary

| Metric | Value |
|--------|-------|
| Workspace crates | 34 (+ 2 excluded PSP) |
| Total LOC | ~215,000 |
| Source files | 468 |
| Test count | ~5,377 |
| Unsafe blocks | 688 (100% with SAFETY comments) |
| CI stages | 15 (format, clippy, test, build, screenshots, deny, PSP, PPSSPP, coverage, etc.) |
| Fuzz targets | 6 |
| External skins | 12 TOML + 18 built-in |
| Apps | 16 (11 extracted to own crates) |
| Terminal commands | 90+ across 17 modules |

**Overall Assessment**: Mature, well-architected Rust codebase with clean crate boundaries, strong safety discipline, and comprehensive CI. Key improvement areas are test coverage consistency, error handling patterns, and further modular extraction from oasis-core.

---

## 2. What's Working Well

### 2.1 Architecture & Crate Design

- **Clean DAG**: Zero circular dependencies across all 34 crates. The dependency graph flows strictly: Foundation (oasis-types) -> Platform services -> Coordination (oasis-core) -> Backends.
- **Backend trait abstraction**: `SdiCore` (13 required methods) + `SdiBackend` (39 optional accelerated primitives) + 8 focused extension traits (`SdiShapes`, `SdiGradients`, `SdiVector`, etc.) cleanly separate platform from logic. Core code never calls platform APIs directly.
- **App extraction**: 11 of 16 apps successfully extracted into dedicated crates with a shared `App` trait (`oasis-app-core/src/lib.rs`), reducing oasis-core coupling.
- **Feature flags**: Well-designed mutual exclusion (`oasis-video`: `h264` vs `ffmpeg` vs `no-std-demux`). TLS, JavaScript, and video decode properly gated with no conflicts.

### 2.2 Safety & Unsafe Discipline

- **100% SAFETY comment coverage** across all 688 unsafe blocks in 49 files. Comments explain WHY unsafe is needed, reference pre/post conditions, and note thread safety constraints.
- **No evidence** of use-after-free, buffer overflows, or race conditions.
- **Targeted `#[allow(dead_code)]`**: Only 27 files use it, all for platform-specific helpers or optional trait extensions. No blanket suppression.

### 2.3 PSP Code Quality

- **Best-in-class PSP Rust development**: Modern RAII patterns, lock-free SpscQueue for inter-thread communication (avoids priority inversion on single-core), VFPU assembly only where it matters (BT.601 YUV->RGB, 10-60x speedup).
- **Graceful degradation**: Missing fonts fall back to bitmap, missing modules fall back gracefully, GU display list overflow silently drops frames rather than crashing.
- **Volatile memory management**: RAII-based 4MB volatile memory allocator on PSP-2000+ with proper Drop cleanup.
- **TLS 1.3 on PSP**: Pure-Rust `embedded-tls` implementation bypassing PSP's 2008-era SSL stack. Handles CDN failover, redirect chains, and DNS endianness correctly.

### 2.4 CI/CD Pipeline

- **15-stage CI pipeline** with Docker reproducibility (explicit base images, cached layers, uid/gid matching).
- **Memory safety CI**: ASAN + Valgrind Massif as non-blocking workflows.
- **6 fuzz targets**: html_tokenizer, css_parser, gemini_parser, http_response, skin_toml, pbp_parser.
- **AI code review**: Gemini-powered async reviews on PRs with up to 5 auto-fix iterations, fork PR guards, and `no-auto-fix` label.
- **cargo-deny**: License/advisory auditing with permissive allowlist (MIT, Apache-2.0, BSD, ISC, Zlib, CC0, Unlicense, MPL-2.0).

### 2.5 Skin System

- **Build-time TOML->Rust code generation**: 12 external TOML skins compiled into const strings at build time, enabling PSP (no filesystem during init) to load skins without a TOML parser at runtime.
- **Theme derivation from 9 base colors**: Full skin specified from minimal input.
- **Zero runtime cost**: Embedded ROM doesn't include TOML parsing dependencies.

### 2.6 Browser Engine

- **44K LOC** with working HTML tokenizer/parser, DOM tree, CSS cascade, selectors, shorthand expansion, block/inline/table layout, paint, forms, JavaScript (QuickJS-NG), Gemini protocol, reader mode, navigation, and history.
- **Well-tested**: 2,321 lines of browser integration tests + 1,999 lines of CSS cascade tests.

### 2.7 Terminal & Commands

- **Clean trait-based registration**: `Command` trait with `CommandRegistry` (HashMap<String, Box<dyn Command>>). Pluggable `registry.register()` pattern.
- **16 command categories** organized by domain (core, text, file, system, dev, fun, security, doc, audio, network, skin, UI, agent, plugin, script, transfer).
- **Shell features**: Variable expansion, glob expansion, aliases, history, piping, control flow.

### 2.8 Documentation

- **8,782 lines** across 17 docs files + 5 ADRs + comprehensive CLAUDE.md.
- **Design doc**: 1,458-line technical document at v2.4.
- **Active guides**: Getting started, adding commands, skin authoring, plugin development, FFI integration.

---

## 3. What's Not Working Well

### 3.1 Error Handling Inconsistency

**3,888 `unwrap()`/`expect()` calls** despite `unwrap_used = "warn"` in workspace lints. The lint warns but is not enforced -- code compiles and ships with unwraps.

**Production code concerns** (not just tests):
- `oasis-backend-psp/src/threading/radio.rs:148` -- `let header_end = header_end.unwrap()` after manual `is_none()` check. Safe but fragile.
- `oasis-backend-psp/src/threading/audio.rs:189` -- `let decoder = aac_decoder.as_mut().unwrap()` in AAC decode loop. Protected by guard block but pattern invites future bugs.

**Test code**: 40+ `panic!("expected ...")` calls in pattern matching instead of `assert_matches!` or `assert_eq!`.

### 3.2 Test Coverage Inconsistency

**Crates with zero tests:**
| Crate | LOC | Risk |
|-------|-----|------|
| oasis-backend-wasm | 5,483 | **HIGH** -- entire WASM rendering untested |
| oasis-app-tv-guide | 4,292 | **HIGH** -- streaming/playback logic untested |
| oasis-backend-ue5 | 2,036 | Medium -- UE5 rendering pipeline |
| oasis-audio | 2,011 | Medium -- audio manager |
| oasis-js | ~200 | Low -- thin wrapper |

**No automated screenshot regression detection**: Screenshots are generated and uploaded as artifacts, but there's no baseline comparison or auto-fail on pixel differences. Note: we would need to seed the date and time so it doesnt change in screenshots and capture the screenshot on exact frames for dynamic shader wallpapers.

### 3.3 Large Files Needing Decomposition

| File | LOC | Issue |
|------|-----|-------|
| `oasis-types/src/backend/mod.rs` | 1,835 | Monolithic trait definitions -- SdiCore + SdiBackend + 8 extensions in one file |
| `oasis-video/src/demux_lite.rs` | 1,834 | Monolithic MP4 parser |
| `oasis-browser/src/css/parser.rs` | 1,948 | Large CSS parser (well-structured but could split) |
| `oasis-app/src/tv_controller/mod.rs` | 1,637 | TV controller with streaming logic |
| `oasis-browser/src/forms/manager.rs` | 1,636 | Form state management |

### 3.4 oasis-core Remaining Bloat

**19,324 LOC** still in oasis-core despite app extraction. The following modules are large enough to be separate crates:

| Module | LOC | Extraction Target |
|--------|-----|-------------------|
| `apps/runner.rs` | 1,303 | oasis-app-runner |
| `plugin/manager.rs` | 1,271 | oasis-plugin |
| `dashboard/mod.rs` | 1,130 | oasis-dashboard |
| `transfer/mod.rs` | 1,056 | oasis-transfer |
| `startmenu.rs` | 825 | oasis-startmenu |
| `osk/keyboard.rs` | 816 | oasis-osk |
| `taskbar.rs` | 812 | oasis-taskbar |
| `statusbar.rs` | 799 | oasis-statusbar |
| `terminal/agent_commands.rs` | 731 | (stays in core -- depends on agent) |
| `terminal/browser_commands.rs` | 682 | (stays in core -- depends on browser) |

Potential **38% reduction** of oasis-core by extracting the top 8 modules.

### 3.5 Missing CI Coverage

| Gap | Severity |
|-----|----------|
| No WASM browser E2E tests (Playwright/Puppeteer) | Medium |
| No plugin load/unload integration test | Medium |
| No screenshot baseline comparison (auto-regression) | Medium |
| No benchmark regression detection (criterion --baseline) | Low |
| No network mock server tests for oasis-net | Medium |
| No audio fixture-based tests | Low |
| Coverage metrics not enforced (no minimum gate) | Low |

---

## 4. Normal vs. Abnormal Patterns

### 4.1 Normal (Expected for This Type of Project)

- **Font duplication in PSP plugin**: The kernel PRX (`oasis-plugin-psp/src/font.rs`, 131 lines) has its own copy of bitmap font data. This is necessary because the PRX is a <64KB binary with no dependency on oasis-types. Standard practice for kernel-mode PSP code.
- **Per-backend shape implementations**: SDL shapes.rs (623 LOC) and PSP shapes.rs (571 LOC) have similar algorithms but different primitives (SDL uses float edge equations, PSP uses fixed-point GU). This is expected platform-specific code, not avoidable duplication.
- **Large browser test files**: 2,321 LOC of browser integration tests and 1,999 LOC of CSS cascade tests are proportional to the browser engine's complexity (~44K LOC total).
- **298 `.clone()` calls in oasis-browser**: CSS/DOM manipulation naturally requires cloning parsed values during cascade resolution. No pathological patterns detected.
- **Rc/RefCell in WASM backend (9 uses)**: JavaScript/DOM callback interactions require interior mutability. Standard WASM-bindgen pattern.
- **`format!()` usage (4,108 occurrences)**: Terminal, UI, and browser subsystems naturally generate strings. Not in hot rendering paths.

### 4.2 Abnormal (Unusual or Concerning)

- **oasis-backend-wasm imports 15 crates directly** (`Cargo.toml` lines 15-22) even though all are transitively available via oasis-core. This is unusual -- SDL and UE5 backends don't do this. Likely for explicit re-export clarity but adds maintenance burden. All other backends depend only on oasis-core + backend-specific crates.

- **Feature flag naming inconsistency**: oasis-backend-wasm has a `youtube` feature that maps to oasis-core's `wasm-youtube` feature. Different names for the same concept across crate boundaries.

- **`unwrap_used = "warn"` lint enabled but 3,888 violations exist**: The lint is declared but not enforced. Either upgrade to `deny` (after cleanup) or remove the lint to avoid false confidence. Having a warning that's universally ignored is worse than no lint at all.

- **Widget trait has no event handling abstraction**: `oasis-ui` widgets implement `measure()` + `draw()` but event handling is entirely external (caller responsibility). This limits widget composability -- Modal, ContextMenu, and Tooltip need special overlay machinery instead of natural nesting. Unusual for a widget system of this maturity.

- **Manual byte-by-byte memcpy in PSP code**: Required to avoid LLVM recursion on MIPS (`audio.rs:394-399`, `textures.rs:136-141`). While documented and necessary, this is an unusual workaround worth noting for anyone unfamiliar with the PSP toolchain.

- **`sceGuGetMemory` can return NULL** (`render.rs:121-124`) with silent failure: If the GU display list is full, rendering silently drops objects. No error propagation or frame-skip detection. Most PSP homebrew at least logs a warning.

---

## 5. PSP Homebrew Comparison

### 5.1 Comparison Table

| Feature | OASIS_OS | Typical C/C++ PSP Homebrew | Assessment |
|---------|----------|---------------------------|------------|
| **Language** | Rust (nightly, rust-psp) | C/C++ with pspsdk | Unique -- very few PSP projects in Rust |
| **GU rendering** | Sprite-based for all primitives | Mixed GU + direct framebuffer writes | Better -- unified approach |
| **VRAM management** | Volatile memory bump allocator + heap fallback | Manual `sceGeEdramGetAddr` + explicit frees | Better -- RAII, no leaks |
| **Font rendering** | System TrueType + bitmap fallback + VRAM atlas | Embedded font file or no fonts | Better -- sophisticated multi-layer |
| **Input** | `psp::input::Controller` (high-level, edge detection) | Raw `sceCtrlPeekBufferPositive` + manual debounce | Better -- cleaner abstraction |
| **Threading** | SpscQueue (lock-free), atomics | Mutexes or busy-wait | Better -- avoids priority inversion |
| **Audio** | Dedicated thread, MP3+AAC decode, streaming radio | Inline audio or no audio at all | Much better -- full streaming pipeline |
| **Networking** | TLS 1.3 (embedded-tls), HTTP client, FTP | HTTP only (sceHttp), no TLS | Much better -- modern TLS on 2006 hardware |
| **Error handling** | Graceful fallbacks (display list overflow -> skip, missing module -> fallback) | Often panics or undefined behavior | Better -- production quality |
| **Documentation** | SAFETY comments on every unsafe block, MEMORY.md, design docs | Minimal comments | Much better -- exceptional |
| **Build system** | Cargo + rust-psp + workspace | Makefile + pspsdk | Comparable -- different ecosystems |
| **Binary size** | ~2-4MB EBOOT, ~60KB PRX | ~200KB-2MB typical | Larger EBOOT (Rust runtime), competitive PRX |
| **Display list** | 256KB static BSS | 16-64KB typical | Generous but safe |
| **ME core** | VFPU YUV->RGB, sceVideocodec H.264 | Rarely used (complex) | Advanced -- few homebrew projects use ME |
| **Plugin (PRX)** | Kernel overlay with syscall hook, runtime NID resolution | Fixed NID tables, no fallbacks | Better -- CFW-portable |

### 5.2 Similar PSP Homebrew Projects

**For context, OASIS_OS occupies a unique niche.** Comparable PSP projects by scope:

| Project | Description | Similarity |
|---------|-------------|------------|
| **CXMB** (by Flavor/Davee) | Custom XMB theme plugin (PRX) | Similar PRX overlay approach but XMB-only |
| **PSPdisp** | Remote display server for PSP | Similar in using framebuffer hooking |
| **LuaPlayer/LuaIDE** | Scriptable PSP shell | Similar terminal/scripting concept |
| **PSPBrowser** | Web browser for PSP (C++) | Similar browser engine goal, but native C++ |
| **DaedalusX64** | N64 emulator (C++) | Similar in ME core usage and VFPU optimization |
| **ONScripter** | Visual novel engine (C++) | Similar in full UI framework on PSP |

**Key differentiation**: OASIS_OS is the **only known PSP homebrew project in Rust** with a full OS-like shell (window manager, VFS, terminal, browser, apps). Most PSP homebrew is single-purpose (emulator, media player, or single game).

---

## 6. Actionable Improvements

### 6.1 DRY (Don't Repeat Yourself)

#### D1. Extract shared shape algorithms into `oasis-shapes` crate
**Priority**: Medium | **Effort**: 4-6 hours | **Impact**: Reduces ~400 LOC duplication

`oasis-backend-sdl/src/shapes.rs` (623 LOC) and `oasis-backend-psp/src/shapes.rs` (571 LOC) share >60% of their algorithm logic (fill_triangle, arc calculation, rounded_rect scanline logic). The algorithms differ only in final pixel-write primitive.

**Action**: Extract common geometry calculations (scanline computation, arc points, corner insets, `isqrt_i32`) into `oasis-shapes` or `oasis-types::geometry`. Each backend provides a `PixelWriter` trait impl for final pixel output.

#### D2. Consolidate input dispatch patterns
**Priority**: Low | **Effort**: 3-4 hours | **Impact**: Reduces ~300 LOC duplication

`oasis-backend-psp/src/input_dispatch.rs` (1,027 LOC) and `oasis-backend-wasm/src/input_dispatch.rs` (527 LOC) share routing patterns. Common input routing logic (focus management, event bubbling, key-to-action mapping) could move to `oasis-types` or a new `oasis-input` module.

#### D3. Standardize feature flag naming
**Priority**: Low | **Effort**: 30 minutes | **Impact**: Clarity

Rename `oasis-backend-wasm`'s `youtube` feature to `wasm-youtube` to match the oasis-core feature name, or vice versa.

#### D4. Deduplicate backend WASM Cargo.toml dependencies
**Priority**: Low | **Effort**: 15 minutes | **Impact**: Maintenance clarity

Remove redundant direct dependencies in `oasis-backend-wasm/Cargo.toml` that are already transitive through `oasis-core`. Add a comment explaining any that must remain explicit.

---

### 6.2 Modularity

#### M1. Extract dashboard, startmenu, taskbar, statusbar from oasis-core
**Priority**: High | **Effort**: 8-12 hours | **Impact**: 38% reduction in oasis-core size

These are self-contained UI modules:
- `dashboard/mod.rs` (1,130 LOC) -> `oasis-dashboard`
- `startmenu.rs` (825 LOC) -> `oasis-startmenu`
- `taskbar.rs` (812 LOC) -> `oasis-taskbar`
- `statusbar.rs` (799 LOC) -> `oasis-statusbar`

Each depends only on oasis-types, oasis-sdi, oasis-skin, and oasis-ui. No circular dependency risk.

#### M2. Extract plugin system from oasis-core
**Priority**: Medium | **Effort**: 4-6 hours | **Impact**: Cleaner plugin architecture

`plugin/manager.rs` (1,271 LOC) -> `oasis-plugin` crate. The plugin system is a self-contained subsystem with its own lifecycle, WASM loading, and trait interfaces.

#### M3. Extract on-screen keyboard from oasis-core
**Priority**: Medium | **Effort**: 3-4 hours | **Impact**: Reusable component

`osk/keyboard.rs` (816 LOC) -> `oasis-osk` crate. The OSK is a standalone input widget that could be reused by any backend.

#### M4. Split `oasis-types/src/backend/mod.rs` into module files
**Priority**: Medium | **Effort**: 2-3 hours | **Impact**: Readability of 1,835-line file

Split into:
- `backend/core.rs` -- `SdiCore` trait (13 required methods)
- `backend/extended.rs` -- `SdiBackend` blanket extensions
- `backend/shapes.rs` -- `SdiShapes` trait
- `backend/gradients.rs` -- `SdiGradients` trait
- `backend/text.rs` -- `SdiText` trait
- `backend/vector.rs` -- `SdiVector` trait
- `backend/input.rs` -- `InputBackend`
- `backend/network.rs` -- `NetworkBackend`
- `backend/audio.rs` -- `AudioBackend`
- `backend/mod.rs` -- re-exports only

#### M5. Split `oasis-video/src/demux_lite.rs` into modules
**Priority**: Low | **Effort**: 2-3 hours | **Impact**: Readability of 1,834-line file

Split by MP4 box type: `moov.rs`, `mdat.rs`, `stbl.rs` (sample tables), `track.rs`.

#### M6. Add event handling to Widget trait
**Priority**: Low | **Effort**: 8-12 hours | **Impact**: Better widget composability

Current `Widget` trait has only `measure()` + `draw()`. Adding `fn handle_event(&mut self, event: &InputEvent) -> EventResult` would allow natural composition without external routing. This is a larger architectural change requiring careful design.

---

### 6.3 Robustness

#### R1. Enforce `unwrap_used` lint or clean up violations
**Priority**: High | **Effort**: 8-16 hours | **Impact**: Prevents future panics in production code

Options:
1. **Incremental**: Add `#[allow(clippy::unwrap_used)]` to test modules, then upgrade lint to `deny` for non-test code
2. **Full cleanup**: Replace production `unwrap()` calls with `?`, `.unwrap_or_default()`, or explicit error handling. Focus on the ~500 non-test occurrences.

#### R2. Replace `panic!("expected ...")` in tests with assert macros
**Priority**: Medium | **Effort**: 2-3 hours | **Impact**: Better test diagnostics

40+ test functions use `panic!("expected ...")` for pattern matching. Replace with:
```rust
assert!(matches!(result, CommandOutput::Text(_)), "expected Text, got {result:?}");
```
Or use `assert_matches!` from the `assert_matches` crate.

#### R3. Add screenshot regression baseline comparison
**Priority**: Medium | **Effort**: 4-6 hours | **Impact**: Catches visual regressions automatically

Current screenshots are uploaded as artifacts but never compared. Options:
- Store baseline PNGs in Git LFS, use `pixelmatch` or `image-diff` in CI to auto-fail on >1% pixel difference
- GitHub Actions comment with visual diff on PRs

#### R4. Add tests for oasis-backend-wasm
**Priority**: Medium | **Effort**: 6-8 hours | **Impact**: 5,483 LOC currently untested

At minimum:
- Unit tests for `WasmBackend` initialization
- Input dispatch tests (DOM event -> InputEvent mapping)
- Canvas drawing tests (mock canvas context)
- Playwright E2E tests for `www/index.html`

#### R5. Add tests for oasis-app-tv-guide
**Priority**: Medium | **Effort**: 4-6 hours | **Impact**: Streaming/playback logic untested

- Channel switching tests with mock HTTP responses
- StreamingBuffer tests (sliding window, probe mode, throttling)
- CDN failover tests (redirect chain handling)

#### R6. Add GU display list overflow detection
**Priority**: Low | **Effort**: 1-2 hours | **Impact**: Better debugging on PSP

`sceGuGetMemory` can return NULL when the display list is full. Currently silently ignored. Add a debug counter or log warning when vertices are dropped.

#### R7. Set minimum test coverage gate in CI
**Priority**: Low | **Effort**: 1 hour | **Impact**: Prevents coverage regression

Add `cargo llvm-cov --fail-under-lines 60` (or appropriate threshold) to main-ci.yml.

#### R8. Add benchmark regression detection
**Priority**: Low | **Effort**: 3 hours | **Impact**: Catches performance regressions

Use `criterion --save-baseline main` and compare in PR CI. Warn on >10% regression.

---

### 6.4 Features

#### F1. CSS @media queries support
**Priority**: Medium | **Effort**: 8-12 hours | **Impact**: Browser compatibility

The browser engine is missing @media queries. Given that skins can set different resolutions (480x272 PSP, 800x600 modern, 1024x768 XP), responsive CSS would enable content to adapt to the active skin's resolution.

#### F2. CSS Flexbox in browser
**Priority**: Low | **Effort**: 16-24 hours | **Impact**: Modern web layout

Currently the browser uses `oasis-ui` FlexContainer as a workaround. Native CSS Flexbox would improve web compatibility significantly.

#### F3. Plugin system integration test in CI
**Priority**: Medium | **Effort**: 2-3 hours | **Impact**: Plugin lifecycle validation

Create a test plugin, verify load/init/tick/shutdown lifecycle, and add to main-ci.yml.

#### F4. WASM E2E tests with Playwright
**Priority**: Medium | **Effort**: 6-8 hours | **Impact**: Web deployment confidence

Add Playwright tests for `www/index.html` verifying:
- Canvas renders (screenshot + hash comparison)
- Keyboard/mouse input reaches the OASIS instance
- Audio playback (Web Audio API)

#### F5. Network mock server tests for oasis-net
**Priority**: Medium | **Effort**: 4-6 hours | **Impact**: TCP/FTP/TLS testing

Add fixture-based tests with mock TCP server for:
- Connection establishment / timeout handling
- TLS handshake (with test certificates)
- FTP file transfer lifecycle
- Remote terminal protocol

#### F6. PSP screenshot diff testing via PPSSPP
**Priority**: Low | **Effort**: 4-6 hours | **Impact**: PSP visual regression detection

PPSSPP headless currently checks "no crash" only. Add screenshot capture at specific frames and compare against baselines.

#### F7. Improve documentation freshness
**Priority**: Low | **Effort**: 2-3 hours | **Impact**: Reduces confusion

- Update `prd-oasis-video-integration.md` (marked TODO but desktop streaming works)
- Archive or mark completed plans (PSP modernization, app extraction)
- Add completion status headers to plan documents

#### F8. Nightly streaming/integration tests
**Priority**: Low | **Effort**: 3-4 hours | **Impact**: Continuous validation

Move `scripts/test-tv-streaming.sh` into a nightly CI workflow. This validates real-world HTTP/HTTPS streaming continues to work as upstream servers change.

---

## Summary: Priority Matrix

### Critical Path (do first)

| ID | Item | Effort | Impact |
|----|------|--------|--------|
| R1 | Enforce unwrap_used lint | 8-16h | Prevents production panics |
| M1 | Extract dashboard/startmenu/taskbar/statusbar | 8-12h | 38% oasis-core reduction |
| R4 | Add WASM backend tests | 6-8h | 5.5K LOC uncovered |

### High Value (do soon)

| ID | Item | Effort | Impact |
|----|------|--------|--------|
| M4 | Split backend/mod.rs into modules | 2-3h | 1,835 LOC file readability |
| R3 | Screenshot regression baselines | 4-6h | Auto-catches visual regressions |
| R5 | TV Guide streaming tests | 4-6h | Critical feature untested |
| D1 | Extract shared shape algorithms | 4-6h | 400 LOC dedup |
| M2 | Extract plugin system | 4-6h | Cleaner architecture |
| R2 | Replace panic! in tests | 2-3h | Better test diagnostics |

### Nice to Have (when time permits)

| ID | Item | Effort | Impact |
|----|------|--------|--------|
| F1 | CSS @media queries | 8-12h | Browser responsive layout |
| F3 | Plugin integration test in CI | 2-3h | Lifecycle validation |
| F5 | Network mock server tests | 4-6h | TCP/FTP/TLS coverage |
| M3 | Extract OSK | 3-4h | Reusable component |
| M5 | Split demux_lite.rs | 2-3h | Readability |
| D2 | Consolidate input dispatch | 3-4h | Code sharing |
| D3 | Feature flag naming | 30m | Clarity |
| F4 | WASM Playwright E2E tests | 6-8h | Web confidence |
| F7 | Documentation freshness | 2-3h | Reduces confusion |
| R7 | Coverage minimum gate | 1h | Prevents regression |
| R8 | Benchmark regression | 3h | Performance tracking |
| F8 | Nightly streaming tests | 3-4h | Continuous validation |
| R6 | GU display list overflow detection | 1-2h | PSP debugging |
| F2 | CSS Flexbox | 16-24h | Web compatibility |
| M6 | Widget event handling trait | 8-12h | Composability |
| F6 | PSP screenshot diff | 4-6h | Visual regression |
