# OASIS OS Comprehensive Improvements Plan

> **Branch:** `feat/comprehensive-improvements`
> **Author:** Automated audit + synthesis
> **Date:** 2026-02-19
> **Status:** DRAFT - Pending review

---

## Audit Summary

A thorough review of the entire codebase (95K+ LOC, 18 crates) was conducted across
7 dimensions: backend parity, test coverage, code quality, documentation, browser engine,
UI framework, and terminal/VFS. The codebase is production-grade with excellent fundamentals
(zero unwrap/panic/todo in workspace, 2,214 passing tests, zero clippy warnings). The
improvements below address the gaps found.

---

## Phase 1: Backend Parity & Desktop Hardening

**Goal:** Bring SDL and UE5 backends to feature parity with PSP where it makes sense.

### 1.1 SDL Backend: Networking (NetworkBackend impl)

**Current state:** SDL has 0/4 NetworkBackend methods implemented.
**Impact:** Desktop/Pi users cannot use remote terminal, FTP, or network features.

- Implement `listen()`, `accept()`, `connect()` using `std::net`
- Implement `tls_provider()` using rustls (already a workspace dependency)
- Wire up to oasis-net for remote terminal and FTP support
- Add tests for connection lifecycle, error handling, TLS handshake

**Files:** `crates/oasis-backend-sdl/src/lib.rs` (new network section)
**Estimate:** ~400 LOC, 20+ tests

### 1.2 SDL Backend: Audio Streaming

**Current state:** SDL audio plays files but streaming returns error stub.
**Impact:** Internet radio and streaming MP3 don't work on desktop.

- Implement `start_stream()` / `feed_stream()` / `stop_stream()` using SDL_mixer
- Buffer management for continuous playback
- Add tests with mock audio data

**Files:** `crates/oasis-backend-sdl/src/sdl_audio.rs`
**Estimate:** ~200 LOC, 10+ tests

### 1.3 UE5 Backend: Audio Stub Improvement

**Current state:** All 12 AudioBackend methods return "not supported" error.
**Impact:** Music Player app shows errors instead of graceful "no audio" state.

- Change error stubs to return `Ok(())` silently for play/pause/stop/volume
- Return meaningful defaults for queries (position_ms -> 0, is_playing -> false)
- Add `has_audio()` capability query so UI can hide audio controls
- Keep load_track as error (no data to load)

**Files:** `crates/oasis-backend-ue5/src/renderer.rs`
**Estimate:** ~50 LOC, 10+ tests

### 1.4 PSP Backend: Extended Shape Rendering

**Current state:** 14 extended rendering methods use trait defaults (fill_rect fallback).
**Impact:** Circles render as squares, no gradients, no rounded rects on PSP.

- Implement `fill_rounded_rect()` using GU line strips
- Implement `fill_circle()` using GU triangle fan
- Implement `draw_line()` using GU line primitive
- Implement vertical/horizontal gradients using GU vertex colors
- Keep complex methods (blit_sub, clip stacks) as defaults

**Files:** `crates/oasis-backend-psp/src/lib.rs`, `render.rs`
**Estimate:** ~300 LOC

---

## Phase 2: Test Infrastructure & Coverage

**Goal:** Close critical coverage gaps and integrate existing test infrastructure into CI.

### 2.1 Integrate Visual Regression Tests into CI

**Current state:** `screenshot_tests.rs` exists but is manual-only.
**Impact:** Visual glitches ship undetected.

- Add CI step: `cargo run -p oasis-app --bin screenshot-tests -- --check`
- Store golden baselines in `screenshots/golden/` (git-tracked)
- Generate HTML diff report on failure
- Add `--bless` workflow for updating baselines after intentional changes

**Files:** `.github/workflows/ci.yml`, `crates/oasis-app/src/screenshot_tests.rs`
**Estimate:** ~100 LOC CI config, ~50 LOC test harness updates

### 2.2 Regenerate All Screenshots

**Current state:** Screenshots dated Feb 12, 2025; user reports visual glitches.
**Impact:** README shows outdated/glitchy images.

- Run `cargo run -p oasis-app --bin oasis-screenshot` for all 13 skins
- Review each screenshot for rendering artifacts
- Fix any rendering bugs discovered (likely off-by-one issues found in audit)
- Update golden baselines
- Commit fresh screenshots

