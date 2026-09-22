# Testing

OASIS_OS has three layers of automated tests:

| Layer | Where | What it catches |
|-------|-------|-----------------|
| Unit tests | `#[cfg(test)] mod tests` in every crate | Logic of one module in isolation |
| Integration suites | `crates/*/tests/` | A crate's public API end to end |
| **Shell end-to-end** | `crates/oasis-app/tests/e2e_*.rs` | Full user flows through the real desktop shell |
| **Backend parity** | `crates/oasis-backend-sdl/tests/backend_parity.rs`, `crates/oasis-app/tests/sdl_parity.rs` | SDL rendering differently from the software rasterizer the harness uses |

Unit tests alone miss *wiring* bugs: a Settings slider that updates app
state but never reaches the audio path, a key the host swallows before the
focused app sees it. The shell e2e harness exists for those.

```bash
# Everything in the desktop crate (unit + e2e):
cargo test -p oasis-app
# Local Windows / no ffmpeg dev libs:
cargo test -p oasis-app --no-default-features --features javascript,video-decode
# One scenario:
cargo test -p oasis-app --test e2e_shell -- start_menu
```

## The shell e2e harness

`oasis-app` is a library plus thin binaries. The whole desktop shell lives
in `oasis_app::Shell` (`crates/oasis-app/src/shell.rs`): the boot sequence,
the per-frame `step` (input dispatch, app ticks, TV / radio / music
controllers, window manager, SDI scene update) and `render`. `main.rs` runs
it in an SDL3 window; `oasis_app::harness::Harness` runs the **same code**
headlessly:

- **Rendering** goes to `HeadlessBackend` (`src/headless.rs`): the UE5
  software RGBA framebuffer (`oasis-backend-ue5`, the rasterizer the FFI
  embeds) plus per-frame recording of every `draw_text` call.
- **Audio** goes to `RecordingAudio` (`src/harness.rs`), which keeps every
  PCM sample, byte and track operation the shell sends.
- **Time** is virtual: each frame advances exactly 1/60 s, and the
  platform clock is frozen at `2025-06-15 12:00:00` (the screenshot CI's
  `OASIS_FIXED_TIME`).
- **Network** is off: `AppState::offline` stops TV catalog fetches, video
  downloads and radio streams. A TV tune starts an *injected* decode
  session instead of a download, so scenarios can feed decoded frames and
  audio into the real player (`Harness::inject_video_audio` /
  `inject_video_frame`).
- **Persistence** is off: no settings file is read or written, no host
  sample media is loaded, no environment variable is consulted. A
  scenario that tests persistence sets `HarnessOptions::prefs_path` to a
  file in a temp directory: it is read at boot like the binary's settings
  file (saved skin, resolution, volume, font scale, reduced motion,
  locale) and written as preferences change; `shutdown()` flushes it and
  a second `Harness` on the same path is the "re-boot". Never point it at
  the real user settings file.

Frames are presented exactly when the shell's idle-frame elision says so,
as in the binary. Each input helper runs at least one frame, so state is
observable right after the call.

### A scenario

```rust
use oasis_app::Mode;
use oasis_app::harness::Harness;

#[test]
fn calculator_opens_and_closes() {
    let mut h = Harness::new("classic"); // any skin name or skin directory
    h.settle();                          // boot entrance transition done
    assert!(h.click_app_icon("Calculator"));
    h.settle();
    assert_eq!(h.mode(), Mode::Desktop);
    h.render_now();
    assert!(h.text_drawn_contains("Calculator")); // window title painted
    assert!(h.close_window("Calculator"));        // titlebar close button
    h.settle();
    assert_eq!(h.mode(), Mode::Dashboard);
}
```

### API

Construction

| Call | Does |
|------|------|
| `Harness::new(skin)` | Boot with defaults (panics if the skin fails to load) |
| `Harness::with_options(HarnessOptions)` | `skin`, `resolution`, `shader_wallpaper` (off by default: slow in debug), `fixed_time`, `prefs_path` (persist preferences to a real file, see above) |
| `Harness::with_observer(opts, &mut obs)` | Also report boot progress (BIOS lines, splash waits) to a `BootObserver` |

