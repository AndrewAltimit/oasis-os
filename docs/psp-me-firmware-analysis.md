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

`sceMeCodecWrapper` references four firmware paths in its strings, but on
FW 6.61 (PSP-3001 TA-090v2), only ONE exists on flash0:

| Image | Path | Status | Size |
|-------|------|--------|------|
| **meimg.img** | `flash0:/kd/resource/meimg.img` | **DOES NOT EXIST** | — |
| **me_blimg.img** | `flash0:/kd/resource/me_blimg.img` | **DOES NOT EXIST** | — |
| **me_sdimg.img** | `flash0:/kd/resource/me_sdimg.img` | **DOES NOT EXIST** | — |
| **me_t2img.img** | `flash0:/kd/resource/me_t2img.img` | **EXTRACTED** | 391,792 bytes |

The `me_t2img.img` file is encrypted (first 128 bytes are zeros, followed
by encrypted payload). It requires `sceMesgLed` (the PSP crypto engine) to
decrypt before loading onto the ME.

`flash0:/kd/resource/` contains only 2 files: `me_t2img.img` and `impose.rsc`.

The missing firmware images (meimg, me_blimg, me_sdimg) may have existed on
earlier firmware versions but were consolidated into `me_t2img.img` by FW 6.61.
The `sceMeCodecWrapper` code likely tries all four paths and uses whichever
exists.

## flash0:/kd/ Codec PRX Files (Extracted)

All PRX files on flash0 are `~PSP` encrypted (magic `0x7E505350`).
Extracted from kernel mode via our PRX plugin.

| File | Size | Purpose |
|------|------|---------|
| **me_wrapper.prx** | 7,008 | sceMeCodecWrapper — ME firmware loader + RPC |
| **avcodec.prx** | 11,856 | sceAvcodec — video/audio codec interface |
| **mpeg.prx** | 18,160 | sceMpeg — kernel-side MPEG decoder |
| **mpeg_vsh.prx** | 23,584 | sceMpeg VSH variant |
| **videocodec_260.prx** | 2,864 | sceVideocodec stubs |
| **codec_09g.prx** | 2,992 | Codec support for 09g hardware revision |
| **mpegbase_260.prx** | 4,384 | MPEG-PS demuxer + color space conversion |

### flash0:/kd/ Access Constraints

- **User mode** (`sceIoDopen("flash0:/kd/")`) → `0x8001000d` (ENOENT) — ARK-4 CFW blocks access
- **Kernel mode** (our PRX plugin) → full read access to all files
- `flash0:/` root listing works from user mode (shows kd/, vsh/, font/, data/, dic/, codepage/)
- `flash0:/vsh/resource/` is accessible from user mode (contains .rco UI resources)

## ME Communication Protocol: RPC (Fully Reverse-Engineered)

Disassembly of `sceMeCore::0xFA398D71` (the RPC dispatch function) reveals
the complete protocol:

### Command Buffer: 0xBFC00600

The ME command buffer is at physical address **0xBFC00600** (in the ME's
boot ROM / shared memory region):

```
Offset  Size  Field
0x00    4     Command ID (determines operation)
0x04    4     (padding/unused)
0x08    4     Parameter 1 (codec buffer pointer, etc.)
0x0C    4     Parameter 2
0x10    4     Parameter 3
0x14    4     Parameter 4
0x18    4     Parameter 5
0x1C    4     Parameter 6
0x20    4     Parameter 7
0x24    4     Parameter 8
0x28    4     Return value (written by ME after command completes)
```

### RPC Protocol Sequence

```
1. WaitSema(SceMeRpc, 1, 0)     — acquire exclusive ME access
2. Write cmd ID to 0xBFC00600
3. Write params to 0xBFC00608..0xBFC00624
4. DcacheWritebackInvalidate(0xBFC00600, size)  — flush to physical RAM
5. sceSysregMeResetEnable()      — trigger ME interrupt/wakeup
6. WaitEventFlag(SceMediaEngineRpc, 1, WAIT_AND|CLEAR, timeout=0)
7. Read return value from 0xBFC00628
8. SignalSema(SceMeRpc, 1)       — release ME access
```

