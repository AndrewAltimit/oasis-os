/* Supplementary linker script layered on top of rustc's
 * auto-forwarded target-spec script for the PSP backend.
 *
 * rust-psp's `mipsel-sony-psp-std.json` target spec ships an inline
 * SECTIONS layout that rustc dumps into a temp file and passes via
 * `--script` on every invocation. rust-lld is happy with that
 * layout, but when we route the final link through `psp-ld`
 * (binutils) — which we do to sidestep rust-lld's strict MIPS ABI
 * enforcement — the generated script is missing one crucial
 * symbol: `_gp`, the MIPS global pointer.
 *
 * The kernel PRX loader on PSP reads `_gp` out of the
 * `sceModuleInfo` descriptor at module load time and uses it to
 * seed the `$gp` register before jumping into user code. Anything
 * that uses small-data addressing (GPREL16 relocations) after that
 * point — and both `compiler_builtins` and `rust-psp`'s libpsp
 * rlib do — indexes off `$gp`. If `_gp` ends up at 0 (e.g. from a
 * `--defsym=_gp=0`), those accesses dereference low memory on the
 * very first small-data read and the console hard-crashes to a
 * shutdown.
 *
 * psp-ld accepts multiple `--script` arguments and merges them, so
 * we pass this file in addition to the rustc-generated one. All it
 * does is synthesise `_gp` with the traditional PSPSDK formula —
 * 32 KB past an aligned address — which puts it in the middle of
 * the small-data window where signed 16-bit offsets can reach both
 * directions.
 */
SECTIONS
{
  _gp = ALIGN(16) + 0x7ff0;
}
