#!/usr/bin/env python3
"""
Analyze sceMeCodecWrapper and sceAvcodec_wrapper runtime dumps.

Uses Capstone for MIPS disassembly to understand:
1. ME boot sequence (firmware loading from flash0)
2. RPC protocol (SceMeRpc semaphore communication)
3. ME driver function implementations (NID → code mapping)
4. Command structures passed to the ME

Input files (decrypted runtime dumps from PSP during video playback):
- 50_sceMeCodecWrapper.bin @ 0x88224900 (11,328 bytes)
- 53_sceAvcodec_wrapper.bin @ 0x88236E00 (19,036 bytes)
"""

import struct
import sys
from capstone import *
from capstone.mips import *

DUMP_DIR = "/home/mikunpc/Downloads/me_dump"

# Base addresses from runtime module dump
ME_WRAPPER_BASE = 0x88224900
AVCODEC_BASE = 0x88236E00

# Known NID → stub address mappings from our Phase 1 analysis
ME_WRAPPER_NIDS = {
    0x0DEFA6A5: ("sceMePower", 0x88225DC0),
    0x1862B784: ("sceMePower", 0x88225D78),
    0x21521BE5: ("sceMeVideo", 0x88225A54),
    0x24317CD0: ("sceMeWrapper_only", 0x88226970),
    0x4D78330C: ("sceMeVideo", 0x88224E00),
    0x5F6BF6DF: ("sceMeWrapper_only", 0x88226A30),
    0x635397BB: ("sceMeCore", 0x88226190),
    0x6AD33F60: ("sceMeAudio", 0x88225354),
    0x6D68B223: ("sceMeVideo", 0x88225060),
    0x6ED69327: ("sceMeMemory", 0x88225AB8),
    0x81956A0B: ("sceMeAudio", 0x88225220),
    0x8768915D: ("sceMeVideo", 0x88224C4C),
    0x8DD56014: ("sceMeVideo", 0x88224F30),
    0x92D3BAA1: ("sceMeMemory", 0x88225A74),
    0x984E2608: ("sceMePower", 0x88225D4C),
    0x9A9E21EE: ("sceMeAudio", 0x8822578C),
    0xB37562AA: ("sceMePower", 0x88225C40),
    0xB57F033A: ("sceMeAudio_only", 0x88225624),
    0xC300D466: ("sceMeAudio", 0x882258F8),
    0xC441994C: ("sceMeVideo", 0x882249D4),
    0xC4EDA9F4: ("sceMeMemory", 0x88225A94),
    0xE8CD3C75: ("sceMeVideo", 0x88224ADC),
    0xE9F69ACF: ("sceMePower", 0x88225D9C),
    0xFA398D71: ("sceMeCore", 0x88226078),
    0x5DFF5C50: ("sceMeCore", 0x8822666C),
    0x905A7500: ("sceMeCore", 0x88225AD8),
}

# Build reverse map: address → (driver, nid)
ADDR_TO_NID = {}
for nid, (driver, addr) in ME_WRAPPER_NIDS.items():
    ADDR_TO_NID[addr] = (driver, nid)


def load_binary(filename, base):
    """Load a binary file and return (data, base_addr)."""
    with open(f"{DUMP_DIR}/{filename}", "rb") as f:
        data = f.read()
    return data, base


def disassemble_function(data, base, start_addr, max_insns=200):
    """Disassemble a function starting at start_addr."""
    md = Cs(CS_ARCH_MIPS, CS_MODE_MIPS32 + CS_MODE_LITTLE_ENDIAN)
    md.detail = True

    offset = start_addr - base
    if offset < 0 or offset >= len(data):
        return []

    # Disassemble until we hit jr $ra followed by a delay slot
    insns = []
    found_jr_ra = False
    for insn in md.disasm(data[offset:], start_addr):
        insns.append(insn)
        if found_jr_ra:
            break  # This was the delay slot after jr $ra
        if insn.mnemonic == "jr" and insn.op_str == "$ra":
            found_jr_ra = True
        if len(insns) >= max_insns:
            break

    return insns


