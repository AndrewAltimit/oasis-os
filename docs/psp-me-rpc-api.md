# PSP Media Engine RPC API Reference

## Date: 2026-03-23

## Hardware: PSP-3001 (TA-090v2), ARK-4 CFW, FW 6.61

## Overview

This document is a complete technical API reference for the PSP Media Engine
RPC protocol, extracted from runtime firmware analysis during UMD Video
playback (Spider-Man 2 movie disc). It covers the command buffer layout,
protocol sequence, all known RPC commands, ME driver NID tables, hardware
registers, boot sequence, codec buffer versioning, firmware files, and module
names for runtime resolution.

For the full reverse-engineering narrative and module inventory, see
[psp-me-firmware-analysis.md](psp-me-firmware-analysis.md).

## Command Buffer Layout

The ME command buffer resides at physical address **0xBFC00600**, in the ME's
shared memory region. It is a 40-byte (0x28 + 4) structure:

```
Offset  Size  Type    Field
------  ----  ------  -----
0x00    4     u32     cmd_id          — RPC command identifier
0x04    4     u32     (padding)       — unused, always 0
0x08    4     u32     params[0]       — parameter 1 (e.g. codec type)
0x0C    4     u32     params[1]       — parameter 2 (e.g. codec_buf_ptr)
0x10    4     u32     params[2]       — parameter 3
0x14    4     u32     params[3]       — parameter 4
0x18    4     u32     params[4]       — parameter 5
0x1C    4     u32     params[5]       — parameter 6
0x20    4     u32     params[6]       — parameter 7
0x24    4     u32     params[7]       — parameter 8
0x28    4     u32     return_value    — written by ME after command completes
```

Total structure size: 44 bytes (0x2C). The main CPU writes `cmd_id` through
`params[7]`, flushes cache, triggers the ME, then reads `return_value`.

## RPC Protocol Sequence

Every ME driver function follows this exact sequence, extracted from
disassembly of `sceMeCore::0xFA398D71`:

```
1. WaitSema(SceMeRpc, 1, NULL)
     Acquire exclusive ME access. SceMeRpc is a binary semaphore
     (initial count 1) stored at sceMeCodecWrapper BSS + 0x7544.

2. Write command buffer
     Store cmd_id at 0xBFC00600, parameters at 0xBFC00608..0xBFC00624.

3. DcacheWritebackInvalidateRange(0xBFC00600, 0x30)
     Flush the command buffer from D-cache to physical RAM so the ME
     can read it. The ME does not participate in the main CPU's cache
     coherency domain.

4. sceSysregMeResetEnable()
     Trigger ME interrupt/wakeup. This writes to hardware register
     0xBC100040 to signal the ME that a command is pending.

5. WaitEventFlag(SceMediaEngineRpc, 1, WAIT_AND, &out, NULL)
     Block until the ME sets bit 0 of the SceMediaEngineRpc event flag.
     The ME firmware sets this flag after command completion. Event flag
     stored at sceMeCodecWrapper BSS + 0x7548.

6. Read return value from 0xBFC00628
     The ME writes its result to offset 0x28 in the command buffer.
     Common values: 0 (success), -1 (parameter error), -2 (version
     mismatch), -3 (unknown error).

7. SignalSema(SceMeRpc, 1)
     Release ME access so other threads can submit commands.
```

### Synchronization Primitives

| Name | Type | Location | Purpose |
|------|------|----------|---------|
| SceMeRpc | Semaphore (count=1) | BSS + 0x7544 | Exclusive ME command access |
| SceMediaEngineRpc | EventFlag | BSS + 0x7548 | ME command completion signal |
| SceMediaEngineAvcPower | EventFlag | (internal) | AVC power state tracking |

## Complete RPC Command Table

Extracted from disassembly of all 47 ME driver functions across 6 driver
libraries:

### Video Commands (sceMeVideo_driver)

| CMD | Hex | Function | Parameters | Notes |
|-----|-----|----------|------------|-------|
| 2 | 0x0002 | VideocodecOpen | type, codec_buf_ptr | Opens codec instance; type selects H.264/MPEG-4 |
| 36 | 0x0024 | VideocodecInit | (via codec_buf) | Initialize opened codec |
| 37 | 0x0025 | VideocodecScanHeader | (via codec_buf) | Parse NAL/VOL headers |
| 38 | 0x0026 | VideocodecDecode | (via codec_buf) | Decode one frame |
| 106 | 0x006A | MpegbaseCSC | (via codec_buf) | YCbCr to RGB color space conversion |
| 225 | 0x00E1 | VideocodecRelease | (via codec_buf) | Close codec, free ME resources |

### Audio Commands (sceMeAudio_driver)

