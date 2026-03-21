# -*- coding: utf-8 -*-
# Ghidra headless analysis script for decrypted usb.prx (PSP 6.61)
#
# Traces the complete USB host VBUS init chain:
#   1. Parse ELF import stubs → NID→function mappings
#   2. Map sceSysreg_driver, sceSyscon_driver, scePower_driver NIDs
#   3. Find GPIO 0xBE24xxxx register accesses (pin 23 = 0x00800000)
#   4. Trace backwards from GPIO callers to build the full init chain
#   5. Output complete call sequence: entry → ... → GPIO_set
#
# Usage (Ghidra headless):
#   analyzeHeadless /path/to/project usb_prx \
#     -import /home/mikunpc/Downloads/USBTRACE/dec_661/usb.prx \
#     -processor MIPS:LE:32:default -cspec default \
#     -postScript ghidra_usb_vbus.py
#
# Or run from Ghidra GUI: Script Manager → Run

from ghidra.app.decompiler import DecompInterface
from ghidra.util.task import ConsoleTaskMonitor
import struct

OUTPUT = "/home/mikunpc/Downloads/USBTRACE/ghidra_usb_vbus_results.txt"

# ── Known NID Databases ─────────────────────────────────────────────────

# sceSysreg_driver NIDs (14 imports from usb.prx)
# Sources: PPSSPP HLE, pspdev NID database, uOFW
SYSREG_NIDS = {
    0x1561BCD2: "sceSysregUsbClkEnable",
    0x1D233EF9: "sceSysregUsbClkDisable",       # or sceSysregGetTachyonVersion
    0x30C0A141: "sceSysregUsbQueryIntr",
    0x6C0EE043: "sceSysregUsbAcquireIntr",
    0x6F3B6D7D: "sceSysregUsbResetDisable",      # or sceSysregSetMasterPriv
    0x72C1CA96: "sceSysregGpioIoEnable",
    0x84A279A4: "sceSysregUsbResetEnable",
    0x87B61303: "sceSysregUsbGetConnectStatus",
    0x9275DD37: "sceSysregUsbSetConnectStatus",
    0x9306F27B: "sceSysregUsbIoEnable",
    0x9A6E7BB8: "sceSysregUsbBusClockEnable",
    0xD7AD9705: "sceSysregUsbBusClockDisable",
    0xE2A5D1EE: "sceSysregUsbIoDisable",
    0xEC03F6E2: "sceSysregGpioClkEnable",
}

# sceSyscon_driver NIDs (6 imports from usb.prx)
SYSCON_NIDS = {
    0xC8D97773: "sceSysconCtrlUsbPower",
    0x23093E69: "sceSysconGetUsbPowerStatus",     # or sceSysconCtrlHRPower
    0xFB148FB6: "sceSysconSetUSBStatus",
    0x5B9ACC97: "sceSysconGetUSBStatus",
    0x4AB44BFC: "sceSysconCtrlCharge",
    0x3E3B0D30: "sceSysconBatteryGetElec",        # or sceSysconGetBaryonVersion
}

# scePower_driver NIDs (3 imports from usb.prx)
POWER_NIDS = {
    0xD3075926: "scePowerGetBatteryLifePercent",  # or scePowerSetUsbEnabled
    0x0442D852: "scePowerRequestColdReset",       # or scePowerRegisterCallback
    0x2875994B: "scePowerBatteryUpdatePhase",     # or scePower_driver_2875994B
}

# sceGpio_driver NIDs (referenced by usb.prx VBUS enable code)
GPIO_NIDS = {
    0x4250D44A: "sceGpioPortRead",
    0x310F0CCF: "sceGpioPortSet",
    0x103C3EB2: "sceGpioPortClear",
    0xFBC85E74: "sceGpioSetPortMode",
    0x317D9D2C: "sceGpioSetPortMode2",  # alternate mode function for VBUS
}

