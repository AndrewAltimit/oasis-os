#!/usr/bin/env python3
"""
PSP PRX NID Analyzer — standalone binary analysis (no Ghidra required).

Scans decrypted PSP PRX ELF files for:
  1. Import stub tables (NID→stub function mappings)
  2. Known NID→name resolution from community databases
  3. Hardware register accesses (GPIO, PHY, OHCI, sceSysreg, etc.)
  4. VBUS-related constants (pin 23, 0x00800000)
  5. Function call flow (JAL instruction targets)

Usage:
  python3 scripts/psp_nid_analyze.py /path/to/decrypted.prx [base_addr]

Example:
  python3 scripts/psp_nid_analyze.py ~/Downloads/USBTRACE/dec_661/usb.prx
  python3 scripts/psp_nid_analyze.py ~/Downloads/USBTRACE/dec_661/lowio.prx
"""

import struct
import sys
import os

# ── NID Database ────────────────────────────────────────────────────────

# sceSysreg_driver NIDs
SYSREG_NIDS = {
    0x1561BCD2: "sceSysregUsbClkEnable",
    0x1D233EF9: "sceSysregUsbClkDisable",
    0x30C0A141: "sceSysregUsbQueryIntr",
    0x6C0EE043: "sceSysregUsbAcquireIntr",
    0x6F3B6D7D: "sceSysregUsbResetDisable",
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

# sceSyscon_driver NIDs
SYSCON_NIDS = {
    0xC8D97773: "sceSysconCtrlUsbPower",
    0x23093E69: "sceSysconCtrlHRPower",
    0xFB148FB6: "sceSysconSetUSBStatus",
    0x5B9ACC97: "sceSysconGetUSBStatus",
    0x4AB44BFC: "sceSysconCtrlCharge",
    0x3E3B0D30: "sceSysconGetBaryonVersion",
}

# scePower_driver NIDs
POWER_NIDS = {
    0xD3075926: "scePower_D3075926",
    0x0442D852: "scePower_0442D852",
    0x2875994B: "scePower_2875994B",
}

# sceGpio_driver NIDs
GPIO_NIDS = {
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

# sceUsb_driver NIDs
USB_NIDS = {
    0xAE5DE6AF: "sceUsbStart",
    0xC2464FA0: "sceUsbStop",
    0x586DB82C: "sceUsbActivate",
    0xC572A9C8: "sceUsbDeactivate",
    0xC21645A4: "sceUsbGetState",
    0x112CC951: "sceUsbGetDrvState",
    0x1C360735: "sceUsbWaitCancel",
}

# sceUsbBus_driver NIDs
USBBUS_NIDS = {
    0xB1644BE7: "sceUsbbdRegister",
    0xC1E2A540: "sceUsbbdUnregister",
    0x23E51D8F: "sceUsbbdReqSend",
    0x913EC15D: "sceUsbbdReqRecv",
    0xCC57EC9D: "sceUsbbdReqCancel",
    0xC5E53685: "sceUsbbdReqCancelAll",
    0x951A24CC: "sceUsbbdClearFIFO",
    0xE65441C1: "sceUsbbdStall",
}

ALL_NIDS = {}
ALL_NIDS.update(SYSREG_NIDS)
ALL_NIDS.update(SYSCON_NIDS)
ALL_NIDS.update(POWER_NIDS)
ALL_NIDS.update(GPIO_NIDS)
ALL_NIDS.update(USB_NIDS)
ALL_NIDS.update(USBBUS_NIDS)

# Hardware register base addresses
HW_REGS = {
    0xBC10: "sceSysreg",
    0xBD10: "OHCI",
    0xBD80: "MUSB",
    0xBDE0: "Syscon_SPI",
    0xBE24: "GPIO",
    0xBE4C: "USB_PHY",
    0xBE50: "Unknown_BE50",
    0xBFF0: "Unknown_BFF0",
    0xA7F0: "Unknown_A7F0",
}


def parse_elf_header(data):
    """Parse ELF header, return (entry, phoff, shoff, phnum, shnum, shstrndx)."""
    if data[:4] != b'\x7fELF':
        return None
    # ELF32 LE
    e_type = struct.unpack_from('<H', data, 16)[0]
    e_entry = struct.unpack_from('<I', data, 24)[0]
    e_phoff = struct.unpack_from('<I', data, 28)[0]
    e_shoff = struct.unpack_from('<I', data, 32)[0]
    e_phnum = struct.unpack_from('<H', data, 44)[0]
    e_shnum = struct.unpack_from('<H', data, 48)[0]
    e_shstrndx = struct.unpack_from('<H', data, 50)[0]
    return {
        'type': e_type,
        'entry': e_entry,
        'phoff': e_phoff,
        'shoff': e_shoff,
        'phnum': e_phnum,
        'shnum': e_shnum,
        'shstrndx': e_shstrndx,
    }


def parse_sections(data, elf):
    """Parse ELF section headers."""
    sections = []
    shoff = elf['shoff']
    shnum = elf['shnum']
    if shoff == 0 or shnum == 0:
        return sections

    # Get string table
    strtab_offset = 0
    if elf['shstrndx'] < shnum:
        str_sh = shoff + elf['shstrndx'] * 40
        strtab_offset = struct.unpack_from('<I', data, str_sh + 16)[0]

    for i in range(shnum):
        off = shoff + i * 40
        if off + 40 > len(data):
            break
        sh_name = struct.unpack_from('<I', data, off)[0]
        sh_type = struct.unpack_from('<I', data, off + 4)[0]
        sh_addr = struct.unpack_from('<I', data, off + 12)[0]
        sh_offset = struct.unpack_from('<I', data, off + 16)[0]
        sh_size = struct.unpack_from('<I', data, off + 20)[0]

        # Get name string
        name = ""
        if strtab_offset and sh_name:
            end = data.find(b'\x00', strtab_offset + sh_name)
            if end > 0:
                name = data[strtab_offset + sh_name:end].decode('ascii', 'replace')

        sections.append({
            'name': name,
            'type': sh_type,
            'addr': sh_addr,
            'offset': sh_offset,
            'size': sh_size,
        })
    return sections


def scan_nids(data):
    """Find all known NIDs in the binary data."""
    found = {}
    for nid, name in ALL_NIDS.items():
        nid_bytes = struct.pack('<I', nid)
        pos = 0
        locs = []
        while True:
            idx = data.find(nid_bytes, pos)
            if idx == -1:
                break
            locs.append(idx)
            pos = idx + 4
        if locs:
            found[nid] = (name, locs)
    return found


def scan_hw_regs(data):
    """Find LUI instructions loading hardware register bases."""
    accesses = {}
    for i in range(0, len(data) - 4, 4):
        word = struct.unpack_from('<I', data, i)[0]
        opcode = (word >> 26) & 0x3F
        if opcode == 0x0F:  # LUI
            rt = (word >> 16) & 0x1F
            imm = word & 0xFFFF
            if imm in HW_REGS:
                name = HW_REGS[imm]
                if name not in accesses:
                    accesses[name] = []
                # Get following instruction for offset
                offset_str = ""
                if i + 4 < len(data):
                    nw = struct.unpack_from('<I', data, i + 4)[0]
                    nop = (nw >> 26) & 0x3F
                    nimm = nw & 0xFFFF
                    if nimm > 0x7FFF:
                        nimm -= 0x10000
                    if nop in [0x09, 0x0D, 0x23, 0x2B]:  # ADDIU/ORI/LW/SW
                        offset_str = "+0x%04X" % (nimm & 0xFFFF)
                accesses[name].append((i, rt, offset_str))
    return accesses


def scan_jal(data):
    """Find all JAL (Jump And Link) instructions."""
    jals = {}
    for i in range(0, len(data) - 4, 4):
        word = struct.unpack_from('<I', data, i)[0]
        opcode = (word >> 26) & 0x3F
        if opcode == 0x03:  # JAL
            target = (word & 0x03FFFFFF) << 2
            if target not in jals:
                jals[target] = []
            jals[target].append(i)
    return jals


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)

    filepath = sys.argv[1]
    if not os.path.exists(filepath):
        print(f"Error: {filepath} not found")
        sys.exit(1)

    with open(filepath, 'rb') as f:
        data = f.read()

    filename = os.path.basename(filepath)
    print("=" * 70)
    print(f"PSP PRX NID Analysis: {filename}")
    print(f"Size: {len(data)} bytes (0x{len(data):X})")
    print("=" * 70)

    # ELF header
    elf = parse_elf_header(data)
    if elf:
        print(f"\nELF Type: 0x{elf['type']:04X} "
              f"({'PRX' if elf['type'] == 0xFFA0 else 'exec' if elf['type'] == 2 else 'other'})")
        print(f"Entry: 0x{elf['entry']:08X}")

        sections = parse_sections(data, elf)
        if sections:
            print(f"\nSections ({len(sections)}):")
            for s in sections:
                if s['size'] > 0:
                    print(f"  {s['name']:20s} addr=0x{s['addr']:08X} "
                          f"off=0x{s['offset']:06X} size=0x{s['size']:06X}")

            # Find .lib.stub section
            for s in sections:
                if s['name'] == '.lib.stub' or 'stub' in s['name'].lower():
                    print(f"\n*** IMPORT STUB SECTION: {s['name']} ***")
                    print(f"    Offset: 0x{s['offset']:06X}, Size: 0x{s['size']:06X}")
                    # Parse stub entries
                    stub_data = data[s['offset']:s['offset'] + s['size']]
                    print(f"    Raw bytes (first 128): {stub_data[:128].hex()}")
    else:
        print("\nNot a valid ELF file (might be raw binary)")

    # NID scan
    print("\n" + "=" * 70)
    print("NID SCAN")
    print("=" * 70)

    found_nids = scan_nids(data)
    if found_nids:
        # Group by library
        by_lib = {
            'sceSysreg_driver': [],
            'sceSyscon_driver': [],
            'scePower_driver': [],
            'sceGpio_driver': [],
            'sceUsb_driver': [],
            'sceUsbBus_driver': [],
            'other': [],
        }
        for nid, (name, locs) in sorted(found_nids.items()):
            if nid in SYSREG_NIDS:
                by_lib['sceSysreg_driver'].append((nid, name, locs))
            elif nid in SYSCON_NIDS:
                by_lib['sceSyscon_driver'].append((nid, name, locs))
            elif nid in POWER_NIDS:
                by_lib['scePower_driver'].append((nid, name, locs))
            elif nid in GPIO_NIDS:
                by_lib['sceGpio_driver'].append((nid, name, locs))
            elif nid in USB_NIDS:
                by_lib['sceUsb_driver'].append((nid, name, locs))
            elif nid in USBBUS_NIDS:
                by_lib['sceUsbBus_driver'].append((nid, name, locs))
            else:
                by_lib['other'].append((nid, name, locs))

        for lib, entries in by_lib.items():
            if entries:
                print(f"\n  {lib} ({len(entries)} NIDs):")
                for nid, name, locs in entries:
                    loc_str = ", ".join(f"0x{l:06X}" for l in locs[:5])
                    print(f"    0x{nid:08X} = {name:35s} at {loc_str}")
    else:
        print("  No known NIDs found")

    # Hardware register access scan
    print("\n" + "=" * 70)
    print("HARDWARE REGISTER ACCESSES")
    print("=" * 70)

    hw_accesses = scan_hw_regs(data)
    for name in sorted(hw_accesses.keys()):
        entries = hw_accesses[name]
        print(f"\n  {name} ({len(entries)} accesses):")
        for offset, rt, off_str in entries[:20]:
            print(f"    0x{offset:06X}: LUI ${rt}, ... {off_str}")
        if len(entries) > 20:
            print(f"    ... and {len(entries) - 20} more")

    # VBUS constant scan
    print("\n" + "=" * 70)
    print("VBUS CONSTANTS")
    print("=" * 70)

    # Search for 0x00800000 as LUI 0x0080
    for i in range(0, len(data) - 4, 4):
        word = struct.unpack_from('<I', data, i)[0]
        opcode = (word >> 26) & 0x3F
        if opcode == 0x0F:  # LUI
            imm = word & 0xFFFF
            if imm == 0x0080:
                rt = (word >> 16) & 0x1F
                print(f"  0x{i:06X}: LUI ${rt}, 0x0080 (= 0x00800000, pin 23 mask)")
        elif opcode == 0x09:  # ADDIU
            rs = (word >> 21) & 0x1F
            rt = (word >> 16) & 0x1F
            imm = word & 0xFFFF
            if imm == 23 and rs == 0:
                print(f"  0x{i:06X}: ADDIU ${rt}, $zero, 23 (pin number)")

    # JAL analysis
    print("\n" + "=" * 70)
    print("CALL GRAPH (JAL targets)")
    print("=" * 70)

    jals = scan_jal(data)
    internal = []
    external = []
    for target, callers in sorted(jals.items()):
        if target < len(data):
            internal.append((target, len(callers)))
        else:
            external.append((target, len(callers)))

    print(f"\n  Internal targets: {len(internal)}")
    for target, count in internal[:50]:
        print(f"    0x{target:06X} called {count} time(s)")

    print(f"\n  External targets (import stubs): {len(external)}")
    for target, count in external[:30]:
        print(f"    0x{target:08X} called {count} time(s)")

    print("\n" + "=" * 70)
    print("Analysis complete!")
    print("=" * 70)


if __name__ == '__main__':
    main()
