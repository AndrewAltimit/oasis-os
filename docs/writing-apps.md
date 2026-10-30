# Writing Apps

Every dashboard app in OASIS_OS (File Manager, Text Editor, Paint,
Calculator, ...) is a type implementing the `App` trait from
`oasis-app-core`. Apps only see `oasis-app-core` types and the
`SdiBackend` / `Vfs` traits, never a platform API. Today the SDL desktop
app and the WASM build host them through `oasis-core`'s `AppRunner`,
launched from an `AppRegistry` that the `oasis-apps` bundle fills. Both
hosts always draw apps in windows: a "fullscreen" app is a kiosk window
that fills the screen, still drawn through `draw_windowed` and clicked
through `handle_click` with the same geometry. There is no separate
fullscreen / SDI render path for apps. The PSP EBOOT and the FFI library
have their own app surfaces. This guide covers the
trait, how hosts drive it, and a minimal working app.

Source of truth: [`crates/oasis-app-core/src/app_trait.rs`](../crates/oasis-app-core/src/app_trait.rs)
and [`crates/oasis-app-core/src/lib.rs`](../crates/oasis-app-core/src/lib.rs).
Design background: [design.md §4.3](design.md#43-input-pipeline)
(4.3.1 keyboard model, 4.3.2 VFS access, 4.3.3 ticking, 4.3.4 idle
elision).

## Where apps live

- One crate per app: `crates/oasis-app-<name>/`, depending on
  `oasis-app-core`, `oasis-types`, `oasis-vfs`, `oasis-sdi`,
  `oasis-skin` and (for widgets) `oasis-ui` -- never on `oasis-core`.
  (`oasis-app-system-monitor` holds three small info apps: System
  Monitor, Network and Package Manager.)
- `oasis-core` owns the runtime but knows no concrete app:
  `AppRunner` (`src/apps/runner.rs`) wraps a `Box<dyn App>`, and an
  `AppRegistry` (`src/apps/registry.rs`) maps a title to an
  `AppRegistration` (factory, optional file factory, dashboard flag,
  owned media). The host owns the registry and passes it
  to `AppRunner::launch(&apps, &entry, &vfs)`; titles it doesn't know
  launch a `SimpleApp` placeholder.
- `crates/oasis-apps` is the bundle: it depends on every app crate and
  exports `builtin_registry(&HostInfo)` (the host's platform / backend
  names reach Settings, System Monitor and Package Manager), the `tv`
  command (`register_commands`) and `TvGuideRunnerExt`. Plugins append
  their apps to the same registry with
  `PluginManager::extend_registry`.
- The desktop host seeds a dashboard tile (`/apps/<Title>` directory,
  `crates/oasis-app/src/vfs_setup.rs`) for every registration marked
  `on_dashboard`, plus the host's own Browser. Apps without a tile
  (Network, Package Manager, System Monitor) launch by title through
  `open <app> [file]` in the terminal or scripts, MCP `open_app` and
  `OASIS_APP=<title>` -- all three search the registry.

## The `App` trait

`pub trait App: AsAny + std::fmt::Debug + Send`. Required methods have
no default; everything else is opt-in. `AsAny` (`as_any` /
`as_any_mut`, for host downcasts) is implemented for every `App` by a
blanket impl -- don't write it yourself; import
`oasis_app_core::AsAny` where you call it on a concrete type.

| Method | Default | Called when | Notes |
|--------|---------|-------------|-------|
| `title(&self) -> &str` | required | always | Title bar / taskbar text |
| `path(&self) -> &str` | required | always | App's VFS path |
| `handle_input(&mut self, &Button, &dyn Vfs) -> AppAction` | required | Gamepad-style button press (d-pad, Confirm, Cancel, Triangle, Square, Start, Select) | The one input path every host has, incl. PSP. Every feature must be reachable from here |
| `handle_text_input(&mut self, char)` | no-op | Printable character typed | Type text from here |
| `handle_backspace(&mut self)` | no-op | Backspace | |
| `handle_key(&mut self, &Key, Modifiers, &dyn Vfs) -> Option<AppAction>` | `None` | Every key-down on keyboard hosts, **before** the key's gamepad twin | Return `Some(action)` to consume the key; the host then drops its twin. Never called on PSP |
| `accepts_text(&self) -> bool` | `false` | Before routing a typing key | While `true`, letters / digits / Space are treated purely as typing (Space no longer fires Triangle, Q/E no longer switch desktops) |
| `handle_scroll(&mut self, delta, &dyn Vfs) -> bool` | repeats `Up` / `Down` through `handle_input` (max 16 per event) | Mouse wheel over the app's window (desktop and WASM) | `delta` < 0 = toward the top. Return `true` when the view changed. Override where Up / Down isn't list movement (Games, Paint's canvas, Calculator, Settings' Audio tab) |
| `layout(&mut self, cw, ch, &ActiveTheme)` | no-op | Right before every `draw_windowed` and `handle_click`, with the same size | Record the content size and layout metrics (visible rows, hit-test geometry) in plain fields -- no `Cell`s in `draw_windowed`. Keys / buttons / wheel see the metrics of the last drawn frame |
| `handle_click(&mut self, lx, ly, &dyn Vfs) -> AppAction` | `AppAction::None` | Click / tap in the content area | Content-local coordinates; hit-test against the geometry `layout` just recorded, i.e. what `draw_windowed` draws (kiosk windows included). Discrete clicks only (no drag / move events). Read-only VFS follow-ups (re-list a folder, open the clicked file) happen right here. `ContentState::line_at(ly)` maps a click to a line for `ContentState` apps |
| `draw_windowed(&self, cx, cy, cw, ch, &mut dyn SdiBackend, &ActiveTheme) -> Result<()>` | required | Every drawn frame, inside the WM's clip rect | The app's only render path: draw directly to the backend |
| `drain_requests(&mut self, &mut Vec<AppRequest>)` | no-op | Once per frame for every open app | Move every queued VFS IPC request (`AppRequest { path, data }`) into the host's list; the host performs all of them, in order |
| `apply_vfs_ops(&mut self, &mut dyn Vfs) -> bool` | `false` | Once per frame for every open app | The only hook with **mutable** VFS access: apply queued writes, deletes, renames, binary saves. Return `true` when something was applied |
| `tick(&mut self, dt_ms: u32, &dyn Vfs) -> bool` | `false` | Once per frame for every open app, even on elided frames | Advance time-driven state by wall time, and poll VFS state you mirror (the Internet Radio's status, Video Embed's search results); return `true` when something you draw changed |
| `take_close_request(&mut self) -> bool` | `false` | Once per frame, after `apply_vfs_ops` and `tick` | Close yourself outside an input handler (e.g. after a "Save & close" write) |
| `release_resources(&mut self, &mut dyn SdiBackend)` | no-op | Once, after the app's window closed and before the app is dropped | Destroy every texture / render target the app created (e.g. the Photo Viewer's image) |
| `on_close_requested(&mut self, &dyn Vfs) -> AppAction` | `Exit` | Titlebar close button, `wm close`, MCP `close_window` | Return `Exit` to close now; anything else keeps the window open (e.g. raise an unsaved-changes prompt, as the Text Editor does) |
| `wants_frame(&self) -> bool` | `false` | Idle-frame check | `true` only for content that changes on its own every frame (video) |
| `lines(&self) -> &[String]` | `&[]` | Shared line-list renderers, tests, host text reads | |
| `browse_dir` / `viewing_file` | `None` | Host queries | For file-browsing apps |

