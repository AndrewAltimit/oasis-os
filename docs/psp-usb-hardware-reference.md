# PSP USB Hardware Reference

**Hardware:** PSP-3001 (TA-090v2), Tachyon 0x82000002, Firmware 6.61, ARK-4 CFW
**Date:** 2026-03-21
**Source:** Firmware decryption + Ghidra decompilation of `usb.prx` and `lowio.prx`, on-device register probing

This document consolidates register maps, NID tables, and hardware findings from reverse engineering the PSP's USB subsystem. See `docs/psp-usb-vbus-findings.md` for the full research narrative.

---

## USB Controller Architecture

**The PSP uses a MUSB (Mentor USB OTG) controller, NOT OHCI for USB device/host mode.**

| Controller | Address | Refs in usb.prx | Status on TA-090v2 |
|---|---|---|---|
| MUSB OTG | 0xBD80xxxx | 143 | Bus-faults (unmapped without bus gate) |
| OHCI | 0xBD10xxxx | 0 in usb.prx, 82 in lowio.prx | Accessible after clock enable |

The OHCI controller at 0xBD100000 is used by `lowio.prx` for low-level I/O, not USB. `usb.prx` exclusively uses the MUSB registers. The MUSB bus gate register is unknown — BC1000C4 was identified as a candidate but rejects writes on TA-090v2.

---

## GPIO Register Map (0xBE240000)

Corrected from Ghidra decompilation of `lowio.prx` (`sceGpioSetPortMode`, `sceGpioSetPortMode2`, `sceGpioPortRead`).

| Offset | Register | R/W on TA-090v2 | Notes |
|---|---|---|---|
| +0x00 | Port 0 Read | R | Pin state readback. Baseline: 0x020000C9 |
| +0x04 | Port 1 Read | R | Baseline: 0x01000041 (bit 6 toggles) |
| +0x08 | Port 1 Set | W | NOT Port 0 Output (corrects wiki) |
| +0x0C | Port 1 Clear | W | |
| +0x10 | Port 0 Direction | R/W | 0=input, 1=output. Accepts writes. |
| +0x14 | Port 0 Set | R/W | Write 1 to set output bits. Accepts writes. |
| +0x18 | Port 0 Clear | W | Write 1 to clear output bits |
| +0x1C | Port 1 Direction | R/W | Baseline: 0x05000010 |
| +0x20 | Interrupt Status | R | Read by `sceGpioGetCapturePort` |
| +0x24 | **Output Enable** | **LOCKED** | Written by `sceGpioSetPortMode2` with `1 << pin`. Rejects writes on TA-090v2 (stays 0). |
| +0x28 | Unknown | R | Reads 0 |
| +0x2C | Unknown | R | Reads 0 |
| +0x30 | Interrupt Config 0 | W | Cleared to 0 during GPIO init |
| +0x34 | Interrupt Config 1 | W | Cleared to 0 during GPIO init |
| +0x40 | Port 0 AltFunc | **LOCKED** | Locked at 0x05000010 on TA-090v2. Controls GPIO vs peripheral routing. |
| +0x44 | Unknown | R | Reads 0 |
| +0x48 | Port 1 AltFunc | R | Polled for busy flag (bits 0-1) by `sceGpioSetPortMode2` |

### GPIO Output Path

From `sceGpioSetPortMode2` decompilation, the output path requires **four** registers:

```
1. BC10007C |= (1 << pin)     // sceSysreg port enable (writable)
2. +0x10 Direction |= (1 << pin)  // output mode (writable)
3. +0x24 OutputEnable = (1 << pin) // output MUX (LOCKED on TA-090v2)
4. +0x14 Set = (1 << pin)         // drive high (writable)
```

Without step 3, the output flip-flop never latches regardless of Direction and Set.

### GPIO Pin Functions (PSP-3001)

| Pin | Function | Notes |
|---|---|---|
| 3 | LCD backlight | Toggling turns off screen |
| 4 | Critical (crash) | Unknown function |
| 19 | USB PHY transceiver | Disrupts USB |
| 23 | **VBUS MOSFET** | Controls 5V USB power output |
| 24 | Critical (crash) | Unknown function |
| 26 | Critical (crash) | Crashes during SetPortMode |

### Writable-Pins Mask