# Combine all for lookup
ALL_NIDS = {}
ALL_NIDS.update(SYSREG_NIDS)
ALL_NIDS.update(SYSCON_NIDS)
ALL_NIDS.update(POWER_NIDS)
ALL_NIDS.update(GPIO_NIDS)


def log(msg):
    print(msg)
    f = open(OUTPUT, "a")
    f.write(msg + "\n")
    f.close()


def decompile_at(addr_int):
    """Decompile function at given address. Creates function if needed."""
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

log("=" * 70)
log("PSP USB VBUS Init Chain — Deep Analysis of usb.prx")
log("=" * 70)

addr_space = currentProgram.getAddressFactory().getDefaultAddressSpace()
mem = currentProgram.getMemory()
listing = currentProgram.getListing()

# ── Step 1: Determine base address ──────────────────────────────────────

# Get the minimum address of the loaded program
min_addr = currentProgram.getMinAddress()
max_addr = currentProgram.getMaxAddress()
base = min_addr.getOffset()
size = max_addr.getOffset() - base + 1

log("\nProgram base: 0x%08X" % base)
log("Program size: 0x%X (%d bytes)" % (size, size))

# ── Step 2: Find import stub tables ────────────────────────────────────

log("\n" + "=" * 70)
log("STEP 1: ELF IMPORT STUB ANALYSIS")
log("=" * 70)

# PSP PRX import stubs are in .lib.stub section
# Each entry: {name_ptr, flags, stub_count, nid_table_ptr, stub_table_ptr}
# Stubs are 8-byte MIPS jump sequences: jr $ra; syscall NID

# List all memory blocks to find sections
log("\nMemory blocks:")
for block in mem.getBlocks():
    log("  %s: 0x%08X - 0x%08X (size 0x%X)" % (
        block.getName(),
        block.getStart().getOffset(),
        block.getEnd().getOffset(),
        block.getSize(),
    ))

# ── Step 3: Scan for NID values in binary ──────────────────────────────

log("\n" + "=" * 70)
log("STEP 2: NID SCAN — Finding known NIDs in binary")
log("=" * 70)

# Read all program bytes
code_bytes = bytearray(int(size))
try:
    mem.getBytes(min_addr, code_bytes)
except:
    log("WARNING: Could not read all bytes, trying block-by-block")
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

# Search for each known NID as a 32-bit LE word
nid_locations = {}  # nid -> [offset, ...]
for nid, name in ALL_NIDS.items():
    nid_bytes = struct.pack('<I', nid)
    pos = 0
    locs = []
    while True:
        idx = bytes(code_bytes).find(nid_bytes, pos)
        if idx == -1:
            break
        locs.append(base + idx)
        pos = idx + 4
    if locs:
        nid_locations[nid] = locs
        log("  NID 0x%08X (%s) found at: %s" % (
            nid, name,
            ", ".join("0x%08X" % a for a in locs)))

# ── Step 4: Scan for GPIO register accesses (LUI 0xBE24) ──────────────

log("\n" + "=" * 70)
log("STEP 3: GPIO REGISTER ACCESS SCAN")
log("=" * 70)

gpio_accesses = []
be4c_accesses = []
bc10_accesses = []
bd10_accesses = []
be50_accesses = []