| CMD | Hex | Function | Parameters | Notes |
|-----|-----|----------|------------|-------|
| 9 | 0x0009 | AudiocodecInit | (via codec_buf) | Initialize audio codec (AAC/ATRAC3+/MP3) |
| 96 | 0x0060 | AudiocodecInit2 | (via codec_buf) | Secondary init path |
| 97 | 0x0061 | AudiocodecRelease | (via codec_buf) | Close audio codec |
| 100 | 0x0064 | AudiocodecDecode | (via codec_buf) | Decode one audio frame |
| 102 | 0x0066 | AudiocodecCheckNeedMem | (via codec_buf) | Query required buffer size |
| 103 | 0x0067 | AudiocodecReset | (via codec_buf) | Reset codec state (seek/flush) |
| 105 | 0x0069 | AudiocodecGetInfo | (via codec_buf) | Query codec parameters |
| 115 | 0x0073 | AudiocodecStep | (via codec_buf) | Step-decode (partial frame) |

### Memory Commands (sceMeMemory_driver)

| CMD | Hex | Function | Parameters | Notes |
|-----|-----|----------|------------|-------|
| 384 | 0x0180 | ME_AllocMemory | size, alignment(?) | Allocate ME-side memory |
| 385 | 0x0181 | ME_FreeMemory | ptr | Free ME-side memory |
| 387 | 0x0183 | ME_FreeMem2 | ptr | Free variant (routed via sceMePower) |

### Unknown Commands

| CMD | Hex | Driver | Notes |
|-----|-----|--------|-------|
| 130 | 0x0082 | sceMeCore_driver | Tested: ME responds with -3 |
| 138 | 0x008A | — | Purpose unknown |
| 146 | 0x0092 | — | Purpose unknown |
| 151 | 0x0097 | — | Purpose unknown |
| 389 | 0x0185 | — | Purpose unknown |

## ME Driver NID Tables

All NIDs resolved at runtime via `sctrlHENFindFunction()` during Spider-Man 2
UMD Video playback. Stub addresses are from that specific boot session and
may vary between boots due to ASLR-like kernel module loading.

### sceMeWrapper_driver (23 functions)

The master ME driver interface exported by `sceMeCodecWrapper`. All other ME
driver libraries are subsets of this one -- shared stub addresses confirm
they route to the same implementation code.

| Index | NID | Stub Address | Also In |
|-------|-----|-------------|---------|
| 0 | 0x0DEFA6A5 | 0x88225DC0 | sceMePower_driver |
| 1 | 0x1862B784 | 0x88225D78 | sceMePower_driver |
| 2 | 0x21521BE5 | 0x88225A54 | sceMeVideo_driver |
| 3 | 0x24317CD0 | 0x88226970 | (wrapper-only) |
| 4 | 0x4D78330C | 0x88224E00 | sceMeVideo_driver |
| 5 | 0x5F6BF6DF | 0x88226A30 | (wrapper-only) |
| 6 | 0x635397BB | 0x88226190 | sceMeCore_driver |
| 7 | 0x6AD33F60 | 0x88225354 | sceMeAudio_driver |
| 8 | 0x6D68B223 | 0x88225060 | sceMeVideo_driver |
| 9 | 0x6ED69327 | 0x88225AB8 | sceMeMemory_driver |
| 10 | 0x81956A0B | 0x88225220 | sceMeAudio_driver |
| 11 | 0x8768915D | 0x88224C4C | sceMeVideo_driver |
| 12 | 0x8DD56014 | 0x88224F30 | sceMeVideo_driver |
| 13 | 0x92D3BAA1 | 0x88225A74 | sceMeMemory_driver |
| 14 | 0x984E2608 | 0x88225D4C | sceMePower_driver |
| 15 | 0x9A9E21EE | 0x8822578C | sceMeAudio_driver |
| 16 | 0xB37562AA | 0x88225C40 | sceMePower_driver |
| 17 | 0xC300D466 | 0x882258F8 | sceMeAudio_driver |
| 18 | 0xC441994C | 0x882249D4 | sceMeVideo_driver |
| 19 | 0xC4EDA9F4 | 0x88225A94 | sceMeMemory_driver |
| 20 | 0xE8CD3C75 | 0x88224ADC | sceMeVideo_driver |
| 21 | 0xE9F69ACF | 0x88225D9C | sceMePower_driver |
| 22 | 0xFA398D71 | 0x88226078 | sceMeCore_driver |

### sceMeVideo_driver (7 functions)

Video-specific ME functions (H.264 / MPEG-4 decode):

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

Audio-specific ME functions (AAC / ATRAC3+ decode):

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

## ME Hardware Registers