`sceGpioSetPortMode2` masks all Direction/Set/Clear writes with a hardware-defined "writable-pins" mask loaded from a platform callback during GPIO init. If a pin is not in this mask, writes to Direction/Set/Clear for that pin are silently discarded. On TA-090v2, pin 23 appears to be excluded from this mask.

---

## sceSysreg Register Map (0xBC100000)

| Offset | Register | Value (TA-090v2) | Writable | Notes |
|---|---|---|---|---|
| +0x40 | Tachyon Version | 0x82000002 | R | Model identifier |
| +0x4C | Bus Control | 0x00000040 | R/W | Changes to 0x00010020 after USB init |
| +0x50 | Peripheral Clock 1 | 0x0000DC1D | R/W | Bit 8 set by USB init (→ 0xDD1D), bit 13 = OHCI gate |
| +0x54 | Unknown | | R | |
| +0x58 | Peripheral Clock 2 | 0x05AD2601 | R/W | Bit 9 = USB clock |
| +0x5C | Unknown | | R | |
| +0x74 | USB Control | 0x00000000 | R/W | Bit 8 set by USB init (→ 0x100) |
| +0x78 | OHCI/USB Clock | 0x03082AFA | R/W | Bit 1 = OHCI, bit 19 = USB PHY |
| +0x7C | **GPIO Port Enable** | 0x070000D9 | R/W | Per-pin enable. Pin 23 NOT set at boot. Accepts `\|= 0x00800000` write. |
| +0x80 | GPIO/IO Control | 0x00000100 | R | Active register used by lowio.prx |
| +0xB0 | USB Host Interrupt | 0x00000000 | R | Documented by SilverSpring as interrupt status |
| +0xB8 | USB Host Bus Gate | 0x00000000 | R/W | Accepts writes (0→1). 15 references in usb.prx. |
| +0xC0 | Unknown | 0x00000000 | R | |
| +0xC4 | **USB Host Mode?** | 0x00000000 | **LOCKED** | Rejects writes. usb.prx writes `\|= 1` at offset 0x5918. |
| +0xC8 | Unknown | 0x00000000 | R | |
| +0xCC | Unknown | 0x00000000 | R | |
| +0xF0 | USB Config | 0x0000008A | R | 3 references in usb.prx |
| +0xF4 | USB Config 2 | 0x0000008A | R | Same value as +0xF0 |
| +0xF8 | Unknown | 0x00000000 | R | |

---

## 0xBE500000 Unknown Peripheral

20 references in kernel dump. Accessible from kernel mode (no bus fault).

| Offset | Value | Notes |
|---|---|---|
| +0x000 | 0x00000000 | |
| +0x004 | 0x00000000 | |
| +0x018 | **0x00000099** | Active — purpose unknown |
| +0x02C | **0x00000071** | Active — purpose unknown |
| +0x040 | 0x00000000 | |
| +0x044 | 0x00000000 | |
| +0x048 | 0x00000000 | |
| +0x300 | 0x00000000 | |

---

## USB PHY Registers (0xBE4C0000)

Already configured for host mode at boot.

| Offset | Register | Value | Notes |
|---|---|---|---|
| +0x00 | Data | - | Serial data R/W |
| +0x18 | Status | - | Bit 5 = TX busy, bit 4 = RX ready |
| +0x24 | Clock Divisor Low | 0x06 | 96MHz / 6 = 16MHz USB clock |
| +0x28 | Clock Divisor High | 0x00 | |
| +0x2C | Config | 0x70 | PHY configuration |
| +0x30 | Mode | 0x301 | Host mode already set (bit 0 + bits 8-9) |
| +0x34 | Sub-mode | 0x00 | |
| +0x44 | Feature Enable | 0x000 | Writes of 0x7FF don't persist |

---

## Syscon Command Map (Baryon 0x00040600, PSP-3001)

Communication via `_sceSysconCommonWrite` at 0x880A6E4C (PSP-3001) / 0x880A6D4C (PSP-1001).

### USB-Related Commands

| Cmd | Type | Response | Purpose |
|---|---|---|---|
| 0x44 | GET | Error 0x80250084 | Not valid on this Tachyon |
| 0x46 | GET | `0A 06 82 5D B7 02` | USB power state (unchanged by OTG adapter) |
| 0x47 | SET | `0A 03 82 70` | USB power control. Accepted for values 0-4 but no effect on GPIO. |
| 0x45 | SET | **DANGEROUS** | Triggers shutdown/reboot |