**Files:** `screenshots/*/` (32 PNG files)

### 2.3 oasis-app Test Coverage (Currently 7%)

**Current state:** input.rs (458 LOC, 0 tests), commands.rs (331 LOC, 0 tests),
main.rs (409 LOC, 0 tests).
**Impact:** Core user-facing code paths untested.

- Add input dispatch tests with mock backend (event routing, button mapping)
- Add command handler tests (argument parsing, error cases)
- Add app lifecycle tests (init, tick, shutdown)
- Target: 25+ new tests, bring density to ~20 tests/1K LOC

**Files:** `crates/oasis-app/src/input.rs`, `commands.rs`
**Estimate:** ~500 LOC of tests

### 2.4 Run Benchmarks in CI

**Current state:** 6 benchmarks exist (browser: 4, VFS: 1, SDI: 1) but not in CI.
**Impact:** Performance regressions go undetected.

- Add CI step to run benchmarks and store results
- Fail on >20% regression (configurable threshold)
- Track benchmark history over time

**Files:** `.github/workflows/ci.yml`
**Estimate:** ~30 LOC CI config

### 2.5 Coverage Metrics Reporting

**Current state:** No coverage metrics tracked.
**Impact:** Can't measure improvement or catch coverage drops.

- Add `cargo-tarpaulin` or `cargo-llvm-cov` to CI
- Generate and upload coverage report
- Set minimum threshold (currently ~23 tests/1K LOC workspace-wide)

**Files:** `.github/workflows/ci.yml`
**Estimate:** ~20 LOC CI config

---

## Phase 3: UI Framework Completeness

**Goal:** Add missing common UI patterns and fix visual glitches.

### 3.1 Dropdown/Combobox Widget

**Current state:** No dropdown widget exists.
**Impact:** Settings app and other UIs can't offer selection lists.

- Implement `Dropdown` widget with:
  - Collapsed state showing selected value
  - Expanded state with scrollable option list
  - Keyboard navigation (up/down/enter/escape)
  - Theme integration
- Add 15+ tests

**Files:** New `crates/oasis-ui/src/dropdown.rs`
**Estimate:** ~300 LOC, 15+ tests

### 3.2 Modal Dialog System

**Current state:** No modal blocking. Confirmations not possible.
**Impact:** Destructive actions (delete file, format) can't ask for confirmation.

- Implement `ModalDialog` with:
  - Input blocking for underlying windows
  - OK/Cancel/Custom button sets
  - Title + message + optional input field
  - Centered positioning with dimmed backdrop
- Integrate with window manager (new `WindowType::Modal`)
- Add 10+ tests

**Files:** New `crates/oasis-ui/src/modal.rs`, updates to `oasis-wm`
**Estimate:** ~400 LOC, 10+ tests

### 3.3 Checkbox and Radio Button Widgets

**Current state:** No boolean/exclusive-choice input widgets.
**Impact:** Settings UI uses Toggle for everything.

- Implement `Checkbox` (independent boolean selection)
- Implement `RadioGroup` (exclusive selection from N options)
- Theme-consistent rendering
- Add 10+ tests each

**Files:** New `crates/oasis-ui/src/checkbox.rs`, `radio.rs`
**Estimate:** ~250 LOC total, 20+ tests

### 3.4 Fix Visual Glitch Sources

**Current state:** Audit identified 5 off-by-one rendering issues.

Fixes needed:
1. **TabBar tab width** - distribute remainder pixels across first N tabs
2. **Button text centering** - round up on odd dimensions
3. **ScrollView thumb** - clamp thumb_y to prevent overrun at scroll limits
4. **ListView clipping** - exact visible count instead of +2 buffer
5. **Window titlebar buttons** - replace magic constant with named padding

**Files:** `crates/oasis-ui/src/tabbar.rs`, `button.rs`, `scrollbar.rs`,
`listview.rs`, `crates/oasis-wm/src/window.rs`
**Estimate:** ~50 LOC changes, update existing tests

### 3.5 Keyboard Navigation

**Current state:** No Tab cycling or arrow key navigation between widgets.
**Impact:** Keyboard-only users (and PSP D-pad) can't navigate UI elements.