To test an app inside the real shell (launch from the dashboard, clicks,
keys, what gets painted), write a scenario with the headless harness; see
[testing.md](testing.md).


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

1. **Launch.** The host calls `AppRunner::launch(&apps, &AppEntry,
   &dyn Vfs)` (or `launch_with_file`), which looks the title up in its
   `AppRegistry` and calls the registration's factory. Construct
   cheaply; defer VFS reads to the first `tick` if they can be slow.
2. **Input.** One physical input reaches the focused app (the wheel:
   the window under the pointer) through at most these hooks, in order:

   | Input | Hooks |
   |---|---|
   | Keyboard key | `handle_key`; if it returns `None`, the key's gamepad twin (`handle_input`, or `handle_backspace`) -- dropped for typing keys while `accepts_text()`; then `handle_text_input` for printable keys |
   | Gamepad button (PSP, on-screen keyboard) | `handle_input` |
   | Typed text (IME, OSK) | `handle_text_input` / `handle_backspace` |
   | Pointer click in the content area | `layout`, then `handle_click` |
   | Mouse wheel | `handle_scroll` |

   Only `Button` presses reach apps: the shoulder triggers (L/R, Q/E)
   switch virtual desktops / dashboard tabs, and `Tab` is the host's.
3. **Per frame, for every open app** (focused or not):
   `drain_requests`, `apply_vfs_ops(&mut vfs)`, `tick(dt_ms, &vfs)`,
   `take_close_request`; then closed apps get `release_resources`.
4. **Draw** (only on frames that are not elided): `layout` +
   `draw_windowed` for each window.