### Telemetry Commands (0x60-0x72)

| Cmd | Response | Purpose |
|---|---|---|
| 0x61 | `0A 06 82 00 57 05` | Battery/power data |
| 0x62 | `0A 04 82 14 5B` | Temperature? |
| 0x63 | `0A 05 82 F0 0F 6F` | Extended status |
| 0x64 | `0A 05 82 20 FF 4F` | Extended status |
| 0x65 | `0A 04 82 4B 24` | Power supply |
| 0x66 | `0A 05 82 57 05 12` | Extended status |
| 0x67-0x68 | `0A 05 82 FE 06 6A` | Same response |
| 0x69 | `0A 05 82 55 01 18` | Extended status |
| 0x6A | `0A 04 82 50 1F` | Extended status |
| 0x6B | `0A 05 82 00 00 6E` | Zero data |
| 0x6C | `0A 07 82 DB 06 A8` | Extended (7-byte) |
| 0x6D | `0A 08 82 78 09 0A` | Extended (8-byte) |
| 0x6E-0x72 | `0A 05 82 00 00 6E` | Zero data |

### General Commands

| Cmd | Response | Purpose |
|---|---|---|
| 0x00 | ACK | NOP |
| 0x01 | `00 06 04 00` | Baryon version (0x00040600) |
| 0x07 | `FF 7F EF` | Extended status |
| 0x09 | `CF FF FF` | Extended status |
| 0x0B | `02` | Power status |
| 0x0C | `01` | USB status |
| 0x0E | `41 08` | USB status (changes with OHCI) |
| 0x52, 0x60, 0x7F | `82 70` | Standard ACK |

### Dangerous Commands

| Cmd | Effect |
|---|---|
| 0x34 | Hard crash |
| 0x45 | Shutdown/reboot (screen black, MS LED solid) |

---

## usb.prx NID Import Table

### sceGpio_driver (3 NIDs)

| NID | Function | Stub Offset | Purpose |
|---|---|---|---|
| 0x103C3EB2 | sceGpioPortClear | 0x8FB0 | Clear GPIO pin (write 1 to clear) |
| 0x310F0CCF | sceGpioPortSet | 0x8FB8 | Set GPIO pin (write 1 to set) |
| 0x317D9D2C | sceGpioSetPortMode2 | 0x8FC0 | Set pin mode (0=disable, 2=output enable) |

### sceSysreg_driver (14 NIDs)

| NID | Function | Resolved Address | Return |
|---|---|---|---|
| 0xEC03F6E2 | sceSysregGpioClkEnable | 0x880875F4 | 0 |
| 0x72C1CA96 | sceSysregGpioIoEnable | 0x880875D4 | 0 |
| 0x1561BCD2 | sceSysregUsbClkEnable | 0x88087510 | 0 |
| 0x9306F27B | sceSysregUsbIoEnable | 0x88086784 | 1 (already enabled) |
| 0x9A6E7BB8 | sceSysregUsbBusClkEnable | 0x880875E4 | 0 |
| 0x84A279A4 | sceSysregUsbResetEnable | 0x88087074 | 9 |
| 0x6F3B6D7D | sceSysregUsbResetDisable | 0x88088228 | 0 |
| 0x87B61303 | sceSysregUsbGetConnectStatus | 0x88088270 | 1 |
| 0x9275DD37 | sceSysregUsbSetConnectStatus | 0x8808751C | 1 |
| 0x30C0A141 | sceSysregUsbQueryIntr | 0x88086778 | 0 |
| 0x6C0EE043 | sceSysregUsbAcquireIntr | 0x8808707C | 9 |
| 0x1D233EF9 | sceSysregUsbClkDisable | 0x88086A2C | 0 |
| 0xD7AD9705 | sceSysregUsbBusClkDisable | 0x88086A20 | 0 |
| 0xE2A5D1EE | sceSysregUsbIoDisable | 0x88088050 | 0x00820000 |

### scePower_driver (3 NIDs)

| NID | Function | Notes |
|---|---|---|
| 0xD3075926 | Unknown | Returns 0 |
| 0x0442D852 | **scePowerRequestColdReset** | **CAUSES REBOOT — do not call** |
| 0x2875994B | Unknown | Returns 0 |

