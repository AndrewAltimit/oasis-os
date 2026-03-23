# PSP Media Engine: Runtime Firmware Analysis

## Date: 2026-03-23

## Hardware: PSP-3001 (TA-090v2), ARK-4 CFW, FW 6.61

## Overview

This document records the results of dumping all loaded kernel and user-mode
modules from a PSP during active UMD video playback (Spider-Man 2 movie disc).
The dump was performed by a custom kernel PRX (OASIS Plugin) that hooks
`sceDisplaySetFrameBuf` and runs alongside the video player.

These findings reveal the complete ME (Media Engine) driver architecture on
real PSP hardware, including previously undocumented kernel modules, ME
firmware image paths, NID tables for 6 internal ME driver interfaces, and
the RPC-based communication protocol between the main CPU and Media Engine.

## Key Discovery: ME Driver Architecture

The PSP Media Engine is NOT controlled directly by user-mode code. Instead,
there is a layered driver architecture with kernel-side modules that
communicate with the ME via an RPC (Remote Procedure Call) protocol:

```
User-space (video player process):
  video_main_plugin_module (319KB) — UMD Video player UI + logic
    └─ sceMpegVsh_library (43KB) — VSH variant of sceMpeg API
         ├─ sceVideocodec — video decode syscalls
         ├─ sceAudiocodec — audio decode syscalls
         └─ sceMpegbase — MPEG-PS demuxer + CSC

Kernel-space:
  sceAvcodec_wrapper (19KB) — avcodec interface layer
    ├─ sceAvcodec_driver — export library (sceVideocodec, sceAudiocodec, sceMpegbase)
    ├─ sceMeVideo_driver — ME video decode commands
    ├─ sceMeAudio_driver — ME audio decode commands
    ├─ sceMeCore_driver — ME core lifecycle (boot, halt, reset)
    ├─ sceMeMemory_driver — ME memory allocation
    ├─ sceMePower_driver — ME clock/power management
    └─ sceDmacplus_driver — DMA for bulk data transfer

  sceMeCodecWrapper (11KB) — ME firmware loader + RPC bridge
    ├─ sceMeWrapper_driver — 23-function master interface
    ├─ sceMeVideo_driver — 7 video-specific functions
    ├─ sceMeAudio_driver — 5 audio-specific functions
    ├─ sceMeMemory_driver — 3 memory management functions
    ├─ sceMeCore_driver — 4 core lifecycle functions
    ├─ sceMePower_driver — 5 power management functions
    ├─ sceSysreg_driver — system register access
    ├─ sceDdr_driver — DDR memory controller
    ├─ sceSyscon_driver — system controller (Baryon)
    ├─ sceLFatFs_driver — FAT filesystem (for firmware loading)
    └─ sceWmd_driver — WMD (unknown, possibly DRM/crypto)
```

## ME Firmware Images

`sceMeCodecWrapper` loads the ME firmware from flash0. Four firmware images
were identified from string analysis:

| Image | Path | Purpose (inferred) |
|-------|------|--------------------|
| **meimg.img** | `flash0:/kd/resource/meimg.img` | Main ME firmware (H.264/MPEG-4/AAC decode) |
| **me_blimg.img** | `flash0:/kd/resource/me_blimg.img` | ME bootloader (initial boot code) |
| **me_sdimg.img** | `flash0:/kd/resource/me_sdimg.img` | ME shutdown image (safe halt sequence) |
| **me_t2img.img** | `flash0:/kd/resource/me_t2img.img` | ME type-2 image (variant/update?) |

These are encrypted firmware binaries stored on the PSP's internal flash.
They are loaded by `sceMeCodecWrapper` during codec initialization and
transferred to the ME core's local memory.

## ME Communication Protocol: RPC

The ME uses a Remote Procedure Call (RPC) mechanism, evidenced by:

- Semaphore `SceMeRpc` — synchronization primitive for RPC calls
- Event `SceMediaEngineRpc` — RPC command/response events
- Event `SceMediaEngineRpcWait` — blocking wait for RPC completion
- Event `SceMediaEngineAvcPower` — AVC-specific power state events
- String `"old ME partition"` — references ME memory partition management

The main CPU sends commands to the ME by:
1. Writing command parameters to shared uncached memory
2. Signaling the ME via the RPC event
3. Waiting on `SceMediaEngineRpcWait` for completion
4. Reading results from shared memory

## NID Tables: ME Driver Interfaces

### sceMeWrapper_driver (23 functions)

The master ME driver interface. All other ME driver libraries are subsets
of this one (shared stub addresses confirm they route to the same code).

