# Boot Splash

Desktop boot is a **functional** splash: the 6.5-second animation runs
in the foreground while `main.rs` performs real initialization work
between animation frames. Each BIOS-phase line reports the result of a
completed probe or registration step, and the splash-phase warms heavy
subsystems (wallpaper, cursor, shader bridge, SDI layout, audio) so the
dashboard's first frame has no startup hitch.

Skip with `OASIS_SKIP_SPLASH=1` — the same init work runs in the same
order, minus the animation.

## Phase timeline

| Elapsed | Event                                              | Work unit                                  |
|--------:|----------------------------------------------------|--------------------------------------------|
|   0.0 s | Splash starts — GPU textures precomputed           | `SplashTextures::create`                   |
|   0.0 s | Banner fades in (logo + subtitle + progress bar)   | —                                          |
|   0.4 s | BIOS line 0 reveals: kernel header + `{arch}`      | Probe: `sysinfo::cpu_arch()`               |
|   0.8 s | Line 1: `HOST KERNEL: {os} | OASIS_OS V{ver}`       | Probe: `sysinfo::os_release()`             |
|   1.2 s | Line 2: RAM + logical core count                   | Probe: `/proc/meminfo`, `/proc/cpuinfo`    |
|   1.6 s | Line 3: VFS file/dir count + total KB              | Populate VFS, man/motd/profile, count     |
|   2.1 s | Line 4: skin name, version, native resolution      | (skin already loaded pre-splash)           |
|   2.6 s | Line 5: plugin + plugin-app counts                 | Build `CommandRegistry`, `PluginManager`   |
|   3.0 s | Line 6: display manager + backend                  | —                                          |
|   3.4 s | CRT flicker transition begins                      | Shader bridge init                          |
|   3.5 s | Splash-phase background reveals                    | Glyph atlas pre-raster, status bar prime   |
|   3.8 s | Horizon glow begins                                | App discovery, dashboard, WM, AppState     |
|   4.0 s | Logo entrance (scale + brightness)                 | `SdiRegistry::apply_layout_scaled`         |
|   4.6 s | (splash continues)                                 | Wallpaper generate + upload                |
|   5.2 s | (splash continues)                                 | Cursor generate + upload; auto-launch      |
|   5.5 s | Progress note fades in: "SYSTEM MODULES INITIALIZED" | —                                        |
|   6.5 s | Fade out, release GPU textures                     | —                                          |

The actual work distribution is set by the caller; the `BootSplash`
struct just owns the animation timeline.

## `BootSplash` API (`crates/oasis-app/src/boot_splash.rs`)

Stateful; the caller drives it frame by frame:

- `BootSplash::start(backend, w, h)` — precompute GPU textures, return
  a handle. Start time is captured here.
- `run_until(backend, target_secs) -> Result<bool>` — render frames
  until `target_secs` elapses, the user presses any button to skip,
  or the splash ends. Returns `true` if skipped.
- `set_bios_line(idx, text)` — overwrite a BIOS line in place. Safe
  before or after the line has revealed.
- `set_status(text)` — set the live `> [spinner] {text}...` line below
  the BIOS block. Empty string hides the line. The spinner cycles
  `| / - \` at 10 Hz.
- `set_progress_note(text)` — set the text shown at the bottom of the
  splash phase (5.5s onward; defaults to "SYSTEM MODULES INITIALIZED").
- `run_to_end(backend)` — convenience wrapper for `run_until(.., 6.5)`.
- `finish(backend)` — consume, fade to black (if not skipped), release
  GPU textures.

### Interleaving pattern

```rust
let mut splash = BootSplash::start(&mut backend, w, h)?;

// Do work, then advance the animation to the next BIOS reveal time.
splash.set_bios_line(2, format!("SYSTEM RAM CHECK... {ram}K OK"));
splash.set_status("Probing physical memory...");
splash.run_until(&mut backend, BIOS_REVEAL_TIMES[2])?;

