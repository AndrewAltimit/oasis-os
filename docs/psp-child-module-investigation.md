# PSP Child Module Loading Investigation

## Date: 2026-03-25

## Problem

`sceKernelStartModule` returns `0x800200D3` (`SCE_ERROR_KERNEL_ILLEGAL_ADDR`) for ALL
calls from rust-psp EBOOTs, while the same calls from PSPSDK C EBOOTs succeed. This
prevents loading child PRX modules (like `mpeg_vsh370.prx` for H.264 video decode) from
Rust homebrew.

## Environment

- PSP-3001 (Slim), FW 6.61, ARK-4 CFW
- rust-psp (local fork at `/home/mikunpc/Documents/repos/rust-psp/`)
- PSPSDK via Docker (`pspdev/pspdev`)

## Definitive Findings

### The error is process-specific, not module-specific

| Test | C EBOOT | Rust EBOOT |
|------|---------|------------|
| Start already-started module | `0x80020001` (correct: "already started") | `0x800200D3` |
| Start nulltest.prx (9 imports) | **SUCCESS** | `0x800200D3` |
| Start mpeg_vsh370.prx | **SUCCESS** | `0x800200D3` |

The SAME kernel function (`sceKernelStartModule`, NID `0x50F0C1EC`) returns different
error codes depending on which EBOOT process is calling it.

### The error is in the binary format, not the code logic

A C EBOOT built with PSPSDK using:
- The SAME module name (`"RustVshTest"`)
- The SAME operations (load + start nulltest.prx)
- Function-pointer calling pattern (simulating Rust)

**WORKS PERFECTLY.** This proves the issue is 100% in the binary format.

### prxgen bug found and fixed

rust-psp's `prxgen` tool left `SHT_REL` relocation sections (type 9) targeting
non-ALLOC sections in the PRX output. PSPSDK's `psp-prxgen` removes these. The PSP
kernel processes ALL relocation sections, so these leftover `SHT_REL` entries cause
bogus relocations that corrupt module state.

**Fix:** In `prxgen.rs`, NULL out section headers for non-ALLOC `SHT_REL` sections.

