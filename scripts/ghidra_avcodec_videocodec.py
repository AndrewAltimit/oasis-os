# -*- coding: utf-8 -*-
# Ghidra headless analysis script for decrypted avcodec.prx (PSP 6.61)
#
# Analyzes the sceVideocodec implementation to understand why
# sceVideocodecOpen returns error 0x806201fe on real PSP hardware.
#
# Specifically:
#   1. Find and decompile sceVideocodecOpen (NID 0xC01EC829)
#   2. Identify what condition produces error 0x806201fe
#   3. Find all sceVideocodec exported functions and decompile them
#   4. Look for ME initialization requirements or module dependencies
#   5. Check for calls to other kernel functions or ME registers
#
# Usage:
#   docker run --rm --platform linux/amd64 \
#     -e HOME=/home/ghidra \
#     -e _JAVA_OPTIONS="-Xint -Xmx2g" \
#     -v ./scripts:/data/scripts:ro \
#     -v /home/mikunpc/Downloads:/data/prx:ro \
#     -v /tmp/ghidra_avcodec:/data/output \
#     -v /tmp/ghidra_project:/data/project \
#     ghidra-headless \
#     /data/project AvcodecProject \
#     -import /data/prx/avcodec_decrypted \
#     -processor "MIPS:LE:32:default" \
#     -postScript /data/scripts/ghidra_avcodec_videocodec.py \
#     -overwrite

from ghidra.app.decompiler import DecompInterface
from ghidra.util.task import ConsoleTaskMonitor

OUTPUT = "/data/output/avcodec_analysis.txt"

# sceVideocodec NIDs (from psplibdoc kd/videocodec.csv + avcodec.csv)
VIDEOCODEC_NIDS = {
    0x17099F0A: "sceVideocodecInit",
    0x17CF7D2C: "sceVideocodec_17CF7D2C",  # unknown
    0x26927D19: "sceVideocodecGetVersion",
    0x2D31F5B1: "sceVideocodecGetEDRAM",
    0x2F385E7F: "sceVideocodecScanHeader",
    0x307E6E1C: "sceVideocodecDelete",
    0x4F160BF4: "sceVideocodecReleaseEDRAM",
    0x627B7D42: "sceVideocodecGetSEI",
    0x745A7B7A: "sceVideocodecSetMemory",
    0x893B32B1: "sceVideocodec_893B32B1",  # unknown
    0xA2F0564E: "sceVideocodecStop",
    0xC01EC829: "sceVideocodecOpen",
    0xD95C24D5: "sceVideocodec_D95C24D5",  # unknown
    0xDBA273FA: "sceVideocodecDecode",
}

AUDIOCODEC_NIDS = {
    0x29681260: "sceAudiocodecReleaseEDRAM",
    0x3A20A200: "sceAudiocodecGetEDRAM",
    0x3DD7EE1A: "sceAudiocodec_3DD7EE1A",
    0x59176A0F: "sceAudiocodec_59176A0F",
    0x5B37EB1D: "sceAudiocodecInit",
    0x70A703F8: "sceAudiocodecDecode",
    0x8ACA11D5: "sceAudiocodec_8ACA11D5",
    0x9D3F790C: "sceAudiocodecCheckNeedMem",
}

# Known error code we're investigating
TARGET_ERROR = 0x806201FE


def log(msg):
    print(msg)
    f = open(OUTPUT, "a")
    f.write(msg + "\n")
    f.close()


def decompile_at(addr_int):
    """Decompile function at given address."""
    addr = (
        currentProgram.getAddressFactory()
        .getDefaultAddressSpace()
        .getAddress(addr_int)
    )
    func = getFunctionAt(addr)
    if func is None:
        createFunction(addr, None)
        func = getFunctionAt(addr)
    if func is None:
        return "// Could not create function at 0x%08X" % addr_int
    decomp = DecompInterface()
    decomp.openProgram(currentProgram)
    results = decomp.decompileFunction(func, 120, ConsoleTaskMonitor())
    if results.decompileCompleted():
        return results.getDecompiledFunction().getC()
    return "// Decompilation failed at 0x%08X" % addr_int