def find_jal_targets(insns):
    """Find all JAL (function call) targets in a list of instructions."""
    targets = []
    for insn in insns:
        if insn.mnemonic == "jal":
            try:
                target = int(insn.op_str, 0)
                targets.append((insn.address, target))
            except ValueError:
                pass
    return targets


def find_lui_ori_pairs(insns):
    """Find LUI+ORI/ADDIU pairs that load 32-bit constants (addresses/values)."""
    pairs = []
    for i in range(len(insns) - 1):
        if insns[i].mnemonic == "lui":
            hi_reg = insns[i].op_str.split(",")[0].strip()
            try:
                hi_val = int(insns[i].op_str.split(",")[1].strip(), 0)
            except (ValueError, IndexError):
                continue

            # Look ahead for matching ORI or ADDIU
            for j in range(i + 1, min(i + 5, len(insns))):
                op = insns[j].op_str
                if insns[j].mnemonic in ("ori", "addiu") and hi_reg in op:
                    parts = op.split(",")
                    if len(parts) >= 3:
                        try:
                            lo_val = int(parts[2].strip(), 0)
                            if insns[j].mnemonic == "ori":
                                full = (hi_val << 16) | lo_val
                            else:
                                # ADDIU sign-extends
                                if lo_val & 0x8000:
                                    lo_val = lo_val - 0x10000
                                full = (hi_val << 16) + lo_val
                            pairs.append((insns[i].address, full))
                        except ValueError:
                            pass
                    break
    return pairs


