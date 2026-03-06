# App Extraction Plan: Breaking Apps Out of oasis-core

## Motivation

`oasis-core` is 30,726 LOC — the largest crate in the workspace. 16 apps live
inside `src/apps/`, totaling ~13,845 lines. Several apps are substantial
standalone modules (Games 2,155, Paint 1,875, Clock 1,775, Text Editor 1,710,
File Manager 1,435, Calculator 1,285, TV Guide 3,401). Extracting the largest
apps into their own crates will:

- Reduce oasis-core to a thin orchestration layer (~17k LOC)
- Enable independent compilation and faster incremental builds
- Clarify dependency boundaries per app
- Make it easier for contributors to work on a single app in isolation

---

## Current Architecture

### App Trait System

All extracted apps implement the `App` trait (`apps/app_trait.rs`, 313 lines):

```
App::title(), path(), lines(), browse_dir(), viewing_file(),
    handle_input(), draw_windowed(), handle_click(),
    peek_pending_request(), take_pending_request()
```

`AppRunner` (`runner.rs`, 1,656 lines) dispatches to apps via `launch()`, which
pattern-matches on app title and creates the appropriate `Box<dyn App>`.

### Current File Layout

```
crates/oasis-core/src/apps/
├── mod.rs              (26 lines)   — module declarations, re-exports
├── app_trait.rs        (313 lines)  — App trait + ContentState
├── runner.rs           (1,656 lines)— AppRunner: dispatch, input, inline Radio/TV
├── runner_sdi.rs       (233 lines)  — SDI scene-graph rendering for apps
├── layout_calc.rs      (121 lines)  — shared layout computation
├── file_viewer.rs      (251 lines)  — shared file/dir viewing helpers
├── simple_app.rs       (343 lines)  — 5 static-content apps (Settings, Network, etc.)
├── browsing_app.rs     (667 lines)  — Music Player + Photo Viewer
├── file_manager.rs     (1,435 lines)— File Manager
├── text_editor.rs      (1,710 lines)— Text Editor (modal vim-like)
├── calculator.rs       (1,285 lines)— Calculator (expression parser)
├── clock.rs            (1,775 lines)— Clock/Stopwatch/Timer/Alarms
├── paint.rs            (1,875 lines)— Paint (multi-layer canvas, 9 tools)
├── games.rs            (2,155 lines)— Games (Snake, Memory Match, Sliding Puzzle)
└── tv_guide/           (3,401 lines)— TV Guide (EPG grid, catalog, schedule)
    ├── mod.rs           (27 lines)
    ├── guide.rs         (1,876 lines)
    ├── catalog.rs       (594 lines)
    ├── schedule.rs      (553 lines)
    ├── channel.rs       (197 lines)
    └── test_data.rs     (154 lines)
```

### Coupled Terminal Commands

Two command files in `oasis-core/src/terminal/` are tightly coupled to apps:

| File | Lines | Coupled App | Coupling Type |
|------|-------|-------------|---------------|
| `browser_commands.rs` | 680 | Browser | Direct `oasis_browser::loader` import |
| `tv_commands.rs` | 285 | TV Guide | Imports `crate::apps::tv_guide` types |

### Shared Dependencies (all apps use these)

- `oasis_types::backend` (SdiBackend, Color, TextureId)
- `oasis_types::input` (Button)
- `oasis_sdi` (SdiRegistry)
- `oasis_skin::active_theme` (ActiveTheme)
- `oasis_vfs` (Vfs, EntryKind)

---

## Extraction Plan

### Phase 0: Prepare Infrastructure (prerequisite)

**Goal:** Move the `App` trait and shared helpers into a new `oasis-app-core`
crate so extracted apps can depend on it without depending on all of oasis-core.

**New crate: `oasis-app-core`**

Move from oasis-core:
- `apps/app_trait.rs` → `oasis-app-core/src/lib.rs` (App trait, ContentState,
  AppAction)
- `apps/layout_calc.rs` → `oasis-app-core/src/layout.rs`
- `apps/file_viewer.rs` → `oasis-app-core/src/file_viewer.rs`

Dependencies:
```toml
[dependencies]
oasis-types = { workspace = true }
oasis-sdi = { workspace = true }
oasis-skin = { workspace = true }
oasis-vfs = { workspace = true }
oasis-ui = { workspace = true }
```

oasis-core then depends on `oasis-app-core` and re-exports `App`, `AppAction`.

**Estimated effort:** Medium. Touches every app's imports but is mechanical.

**Files changed:**
- NEW: `crates/oasis-app-core/Cargo.toml`, `crates/oasis-app-core/src/lib.rs`,
  `layout.rs`, `file_viewer.rs`
