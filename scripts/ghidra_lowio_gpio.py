# -*- coding: utf-8 -*-
# Ghidra headless analysis script for decrypted lowio.prx (PSP 6.61)
#
# Analyzes the GPIO driver implementation to understand:
#   1. How sceGpioPortSet propagates to the Output register
#   2. What output enable mechanism exists (AltFunc, sceSysreg, etc.)
#   3. TA-090v2 specific code paths (Tachyon version checks)
#   4. Any GPIO output enable registers in BC100xxx range
#
# Usage (Ghidra headless):
#   analyzeHeadless /path/to/project lowio_prx \
#     -import /home/mikunpc/Downloads/USBTRACE/dec_661/lowio.prx \
#     -processor MIPS:LE:32:default -cspec default \
#     -postScript ghidra_lowio_gpio.py
#
# Or run from Ghidra GUI: Script Manager → Run

from ghidra.app.decompiler import DecompInterface
from ghidra.util.task import ConsoleTaskMonitor
import struct

OUTPUT = "/home/mikunpc/Downloads/USBTRACE/ghidra_lowio_gpio_results.txt"


def log(msg):
    print(msg)
    f = open(OUTPUT, "a")
    f.write(msg + "\n")
    f.close()


def decompile_at(addr_int):
    """Decompile function at given address."""
    addr = currentProgram.getAddressFactory().getDefaultAddressSpace().getAddress(addr_int)
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


# GPIO NID table for lowio.prx exports
GPIO_EXPORT_NIDS = {
    0x4250D44A: "sceGpioPortRead",
    0x310F0CCF: "sceGpioPortSet",
    0x103C3EB2: "sceGpioPortClear",
    0xFBC85E74: "sceGpioSetPortMode",
    0x317D9D2C: "sceGpioSetPortMode2",
    0xC6928224: "sceGpioGetPortMode",
    0x37C8DADC: "sceGpioPortInvert",
    0x95D7F3B8: "sceGpioEnableTimerCapture",
    0x31F34AE6: "sceGpioDisableTimerCapture",
    0x6B19C009: "sceGpioSetIntrMode",
    0xF4524F3C: "sceGpioGetCapturePort",
    0xEEB06022: "sceGpioAcquireIntr",
    0x5691CEFA: "sceGpioQueryIntr",
    0xE4E9985C: "sceGpioEnableIntr",
    0xBBCEAEC8: "sceGpioDisableIntr",
}

# Known sceSysreg NIDs that lowio.prx might import
SYSREG_NIDS = {
    0x72C1CA96: "sceSysregGpioIoEnable",
    0xEC03F6E2: "sceSysregGpioClkEnable",
    0x2112E686: "sceSysregGpioIoDisable",
    0x49A24FF2: "sceSysregGpioClkDisable",
}


# Clear output
f = open(OUTPUT, "w")
f.write("")
f.close()

log("=" * 70)
log("PSP lowio.prx GPIO Driver Analysis")
log("=" * 70)

addr_space = currentProgram.getAddressFactory().getDefaultAddressSpace()
mem = currentProgram.getMemory()

# Get program bounds
min_addr = currentProgram.getMinAddress()
max_addr = currentProgram.getMaxAddress()
base = min_addr.getOffset()
size = max_addr.getOffset() - base + 1

log("\nProgram base: 0x%08X" % base)
log("Program size: 0x%X (%d bytes)" % (size, size))

# ── Step 1: Memory blocks ─────────────────────────────────────────────

log("\nMemory blocks:")
for block in mem.getBlocks():
    log("  %s: 0x%08X - 0x%08X (size 0x%X)" % (
        block.getName(),
        block.getStart().getOffset(),
        block.getEnd().getOffset(),
        block.getSize(),
    ))

# ── Step 2: Read all bytes ─────────────────────────────────────────────

code_bytes = bytearray(int(size))
try:
    mem.getBytes(min_addr, code_bytes)
except:
    for block in mem.getBlocks():
        try:
            block_size = block.getSize()
            block_bytes = bytearray(int(block_size))
            mem.getBytes(block.getStart(), block_bytes)
            offset = block.getStart().getOffset() - base
            for i in range(int(block_size)):
                if offset + i < len(code_bytes):
                    code_bytes[int(offset + i)] = block_bytes[i]
        except:
            pass

# ── Step 3: Find GPIO register accesses ────────────────────────────────

log("\n" + "=" * 70)
log("STEP 1: GPIO REGISTER ACCESS SCAN (LUI 0xBE24)")
log("=" * 70)