def find_export_table(data, lib_name):
    """Find NID export table for a library name in the ELF."""
    needle = lib_name.encode("ascii") + b"\x00"
    pos = 0
    results = []
    while True:
        idx = data.find(needle, pos)
        if idx < 0:
            break
        # After the null-terminated string (padded to alignment),
        # the NID table follows
        nid_start = idx + len(needle)
        # Align to 4 bytes
        nid_start = (nid_start + 3) & ~3
        results.append((idx, nid_start))
        pos = idx + 1
    return results


def scan_for_error_code(data, error_code):
    """Find all locations where the error code appears in the binary."""
    import struct

    locations = []
    needle = struct.pack("<I", error_code)
    pos = 0
    while True:
        idx = data.find(needle, pos)
        if idx < 0:
            break
        locations.append(idx)
        pos = idx + 1

    # Also check for LUI/ORI patterns that construct the error code
    # LUI loads upper 16 bits, ORI adds lower 16 bits
    upper = (error_code >> 16) & 0xFFFF
    lower = error_code & 0xFFFF
    # Search for the upper half as an immediate in LUI instructions
    for i in range(0, len(data) - 4, 4):
        instr = struct.unpack_from("<I", data, i)[0]
        # LUI rt, imm: 0011 11xx xxxT TTTT IIII IIII IIII IIII
        if (instr >> 26) == 0x0F:  # LUI opcode
            imm = instr & 0xFFFF
            if imm == upper:
                locations.append(i)

    return locations


# ── Main analysis ──────────────────────────────────────────────────────

# Clear output
f = open(OUTPUT, "w")
f.write("")
f.close()

log("=" * 72)
log("avcodec.prx Analysis — sceVideocodec Error 0x806201FE Investigation")
log("=" * 72)

# Get the raw bytes for scanning
memory = currentProgram.getMemory()
blocks = list(memory.getBlocks())
log("\nMemory blocks:")
for blk in blocks:
    log(
        "  %s: %s - %s (%d bytes)"
        % (blk.getName(), blk.getStart(), blk.getEnd(), blk.getSize())
    )

# Read all program bytes for pattern scanning
base_addr = blocks[0].getStart().getOffset() if blocks else 0
raw_data = bytearray()
for blk in blocks:
    size = blk.getSize()
    buf = bytearray(size)
    for i in range(size):
        try:
            buf[i] = memory.getByte(blk.getStart().add(i)) & 0xFF
        except:
            buf[i] = 0
    raw_data.extend(buf)

log("\nTotal bytes read: %d" % len(raw_data))

# Scan for error code 0x806201FE in the binary
log("\n" + "=" * 72)
log("Scanning for error code 0x%08X..." % TARGET_ERROR)
log("=" * 72)

import struct

# Direct word match
needle = struct.pack("<I", TARGET_ERROR)
pos = 0
error_locs = []
while True:
    idx = raw_data.find(needle, pos)
    if idx < 0:
        break
    error_locs.append(base_addr + idx)
    pos = idx + 1

# LUI/ORI pattern: LUI loads 0x8062, ORI adds 0x01FE
upper = (TARGET_ERROR >> 16) & 0xFFFF  # 0x8062
lower = TARGET_ERROR & 0xFFFF  # 0x01FE
for i in range(0, len(raw_data) - 4, 4):
    instr = struct.unpack_from("<I", raw_data, i)[0]
    if (instr >> 26) == 0x0F:  # LUI
        imm = instr & 0xFFFF
        if imm == upper:
            error_locs.append(base_addr + i)

log("Found %d locations referencing error code:" % len(error_locs))
for loc in error_locs:
    log("  0x%08X" % loc)
    # Try to find containing function and decompile
    addr = (
        currentProgram.getAddressFactory()
        .getDefaultAddressSpace()
        .getAddress(loc)
    )
    func = getFunctionContaining(addr)
    if func:
        log("    -> in function: %s at 0x%08X" % (func.getName(), func.getEntryPoint().getOffset()))