- EDIT: `Cargo.toml` (workspace members), `crates/oasis-core/Cargo.toml`,
  `crates/oasis-core/src/apps/mod.rs`, all app files (change `use crate::` →
  `use oasis_app_core::`)

---

### Phase 1: Extract Self-Contained Apps (no coupled commands)

These apps have zero coupling to oasis-core internals beyond the App trait.
Extract them in order of decreasing size (biggest wins first).

#### Step 1.1: `oasis-app-games` (2,155 lines)

Move `apps/games.rs` → `crates/oasis-app-games/src/lib.rs`

Dependencies: `oasis-app-core`, `oasis-types`, `oasis-sdi`, `oasis-skin`,
`oasis-vfs`

No external crate deps. Self-contained PRNG. Cleanest extraction candidate.

#### Step 1.2: `oasis-app-paint` (1,875 lines)

Move `apps/paint.rs` → `crates/oasis-app-paint/src/lib.rs`

Dependencies: same as games.

Self-contained canvas/tool/layer system. No VFS IPC.

#### Step 1.3: `oasis-app-clock` (1,775 lines)

Move `apps/clock.rs` → `crates/oasis-app-clock/src/lib.rs`

Dependencies: same + `oasis-platform` (time services, injected).

Injectable time source — no `SystemTime::now()` calls.

#### Step 1.4: `oasis-app-text-editor` (1,710 lines)

Move `apps/text_editor.rs` → `crates/oasis-app-text-editor/src/lib.rs`

Dependencies: same as games.

Modal editing system is fully self-contained.

#### Step 1.5: `oasis-app-file-manager` (1,435 lines)

Move `apps/file_manager.rs` → `crates/oasis-app-file-manager/src/lib.rs`

Dependencies: same + uses `file_viewer` helpers (now in oasis-app-core).

#### Step 1.6: `oasis-app-calculator` (1,285 lines)

Move `apps/calculator.rs` → `crates/oasis-app-calculator/src/lib.rs`

Dependencies: same as games.

Pure recursive-descent parser — zero external coupling.

---

### Phase 2: Extract Apps with Shared Code

#### Step 2.1: `oasis-app-media` (667 lines)

Move `apps/browsing_app.rs` → `crates/oasis-app-media/src/lib.rs`

Contains both Music Player and Photo Viewer (same `BrowsingApp` code with
different viewer modes). Keep them together since they share the browsing
infrastructure.

Dependencies: same + `oasis-audio` (for metadata display).

Uses `file_viewer` helpers from oasis-app-core.

---

### Phase 3: Extract TV Guide (most complex)

#### Step 3.1: `oasis-app-tv-guide` (3,401 lines)

Move the entire `apps/tv_guide/` module directory →
`crates/oasis-app-tv-guide/src/`

```
crates/oasis-app-tv-guide/src/
├── lib.rs       (from mod.rs — re-exports)
├── guide.rs     (1,876 lines — TvGuideState)
├── catalog.rs   (594 lines — ChannelCatalog)
├── schedule.rs  (553 lines — ScheduleSlot)
├── channel.rs   (197 lines — ChannelConfig)
└── test_data.rs (154 lines — test fixtures)
```

Dependencies: `oasis-app-core`, `oasis-types`, `oasis-sdi`, `oasis-skin`,
`oasis-vfs`, `toml`, `serde`

#### Step 3.2: Migrate `tv_commands.rs` coupling

`tv_commands.rs` (285 lines) imports `crate::apps::tv_guide` types. After
extraction, change to `use oasis_app_tv_guide::{ChannelConfig, ...}`.

oasis-core gains a dependency on `oasis-app-tv-guide` for the terminal commands.
This is acceptable — oasis-core already depends on `oasis-browser` for
`browser_commands.rs`.

#### Step 3.3: Extract TV Guide from `runner.rs` inline handling

TV Guide is currently a "special case" handled inline in `AppRunner` (not
delegated via the `App` trait). Two approaches:

**Option A (recommended):** Implement the `App` trait for `TvGuideState` inside
`oasis-app-tv-guide`. The guide already has `draw_windowed()`, `handle_input()`,
and VFS IPC — it just needs to conform to the trait interface. Then remove the
special-case code from `runner.rs`.

**Option B:** Keep the inline handling in `runner.rs` but import from the
external crate. Simpler but doesn't clean up the architecture.

---

### Phase 4: Extract Internet Radio from runner.rs

#### Step 4.1: `oasis-app-radio` (~240 lines of logic)

Internet Radio is currently inline in `runner.rs` with methods like
`radio_content()`, `handle_radio_input()`, `refresh_radio()`.