Frames and time

| Call | Does |
|------|------|
| `step(&[InputEvent])` | One frame with raw events; renders if the shell wants a redraw |
| `run_frames(n)` / `advance(Duration)` | Idle frames |
| `settle()` | Run until no transition / window animation and the SDI scene is unchanged for 3 frames (max 5 s) |
| `render_now()` | Present a frame even if it would be elided (before pixel assertions) |
| `frames()`, `rendered_frames()`, `last_outcome()`, `quit_requested()` | Loop bookkeeping |

Input (mirrors what the SDL backend emits)

| Call | Does |
|------|------|
| `click(x, y)` | Cursor move + press this frame, release next frame |
| `move_to(x, y)`, `drag(from, to, steps)`, `scroll(delta)` | Pointer |
| `key(Key)`, `key_with(Key, Modifiers)` | `InputEvent::Key` + its gamepad twin (`Key::legacy_press`), then the release |
| `type_text(str)` | Per char: key event + twin + `TextInput` |
| `button(Button)`, `trigger(Trigger)` | Gamepad press + release (PSP-style input) |
| `send(&[InputEvent])` | Anything else (e.g. `ToggleFullscreen`, `Quit`) |

Shell-level actions

| Call | Does |
|------|------|
| `dashboard_apps()`, `app_icon_rect(title)`, `click_app_icon(title)` | Dashboard icons on the current page |
| `open_app(title)` | Launch like `OASIS_APP` auto-launch (no clicking) |
| `windows()`, `find_window(title)` | `WindowInfo { id, title, frame, content, close_button, minimized, fullscreen }` |
| `close_window(title)` | Click the titlebar close button |
| `app_runner(title)` | The open app's `AppRunner` (e.g. `.tv_guide_state()` to inject fixtures) |
| `terminal(cmd)` | Open the terminal (Start), type `cmd`, press Enter |
| `click_sdi_text(text)`, `find_sdi_text(text)`, `sdi_rect(name)` | Hit things by their SDI label / object name |
| `vfs()`, `vfs_mut()` | The shell VFS (e.g. post a Settings IPC request) |
| `inject_video_audio(pcm, ch, rate)`, `inject_video_frame(rgba, w, h, pts)` | Feed the injected TV decode session (`_video` builds) |

Observation

| Call | Does |
|------|------|
| `mode()`, `state()`, `state_mut()`, `shell` | Full `AppState`, SDI registry, backend |
| `screenshot()`, `pixel(x, y)`, `size()`, `distinct_colors()` | Framebuffer (RGBA8) |
| `frame_text()`, `frame_text_calls()`, `text_drawn_contains(s)` | Strings painted in the last presented frame, including window content drawn outside the scene graph |
| `sdi_texts()`, `sdi_text_contains(s)` | Text of visible SDI objects |
| `audio()`, `audio_fed()` | `AudioLog`: PCM chunks (with track / channels / rate), bytes, SFX samples, tracks opened / unloaded, volume |
| `save_png(path)` | Dump the framebuffer when debugging a failure |
| `shutdown()` | Shut down like the binary on exit (flushes the settings file) |

### Guidelines

- **Drive the UI like a user.** Prefer `click_app_icon`, `key`, `click`
  on something located from the rendered frame over poking state. Inject
  state only for what the harness cannot produce offline (catalogs,
  decoded media).
- **Assert on the output, not only on state.** The TV volume scenario
  checks the guide's `volume` *and* the samples that reached
  `RecordingAudio`; the splash test checks pixels.
- **Keep scenarios hermetic.** No network, no files outside a temp dir,
  no environment variables (tests run in parallel threads). The one
  exception is loopback: `tests/e2e_remote.rs` starts the remote terminal,
  FTP and MCP servers from the shell terminal and drives them with real
  `127.0.0.1` clients, stepping the harness between non-blocking reads.