### sceSyscon_driver (NID resolution issue)

NID 0xC8D97773 (`sceSysconCtrlUsbPower`) resolves to 0x880A7690 on PSP-3001, but this points to a **getter stub region** (consecutive `LUI`/`LW`/`JR $ra` sequences), not the real function. The real sceSysconCtrlUsbPower is at a different address not reachable via NID resolution on this firmware.

---

## VBUS Enable/Disable Functions (from usb.prx decompilation)

### VBUS Disable — `FUN_00008bd0`

```c
int vbus_disable(void) {
    int ret = sceGpioSetPortMode2(23, 0);   // mode 0 = disable output
    if (ret >= 0) {
        sceGpioPortClear(0x00800000);        // clear pin 23
        ret = 0;
    }
    return ret;
}
```

### VBUS Enable — `FUN_00008c0c`

```c
void vbus_enable(void) {
    sceGpioSetPortMode2(23, 2);   // mode 2 = enable output
    // PortSet(0x00800000) called separately by caller via vtable
}
```

### Callback Registration — `FUN_00008afc`

```c
void vbus_callback(void) {
    if (driver_state != 0 && vbus_flag != 0) {
        (*(callback_table + 0x0c))();   // vtable call to enable/disable
        vbus_flag = 0;
    }
}
```

VBUS enable/disable functions are registered in a callback vtable at struct offset +0x648, called by the USB driver state machine during `sceUsbActivate`. The vtable is only invoked when a real USB accessory is detected.

---

## Register Write Lock Summary (TA-090v2)

| Register | Address | Writable? | Notes |
|---|---|---|---|
| GPIO Direction | +0x10 | **Yes** | |
| GPIO Set | +0x14 | **Yes** | |
| GPIO Clear | +0x18 | **Yes** | |
| **GPIO Output Enable** | **+0x24** | **No** | Stays 0. Silicon-locked. |
| **GPIO AltFunc** | **+0x40** | **No** | Stays 0x05000010. Silicon-locked. |
| BC10007C Port Enable | +0x7C | **Yes** | Accepts pin 23 bit |
| BC1000B8 Bus Gate | +0xB8 | **Yes** | Accepts writes |
| **BC1000C4 Host Mode** | **+0xC4** | **No** | Stays 0. Silicon-locked. |
| BC100050 Clock 1 | +0x50 | **Yes** | |
| BC100074 USB Control | +0x74 | **Yes** | |

The three locked registers (Output Enable, AltFunc, BC1000C4) are set during the Tachyon mask ROM boot sequence before any firmware code executes. No kernel-level code, Syscon command, or USB ID pin signal can unlock them.

---

## Tools

| Tool | Location | Purpose |
|---|---|---|
| `psp_nid_analyze.py` | `scripts/` | Standalone PSP PRX NID analyzer — scans ELF for NIDs, hardware register accesses, call graphs. No Ghidra needed. |
| `ghidra_usb_vbus.py` | `scripts/` | Ghidra headless script for usb.prx — traces VBUS init chain, maps NIDs to stubs |
| `ghidra_lowio_gpio.py` | `scripts/` | Ghidra headless script for lowio.prx — decompiles GPIO driver functions |
| `docker/ghidra/Dockerfile` | `docker/ghidra/` | amd64 Ghidra container for ARM64 hosts (requires qemu-user-static) |
| `oasis-usb-vbus-psp` | `crates/` | 21-step interactive kernel EBOOT for on-device register probing |
| `oasis-prx-decrypt-psp` | `crates/` | On-device PRX decryption using Kirk hardware engine |

---

## Decrypted Firmware Files

Decrypted on-device via memlmd NID 0xEF73E85B (Kirk hardware engine). Located at `/home/mikunpc/Downloads/USBTRACE/dec_661/`.

| File | Size | Key Findings |
|---|---|---|
| usb.prx | 43 KB | 143 MUSB refs, VBUS code at 0x8C70, 14 sceSysreg + 3 GPIO imports |
| lowio.prx | 56 KB | GPIO driver (50 GPIO, 90 sceSysreg refs), output enable mechanism |
| syscon.prx | 22 KB | Also controls GPIO pin 23, accesses BC100080 |
| usbcam.prx | 41 KB | No GPIO/sceSysreg — delegates all hardware init to usb.prx |
