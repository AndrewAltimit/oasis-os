# OASIS OS Comprehensive Improvements Plan v2

> **Branch:** `feat/comprehensive-improvements`
> **Date:** 2026-03-02
> **Status:** DRAFT - Pending review
> **Scope:** 13 phases, ~20K+ LOC, 500+ new tests

---

## Overview

Deep analysis of the entire codebase (100K+ LOC, 21 workspace crates) across 6 dimensions:
code duplication, skinning/widgets, modularity/architecture, performance, new features,
and testing/quality. Covers architecture, DRY, performance, and new features.

---

## Phase 1: DRY & Code Quality Foundation

**Goal:** Eliminate duplicated patterns across backends, widgets, and commands.
Establish shared helpers that all subsequent phases build on.

### 1.1 Backend Error Helper Trait

**Problem:** `OasisError::Backend(e.to_string())` repeated 20-30 times across SDL, WASM,
UE5 backends. Each backend has its own ad-hoc error wrapping.

- Create `BackendErrExt<T>` trait in `oasis-types/src/backend.rs`
- Provides `.backend_err("context")` on any `Result<T, E: Display>`
- Provides `.texture_not_found(id)` helper for the 5+ texture lookup sites
- Migrate all 3 desktop backends to use the new trait

**Files:** `oasis-types/src/backend.rs`, `oasis-backend-sdl/src/lib.rs`,
`oasis-backend-wasm/src/renderer.rs`, `oasis-backend-ue5/src/renderer.rs`

### 1.2 Texture Validation Helper

**Problem:** Identical `width * height * 4` RGBA validation in SDL, UE5, and PSP backends
(~15 duplicated lines).

- Create `validate_rgba_data(width, height, data) -> Result<()>` in `oasis-types/src/backend.rs`
- Replace 3 backend implementations with single call
- Add overflow-safe multiplication check

**Files:** `oasis-types/src/backend.rs`, 3 backend `load_texture` impls

### 1.3 Widget Test Macro

**Problem:** 24 widget files repeat identical "draw all themes no panic" test scaffolding
(~200+ duplicated lines across checkbox, radio, button, dropdown, etc.)

- Create `test_draw_all_themes!($widget_expr)` macro in `oasis-ui/src/test_utils.rs`
- Iterates Theme::dark/light/classic/high_contrast with MockBackend
- Replace all 24 widget test files to use the macro

**Files:** `oasis-ui/src/test_utils.rs`, all 24 widget test modules

### 1.4 Terminal Command Argument Helpers

**Problem:** 90+ commands repeat argument count validation, subcommand dispatch,
and path resolution logic (~50-70 duplicated lines).

- Create `CommandArgs` helper struct in `oasis-terminal/src/lib.rs`:
  - `require_min(args, n, cmd_name) -> Result<()>`
  - `require_subcommand(args, valid_list, cmd_name) -> Result<&str>`
  - `resolve_path_arg(args, index, cwd) -> String`
- Migrate commands to use helpers (can be incremental)

**Files:** `oasis-terminal/src/lib.rs`, command modules (incremental)

### 1.5 Theme Color State Helpers

**Problem:** Checkbox, radio, button, toggle, dropdown all repeat disabled/selected/hover
color lookup patterns (~100+ duplicated lines).

- Add helper methods to `Theme` in `oasis-ui/src/theme.rs`:
  - `interactive_border(disabled, selected) -> Color`
  - `interactive_bg(disabled, selected, hovered) -> Color`
  - `state_text(disabled) -> Color`
- Migrate widgets to use helpers

**Files:** `oasis-ui/src/theme.rs`, widget files

### 1.6 App Layout Calculator

**Problem:** `runner.rs` repeats title_h / line_h / usable_h / max_visible calculations
3+ times with slight variations.

- Extract `AppLayoutCalc` struct with `compute(theme, screen_h) -> Self`
- Provides `title_h`, `line_h`, `usable_h`, `max_visible` fields
- Replace 3+ calculation sites in runner.rs

**Files:** New `oasis-core/src/apps/layout_calc.rs`, `runner.rs`

---

## Phase 2: Architecture — App Trait System

**Goal:** Replace hardcoded app handlers in the monolithic `AppRunner` (2,648 LOC)
with a trait-based extensible app system.

