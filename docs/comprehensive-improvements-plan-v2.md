# OASIS OS Comprehensive Improvements Plan v2 — Remaining Work

> **Status**: Most phases complete. This document tracks only the remaining incomplete items.
> Items from completed phases (1, 2, 6, 7, 8, 10, 13) have been removed — see git history for the original plan.

---

## Phase 3: Architecture — State Decomposition

**Goal:** Break up the `AppState` god object (30+ fields) into layered, testable service groups.

**Status:** NOT STARTED

### 3.1 Extract UI Layer

- Create `UiLayer` struct holding:
  - `dashboard: DashboardState`
  - `status_bar: StatusBar`
  - `bottom_bar: BottomBar`
  - `start_menu: StartMenuState`
  - `cursor: CursorState`
- Move related update/render methods into `UiLayer` impl
- `AppState` holds `ui: UiLayer` instead of 5 separate fields

**Files:** New `oasis-app/src/ui_layer.rs`, refactor `app_state.rs`

### 3.2 Extract System Layer

- Create `SystemLayer` struct holding:
  - `cmd_reg: CommandRegistry`
  - `cwd: String`
  - `input_buf: String`
  - `output_lines: Vec<String>`
  - `plugin_mgr: PluginManager`
- Shell/terminal operations route through this layer

**Files:** New `oasis-app/src/system_layer.rs`, refactor `app_state.rs`

### 3.3 Extract Network Layer

- Create `NetworkLayer` struct holding:
  - `net_backend: Box<dyn NetworkBackend>`
  - `listener: Option<RemoteListener>`
  - `ftp_server: Option<FtpServer>`
  - `remote_client: Option<RemoteClient>`
  - `tls_provider: Box<dyn TlsProvider>`
- Network operations self-contained and testable

**Files:** New `oasis-app/src/network_layer.rs`, refactor `app_state.rs`

### 3.4 Extract Content Layer

- Create `ContentLayer` struct holding:
  - `app_runner: Option<AppRunner>`
  - `open_runners: Vec<(String, AppRunner)>`
  - `browser: Option<BrowserWidget>`
  - `radio_manager: RadioManager`
- Content display/interaction routes through this layer

**Files:** New `oasis-app/src/content_layer.rs`, refactor `app_state.rs`

### 3.5 Remove Blanket Re-exports

- Replace `pub use oasis_terminal::*` in `oasis-core/src/lib.rs` with explicit imports
- Identify actual used items and import only those
- Prevents namespace pollution and circular reasoning

**Files:** `oasis-core/src/lib.rs`, downstream users

---

## Phase 4: Performance Optimizations (Partial)

**Done:** Text measurement cache (4.1)
**Remaining:**

### 4.2 Texture Deduplication Cache

- Add content hash (FxHash of rgba_data) to texture cache
- Return existing TextureId if same image already loaded
- Add LRU eviction with configurable size limit (default 64MB)
- Apply to SDL and WASM backends

**Files:** `oasis-backend-sdl/src/lib.rs`, `oasis-backend-wasm/src/renderer.rs`

### 4.3 SDI Z-Order Optimization

- Replace `Vec<String>` z_sorted_names with `Vec<usize>` indices
- Partition into base and overlay vectors during sort (single iteration per render)
- Pre-filter invisible/zero-alpha objects during sort

**Files:** `oasis-sdi/src/registry.rs`

### 4.4 FxHashMap in CSS

- Replace std `HashMap<String, Vec<IndexedRule>>` with `rustc_hash::FxHashMap`
- Cache lowercased tag names (avoid `.to_ascii_lowercase()` per element)
- Pre-sort and deduplicate at index build time instead of per-query

**Files:** `oasis-browser/src/css/`, add `rustc-hash` dependency

### 4.5 Reduce String Cloning in AppRunner

- Use `Cow<str>` or references where possible in SDI text updates
- Cache formatted directory listings (invalidate on directory change)
- Pre-allocate Vecs with `with_capacity()` hints throughout layout engine

**Files:** `oasis-core/src/apps/runner.rs`, `oasis-browser/src/layout/block.rs`

### 4.6 Glyph Rendering Batching (SDL)

- Accumulate horizontal pixel runs into single `fill_rect()` calls
- Pre-cache glyph bitmaps as small textures for repeated characters
- Batch color state changes (set once per draw_text call)

**Files:** `oasis-backend-sdl/src/lib.rs` (draw_text_styled)

---

## Phase 5: Skin System Enhancements (Partial)

**Done:** Per-app theme overrides (5.1), state-based widget styling (5.2), skin inheritance (5.4)
**Remaining:**

### 5.3 Animation Timing in Theme

- Add animation library to theme:
  ```toml
  [animations]
  button_press = { duration_ms = 100, easing = "ease_out_quad" }
  page_transition = { duration_ms = 200, easing = "ease_in_out_cubic" }
  ```
