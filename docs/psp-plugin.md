# PSP Overlay Plugin Guide

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
RUST_PSP_BUILD_STD=1 cargo +nightly psp --release
```

Output: `target/mipsel-sony-psp-std/release/oasis_plugin.prx`

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

# Music directory path
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

Playback uses the PSP's hardware MP3 decoder (`sceMp3*` APIs) with streaming file I/O -- MP3 data is fed to the decoder in chunks rather than loaded entirely into memory.

Audio output uses one of the PSP's 8 hardware audio channels at the configured volume level.

## Overlay Display

When active, the overlay shows:

- **Status bar**: Battery percentage, CPU clock speed, current time
- **Now playing**: Current track name (if music is active)
- **Menu**: Navigable list of actions with cursor highlighting

The overlay renders directly to the game's framebuffer using alpha-blended rectangles and an 8x8 bitmap font. All rendering happens in the `sceDisplaySetFrameBuf` hook -- the game renders normally first, then the overlay is drawn on top.

## Technical Details

### Memory Budget

The PRX targets <64KB total binary size:
- Code: ~32KB (optimized with `opt-level = "z"` + LTO)
- Static data: ~32KB (font glyphs, playlist, decode buffers)

No heap allocator is used for the core overlay logic -- all state is in static buffers and atomics.

### Thread Model

- **Main thread**: Plugin entry point, installs hooks, then sleeps
- **Display hook**: Runs in the game's display thread context via syscall hook. Polls controller, updates state machine, blits overlay
- **Audio thread**: Dedicated kernel thread for MP3 decode + audio output loop

### Cache Coherency

- Data cache is flushed after framebuffer writes (`sceKernelDcacheWritebackRange`)
- Instruction cache is cleared after hook installation (`sceKernelIcacheClearAll`)

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