for i in range(0, len(code_bytes) - 4, 4):
    word = struct.unpack_from('<I', bytes(code_bytes), i)[0]
    opcode = (word >> 26) & 0x3F

    if opcode == 0x0F:  # LUI instruction
        rt = (word >> 16) & 0x1F
        imm = word & 0xFFFF
        addr_val = base + i

        if imm == 0xBE24:  # GPIO base
            func = getFunctionContaining(addr_space.getAddress(addr_val))
            fname = func.getName() if func else "?"
            gpio_accesses.append((addr_val, rt, fname))
        elif imm == 0xBE4C:  # USB PHY base
            func = getFunctionContaining(addr_space.getAddress(addr_val))
            fname = func.getName() if func else "?"
            be4c_accesses.append((addr_val, rt, fname))
        elif imm == 0xBC10:  # System control
            func = getFunctionContaining(addr_space.getAddress(addr_val))
            fname = func.getName() if func else "?"
            bc10_accesses.append((addr_val, rt, fname))
        elif imm == 0xBD10:  # OHCI controller
            func = getFunctionContaining(addr_space.getAddress(addr_val))
            fname = func.getName() if func else "?"
            bd10_accesses.append((addr_val, rt, fname))
        elif imm == 0xBE50:  # Unknown peripheral (20 refs in kernel)
            func = getFunctionContaining(addr_space.getAddress(addr_val))
            fname = func.getName() if func else "?"
            be50_accesses.append((addr_val, rt, fname))

log("\nGPIO (0xBE24xxxx) accesses: %d" % len(gpio_accesses))
for addr_val, rt, fname in gpio_accesses:
    log("  0x%08X: LUI $%d, 0xBE24 in %s" % (addr_val, rt, fname))

log("\nUSB PHY (0xBE4Cxxxx) accesses: %d" % len(be4c_accesses))
for addr_val, rt, fname in be4c_accesses:
    log("  0x%08X: LUI $%d, 0xBE4C in %s" % (addr_val, rt, fname))

log("\nSystem Control (0xBC10xxxx) accesses: %d" % len(bc10_accesses))
for addr_val, rt, fname in bc10_accesses:
    log("  0x%08X: LUI $%d, 0xBC10 in %s" % (addr_val, rt, fname))

log("\nOHCI (0xBD10xxxx) accesses: %d" % len(bd10_accesses))
for addr_val, rt, fname in bd10_accesses:
    log("  0x%08X: LUI $%d, 0xBD10 in %s" % (addr_val, rt, fname))

log("\nUnknown 0xBE50xxxx accesses: %d" % len(be50_accesses))
for addr_val, rt, fname in be50_accesses:
    log("  0x%08X: LUI $%d, 0xBE50 in %s" % (addr_val, rt, fname))

# ── Step 5: Find the VBUS pin 23 constant (0x00800000) ─────────────────

log("\n" + "=" * 70)
log("STEP 4: VBUS PIN 23 CONSTANT SCAN (0x00800000)")
log("=" * 70)

# Search for LUI loading 0x0080 (upper half of 0x00800000)
vbus_refs = []
for i in range(0, len(code_bytes) - 4, 4):
    word = struct.unpack_from('<I', bytes(code_bytes), i)[0]
    opcode = (word >> 26) & 0x3F
    if opcode == 0x0F:  # LUI
        imm = word & 0xFFFF
        if imm == 0x0080:
            addr_val = base + i
            func = getFunctionContaining(addr_space.getAddress(addr_val))
            fname = func.getName() if func else "?"
            vbus_refs.append((addr_val, fname))
            log("  0x%08X: LUI $r, 0x0080 in %s" % (addr_val, fname))

# Also search for ORI/ADDIU with 0x0000 after LUI 0x0080 (forming 0x00800000)
# And ADDIU $r, $0, 23 (pin number as immediate)
pin23_refs = []
for i in range(0, len(code_bytes) - 4, 4):
    word = struct.unpack_from('<I', bytes(code_bytes), i)[0]
    opcode = (word >> 26) & 0x3F
    if opcode == 0x09:  # ADDIU
        rs = (word >> 21) & 0x1F
        rt = (word >> 16) & 0x1F
        imm = word & 0xFFFF
        if imm == 23 and rs == 0:  # ADDIU $rt, $zero, 23
            addr_val = base + i
            func = getFunctionContaining(addr_space.getAddress(addr_val))
            fname = func.getName() if func else "?"
            pin23_refs.append((addr_val, rt, fname))
            log("  0x%08X: ADDIU $%d, $zero, 23 in %s" % (addr_val, rt, fname))
    elif opcode == 0x0D:  # ORI
        rs = (word >> 21) & 0x1F
        rt = (word >> 16) & 0x1F
        imm = word & 0xFFFF
        if imm == 0x17:  # ORI $rt, $rs, 0x17 (23 decimal)
            addr_val = base + i
            func = getFunctionContaining(addr_space.getAddress(addr_val))
            fname = func.getName() if func else "?"

