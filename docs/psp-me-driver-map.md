# PSP Media Engine Driver Map

## Date: 2026-03-23

## Hardware: PSP-3001 (TA-090v2), ARK-4 CFW, FW 6.61

## Overview

This document is a concise visual reference for the PSP Media Engine driver
architecture, showing the complete chain from user-space video player down to
ME hardware. Based on runtime module dumps during UMD Video playback
(Spider-Man 2 movie disc).

For the full RPC protocol and NID tables, see
[psp-me-rpc-api.md](psp-me-rpc-api.md). For the reverse-engineering
narrative, see [psp-me-firmware-analysis.md](psp-me-firmware-analysis.md).

## Complete Driver Chain

```
User-space (video player process):
  video_main_plugin_module (319KB)      UMD Video player UI + logic
    +-- sceMpegVsh_library (43KB)       VSH variant of sceMpeg API
          |-- sceVideocodec syscalls    H.264 / MPEG-4 video decode
          |-- sceAudiocodec syscalls    AAC / ATRAC3+ audio decode
          +-- sceMpegbase syscalls      MPEG-PS demuxer + CSC

        [syscall boundary -- kernel trap]

Kernel-space:
  sceAvcodec_wrapper (19KB)             Kernel codec interface layer
    Exports: sceVideocodec, sceAudiocodec, sceMpegbase,
             sceMpegbase_driver, sceJpeg
    Imports:
      |-- sceMeVideo_driver             7 video ME functions
      |-- sceMeAudio_driver             5 audio ME functions
      |-- sceMeCore_driver              4 core lifecycle functions
      |-- sceMeMemory_driver            3 ME memory functions
      |-- sceMePower_driver             5 clock/power functions
      |-- sceDmacplus_driver            DMA bulk transfers
      |-- ThreadManForKernel            Threading primitives
      |-- UtilsForKernel                Cache ops, etc.
      +-- SysMemForKernel               Memory allocation

  sceMeCodecWrapper (11KB)              ME firmware loader + RPC bridge
    Exports: sceMeWrapper_driver (23 functions, superset)
             sceMeVideo_driver, sceMeAudio_driver,
             sceMeCore_driver, sceMeMemory_driver,
             sceMePower_driver
    Imports:
      |-- sceSysreg_driver              System register access (ME reset)
      |-- sceDdr_driver                 DDR memory controller
      |-- sceSyscon_driver              System controller (Baryon IC)
      |-- sceLFatFs_driver              FAT filesystem (firmware loading)
      +-- sceWmd_driver                 WMD (possibly DRM/crypto)
    Resources:
      +-- flash0:/kd/resource/me_t2img.img  (392KB, encrypted ME firmware)

        [hardware boundary -- RPC over shared memory]

  Media Engine (ME) coprocessor:
    Command buffer at 0xBFC00600 (40 bytes)
    Synchronization: SceMeRpc semaphore + SceMediaEngineRpc event flag
    Firmware: decrypted me_t2img.img loaded at boot
```

## Module Inventory (68 modules during UMD Video playback)

### Base Kernel Modules (32, always loaded)

These are present regardless of whether video is playing:

| # | Module | Address | Size |
|---|--------|---------|------|
| 1 | sceSystemMemoryManager | 0x88000000 | 42.7KB |
| 2 | sceInit | 0x88015000 | 6.7KB |
| 3 | sceLoaderCore | 0x88017F00 | 42.6KB |
| 4 | sceThreadManager | 0x88025900 | 75.1KB |
| 5 | sceInterruptManager | 0x88038700 | 9.1KB |
| 6 | sceGE_Manager | 0x8803B000 | 13.1KB |
| 7 | sceSuspendForKernel | 0x8803F000 | 5.3KB |
| 8 | sceDisplay_Service | 0x88040800 | 14.1KB |
| 9 | sceController_Service | 0x88044A00 | 7.5KB |
| 10 | sceDmacManForKernel | 0x88047700 | 4.7KB |
| 11 | sceDmacplusForKernel | 0x88048D00 | 5.2KB |
| 12 | sceSYSCON_Driver | 0x8804A400 | 15.9KB |
| 13 | sceSysreg_Driver | 0x8804F100 | 9.6KB |
| 14 | sceIOFileManager | 0x88052700 | 34.3KB |
| 15 | sceStdio_Service | 0x8805C800 | 6.4KB |
| 16 | sceCodecEngine | 0x8805E600 | 7.6KB |
| 17 | sceClockgen_Driver | 0x88060C00 | 2.2KB |
| 18 | sceGPIO_Driver | 0x88061700 | 4.0KB |
| 19 | scePower_Service | 0x88062800 | 27.2KB |
| 20 | sceLowIO_Driver | 0x8806A100 | 12.3KB |
| 21 | sceNandFlash_Driver | 0x8806D700 | 12.8KB |
| 22 | sceMSstor_Driver | 0x88071300 | 7.3KB |
| 23 | sceLflashFatfmt | 0x88073B00 | 5.3KB |
| 24 | sceMediaSync | 0x88075300 | 7.3KB |
| 25 | sceMesgd_driver | 0x88077600 | 17.5KB |
| 26 | sceDNAS | 0x8807C900 | 1.3KB |
| 27 | sceModuleManager | 0x8807D700 | 16.4KB |
| 28 | sceExceptionHandler | 0x88081D00 | 3.9KB |
| 29 | sceLibUpdateDL | 0x88082E00 | 2.3KB |
| 30 | sceSYSTimer_Driver | 0x88083700 | 1.7KB |
| 31 | memlmd_02g | 0x88084C00 | 5.5KB |
| 32 | ARK Core | 0x88FC0000 | 68.0KB |