| Index | NID | Stub Address | Notes |
|-------|-----|-------------|-------|
| 0 | 0x0DEFA6A5 | 0x88225DC0 | Also in sceMePower_driver |
| 1 | 0x1862B784 | 0x88225D78 | Also in sceMePower_driver |
| 2 | 0x21521BE5 | 0x88225A54 | Also in sceMeVideo_driver |
| 3 | 0x24317CD0 | 0x88226970 | Wrapper-only |
| 4 | 0x4D78330C | 0x88224E00 | Also in sceMeVideo_driver |
| 5 | 0x5F6BF6DF | 0x88226A30 | Wrapper-only |
| 6 | 0x635397BB | 0x88226190 | Also in sceMeCore_driver |
| 7 | 0x6AD33F60 | 0x88225354 | Also in sceMeAudio_driver |
| 8 | 0x6D68B223 | 0x88225060 | Also in sceMeVideo_driver |
| 9 | 0x6ED69327 | 0x88225AB8 | Also in sceMeMemory_driver |
| 10 | 0x81956A0B | 0x88225220 | Also in sceMeAudio_driver |
| 11 | 0x8768915D | 0x88224C4C | Also in sceMeVideo_driver |
| 12 | 0x8DD56014 | 0x88224F30 | Also in sceMeVideo_driver |
| 13 | 0x92D3BAA1 | 0x88225A74 | Also in sceMeMemory_driver |
| 14 | 0x984E2608 | 0x88225D4C | Also in sceMePower_driver |
| 15 | 0x9A9E21EE | 0x8822578C | Also in sceMeAudio_driver |
| 16 | 0xB37562AA | 0x88225C40 | Also in sceMePower_driver |
| 17 | 0xC300D466 | 0x882258F8 | Also in sceMeAudio_driver |
| 18 | 0xC441994C | 0x882249D4 | Also in sceMeVideo_driver |
| 19 | 0xC4EDA9F4 | 0x88225A94 | Also in sceMeMemory_driver |
| 20 | 0xE8CD3C75 | 0x88224ADC | Also in sceMeVideo_driver |
| 21 | 0xE9F69ACF | 0x88225D9C | Also in sceMePower_driver |
| 22 | 0xFA398D71 | 0x88226078 | Also in sceMeCore_driver |

### sceMeVideo_driver (7 functions)

Video-specific ME functions (H.264/MPEG-4 decode):

| Index | NID | Stub Address |
|-------|-----|-------------|
| 0 | 0x21521BE5 | 0x88225A54 |
| 1 | 0x4D78330C | 0x88224E00 |
| 2 | 0x6D68B223 | 0x88225060 |
| 3 | 0x8768915D | 0x88224C4C |
| 4 | 0x8DD56014 | 0x88224F30 |
| 5 | 0xC441994C | 0x882249D4 |
| 6 | 0xE8CD3C75 | 0x88224ADC |

### sceMeAudio_driver (5 functions)

Audio-specific ME functions (AAC/ATRAC3+ decode):

| Index | NID | Stub Address |
|-------|-----|-------------|
| 0 | 0x6AD33F60 | 0x88225354 |
| 1 | 0x81956A0B | 0x88225220 |
| 2 | 0x9A9E21EE | 0x8822578C |
| 3 | 0xB57F033A | 0x88225624 |
| 4 | 0xC300D466 | 0x882258F8 |

### sceMeMemory_driver (3 functions)

ME memory management:

| Index | NID | Stub Address |
|-------|-----|-------------|
| 0 | 0x6ED69327 | 0x88225AB8 |
| 1 | 0x92D3BAA1 | 0x88225A74 |
| 2 | 0xC4EDA9F4 | 0x88225A94 |

### sceMeCore_driver (4 functions)

ME core lifecycle (boot, halt, reset):

| Index | NID | Stub Address |
|-------|-----|-------------|
| 0 | 0x5DFF5C50 | 0x8822666C |
| 1 | 0x635397BB | 0x88226190 |
| 2 | 0x905A7500 | 0x88225AD8 |
| 3 | 0xFA398D71 | 0x88226078 |

### sceMePower_driver (5 functions)

ME clock and power management:

| Index | NID | Stub Address |
|-------|-----|-------------|
| 0 | 0x0DEFA6A5 | 0x88225DC0 |
| 1 | 0x1862B784 | 0x88225D78 |
| 2 | 0x984E2608 | 0x88225D4C |
| 3 | 0xB37562AA | 0x88225C40 |
| 4 | 0xE9F69ACF | 0x88225D9C |

## Module Inventory During Video Playback

68 modules were loaded during UMD Video playback. Notable additions
compared to idle (game not running):

### Additional Kernel Modules (vs idle baseline of 32)