- Widgets look up named animations from theme
- Falls back to current hardcoded values

**Files:** `oasis-skin/src/theme.rs`, `oasis-ui/src/animation.rs`, widget files

### 5.5 Named Gradient Presets

- Add `[gradients]` section to theme:
  ```toml
  [gradients]
  primary = { from = "#0066FF", to = "#0044AA" }
  accent = { from = "#FF6B00", to = "#FF8C00" }
  ```
- Components reference gradients by name

**Files:** `oasis-skin/src/theme.rs`, `oasis-sdi/src/object.rs`

---

## Phase 9: Browser Engine Improvements (Partial)

**Done:** Flexbox layout (9.1), CSS positioning (9.2)
**Remaining:**

### 9.3 HTML Form Elements

- `<input type="text">` with text input
- `<input type="checkbox">` and `<input type="radio">`
- `<select>` dropdown
- `<textarea>` multiline input
- `<button>` and `<input type="submit">`
- Form submission (GET/POST)
- 30+ tests

**Files:** New `oasis-browser/src/forms.rs`, `layout/` updates

### 9.4 In-Page Text Search

- Ctrl+F opens search bar
- Highlight all matches in page
- Navigate between matches (next/previous)
- Case-insensitive by default
- Match count display
- 10+ tests

**Files:** `oasis-browser/src/lib.rs`, `paint.rs`

### 9.5 Additional CSS Selectors

- Attribute selectors: `[href]`, `[data-x="y"]`
- Adjacent sibling: `div + p`
- General sibling: `div ~ p`
- `:not()`, `:nth-child()`, `:nth-of-type()`
- 40+ tests

**Files:** `oasis-browser/src/css/`, `selector.rs`

---

## Phase 11: Accessibility & Internationalization (Partial)

**Done:** ARIA label infrastructure (11.5 partial), focus navigation (11.2 partial)
**Remaining:**

### 11.1 Expanded Accessibility Themes

- Add protanopia-safe theme (red-blind)
- Add tritanopia-safe theme (blue-blind)
- Improve high-contrast theme with stronger borders
- Ensure all themes meet WCAG AA contrast ratios
- 10+ contrast validation tests

**Files:** `oasis-ui/src/theme.rs`

### 11.3 Font Size Scaling

- System-wide font scale setting (0.5x to 3.0x)
- All text rendering respects scale factor
- UI layout adjusts to larger text
- Accessible from Settings app
- 10+ tests

**Files:** `oasis-ui/src/theme.rs`, backend text rendering, Settings app

### 11.4 Internationalization Framework

- String resource system: `strings/<lang>.toml` files
- `tr!("key")` macro for string lookup
- Language switching at runtime
- Date/number formatting per locale
- Initial languages: English, Japanese (for PSP heritage)
- RTL support infrastructure (text direction flag)
- 20+ tests

**Files:** New `oasis-i18n/` crate, integration across UI crates

---

## Phase 12: Testing & Documentation (Partial)

**Done:** Screenshot test harness (12.4), many tests added (12.2, 12.3 partial)
**Remaining:**

### 12.1 Reduce Unwrap Usage in Hot Paths

- Audit and replace unwraps with proper error propagation in top-10 files
- Add `OasisError` variants where missing
- Keep unwrap only for truly infallible cases (document with comments)

**Files:** `oasis-wm/`, `oasis-terminal/`, `oasis-browser/`

### 12.5 API Documentation Pass

- Add `///` doc comments to all public types and functions
- Focus on: oasis-types (foundation), oasis-ui (widget API),
  oasis-skin (theme API), oasis-sdi (scene graph API)
- Add doc examples where helpful
- `#![deny(missing_docs)]` on fully documented crates

**Files:** lib.rs and public API files across documented crates

---

## Summary of Remaining Work

| Phase | Focus | Status |
|-------|-------|--------|
| **3** | State Decomposition | Not started |
| **4** | Performance | 1/6 done (text cache) |
| **5** | Skin Enhancements | 3/5 done |
| **9** | Browser Engine | 2/5 done (flexbox, positioning) |
| **11** | Accessibility & i18n | Partial infrastructure only |
| **12** | Testing & Documentation | Partial (screenshot harness, many tests added) |

## Key Principles

1. **Every change must pass CI**: `cargo fmt`, `clippy -D warnings`, `cargo test`, `cargo deny`
2. **No new unwraps**: All new code uses proper error propagation
3. **Tests for everything**: Minimum 10 tests per new module
4. **Backward compatible**: Existing skins and commands continue working
5. **Incremental delivery**: Each sub-phase can be merged independently
6. **PSP-aware**: Changes must not break PSP backend (no_std-compatible where relevant)
