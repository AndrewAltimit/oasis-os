# Testing

OASIS_OS has three layers of automated tests:

| Layer | Where | What it catches |
|-------|-------|-----------------|
| Unit tests | `#[cfg(test)] mod tests` in every crate | Logic of one module in isolation |
| Integration suites | `crates/*/tests/` | A crate's public API end to end |
| **Shell end-to-end** | `crates/oasis-app/tests/e2e_*.rs` | Full user flows through the real desktop shell |

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
  sample media is loaded, no environment variable is consulted.

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
| `Harness::with_options(HarnessOptions)` | `skin`, `resolution`, `shader_wallpaper` (off by default: slow in debug), `fixed_time` |
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

### Guidelines

- **Drive the UI like a user.** Prefer `click_app_icon`, `key`, `click`
  on something located from the rendered frame over poking state. Inject
  state only for what the harness cannot produce offline (catalogs,
  decoded media).
- **Assert on the output, not only on state.** The TV volume scenario
  checks the guide's `volume` *and* the samples that reached
  `RecordingAudio`; the splash test checks pixels.
- **Keep scenarios hermetic.** No network, no files outside a temp dir,
  no environment variables (tests run in parallel threads).
- **Per-skin loops** (`builtin_names()`) catch skin-specific layout and
  feature-flag breakage cheaply: a boot is ~0.1 s, a settled frame a few ms
  (debug build).
- Regression tests for bugs the harness found go in
  `tests/e2e_regressions.rs` with a comment describing the old behavior.

### Limits

- Rendering is the software rasterizer, not SDL/GPU. Backend-specific
  bugs (e.g. SDL texture pitch handling) need backend tests; the harness
  catches everything above the `SdiBackend` boundary.
- The animated boot splash is not played during `Harness` boots (the
  `BootObserver` hook receives its progress instead); render splash frames
  directly with `BootSplash::render_at`.
- The ffmpeg decode path and real network streaming are out of scope;
  `oasis-video` and `tv_controller` have their own tests for those.