| Module | Address | Size | Purpose |
|--------|---------|------|---------|
| sceWM1801_Driver | 0x88178F00 | 4.1KB | WM1801 codec IC driver |
| sceAudio_Driver | 0x8817A000 | 13.6KB | Audio hardware driver |
| sceHP_Remote_Driver | 0x8817EB00 | 13.9KB | Headphone remote |
| sceOpenPSID_Service | 0x88182600 | 15.0KB | Console ID service |
| sceUSB_Driver | 0x88186C00 | 38.9KB | USB subsystem |
| sceWlan_Driver | 0x88190B00 | 103KB | WiFi driver |
| sceWlanFirmVoyager_driver | 0x881BA000 | 97.7KB | WiFi firmware |
| sceRegistry_Service | 0x881D2000 | 28.4KB | Registry (settings) |
| sceMgr_Driver | 0x881E1000 | 49.1KB | Memory manager |
| sceMsAudio_Service | 0x881F0000 | 27.3KB | MS audio service |
| sceChkreg | 0x881F8000 | 2.6KB | Region check |
| **sceMesgLed** | 0x881F9600 | 42.3KB | **Crypto/decrypt engine** |
| sceSemawm | 0x88204300 | 23.0KB | Semaphore WM |
| sceAmctrl_driver | 0x8820B800 | 7.4KB | AM controller |
| scePspNpDrm_Driver | 0x8820EE00 | 17.2KB | DRM |
| sceIoFilemgrDNAS | 0x88214200 | 6.5KB | DNAS file manager |
| sceChnnlsv | 0x88216500 | 6.7KB | Channel service |
| sceUtility_Driver | 0x88219000 | 45.1KB | Utility driver |
| **sceMeCodecWrapper** | 0x88224900 | **11.3KB** | **ME firmware loader + RPC** |
| sceVaudio_driver | 0x8822B300 | 5.2KB | Virtual audio |
| sceImpose_Driver | 0x8822C800 | 41.4KB | System impose |
| **sceAvcodec_wrapper** | 0x88236E00 | **19.0KB** | **avcodec kernel interface** |
| VshCtrl | 0x8823C000 | 31.8KB | VSH controller |
| sceVshBridge_Driver | 0x88245F00 | 27.3KB | VSH bridge |
| **OasisPlugin** | 0x88252600 | 73.1KB | Our kernel PRX |

### User-Mode Modules (loaded by VSH video player)

| Module | Address | Size | Purpose |
|--------|---------|------|---------|
| XmbControl | 0x0880A700 | 36.3KB | XMB menu controller |
| sceKernelLibrary | 0x08813E00 | 3.0KB | Kernel library stubs |
| sceATRAC3plus_Library | 0x08817E00 | 16.2KB | ATRAC3+ audio codec |
| scePaf_Module | 0x0881C400 | 1.6MB | PAF UI framework |
| sceVshCommonGui_Module | 0x0A03EC00 | 29.1KB | VSH common GUI |
| sceVshCommonUtil_Module | 0x0A045F00 | 16.9KB | VSH common utilities |
| vsh_module | 0x0A04A200 | 355.2KB | Main VSH module |
| **sceMpegVsh_library** | 0x0A0A6F00 | **43.6KB** | **VSH sceMpeg library** |
| impose_plugin_module | 0x0A0B1C00 | 7.2KB | Impose (volume/brightness) |
| video_plugin_module | 0x0A0B3900 | 12.6KB | Video plugin interface |
| **video_main_plugin_module** | 0x0A0B8600 | **319KB** | **Full UMD video player** |

## Relationship to Homebrew avcodec.prx Problem

The homebrew `avcodec.prx` (loaded via `sceUtilityLoadModule(AvCodec)`) has
**empty ME submission stubs** because:

1. Homebrew loads `avcodec.prx` from flash0 via `sceUtilityLoadModule`
2. This loads the user-mode `sceAvcodec_driver` library
3. But the kernel-side **`sceMeCodecWrapper`** and **`sceAvcodec_wrapper`**
   are NOT loaded by `sceUtilityLoadModule` — they are only loaded by the
   VSH (XMB) or by UMD game boot sequences
4. Without the kernel-side wrapper, the user-mode stubs have nothing to
   call, so they return immediately (empty stubs)

### The Fix Path

To enable H.264 decode from homebrew, we need to either:

**Option A: Load the kernel ME modules manually**
1. Load `sceMeCodecWrapper` from flash0 (it will load ME firmware images)
2. Load `sceAvcodec_wrapper` from flash0
3. This populates the kernel-side ME driver functions
4. User-mode `sceVideocodec`/`sceMpeg` calls will then work

**Option B: Call ME drivers directly from kernel mode**
1. Use the NID tables above to resolve ME driver functions via
   `sctrlHENFindFunction` (they won't be found unless the modules
   are loaded — see Option A)