Extract to `crates/oasis-app-radio/src/lib.rs` implementing the `App` trait.

Dependencies: `oasis-app-core`, `oasis-types`, `oasis-sdi`, `oasis-skin`,
`oasis-vfs`, `oasis-audio` (StationRegistry, RADIO_REQUEST_PATH), `toml`,
`serde`

This also eliminates the second "special case" in `runner.rs`, making all 16
apps use the uniform `App` trait dispatch.

---

### Phase 5: Clean Up runner.rs

After all extractions, `runner.rs` shrinks from 1,656 lines to approximately
600-800 lines containing:

- `AppRunner` struct and `launch()` dispatch table
- `SimpleApp` handling for the 5 static apps (343 lines, stays in oasis-core)
- Generic input/rendering delegation to `Box<dyn App>`
- No more inline app logic

Update `runner.rs` dispatch to import from new crates:
```rust
use oasis_app_games::GamesApp;
use oasis_app_paint::PaintApp;
use oasis_app_clock::ClockApp;
use oasis_app_text_editor::TextEditorApp;
use oasis_app_file_manager::FileManagerApp;
use oasis_app_calculator::CalculatorApp;
use oasis_app_media::BrowsingApp;
use oasis_app_tv_guide::TvGuideState;  // now implements App
use oasis_app_radio::RadioApp;         // now implements App
```

---

## Execution Order Summary

| Phase | Crate | Lines Extracted | Complexity |
|-------|-------|----------------|------------|
| 0 | `oasis-app-core` (trait + helpers) | 685 | Medium |
| 1.1 | `oasis-app-games` | 2,155 | Low |
| 1.2 | `oasis-app-paint` | 1,875 | Low |
| 1.3 | `oasis-app-clock` | 1,775 | Low |
| 1.4 | `oasis-app-text-editor` | 1,710 | Low |
| 1.5 | `oasis-app-file-manager` | 1,435 | Low |
| 1.6 | `oasis-app-calculator` | 1,285 | Low |
| 2.1 | `oasis-app-media` | 667 | Low |
| 3 | `oasis-app-tv-guide` | 3,401 | Medium-High |
| 4 | `oasis-app-radio` | ~240 | Medium |
| 5 | Clean up runner.rs | — | Low |
| **Total** | **10 new crates** | **~15,228** | |

## Impact on oasis-core

| Metric | Before | After |
|--------|--------|-------|
| oasis-core LOC | ~30,726 | ~15,500 |
| Apps in oasis-core | 16 | 5 (SimpleApp static screens) |
| Special-case apps in runner.rs | 2 (Radio, TV Guide) | 0 |
| New workspace crates | 0 | 10 |

## Dependency Graph (after extraction)

```
oasis-app-core          (App trait, layout, file_viewer)
├── oasis-app-games
├── oasis-app-paint
├── oasis-app-clock      (+oasis-platform)
├── oasis-app-text-editor
├── oasis-app-file-manager
├── oasis-app-calculator
├── oasis-app-media      (+oasis-audio)
├── oasis-app-tv-guide   (+toml, serde)
└── oasis-app-radio      (+oasis-audio, toml, serde)

oasis-core
├── oasis-app-core       (re-export App trait)
├── oasis-app-games      (dispatch)
├── oasis-app-paint      (dispatch)
├── oasis-app-clock      (dispatch)
├── oasis-app-text-editor(dispatch)
├── oasis-app-file-manager(dispatch)
├── oasis-app-calculator (dispatch)
├── oasis-app-media      (dispatch)
├── oasis-app-tv-guide   (dispatch + tv_commands.rs)
├── oasis-app-radio      (dispatch)
└── (existing deps unchanged)
```

## Rules

1. **No behavioral changes** — every app must work identically after extraction
2. **Tests move with the app** — `#[cfg(test)] mod tests` blocks go to the new crate
3. **One phase per commit** — each phase is a single atomic commit
4. **CI green between phases** — `cargo test --workspace && cargo clippy --workspace -- -D warnings`
5. **No new public API beyond what runner.rs needs** — keep app internals `pub(crate)`

## What Stays in oasis-core

- `AppRunner` (orchestration, dispatch table)
- `runner_sdi.rs` (SDI scene-graph wiring)
- `simple_app.rs` (5 lightweight static-content apps: Settings, Network,
  Package Manager, Browser placeholder, System Monitor, Terminal)
- All terminal commands (`browser_commands.rs`, `tv_commands.rs`, etc.)
- Dashboard, agent/MCP, plugin system, scripting, status/bottom bars
- All other non-app modules
