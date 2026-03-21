# PSP USB VBUS Power Output — Research Findings

**Date:** 2026-03-21
**Hardware:** PSP-3001 (TA-090v2), Tachyon 0x82000002, Firmware 6.61, ARK-4 CFW
**Branch:** `feat/usb-host-phase0`
**Crate:** `crates/oasis-usb-vbus-psp/`
**Goal:** Enable 5V VBUS output on PSP Mini-B port to power Luckfox Pico via single USB cable

---

## Executive Summary

We built two kernel-mode EBOOTs for PSP-3001 VBUS research:
- **`oasis-usb-vbus-psp`** — interactive 14-step test tool across 5 phases (GPIO, Syscon, register init, GPIO sweep, VBUS enable)
- **`oasis-prx-decrypt-psp`** — flash0 PRX decryption tool using Kirk hardware engine

### Key Results

1. **PRX decryption successful** — all 15 USB/system PRXs from flash0:/kd/ decrypted on-device via memlmd NID
2. **Firmware RE identified GPIO pin 23** (mask 0x00800000) as the VBUS MOSFET control in `usb.prx`
3. **GPIO Direction register IS writable** — pin 23 direction successfully set to output
4. **GPIO Output does NOT latch** — Set register accepts writes but Output stays 0x00000000
5. **AltFunc register is locked** — writes ignored, stuck at 0x05000010
6. **The GPIO output stage has an additional gate** on TA-090v2 not present on earlier models — likely controlled by a sceSysreg or firmware init step we haven't replicated yet

### Current Blocker

The firmware's USB host init runs a sequence of sceUsb_driver, sceSysreg_driver, and sceSyscon_driver calls BEFORE the GPIO pin 23 set. We're only doing the final GPIO step without the preceding setup. A full Ghidra analysis of the decrypted `usb.prx` call chain is needed to find the missing steps.

**Next step:** Decrypt flash0 PRX firmware files to find the actual VBUS control sequence.

---

## Hardware Register Map

### Register Accessibility from Kernel Mode

| Register Block | Address | Accessible? | Notes |
|---|---|---|---|
| System Clock 1 | 0xBC100050 | Yes (R/W) | Value: 0x0000DC1D, all USB bits already set |
| System Clock 2 | 0xBC100058 | Yes (R/W) | Value: 0x05AD2601 |
| OHCI Clock | 0xBC100078 | Yes (R/W) | Value: 0x03082AFA, bit 19 already set |
| Tachyon | 0xBC100040 | Yes (R/W) | Value: 0x82000002 |
| GPIO Port 0 | 0xBE240000 | Yes (R/W) | Read=020000C9, Dir=0, Out=0 |
| USB PHY | 0xBE4C0024-44 | Yes (R/W) | Mode=0x301 (host config already set!) |
| OHCI Controller | 0xBD100000-1038 | Partial | Reads return 0x8760624F (not initialized) |
| MUSB Controller | 0xBD800060 | **NO** | Bus-fault, completely unmapped |
| Syscon SPI | 0xBDE00000 | Via function | Packet-based protocol |

### GPIO Block (0xBE240000) Layout

From kernel dump analysis, the PSP GPIO has **two ports**:

| Offset | Function |
|---|---|
| +0x00 | Port 0 Read (pin state) |
| +0x04 | Port 1 Read |
| +0x08 | Port 0 Output Data |
| +0x0C | Port 1 Output Data |
| +0x10 | Port 0 Direction (0=in, 1=out) |
| +0x14 | Port 0 Set (write 1 to set) |
| +0x18 | Port 0 Clear (write 1 to clear) |
| +0x1C | Port 1 Direction/Set/Clear (TBD) |
| +0x40 | Port 0 Alternate Function |
| +0x48 | Port 1 Alternate Function |

**Critical finding:** We only tested Port 0 (offsets +0x00/+0x08/+0x10/+0x14/+0x18). Port 1 at +0x04/+0x0C was never tested and may control USB VBUS.

### Additional Register Blocks (from kernel dump)

| Block | Refs | Possible Function |
|---|---|---|
| 0xBE300000 | 7 | Unknown peripheral |
| 0xBE500000 | 20 | Heavily used, offsets 0x00/0x18/0x2C/0x40/0x300 — possibly USB OTG? |

---

## Phase 1: GPIO Discovery

### 1.1 GPIO Register Dump