### Synchronization Primitives

- **SceMeRpc** — semaphore (initial count 1), stored at module BSS + 0x7544
- **SceMediaEngineRpc** — event flag, stored at module BSS + 0x7548
- **SceMediaEngineAvcPower** — event flag for AVC power state
- **0xBFC00718** — ME power state register (cached by sceMePower functions)

### ME RPC Command Table

Extracted from disassembly of all 47 ME driver functions:

| CMD | Hex | Function | Driver |
|-----|-----|----------|--------|
| 2 | 0x0002 | VideocodecOpen | sceMeVideo |
| 9 | 0x0009 | AudiocodecInit | sceMeAudio |
| 36 | 0x0024 | VideocodecInit | sceMeVideo |
| 37 | 0x0025 | VideocodecScanHeader | sceMeVideo |
| 38 | 0x0026 | VideocodecDecode | sceMeVideo |
| 96 | 0x0060 | AudiocodecInit2 | sceMeAudio |
| 97 | 0x0061 | AudiocodecRelease | sceMeAudio |
| 100 | 0x0064 | AudiocodecDecode | sceMeAudio |
| 102 | 0x0066 | AudiocodecCheckNeedMem | sceMeAudio |
| 103 | 0x0067 | AudiocodecReset | sceMeAudio |
| 105 | 0x0069 | AudiocodecGetInfo | sceMeAudio |
| 106 | 0x006A | MpegbaseCSC (color space convert) | sceMeVideo |
| 115 | 0x0073 | AudiocodecStep | sceMeAudio |
| 130 | 0x0082 | Unknown | — |
| 138 | 0x008A | Unknown | — |
| 146 | 0x0092 | Unknown | — |
| 151 | 0x0097 | Unknown | — |
| 225 | 0x00E1 | VideocodecRelease | sceMeVideo |
| 384 | 0x0180 | ME_AllocMemory | sceMeMemory |
| 385 | 0x0181 | ME_FreeMemory | sceMeMemory |
| 387 | 0x0183 | ME_FreeMem2 | sceMePower |
| 389 | 0x0185 | Unknown | — |

### Version Check Constant

All sceMeVideo functions check `buf[0]` against **0x05100601** before
processing. This is the codec version identifier — the same value we saw
in Ghidra analysis of `avcodec.prx`. If the codec buffer's version field
doesn't match, the function returns -2 (invalid version).

### ME Boot Sequence (MODULE_ENTRY at 0x88224900)

The module entry point performs ME hardware initialization:

```
1. Read SysCtrl register 0xBC100000 — check if ME is already running
2. If not running:
   a. Write 0 to SysCtrl+0x40 (ME reset control)
   b. Write 7 to SysCtrl+0x50 (ME clock enable: bus+ME+AW)
   c. Write -1 to SysCtrl+0x04 (clear interrupt flags)
   d. sync
   e. Invalidate I-cache (16KB, 64-byte lines)
   f. Invalidate D-cache (16KB, 64-byte lines)
   g. Configure COP0 registers (Status, Cause)
   h. Write 1 to 0xBCC00010 (ME power/clock controller)
   i. Poll 0xBCC00010 until bit 0 clears (ME boot complete)
   j. Configure ME memory controller (0xBCC00070, 0xBCC00030, 0xBCC00040)
   k. Jump to module init at 0x882268A8 with SP=0x88400000
```

### Key Hardware Registers

| Register | Purpose |
|----------|---------|
| 0xBC100000 | SysCtrl: ME running status |
| 0xBC100004 | SysCtrl: interrupt flags |
| 0xBC100040 | SysCtrl: ME reset control |
| 0xBC100050 | SysCtrl: ME clock enable |
| 0xBC100070 | SysCtrl: ME power state |
| 0xBCC00010 | ME clock controller: boot trigger |
| 0xBCC00030 | ME memory controller |
| 0xBCC00040 | ME memory controller |
| 0xBCC00070 | ME memory controller |
| 0xBFC00600 | ME command buffer (40 bytes) |
| 0xBFC00628 | ME return value |
| 0xBFC00718 | ME power state cache |

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