- Add focus tracking to widget system (focused widget index)
- Tab/Shift-Tab to cycle focus between interactive widgets
- Arrow keys for in-widget navigation (list items, tabs)
- Visual focus indicator (highlight ring or color change)
- Add 15+ tests

**Files:** `crates/oasis-ui/src/lib.rs` (focus system), widget files
**Estimate:** ~400 LOC, 15+ tests

---

## Phase 4: Browser Engine Modernization

**Goal:** Bring CSS support from CSS 2.1 to practical CSS 3 subset.

### 4.1 CSS Flexbox Layout

**Current state:** `display: flex` not supported. ~30% of modern web layouts broken.
**Impact:** Most modern websites render incorrectly.

- Implement flex container layout algorithm (CSS Flexible Box Level 1):
  - `flex-direction`: row, column, row-reverse, column-reverse
  - `flex-wrap`: nowrap, wrap
  - `justify-content`: flex-start, flex-end, center, space-between, space-around
  - `align-items`: flex-start, flex-end, center, stretch, baseline
  - `flex-grow`, `flex-shrink`, `flex-basis`
  - `order` property
  - `align-self` per-item override
- Wire into layout engine alongside existing block/inline/table
- Add 50+ tests and benchmark

**Files:** New `crates/oasis-browser/src/layout/flex.rs`, updates to `layout/mod.rs`
**Estimate:** ~2000 LOC, 50+ tests

### 4.2 CSS Positioning (absolute, fixed, relative)

**Current state:** `position` property not implemented. Modals, overlays, sticky
headers all broken.
**Impact:** Many sites use absolute/fixed positioning for navigation, popups.

- Implement `position: relative` (offset from normal flow)
- Implement `position: absolute` (relative to positioned ancestor)
- Implement `position: fixed` (relative to viewport)
- Properties: `top`, `left`, `right`, `bottom`, `z-index`
- Stacking context management
- Add 30+ tests

**Files:** Updates to `crates/oasis-browser/src/layout/block.rs`,
new `positioning.rs`
**Estimate:** ~1500 LOC, 30+ tests

### 4.3 Additional CSS Selectors

**Current state:** Missing attribute selectors, sibling combinators,
functional pseudo-classes.
**Impact:** CSS framework styles (Bootstrap, Tailwind) partially broken.

- Implement attribute selectors: `[href]`, `[data-x="y"]`, `[class~="word"]`
- Implement adjacent sibling: `div + p`
- Implement general sibling: `div ~ p`
- Implement `:not()`, `:nth-child()`, `:nth-of-type()`
- Add 20+ tests per selector type

**Files:** `crates/oasis-browser/src/css/cascade.rs`, `selector.rs`
**Estimate:** ~800 LOC, 60+ tests

### 4.4 Image Format Support

**Current state:** Only BMP decoded by default. PNG/JPEG show broken placeholders.
**Impact:** Nearly all web images are unviewable.

- Add `png` crate for PNG decoding
- Add `jpeg-decoder` crate for JPEG decoding
- Wire into `image.rs` decode pipeline
- Handle progressive JPEG, interlaced PNG
- Respect max image dimensions (480x480 cap)
- Add tests with sample images

**Files:** `crates/oasis-browser/src/image.rs`, `Cargo.toml`
**Estimate:** ~200 LOC, 15+ tests

---

## Phase 5: Documentation & Developer Experience

**Goal:** Fill documentation gaps and improve onboarding.

### 5.1 Getting Started Guide

**Current state:** No standalone dev setup guide.
**Impact:** New contributors struggle to set up environment.

- Create `docs/getting-started.md` covering:
  - System requirements (Rust 1.91+, SDL2 dev libs)
  - Clone, build, run (desktop)
  - Run tests
  - Take screenshots
  - Docker-based development
  - PSP cross-compilation setup

**Estimate:** ~200 lines

### 5.2 FFI Integration Guide

**Current state:** FFI API listed in AGENTS.md but no standalone guide.
**Impact:** UE5/C++ developers can't easily integrate OASIS_OS.

- Create `docs/ffi-integration.md` covering:
  - C header equivalent
  - Lifecycle (create -> tick -> destroy)
  - Input feeding
  - Buffer reading
  - VFS setup
  - Callback registration
  - Example C/C++ integration code

