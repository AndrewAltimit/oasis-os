# Writing Apps

Every dashboard app in OASIS_OS (File Manager, Text Editor, Paint,
Calculator, ...) is a type implementing the `App` trait from
`oasis-app-core`. Apps only see `oasis-app-core` types and the
`SdiBackend` / `Vfs` traits, never a platform API. Today the SDL desktop
app and the WASM build host them through `oasis-core`'s `AppRunner`,
windowed or fullscreen (the PSP EBOOT and the FFI library have their
own app surfaces). This guide covers the trait, how hosts drive it, and
a minimal working app.

Source of truth: [`crates/oasis-app-core/src/app_trait.rs`](../crates/oasis-app-core/src/app_trait.rs)
and [`crates/oasis-app-core/src/lib.rs`](../crates/oasis-app-core/src/lib.rs).
Design background: [design.md §4.3](design.md#43-input-pipeline)
(4.3.1 keyboard model, 4.3.2 VFS access, 4.3.3 ticking, 4.3.4 idle
elision).

## Where apps live

- One crate per app: `crates/oasis-app-<name>/`, depending on
  `oasis-app-core`, `oasis-types`, `oasis-vfs`, `oasis-sdi`,
  `oasis-skin` and (for widgets) `oasis-ui` -- never on `oasis-core`.
- `oasis-core` owns the runtime: `AppRunner` (`src/apps/runner.rs`)
  wraps a `Box<dyn App>`, and `APP_REGISTRY` (`src/apps/registry.rs`)
  maps a dashboard title to a factory `fn(&str, &dyn Vfs) -> Box<dyn App>`.
  Titles not in the registry launch a `SimpleApp` placeholder; plugins
  can hand a pre-built app to `AppRunner::from_delegate`.
- The desktop host seeds dashboard entries as `/apps/<Title>`
  directories (`crates/oasis-app/src/vfs_setup.rs`). Registered apps
  without a directory (Network, Package Manager, System Monitor) are
  still launchable by name from the terminal, scripts and MCP.

## The `App` trait

`pub trait App: std::fmt::Debug + Send`. Required methods have no
default; everything else is opt-in.

| Method | Default | Called when | Notes |
|--------|---------|-------------|-------|
| `title(&self) -> &str` | required | always | Title bar / taskbar text |
| `path(&self) -> &str` | required | always | App's VFS path |
| `handle_input(&mut self, &Button, &dyn Vfs) -> AppAction` | required | Gamepad-style button press (d-pad, Confirm, Cancel, Triangle, Square, Start, Select) | The one input path every host has, incl. PSP. Every feature must be reachable from here |
| `handle_text_input(&mut self, char)` | no-op | Printable character typed | Type text from here |
| `handle_backspace(&mut self)` | no-op | Backspace | |
| `handle_key(&mut self, &Key, Modifiers, &dyn Vfs) -> Option<AppAction>` | `None` | Every key-down on keyboard hosts, **before** the key's gamepad twin | Return `Some(action)` to consume the key; the host then drops its twin. Never called on PSP |
| `accepts_text(&self) -> bool` | `false` | Before routing a typing key | While `true`, letters / digits / Space are treated purely as typing (Space no longer fires Triangle, Q/E no longer switch desktops) |
| `handle_click(&mut self, lx, ly, cw, ch, fullscreen) -> AppAction` | `AppAction::None` | Click / tap in the content area | Content-local coordinates; discrete clicks only (no drag / move events) |
| `update_sdi(&mut self, &mut SdiRegistry, &ActiveTheme)` | required | Fullscreen rendering | Create / update named SDI objects |
| `draw_windowed(&self, cx, cy, cw, ch, &mut dyn SdiBackend, &ActiveTheme) -> Result<()>` | required | Windowed rendering, inside the WM's clip rect | Draw directly to the backend |
| `hide_sdi(&self, &mut SdiRegistry)` | required | Leaving fullscreen / switching apps | Hide every SDI object you created |
| `take_pending_request` / `peek_pending_request` | `None` | Each frame | String-only VFS IPC request `(path, data)` for the host |
| `refresh(&mut self, &dyn Vfs)` | no-op | Right after a forwarded click or a consumed key | Read-only follow-up (re-list a folder, reload state). Not a clock |
| `apply_vfs_ops(&mut self, &mut dyn Vfs) -> bool` | `false` | Once per frame for every open app | The only hook with **mutable** VFS access: apply queued writes, deletes, renames, binary saves. Return `true` when something was applied |
| `tick(&mut self, dt_ms: u32, &dyn Vfs) -> bool` | `false` | Once per frame for every open app, even on elided frames | Advance time-driven state by wall time; return `true` when something you draw changed |
| `wants_frame(&self) -> bool` | `false` | Idle-frame check | `true` only for content that changes on its own every frame (video) |
| `lines(&self) -> &[String]` | required | Generic scroll / render helpers, terminal mirroring | |
| `browse_dir` / `viewing_file` | `None` | Host queries | For file-browsing apps |
| `as_any` / `as_any_mut` | required | Host downcasts to a concrete app | Return `self` |

### `AppAction`

Returned from `handle_input`, `handle_key` and `handle_click`:

| Variant | Host behavior |
|---------|---------------|
| `None` | Input consumed, nothing else to do |
| `Exit` | Close the app / window and return to the dashboard |
| `SwitchToTerminal` | Switch to terminal mode |
| `RequestFullscreen` | Enter fullscreen kiosk mode |
| `LaunchAppWithFile { app_title, file_path }` | Launch another app with a file pre-opened (File Manager -> Photo Viewer / Music Player / Text Editor) |

## Lifecycle and frame order

1. **Launch.** The host calls `AppRunner::launch(&AppEntry, &dyn Vfs)`
   (or `launch_with_file`), which looks the title up in `APP_REGISTRY`
   and calls the factory. Construct cheaply; defer VFS reads to the
   first `tick` / `refresh` if they can be slow.
2. **Input.** For each input event the host routes to the focused app:
   - `Key { key, mods }` -> `handle_key`. If it returns `Some(action)`,
     the gamepad-style twin that follows (e.g. Enter -> `Confirm`) is
     dropped. If the app `accepts_text()` and the key just types a
     character, the twin is dropped too.
   - Button / trigger events -> `handle_input`; `TextInput` ->
     `handle_text_input`; Backspace -> `handle_backspace`.
   - Pointer clicks inside the window -> `handle_click` (content-local).
   - After a click or a consumed key the host calls `refresh(&dyn Vfs)`.
3. **Per frame, for every open app** (focused or not, windowed or
   fullscreen): `tick(dt_ms, &vfs)`, then `apply_vfs_ops(&mut vfs)`.
4. **Draw** (only on frames that are not elided): `draw_windowed` for
   windows, `update_sdi` for the fullscreen app.
5. **Exit.** On `AppAction::Exit` (or the window's close button) the
   host closes the window and drops the runner; implement `hide_sdi` so
   leaving fullscreen removes your SDI objects.

Hooks that only receive `&dyn Vfs` cannot write. Queue the work in a
field and perform it in `apply_vfs_ops` (the Text Editor's saves, the
File Manager's copy / move / delete queue and Paint's BMP saves all do
this), then report success or failure in your own UI.

## Frames and idle elision

The SDL host skips clear / draw / present on frames where nothing
visible changed. For an app window it redraws when:

- the runner saw input, `tick` returned `true`, `apply_vfs_ops` returned
  `true`, or the app's mirrored lines / browse dir / viewing file
  changed (tracked by `AppRunner::wants_frame` until `mark_drawn`), or
- `App::wants_frame()` returns `true`.

Rules of thumb:

- Drive time from `tick`'s `dt_ms` (accumulate it; never count calls).
  Games step a fixed 60 Hz simulation and cap catch-up per tick.
- Return `true` from `tick` **only** when the drawn output changed (a
  clock that shows tenths returns `true` ten times a second, not sixty).
- Leave `wants_frame` at `false` unless your content animates every
  frame without state changes the runner can see.
- Mutating an app from the host behind the runner's back? Call
  `AppRunner::request_redraw`.

## Rendering

Two paths, both required:

- **Windowed** (`draw_windowed`): draw into `(cx, cy, cw, ch)` with the
  backend. Use `oasis-ui` widgets through a `DrawContext` (see
  [ui-widgets.md](ui-widgets.md)), and keep one layout struct that both
  drawing and `handle_click` use so hit-testing matches what was drawn.
- **Fullscreen** (`update_sdi`): create or update named SDI objects and
  hide them in `hide_sdi`.

Line-oriented apps can skip both by embedding a `ContentState` and
using `impl_content_app_methods!(field)`, which implements `title`,
`path`, `lines`, `update_sdi`, `draw_windowed`, `hide_sdi`,
`take/peek_pending_request`, `as_any` and `as_any_mut` with the shared
renderers in `oasis_app_core::render`. `ContentState` also provides
cursor / scroll helpers (`navigate_up`, `navigate_down`).

Take every color from the `ActiveTheme`; per-app overrides go under
`[app_themes.<app>]` in the skin (see
[skin-authoring.md](skin-authoring.md)).

## Minimal example: a stopwatch

[`crates/oasis-app-core/examples/stopwatch_app.rs`](../crates/oasis-app-core/examples/stopwatch_app.rs)
is a complete app that compiles with the workspace (examples are built
by `cargo test`) and runs headless:

```bash
cargo run -p oasis-app-core --example stopwatch_app
```

It uses `ContentState` for rendering and implements the behavior hooks
(excerpt):

```rust,ignore
use oasis_app_core::{App, AppAction, ContentState, impl_content_app_methods};
use oasis_types::input::{Button, Key, Modifiers};
use oasis_vfs::Vfs;

const LAPS_PATH: &str = "/home/user/laps.txt";

#[derive(Debug)]
pub struct StopwatchApp {
    content: ContentState,
    running: bool,
    elapsed_ms: u64,
    laps: Vec<u64>,
    save_pending: bool,
    status: String,
}

impl App for StopwatchApp {
    // title, path, lines, update_sdi, draw_windowed, hide_sdi,
    // take/peek_pending_request, as_any, as_any_mut.
    impl_content_app_methods!(content);

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Cancel => return AppAction::Exit,
            Button::Confirm => {
                self.running = !self.running;
                self.status = if self.running { "Running" } else { "Stopped" }.to_string();
            },
            Button::Square => self.laps.push(self.elapsed_ms),
            Button::Up => self.content.navigate_up(),
            Button::Down => self.content.navigate_down(),
            _ => return AppAction::None,
        }
        self.rebuild_lines();
        AppAction::None
    }

    fn handle_key(&mut self, key: &Key, mods: Modifiers, _vfs: &dyn Vfs) -> Option<AppAction> {
        // Claim only exact Ctrl+<letter> combos; everything else falls
        // through to `handle_input` via the key's gamepad-style twin.
        if !mods.only(Modifiers::CTRL) {
            return None;
        }
        match key {
            Key::Char('r') => self.reset(),
            Key::Char('s') => {
                // Input hooks only get `&dyn Vfs`: queue the write.
                self.save_pending = true;
                self.status = "Saving...".to_string();
                self.rebuild_lines();
            },
            _ => return None,
        }
        Some(AppAction::None)
    }

    fn tick(&mut self, dt_ms: u32, _vfs: &dyn Vfs) -> bool {
        if !self.running {
            return false;
        }
        let before = self.elapsed_ms / 100;
        self.elapsed_ms += u64::from(dt_ms);
        // Only ask for a redraw when the displayed tenth changes.
        if self.elapsed_ms / 100 != before {
            self.rebuild_lines();
            return true;
        }
        false
    }

    fn apply_vfs_ops(&mut self, vfs: &mut dyn Vfs) -> bool {
        if !self.save_pending {
            return false;
        }
        self.save_pending = false;
        let text: String = self
            .laps
            .iter()
            .map(|lap| format!("{}\n", Self::format(*lap)))
            .collect();
        let result = vfs
            .mkdir("/home/user")
            .and_then(|()| vfs.write(LAPS_PATH, text.as_bytes()));
        self.status = match result {
            Ok(()) => format!("Saved {} laps to {LAPS_PATH}", self.laps.len()),
            Err(e) => format!("Save failed: {e}"),
        };
        self.rebuild_lines();
        true
    }
}
```

(`new`, `format`, `reset` and `rebuild_lines` are plain inherent
methods; see the example file.) The example's `main` drives the app the
way a host does: a `Confirm`, 90 ticks of 16 ms, a lap, `Ctrl+S` through
`handle_key`, then the `apply_vfs_ops` drain, and prints the result.

## Registering the app

1. Create `crates/oasis-app-<name>/` (copy a small crate such as
   `oasis-app-package-manager`), add it to the workspace `members` and
   `[workspace.dependencies]` in the root `Cargo.toml`, and add it as a
   dependency of `oasis-core`.
2. Add a factory to `APP_REGISTRY` in
   `crates/oasis-core/src/apps/registry.rs`:

   ```rust,ignore
   ("Stopwatch", |path, _vfs| {
       Box::new(oasis_app_stopwatch::StopwatchApp::new(path))
   }),
   ```

   For "open this file with me" support, add a case to
   `create_app_delegate_for_file` and return
   `AppAction::LaunchAppWithFile` from the launching app.
3. Give it a dashboard tile by adding the title to the `/apps` list in
   `crates/oasis-app/src/vfs_setup.rs` (and to its
   `populate_creates_all_app_dirs` test).

## Testing

Apps are plain structs, so test them in-module (`#[cfg(test)] mod tests`)
against `oasis_vfs::MemoryVfs`: feed `handle_input` / `handle_key` /
`handle_click`, call `tick` with synthetic `dt_ms`, drain
`apply_vfs_ops(&mut vfs)` and assert on `lines()`, your own state (via
`as_any().downcast_ref`) or VFS contents. Drawing can be exercised with
the mock backends in `oasis-test-backend`.