### 2.1 Define App Trait

- Create `App` trait in `oasis-core/src/apps/mod.rs`:
  ```
  trait App {
      fn name(&self) -> &str;
      fn icon(&self) -> Option<&str>;
      fn init(&mut self, ctx: &mut AppContext) -> Result<()>;
      fn update(&mut self, ctx: &mut AppContext) -> Result<()>;
      fn render(&self, ctx: &mut RenderContext) -> Result<()>;
      fn handle_input(&mut self, event: &InputEvent, ctx: &mut AppContext) -> AppAction;
      fn shutdown(&mut self) -> Result<()>;
  }
  ```
- Define `AppContext` with VFS, theme, audio, commands (replaces passing all of AppRunner)
- Define `AppAction` enum: None, Exit, OpenFile, SwitchMode, etc.

**Files:** `oasis-core/src/apps/mod.rs`, `oasis-core/src/apps/context.rs`

### 2.2 Extract File Manager App

- Move dual-panel file browsing logic from runner.rs into `FileManagerApp` implementing `App`
- File panel state (browse_dir, cursor, lines) moves into the app struct
- Directory listing, navigation, file operations become app methods
- Runner delegates to `FileManagerApp` for file manager mode

**Files:** New `oasis-core/src/apps/file_manager.rs`, refactor `runner.rs`

### 2.3 Extract Settings App

- Move settings display logic into `SettingsApp` implementing `App`
- System info queries become app methods
- Settings categories become app state

**Files:** New `oasis-core/src/apps/settings_app.rs`, refactor `runner.rs`

### 2.4 Extract Remaining Apps

- Music Player -> `MusicPlayerApp`
- Photo Viewer -> `PhotoViewerApp`
- System Monitor -> `SystemMonitorApp`
- Network -> `NetworkApp`
- Package Manager -> `PackageManagerApp`
- Each implements `App` trait, extracts from runner.rs

**Files:** 5 new app files, `runner.rs` shrinks by ~1500+ LOC

### 2.5 App Registry

- Create `AppRegistry` that holds `Vec<Box<dyn App>>`
- Apps register at startup; plugins can register additional apps
- `AppRunner` becomes thin dispatcher routing to registered apps
- Target: runner.rs < 500 LOC (down from 2,648)

**Files:** `oasis-core/src/apps/registry.rs`, `runner.rs`

---

## Phase 3: Architecture — State Decomposition

**Goal:** Break up the `AppState` god object (30+ fields) into layered,
testable service groups.

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

## Phase 4: Performance Optimizations

**Goal:** Measurable rendering and allocation improvements across hot paths.

### 4.1 Text Measurement Cache

**Problem:** Browser layout engine calls `bitmap_measure_text()` per text node on
every layout pass with no caching. Inline layout measures repeatedly for line breaking.

- Implement LRU cache: `(text_hash, font_size) -> width` in `oasis-browser`
- Cache at layout phase, invalidate on DOM mutation
- Expected: 30-50% speedup on text-heavy pages

**Files:** `oasis-browser/src/lib.rs`, new `text_cache.rs`

### 4.2 Texture Deduplication Cache

**Problem:** SDL backend creates new GPU texture for every `load_texture()` call,
even for identical images. No eviction, unbounded HashMap growth.

- Add content hash (FxHash of rgba_data) to texture cache
- Return existing TextureId if same image already loaded
- Add LRU eviction with configurable size limit (default 64MB)
- Apply to SDL and WASM backends

**Files:** `oasis-backend-sdl/src/lib.rs`, `oasis-backend-wasm/src/renderer.rs`

### 4.3 SDI Z-Order Optimization

**Problem:** `ensure_z_sorted()` clones all HashMap keys into Vec<String> on every
z-dirty event. Two-pass rendering iterates the list twice with redundant visibility checks.

- Replace `Vec<String>` z_sorted_names with `Vec<usize>` indices
- Partition into base and overlay vectors during sort (single iteration per render)
- Pre-filter invisible/zero-alpha objects during sort

**Files:** `oasis-sdi/src/registry.rs`

### 4.4 FxHashMap in CSS Cascade

