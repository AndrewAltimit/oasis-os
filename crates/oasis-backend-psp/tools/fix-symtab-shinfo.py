#!/usr/bin/env python3
"""Recompute ``.symtab`` ``sh_info`` so rust-lld accepts GCC 15 MIPS output.

psp-gcc 15.2 + binutils 2.43 emit ELF32 object files where the
``.symtab`` section's ``sh_info`` field under-counts the number of
locally-bound symbols. Binutils ld/psp-ld ignore this and re-scan
bindings directly, but rust-lld strictly trusts ``sh_info`` and treats
any LOCAL symbol whose index >= sh_info as "invalid binding: 0".

This script walks a little-endian ELF32 MIPS object, finds the
``.symtab`` SHT_SYMTAB section, determines the true "first non-local
symbol" index by scanning entries, and rewrites ``sh_info`` to match.
Idempotent: re-running is a no-op.

Invoked by ``crates/oasis-backend-psp/tools/psp-gcc-wrap.sh`` after every
``psp-gcc -c ... -o foo.o`` invocation from cc-rs while building
``rquickjs-sys`` for the PSP backend. The patched ``.o`` files are
functionally identical — only the section-header field changes.

Usage: ``fix-symtab-shinfo.py <object-file>`` (mutates in place).
"""
import struct
import sys


def fix(path: str) -> None:
    with open(path, "rb") as f:
        data = bytearray(f.read())
    if data[:4] != b"\x7fELF":
        return  # not an ELF — silently ignore (e.g. response files).
    if data[4] != 1 or data[5] != 1:
        return  # not ELF32LE — we only emit 32-bit LE for PSP.

    # ELF32 header layout we care about.
    e_shoff = struct.unpack_from("<I", data, 0x20)[0]
    e_shentsize = struct.unpack_from("<H", data, 0x2E)[0]
    e_shnum = struct.unpack_from("<H", data, 0x30)[0]
    e_shstrndx = struct.unpack_from("<H", data, 0x32)[0]

    def sh(i: int) -> tuple[int, int, int, int, int, int, int, int, int, int]:
        base = e_shoff + i * e_shentsize
        return struct.unpack_from("<IIIIIIIIII", data, base)

    # Resolve section-header string table for name lookups.
    _, _, _, _, shstr_off, shstr_size, *_ = sh(e_shstrndx)

    def name(i: int) -> str:
        sh_name, *_ = sh(i)
        end = data.find(b"\x00", shstr_off + sh_name, shstr_off + shstr_size)
        return data[shstr_off + sh_name:end].decode("ascii", "replace")

    for i in range(e_shnum):
        (
            _sh_name,
            sh_type,
            _sh_flags,
            _sh_addr,
            sh_offset,
            sh_size,
            _sh_link,
            sh_info,
            _sh_addralign,
            sh_entsize,
        ) = sh(i)
        if sh_type != 2:  # SHT_SYMTAB
            continue
        if sh_entsize == 0:
            continue
        n_syms = sh_size // sh_entsize
        # Find the first symbol whose binding (st_info >> 4) is not
        # STB_LOCAL (0). Elf32_Sym: st_name(4) st_value(4) st_size(4)
        # st_info(1) st_other(1) st_shndx(2). st_info is at offset 12.
        first_non_local = n_syms
        for j in range(n_syms):
            st_info = data[sh_offset + j * sh_entsize + 12]
            binding = st_info >> 4
            if binding != 0:
                first_non_local = j
                break
        if first_non_local == sh_info:
            continue  # already correct.
        # Rewrite sh_info for this section-header entry.
        sh_info_off = e_shoff + i * e_shentsize + 28  # offset of sh_info.
        struct.pack_into("<I", data, sh_info_off, first_non_local)

    with open(path, "wb") as f:
        f.write(data)


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("usage: fix-symtab-shinfo.py <object>", file=sys.stderr)
        return 2
    for p in argv[1:]:
        fix(p)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