def analyze_module(filename, base, label):
    """Full analysis of a module binary."""
    data, base_addr = load_binary(filename, base)
    text_size = len(data)

    print(f"\n{'='*70}")
    print(f" {label}")
    print(f" Base: 0x{base:08X}  Size: {text_size} bytes ({text_size//1024}KB)")
    print(f"{'='*70}")

    # Find all known stub addresses in this module
    print(f"\n--- Known ME Driver Functions ---")
    stubs_in_module = []
    for addr, (driver, nid) in sorted(ADDR_TO_NID.items()):
        offset = addr - base
        if 0 <= offset < text_size:
            stubs_in_module.append((addr, driver, nid))
            print(f"  0x{addr:08X} (+0x{offset:04X}): {driver} NID 0x{nid:08X}")

    # Disassemble each known function
    print(f"\n--- Function Disassembly ---")
    for addr, driver, nid in stubs_in_module:
        insns = disassemble_function(data, base, addr, max_insns=100)
        if not insns:
            print(f"\n  [0x{addr:08X}] {driver}::0x{nid:08X} — could not disassemble")
            continue

        print(f"\n  [0x{addr:08X}] {driver}::0x{nid:08X} ({len(insns)} insns)")

        # Find calls and constants
        calls = find_jal_targets(insns)
        constants = find_lui_ori_pairs(insns)

        for insn in insns:
            # Annotate known addresses
            annotation = ""
            if insn.mnemonic == "jal":
                try:
                    target = int(insn.op_str, 0)
                    if target in ADDR_TO_NID:
                        d, n = ADDR_TO_NID[target]
                        annotation = f"  ; → {d}::0x{n:08X}"
                except ValueError:
                    pass

            print(f"    0x{insn.address:08X}: {insn.mnemonic:8s} {insn.op_str}{annotation}")

        if calls:
            print(f"    Calls:")
            for call_addr, target in calls:
                info = ""
                if target in ADDR_TO_NID:
                    d, n = ADDR_TO_NID[target]
                    info = f" ({d}::0x{n:08X})"
                print(f"      0x{call_addr:08X} → 0x{target:08X}{info}")

        if constants:
            print(f"    Constants/Addresses:")
            for const_addr, val in constants:
                info = ""
                if val in ADDR_TO_NID:
                    d, n = ADDR_TO_NID[val]
                    info = f" ({d}::0x{n:08X})"
                elif 0x88220000 <= val <= 0x8823FFFF:
                    info = " (kernel code)"
                elif 0x04000000 <= val <= 0x04200000:
                    info = " (EDRAM)"
                elif 0x40000000 <= val <= 0x50000000:
                    info = " (uncached)"
                elif 0xBC000000 <= val <= 0xBD000000:
                    info = " (HW register)"
                print(f"      0x{const_addr:08X}: 0x{val:08X}{info}")

    # Scan for interesting patterns across the whole binary
    print(f"\n--- String References ---")
    # Find all embedded strings
    i = 0
    while i < len(data):
        # Look for printable ASCII runs of 8+ chars
        if 0x20 <= data[i] <= 0x7e:
            end = i
            while end < len(data) and 0x20 <= data[end] <= 0x7e:
                end += 1
            if end - i >= 8:
                s = data[i:end].decode('ascii', errors='replace')
                addr = base + i
                print(f"  0x{addr:08X}: \"{s}\"")
            i = end
        else:
            i += 1

    # Find all JAL instructions across the entire binary
    print(f"\n--- All Function Calls (JAL targets) ---")
    md = Cs(CS_ARCH_MIPS, CS_MODE_MIPS32 + CS_MODE_LITTLE_ENDIAN)
    call_counts = {}
    for insn in md.disasm(data, base):
        if insn.mnemonic == "jal":
            try:
                target = int(insn.op_str, 0)
                call_counts[target] = call_counts.get(target, 0) + 1
            except ValueError:
                pass

    for target, count in sorted(call_counts.items(), key=lambda x: -x[1]):
        info = ""
        if target in ADDR_TO_NID:
            d, n = ADDR_TO_NID[target]
            info = f" ({d}::0x{n:08X})"
        offset = target - base
        if 0 <= offset < text_size:
            loc = "internal"
        else:
            loc = "external"
        print(f"  0x{target:08X} called {count}x [{loc}]{info}")

    # Scan for hardware register access patterns
    print(f"\n--- Hardware Register References ---")
    md = Cs(CS_ARCH_MIPS, CS_MODE_MIPS32 + CS_MODE_LITTLE_ENDIAN)
    md.detail = True
    hw_refs = set()
    prev_lui = {}
    for insn in md.disasm(data, base):
        if insn.mnemonic == "lui":
            parts = insn.op_str.split(",")
            if len(parts) == 2:
                try:
                    reg = parts[0].strip()
                    val = int(parts[1].strip(), 0)
                    prev_lui[reg] = (val, insn.address)
                except ValueError:
                    pass
        elif insn.mnemonic in ("lw", "sw", "lh", "sh", "lb", "sb"):
            # Check for HW register access
            op = insn.op_str
            for reg, (hi, lui_addr) in prev_lui.items():
                if reg in op and 0xBC00 <= hi <= 0xBF00:
                    # Extract offset
                    try:
                        off_str = op.split("(")[0].strip()
                        off = int(off_str, 0)
                        full_addr = (hi << 16) + off
                        hw_refs.add((lui_addr, full_addr))
                    except (ValueError, IndexError):
                        pass

    for addr, hw_addr in sorted(hw_refs):
        region = ""
        if 0xBC100000 <= hw_addr <= 0xBC1FFFFF:
            region = "SysCtrl"
        elif 0xBC000000 <= hw_addr <= 0xBC0FFFFF:
            region = "Clock/Power"
        elif 0xBD000000 <= hw_addr <= 0xBD0FFFFF:
            region = "Display"
        elif 0xBE000000 <= hw_addr <= 0xBFFFFFFF:
            region = "ME/GE"
        print(f"  0x{addr:08X}: access 0x{hw_addr:08X} [{region}]")


if __name__ == "__main__":
    # Analyze sceMeCodecWrapper
    analyze_module("50_sceMeCodecWrapper.bin", ME_WRAPPER_BASE, "sceMeCodecWrapper")

    print("\n" + "=" * 70)
    print("=" * 70)

    # Analyze sceAvcodec_wrapper
    analyze_module("53_sceAvcodec_wrapper.bin", AVCODEC_BASE, "sceAvcodec_wrapper")