5. **Exit.** On `AppAction::Exit`, the window's close button or a
   programmatic close (`wm close`, MCP; on the desktop those ask
   `on_close_requested` first) the host closes the window, stops the
   media the app owns (`AppRegistration::owns_media`: Music Player,
   Internet Radio), calls `release_resources` and drops the runner.

Hooks that only receive `&dyn Vfs` cannot write. Queue the work in a
field and perform it in `apply_vfs_ops` (the Text Editor's saves, the
File Manager's copy / move / delete queue and Paint's BMP saves all do
this), then report success or failure in your own UI.

## Frames and idle elision

The SDL host skips clear / draw / present on frames where nothing
visible changed. For an app window it redraws when:

- the runner saw input (or a wheel event that `handle_scroll` reported
  as a change), `tick` returned `true`, `apply_vfs_ops` returned `true`,
  or the host reached into the app with `delegate_as_mut` (tracked by
  `AppRunner::wants_frame` until `mark_drawn`), or
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

`draw_windowed` is the only render path: draw into `(cx, cy, cw, ch)`
with the backend. Use `oasis-ui` widgets through a `DrawContext` (see
[ui-widgets.md](ui-widgets.md)). Compute your layout in `layout` and
keep it in a field that both drawing and `handle_click` use, so
hit-testing matches what was drawn. A kiosk-fullscreen window is just a
bigger content area: there is no fullscreen flag and no SDI path to
keep in sync.

Line-oriented apps can skip it by embedding a `ContentState` and
using `impl_content_app_methods!(field)`, which implements `title`,
`path`, `lines`, `browse_dir`, `viewing_file`, `layout`,
`draw_windowed` and `drain_requests` with the shared renderer in
`oasis_app_core::render`. `ContentState` also provides
cursor / scroll helpers (`navigate_up`, `navigate_down`, `handle_nav`
for the standard Cancel / Up / Down), `post_request` to queue a VFS IPC
request, and `update_layout_windowed` / `windowed_metrics` so a click
handler can map a click to a line with `render::windowed_line_at`.

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
    // title, path, lines, browse_dir, viewing_file, layout, draw_windowed,
    // drain_requests (as_any comes from AsAny).
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

1. Create `crates/oasis-app-<name>/` (start from
   [`stopwatch_app.rs`](../crates/oasis-app-core/examples/stopwatch_app.rs)),
   add it to the workspace `members` and `[workspace.dependencies]` in
   the root `Cargo.toml`, and add it as a dependency of `oasis-apps`
   (not `oasis-core`).
2. Register it in `builtin_registry` in
   [`crates/oasis-apps/src/lib.rs`](../crates/oasis-apps/src/lib.rs):

   ```rust,ignore
   apps.register(
       AppRegistration::new("Stopwatch", |path, _vfs| {
           Box::new(oasis_app_stopwatch::StopwatchApp::new(path))
       })
       .on_dashboard(),
   );
   ```

   Builder options: `.on_dashboard()` (a tile on hosts that seed tiles
   from the registry), `.open_file(|path, file, vfs| ...)` ("open this
   file with me": `AppAction::LaunchAppWithFile` hand-offs and
   `open <app> <file>`), `.owns_media(OwnedMedia::Music | Radio)`.
3. Make sure the title maps to its own dashboard icon category
   (`oasis_vector::icon_set::IconCategory::from_app_title`, a title
   keyword classifier). The `oasis-apps` tests run every registered
   title through it and through the dashboard label layout of every
   built-in skin, so a title with no category, a category clash or a
   label that overflows its cell fails there.

## Testing

Apps are plain structs, so test them in-module (`#[cfg(test)] mod tests`)
against `oasis_vfs::MemoryVfs`: feed `handle_input` / `handle_key` /
`handle_click`, call `tick` with synthetic `dt_ms`, drain
`apply_vfs_ops(&mut vfs)` and `drain_requests`, and assert on `lines()`,
your own state or VFS contents. Drawing can be exercised with the mock
backends in `oasis-test-backend`.

For end-to-end app tests, enable `oasis-app-core`'s `testing` feature in
your dev-dependencies and use `oasis_app_core::testing::AppHarness`. It
hosts one app the way the desktop shell does: `press`, `key` /
`key_mods` / `ctrl` (routed `handle_key` -> twin -> text like the
shell), `type_text`, `click` / `click_text` (after `layout`), `scroll`,
`frame(dt_ms)` (requests, `apply_vfs_ops`, `tick`, close poll),
`take_request` / `requests`, `set_size` / `set_theme`, and `draw` /
`screen_text`, which fail the test if anything is drawn outside the
window. `fuzz_app` runs a seeded random-input property test over sizes
and themes. Most `oasis-app-*` crates have a `tests/e2e.rs` built on it.
