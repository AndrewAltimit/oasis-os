# PSP Autorun Scripts

The PSP backend has a boot-time script runner gated by the
`autorun-script` cargo feature. When enabled, the EBOOT looks for
`ms0:/PSP/GAME/OASISOS/AUTORUN.txt` at startup. If the file exists, the
runner parses it and dispatches one command per frame from the main
loop. Output (logs and screenshot-request sentinels) lands on the
memstick where a host test harness can pick it up.

The runner replaces ad-hoc xdotool key-injection scripts: a test
becomes a small text file, and the same script runs on real hardware
or in PPSSPP without any input simulation.

## Building

```bash
cd crates/oasis-backend-psp
RUST_PSP_BUILD_STD=1 cargo psp --release --features autorun-script
```

The feature is **off by default** so production EBOOTs don't pay the
parser size cost.

## Grammar

Lines starting with `#` are comments; blank lines are skipped. Each
non-empty line is `<verb> [arg1 [arg2 ...]]`:

| verb | args | effect |
|---|---|---|
| `launch <app_id>` | dashboard app id (`filemgr`, `settings`, `terminal`, `browser`, `radio`, `tvguide`, ...) | clicks the named dashboard icon (cursor-warps to the icon center, then injects Confirm) |
| `press <button>` | `cross\|x\|confirm`, `circle\|o\|cancel`, `square`, `triangle`, `up\|down\|left\|right`, `start`, `select`, `ltrigger\|l`, `rtrigger\|r` | queues a 1-frame button press + release |
| `cursor <x> <y>` | PSP screen coords (480×272) | injects a `CursorMove` event |
| `skin <key>` | `psix`, `classic`, `balatro`, `retro-cga`, `solarized`, `highcontrast`, `altimit` | applies a theme preset and persists to `config.rcfg` |
| `wait <frames>` | u32 | pauses N frames before next command |
| `screenshot <ms0:/path>` | path under `ms0:/PSP/GAME/OASISOS/` | drops a 0-byte sentinel `<path>.req`, then **blocks** until the host removes it. Paths outside the OASISOS directory are rejected at parse time |
| `log <message>` | free text | appends a line to `autorun.log` |
| `exit [code]` | i32 (default 0) | writes `autorun.done` then `sceKernelExitGame` |

The runner deletes the AUTORUN.txt sentinel after parsing — a crashed
script doesn't re-run on next boot.

## Output files

All paths are relative to the PSP memstick root. In PPSSPP, this maps
to `<MemStickDir>/GAME/OASISOS/` (PPSSPP strips the leading `PSP/`).

- `autorun.log` — every dispatched command, errors, and `log` messages.
- `autorun.done` — written when the script reaches `exit`. Body is the
  exit code as ASCII text. Watch for this from the host harness.
- `<path>.req` — screenshot sentinel. The host harness sees the file,
  captures the PPSSPP window via `scrot`, then deletes the sentinel.
  Autorun blocks while the sentinel exists, so the host has time to
  capture before the next command advances the emulator.

## Why screenshots use sentinels (PPSSPP-specific)

On real PSP hardware, dumping VRAM at `0x44000000` produces a valid
framebuffer (see `cmd_server::take_screenshot`). PPSSPP's GU emulation
renders to internal textures and never syncs pixels back to the PSP
RAM mirror, so VRAM reads in the emulator return only stale boot-time
content. The sentinel approach delegates capture to the host (`scrot`
against the PPSSPP X11 window), giving PPSSPP-correct screenshots
without depending on VRAM readback. Real hardware can re-use the same
sentinel mechanism plus a PRX kernel plugin or the existing TCP
`cmd_server` screenshot path.

## Example: Settings theme test

`scripts/test-settings-ppsspp.sh` writes the following AUTORUN.txt:

```text
log boot ok, capturing dashboard
wait 60
screenshot ms0:/PSP/GAME/OASISOS/01-dashboard-initial.bmp

log opening Settings via launch
launch settings
wait 90
screenshot ms0:/PSP/GAME/OASISOS/02-settings-opened.bmp

log navigate down to Retro CGA
press down
wait 6
press down
wait 6
press down
wait 30
screenshot ms0:/PSP/GAME/OASISOS/03-retrocga-highlighted.bmp

log apply theme
press cross
wait 60
screenshot ms0:/PSP/GAME/OASISOS/04-theme-applied.bmp

log close Settings
press circle
wait 60
screenshot ms0:/PSP/GAME/OASISOS/05-dashboard-with-new-theme.bmp

log all done
exit 0
```

Run: `./scripts/test-settings-ppsspp.sh`. Output PNGs land in
`screenshots/settings-test/`.

## Implementation

- `crates/oasis-backend-psp/src/autorun.rs` — runner, parser, I/O.
- `cmd_server::request_skin_change`, `cmd_server::inject_event`,
  `desktop::dashboard_icon_center` — shared with the TCP command server.
- `main.rs` — `let mut autorun = autorun::AutorunRunner::load();` at
  boot, `autorun.tick()` once per frame before `poll_events_inner`.
