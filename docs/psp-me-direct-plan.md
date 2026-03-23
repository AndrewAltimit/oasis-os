# PSP Media Engine: Direct Programming Plan

## Context

H.264 video decode via `sceVideocodec` and `sceMpeg` APIs is blocked on
PSP-3001 (TA-090v2) / ARK-4 CFW / FW 6.61 because `avcodec.prx` has
empty ME submission stubs. Both API paths delegate to these stubs:

- `sceVideocodec` → ME stubs at FUN_00004414/4424/4434 → `void{return;}`
- `sceMpeg` → kernel demuxer → sceVideocodec → same empty stubs

Games work because UMD boot loads patched firmware with populated stubs.
Homebrew never gets this initialization.

## Goal

Bypass `avcodec.prx` entirely. Load the ME firmware binary, set up shared
memory, and communicate directly with the ME coprocessor for H.264 decode.

## Background: PSP Media Engine Architecture

The PSP has two CPUs:
- **Main CPU (Allegrex)**: MIPS R4000, 1-333MHz, runs user/kernel code
- **Media Engine (ME)**: Identical MIPS core, dedicated to media decode

Communication:
- **Shared uncached RAM**: 0x04000000-0x041FFFFF (2MB EDRAM)
- **Hardware mutex**: Spinlock between CPUs
- **Software interrupts**: ME signals main CPU via interrupt
- **DMA**: Direct memory access for bulk data transfer

The ME runs its own firmware binary (loaded by mpeg.prx during sceMpegCreate).
The firmware implements H.264/MPEG-4 decode. The main CPU sends commands
via shared memory structures, and the ME writes decoded frames to EDRAM.

## Research Findings (Phase 1 Complete — 2026-03-23)

### 1. ME Firmware Binary — LOCATED

ME firmware is NOT embedded in mpeg.prx or avcodec.prx. It is loaded from
**flash0** by the kernel module `sceMeCodecWrapper`:

- `flash0:/kd/resource/meimg.img` — Main ME firmware (H.264/MPEG-4/AAC)
- `flash0:/kd/resource/me_blimg.img` — ME bootloader
- `flash0:/kd/resource/me_sdimg.img` — ME shutdown image
- `flash0:/kd/resource/me_t2img.img` — ME type-2 variant

These are encrypted images. `sceMesgLed` (42KB crypto module) handles
decryption during loading.

### 2. ME Driver Architecture — DISCOVERED

Runtime dump of 68 modules during UMD Video playback (Spider-Man 2) revealed
the complete ME driver chain. See `docs/psp-me-firmware-analysis.md` for full
NID tables and analysis.

**Key insight**: Homebrew ME stubs are empty because the kernel-side modules
`sceMeCodecWrapper` and `sceAvcodec_wrapper` are only loaded by VSH/game boot,
not by `sceUtilityLoadModule`.

**Driver stack**:
```
sceAvcodec_wrapper (kernel) → sceMeVideo/Audio/Core/Memory/Power_driver
  → sceMeCodecWrapper (kernel) → loads ME firmware from flash0
    → communicates via RPC (SceMeRpc semaphore)
```

### 3. Command Protocol — RPC-based

The ME uses Remote Procedure Call, not direct shared-memory polling:
- Semaphore: `SceMeRpc`
- Events: `SceMediaEngineRpc`, `SceMediaEngineRpcWait`, `SceMediaEngineAvcPower`
- `sceMeWrapper_driver` has 23 functions covering all ME operations
- `sceMeVideo_driver` has 7 functions specific to video decode

### 4. Existing Resources

- **uOFW**: Has NOT reverse-engineered mpeg.prx (orphan module)
- **JPCSP**: Java emulation, uses host CPU for decode (no ME protocol)
- **PPSSPP**: C++ HLE, uses ffmpeg for decode (no ME protocol)
- **Ghidra output**: 208 functions decompiled from user-mode mpeg.prx
  (in `/home/mikunpc/Downloads/output/mpeg_ringbuffer.txt`)
- **psdevwiki**: ME documentation at https://www.psdevwiki.com/psp/Media_Engine
- **Our kernel PRX**: Can run code in kernel mode, access ME registers

## Implementation Strategy (Updated with Phase 1 Findings)

### Phase 1: ME Firmware Extraction — COMPLETE

Runtime dump of all 68 loaded modules during UMD Video playback. Key modules
dumped to `ms0:/seplugins/me_dump/` and `/home/mikunpc/Downloads/me_dump/`.

### Phase 2: Load Kernel ME Modules from Homebrew (Preferred Path)

The simplest fix: have our kernel PRX load the same kernel modules that
VSH/games load. This would populate the ME stubs without any custom code.

1. From our kernel PRX (`oasis-plugin-psp`), load `sceMeCodecWrapper` module
   - It will load ME firmware from flash0 automatically
   - It will register the sceMeWrapper/Video/Audio/etc driver libraries
2. Load `sceAvcodec_wrapper` module
   - It will register sceVideocodec/sceAudiocodec/sceMpegbase exports
   - User-mode avcodec.prx stubs will now have real implementations to call
3. Test: homebrew `sceVideocodecOpen` should now succeed

Challenge: these modules may be encrypted on flash0 and require
`sceMesgLed` to decrypt. May need to extract them from a running game
instead of flash0.

### Phase 2b: Call ME Drivers Directly (Fallback)

If module loading fails, use the NID tables from Phase 1 to call ME
driver functions directly from kernel mode:

1. Resolve functions via `sctrlHENFindFunction` using discovered module names
   (`sceMeCodecWrapper`, `sceAvcodec_wrapper`)
2. Call `sceMeCore_driver` NIDs to boot the ME (4 functions)
3. Call `sceMeVideo_driver` NIDs for H.264 decode (7 functions)
4. Call `sceMeMemory_driver` NIDs for buffer management (3 functions)

### Phase 3: Extract ME Firmware from Flash0

If Phase 2 requires the firmware images directly:

1. Mount flash0 (user already demonstrated access via USB)
2. Copy `meimg.img`, `me_blimg.img`, `me_sdimg.img`, `me_t2img.img`
3. May need decryption via `sceMesgLed` (42KB, also dumped)
4. Disassemble with Ghidra to understand RPC command structures

### Phase 4: Integration

1. Replace SceMpegDecoder with working video decode path
2. Feed streaming H.264 AUs to ME via the working API
3. Convert decoded YCbCr to RGBA (VFPU or sceMpegBaseCsc)
4. Push frames to the video texture queue

## Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| ME firmware not extractable | High | Try multiple dump methods; RE the boot protocol |
| Command protocol unknown | High | RE the ME firmware; compare with JPCSP/PPSSPP ME stubs |
| ME requires kernel patching | Medium | Our PRX runs in kernel mode |
| Memory conflicts with other modules | Medium | Careful EDRAM management |
| ME firmware version-specific | Medium | Target our specific FW 6.61 |

## Files

| File | Purpose |
|------|---------|
| `crates/oasis-plugin-psp/` | Kernel PRX for ME access |
| `scripts/ghidra_mpeg_ringbuffer.py` | Ghidra analysis script |
| `/home/mikunpc/Downloads/output/mpeg_ringbuffer.txt` | Full decompilation output |
| `/home/mikunpc/Downloads/mpeg_decrypted.bin` | User-mode mpeg.prx dump |
