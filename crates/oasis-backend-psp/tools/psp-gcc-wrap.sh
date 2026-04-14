#!/usr/bin/env bash
# Wrapper that runs pspdev's psp-gcc and post-processes any .o output.
#
# GCC 15 / binutils 2.43 on the mipsel-sony-psp target emits ELF32
# object files whose `.symtab` section header declares `sh_info` below
# the index of the last locally-bound symbol. rust-lld strictly trusts
# sh_info and rejects any LOCAL symbol whose index >= sh_info with
# "invalid binding: 0". binutils ld is permissive and ignores this.
#
# Fix: after every `psp-gcc -c ... -o foo.o`, we run psp-ld in
# partial-link mode (`-r`) which re-emits the `.symtab` with locals
# physically before globals and updates sh_info to match. This is a
# cheap reassembly — no code motion, no inlining, no cross-file work.
#
# Used only during cross-compilation of rquickjs-sys — wired up via
# `CC_mipsel_sony_psp_std = ".../tools/psp-gcc-wrap.sh"` in the
# backend's `.cargo/config.toml`.
set -euo pipefail

PSP_GCC="${PSP_GCC_REAL:-/opt/pspdev/bin/psp-gcc}"
PSP_LD="${PSP_LD_REAL:-/opt/pspdev/bin/psp-ld}"

HERE="$(cd "$(dirname "$0")" && pwd)"
"$PSP_GCC" "-isystem${HERE}/newlib-shim" "$@"

# Scan args for the output path (`-o <path>`), if any, and rewrite it.
prev=""
out=""
for arg in "$@"; do
  if [ "$prev" = "-o" ]; then
    out="$arg"
    break
  fi
  prev="$arg"
done

if [ -n "$out" ] && [ "${out##*.}" = "o" ] && [ -f "$out" ]; then
  # Step 1: psp-ld -r re-emits .symtab with all locals packed before
  # globals and remaps relocations. Step 2: our own patcher fixes up
  # `.symtab`'s `sh_info`, which binutils 2.43's partial-link writes
  # as the position of the first FILE symbol rather than the first
  # non-local. Without step 2, rust-lld still rejects symbols 13+.
  tmp="${out}.reorder.o"
  "$PSP_LD" -r "$out" -o "$tmp"
  mv "$tmp" "$out"
  python3 "$HERE/fix-symtab-shinfo.py" "$out"
fi

exit 0