This fix resolves a separate issue (C ELF + rust-psp prxgen couldn't boot at all)
but does NOT fix the child module start issue for Rust-compiled ELFs.

### Two separate bugs

1. **prxgen SHT_REL bug** — FIXED. Leftover SHT_REL sections corrupt module state.
2. **Rust ELF content issue** — UNSOLVED. Something in LLVM-generated MIPS code or
   rust-psp module metadata causes `sceKernelStartModule` to fail.

## What Was Eliminated

| Hypothesis | Test | Result |
|------------|------|--------|
| Module version byte order | Fixed in rust-psp `[minor, major]` | Still fails |
| Syslib version non-zero | Set to `(0, 0)` | Still fails |
| Missing module_sdk_version | Added to syslib exports | Still fails |
| sceKernelSetCompiledSdkVersion | Removed | Still fails |
| .sceStub.text format (data vs code) | Patched to `jr $ra; nop` | Still fails |
| .rel.sceStub.text relocations | Zeroed section | Still fails |
| ELF flags (missing ALLEGREX) | Patched to `0x10A23001` | Still fails |
| PARAM.SFO MEMSIZE | Added `MEMSIZE=1` | Still fails |
| prxgen tool | PSPSDK psp-prxgen on Rust ELF | Still fails |
| K1 pointer validation | Patched 16+ kernel checks | Still fails |
| Module name | C test with name "RustVshTest" | C test works |
| Section ordering | PSPSDK-style linker script | Still fails |
| .MIPS.abiflags presence | Kept in loaded segment | Still fails |
| .MIPS.abiflags + e_flags | Zeroed abiflags + C flags | Still fails |
| NID/stub mapping | Verified all match | Correct |
| Syscall numbers | Dumped from stubs | Correct (different per-process, normal) |

## Kernel Analysis

### Dumped kernel modules
- `sceModuleManager` (modulemgr) at `0x8805A400`, 39KB
- `sceLoaderCore` (loadcore) at `0x88017000`, 33KB
- Raw dumps saved to PSP at `OASISOS/modulemgr_raw.bin` and `OASISOS/loadcore_raw.bin`

### Key kernel functions identified
- `sceKernelStartModule`: `0x8805E198` (modulemgr export, NID `0x50F0C1EC`)
- `_PrologueModule` candidate: `0x8805EA28` (internal, no K1 shift)
- `sceKernelLinkLibraryEntriesWithModule` wrapper: `0x8805E5E8`
- `sceKernelLinkLibraryEntriesForUser`: `0x8801E038` (loadcore export)

### ARK-4 CFW hooks (from source analysis)
- `prologue_module_hook` in modulemgr (hooks `_PrologueModule`)
- `InitKernelStartModule` in init.prx bootstart (NOT user-mode)
- `stargateStartModuleHandler` for game-mode modules
- `patchLoadModuleFuncs` hooks `sceKernelLoadModule` in all modules
- None of these hook user-mode `sceKernelStartModule` directly

## Binary Format Differences (C vs Rust PRX)

| Characteristic | C (PSPSDK) | Rust (rust-psp) |
|---------------|-----------|-----------------|
| e_flags | `0x10A23001` (ALLEGREX, EABI32) | `0x10001001` (CPIC only) |
| .sceStub.text | `jr $ra; nop` code | Data pointer pairs |
| .MIPS.abiflags | All zeros (24 bytes) | Non-zero (ISA=2, GPR=1) |
| p_offset | `0x60` | `0x160` (larger due to more PHs) |
| Sections | .init/.fini/.sdata/.sbss | .got/.gcc_except_table |
| SHT_REL in PRX | None (stripped) | Present (FIXED in prxgen) |
| Code generation | GCC (abicalls) | LLVM (non-abicalls) |

## Test Infrastructure

- Minimal Rust test: `/tmp/rust-vsh-test/` (reproduces the issue)
- C test (works): `/tmp/psp-vsh-test/`
- C hybrid (works): `/tmp/c-hybrid-test/`
- nulltest.prx: PSPSDK zero-import test module at `RUSTVSH/nulltest.prx`
- PRX kernel patches: `crates/oasis-plugin-psp/src/me_dump.rs`

## ROOT CAUSE: EABI32 vs O32 ABI Mismatch

**LLVM generates O32 MIPS code** for the PSP target. On O32, arguments 5+ are
passed on the stack. **The PSP kernel uses EABI32**, where arguments 5-8 go in
registers `$t0`-`$t3`.

For `sceKernelStartModule(modId, argSize, argp, pModResult, pOption)`:
- Rust/LLVM (O32): `$a0`=modId, `$a1`=argSize, `$a2`=argp, `$a3`=pModResult,
  **stack**=pOption
- PSP kernel (EABI32): `$a0`=modId, `$a1`=argSize, `$a2`=argp, `$a3`=pModResult,
  **`$t0`**=pOption

The kernel read garbage from `$t0` as `pOption`. If `$t0` was non-zero, the kernel
validated it as a pointer, failed K1 validation, and returned `0x800200D3`.

## Fix

rust-psp already has EABI32 bridge functions in `eabi.rs` (`i5`, `i6`, `i7`) that
load args 5-8 from the O32 stack into `$t0`-`$t3` before calling the stub. These
just needed to be applied to ALL 5+ argument PSP functions via the `#[psp(NID, i5)]`
annotation in `psp_extern!` blocks.

Functions fixed:
- `sceKernelStartModule` (5 args) → `i5`
- `sceKernelStopModule` (5 args) → `i5`
- `sceFontGetCharGlyphImage_Clip` (5 args) → `i5`
- `sceFontGetShadowGlyphImage_Clip` (5 args) → `i5`

Also found and fixed: prxgen leaving SHT_REL sections in PRX output (separate bug).

## Verification

nulltest.prx successfully started from Rust EBOOT after applying the `i5` mapper
to `sceKernelStartModule`.