```
PSP-3001 (TA-090v2):
  Read      = 0x020000C9
  Output    = 0x00000000
  Direction = 0x00000000
  AltFunc   = 0x05000010

PSP-1001 baseline (from prior work):
  Read      = 0x05000010
  Direction = 0x020000EF
  Output    = 0x01000067
```

**Key difference:** PSP-3001 has Direction=0 (all pins input) vs PSP-1001 Direction=0x020000EF (several outputs). The PSP-3001 may use different GPIO routing or control output pins through Syscon.

### 1.2 GPIO NID Resolution

All 4 GPIO NIDs resolved successfully from `sceLowIO_Driver` / `sceGpio_driver`:

| Function | NID | Status |
|---|---|---|
| sceGpioPortRead | 0x4250D44A | Resolved |
| sceGpioPortSet | 0x310F0CCF | Resolved |
| sceGpioPortClear | 0x103C3EB2 | Resolved |
| sceGpioSetPortMode | 0xFBC85E74 | Resolved |

### 1.3 GPIO Monitor During USB Init

**NOT TESTED** — requires Go!Cam USB camera accessory (not yet acquired).

---

## Phase 2: Syscon USB Power Commands

### 2.1 sceSysconCtrlUsbPower

- **NID 0xC8D97773** resolves to address **0x880A7690**
- Calling `sceSysconCtrlUsbPower(1)` returns **0xCCE56536** (garbage)
- **Root cause:** NID resolution points 4 bytes into the epilogue of the previous function. The actual function prologue is at 0x880A769C.
- The function calls 0x880A7A7C internally, which accesses a Syscon command table at 0x880BA6B0
- No GPIO or register changes observed after call

### 2.2 Syscon GET Status

| Command | Return | Response Bytes | Interpretation |
|---|---|---|---|
| GET 0x44 | 0x80250084 (error) | 0A 03 84 6E FF FF | Not valid on this Tachyon |
| GET 0x46 | 0x00000000 (ok) | 0A 06 82 5D B7 02 | USB power state data |
| GET 0x07 | 0x00000000 (ok) | 0A 07 07 FF 3F EF | Baryon version/status |
| GET 0x09 | 0x00000000 (ok) | 0A 07 09 5C 4F 02 | Power supply state |

**GET 0x46 response analysis:**
- Byte 0: 0x0A = response header
- Byte 1: 0x06 = response length
- Byte 2: 0x82 = USB mode indicator (device mode / power off?)
- Bytes 3-5: 0x5D B7 02 = status flags

### 2.3 Syscon SET Commands

| Command | Value | Return | Response | Effect |
|---|---|---|---|---|
| SET 0x47 | 0 | 0 (ok) | 0A 03 82 70 FF FF | None |
| SET 0x47 | 1 | 0 (ok) | 0A 03 82 70 FF FF | None |
| SET 0x47 | 2 | 0 (ok) | 0A 03 82 70 FF FF | None |
| SET 0x45 | 1 | - | - | **Screen goes black / reboot** |

Note: All SET 0x47 calls return the same response with byte 2 = 0x82, matching the GET 0x46 state. The Syscon accepts the command but doesn't change USB power state.

**SET 0x45 is dangerous** — triggers what appears to be a partial reboot sequence (screen black, memory card LED solid). This matches the firmware's USB reboot path.

---

## Phase 3: Direct Register Init

### 3.1 Clock + PHY

All clock bits were **already set** before any writes:
- BC100050 bit 14 (0x4000): already set
- BC100058 bit 9 (0x200): already set
- BC100078 bit 19 (0x80000): already set

PHY registers:
- ClkDiv = 6 (96MHz/6 = 16MHz USB clock) — already configured
- Mode = 0x301 (host mode bits already set!)
- Feature = 0x000 — **writes of 0x7FF don't persist**
- Config changes from 0x70 → 0x60 after write

**Key insight:** The PHY is already configured for host mode. The firmware or boot ROM sets these up early. But the Feature register won't accept writes, suggesting it's locked or requires a specific unlock sequence.

### 3.2/3.5 OHCI + Full Init

OHCI registers all return **0x8760624F** for every register (Revision, Control, CmdStatus, RhStatus, PortStatus). This is a bus default, not real register data. The OHCI controller is not initialized — just having clock bits set is insufficient.

MUSB (0xBD800060) bus-faults on any access. This is a separate peripheral that requires its own bus enable, likely through the USB bus driver module.