**Estimate:** ~300 lines

### 5.3 Examples Directory

**Current state:** Zero runnable examples.
**Impact:** Developers learn by example; none available.

- Create `examples/` with:
  - `minimal_sdl.rs` - bare minimum SDL app
  - `custom_skin.rs` - loading a TOML skin
  - `headless_screenshot.rs` - render to PNG without display
  - `ffi_demo.c` - C program using oasis-ffi

**Estimate:** ~400 LOC total

### 5.4 Architecture Decision Records

**Current state:** Major design decisions undocumented.
**Impact:** "Why was X chosen over Y?" questions unanswerable.

- Create `docs/adr/` directory with records for:
  - ADR-001: Arena-based DOM vs reference-counted nodes
  - ADR-002: VFS abstraction vs direct filesystem access
  - ADR-003: Backend trait design (4 traits vs single trait)
  - ADR-004: PSP two-binary architecture (EBOOT + PRX)
  - ADR-005: TOML skin system vs compiled themes

**Estimate:** ~100 lines each, 5 records

---

## Phase 6: Terminal & Shell Enhancements

**Goal:** Fill the most impactful shell feature gaps.

### 6.1 Extended Glob Patterns

**Current state:** Only `*` and `?` supported.
**Impact:** `{a,b}` brace expansion and `[a-z]` character classes not available.

- Implement brace expansion: `file.{rs,toml}` -> `file.rs file.toml`
- Implement character classes: `[a-z]`, `[0-9]`, `[!abc]`
- Add 15+ tests

**Files:** `crates/oasis-terminal/src/interpreter.rs`
**Estimate:** ~200 LOC, 15+ tests

### 6.2 Stderr Separation

**Current state:** All output merged into stdout. No `2>` redirect.
**Impact:** Can't separate error messages from command output in pipelines.

- Add `stderr` field to `CommandOutput`
- Implement `2>` and `2>&1` redirect syntax
- Commands emit errors to stderr channel
- Add 10+ tests

**Files:** `crates/oasis-terminal/src/interpreter.rs`, command modules
**Estimate:** ~300 LOC, 10+ tests

### 6.3 Shell Functions

**Current state:** No user-defined functions.
**Impact:** Complex scripts require separate files for reuse.

- Implement `function name() { ... }` definition syntax
- Local variable scope within functions
- Return value via exit code
- Recursive calls (with depth limit)
- Add 10+ tests

**Files:** `crates/oasis-terminal/src/interpreter.rs`
**Estimate:** ~250 LOC, 10+ tests

---

## Phase 7: Window Manager Polish

**Goal:** Professional-quality window management behavior.

### 7.1 Screen Bounds Enforcement

**Current state:** Windows can be dragged off-screen.
**Impact:** Users lose windows, especially on 480x272 display.

- Clamp window position to keep at least titlebar visible
- Prevent resize below minimum dimensions
- Snap-to-edge when within 8px of screen boundary
- Add 10+ tests

**Files:** `crates/oasis-wm/src/lib.rs`
**Estimate:** ~100 LOC, 10+ tests

### 7.2 Titlebar Gradient Rendering

**Current state:** Theme has gradient fields but they're unused. Solid colors only.
**Impact:** Classic skin styles (XP, macOS) look flat.

- Wire existing `WmTheme` gradient fields to actual rendering
- Implement horizontal gradient fill in titlebar
- Active vs inactive window gradients
- Add 5+ tests

**Files:** `crates/oasis-wm/src/window.rs`
**Estimate:** ~80 LOC, 5+ tests

### 7.3 Always-on-Top and Modal Flags

**Current state:** No z-order pinning. No modal input blocking.
**Impact:** Dialogs don't block parent. Floating widgets sink.

- Add `always_on_top` flag to `WindowConfig`
- Add `modal` flag that blocks input to windows below
- Dim background behind modal windows
- Add 10+ tests

**Files:** `crates/oasis-wm/src/lib.rs`, `window.rs`
**Estimate:** ~150 LOC, 10+ tests

---

## Phase 8: Performance & Robustness

**Goal:** Measurable performance improvements and hardening.

### 8.1 Incremental Layout in Browser