### Video/Codec Kernel Modules (loaded for video playback)

| Module | Address | Size | Purpose |
|--------|---------|------|---------|
| **sceMeCodecWrapper** | 0x88224900 | 11.3KB | ME firmware loader + RPC bridge |
| **sceAvcodec_wrapper** | 0x88236E00 | 19.0KB | Kernel codec interface (real ME stubs) |
| sceMesgLed | 0x881F9600 | 42.3KB | Crypto/decrypt engine (ME firmware decryption) |
| sceCodecEngine | 0x8805E600 | 7.6KB | Codec engine management |

### VSH / UI Kernel Modules

| Module | Address | Size | Purpose |
|--------|---------|------|---------|
| VshCtrl | 0x8823C000 | 31.8KB | VSH controller |
| sceVshBridge_Driver | 0x88245F00 | 27.3KB | VSH bridge |
| sceImpose_Driver | 0x8822C800 | 41.4KB | System impose (volume/brightness overlay) |
| sceUtility_Driver | 0x88219000 | 45.1KB | Utility driver (module loading, dialogs) |
| sceRegistry_Service | 0x881D2000 | 28.4KB | Registry (system settings) |

### User-Space Video Player Modules

| Module | Address | Size | Purpose |
|--------|---------|------|---------|
| **video_main_plugin_module** | 0x0A0B8600 | 319KB | Full UMD video player (UI + logic) |
| **sceMpegVsh_library** | 0x0A0A6F00 | 43.6KB | VSH sceMpeg library |
| video_plugin_module | 0x0A0B3900 | 12.6KB | Video plugin interface |
| impose_plugin_module | 0x0A0B1C00 | 7.2KB | Impose plugin (overlay controls) |
| vsh_module | 0x0A04A200 | 355.2KB | Main VSH module (XMB) |
| scePaf_Module | 0x0881C400 | 1.6MB | PAF UI framework |
| sceVshCommonGui_Module | 0x0A03EC00 | 29.1KB | VSH common GUI |
| sceVshCommonUtil_Module | 0x0A045F00 | 16.9KB | VSH common utilities |
| XmbControl | 0x0880A700 | 36.3KB | XMB menu controller |
| sceKernelLibrary | 0x08813E00 | 3.0KB | Kernel library stubs |
| sceATRAC3plus_Library | 0x08817E00 | 16.2KB | ATRAC3+ audio codec |

### Other Kernel Modules (loaded for video session)

| Module | Address | Size | Purpose |
|--------|---------|------|---------|
| sceAudio_Driver | 0x8817A000 | 13.6KB | Audio hardware driver |
| sceVaudio_driver | 0x8822B300 | 5.2KB | Virtual audio |
| sceWM1801_Driver | 0x88178F00 | 4.1KB | WM1801 codec IC driver |
| sceHP_Remote_Driver | 0x8817EB00 | 13.9KB | Headphone remote |
| sceUSB_Driver | 0x88186C00 | 38.9KB | USB subsystem |
| sceWlan_Driver | 0x88190B00 | 103KB | WiFi driver |
| sceWlanFirmVoyager_driver | 0x881BA000 | 97.7KB | WiFi firmware |
| sceMgr_Driver | 0x881E1000 | 49.1KB | Memory manager |
| sceMsAudio_Service | 0x881F0000 | 27.3KB | MS audio service |
| sceChkreg | 0x881F8000 | 2.6KB | Region check |
| sceSemawm | 0x88204300 | 23.0KB | Semaphore WM |
| sceAmctrl_driver | 0x8820B800 | 7.4KB | AM controller |
| scePspNpDrm_Driver | 0x8820EE00 | 17.2KB | DRM |
| sceIoFilemgrDNAS | 0x88214200 | 6.5KB | DNAS file manager |
| sceChnnlsv | 0x88216500 | 6.7KB | Channel service |
| sceOpenPSID_Service | 0x88182600 | 15.0KB | Console ID service |
| **OasisPlugin** | 0x88252600 | 73.1KB | Our kernel PRX (performing the dump) |