**Problem:** CSS `SelectorIndex` uses std `HashMap<String, Vec<IndexedRule>>` which is
slow for short string keys. `candidates()` allocates new Vec per element match.

- Replace with `rustc_hash::FxHashMap` for tag/id/class indexes
- Cache lowercased tag names (avoid `.to_ascii_lowercase()` per element)
- Pre-sort and deduplicate at index build time instead of per-query

**Files:** `oasis-browser/src/css/cascade.rs`, add `rustc-hash` dependency

### 4.5 Reduce String Cloning in AppRunner

**Problem:** `update_sdi()` clones Vec<String> entries every frame for each visible line.
Path manipulation clones repeatedly in file navigation.

- Use `Cow<str>` or references where possible in SDI text updates
- Cache formatted directory listings (invalidate on directory change)
- Pre-allocate Vecs with `with_capacity()` hints throughout layout engine

**Files:** `oasis-core/src/apps/runner.rs`, `oasis-browser/src/layout/block.rs`

### 4.6 Glyph Rendering Batching (SDL)

**Problem:** `draw_text_styled()` in SDL backend calls `canvas.draw_point()` per pixel
for bold text. Each glyph is 8x8 with per-bit testing.

- Accumulate horizontal pixel runs into single `fill_rect()` calls
- Pre-cache glyph bitmaps as small textures for repeated characters
- Batch color state changes (set once per draw_text call)

**Files:** `oasis-backend-sdl/src/lib.rs` (draw_text_styled)

---

## Phase 5: Skin System Enhancements

**Goal:** Make the skin system more powerful, flexible, and composable.

### 5.1 Per-App Theme Overrides

**Problem:** TV Guide has 20+ hardcoded COLOR_* constants that ignore the skin system
entirely. Other apps may have similar issues.

- Add `[app_themes.<app_name>]` section to theme.toml:
  ```toml
  [app_themes.tv_guide]
  bg = "#0A1628"
  grid_line = "#1A3A5C"
  header_bg = "#0C1932"
  ```
- Apps query theme for app-specific colors, falling back to global theme
- Migrate TV Guide hardcoded colors to theme system
- Provide defaults that match current appearance

**Files:** `oasis-skin/src/theme.rs`, `oasis-core/src/apps/tv_guide/guide.rs`

### 5.2 State-Based Widget Styling in Theme

**Problem:** Widget state colors (hover, pressed, disabled) are computed internally
with hardcoded lighten/darken. Skins can't customize per-state appearance.

- Add state palette sections to theme:
  ```toml
  [widget_states.button]
  normal_bg = "#505050"
  hover_bg = "#656565"
  pressed_bg = "#353535"
  disabled_bg = "#3A3A3A"
  disabled_text = "#555555"
  ```
- Widgets query state-specific colors from theme
- Falls back to computed values if not specified

**Files:** `oasis-skin/src/theme.rs`, `oasis-ui/src/button.rs` and other widgets

### 5.3 Animation Timing in Theme

**Problem:** Animation durations/easings are hardcoded in widget code. Skins can't
control transition speed or easing curves.

- Add animation library to theme:
  ```toml
  [animations]
  button_press = { duration_ms = 100, easing = "ease_out_quad" }
  page_transition = { duration_ms = 200, easing = "ease_in_out_cubic" }
  cursor_move = { duration_ms = 150, easing = "ease_out_elastic" }
  toast_slide = { duration_ms = 300, easing = "ease_out_cubic" }
  ```
- Widgets look up named animations from theme
- Falls back to current hardcoded values

**Files:** `oasis-skin/src/theme.rs`, `oasis-ui/src/animation.rs`, widget files

### 5.4 Skin Inheritance

**Problem:** Creating a new skin requires defining everything from scratch. Can't create
a variant (e.g., "dark_modern") by just overriding a few colors from "modern".

- Add `inherits = "parent_skin_name"` to skin.toml manifest
- Child skin only specifies overrides; rest inherited from parent
- Recursive inheritance (max depth 3)
- Enables rapid skin variant creation

**Files:** `oasis-skin/src/loader.rs`, `oasis-skin/src/builtin.rs`

### 5.5 Named Gradient Presets

**Problem:** Gradients are defined inline wherever used. No way to reuse gradient
definitions across components.

