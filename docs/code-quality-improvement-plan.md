# Code Quality Improvement Plan

Comprehensive plan for improving code quality, DRY, modularity, and robustness across the OASIS_OS codebase. Generated from a deep dive analysis of all 20+ workspace crates.

**Current state:** The codebase is well-architected overall (8/10 modularity score). No circular dependencies, clean trait hierarchy, good error types, excellent unsafe documentation. The improvements below target specific, actionable gaps.

---

## Phase 1: DRY — Extract Shared Backend Primitives

**Impact: HIGH | ~700+ duplicated lines across 4 backends**

### 1.1 Extract software rasterization to shared module

All four backends (SDL, UE5, WASM, PSP) re-implement nearly identical algorithms for:
- `fill_rounded_rect` (~50-70 lines each, 4 copies)
- `draw_line` (Bresenham's algorithm, 4 copies)
- `fill_circle` / `draw_circle` (arc-drawing, 4 copies)
- `fill_rect_gradient` (color interpolation loop, 3 copies ~50 lines each)

**Action:** Create `oasis-types/src/rasterize.rs` (or a new `oasis-raster` crate) with:
- `fn software_rounded_rect(buf, x, y, w, h, radius, color)` — pixel-buffer rasterizer
- `fn software_line(buf, x0, y0, x1, y1, color)` — Bresenham
- `fn software_circle(buf, cx, cy, r, color, fill: bool)`
- `fn software_gradient(buf, x, y, w, h, style, color_start, color_end)`

Each backend calls these shared implementations in their `SdiBackend` default methods. Backends with hardware acceleration (SDL via SDL3, WASM via Canvas2D, PSP via GU) override with native calls.

**Files affected:**
- `crates/oasis-backend-sdl/src/shapes.rs:66-660`
- `crates/oasis-backend-ue5/src/renderer.rs:463-750`
- `crates/oasis-backend-wasm/src/renderer.rs:459-750`
- `crates/oasis-backend-psp/src/shapes.rs:180-410`

### 1.2 Consolidate duplicated utility functions

| Function | Locations | Action |
|----------|-----------|--------|
| `lerp_color` | SDL `shapes.rs:648`, UE5 `renderer.rs:63` | Move to `oasis-types::Color::lerp()` method |
| `intersect_clip` | SDL `shapes.rs:602`, UE5 `renderer.rs:73` | Move to `oasis-types::Rect::intersect()` |
| `ClipRect` struct | SDL `lib.rs:54`, UE5 `renderer.rs:45`, WASM `renderer.rs:101` | Define once in `oasis-types::backend` |
| `resolve_path` | Terminal `expander.rs:354`, Browser `loader/mod.rs:335` | Consolidate in `oasis-vfs` or `oasis-types` |

### 1.3 Deduplicate backend test assertions

UE5 and SDL backends have near-identical tests for `intersect_clip`, `lerp_color`, etc. Once utilities move to `oasis-types`, tests move there too — eliminating ~200 lines of duplicated test code.

---

## Phase 2: DRY — Terminal Command Boilerplate

**Impact: MEDIUM | ~800 lines of boilerplate**

### 2.1 Expand `define_command!` macro usage

A `define_command!` macro exists (`crates/oasis-terminal/src/command_macro.rs:29-57`) but is only used in 4 commands in `system_commands.rs`. Twenty+ commands in `commands.rs` still use manual trait implementations.

**Action:** Apply the macro to all command structs in:
- `crates/oasis-terminal/src/commands.rs` (20+ commands: Help, Ls, Cd, Pwd, Cat, etc.)
- Saves ~30-50 lines per command (name/description/usage/category boilerplate)

### 2.2 Apply test helper macros

`crates/oasis-terminal/src/test_helpers.rs:20` documents: *"TODO: Apply these macros to the remaining ~170 match/panic patterns across all test modules"*

**Action:** Replace manual `match output { ... => panic!() }` patterns with:
- `assert_text!(output, "expected")`
- `assert_clear!(output)`
- `assert_none_output!(output)`

Across 13 test modules: commands.rs, interpreter.rs, text_commands.rs, file_commands.rs, dev_commands.rs, fun_commands.rs, system_commands.rs, network_commands.rs, skin_commands.rs, ui_commands.rs, audio_commands.rs, doc_commands.rs, radio_commands.rs.

---

## Phase 3: Modularity — Split Large App Files

**Impact: MEDIUM-HIGH | 5 files >1700 lines**

### 3.1 Split `oasis-app-paint/src/lib.rs` (1873 lines)

Extract into:
- `canvas.rs` — layer management, pixel operations, undo/redo buffer
- `tools.rs` — Pencil, Line, Rectangle, Circle, Fill tool logic
- `palette.rs` — color selection, swatches, recent colors
- `lib.rs` — app coordinator, event dispatch (~300 lines)

### 3.2 Split `oasis-app-clock/src/lib.rs` (1772 lines)

Extract into:
- `timezones.rs` — 48+ timezone definitions and conversions
- `alarms.rs` — alarm state machine, scheduling
- `display.rs` — analog/digital clock rendering
- `lib.rs` — app coordinator (~300 lines)

### 3.3 Split `oasis-app-text-editor/src/lib.rs` (1707 lines)

Extract into:
- `buffer.rs` — text buffer management, line tracking
- `editor.rs` — cursor, selection, editing commands
- `render.rs` — viewport/window rendering logic
- `lib.rs` — app coordinator (~300 lines)

### 3.4 Split `oasis-app-tv-guide/src/guide.rs` (2326 lines)

Extract into:
- `grid_state.rs` — state machine, selection, scroll position
- `grid_render.rs` — SDI drawing, cell rendering
- `grid_layout.rs` — position calculations, VISIBLE_TIME_SLOTS, SLOT_DURATION
- `guide.rs` — coordinator (~400 lines)

### 3.5 Consider splitting `oasis-wm/src/manager.rs` (1843 lines)

Already has helper modules (`drag_resize.rs`, `hit_test.rs`, `window.rs`), but `manager.rs` still handles window lifecycle, input dispatch, cascade logic, and focus management. Consider extracting `input_dispatch.rs` and `cascade.rs`.

---

## Phase 4: Robustness — Error Handling & Safety

**Impact: MEDIUM | Prevents production panics**

### 4.1 Replace production `.expect()` calls with proper error handling

| File | Line | Issue | Fix |
|------|------|-------|-----|
| `terminal/line_edit.rs` | 1132 | `search_display().expect("in search")` | Return `Option`, handle `None` |
| `terminal/expander.rs` | 34 | `chars.next().expect("peek() confirmed")` | Add `debug_assert!` or use `if let` |
| `terminal/pipeline.rs` | 110 | Same peek/next pattern | Same fix |

### 4.2 Strengthen SDL texture lifetime safety

`crates/oasis-backend-sdl/src/lib.rs:67-88` uses `transmute` to erase `Texture` lifetimes, relying on field declaration order + explicit `Drop` impl for soundness.

**Action:**
- Add `#[doc(hidden)]` and prominent rustdoc warning about field order invariant
- Consider a `TextureStore` wrapper type that encapsulates the safety invariant
- Add a compile-time or runtime assertion that validates the Drop ordering

### 4.3 Tighten error types — reduce `Other(String)` fallback usage

The error system in `oasis-types/src/error.rs` has proper domain-specific variants but also `Other(String)` catch-alls in `SdiError`, `VfsError`, `CommandError`, `BackendError`.

**Action:** Audit call sites using `Other(String)` and convert the most common patterns to named variants. Goal: reduce `Other(String)` usage by 50%.

### 4.4 Audit and justify `#[allow(dead_code)]` annotations

26 files use `#[allow(dead_code)]`. Audit each:
- Remove annotations where the code is actually unused (delete the dead code)
- Add justification comments where the annotation is intentional (e.g., platform-conditional code)
- Key files: `oasis-sdi/src/registry.rs`, `oasis-browser/src/layout/table.rs`, `oasis-browser/src/layout/float.rs`

---

## Phase 5: Modularity — Public API Tightening

**Impact: LOW-MEDIUM | Improves encapsulation**

### 5.1 Restrict over-exposed internals

| File | Lines | Action |
|------|-------|--------|
| `oasis-skin/src/active_theme/derive.rs` | 1151 | Make ~40% of helper functions `pub(crate)` |
| `oasis-browser/src/css/values/apply.rs` | 1148 | Expose only `apply_styles()`, rest `pub(crate)` |
| `oasis-ui/src/flex.rs` | 934 | Consider hiding `ComputedRect`/`FlexChild` internals behind builder API |
| `oasis-browser/src/forms/manager.rs` | 1636 | Move `validate_field()`/`apply_preset()` to `pub(crate)` |

### 5.2 Consider optional app features in `oasis-core`

Allow users to exclude apps they don't need:
```toml
# oasis-core/Cargo.toml
[features]
apps-all = ["app-paint", "app-clock", "app-text-editor", ...]
app-paint = ["dep:oasis-app-paint"]
app-clock = ["dep:oasis-app-clock"]
# ...
```

This enables smaller builds for embedded deployments.

---

## Phase 6: Code Style — Magic Numbers & Constants

**Impact: LOW-MEDIUM | Improves readability**

### 6.1 Extract layout percentages to named constants

`crates/oasis-core/src/apps/runner.rs:1230-1232`:
```rust
// Before:
let header_h = (usable_h * 20 / 100).max(60);
let time_header_h = (usable_h * 4 / 100).max(20);

// After:
const HEADER_PERCENT: u32 = 20;
const HEADER_MIN_H: u32 = 60;
let header_h = (usable_h * HEADER_PERCENT / 100).max(HEADER_MIN_H);
```

### 6.2 Define default resolution constant

`480x272` (PSP native) appears 7+ times as raw literals in tests across `simple_app.rs`. Define:
```rust
// In oasis-types or oasis-core
pub const DEFAULT_WIDTH: u32 = 480;
pub const DEFAULT_HEIGHT: u32 = 272;
```

### 6.3 Centralize shared dimension constants

Multiple backends redefine `SCREEN_WIDTH`/`SCREEN_HEIGHT`/`CIRCLE_SEGMENTS`. Centralize in `oasis-types`.

---

## Phase 7: Test Coverage Gaps

**Impact: MEDIUM | Prevents regressions**

### 7.1 Window manager edge cases

`oasis-wm/` has limited input dispatch tests. Add tests for:
- Cascade positioning with >10 windows
- Focus switching with overlapping windows
- Z-order changes during drag operations
- Minimize/maximize/restore state transitions

### 7.2 Video streaming error handling

`oasis-video/` needs edge case tests for:
- Connection failures mid-stream
- Malformed MP4 atoms (truncated moov, invalid sizes)
- Seek to out-of-range positions
- Concurrent read/write race conditions on `StreamingBuffer`

### 7.3 Network layer mocks

`oasis-net/` has minimal tests. Add mock TCP backend for testing:
- PSK authentication handshake
- Remote terminal command dispatch
- FTP transfer error recovery

---

## Phase 8: Documentation

**Impact: LOW | Improves maintainability**

### 8.1 Document under-documented public APIs

Priority targets (public items with <2 doc lines per item):
- `oasis-core/src/terminal/agent_commands.rs` (7 public items, 10 doc lines)
- `oasis-core/src/terminal/plugin_commands.rs` (2 public items, 9 doc lines)

### 8.2 Add examples to UI widgets

Smaller widgets (`checkbox.rs`, `toggle.rs`, `radio.rs`) have adequate but minimal docs. Add usage examples in doc comments.

---

## Implementation Order & Dependencies

```
Phase 1.2 (utilities)  ──→ Phase 1.1 (rasterization) ──→ Phase 1.3 (tests)
Phase 2.1 (macro)      ──→ Phase 2.2 (test macros)
Phase 3.1-3.5          (independent, can parallelize)
Phase 4.1-4.4          (independent, can parallelize)
Phase 5.1              ──→ Phase 5.2
Phase 6.1-6.3          (independent, can parallelize)
Phase 7.1-7.3          (independent, can parallelize)
Phase 8.1-8.2          (independent, can parallelize)
```

**Recommended execution order:** Phase 4 (robustness) → Phase 1 (DRY backends) → Phase 3 (split files) → Phase 2 (terminal DRY) → Phase 6 (constants) → Phase 5 (API) → Phase 7 (tests) → Phase 8 (docs)

Rationale: Fix safety issues first, then reduce duplication (prevents duplicating fixes), then modularize (smaller files are easier to work with), then polish.

---

## Out of Scope (Already Good)

These areas were analyzed and found to be well-designed — no changes needed:

- **Backend trait hierarchy** (`SdiCore` → `SdiBackend` → extension traits) — exemplary design
- **Error type system** (`oasis-types/src/error.rs`) — structured with proper `thiserror` integration
- **Font rendering** — shared via `oasis-types::bitmap_font`, PSP variant justified
- **FFmpeg unsafe code** — all 30+ blocks have proper SAFETY comments
- **Streaming buffer thread safety** — correct Mutex/Condvar/Atomic patterns, no deadlock risks
- **Dependency graph** — no circular dependencies, clean hierarchy
- **SimpleApp pattern** — good DRY design for similar apps
- **Feature flags** — clean, well-documented, no entanglement
- **Code hygiene** — only 1 TODO, no FIXME/HACK, no debug prints in library code
- **Naming conventions** — consistent Rust style throughout