## Key Finding: Why Homebrew H.264 Fails

### Root Cause

When homebrew calls `sceUtilityLoadModule(PSP_MODULE_AV_AVCODEC)`, the system
loads the **user-mode** `avcodec.prx` from `flash0:/kd/`. This module exports
the `sceVideocodec`, `sceAudiocodec`, and `sceMpegbase` library interfaces,
but its ME submission functions are **empty stubs** that return immediately
without doing anything.

The **real** implementations live in two kernel modules:

1. **sceMeCodecWrapper** (11KB) -- loads ME firmware, implements RPC protocol
2. **sceAvcodec_wrapper** (19KB) -- imports from sceMeCodecWrapper's driver
   libraries, exports the same `sceVideocodec`/`sceAudiocodec`/`sceMpegbase`
   interfaces with actual ME communication

These kernel modules are **only** loaded by:
- The VSH (XMB) when launching a UMD Video
- The game boot sequence for UMD games that use MPEG playback
- NOT by `sceUtilityLoadModule` from homebrew context

### The Two avcodec Modules

| Property | User-mode avcodec.prx | Kernel-mode sceAvcodec_wrapper |
|----------|----------------------|-------------------------------|
| Size | 11,856 bytes | 19,000 bytes |
| Loaded by | sceUtilityLoadModule | VSH / game boot |
| Exports | sceVideocodec, sceAudiocodec, sceMpegbase | Same + sceMpegbase_driver, sceJpeg |
| ME imports | None (empty stubs) | sceMeVideo/Audio/Core/Memory/Power_driver |
| H.264 decode | Returns 0 (no-op) | Submits RPC to ME via command buffer |

### Consequence

Any homebrew that calls `sceVideocodecInit()` / `sceVideocodecDecode()` after
`sceUtilityLoadModule(AvCodec)` gets silent success (return 0) but no actual
decode happens. The output buffer is never written to. This is the root cause
of the `0x806201fe` errors and blank frames observed in homebrew H.264
attempts.

## Verified Working Paths

Tested from the OASIS Plugin kernel PRX during Spider-Man 2 UMD Video
playback:

| Test | Method | Result |
|------|--------|--------|
| sceAudiocodecCheckNeedMem | Through kernel sceAvcodec_wrapper | SUCCESS (0x0) |
| sceMeVideo_driver direct call | Bypass sceAvcodec_wrapper, call ME driver NID directly | ME responds -1 (parameter error, NOT empty stub) |
| RPC command 0x82 | Via sceMeCore_driver NID 0x5DFF5C50 | ME responds -3 (unknown command?) |
| sctrlHENFindFunction for all 6 ME libraries | NID resolution during video playback | All 47 NIDs resolved successfully |

The -1 return from direct sceMeVideo_driver calls confirms the ME is alive
and processing commands. The error is a parameter validation failure (likely
missing or incorrectly formatted codec buffer), not an empty stub.

## Access Constraints

### flash0:/kd/ Access

- **User mode**: `sceIoDopen("flash0:/kd/")` returns `0x8001000d` (ENOENT).
  ARK-4 CFW blocks directory listing from user mode.
- **Kernel mode**: Full read access to all files via our PRX plugin.
- `flash0:/` root listing works from user mode (shows kd/, vsh/, font/,
  data/, dic/, codepage/).
- `flash0:/vsh/resource/` is accessible from user mode (contains .rco UI
  resources).

### Module Loading

- `sceKernelLoadModule` during game context returns **EXCLUSIVE_LOAD**
  (`0x80020149`). Cannot load additional kernel modules while a game is
  running.
- `sceKernelGetModuleIdList` second parameter is **byte size** of the output
  array, NOT element count. Passing element count causes buffer overrun.

### Kernel PRX Restrictions

- Display hook callback (`sceDisplaySetFrameBuf` hook) runs in interrupt
  context. No syscalls work (sceCtrlPeekBufferPositive, file I/O, etc.).
  Must use a separate kernel thread for anything beyond register reads and
  framebuffer writes.
- ME driver functions require kernel mode. User-mode homebrew cannot call
  sceMeVideo_driver NIDs directly even if resolved.