- Add `[gradients]` section to theme:
  ```toml
  [gradients]
  primary = { from = "#0066FF", to = "#0044AA" }
  accent = { from = "#FF6B00", to = "#FF8C00" }
  ```
- Components reference gradients by name
- Reduces repetition in complex skins like xp

**Files:** `oasis-skin/src/theme.rs`, `oasis-sdi/src/object.rs`

---

## Phase 6: Widget System Expansion

**Goal:** Add missing essential widgets for richer application UIs.

### 6.1 Slider / Range Input Widget

**Priority: HIGH** — Needed for volume, brightness, seek bars, zoom controls.

- Horizontal and vertical orientation
- Thumb dragging with keyboard support (left/right arrows)
- Optional value label
- Theme-consistent track/thumb colors
- Step snapping (integer vs continuous)
- 15+ tests

**Files:** New `oasis-ui/src/slider.rs`

### 6.2 Context Menu Widget

**Priority: HIGH** — Enables right-click menus in file manager, browser, desktop.

- Popup menu at cursor position
- Nested submenus (1 level)
- Keyboard navigation (up/down/enter/escape)
- Separator items
- Disabled/grayed items
- Auto-dismiss on click outside
- 10+ tests

**Files:** New `oasis-ui/src/context_menu.rs`

### 6.3 Tree View Widget

**Priority: MEDIUM** — Needed for file trees, settings hierarchy, DOM inspector.

- Expandable/collapsible nodes
- Indentation with connecting lines
- Single/multi selection
- Lazy loading (expand callback)
- Keyboard navigation (arrows, enter to toggle)
- 15+ tests

**Files:** New `oasis-ui/src/tree_view.rs`

### 6.4 Split Pane Widget

**Priority: MEDIUM** — Enables resizable panels (file manager dual-pane, editor+preview).

- Horizontal and vertical split
- Draggable divider
- Min/max pane size constraints
- Collapse/expand individual panes
- 10+ tests

**Files:** New `oasis-ui/src/split_pane.rs`

### 6.5 Toast / Notification System

**Priority: MEDIUM** — System-wide transient notifications.

- Slide-in from corner with configurable position
- Auto-dismiss with configurable timeout
- Severity levels (info, success, warning, error)
- Queue multiple notifications
- Theme-consistent styling
- 10+ tests

**Files:** New `oasis-ui/src/toast.rs`

### 6.6 Accordion / Collapsible Sections

**Priority: LOW** — Useful for settings panels, help content.

- Multiple sections with headers
- Expand/collapse with animation
- Single-expand or multi-expand mode
- Keyboard navigation
- 10+ tests

**Files:** New `oasis-ui/src/accordion.rs`

---

## Phase 7: Window Manager Enhancements

**Goal:** Professional window management with modern features.

### 7.1 Window Snap Zones

- Drag to screen edges triggers snap preview
- Left/right half-screen snap (like Windows Aero Snap)
- Top edge = maximize, corners = quarter-screen
- Keyboard shortcuts: Meta+Arrow for snap
- 10+ tests

**Files:** `oasis-wm/src/lib.rs`, `oasis-wm/src/snap.rs`

### 7.2 Window Animations

- Minimize: shrink to taskbar position
- Maximize: expand from current size
- Open: fade-in or scale-up
- Close: fade-out or scale-down
- Respects theme `reduced_motion` flag
- Configurable duration in theme

**Files:** `oasis-wm/src/animation.rs`, `oasis-wm/src/lib.rs`

### 7.3 Virtual Desktops

- N virtual desktops (default 4)
- Switch via keyboard shortcut or UI indicator
- Windows belong to a desktop (or "all desktops")
- Move window to desktop via context menu
- Desktop indicator in status bar
- 15+ tests

**Files:** New `oasis-wm/src/desktops.rs`, `oasis-wm/src/lib.rs`

### 7.4 Tiling Mode

- Toggle between floating and tiling layouts
- Tiling algorithms: master+stack, grid, columns
- Keyboard shortcuts for layout manipulation
- Respect minimum window sizes
- Can mix tiled and floating windows
- 15+ tests

**Files:** New `oasis-wm/src/tiling.rs`, `oasis-wm/src/lib.rs`