# ── Step 6: Decompile ALL functions that access GPIO ───────────────────

log("\n" + "=" * 70)
log("STEP 5: DECOMPILE GPIO-ACCESSING FUNCTIONS")
log("=" * 70)

# Collect unique function entry points that access GPIO
gpio_funcs = set()
for addr_val, rt, fname in gpio_accesses:
    func = getFunctionContaining(addr_space.getAddress(addr_val))
    if func:
        gpio_funcs.add(func.getEntryPoint().getOffset())

for faddr in sorted(gpio_funcs):
    func = getFunctionAt(addr_space.getAddress(faddr))
    fname = func.getName() if func else "unknown"
    log("\n" + "-" * 70)
    log("  %s @ 0x%08X (GPIO accessor)" % (fname, faddr))
    log("-" * 70)
    try:
        code = decompile_at(faddr)
        log(code)
    except Exception as e:
        log("// Error: %s" % str(e))

# ── Step 7: Find callers of GPIO functions (VBUS enable chain) ─────────

log("\n" + "=" * 70)
log("STEP 6: CALLERS OF GPIO FUNCTIONS (VBUS init chain)")
log("=" * 70)

caller_funcs = set()
for faddr in sorted(gpio_funcs):
    refs = getReferencesTo(addr_space.getAddress(faddr))
    log("\nCallers of 0x%08X:" % faddr)
    for ref in refs:
        from_addr = ref.getFromAddress().getOffset()
        caller = getFunctionContaining(ref.getFromAddress())
        cname = caller.getName() if caller else "?"
        log("  0x%08X in %s (%s)" % (from_addr, cname, ref.getReferenceType()))
        if caller:
            caller_funcs.add(caller.getEntryPoint().getOffset())

# Decompile callers
for faddr in sorted(caller_funcs):
    if faddr in gpio_funcs:
        continue  # Already decompiled above
    func = getFunctionAt(addr_space.getAddress(faddr))
    fname = func.getName() if func else "unknown"
    log("\n" + "-" * 70)
    log("  %s @ 0x%08X (GPIO caller)" % (fname, faddr))
    log("-" * 70)
    try:
        code = decompile_at(faddr)
        log(code)
    except Exception as e:
        log("// Error: %s" % str(e))

# ── Step 8: 2nd-level callers (who calls the VBUS enable?) ─────────────

log("\n" + "=" * 70)
log("STEP 7: 2ND-LEVEL CALLERS (full init chain)")
log("=" * 70)

second_level = set()
for faddr in sorted(caller_funcs):
    refs = getReferencesTo(addr_space.getAddress(faddr))
    for ref in refs:
        caller = getFunctionContaining(ref.getFromAddress())
        if caller:
            caddr = caller.getEntryPoint().getOffset()
            if caddr not in gpio_funcs and caddr not in caller_funcs:
                second_level.add(caddr)
                from_addr = ref.getFromAddress().getOffset()
                log("  0x%08X calls 0x%08X from %s" % (
                    from_addr, faddr, caller.getName()))

for faddr in sorted(second_level):
    func = getFunctionAt(addr_space.getAddress(faddr))
    fname = func.getName() if func else "unknown"
    log("\n" + "-" * 70)
    log("  %s @ 0x%08X (2nd-level caller)" % (fname, faddr))
    log("-" * 70)
    try:
        code = decompile_at(faddr)
        log(code)
    except Exception as e:
        log("// Error: %s" % str(e))

# ── Step 9: Find all import stubs (JAL targets outside program) ────────