gpio_funcs = set()
gpio_details = []

for i in range(0, len(code_bytes) - 4, 4):
    word = struct.unpack_from('<I', bytes(code_bytes), i)[0]
    opcode = (word >> 26) & 0x3F
    if opcode == 0x0F:  # LUI
        rt = (word >> 16) & 0x1F
        imm = word & 0xFFFF
        if imm == 0xBE24:
            addr_val = base + i
            func = getFunctionContaining(addr_space.getAddress(addr_val))
            fname = func.getName() if func else "?"
            gpio_details.append((addr_val, rt, fname))
            if func:
                gpio_funcs.add(func.getEntryPoint().getOffset())

            # Look at following instructions for offset (ADDIU/ORI/SW/LW)
            if i + 4 < len(code_bytes):
                next_word = struct.unpack_from('<I', bytes(code_bytes), i + 4)[0]
                next_op = (next_word >> 26) & 0x3F
                next_imm = next_word & 0xFFFF
                if next_imm > 0x7FFF:
                    next_imm = next_imm - 0x10000  # sign extend
                offset_str = "+0x%04X" % (next_imm & 0xFFFF) if next_op in [0x09, 0x0D, 0x23, 0x2B] else ""
                log("  0x%08X: LUI $%d, 0xBE24 %s in %s" % (
                    addr_val, rt, offset_str, fname))
            else:
                log("  0x%08X: LUI $%d, 0xBE24 in %s" % (addr_val, rt, fname))

log("\nTotal GPIO accesses: %d in %d functions" % (
    len(gpio_details), len(gpio_funcs)))

# ── Step 4: Find BC100xxx accesses (sceSysreg) ────────────────────────

log("\n" + "=" * 70)
log("STEP 2: sceSysreg REGISTER SCAN (LUI 0xBC10)")
log("=" * 70)

bc10_funcs = set()
for i in range(0, len(code_bytes) - 4, 4):
    word = struct.unpack_from('<I', bytes(code_bytes), i)[0]
    opcode = (word >> 26) & 0x3F
    if opcode == 0x0F:  # LUI
        imm = word & 0xFFFF
        rt = (word >> 16) & 0x1F
        if imm == 0xBC10:
            addr_val = base + i
            func = getFunctionContaining(addr_space.getAddress(addr_val))
            fname = func.getName() if func else "?"
            if func:
                bc10_funcs.add(func.getEntryPoint().getOffset())

            if i + 4 < len(code_bytes):
                next_word = struct.unpack_from('<I', bytes(code_bytes), i + 4)[0]
                next_imm = next_word & 0xFFFF
                log("  0x%08X: LUI $%d, 0xBC10 +0x%04X in %s" % (
                    addr_val, rt, next_imm, fname))
            else:
                log("  0x%08X: LUI $%d, 0xBC10 in %s" % (addr_val, rt, fname))

# ── Step 5: Scan for Tachyon version checks ────────────────────────────

log("\n" + "=" * 70)
log("STEP 3: TACHYON VERSION CHECKS (0xBC100040)")
log("=" * 70)

# Tachyon version is read from BC100040 and compared against known values
# PSP-3001 TA-090v2 = 0x82000002
# Look for comparisons with 0x0050, 0x0090, 0x8200, etc.
tachyon_checks = []
for i in range(0, len(code_bytes) - 4, 4):
    word = struct.unpack_from('<I', bytes(code_bytes), i)[0]
    opcode = (word >> 26) & 0x3F
    # SLTIU/SLTI for version comparisons
    if opcode in [0x0A, 0x0B]:  # SLTI, SLTIU
        imm = word & 0xFFFF
        if imm in [0x0050, 0x0051, 0x0090, 0x0091, 0x0500, 0x0900, 0x8200]:
            addr_val = base + i
            func = getFunctionContaining(addr_space.getAddress(addr_val))
            fname = func.getName() if func else "?"
            tachyon_checks.append((addr_val, imm, fname))
            log("  0x%08X: version check imm=0x%04X in %s" % (
                addr_val, imm, fname))

# ── Step 6: Find NID export table ──────────────────────────────────────

log("\n" + "=" * 70)
log("STEP 4: NID EXPORT TABLE SCAN")
log("=" * 70)

