# Focused Ghidra script: decompile ME submission functions and error table
from ghidra.app.decompiler import DecompInterface
from ghidra.util.task import ConsoleTaskMonitor
import struct

OUTPUT = "/data/output/avcodec_me.txt"

def log(msg):
    print(msg)
    f = open(OUTPUT, "a")
    f.write(msg + "\n")
    f.close()

def decompile_at(addr_int):
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

f = open(OUTPUT, "w")
f.write("")
f.close()

log("=" * 72)
log("avcodec.prx ME Submission Functions + Error Table")
log("=" * 72)

# Dump the error jump table at DAT_000049bc
log("\n--- Error jump table at 0x49BC ---")
memory = currentProgram.getMemory()
for i in range(16):
    addr_int = 0x49BC + i * 4
    try:
        addr = currentProgram.getAddressFactory().getDefaultAddressSpace().getAddress(addr_int)
        val = memory.getInt(addr)
        log("  [%d] 0x%04X: 0x%08X" % (i, addr_int, val & 0xFFFFFFFF))
    except:
        log("  [%d] 0x%04X: (read error)" % (i, addr_int))

# Key functions to decompile
targets = [
    (0x00001c80, "FUN_00001c80 (pre-decode check)"),
    (0x00004414, "FUN_00004414 (ME submit - used by Open)"),
    (0x00004424, "FUN_00004424 (ME submit - used by ScanHeader)"),
    (0x00004434, "FUN_00004434 (ME submit - used by Init)"),
    (0x0000441c, "FUN_0000441c (ME submit - used by D95C24D5)"),
    (0x00004344, "FUN_00004344 (cache flush helper)"),
    (0x00004354, "FUN_00004354 (cache flush helper 2)"),
    (0x000042f4, "FUN_000042f4 (semaphore/lock)"),
    (0x000042e4, "FUN_000042e4 (semaphore/unlock)"),
    (0x000043ec, "FUN_000043ec (EDRAM alloc)"),
    (0x00004394, "FUN_00004394 (ME worker loop body)"),
    (0x0000438c, "FUN_0000438c (ME wait/poll)"),
    (0x00004314, "FUN_00004314 (post-decode cleanup)"),
    (0x00001a4c, "FUN_00001a4c (pre-D95C24D5 setup)"),
]

for addr, name in targets:
    log("\n" + "-" * 72)
    log("%s (0x%04X)" % (name, addr))
    log("-" * 72)
    code = decompile_at(addr)
    log(code)

# Also dump the full ScanHeader and Decode flow disassembly
log("\n" + "-" * 72)
log("Raw bytes around ScanHeader (0x1500-0x15A0)")
log("-" * 72)
for i in range(0, 0xA0, 4):
    addr_int = 0x1500 + i
    try:
        addr = currentProgram.getAddressFactory().getDefaultAddressSpace().getAddress(addr_int)
        val = memory.getInt(addr)
        log("  0x%04X: 0x%08X" % (addr_int, val & 0xFFFFFFFF))
    except:
        pass

log("\n" + "=" * 72)
log("Analysis complete.")
log("=" * 72)