### 3.4 Tachyon Mode Bit

Not tested (marked as dangerous — could trigger Tachyon monitoring).

---

## Phase 4: GPIO VBUS Toggle

### 4.1 Full GPIO Sweep (Port 0, All 32 Pins)

Tested all GPIO pins 0-31 (skipping dangerous ones) via NID functions:

| Pins Tested | Method | Result |
|---|---|---|
| 0-2, 5-18, 20-22, 25, 27-31 | sceGpioSetPortMode + sceGpioPortSet | All return 0 (success), no GPIO change, no FNB58 activation |
| 3 | - | **LCD backlight turns off** |
| 4 | - | **Hard crash** |
| 19 | - | Known USB PHY disruptor (skipped) |
| 23 | - | **Hard crash** |
| 24 | - | **Hard crash** |
| 26 | - | **Hard crash** (during SetPortMode) |

**Critical observation:** Despite NID calls returning success (0), the GPIO Output register never changes from 0x00000000. The sceGpio_driver NID functions appear to be no-ops or are operating on a different hardware path than the MMIO registers at 0xBE240000.

### 4.2 Combined Approach

Syscon SET 0x45 v=1 causes screen fade / reboot. GPIO sweep after Syscon 0x47 shows no VBUS activation.

---

## Dangerous GPIO Pins (PSP-3001)

| Pin | Effect | Likely Function |
|---|---|---|
| 3 | Screen turns off | LCD backlight / display power |
| 4 | Hard freeze | Unknown critical hardware |
| 19 | USB PHY disruption | USB transceiver control |
| 23 | Hard freeze | Unknown critical hardware |
| 24 | Hard freeze | Unknown critical hardware |
| 26 | Hard freeze (during SetPortMode) | Unknown critical hardware |

---

## Kernel Memory Analysis

### NID Resolution Issue

The `sceSysconCtrlUsbPower` NID (0xC8D97773) was found in the kernel NID table at 0x88072370, but resolves to 0x880A7690 which is offset by 4 bytes from the actual function prologue at 0x880A769C. This explains the garbage return value (0xCCE56536).

The real function:
```
880A769C: addiu $sp, $sp, -0x10    ; prologue
880A76A0: sw $ra, 0($sp)
880A76A4: jal 0x880A7A7C           ; calls inner Syscon handler
880A76A8: addiu $6, $0, 4          ; arg3 = 4 (Syscon sub-command?)
```

### Additional Register Blocks

53 references to GPIO base 0xBE240000 in kernel, using offsets: +0x00, +0x04, +0x08, +0x0C, +0x10, +0x14, +0x18, +0x1C, +0x20, +0x30, +0x40, +0x48.

20 references to 0xBE500000 — heavily used in a driver around 0x88172xxx with offsets 0x00, 0x18, 0x2C, 0x40, 0x300. This could be the USB OTG controller or another power-management peripheral.

---

## Phase 5: VBUS Enable Attempt (Pin 23)

### GPIO Pin 23 Register Behavior (BEFORE → AFTER enable sequence)

