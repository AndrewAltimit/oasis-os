# -*- coding: utf-8 -*-
# Focused Ghidra script: decompile sceVideocodecOpen at 0x1224
# and all functions that reference error 0x806201FE

from ghidra.app.decompiler import DecompInterface
from ghidra.util.task import ConsoleTaskMonitor

OUTPUT = "/data/output/avcodec_open.txt"


def log(msg):
    print(msg)
    f = open(OUTPUT, "a")
    f.write(msg + "\n")
    f.close()


def decompile_func(func):
    """Decompile a function object."""
    decomp = DecompInterface()
    decomp.openProgram(currentProgram)
    results = decomp.decompileFunction(func, 120, ConsoleTaskMonitor())
    if results.decompileCompleted():
        return results.getDecompiledFunction().getC()
    return "// Decompilation failed for %s" % func.getName()


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
    return decompile_func(func)


# Clear output
f = open(OUTPUT, "w")
f.write("")
f.close()

log("=" * 72)
log("avcodec.prx — sceVideocodecOpen Decompilation")
log("=" * 72)

# Known function offsets from the NID/address table analysis:
# The export address table for sceVideocodec has 14 entries.
# NID 0xC01EC829 (sceVideocodecOpen) is at index 11 in the sorted NID list.
# From our Python analysis, the address table starts at file offset 0x47A8.
# Entry [11] = offset 0x1224.
#
# All 14 sceVideocodec function offsets (from the address table):
VIDEOCODEC_FUNCS = {
    0x00000DE0: "sceVideocodecInit",        # NID 0x17099F0A
    0x00001224: "sceVideocodecOpen",         # NID 0xC01EC829 <<<
    0x000010B0: "sceVideocodec_17CF7D2C",
    0x00000BF8: "sceVideocodecGetVersion",   # NID 0x26927D19
    0x00000F80: "sceVideocodecGetEDRAM",     # NID 0x2D31F5B1
    0x0000151C: "sceVideocodecScanHeader",   # NID 0x2F385E7F
    0x00001308: "sceVideocodecDelete",       # NID 0x307E6E1C
    0x00001740: "sceVideocodecReleaseEDRAM", # NID 0x4F160BF4
    0x000017E4: "sceVideocodecGetSEI",       # NID 0x627B7D42
    0x000016DC: "sceVideocodecSetMemory",    # NID 0x745A7B7A
    0x00001B24: "sceVideocodec_893B32B1",
    0x00001CD0: "sceVideocodecStop",         # NID 0xA2F0564E
    0x00001434: "sceVideocodec_D95C24D5",
    0x00001594: "sceVideocodecDecode",       # NID 0xDBA273FA
}

# First, decompile sceVideocodecOpen (the critical function)
log("\n" + "-" * 72)
log("sceVideocodecOpen (0x1224) — THE FUNCTION THAT RETURNS 0x806201FE")
log("-" * 72)
code = decompile_at(0x1224)
log(code)

# Then decompile all other sceVideocodec functions
for offset, name in sorted(VIDEOCODEC_FUNCS.items()):
    if offset == 0x1224:
        continue  # already done
    log("\n" + "-" * 72)
    log("%s (0x%04X)" % (name, offset))
    log("-" * 72)
    code = decompile_at(offset)
    log(code)

# Also look for any helper/init functions called by sceVideocodecOpen
# by scanning for CALLs in the 0x1224 function range
log("\n" + "-" * 72)
log("Functions called from sceVideocodecOpen region (0x1224-0x1308)")
log("-" * 72)

import struct
memory = currentProgram.getMemory()
for addr_int in range(0x1224, 0x1308, 4):
    addr = currentProgram.getAddressFactory().getDefaultAddressSpace().getAddress(addr_int)
    try:
        instr_val = memory.getInt(addr)
        # JAL instruction: 0000 11xx xxxx xxxx xxxx xxxx xxxx xxxx
        if (instr_val >> 26) == 0x03:  # JAL
            target = (instr_val & 0x03FFFFFF) << 2
            name = VIDEOCODEC_FUNCS.get(target, "FUN_%08X" % target)
            log("  0x%04X: JAL 0x%08X (%s)" % (addr_int, target, name))
    except:
        pass

log("\n" + "=" * 72)
log("Analysis complete.")
log("=" * 72)
