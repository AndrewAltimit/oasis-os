# PSP Overlay Plugin Guide

> **Last reviewed:** 2026-10-30. Architecture, build flow, memory budget and cache handling checked against the source.

The OASIS overlay plugin is a kernel-mode PRX that stays resident in memory alongside PSP games, providing an in-game overlay UI and background MP3 playback.

## Architecture

OASIS uses a two-binary design on PSP:

| Binary | Format | Purpose |
|--------|--------|---------|
| `oasis-backend-psp` | EBOOT.PBP | Full shell (dashboard, terminal, browser, apps) |
| `oasis-plugin-psp` | PRX | Lightweight overlay + background music |

The PRX is loaded by custom firmware (ARK-4, PRO, ME) at boot time via `PLUGINS.TXT` and stays resident when games launch. It hooks `sceDisplaySetFrameBuf` to draw on top of the game's framebuffer.

## Installation

### From OASIS Terminal

The easiest way to install is from the OASIS terminal:

```
plugin install
```

This copies the PRX to `ms0:/seplugins/oasis_plugin.prx`, adds it to `PLUGINS.TXT`, and creates a default `oasis.ini` configuration file.

To remove:
```
plugin remove
```

To check status:
```
plugin status
```

### Manual Installation

1. Copy `oasis_plugin.prx` to `ms0:/seplugins/`
2. Add this line to `ms0:/seplugins/PLUGINS.TXT`:
   ```
   game, ms0:/seplugins/oasis_plugin.prx, on
   ```
3. Reboot the PSP

## Building

```bash
cd crates/oasis-plugin-psp
RUST_PSP_BUILD_STD=1 cargo psp --release
# Strip it: keep only the loaded sections + relocations (prxmin ships with cargo-psp)
prxmin target/mipsel-sony-psp-std/release/oasis-plugin-psp.prx oasis_plugin.prx
```

`cargo psp` writes `target/mipsel-sony-psp-std/release/oasis-plugin-psp.prx`,
which still carries the MIPS `.pdr` procedure descriptors, the symbol table
and its strings (~135 KB the module loader never reads). `prxmin` drops
them; the module it loads is byte-for-byte the same. CI does the same and
ships the stripped file as `oasis-overlay.prx`; install it under the name
above (`oasis_plugin.prx`), which is what `plugin install` / `status` /
`remove` look for.

## Controls

| Button | Action |
|--------|--------|
| NOTE (default) | Toggle overlay menu |
| D-pad Up/Down | Navigate menu items |
| Cross (X) | Select menu item |

The trigger button can be changed to SCREEN via configuration.

### Menu Items

| Item | Description |
|------|-------------|
| Play / Pause | Toggle music playback |
| Next Track | Skip to next MP3 |
| Prev Track | Go to previous MP3 |
| Volume Up | Increase volume |
| Volume Down | Decrease volume |
| CPU Clock | Cycle between 333/266/222 MHz |
| Hide Overlay | Close the menu |

## Configuration

The plugin reads `ms0:/seplugins/oasis.ini` at startup:

```ini
# Overlay trigger button: note or screen
trigger = note

# Music directory path (a trailing / is added if missing)
music_dir = ms0:/MUSIC/

# Overlay background opacity (0-255)
opacity = 180

# Auto-start music on game launch
autoplay = false
```

### Configuration Options

| Key | Values | Default | Description |
|-----|--------|---------|-------------|
| `trigger` | `note`, `screen` | `note` | Button to toggle the overlay |
| `music_dir` | path | `ms0:/MUSIC/` | Directory to scan for MP3 files |
| `opacity` | 0-255 | 180 | Overlay background transparency |
| `autoplay` | `true`/`false` | `false` | Start music automatically on plugin load |

## Music Playback

Place MP3 files in `ms0:/MUSIC/` (or the configured `music_dir`). The plugin scans the directory at startup and builds a playlist (up to 32 tracks).

Playback uses the Media Engine's MP3 decoding, through `sceAudiocodec` when the plugin can resolve it (from the game's own import stubs or by NID) and `sceMp3*` otherwise, with streaming file I/O -- MP3 data is fed to the decoder in chunks rather than loaded entirely into memory. The buffers the ME reads and writes start on 64-byte boundaries.