log("\n" + "=" * 70)
log("STEP 8: IMPORT STUB ANALYSIS")
log("=" * 70)

# In relocated PRX, import stubs are typically at known offsets.
# PSP stubs use: j <addr>; nop  or  jr $ra; syscall <nid>
# Find all JAL/J targets to identify external calls

jal_targets = {}  # target_addr -> [caller_addr, ...]
for i in range(0, len(code_bytes) - 4, 4):
    word = struct.unpack_from('<I', bytes(code_bytes), i)[0]
    opcode = (word >> 26) & 0x3F
    if opcode == 0x03:  # JAL
        target = (word & 0x03FFFFFF) << 2
        # Adjust target relative to program base region
        caller_addr = base + i
        if target not in jal_targets:
            jal_targets[target] = []
        jal_targets[target].append(caller_addr)

log("\nJAL targets (potential import stubs or internal functions):")
# Show targets that are NOT within our program range (external/import stubs)
for target in sorted(jal_targets.keys()):
    callers = jal_targets[target]
    # Check if target is within our loaded program
    target_in_program = (base <= target < base + size)
    if not target_in_program:
        log("  JAL 0x%08X (EXTERNAL) called from %d locations" % (
            target, len(callers)))

# ── Step 10: Analyze all functions in the program ──────────────────────

log("\n" + "=" * 70)
log("STEP 9: ALL FUNCTIONS IN usb.prx")
log("=" * 70)

func_iter = currentProgram.getFunctionManager().getFunctions(True)
func_list = []
while func_iter.hasNext():
    func = func_iter.next()
    func_list.append((func.getEntryPoint().getOffset(), func.getName(),
                       func.getBody().getNumAddresses()))

log("\nTotal functions found: %d" % len(func_list))
for faddr, fname, fsize in sorted(func_list):
    log("  0x%08X: %s (size=%d)" % (faddr, fname, fsize))

# ── Step 11: Decompile functions near VBUS addresses ───────────────────

log("\n" + "=" * 70)
log("STEP 10: DECOMPILE KEY VBUS-RELATED ADDRESSES")
log("=" * 70)

# From prior analysis: VBUS enable at virtual offset 0x008C0C, disable at 0x008BD0
# These are relative to the module base address
# Try decompiling at base + offset
vbus_candidates = [
    (base + 0x008BD0, "vbus_disable_candidate"),
    (base + 0x008C0C, "vbus_enable_candidate"),
    # Also try nearby addresses in case of relocation offset differences
    (base + 0x008B00, "vbus_area_start"),
    (base + 0x008C80, "vbus_area_end"),
]

for addr_int, name in vbus_candidates:
    log("\n" + "-" * 70)
    log("  %s @ 0x%08X" % (name, addr_int))
    log("-" * 70)
    try:
        # Check if there's a function here or nearby
        addr = addr_space.getAddress(addr_int)
        func = getFunctionAt(addr)
        if func is None:
            func = getFunctionContaining(addr)
        if func:
            log("  -> In function %s @ 0x%08X" % (
                func.getName(), func.getEntryPoint().getOffset()))
            code = decompile_at(func.getEntryPoint().getOffset())
            log(code)
        else:
            # Try creating function
            code = decompile_at(addr_int)
            log(code)
    except Exception as e:
        log("// Error: %s" % str(e))

# ── Step 12: Scan for function pointer tables (vtables) ────────────────

log("\n" + "=" * 70)
log("STEP 11: FUNCTION POINTER TABLE SCAN")
log("=" * 70)

