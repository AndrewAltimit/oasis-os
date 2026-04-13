/*
 * Newlib header shim used by the rquickjs-sys C build on PSP.
 *
 * pspdev's `sys/config.h` unconditionally defines
 *
 *     #define __ATTRIBUTE_IMPURE_PTR__ __attribute__((__section__(".sdata")))
 *
 * under `#if defined(__mips__) && !defined(__rtems__)`. That forces
 * `_impure_ptr` into the `.sdata` small-data section, which in turn
 * makes every reference GPREL16-addressed. QuickJS-NG compiles with
 * `-G0` so it never contributes to small-data itself, but it still
 * emits GPREL16 relocations against `_impure_ptr` because the
 * extern declaration in `<sys/reent.h>` carries the section
 * attribute from the header. Whenever total small-data exceeds 32 KB
 * (which happens fast in a 400 KB library like QuickJS), those
 * relocations overflow and the link fails with
 *
 *     relocation truncated to fit: R_MIPS_GPREL16 against `_impure_ptr'
 *
 * Fix: intercept `<sys/config.h>` via an earlier `-isystem` search
 * path, forward to the real upstream header with `#include_next`,
 * then unconditionally override `__ATTRIBUTE_IMPURE_PTR__` to a
 * nop. Because the real `<sys/reent.h>` reads that macro *after*
 * `<sys/config.h>` has been fully processed, our override wins and
 * `_impure_ptr` ends up in regular `.data`, reachable via the
 * standard 32-bit address relocations.
 *
 * Only C files compiled via `psp-gcc-wrap.sh` see this shim —
 * pspdev's own prebuilt libs are unaffected because they're never
 * recompiled during a cargo build.
 */

#include_next <sys/config.h>

#undef __ATTRIBUTE_IMPURE_PTR__
#define __ATTRIBUTE_IMPURE_PTR__ /* empty — keep `_impure_ptr` out of .sdata */
