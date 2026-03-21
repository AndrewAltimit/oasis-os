# PSP USB Host Mode — Phase 0 Debug Tools

Debug EBOOTs for investigating USB host mode and VBUS power sourcing on the PSP.
Part of the USB Host Mode PRD v2 implementation.

## Crates

### `oasis-usb-debug-psp`
Phase 0 user-mode EBOOT. Calls Sony's camera driver init sequence and monitors
USB state flags. Confirms whether VBUS activates as a side-effect of camera
driver initialization.

**Result:** No VBUS output. `sceUsbActivate(0x282)` returns `0x80243002`
(DRIVER_NOT_FOUND) without touching hardware — the driver bails before
reaching any USB controller code.

### `oasis-usb-trace-psp`
Phase 0b kernel-mode EBOOT. Iterated through 8 versions (0b through 0b-8)
performing increasingly targeted experiments:

- **0b:** MMIO register snapshot diffs during USB operations
- **0b-2:** Expanded scan ranges + GPIO VBUS experiment
- **0b-3:** Syscon/Sysreg NID resolution + USB storage mode with PC cable
- **0b-4:** Raw Syscon CmdExec packets (discovered wrong packet format)
- **0b-5:** CmdExec hook (captured 0 calls — wrappers bypass CmdExec)
- **0b-6:** Function disassembly from kernel memory (found SetUSBStatus bug)
- **0b-7:** Correct packet format from disassembly (cmd 0x0C/0x0E, tx at +0x0C)
- **0b-8:** SYSCTL 0xBC100040 register writes (camera driver's actual code path)
- **Memory dump:** 4MB kernel RAM dumper for offline RE

## Build

```bash
cd crates/oasis-usb-debug-psp && ./build.sh release
cd crates/oasis-usb-trace-psp && ./build.sh release
```

Deploy EBOOT.PBP to `ms0:/PSP/GAME/USBPHASE0/` or `ms0:/PSP/GAME/USBTRACE/`.

## Key Findings

### Hardware Register Map

| Address | Purpose |
|---------|---------|
| `0xBC100040` | USB mode control (camera driver sets bits 0+1) |
| `0xBC100078` | USB peripheral clock enable (bit 2) |
| `0xBC10007C` | USB bus clock enable (bit 23) |
| `0xBC1000B8` | USB bus enable (bit 0) |
| `0xBE240000` | GPIO — bit 19 = USB PHY enable (not VBUS) |

### Syscon Protocol (Baryon 0x00040600)

- GET USB status: cmd `0x0C`, length 2 → returns status in rx[3]
- SET USB status: cmd `0x0E`, length 2 (no value parameter)
- Packet TX starts at offset `+0x0C` (not `+0x08` as in uOFW docs)
- `sceSysconSetUSBStatus` wrapper is **buggy** on 6.61 — crashes for non-zero args

### Architecture

`sceUsbActivate` does NO hardware writes. It checks a driver registration table
and calls the registered driver's callback. Without a real USB accessory
connected, activation fails at the software level before any hardware is touched.

The camera driver (`usb_cam.prx`) only accesses `0xBC100040` and `0xBE240000` —
both of which were tested with all bit combinations without producing VBUS output.

### VBUS Status

**Not found** after testing on PSP-1001 (6.61 ARK-4) and PSP-3001 (6.20 PRO-C2).
VBUS control may be inside the USB bus driver's internal callback (unreachable
without a real accessory) or the PSP-1000/3000 hardware may not support VBUS
sourcing on the Mini-B connector.

See `docs/psp-usb-host-findings.md` for complete technical findings.