| Register | Address | Purpose |
|----------|---------|---------|
| SysCtrl ME status | 0xBC100000 | ME running state (read to check if ME is active) |
| SysCtrl interrupt flags | 0xBC100004 | Write -1 to clear all interrupt flags |
| ME reset control | 0xBC100040 | Write 0 to reset; sceSysregMeResetEnable() target |
| ME clock enable | 0xBC100050 | Write 7 to enable bus + ME + AW clocks |
| ME power state | 0xBC100070 | ME power management register |
| ME boot trigger | 0xBCC00010 | Write 1 to start ME boot; poll until bit 0 clears |
| ME memory controller | 0xBCC00030 | ME memory configuration |
| ME memory controller | 0xBCC00040 | ME memory configuration |
| ME memory controller | 0xBCC00070 | ME memory configuration |
| ME command buffer | 0xBFC00600 | 40-byte RPC command structure (see layout above) |
| ME return value | 0xBFC00628 | Result of last ME command (offset 0x28 in buffer) |
| ME power state cache | 0xBFC00718 | Cached power state used by sceMePower functions |

## ME Boot Sequence

Register writes from `sceMeCodecWrapper` MODULE_ENTRY at 0x88224900:

```
 1. Read SysCtrl register 0xBC100000
      Check if ME is already running. If yes, skip boot.

 2. Write 0 to 0xBC100040 (ME reset control)
      Assert ME reset.

 3. Write 7 to 0xBC100050 (ME clock enable)
      Enable bus clock + ME clock + AW clock (bits 0+1+2).

 4. Write -1 (0xFFFFFFFF) to 0xBC100004
      Clear all pending interrupt flags.

 5. sync
      MIPS sync instruction -- memory barrier.

 6. Invalidate I-cache (16KB, 64-byte lines)
 7. Invalidate D-cache (16KB, 64-byte lines)
      Ensure no stale cached data from previous ME session.

 8. Configure COP0 registers (Status, Cause)
      Set up exception handling for ME context.

 9. Write 1 to 0xBCC00010 (ME boot trigger)
      Start the ME boot process.

10. Poll 0xBCC00010 until bit 0 clears
      ME boot complete when the trigger bit self-clears.

11. Configure ME memory controller
      Write to 0xBCC00070, 0xBCC00030, 0xBCC00040.

12. Jump to module init at 0x882268A8 with SP=0x88400000
      ME firmware initialization entry point.
```

## Codec Buffer Version

All `sceMeVideo_driver` functions check `buf[0]` against the magic constant
**0x05100601** before processing any command. This is the codec version
identifier -- the same value found in Ghidra analysis of `avcodec.prx`.

If the codec buffer's version field does not match, the function returns
**-2** (invalid version) without submitting an RPC command to the ME.

This means any homebrew code that constructs a codec buffer manually must
set `buf[0] = 0x05100601` or the kernel driver will reject it before the
ME ever sees the request.

## flash0 Firmware Files

All PRX files on flash0 are `~PSP` encrypted (magic `0x7E505350`). Extracted
from kernel mode via the OASIS Plugin PRX.

| File | Path | Size | Purpose |
|------|------|------|---------|
| me_t2img.img | flash0:/kd/resource/me_t2img.img | 391,792 | ME firmware image (encrypted) |
| me_wrapper.prx | flash0:/kd/me_wrapper.prx | 7,008 | sceMeCodecWrapper -- ME RPC bridge |
| avcodec.prx | flash0:/kd/avcodec.prx | 11,856 | sceAvcodec -- codec interface |
| mpeg.prx | flash0:/kd/mpeg.prx | 18,160 | sceMpeg -- kernel MPEG decoder |
| mpeg_vsh.prx | flash0:/kd/mpeg_vsh.prx | 23,584 | sceMpeg VSH variant |
| videocodec_260.prx | flash0:/kd/videocodec_260.prx | 2,864 | sceVideocodec stubs |
| codec_09g.prx | flash0:/kd/codec_09g.prx | 2,992 | Codec support for 09g HW revision |
| mpegbase_260.prx | flash0:/kd/mpegbase_260.prx | 4,384 | MPEG-PS demuxer + CSC |

The `me_t2img.img` firmware requires decryption via `sceMesgLed` (42KB crypto
module) before loading onto the ME. The first 128 bytes are zeros, followed
by the encrypted payload.

Three other firmware image paths are referenced in `sceMeCodecWrapper` strings
(`meimg.img`, `me_blimg.img`, `me_sdimg.img`) but do not exist on FW 6.61.
They were likely consolidated into `me_t2img.img` in later firmware versions.

## Module Names for sctrlHENFindFunction

To resolve ME driver functions at runtime from a kernel PRX, use these exact
module and library name strings with `sctrlHENFindFunction()`:

### ME Kernel Modules (loaded during video playback)