Audio output uses one of the PSP's 8 hardware audio channels at the configured volume level.

## Overlay Display

When active, the overlay shows:

- **Status bar**: Battery percentage, CPU clock speed, current time
- **Now playing**: Current track name (if music is active)
- **Menu**: Navigable list of actions with cursor highlighting

The overlay renders directly to the game's framebuffer using alpha-blended rectangles and an 8x8 bitmap font (the plugin's own CGA-style copy in `font.rs`, so the PRX stays self-contained). All rendering happens in the `sceDisplaySetFrameBuf` hook -- the game renders normally first, then the overlay is drawn on top. It only draws on 32-bit (`PSM_8888`) framebuffers with a stride of 480 to 2048 pixels.

## Technical Details

### Memory Budget

Measured at plugin v1.4.0 (`opt-level = "z"`, fat LTO, one codegen unit); the
stripped PRX was 156,546 B at the 2026-10-30 review:

| | Size |
|---|---|
| `.text` (code) | ~82 KB, of which ~15 KB is `libunwind` (the `psp` crate links it for `panic = "unwind"`) |
| `.rodata`, `.eh_frame`, stubs, `.data` | ~30 KB |
| Relocations (needed: the loader relocates the PRX) | ~40 KB |
| **Stripped PRX file** (`prxmin`) | **~153 KB** (156,426 B) |
| `.bss` (zeroed at load, not in the file) | ~101 KB: 64 KB MP3 stream buffer, 18 KB PCM, playlist, network buffers |
| **Resident footprint** (code + data + bss) | **~212 KB** |

The original "<64 KB" target dates from the first version of the plugin
(overlay + stub audio, ~1.5k lines); MP3/AAC playback, internet radio, PIP
video, the ME diagnostics and the devloop server have since grown it to
~10k lines (the ME dump diagnostics alone are ~11 KB of code, the devloop
server ~3 KB). `main-ci.yml` fails if the stripped PRX exceeds 192 KiB
(`PRX_SIZE_BUDGET`). PIP video buffers (~113 KB) are allocated on demand
from user partition 2, not counted above.

`panic = "unwind"` is kept on purpose: `panic = "abort"` was measured to
save only ~10 KB of the stripped PRX (`.eh_frame`, landing pads;
`libunwind` stays linked either way), while unwind lets `module_start`'s
`catch_unwind` catch a panic in `psp_main`, and a panic on one of the raw
kernel threads ends in the `psp` crate's print-and-exit-thread path rather
than an abort trap under a running game.

No heap allocator is used for the core overlay logic -- all state is in static buffers and atomics.

### Thread Model

- **Main thread**: Plugin entry point, installs hooks, then sleeps
- **Display hook**: Runs in the game's display thread context via syscall hook. Polls controller, updates state machine, blits overlay
- **Audio thread**: Dedicated kernel thread for MP3 decode + audio output loop

### Cache Coherency

- Overlay pixels are written through the framebuffer's uncached mirror
  (`addr | 0x4000_0000`), so they reach RAM before the display scans the
  buffer out and no data-cache flush is needed
- After installing an inline hook the data cache is written back and the
  instruction cache invalidated (`sceKernelDcacheWritebackAll` +
  `sceKernelIcacheInvalidateAll`) so the CPU fetches the patched code

### CFW Compatibility

The plugin uses `sctrlHENFindFunction` and `sctrlHENPatchSyscall` from the SystemCtrlForKernel library, which is provided by:
- ARK-4
- PRO CFW
- ME/LME CFW

These APIs are the standard mechanism for kernel-mode syscall hooking on custom firmware.

## Troubleshooting

| Issue | Solution |
|-------|----------|
| Overlay doesn't appear | Verify PRX is in `ms0:/seplugins/` and listed in `PLUGINS.TXT` with `, on` |
| No audio | Check that MP3 files are in the configured `music_dir` |
| Game crashes on boot | Some games conflict with overlay hooks -- try disabling the plugin for that game |
| NOTE button doesn't work | Ensure you're using a CFW that exposes kernel-only buttons |