**Current state:** Full relayout on every change.
**Impact:** Dynamic content updates are expensive (unnecessary on PSP).

- Add dirty flag to layout boxes
- Only relayout subtrees that changed
- Cache computed styles
- Benchmark improvement

**Files:** `crates/oasis-browser/src/layout/`
**Estimate:** ~500 LOC

### 8.2 Property-Based Testing Expansion

**Current state:** proptest used in VFS and browser, but not broadly.
**Impact:** Edge cases in layout, CSS cascade, terminal parsing may lurk.

- Add proptest generators for:
  - CSS property values (random valid/invalid CSS)
  - HTML document fragments
  - Terminal command strings
  - VFS path sequences
- Target: 30+ new property-based tests

**Files:** Various test modules
**Estimate:** ~400 LOC of tests

### 8.3 Fuzz Testing in CI

**Current state:** 6 fuzz targets exist but never run in CI.
**Impact:** Parser robustness only verified manually.

- Add nightly CI job for 60-second fuzz runs per target
- Store and commit new corpus entries
- Alert on crashes

**Files:** `.github/workflows/ci.yml` or new `fuzz.yml`
**Estimate:** ~50 LOC CI config

---

## Phase Summary & Priority

| Phase | Focus Area | New LOC | New Tests | Priority |
|-------|-----------|---------|-----------|----------|
| **1** | Backend parity (SDL net, audio; UE5 audio; PSP shapes) | ~950 | 40+ | **HIGH** |
| **2** | Test infrastructure (CI screenshots, app tests, coverage) | ~700 | 25+ | **HIGH** |
| **3** | UI widgets (dropdown, modal, checkbox, radio, fixes) | ~1,400 | 60+ | **HIGH** |
| **4** | Browser CSS3 (flex, position, selectors, images) | ~4,500 | 155+ | **MEDIUM** |
| **5** | Documentation (getting started, FFI, examples, ADRs) | ~1,200 | 0 | **MEDIUM** |
| **6** | Terminal enhancements (globs, stderr, functions) | ~750 | 35+ | **LOW** |
| **7** | Window manager polish (bounds, gradients, modals) | ~330 | 25+ | **LOW** |
| **8** | Performance & robustness (incremental layout, proptest, fuzz) | ~950 | 30+ | **LOW** |
| | **TOTALS** | **~10,800** | **370+** | |

---

## Execution Order Recommendation

```
Phase 1.1-1.3  (Backend parity: SDL net, SDL streaming, UE5 audio)
    |
Phase 2.2      (Regenerate screenshots, fix rendering bugs found)
    |
Phase 3.4      (Fix visual glitch sources identified in audit)
    |
Phase 2.1      (Integrate visual regression tests into CI)
    |
Phase 3.1-3.3  (New widgets: dropdown, modal, checkbox, radio)
    |
Phase 3.5      (Keyboard navigation)
    |
Phase 1.4      (PSP extended shapes)
    |
Phase 2.3-2.5  (App tests, benchmarks in CI, coverage)
    |
Phase 4.4      (Image format support - quick win)
    |
Phase 4.1-4.2  (Flexbox + positioning - large effort)
    |
Phase 4.3      (CSS selectors)
    |
Phase 5.1-5.4  (Documentation)
    |
Phase 6-8      (Terminal, WM polish, performance)
```

---

## Key Findings from Audit

### Strengths (preserve these)
- Zero unwrap/panic/todo/println in 95K LOC workspace -- exceptional discipline
- 2,214 passing tests with zero failures
- Clean 16-crate architecture with clear separation of concerns
- Backend trait abstraction is well-designed and consistent
- VFS security is bulletproof (path traversal protection verified)
- Terminal has no injection vulnerabilities
- Error handling is comprehensive throughout

### Weakest Areas (address first)
1. **SDL backend missing networking** -- desktop users can't use remote terminal/FTP
2. **Visual regression tests not in CI** -- rendering bugs ship undetected
3. **oasis-app has 7% test density** -- core user-facing code barely tested
4. **Browser lacks flexbox/positioning** -- most modern websites render wrong
5. **No dropdown/modal widgets** -- basic UI patterns missing
6. **Screenshots may have visual glitches** -- identified 5 off-by-one rendering issues
7. **UE5 audio stubs return errors** -- should return silent success instead