| Module Name | Library Name | Functions |
|-------------|-------------|-----------|
| sceMeCodecWrapper | sceMeWrapper_driver | 23 (master interface, superset) |
| sceMeCodecWrapper | sceMeVideo_driver | 7 (video decode) |
| sceMeCodecWrapper | sceMeAudio_driver | 5 (audio decode) |
| sceMeCodecWrapper | sceMeMemory_driver | 3 (ME memory) |
| sceMeCodecWrapper | sceMeCore_driver | 4 (lifecycle) |
| sceMeCodecWrapper | sceMePower_driver | 5 (clock/power) |

### Codec User/Kernel Modules

| Module Name | Library Name | Functions |
|-------------|-------------|-----------|
| sceAvcodec_wrapper | sceVideocodec | Video codec syscalls |
| sceAvcodec_wrapper | sceAudiocodec | Audio codec syscalls |
| sceAvcodec_wrapper | sceMpegbase | MPEG-PS demuxer + CSC |
| sceAvcodec_wrapper | sceMpegbase_driver | Kernel-only MPEG variant |
| sceAvcodec_wrapper | sceJpeg | JPEG decode |

All 6 ME driver libraries are exported by the single `sceMeCodecWrapper`
module (11KB). The `sceAvcodec_wrapper` module (19KB) imports from these
libraries and re-exports them as the public `sceVideocodec` / `sceAudiocodec`
/ `sceMpegbase` APIs that user-mode code calls via syscalls.

## Kernel sceAvcodec_wrapper Error Tables

### Error Code Table (offset +0x49A0 in kernel module)

The kernel `sceAvcodec_wrapper` stores error codes in a lookup table.
Internal decode logic indexes into this table to return specific errors.

| Index | Offset | Code | Name | Description |
|-------|--------|------|------|-------------|
| 0 | +0x49A0 | 0x80000002 | GENERIC_ERROR | General failure |
| 1 | +0x49A4 | 0x807F00FF | AVCODEC_INVALID_DATA | Invalid input data |
| 2 | +0x49A8 | 0x807F0001 | AVCODEC_ERROR | Generic avcodec error |
| 3 | +0x49AC | **0x80628002** | **AVC_DECODE_FATAL** | ME decode failed fatally |
| 4 | +0x49B0 | 0x80628001 | AVC_DECODE_ERROR | ME decode non-fatal error |
| 5 | +0x49B4 | 0x80620002 | AVC_ERROR | AVC processing error |
| 6 | +0x49B8 | 0x806201FE | AVC_EMPTY_STUB | Empty ME stub returned garbage |

### Error Code Ranges

| Range | Source Module |
|-------|-------------|
| 0x8000xxxx | Generic kernel errors |
| 0x8061xxxx | sceMpeg library (user-mode) |
| 0x8062xxxx | sceAvcodec / sceVideocodec (kernel) |
| 0x807Fxxxx | Avcodec internal errors |
| 0x8011xxxx | Module loading errors |
| 0x8002xxxx | Kernel module manager errors |

### Common Error Codes Encountered

| Code | Name | Context |
|------|------|---------|
| 0x806201FE | AVC_EMPTY_STUB | sceVideocodecOpen/Decode through empty ME stubs |
| 0x80628002 | AVC_DECODE_FATAL | sceMpegAvcDecode — ME cannot decode frame |
| 0x80628001 | AVC_DECODE_ERROR | sceMpegAvcDecode — non-fatal (appears between keyframes) |
| 0x806101FE | MPEG_NO_DATA | sceMpegGetAvcNalAu — ME not booted for codec use |
| 0x80618001 | MPEG_NO_AU | sceMpegGetAvcAu — no AU available in ringbuffer |
| 0x80618003 | MPEG_ALREADY_INIT | sceMpegInit — already initialized |
| 0x80618005 | MPEG_ALREADY_INIT_FW | sceMpegInit — firmware-specific "already init" |
| 0x80020149 | EXCLUSIVE_LOAD | sceKernelLoadModule — module already loaded |
| 0x80020148 | ALREADY_LOADED | sceKernelLoadModule — module already present |
| 0x800200D3 | LIBRARY_NOT_YET_LINKED | sceKernelStartModule — imports unresolvable |
| 0x80110F02 | MODULE_ALREADY_LOADED | sceUtilityLoadAvModule — already loaded |
| 0x8002013A | LIBRARY_ALREADY_EXISTS | sceMpegInit — library already registered |

### Key Finding: Empty Stub at +0x2388

The kernel `sceAvcodec_wrapper` has exactly ONE empty stub (`jr $ra; nop`)
at offset +0x2388. However, **this stub has zero callers** — no code in the
module calls or jumps to it. It is dead code and NOT on any decode path.

The `0x80628002` (AVC_DECODE_FATAL) error is generated by the kernel
avcodec's internal decode logic, not by a missing function call.