- **Per-skin loops** (`builtin_names()`) catch skin-specific layout and
  feature-flag breakage cheaply: a boot is ~0.1 s, a settled frame a few ms
  (debug build).
- **Multi-app user journeys** (Settings, File Manager / Text Editor /
  Photo Viewer, Music, Radio, terminal to apps, locale, several windows)
  live in `tests/e2e_user_flows.rs`. Its helpers find things by the text
  painted in the last frame (`click_text_in`, `window_shows`,
  `click_taskbar`).
- The UI locale is process-global: a scenario that switches it must not
  run at the same time as scenarios asserting English text in the same
  test binary (`e2e_user_flows.rs` serializes them with an `RwLock`).
- Regression tests for bugs the harness found go in
  `tests/e2e_regressions.rs` with a comment describing the old behavior.

### Limits

- Rendering is the software rasterizer, not SDL/GPU. Backend-specific
  bugs (e.g. SDL texture pitch handling) are covered by the backend
  parity suites below; the harness catches everything above the
  `SdiBackend` boundary.
- The animated boot splash is not played during `Harness` boots (the
  `BootObserver` hook receives its progress instead); render splash frames
  directly with `BootSplash::render_at`.
- The ffmpeg decode path and real network streaming are out of scope;
  `oasis-video` and `tv_controller` have their own tests for those.

## Backend parity

The SDL3 backend, the UE5 software framebuffer (`oasis-rasterize`, what
the harness and the FFI render with) and the WASM canvas backend must put
the same pixels on screen for the same `SdiBackend` calls. Shapes, glyphs
and line metrics come from shared definitions in `oasis-rasterize`
(`glyph_mask`, `bitmap_line_height` / `bitmap_ascent`, and the span
generators `thick_line_rows`, `stroke_circle_rows`,
`stroke_rounded_rect_rows`, `polygon_rows`, `rounded_rect_rows`); the
parity suites check the backends actually agree.

```bash
# ~60 primitive scenarios, SDL vs software, plus SDL-only texture
# streaming (update_texture at odd widths, the shader wallpaper bridge):
cargo test -p oasis-backend-sdl --test backend_parity
# The real boot splash and settled dashboards through Shell<SdlBackend>
# vs the harness backend:
cargo test -p oasis-app --test sdl_parity
```

- **Scenarios** live in `oasis_test_backend::conformance::SCENARIOS`:
  a draw function over `&mut dyn SdiBackend`, optional absolute checks
  (exact rect bounds, clip containment, unsheared odd-width textures, the
  mathematically expected blend of a translucent layer) and a
  cross-backend `Tolerance`. Any tolerance above exact must carry a note
  saying why; today they are alpha-blend rounding (SDL truncates, the
  rasterizer rounds: 1-3 levels) and SDL's linear filtering of *scaled*
  textures (the rasterizer samples nearest). Add a scenario whenever a
  backend grows a new primitive or a rendering bug is fixed.
- **Headless SDL**: `SdlBackend::new_headless` forces the `offscreen`
  video driver and the `software` renderer through in-process SDL hints,
  so the suites need no display and run in the CI container. SDL may only
  be initialized from one thread at a time, so each suite serializes its
  SDL tests behind a lock.
- **Hardware renderers**: the software renderer packs texture rows
  tightly, so it can never exercise a padded texture pitch (the D3D11
  "diagonal stripes" bug). Run the same suites against a GPU renderer
  locally with `OASIS_PARITY_RENDER_DRIVER=direct3d11` (or `opengl`,
  `vulkan`, `metal`).
- **Failure dumps**: mismatching frames are written as PNGs (`-sdl`,
  `-ue5`/`-soft`, `-diff`) under `target/tmp/backend-parity/` and
  `target/tmp/sdl-parity/`; `OASIS_PARITY_DUMP=1` dumps every shell frame.
- **Not covered**: skin TrueType fonts (`SdiText::set_font`) are
  rendered by SDL only; the UE5 backend keeps the bitmap font. The WASM
  backend's canvas calls cannot run natively; only its shared pure-Rust
  pieces (glyph masks, metrics) are exercised.
