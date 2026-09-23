# Window Manager

`oasis-wm` turns the flat SDI scene graph into overlapping, movable,
resizable windows. Skins opt in with `window_manager = true` in
`features.toml` (most skins do, including `classic`, so apps open as
windows over the dashboard; `paper`, `retro-cga`, `vaporwave`,
`highcontrast` and `corrupted` do not). The desktop (SDL), WASM and PSP
hosts all drive the same `WindowManager`; the PSP backend uses a
PSP-tuned `WmTheme` (`psp_wm_theme()`).

This guide is the practical API summary. The design rationale lives in
[design.md §4.5](design.md#45-window-manager) (4.5.1 - 4.5.9).

## Model

- **SDI stays flat.** A window with id `editor_01` owns SDI objects named
  `editor_01.frame`, `editor_01.titlebar`, `editor_01.title_text`,
  `editor_01.btn_close` / `btn_minimize` / `btn_maximize` and
  `editor_01.content`. Moving, focusing or hiding a window updates every
  object with that prefix; the WM never adds hierarchy to SDI.
- **The host draws content.** The WM owns chrome and geometry. Window
  content (an app runner, the browser widget, the terminal) is painted
  by the host inside a clip rect the WM sets up
  (`draw_with_clips*`).
- **Behavior is uniform, appearance is skin data.** Colors, titlebar
  height, radii, glyphs, button side and title alignment come from
  `WmTheme`, built from the skin with `SkinTheme::build_wm_theme()`
  (`[wm_theme]` overrides, see [skin-authoring.md](skin-authoring.md)).

## Types

All re-exported from `oasis_wm` (and from `oasis_core::wm`).

| Type | Purpose |
|------|---------|
| `WindowManager` | Owns the window list, drag/resize state, snapping, tiling and animations |
| `WindowConfig` | Creation parameters: `id`, `title`, optional `x`/`y` (cascade when `None`), content `width`/`height`, `window_type`, `always_on_top`, `modal` |
| `WindowType` | `AppWindow`, `Dialog` (modal, centered), `Panel` (docked), `FloatingWidget` (always on top, no min/max), `Fullscreen` (no chrome) |
| `WindowState` | `Normal`, `Minimized`, `Maximized` |
| `Window`, `WindowId`, `Geometry` | A managed window, its id (`Rc<str>`), and an `x, y, w, h` rect |
| `TitleLayout` | Result of `Window::title_layout`: button group placement, title position, truncated title |
| `WmTheme` | Visual parameters (titlebar colors / gradients, button glyphs and colors, `button_side`, `title_align`, radii, shadows, `maximize_top_inset` / `maximize_bottom_inset`) |
| `WmEvent` | What `handle_input` did: `WindowFocused`, `WindowMoved`, `WindowResized`, `WindowClosed`, `WindowMinimized`, `WindowMaximized`, `WindowRestored`, `ContentClick(id, x, y)` (content-local), `DesktopClick(x, y)`, `None` |
| `HitRegion`, `ButtonKind`, `ResizeEdge` | Hit-test results (`hit_test` module) |
| `SnapManager`, `SnapZone`, `SnapPreview`, `KeyboardSnapDirection` | Edge snapping (`snap` module) |
| `TilingManager`, `TilingLayout`, `TilingConfig` | Tiling layouts: `MasterStack`, `Grid`, `Columns`, `Rows`, `Monocle` (`tiling` module) |
| `AnimationManager`, `AnimationKind`, `AnimationDurations` | Window open/close/minimize/restore animations (`animation` module) |
| `DesktopManager` | Virtual desktops: switch, assign/move windows, sticky windows (`desktops` module) |

## `WindowManager` API

Construction and configuration:

```rust,ignore
let mut wm = WindowManager::with_theme(screen_w, screen_h, skin.theme.build_wm_theme());
wm.set_screen_size(w, h);          // after a resolution change, then:
wm.fit_to_screen(&mut sdi);        // refit maximized/snapped/tiled/kiosk windows,
                                   // pull off-screen ones back into the work area
wm.set_theme(theme);               // after a skin switch
wm.set_snap_enabled(true);         // drag-to-edge snapping (on by default)
wm.set_motion_enabled(!skin.features.reduced_motion); // animations (off by default)
```

Lifecycle (all take `&mut SdiRegistry`):

| Method | Effect |
|--------|--------|
| `create_window(&WindowConfig, sdi) -> Result<WindowId>` | Create chrome objects, cascade position, focus the window |
| `close_window(id, sdi)` / `close_all(sdi)` | Remove the window; with motion on, its chrome animates out before the SDI objects are destroyed |
| `minimize_window` / `maximize_window` / `restore_window` | State changes; maximize fills the work area |
| `enter_fullscreen` / `exit_fullscreen` | Kiosk fullscreen for an app window |
| `focus_window(id, sdi)` | Bring to front and mark active |
| `cycle_focus(forward, sdi) -> Option<WindowId>` | Alt+Tab behavior, skipping minimized windows |
| `move_window` / `resize_window` | Programmatic geometry changes |
| `hide_all_window_sdi` / `show_all_window_sdi` | Hide or show every window's objects (e.g. when switching to the fullscreen terminal) |

Input:

- `handle_input(&InputEvent, sdi) -> WmEvent` consumes `PointerClick`,
  `CursorMove` and `PointerRelease`. Clicks hit-test topmost-first:
  titlebar buttons, titlebar (drag; a second click within 500 ms and
  6 px toggles maximize), resize handles, content (`ContentClick` with
  content-local coordinates), then desktop. Clicking an unfocused window
  focuses it first. Modal windows block input to windows below.
- Keyboard shortcuts are handled by the host, not by
  `handle_input` (see below).

Queries: `window_count`, `windows`, `get_window`, `active_window`,
`window_at(x, y)`, `topmost_visible`, `has_modal`, `topmost_modal`,
`has_fullscreen_kiosk`, `is_dragging`, `theme`.

Rendering:

- `draw_with_clips(sdi, backend, |id, x, y, w, h, backend| ...)` draws
  the base SDI scene, then each visible window's chrome and, inside a
  clip rect, calls the closure to paint that window's content.
- `draw_with_clips_overlay(sdi, backend, overlay, content)` adds a hook
  between the base pass and the windows (for vector-icon dashboards).
- `draw_with_clips_noalloc` is the allocation-free variant for
  constrained platforms.

## Snapping

While a titlebar drag is inside a screen-edge zone (16 px edges, 64 px
corners) the WM publishes `snap_preview() -> Option<SnapPreview>`; the
desktop host draws it as a translucent rectangle tinted with the titlebar
color. Releasing snaps the window:

| Zone | Result |
|------|--------|
| Left / right edge | Left / right half of the work area |
| Corners | Quarter of the work area |
| Top edge | Maximize |

The work area is the screen minus `maximize_top_inset` /
`maximize_bottom_inset` (status bar, taskbar). Snapped windows remember
their pre-snap geometry: dragging one away (or `unsnap_window`) restores
it, and a manual resize clears the snap. `snap_window(id, zone, sdi)`
and `snap_zone_geometry(zone)` expose the same logic programmatically.

## Keyboard window management

`keyboard_snap_window(id, KeyboardSnapDirection, sdi)` and
`cycle_tiling(sdi) -> Option<TilingLayout>` implement the shortcuts; the
desktop host (`crates/oasis-app/src/input.rs`, `handle_wm_shortcut`)
maps keys to them before routing keys to the focused app:

| Shortcut | Action |
|----------|--------|
| Alt+Tab / Alt+Shift+Tab | Cycle focus forward / backward, skipping minimized windows (disabled while a modal is open) |
| Super+Left/Right or Ctrl+Alt+Left/Right | Snap the active window to that half; the opposite direction unsnaps |
| Super+Up or Ctrl+Alt+Up | Maximize |
| Super+Down or Ctrl+Alt+Down | Restore a maximized/snapped window, otherwise minimize |
| Super+T or Ctrl+Alt+T | Cycle tiling: master/stack, grid, columns, rows, monocle, then back to floating (pre-tiling geometry restored) |

Every combo needs Ctrl, Alt or Super, so plain Tab, arrows and letters
still reach text-entry windows. Ctrl+Alt+Arrow exists because desktop
OSes often swallow Super+Arrow.

## Animations

With `set_motion_enabled(true)`, call `tick_animations(sdi)` once per
frame (or `tick_animations_by(delta_ms, sdi)` for deterministic stepping):

| Kind | Motion | Default duration |
|------|--------|------------------|
| Open | Grow from 90% + fade in | 200 ms |
| Close | Shrink + fade out; SDI objects destroyed when done | 150 ms |
| Minimize | Shrink toward the bottom edge, then hide | 250 ms |
| Restore from minimized | Grow back from the bottom edge | 200 ms |

Animations are visual only: logical geometry and state change
immediately, and any logical geometry change (maximize, snap, resize)
finishes a running animation. `is_animating()` tells idle-frame elision
to keep drawing (see [writing-apps.md](writing-apps.md#frames-and-idle-elision)).
Motion is off by default so tests, the screenshot tool and other
headless hosts stay instant; the desktop app enables it unless the skin
sets `reduced_motion`.

## Decorations

- `button_side = "left"` gives the macOS order (close, minimize, maximize
  from the corner inward); `"right"` gives minimize, maximize, close.
- `Window::title_layout` reserves the button group on its side (mirrored
  on the other side for `title_align = "center"`), centers titles on
  their measured glyph width, and truncates titles that do not fit with
  `...`. It is recomputed on resize.
- Glyphs, hover colors, gradients, separators, text shadow and content
  stroke are all `WmTheme` fields driven by `[wm_theme]`.

## Tests

Unit tests live next to each module; `crates/oasis-wm/tests/wm_integration.rs`
drives full create / drag / snap / close sequences against an
`SdiRegistry`.