# Find and decompile all sceVideocodec exports
log("\n" + "=" * 72)
log("sceVideocodec Export Analysis")
log("=" * 72)

# The NID table and address table we parsed manually:
# NIDs at offsets 0x4770-0x47A4 (14 entries)
# Addresses at offsets 0x47A8-0x47E0 (14 entries)
# But these are file offsets, not loaded addresses. Let's find them
# by searching for the NID values in the loaded program.

# Try to find NID 0xC01EC829 in memory
nid_needle = struct.pack("<I", 0xC01EC829)
nid_pos = raw_data.find(nid_needle)
if nid_pos >= 0:
    log("\nFound sceVideocodecOpen NID at offset 0x%X (addr 0x%08X)" % (nid_pos, base_addr + nid_pos))
    # The address table follows the NID table
    # Count backwards to find start of NID table
    # NID 0xC01EC829 is at index 11 in the sorted list
    # The address at the same index gives us the function entry point

# Simpler approach: use the offsets we found from our Python analysis
# NID table: 14 entries starting at file offset 0x4770
# Address table: 14 entries starting at file offset 0x47A8
log("\nParsing export tables from known offsets...")

nid_table_off = 0x4770
addr_table_off = 0x47A8
num_exports = 14

sorted_nids = []
for i in range(num_exports):
    noff = nid_table_off + i * 4
    aoff = addr_table_off + i * 4
    if noff + 4 <= len(raw_data) and aoff + 4 <= len(raw_data):
        nid = struct.unpack_from("<I", raw_data, noff)[0]
        func_offset = struct.unpack_from("<I", raw_data, aoff)[0]
        name = VIDEOCODEC_NIDS.get(nid, "unknown_0x%08X" % nid)
        # func_offset is relative to module base
        func_addr = base_addr + func_offset
        sorted_nids.append((nid, name, func_addr))
        log("  NID 0x%08X = %s @ 0x%08X (offset 0x%04X)" % (nid, name, func_addr, func_offset))

# Decompile the key functions
log("\n" + "=" * 72)
log("Decompilation of sceVideocodecOpen (NID 0xC01EC829)")
log("=" * 72)

for nid, name, func_addr in sorted_nids:
    if nid == 0xC01EC829:
        log("\nFunction %s at 0x%08X:" % (name, func_addr))
        code = decompile_at(func_addr)
        log(code)
        break

log("\n" + "=" * 72)
log("Decompilation of ALL sceVideocodec functions")
log("=" * 72)

for nid, name, func_addr in sorted_nids:
    log("\n--- %s (NID 0x%08X) at 0x%08X ---" % (name, nid, func_addr))
    code = decompile_at(func_addr)
    log(code)

# Also decompile sceAudiocodec exports for comparison
log("\n" + "=" * 72)
log("sceAudiocodec Export Analysis (for comparison)")
log("=" * 72)

ac_nid_table_off = 0x47F0
ac_addr_table_off = 0x4810
ac_num_exports = 8

for i in range(ac_num_exports):
    noff = ac_nid_table_off + i * 4
    aoff = ac_addr_table_off + i * 4
    if noff + 4 <= len(raw_data) and aoff + 4 <= len(raw_data):
        nid = struct.unpack_from("<I", raw_data, noff)[0]
        func_offset = struct.unpack_from("<I", raw_data, aoff)[0]
        name = AUDIOCODEC_NIDS.get(nid, "unknown_0x%08X" % nid)
        func_addr = base_addr + func_offset
        log("\n--- %s (NID 0x%08X) at 0x%08X ---" % (name, nid, func_addr))
        code = decompile_at(func_addr)
        log(code)

log("\n" + "=" * 72)
log("Analysis complete. Output saved to: %s" % OUTPUT)
log("=" * 72)