# VBUS enable/disable are called via function pointers from usb.prx
# Look for data regions containing addresses of GPIO-accessing functions
log("\nSearching for function pointer tables containing GPIO function addrs...")
for gpio_faddr in sorted(gpio_funcs):
    faddr_bytes = struct.pack('<I', gpio_faddr)
    pos = 0
    while True:
        idx = bytes(code_bytes).find(faddr_bytes, pos)
        if idx == -1:
            break
        data_addr = base + idx
        log("  Pointer to 0x%08X found at data offset 0x%08X" % (
            gpio_faddr, data_addr))
        # Show surrounding context (potential vtable)
        context_start = max(0, idx - 16)
        context_end = min(len(code_bytes), idx + 32)
        context = ""
        for j in range(context_start, context_end, 4):
            if j + 4 <= len(code_bytes):
                val = struct.unpack_from('<I', bytes(code_bytes), j)[0]
                marker = " <--" if j == idx else ""
                context += "  0x%08X: 0x%08X%s\n" % (base + j, val, marker)
        log(context)
        pos = idx + 4

# ── Step 13: Scan for sceSysreg register writes ───────────────────────

log("\n" + "=" * 70)
log("STEP 12: sceSysreg REGISTER WRITE SCAN (BC100xxx)")
log("=" * 70)

# Decompile all functions that access BC10xxxx
bc10_funcs = set()
for addr_val, rt, fname in bc10_accesses:
    func = getFunctionContaining(addr_space.getAddress(addr_val))
    if func:
        bc10_funcs.add(func.getEntryPoint().getOffset())

for faddr in sorted(bc10_funcs):
    func = getFunctionAt(addr_space.getAddress(faddr))
    fname = func.getName() if func else "unknown"
    log("\n" + "-" * 70)
    log("  %s @ 0x%08X (BC10xxxx accessor)" % (fname, faddr))
    log("-" * 70)
    try:
        code = decompile_at(faddr)
        log(code)
    except Exception as e:
        log("// Error: %s" % str(e))

# ── Step 14: Entry point analysis ──────────────────────────────────────

log("\n" + "=" * 70)
log("STEP 13: MODULE ENTRY POINT")
log("=" * 70)

# PSP module entry (module_start)
entry_addr = currentProgram.getSymbolTable().getExternalEntryPointIterator()
log("\nExternal entry points:")
while entry_addr.hasNext():
    addr = entry_addr.next()
    log("  0x%08X" % addr.getOffset())
    try:
        code = decompile_at(addr.getOffset())
        log(code)
    except Exception as e:
        log("// Error: %s" % str(e))

# Also try the first function in the binary
if func_list:
    first_func = sorted(func_list)[0]
    log("\n" + "-" * 70)
    log("  First function: %s @ 0x%08X" % (first_func[1], first_func[0]))
    log("-" * 70)
    try:
        code = decompile_at(first_func[0])
        log(code)
    except Exception as e:
        log("// Error: %s" % str(e))

# ── Summary ────────────────────────────────────────────────────────────

log("\n" + "=" * 70)
log("SUMMARY")
log("=" * 70)

log("\nNID Mapping Results:")
for nid in sorted(SYSREG_NIDS.keys()):
    name = SYSREG_NIDS[nid]
    found = "FOUND" if nid in nid_locations else "not found"
    log("  sceSysreg: 0x%08X = %s [%s]" % (nid, name, found))

log("")
for nid in sorted(SYSCON_NIDS.keys()):
    name = SYSCON_NIDS[nid]
    found = "FOUND" if nid in nid_locations else "not found"
    log("  sceSyscon: 0x%08X = %s [%s]" % (nid, name, found))

log("")
for nid in sorted(POWER_NIDS.keys()):
    name = POWER_NIDS[nid]
    found = "FOUND" if nid in nid_locations else "not found"
    log("  scePower:  0x%08X = %s [%s]" % (nid, name, found))

log("\nGPIO Access Functions: %d" % len(gpio_funcs))
log("GPIO Caller Functions: %d" % len(caller_funcs))
log("2nd-Level Callers:     %d" % len(second_level))
log("Total BC10 Accessors:  %d" % len(bc10_funcs))

log("\n" + "=" * 70)
log("Analysis complete!")
log("Results saved to: %s" % OUTPUT)
log("=" * 70)