### 7.5 Always-on-Top and Modal Flags

- `always_on_top` flag on WindowConfig
- `modal` flag blocks input to windows below
- Dim backdrop behind modal windows
- Z-order respects these flags during sort
- 10+ tests

**Files:** `oasis-wm/src/lib.rs`, `oasis-wm/src/window.rs`

---

## Phase 8: Terminal & Shell Improvements

**Goal:** Make the shell more capable for scripting and interactive use.

### 8.1 Line Editing (Readline-style)

- Ctrl+A (beginning), Ctrl+E (end), Ctrl+U (clear line), Ctrl+K (kill to end)
- Ctrl+W (delete word), Alt+B/F (word movement)
- Ctrl+R (history search)
- Up/Down arrows for history navigation (already exists, verify completeness)
- 15+ tests

**Files:** `oasis-terminal/src/interpreter.rs` or new `oasis-terminal/src/line_edit.rs`

### 8.2 Syntax Highlighting

- Colorize command names (green for valid, red for unknown)
- String literals in quotes highlighted
- Variables ($VAR) highlighted
- Pipes/redirects highlighted
- Comments (#) dimmed
- 10+ tests

**Files:** New `oasis-terminal/src/highlight.rs`

### 8.3 Extended Scripting

- `while condition; do ... done` loops
- `case $var in pattern) ... ;; esac` matching
- `if/elif/else/fi` improvements
- Local variables in functions (`local var=value`)
- Return values from functions
- 20+ tests

**Files:** `oasis-terminal/src/interpreter.rs`

### 8.4 Tab Completion Improvements

- Complete file paths with directory traversal
- Complete command names from registry
- Complete variable names after $
- Show completion candidates on double-tab
- Cycle through completions on repeated tab
- 10+ tests

**Files:** `oasis-terminal/src/interpreter.rs` or new `oasis-terminal/src/completion.rs`

### 8.5 Job Control (Background Tasks)

- `command &` runs in background
- `jobs` lists running jobs
- `fg %N` brings job to foreground
- `bg %N` resumes in background
- Job completion notification
- 10+ tests

**Files:** New `oasis-terminal/src/jobs.rs`, `interpreter.rs`

---

## Phase 9: Browser Engine Improvements

**Goal:** Better CSS support and essential web features.

### 9.1 CSS Flexbox Layout

- Implement CSS Flexible Box Level 1:
  - `flex-direction`, `flex-wrap`, `justify-content`, `align-items`
  - `flex-grow`, `flex-shrink`, `flex-basis`
  - `order`, `align-self`
- Wire into layout engine alongside block/inline/table
- 50+ tests

**Files:** New `oasis-browser/src/layout/flex.rs`, `layout/mod.rs`

### 9.2 CSS Positioning

- `position: relative` (offset from normal flow)
- `position: absolute` (relative to positioned ancestor)
- `position: fixed` (relative to viewport)
- `top`, `left`, `right`, `bottom`, `z-index`
- Stacking context management
- 30+ tests

**Files:** `oasis-browser/src/layout/block.rs`, new `positioning.rs`

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

**Files:** `oasis-browser/src/css/cascade.rs`, `selector.rs`

---

## Phase 10: New Applications

**Goal:** Add essential utility applications that users expect.

### 10.1 Text Editor

- Open/save files via VFS
- Line numbers, cursor movement
- Basic editing: insert, delete, backspace, newline
- Selection with shift+arrows
- Copy/paste (internal clipboard)
- Syntax highlighting for common formats (TOML, Rust, HTML)
- Find/replace
- Undo/redo (simple history stack)

**Files:** New `oasis-core/src/apps/text_editor.rs`

### 10.2 Calculator

- Basic operations: +, -, *, /
- Expression parsing with operator precedence
- Parentheses support
- Memory functions (M+, M-, MR, MC)
- History of calculations
- Keyboard input for digits and operators

**Files:** New `oasis-core/src/apps/calculator.rs`

### 10.3 Clock / Timer / Stopwatch

- Digital clock display with current time
- Countdown timer with alarm
- Stopwatch with lap times
- Alarm clock with configurable alarms
- Uses platform TimeService

**Files:** New `oasis-core/src/apps/clock.rs`

### 10.4 Paint / Drawing App

- Canvas with pixel-level drawing
- Tools: pencil, line, rectangle, circle, fill, eraser
- Color palette (uses theme accent colors + custom)
- Brush size selection
- Undo/redo
- Save as PNG via VFS
- Layer support (basic: 2-3 layers)

**Files:** New `oasis-core/src/apps/paint.rs`

### 10.5 Games Collection

- Snake (classic grid-based)
- Memory/Matching card game
- Simple puzzle game (sliding tiles)
- Score tracking
- D-pad / keyboard controls

**Files:** New `oasis-core/src/apps/games/` directory

---

## Phase 11: Accessibility & Internationalization

**Goal:** Make OASIS usable by more people in more languages.

### 11.1 Expanded Accessibility Themes

- Add protanopia-safe theme (red-blind)
- Add tritanopia-safe theme (blue-blind)
- Improve high-contrast theme with stronger borders
- Ensure all themes meet WCAG AA contrast ratios
- 10+ contrast validation tests

**Files:** `oasis-ui/src/theme.rs`

### 11.2 Focus Navigation System

- Tab/Shift-Tab cycles between interactive widgets
- Visual focus ring on focused widget
- Arrow keys for in-widget navigation
- Escape to defocus / close current context
- Focus trapping in modals
- 15+ tests

**Files:** `oasis-ui/src/focus.rs` (expand existing), widget files

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

### 11.5 Screen Reader Hints

- Add semantic labels to SDI objects (`aria_label` field)
- Announce focus changes, button presses, navigation
- Text output channel for screen reader integration
- Structural hints (heading level, list item position)

**Files:** `oasis-sdi/src/object.rs`, new `oasis-a11y/` module or crate

---

## Phase 12: Testing & Documentation

**Goal:** Close test coverage gaps and improve developer experience.

### 12.1 Reduce Unwrap Usage in Hot Paths

**Problem:** 2058 unwrap/expect calls across 97 files. Worst offenders:
- `oasis-wm/manager.rs`: 291 unwraps
- `oasis-terminal/interpreter.rs`: 132 unwraps
- `oasis-browser/lib.rs`: 99 unwraps

- Audit and replace unwraps with proper error propagation in top-10 files
- Add `OasisError` variants where missing
- Keep unwrap only for truly infallible cases (document with comments)
- Target: <500 unwraps in workspace (from 2058)

**Files:** `oasis-wm/`, `oasis-terminal/`, `oasis-browser/`

### 12.2 Add Tests to Untested Crates

**Problem:** Only 6 of 21 crates have explicit test modules. Critical gaps:
oasis-terminal, oasis-core, oasis-sdi, oasis-wm.

- Add #[cfg(test)] modules to 10+ crates
- Priority: oasis-sdi (scene graph core), oasis-wm (window management),
  oasis-terminal (command dispatch), oasis-core (app coordination)
- Target: 200+ new tests across these crates

**Files:** Test modules in each crate's source files

### 12.3 Integration Test Suite

- Browser: load HTML -> layout -> paint -> verify pixel output
- Terminal: parse command -> execute -> verify output
- VFS: file operations across MemoryVfs/RealVfs
- App lifecycle: init -> input -> update -> render -> shutdown
- 30+ integration tests

**Files:** New integration test files in relevant crates

### 12.4 Visual Regression Tests in CI

- `cargo run -p oasis-app --bin screenshot-tests -- --check`
- Golden baselines in `screenshots/golden/`
- HTML diff report on failure
- `--bless` workflow for updating baselines
- Run for all 13 skins

**Files:** CI config, `oasis-app/src/screenshot_tests.rs`

### 12.5 API Documentation Pass

- Add `///` doc comments to all public types and functions
- Focus on: oasis-types (foundation), oasis-ui (widget API),
  oasis-skin (theme API), oasis-sdi (scene graph API)
- Add doc examples where helpful
- `#![deny(missing_docs)]` on fully documented crates

**Files:** lib.rs and public API files across documented crates

---

## Phase 13: New Skins

**Goal:** Demonstrate skin system flexibility with diverse new skins.

### 13.1 macOS Skin

- Light appearance with system font sizing
- Traffic light window buttons (red/yellow/green circles)
- Dock-style app bar at bottom
- Translucent menu bar at top
- SF-style rounded corners and shadows
- Frosted glass effect (simulated with alpha blending)

**Files:** New `skins/macos/` directory

### 13.2 Linux / GNOME Skin

- Dark Adwaita-inspired theme
- Top bar with activities/clock/system tray
- Rounded window corners with headerbar
- GNOME-style app grid
- Muted accent colors (blue/teal)

**Files:** New `skins/gnome/` directory

### 13.3 Retro CGA/EGA Skin

- 4-color CGA palette (cyan/magenta/white/black)
- 8x8 pixel aesthetic
- No gradients or rounded corners
- Blocky window borders
- Nostalgic DOS-era feel

**Files:** New `skins/retro-cga/` directory

### 13.4 Cyberpunk / Neon Skin

- Dark background with neon accent colors (cyan, magenta, yellow)
- Glow effects on focused elements
- Scanline overlay
- Angular/geometric window borders
- Animated pulse on active elements

**Files:** New `skins/cyberpunk/` directory

### 13.5 Paper / Minimal Skin

- White/cream background with serif-inspired spacing
- Minimal borders (just subtle lines)
- No shadows, no gradients
- Black text on light background
- Focuses on content readability
- Ideal for e-reader / document viewing

**Files:** New `skins/paper/` directory

---

## Phase Summary

| Phase | Focus | Est. LOC | Est. Tests | Priority |
|-------|-------|----------|------------|----------|
| **1** | DRY & Code Quality Foundation | ~400 | 30+ | **CRITICAL** |
| **2** | Architecture: App Trait System | ~1,500 | 50+ | **HIGH** |
| **3** | Architecture: State Decomposition | ~800 | 30+ | **HIGH** |
| **4** | Performance Optimizations | ~600 | 20+ | **HIGH** |
| **5** | Skin System Enhancements | ~800 | 30+ | **HIGH** |
| **6** | Widget System Expansion | ~1,500 | 70+ | **MEDIUM** |
| **7** | Window Manager Enhancements | ~1,200 | 60+ | **MEDIUM** |
| **8** | Terminal & Shell Improvements | ~1,500 | 65+ | **MEDIUM** |
| **9** | Browser Engine Improvements | ~4,000 | 160+ | **MEDIUM** |
| **10** | New Applications | ~3,000 | 50+ | **MEDIUM** |
| **11** | Accessibility & i18n | ~2,000 | 75+ | **LOW** |
| **12** | Testing & Documentation | ~2,000 | 230+ | **LOW** |
| **13** | New Skins | ~1,500 | 20+ | **LOW** |
| | **TOTALS** | **~20,800** | **890+** | |

---

## Execution Order

```
Phase 1 (DRY foundation) ──────────────────────────────┐
    │                                                    │
Phase 2 (App trait system) ─── Phase 4 (Performance) ──┤
    │                              │                    │
Phase 3 (State decomposition) ─────────────────────────┤
    │                                                    │
Phase 5 (Skin enhancements) ─── Phase 6 (Widgets) ────┤
    │                              │                    │
Phase 7 (Window manager) ──── Phase 8 (Terminal) ─────┤
    │                              │                    │
Phase 9 (Browser) ─────────── Phase 10 (New apps) ────┤
    │                              │                    │
Phase 11 (Accessibility) ──── Phase 12 (Testing) ─────┤
    │                                                    │
Phase 13 (New skins) ──────────────────────────────────┘
```

Phases on the same row can be worked in parallel.
Phase 1 is prerequisite for all others (establishes shared helpers).
Phases 2-3 should precede 10 (new apps benefit from App trait system).
Phase 5 should precede 13 (skin inheritance enables rapid skin creation).

---

## Key Principles

1. **Every change must pass CI**: `cargo fmt`, `clippy -D warnings`, `cargo test`, `cargo deny`
2. **No new unwraps**: All new code uses proper error propagation
3. **Tests for everything**: Minimum 10 tests per new module
4. **Backward compatible**: Existing skins and commands continue working
5. **Incremental delivery**: Each sub-phase can be merged independently
6. **PSP-aware**: Changes must not break PSP backend (no_std-compatible where relevant)
