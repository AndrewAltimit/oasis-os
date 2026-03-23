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

## Research Needed

### 1. ME Firmware Binary

- Where is it? Likely embedded in `mpeg.prx` or `avcodec.prx`, or loaded
  from flash0 at runtime
- How to extract it? Memory dump after sceMpegCreate (ME firmware is
  loaded to a known address range)
- What format? Raw MIPS code for the ME core

### 2. ME Boot Protocol

From our Ghidra analysis of mpeg.prx:
- `FUN_08c0c940` (codec init) calls through to sceVideocodec
- 4-step ME init: `FUN_08c0d5e4` → `FUN_08c0d594` → `FUN_08c0d5a4` → `FUN_08c0d5cc`
- These are syscall stubs jumping to kernel addresses

From rust-psp's `me` module:
- `psp::me::start()` can boot the ME with custom code
- `psp::me::stop()` halts the ME
- Requires kernel mode

### 3. Command Protocol

The main CPU sends decode commands via shared memory:
- Command buffer in uncached EDRAM (0x04000000 | 0x40000000)
- Structure: AU data pointer, AU size, output buffer pointer, codec params
- ME polls or is interrupted for new commands
- ME writes decoded YCbCr/ABGR to output buffer

### 4. Existing Resources

- **uOFW**: Has NOT reverse-engineered mpeg.prx (orphan module)
- **JPCSP**: Java emulation, uses host CPU for decode (no ME protocol)
- **PPSSPP**: C++ HLE, uses ffmpeg for decode (no ME protocol)
- **Ghidra output**: 208 functions decompiled from user-mode mpeg.prx
  (in `/home/mikunpc/Downloads/output/mpeg_ringbuffer.txt`)
- **psdevwiki**: ME documentation at https://www.psdevwiki.com/psp/Media_Engine
- **Our kernel PRX**: Can run code in kernel mode, access ME registers

## Implementation Strategy

### Phase 1: ME Firmware Extraction

1. Use our kernel PRX to dump the ME firmware after a game loads it
   (hook sceDisplaySetFrameBuf during game boot, dump ME memory region)
2. Or: extract from decrypted mpeg.prx binary (look for embedded firmware)
3. Disassemble the ME firmware to understand the command protocol

### Phase 2: ME Boot from Homebrew

1. Use `psp::me::start(entry_point)` from kernel mode
2. Load the extracted ME firmware to the correct address
3. Verify ME is running (read shared memory status register)

### Phase 3: Direct Decode Commands

1. Implement the command buffer protocol (from ME firmware RE)
2. Copy H.264 AU data to uncached shared memory
3. Signal ME to decode
4. Wait for completion (poll status or wait for interrupt)
5. Read decoded frame from EDRAM

### Phase 4: Integration

1. Replace SceMpegDecoder with DirectMeDecoder
2. Feed streaming H.264 AUs directly to ME
3. Convert decoded YCbCr to RGBA (VFPU or ME's CSC)
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
