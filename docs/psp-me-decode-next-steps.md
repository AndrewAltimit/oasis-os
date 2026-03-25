# PSP ME H.264 Decode: Next Steps Plan

## Date: 2026-03-25 (Updated)

## BREAKTHROUGH: Child Module Loading Fixed (2026-03-25)

Two bugs in `rust-psp` were preventing `sceKernelStartModule` from working:

1. **EABI32 vs O32 ABI mismatch** (root cause): LLVM generates O32 code (args 5+
   on stack), PSP kernel expects EABI32 (args 5-8 in `$t0`-`$t3`). The 5th argument
   to `sceKernelStartModule` (`pOption`) was garbage in `$t0`, failing K1 validation.
   **Fix:** Added `i5` EABI mapper to `sceKernelStartModule` and other 5-arg functions.

2. **prxgen SHT_REL bug**: `prxgen` left `SHT_REL` sections for non-ALLOC targets in
   PRX output. Kernel processed them as real relocations, corrupting state.
   **Fix:** NULL out non-ALLOC `SHT_REL` section headers in prxgen.

**Result:** `sceKernelStartModule` now returns SUCCESS for child PRX modules loaded
from Rust EBOOTs. `mpeg_vsh370.prx` loading is unblocked.

See: `docs/psp-child-module-investigation.md` for full investigation trail.

## Current Status

- **NAL feeding works**: `sceMpegGetAvcNalAu` returns 0 for every frame
- **ME is booted**: `sceMeBootStart660` returns 0
- **AvcDecode fails**: returns `0x80628002` (AVC_DECODE_FATAL) on every frame, pic_num=0
- **PMPlayer works**: proves the hardware CAN decode H.264 on this exact PSP

## Phase A: Complete — Key Findings

### Module Comparison (OASIS vs PMPlayer)

| Module | OASIS OS | PMPlayer |
|--------|----------|----------|
| sceMpeg_library | Loaded (user, @0x08C0*) | NOT loaded |
| sceMpegVsh_library | NOT loaded | Loaded (user, @0x09E1*) |
| cooleyesBridge | NOT loaded | Loaded (kernel, 1264 bytes) |
| sceAvcodec_wrapper | Loaded (kernel) | Loaded (kernel) |
| Total modules | 62 | 56 |

**Root cause**: PMPlayer uses `sceMpegVsh_library` (from decrypted `mpeg_vsh370.prx`,
FW 3.71 era, plain ELF) instead of `sceMpeg_library` (from AvMpegBase, encrypted).

### cooleyesBridge.prx — Source Found

Just a thin FW-version dispatch kernel module (GPL, by cooleyes):
- `cooleyesMeBootStart(devkitVersion, mebooterType)` → calls `sceMeBootStart660` for FW 6.61
- `cooleyesAudioSetFrequency(devkitVersion, frequency)` → calls `sceAudioSetFrequency660`
- Uses `pspSdkSetK1(0)` to clear kernel protection bits
- NOT the "secret sauce" — just convenience wrappers

### mpeg_vsh370.prx — Import Analysis

**Import libraries** (all with strong flags `0x4001`):
| Library | Functions | Unknown 6.61 NIDs |
|---------|-----------|-------------------|
| ThreadManForUser | 7 | 0 |
| sceVideocodec | 12 | 2 |
| sceAudiocodec | 5 | 0 |
| sceMpegbase | 6 | 4 |
| sceMpeg | 59 (self-import) | N/A |

6 NIDs don't exist on FW 6.61 — FW 3.71→6.61 NID changes.

### What We Tried (All Failed with 0x800200D3)

1. `kuKernelLoadModule` + `sceKernelStartModule` for mpeg_vsh370.prx → LIBRARY_NOT_YET_LINKED
2. Same with AvMpegBase loaded first (provides sceMpegbase + sceMpeg) → same error
3. Same WITHOUT AvMpegBase → same error
4. Patched mpeg_vsh370.prx: all imports changed to weak (0x4009) → same error
5. Loaded cooleyesBridge.prx first (from EBOOT) → cooleyesBridge itself fails too
6. Kernel `sceKernelLoadModule` from PRX → 0x80020149 (unsupported PRX type)
7. VSH syscall extraction from PRX → only works during XMB, not after game launch

**Key blocker**: Module attribute `0x06` (firmware module) requires `sceUtilityLoadModule`
handling, but there's no utility module ID for mpeg_vsh. The weak import patch doesn't
help — the kernel rejects the module for reasons beyond NID resolution.

**Rust-psp already builds as PRX**: cargo-psp's `prxgen` tool automatically converts
every EBOOT to relocatable PRX format (ELF type 0xFFA0). BUILD_PRX=1 is NOT the
differentiator. Both OASIS and PMPlayer have `attr=0x00000000`.