| Register | Before | After | Writable? |
|---|---|---|---|
| P0 Direction (+0x10) | 0x00000000 | **0x00800000** | YES |
| P0 Set (+0x14) | 0x05000000 | **0x00800000** | YES (but doesn't latch) |
| P0 Output (+0x08) | 0x00000000 | 0x00000000 | **NO** |
| P0 Read (+0x00) | 0x020000C9 | 0x020000C9 | N/A (read-only) |
| P0 AltFunc (+0x40) | 0x05000010 | 0x05000010 | **NO** (locked) |
| P0 Clear (+0x18) | 0x05000010 | 0x05000010 | N/A |
| BC100074 | 0x00000000 | **0x00000100** | YES (sceSysreg) |
| BC10004C | 0x00000040 | **0x00000000** | YES (sceSysreg) |
| BC1000B8 | 0x00000000 | 0x00000000 | Briefly (resets) |

### Diagnosis

The GPIO Direction register and Set register both accept writes for bit 23 (0x00800000). However, the Set register write does NOT propagate to the Output register — the output flip-flop isn't being clocked or is gated by something upstream.

On earlier PSP models (TA-079v3 etc.), writing to the Set register immediately latches into Output. On TA-090v2, an additional gate exists — likely the **AltFunc register** (which is locked at 0x05000010) or a **sceSysreg output enable** register we haven't found.

The firmware's `usb.prx` calls GPIO functions in the middle of a larger USB init sequence involving `sceSysreg_driver` (14 imported functions), `sceSyscon_driver` (6 functions), and `scePower_driver` (3 functions). The GPIO output likely only works after specific sceSysreg/sceSyscon/scePower calls that configure the output MUX.

---

## Phase 2 NID Analysis Results (standalone, no Ghidra needed)

Script: `scripts/psp_nid_analyze.py`

### usb.prx (43,402 bytes, PRX type 0xFFA0)

**Entry point:** 0x00002D48

**NID import table confirmed (all 14 sceSysreg_driver at 0x9728-0x975C):**

| NID | Name | Offset |
|---|---|---|
| 0x1561BCD2 | sceSysregUsbClkEnable | 0x9728 |
| 0x1D233EF9 | sceSysregUsbClkDisable | 0x972C |
| 0x30C0A141 | sceSysregUsbQueryIntr | 0x9730 |
| 0x6C0EE043 | sceSysregUsbAcquireIntr | 0x9734 |
| 0x6F3B6D7D | sceSysregUsbResetDisable | 0x9738 |
| 0x72C1CA96 | sceSysregGpioIoEnable | 0x973C |
| 0x84A279A4 | sceSysregUsbResetEnable | 0x9740 |
| 0x87B61303 | sceSysregUsbGetConnectStatus | 0x9744 |
| 0x9275DD37 | sceSysregUsbSetConnectStatus | 0x9748 |
| 0x9306F27B | sceSysregUsbIoEnable | 0x974C |
| 0x9A6E7BB8 | sceSysregUsbBusClockEnable | 0x9750 |
| 0xD7AD9705 | sceSysregUsbBusClockDisable | 0x9754 |
| 0xE2A5D1EE | sceSysregUsbIoDisable | 0x9758 |
| 0xEC03F6E2 | sceSysregGpioClkEnable | 0x975C |

**GPIO NIDs (3):** PortClear (0x103C3EB2), PortSet (0x310F0CCF), SetPortMode2 (0x317D9D2C)

**VBUS pin 23 code locations:**
- 0x008C74: `ADDIU $4, $zero, 23` (pin number)
- 0x008C8C: `LUI $4, 0x0080` (mask 0x00800000)
- 0x008CB0: `ADDIU $4, $zero, 23` (disable path)
- 0x008CDC: `LUI $4, 0x0080` (disable mask)

**CRITICAL: 143 MUSB accesses (0xBD80xxxx), NOT OHCI!**
- usb.prx uses MUSB controller (Mentor USB OTG), not OHCI
- This explains why MUSB at 0xBD80xxxx bus-faults — the controller is powered down
- MUSB bus enable is likely through a sceSysreg register we haven't tried

**sceSysreg registers directly written by usb.prx:**
- BC1000B8 (15 references — our VBUS experiments showed this resets to 0)
- BC1000C4 (2 references — new, untested)
- BC1000F0 (3 references — new, untested)

### lowio.prx (56,052 bytes) — GPIO Driver

**Key findings:**
- 50 GPIO (0xBE24xxxx) accesses — the actual GPIO hardware driver
- 90 sceSysreg (0xBC10xxxx) accesses — GPIO output goes through sceSysreg
- New sceSysreg offsets: **BC100080** and **BC1000B0** (not tested in Phase 1)
- 82 OHCI references (0xBD10xxxx) — lowio includes the OHCI driver too

### syscon.prx (22,036 bytes) — Syscon Controller

**CRITICAL:** syscon.prx also has GPIO NIDs AND the 0x00800000 VBUS mask!
- Accesses GPIO+0x08 (Output register) directly
- Accesses BC100083 (sceSysreg — GPIO output MUX?)
- The Syscon controller may be the one that actually drives GPIO pin 23

---

## Conclusions

1. **GPIO pin 23 is confirmed** as the VBUS MOSFET control in `usb.prx` firmware
2. **Direction register works**, Set register accepts writes, but **Output doesn't latch** on TA-090v2
3. **AltFunc register is locked** — cannot be modified from kernel mode without finding the unlock mechanism
4. **sceSysreg GPIO enables (IoEnable, ClkEnable) resolve and return success** but are insufficient alone
5. **The full USB host init chain in usb.prx must be traced** — the GPIO set is the LAST step; preceding sceSysreg/sceSyscon calls are needed to unlock the output stage
6. **PRX decryption is complete** — 15 decrypted ELFs available for Ghidra analysis
7. **usb.prx uses MUSB (0xBD80xxxx), not OHCI** — 143 MUSB accesses vs 0 OHCI
8. **BC1000C4 and BC1000F0** are new sceSysreg registers written by usb.prx (untested)
9. **BC100080 and BC1000B0** are referenced by lowio.prx GPIO driver (untested)
10. **syscon.prx directly controls GPIO pin 23** via the Output register and BC100083

## Phase 2 On-Device Test Results (2026-03-21)

### Step 6.1: sceSysconCtrlUsbPower NID Analysis
- NID 0xC8D97773 resolves to **0x880A7690** — this is a **getter stub** region, NOT the actual function
- Instruction at +12 (0x880A769C) = `0x03E00008` (`jr $ra` — return), confirms it's a stub
- The real sceSysconCtrlUsbPower is elsewhere, not reachable via NID resolution
- Syscon SET 0x47 v=4 accepted but no effect (same as v=1,2)

### Step 6.1 Method C: New sceSysreg Registers
| Register | Value | Notes |
|---|---|---|
| BC1000C4 | 0x00000000 | Zero |
| BC1000F0 | 0x0000008A | Active — usb.prx writes this (3 refs) |
| BC1000F4 | 0x0000008A | Same value as F0 |
| BC100080 | 0x00000100 | Active — lowio.prx GPIO driver uses this |
| BC1000B0 | 0x00000000 | Zero |
| BC1000C0/C8/CC/F8 | 0x00000000 | All zero |

### Step 6.3: Full sceSysreg Init (14/14 NIDs resolved)
All 14 sceSysreg_driver NIDs resolved and called successfully:
- **BC100050 changed**: 0x0000DC1D → 0x0000DE1D (bit 9 = 0x200 set)
- UsbIoEn returns 1 (already enabled), UsbResetEn returns 9
- NID_E2A5D1EE (UsbIoDisable) returns 0x00820000

### Step 6.4: Full Chain + VBUS (consistent across 4 runs)
| Register | Before | After | Change |
|---|---|---|---|
| BC100050 | 0x0000DC1D | 0x0000DD1D | bit 8 (0x100) set |
| BC10004C | 0x00000040 | 0x00010020 | Major change! |
| BC100074 | 0x00000000 | 0x00000100 | bit 8 set |
| P1 Dir | 0x05000010 | 0x04000010 | bit 24 cleared |
| P0 Output | 0x00000000 | 0x00000000 | **STILL ZERO** |

- **scePower NID 0x0442D852 = scePowerRequestColdReset** — caused clean reboot (now skipped)
- **MUSB (0xBD800060) still bus-faults** even after BC10004C change — NOT the MUSB bus gate
- **PHY host mode init disrupts MS I/O** — screen flashes, log writes lost during step 4-7
- **sceCtrlPeekBufferPositive crashes** after USB sceSysreg init — input polling corrupted

### Step 6.5: sceUsbActivate Trace
- All USB modules load successfully (UsbPspCm, UsbAcc, UsbCam = 0)
- USBBusDriver + USBCamDriver start OK
- **sceUsbActivate(0x282) returns 0x80243002** — error, no camera connected
- **Zero GPIO changes** across entire module load/start/activate sequence
- USB state = 0x111 (driver loaded, no connection)
- VBUS enable only triggers when activation succeeds with real camera hardware

### Blocking Conclusion
GPIO pin 23 output **will not latch on TA-090v2** regardless of:
- All 14 sceSysreg enables called
- Syscon SET 0x47 commands
- PHY host mode configuration
- OHCI port power
- Direct MMIO writes to GPIO Direction/Set/Output registers

The output gate is controlled by either the **MUSB OTG controller** (inaccessible — bus-faults) or a hardware path in the **firmware reboot sequence**. A Go!Cam camera accessory is needed to trigger the firmware's own VBUS enable path and observe what additional register writes occur.

## Ghidra Decompilation Results (2026-03-21, via Docker amd64 container)

### usb.prx — VBUS Enable/Disable Functions

**VBUS Disable (`FUN_00008bd0`):**
```c
int FUN_00008bd0(void) {
    int ret = sceGpioSetPortMode2(23, 0);  // mode 0 = disable
    if (ret >= 0) {
        sceGpioPortClear(0x800000);         // clear pin 23
        ret = 0;
    }
    return ret;
}
```

**VBUS Enable (`FUN_00008c0c`):**
```c
void FUN_00008c0c(void) {
    sceGpioSetPortMode2(23, 2);  // mode 2 = enable output
    // NOTE: PortSet is called separately by the caller via vtable
}
```

**Vtable callback system (`FUN_00008afc`):**
```c
void FUN_00008afc(void) {
    if (driver_state != 0 && vbus_active != 0) {
        (*(callback_table + 0xc))();  // calls VBUS enable/disable via vtable
        vbus_active = 0;
    }
}
```

### lowio.prx — GPIO Driver Internals (CRITICAL)

**`sceGpioSetPortMode2` (`FUN_00002fd0`) — the function usb.prx calls for VBUS:**
```c
// mode 0: Direction=1, Set=1, Clear=0 (output + drive high)
// mode 2: Direction=0, Set=0, Clear=0 (input/disable)
// mode 3: Direction=0, Set=0, Clear=1 (input + clear)

// Critically:
do {} while ((_DAT_be240048 & 3) != 0);  // WAIT for AltFunc Port 1 busy flag!
_DAT_be240010 = Direction (MASKED by writable-pins mask)
_DAT_be240024 = 1 << pin;                // OUTPUT ENABLE REGISTER!
_DAT_be240014 = Set (MASKED)
_DAT_be240018 = Clear (MASKED)
```

**Key finding: `0xBE240024` is the OUTPUT ENABLE register!** Written with `1 << pin` during SetPortMode2.
Our register dumps showed `+0x24 = 0x00000000` — this register was never written during our tests.

**`sceGpioSetPortMode` (`FUN_00002e24`):**
- Mode 0 = input, AltFunc=1
- Mode 1 = output, AltFunc=0
- Mode 2 = input, AltFunc=0
- Writes to `0xBE240000` (Read shadow) and `0xBE240040` (AltFunc) — but AltFunc is LOCKED on TA-090v2

**Writable-pins mask (`_UNK_00000240`):**
Loaded from hardware via `FUN_0000b214()` callback during GPIO init (`FUN_00002cfc`).
If pin 23 is NOT in this mask, all Direction/Set/Clear writes for that pin are silently masked out.

### Corrected GPIO Register Map
| Offset | Function | Notes |
|---|---|---|
| +0x00 | Port 0 Read | Pin state readback |
| +0x04 | Port 1 Read | |
| +0x08 | Port 1 Set | NOT Port 0 Output! |
| +0x0C | Port 1 Clear | |
| +0x10 | Port 0 Direction | Masked by writable-pins |
| +0x14 | Port 0 Set | Masked by writable-pins |
| +0x18 | Port 0 Clear | Masked by writable-pins |
| +0x1C | Port 1 Direction | |
| +0x20 | Port 0 Interrupt? | Read in FUN_000033f4 |
| **+0x24** | **Port 0 Output Enable** | **Written by SetPortMode2 with 1<<pin** |
| +0x30 | Interrupt config 0 | Cleared during init |
| +0x34 | Interrupt config 1 | Cleared during init |
| +0x40 | Port 0 AltFunc | Locked on TA-090v2 (0x05000010) |
| +0x48 | Port 1 AltFunc | Polled for busy (bits 0-1) |

---

## Phase 2 Plan — Next Steps

### Phase A: Ghidra Deep Analysis (OFFLINE)

Scripts: `scripts/ghidra_usb_vbus.py` and `scripts/ghidra_lowio_gpio.py`

1. **usb.prx analysis** — Load into Ghidra as MIPS LE 32-bit ELF, run `ghidra_usb_vbus.py`
   - Maps all 14 sceSysreg_driver NIDs, 6 sceSyscon_driver, 3 scePower_driver
   - Traces USB host init call chain from entry to GPIO pin 23 set
   - Identifies every register write and function call preceding GPIO set
   - Scans for function pointer tables (VBUS enable/disable called via vtable)

2. **lowio.prx analysis** — Run `ghidra_lowio_gpio.py`
   - Finds sceGpioPortSet implementation (how Set→Output works)
   - Checks for GPIO output enable in sceSysreg (BC100xxx)
   - Looks for TA-090v2 specific code paths

### Phase B: On-Device Testing (new menu steps 6.1–6.5)

3. **Fix sceSysconCtrlUsbPower (step 6.1)** — Call 0x880A769C directly, try inner 0x880A7A7C
4. **Probe 0xBE500000 (step 6.2)** — Read offsets 0x00, 0x18, 0x2C, 0x40, 0x300
5. **Full sceSysreg init (step 6.3)** — Resolve and call all 14 sceSysreg NIDs
6. **Full chain + VBUS (step 6.4)** — Complete init: sceSysreg → sceSyscon → scePower → PHY → OHCI → GPIO
7. **Hook sceUsbActivate (step 6.5)** — Load USB camera modules, trace register changes

### Phase C: Hardware Acquisition

8. **Acquire Go!Cam** — Camera driver triggers VBUS enable, step 1.3 monitors changes

---

## File Inventory

### Crates
- `crates/oasis-usb-vbus-psp/` — Interactive kernel-mode VBUS test EBOOT (14 menu steps)
  - `src/main.rs` — Phase selector, GPIO/Syscon/OHCI/PHY tests, VBUS enable
  - `src/gpio.rs` — GPIO register read/write, NID resolution (4 NIDs + 0x317D9D2C)
  - `src/syscon.rs` — Syscon GET/SET packets, sceSysconCtrlUsbPower NID
  - `src/ohci.rs` — OHCI controller, clock enable, MUSB (disabled — bus-faults)
  - `src/phy.rs` — USB PHY configuration
  - `src/screen.rs` — Direct VRAM rendering (MSX font, instant menu refresh)

- `crates/oasis-prx-decrypt-psp/` — Flash0 PRX decryption EBOOT
  - Uses memlmd NID 0xEF73E85B (Kirk hardware engine)
  - Decrypts 15 PRXs from flash0:/kd/ to ms0:/PSP/GAME/PRXDEC/dec/

### Decrypted PRXs (at `/home/mikunpc/Downloads/USBTRACE/dec_661/`)
| File | Size | Format | Priority |
|---|---|---|---|
| usb.prx | 43,402 | ELF | **HIGH** — contains GPIO pin 23 VBUS code |
| lowio.prx | 56,052 | ELF | **HIGH** — GPIO driver implementation |
| syscon.prx | 22,036 | ELF | **HIGH** — Syscon command interface |
| usbcam.prx | 41,484 | ELF | MEDIUM — camera driver (triggers VBUS) |
| usbpspcm.prx | 18,418 | ELF | MEDIUM — USB communication |
| usbacc.prx | 3,884 | ELF | LOW — accessories |
| loadexec_09g.prx | 47,364 | KL4E | LOW — boot/reboot |
| usbstor.prx | 10,444 | ELF | LOW |
| usbstormgr.prx | 31,262 | ELF | LOW |
| usbstorms.prx | 24,540 | ELF | LOW |
| usbstorboot.prx | 3,761 | KL4E | LOW |
| usbmic.prx | 8,204 | ELF | LOW |
| usbgps.prx | 25,176 | ELF | LOW |
| usbdmb.prx | 9,691 | KL4E | LOW |
| usb1seg.prx | 23,542 | ELF | LOW |

### Analysis Scripts
- `scripts/ghidra_usb_vbus.py` — Ghidra headless script for usb.prx: NID mapping, GPIO trace, init chain
- `scripts/ghidra_lowio_gpio.py` — Ghidra headless script for lowio.prx: GPIO driver, Set→Output, AltFunc

### Analysis Data
- `/home/mikunpc/Downloads/USBTRACE/psp_kernel_4mb.bin` — 4MB kernel memory dump (decrypted drivers in memory)
- `/home/mikunpc/Downloads/USBTRACE/flash0_live_661.bin` — 41MB raw flash0 dump (FAT12)
- `/home/mikunpc/Downloads/USBTRACE/ghidra_firmware*_results.txt` — Prior Ghidra analysis
- `/home/mikunpc/Downloads/USBTRACE/ghidra_project/` — Existing Ghidra databases
- `/home/mikunpc/Downloads/USBTRACE/pspdecrypt/` — Desktop PRX decryption tool (lacks 6.61 keys)

### Logs
- `ms0:/PSP/GAME/USBVBUS/vbus.log` — VBUS test output (append mode)
- `ms0:/PSP/GAME/PRXDEC/decrypt.log` — PRX decryption results