// ... repeat for each line / splash-phase step ...

splash.set_progress_note("SYSTEM MODULES INITIALIZED");
splash.run_to_end(&mut backend)?;
splash.finish(&mut backend)?;
```

## What the visible banner is drawing

- **Dark rounded-rect banner** spanning the top of the screen.
- **Miniature `[OASIS]` logo** on the left, rendered via
  `paint_mini_oasis_logo` using the same stroke coordinates as the
  full-screen splash logo but at ~0.19× scale.
- **Subtitle** on the right: `BIOS / RUNTIME SERVICES CORE v7.0.4`.
- **Phase progress bar** below the banner — fills as
  `elapsed / 3.0s`, so the user sees continuous progress during the
  BIOS phase rather than a static chrome element.

## Live status line

Drawn below the last BIOS line at y≈360 (base 720). Shows
`> [{spinner}] {status_line}` where:

- Spinner ∈ `["|", "/", "-", "\\"]`, cycling at 10 Hz via
  `((elapsed * 10.0) as usize) % 4`.
- `status_line` is the last value passed to `set_status`.
- Long text is truncated with `…` to fit the viewport.

Hidden when `status_line` is empty.

## Pre-warmed subsystems (splash phase, 3.5s–6.5s)

Moved from lazy/frame-0 init to splash-phase so frame 0 of the main
loop is hitch-free:

- **Wallpaper texture** — `wallpaper::generate_from_config` + GPU
  upload. Used to be frame 0 and caused a visible first-paint hitch.
- **Cursor texture** — `cursor::generate_cursor_pixels` + GPU upload.
- **Shader bridge** — `SdlShaderBridge::new` (GL context +
  framebuffer).
- **SDI scene graph** — `apply_layout_scaled` builds the full object
  tree from the skin.
- **Audio backend** — `SdlAudioBackend::init()` opens the device.
- **Glyph atlas** — `prewarm_glyph_cache` rasterizes the printable
  ASCII range + common Unicode extras at six common font sizes.
  Drawn off-screen with alpha=0 so nothing flickers. Removes
  per-glyph first-use hitches on the dashboard's initial paint.
- **Status bar** — primed with real time + power info so the first
  frame shows actual values instead of `--:--` / `--%` placeholders.
- **Radio station registry** — loaded from
  `/etc/radio/stations.toml`.
- **Auto-launch** (`OASIS_APP=…`) — completed before the main loop.

## System probes (`crates/oasis-app/src/sysinfo.rs`)

Zero-dependency `/proc` readers that degrade gracefully on non-Linux
hosts:

| Function               | Source                                   | BIOS line |
|------------------------|------------------------------------------|-----------|
| `total_ram_kb`         | `/proc/meminfo` MemTotal                 | 2         |
| `cpu_core_count`       | `/proc/cpuinfo` processor lines          | 2         |
| `os_release`           | `/proc/sys/kernel/{ostype,osrelease}`    | 1         |
| `cpu_arch`             | `std::env::consts::ARCH`                 | 0         |
| `count_vfs_entries`    | VFS `readdir` walk                       | 3         |
| `total_vfs_bytes`      | VFS walk, sums file sizes                | 3         |

## Skipping

Any `ButtonPress` event (keyboard/controller) during `run_until` sets
`self.skipped = true` and returns immediately. Subsequent `run_until`
calls short-circuit, so the remaining init work continues at full
speed with no animation. `finish()` detects the skipped flag and
fades directly to black without the 0.3s transition.

## Related files

- **Animation:** `crates/oasis-app/src/boot_splash.rs`
- **Orchestration:** `crates/oasis-app/src/main.rs`
  (`splash_wait!`, `splash_set_line!`, `splash_status!` macros)
- **System probes:** `crates/oasis-app/src/sysinfo.rs`
- **Warm-up helpers:** `main::prewarm_glyph_cache`,
  `main::format_thousands`