## Recommended Next Steps (Priority Order)

### Option 1: Minimal C PSPSDK Test (Highest Priority)

**Goal**: Confirm whether mpeg_vsh370.prx loading works from a C EBOOT built with
PSPSDK, ruling out Rust-specific issues.

1. Build a minimal PSPSDK EBOOT that:
   - Calls `kuKernelLoadModule("mpeg_vsh370.prx")` + `sceKernelStartModule`
   - Logs result to file
   - Does NOT link against sceMpeg (no import stubs)
2. If it works → the issue is our EBOOT's sceMpeg import stubs conflicting
3. If it fails → the module is genuinely incompatible with our setup

**Time estimate**: Small — one C file, standard PSPSDK build

### Option 2: Build Custom mpeg_vsh for FW 6.61

**Goal**: Use uOFW's open-source sceMpeg reimplementation to build a compatible module.

1. Clone uOFW (https://github.com/uofw/uofw)
2. Find the sceMpeg module reimplementation
3. Build it targeting FW 6.61 NIDs
4. Load our custom module instead of mpeg_vsh370.prx

**Risk**: uOFW's reimplementation may be incomplete

### Option 3: Phase B — Kernel Avcodec Disassembly

**Goal**: Find why `sceMpeg_library` gets 0x80628002 and fix the root cause.

Binary: `/home/mikunpc/Downloads/me_dump/53_sceAvcodec_wrapper.bin` (19KB)
Module base during dump: `0x88236E00`

**What we found so far**:
- Error table at +0x49A0 (7 entries), extended table at +0x4998 (9 entries)
- 5 error table access sites (Pattern 2: base -0x4868 via SLL+LW+0x10)
- Each site calls ME submission stubs (+0x439C-0x43BC) then maps return value to error
- SLTIU+MOVN filters: return 0=success, non-special=0x807F0001, -4..-1=specific errors
- 0x80628002 is NEVER constructed inline — only exists in data table
- LUI 0x8062 always pairs with ORI 0x01FE (→ 0x806201FE), never 0x8002
- ME stubs at +0x4380-0x443C all populated with J instructions to sceMeCodecWrapper

**What's missing**: The code path that produces error table index 3 (0x80628002).
It's not reachable through the 5 Pattern 2 sites. Must come from a different mechanism
(possibly the ME writing directly to shared codec buffer, or a different lookup pattern).

### Option 4: Phase C — Direct ME RPC Bridge

**Goal**: Bypass kernel avcodec entirely using the ME RPC protocol.

1. Create kernel PRX that exports `oasisMeAvcDecode(codec_buf)`
2. Internally calls `sceMeVideo_driver::Decode` (NID 0x6D68B223)
3. Translates sceMpeg AU format to sceVideocodec codec_buf format
4. Returns decoded frame pointer

**Risk**: High complexity, requires codec buffer format reverse engineering

## Infrastructure Built This Session

- **Spy log**: Auto-flush every 5s, module dump at 30s boot
- **Module dump**: Kernel + user modules with addresses, sizes, attributes
- **Overlay UX**: L+R+START trigger works in OASIS OS (Start ignored with other buttons)
- **NID extraction**: Full import NID table from mpeg_vsh370.prx
- **Binary patching**: Weak import flag tool for PSP PRX files
- **VSH syscall extraction**: Works during XMB (not after game launch)
- **Ctrl hook infrastructure**: Written but disabled (crashes PMPlayer)

## Files Reference

| File | Purpose |
|------|---------|
| `crates/oasis-plugin-psp/src/main.rs` | Kernel PRX: ME boot, spy hooks, VSH syscall extraction |
| `crates/oasis-plugin-psp/src/me_dump.rs` | Module dumping, spy log, file IPC, VSH addr resolution |
| `crates/oasis-backend-psp/src/video.rs` | NalDecoder, VSH module loading attempts |
| `crates/oasis-plugin-psp/src/hook.rs` | Display/ctrl hooks (ctrl disabled) |
| `crates/oasis-plugin-psp/src/overlay.rs` | Overlay menu with spy dump |
| `/home/mikunpc/Downloads/me_dump/53_sceAvcodec_wrapper.bin` | Kernel avcodec dump (19KB) |
| `scripts/analyze_me_wrapper.py` | Capstone analysis script |
| `docs/psp-me-rpc-api.md` | RPC command table reference |
| `docs/psp-me-firmware-analysis.md` | Full ME architecture docs |
| `ms0:/PSP/GAME/OASISOS/mpeg_vsh370_patched.prx` | Patched PRX (weak imports) |
| `ms0:/PSP/GAME/UoPMPlayer_660/src/` | cooleyesBridge source code |