for nid, name in sorted(GPIO_EXPORT_NIDS.items(), key=lambda x: x[1]):
    nid_bytes = struct.pack('<I', nid)
    pos = 0
    while True:
        idx = bytes(code_bytes).find(nid_bytes, pos)
        if idx == -1:
            break
        data_addr = base + idx
        log("  NID 0x%08X (%s) at offset 0x%08X" % (nid, name, data_addr))

        # The stub table entry (function pointer) is typically at a parallel
        # offset in another table. Show surrounding context.
        context_start = max(0, idx - 8)
        context_end = min(len(code_bytes), idx + 40)
        for j in range(context_start, context_end, 4):
            if j + 4 <= len(code_bytes):
                val = struct.unpack_from('<I', bytes(code_bytes), j)[0]
                marker = " <-- NID" if j == idx else ""
                log("    0x%08X: 0x%08X%s" % (base + j, val, marker))
        pos = idx + 4

# ── Step 7: Decompile ALL GPIO functions ───────────────────────────────

log("\n" + "=" * 70)
log("STEP 5: DECOMPILE ALL GPIO-ACCESSING FUNCTIONS")
log("=" * 70)

for faddr in sorted(gpio_funcs):
    func = getFunctionAt(addr_space.getAddress(faddr))
    fname = func.getName() if func else "unknown"
    log("\n" + "-" * 70)
    log("  %s @ 0x%08X" % (fname, faddr))
    log("-" * 70)
    try:
        code = decompile_at(faddr)
        log(code)
    except Exception as e:
        log("// Error: %s" % str(e))

# ── Step 8: Decompile BC10xxxx functions ───────────────────────────────

log("\n" + "=" * 70)
log("STEP 6: DECOMPILE sceSysreg FUNCTIONS (BC10xxxx)")
log("=" * 70)

for faddr in sorted(bc10_funcs):
    if faddr in gpio_funcs:
        continue
    func = getFunctionAt(addr_space.getAddress(faddr))
    fname = func.getName() if func else "unknown"
    log("\n" + "-" * 70)
    log("  %s @ 0x%08X" % (fname, faddr))
    log("-" * 70)
    try:
        code = decompile_at(faddr)
        log(code)
    except Exception as e:
        log("// Error: %s" % str(e))

# ── Step 9: List ALL functions ─────────────────────────────────────────

log("\n" + "=" * 70)
log("STEP 7: ALL FUNCTIONS IN lowio.prx")
log("=" * 70)

func_iter = currentProgram.getFunctionManager().getFunctions(True)
func_list = []
while func_iter.hasNext():
    func = func_iter.next()
    func_list.append((func.getEntryPoint().getOffset(), func.getName(),
                       func.getBody().getNumAddresses()))

log("\nTotal functions: %d" % len(func_list))
for faddr, fname, fsize in sorted(func_list):
    log("  0x%08X: %s (size=%d)" % (faddr, fname, fsize))

# ── Step 10: Find the Set→Output propagation ──────────────────────────

log("\n" + "=" * 70)
log("STEP 8: SET→OUTPUT PROPAGATION ANALYSIS")
log("=" * 70)

# Look for functions that:
# 1. Read from +0x14 (Set register) or write to +0x14
# 2. Then write to +0x08 (Output register)
# These offsets relative to 0xBE240000

# Also look for interrupt/DMA-style register updates
# The GPIO Set register (+0x14) is write-1-to-set, meaning writes
# should OR into the Output register (+0x08). If this doesn't happen,
# there's a gate.

# Look for any reference to offset 0x44 or 0x48 (AltFunc registers)
log("\nSearching for AltFunc register references (0xBE240040/48)...")
for i in range(0, len(code_bytes) - 4, 4):
    word = struct.unpack_from('<I', bytes(code_bytes), i)[0]
    opcode = (word >> 26) & 0x3F
    if opcode in [0x09, 0x0D, 0x23, 0x2B]:  # ADDIU, ORI, LW, SW
        imm = word & 0xFFFF
        if imm in [0x0040, 0x0044, 0x0048]:
            addr_val = base + i
            func = getFunctionContaining(addr_space.getAddress(addr_val))
            fname = func.getName() if func else "?"
            log("  0x%08X: offset 0x%04X access in %s" % (
                addr_val, imm, fname))

# ── Summary ────────────────────────────────────────────────────────────

log("\n" + "=" * 70)
log("SUMMARY")
log("=" * 70)
log("\nGPIO (0xBE24) accessing functions: %d" % len(gpio_funcs))
log("BC10 (sceSysreg) accessing functions: %d" % len(bc10_funcs))
log("Tachyon version checks: %d" % len(tachyon_checks))
log("Total functions in lowio.prx: %d" % len(func_list))

log("\n" + "=" * 70)
log("Analysis complete!")
log("Results saved to: %s" % OUTPUT)
log("=" * 70)
