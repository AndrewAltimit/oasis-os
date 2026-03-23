# Ghidra headless script: analyze sceMpeg_library for sceMpegRingbufferPut
#
# Load the raw MIPS binary (dec_mpeg.bin) at base address 0x08c05c00
# then find and decompile key functions.
#
# Usage:
#   docker run --rm -v /home/mikunpc/Downloads:/data \
#     ghidra-headless /data/ghidra_mpeg_project mpeg_analysis \
#     -import /data/mpeg_decrypted.bin \
#     -processor MIPS:LE:32:default \
#     -baseAddr 0x08c05c00 \
#     -postScript /data/ghidra_mpeg_ringbuffer.py
#
# Or use analyzeHeadless directly.

from ghidra.app.decompiler import DecompInterface
from ghidra.util.task import ConsoleTaskMonitor
from ghidra.program.model.symbol import SourceType

OUTPUT = "/data/output/mpeg_ringbuffer.txt"
BASE = 0x08c05c00

# NID -> function name mapping for sceMpeg_library
NIDS = {
    0xB240A59E: "sceMpegRingbufferPut",
    0x37295ED8: "sceMpegRingbufferConstruct",
    0x13407F13: "sceMpegRingbufferDestruct",
    0xB5F6DC87: "sceMpegRingbufferAvailableSize",
    0xD8C5F121: "sceMpegCreate",
    0x606A4649: "sceMpegDelete",
    0x682A619B: "sceMpegInit",
    0xFE246728: "sceMpegGetAvcAu",
    0x0E3C2E9D: "sceMpegAvcDecode",
    0x21FF80E4: "sceMpegQueryStreamOffset",
    0x611E9E11: "sceMpegQueryStreamSize",
    0x42560F23: "sceMpegRegistStream",
    0xA11C7026: "sceMpegAvcDecodeMode",
    0xA780CF7E: "sceMpegMallocAvcEsBuf",
    0x167AFD9E: "sceMpegInitAu",
    0xC132E22F: "sceMpegQueryMemSize",
    0x874624D6: "sceMpegFinish",
}

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

# Clear output
f = open(OUTPUT, "w")
f.write("")
f.close()

log("=" * 72)
log("sceMpeg_library Analysis (mpeg.prx / dec_mpeg.bin)")
log("Base address: 0x%08X" % BASE)
log("Binary size: %d bytes" % currentProgram.getMemory().getSize())
log("=" * 72)

# Try to find the export table / NID table in the binary.
# PSP PRX modules have an export table that maps NIDs to function addresses.
# For a raw memory dump (not ELF), we need to scan for the NID table.
log("\n--- Scanning for NID table ---")

memory = currentProgram.getMemory()
size = 0
for block in memory.getBlocks():
    size = max(size, block.getEnd().getOffset() - block.getStart().getOffset())

# Scan for known NIDs in the binary (as 32-bit LE values)
import struct

nid_locations = {}
for nid, name in NIDS.items():
    nid_bytes = struct.pack("<I", nid)
    # Search for this NID value in the binary
    addr = memory.findBytes(
        currentProgram.getMinAddress(),
        nid_bytes,
        None,
        True,
        ConsoleTaskMonitor()
    )
    if addr is not None:
        nid_locations[nid] = addr.getOffset()
        log("  Found NID 0x%08X (%s) at offset 0x%04X" % (
            nid, name, addr.getOffset() - BASE))

# If we found NIDs, try to find the corresponding function pointers
# The PSP export table format has NID array followed by function pointer array
if nid_locations:
    log("\n--- Resolving function addresses from export table ---")

    # Find the lowest NID address - the NID table starts there
    nid_addrs = sorted(nid_locations.values())
    nid_table_start = nid_addrs[0]

    # Count how many consecutive NIDs we find
    nid_count = 0
    test_addr = nid_table_start
    while True:
        val_addr = currentProgram.getAddressFactory().getDefaultAddressSpace().getAddress(test_addr)
        try:
            val = memory.getInt(val_addr) & 0xFFFFFFFF
            if val in NIDS:
                nid_count += 1
                test_addr += 4
            else:
                break
        except:
            break

    log("  NID table at 0x%08X, %d entries" % (nid_table_start, nid_count))

    # Function pointer table should be right after NID table
    func_table_start = nid_table_start + nid_count * 4
    log("  Function table at 0x%08X" % func_table_start)

    for i in range(nid_count):
        nid_addr = currentProgram.getAddressFactory().getDefaultAddressSpace().getAddress(
            nid_table_start + i * 4)
        func_addr_ptr = currentProgram.getAddressFactory().getDefaultAddressSpace().getAddress(
            func_table_start + i * 4)

        nid_val = memory.getInt(nid_addr) & 0xFFFFFFFF
        func_val = memory.getInt(func_addr_ptr) & 0xFFFFFFFF

        name = NIDS.get(nid_val, "unknown_0x%08X" % nid_val)
        log("  [%d] NID=0x%08X func=0x%08X %s" % (i, nid_val, func_val, name))

        # Create and label the function
        if func_val >= BASE and func_val < BASE + 0x10000:
            func_entry = currentProgram.getAddressFactory().getDefaultAddressSpace().getAddress(func_val)
            createFunction(func_entry, name)
            try:
                func = getFunctionAt(func_entry)
                if func:
                    func.setName(name, SourceType.USER_DEFINED)
            except:
                pass

# Decompile key functions
log("\n" + "=" * 72)
log("Decompiling key functions")
log("=" * 72)

# Priority targets for understanding the ringbuffer freeze
priority_funcs = [
    "sceMpegRingbufferPut",
    "sceMpegRingbufferConstruct",
    "sceMpegGetAvcAu",
    "sceMpegCreate",
    "sceMpegAvcDecode",
]

# Try to decompile by scanning for labeled functions
listing = currentProgram.getListing()
func_iter = listing.getFunctions(True)
decompiled = set()

while func_iter.hasNext():
    func = func_iter.next()
    name = func.getName()
    if name in priority_funcs:
        log("\n" + "-" * 72)
        log("%s (0x%08X)" % (name, func.getEntryPoint().getOffset()))
        log("-" * 72)
        code = decompile_at(func.getEntryPoint().getOffset())
        log(code)
        decompiled.add(name)

# If we didn't find labeled functions, try all functions
if len(decompiled) < len(priority_funcs):
    log("\n--- Some functions not found by label, dumping all functions ---")
    func_iter = listing.getFunctions(True)
    func_list = []
    while func_iter.hasNext():
        func = func_iter.next()
        func_list.append((func.getEntryPoint().getOffset(), func.getName(),
                         func.getBody().getNumAddresses()))

    for addr, name, size in sorted(func_list):
        if size > 20:  # Skip tiny stubs
            log("  0x%08X  %-30s  %d bytes" % (addr, name, size))

log("\n" + "=" * 72)
log("Analysis complete.")
log("=" * 72)