2. Call `sceMeCore_driver` to boot the ME
3. Call `sceMeVideo_driver` to submit H.264 decode commands
4. Call `sceMeMemory_driver` to manage decode buffers

**Option C: Load ME firmware manually via psp::me**
1. Extract `meimg.img` from flash0 (may need decryption via `sceMesgLed`)
2. Load it to ME memory via `psp::me::MeExecutor`
3. Implement the RPC protocol based on the `SceMeRpc` semaphore pattern
4. Send H.264 commands directly

## sceAvcodec_wrapper vs Homebrew avcodec.prx

Key difference identified:

- **Homebrew avcodec.prx** (`sceAvcodec_driver` user library): 19KB,
  exports `sceVideocodec`, `sceAudiocodec`, `sceMpegbase` — but the
  ME submission functions are empty stubs (`void { return; }`)

- **Game-time sceAvcodec_wrapper** (kernel module): 19KB, same exports
  but imports from `sceMeVideo_driver`, `sceMeAudio_driver`, etc. —
  these route to `sceMeCodecWrapper` which does the actual ME communication

The user-mode `avcodec.prx` and the kernel-mode `sceAvcodec_wrapper` are
**different modules** that export the same library names (`sceVideocodec`,
`sceAudiocodec`, `sceMpegbase`). The kernel module has the real implementation;
the user module is a stub that delegates via syscalls.

## sceAvcodec_wrapper Import Libraries

From string analysis of the kernel-mode `sceAvcodec_wrapper`:

| Library | Purpose |
|---------|---------|
| sceMeVideo_driver | ME video decode commands |
| sceMeAudio_driver | ME audio decode commands |
| sceMeCore_driver | ME core lifecycle |
| sceMeMemory_driver | ME memory management |
| sceMePower_driver | ME power/clock |
| sceDmacplus_driver | DMA transfers |
| ThreadManForKernel | Threading primitives |
| UtilsForKernel | Utility functions |
| SysMemForKernel | Memory allocation |

Exports:
- `sceVideocodec` (same NIDs as user-mode avcodec.prx)
- `sceAudiocodec` (same NIDs)
- `sceMpegbase` (same NIDs)
- `sceMpegbase_driver` (kernel-only variant)
- `sceJpeg` (JPEG decode, bonus)

## sceMpegVsh_library vs Homebrew sceMpeg

The VSH uses `sceMpegVsh_library` (43KB) instead of the standard
`sceMpeg_library` (32KB) we dumped from homebrew. Key differences:

- Module name: `sceMpegVsh_library` vs `sceMpeg_library`
- Identifier string: `"Lib-PSP libmpeg_vsh"` vs standard `"Lib-PSP mpeg"`
- Same internal structures: `SceMpegAvcResource`, `SceMpegLibmpeg`,
  `SceMpegAvc`, `SceMpegAtrac`, `SceMpegMpegData`, `SceMpegRingBufferPut`
- Imports `sceVideocodec` and `sceAudiocodec` (same as standard version)

## Files

| File | Location | Description |
|------|----------|-------------|
| sceMeCodecWrapper dump | `seplugins/me_dump/50_sceMeCodecWrapper.bin` | 11KB kernel ME RPC bridge |
| sceAvcodec_wrapper dump | `seplugins/me_dump/53_sceAvcodec_wrapper.bin` | 19KB kernel avcodec |
| sceMpegVsh_library dump | `seplugins/me_dump/64_sceMpegVsh_library.bin` | 43KB VSH mpeg library |
| video_plugin dump | `seplugins/me_dump/66_video_plugin_module.bin` | 12KB video plugin |
| video_main_plugin dump | `seplugins/me_dump/67_video_main_plugin_module.bin` | 319KB video player |
| Module list | `seplugins/me_dump/modules.txt` | All 68 loaded modules |
| Codec probes | `seplugins/me_dump/codec_probes.txt` | sctrlHENFindFunction results |
| All 68 module dumps | `seplugins/me_dump/*.bin` | Complete kernel state |

## Next Steps

1. **Extract ME firmware from flash0**: Mount flash0 and copy
   `meimg.img`, `me_blimg.img`, `me_sdimg.img`, `me_t2img.img`
2. **Disassemble sceMeCodecWrapper** with Ghidra: understand the ME boot
   sequence and RPC protocol in detail
3. **Disassemble sceAvcodec_wrapper** with Ghidra: map the 7 sceMeVideo_driver
   NIDs to sceVideocodec API functions (Open, Init, Decode, etc.)
4. **Try loading kernel ME modules from homebrew**: Use our kernel PRX to
   load `sceMeCodecWrapper` and `sceAvcodec_wrapper` at boot time
5. **Decrypt ME firmware images**: May require `sceMesgLed` (42KB crypto
   module, also dumped)
