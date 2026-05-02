//! PSP USB Host Mode — Phase 0: Zero-RE VBUS Experiment
//!
//! Minimal user-mode EBOOT that attempts to enable VBUS output by calling
//! Sony's camera driver init sequence. Run with an FNB58 inline power
//! meter connected via Mini-B OTG adapter to detect any VBUS voltage.
//!
//! Steps:
//!   1. Load USB accessory + camera modules via sceUtilityLoadUsbModule
//!   2. sceUsbStart("USBBusDriver")
//!   3. sceUsbStart("USBCamDriver")
//!   4. sceUsbActivate(USB_CAM_PID = 0x282)
//!   5. Poll sceUsbGetState() and display flags continuously
//!
//! All results are displayed on screen and logged to:
//!   ms0:/PSP/GAME/USBPHASE0/usb_phase0.log
//!
//! Controls:
//!   CROSS    — Advance to next step (step-by-step mode)
//!   CIRCLE   — Run all steps automatically
//!   TRIANGLE — Deactivate/stop USB (cleanup)
//!   START    — Exit

#![no_std]
#![no_main]

use core::ffi::c_void;

use psp::sys::{
    self, CtrlButtons, CtrlMode, IoOpenFlags, SceCtrlData, UsbState,
    sceCtrlPeekBufferPositive, sceCtrlSetSamplingMode,
    sceKernelDelayThread,
    sceUsbActivate, sceUsbDeactivate, sceUsbGetDrvState, sceUsbGetState, sceUsbStart, sceUsbStop,
};

psp::module!("USBHostPhase0", 1, 0);

// ── Log file ────────────────────────────────────────────────────────────

const LOG_PATH: &str = "ms0:/PSP/GAME/USBPHASE0/usb_phase0.log";

/// Global log file descriptor. Opened once at startup, closed on exit.
static mut LOG_FD: psp::sys::SceUid = psp::sys::SceUid(0);

/// Open the log file (truncate + create).
fn log_open() {
    let mut path_buf = [0u8; 256];
    let bytes = LOG_PATH.as_bytes();
    path_buf[..bytes.len()].copy_from_slice(bytes);
    path_buf[bytes.len()] = 0;

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe {
        LOG_FD = psp::sys::sceIoOpen(
            path_buf.as_ptr(),
            IoOpenFlags::WR_ONLY | IoOpenFlags::CREAT | IoOpenFlags::TRUNC,
            0o777,
        );
    }
}

/// Write a line to the log file.
fn log_write(msg: &str) {
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe {
        if LOG_FD.0 > 0 {
            psp::sys::sceIoWrite(
                LOG_FD,
                msg.as_ptr() as *const c_void,
                msg.len(),
            );
            psp::sys::sceIoWrite(
                LOG_FD,
                b"\n".as_ptr() as *const c_void,
                1,
            );
        }
    }
}

/// Close the log file.
fn log_close() {
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe {
        if LOG_FD.0 > 0 {
            psp::sys::sceIoClose(LOG_FD);
            LOG_FD = psp::sys::SceUid(0);
        }
    }
}

// ── Display helpers ─────────────────────────────────────────────────────

/// Print a line to both screen and log file.
macro_rules! log_println {
    ($($arg:tt)*) => {{
        psp::dprintln!($($arg)*);
        // Format into a stack buffer for the log file.
        let mut buf = [0u8; 256];
        let len = fmt_to_buf(&mut buf, format_args!($($arg)*));
        #[allow(unused_unsafe)]
        // SAFETY: fmt_to_buf only writes valid UTF-8 bytes from format_args.
        let s = unsafe { core::str::from_utf8_unchecked(&buf[..len]) };
        log_write(s);
    }};
}

/// Format `core::fmt::Arguments` into a fixed buffer. Returns bytes written.
fn fmt_to_buf(buf: &mut [u8; 256], args: core::fmt::Arguments<'_>) -> usize {
    use core::fmt::Write;

    struct BufWriter<'a> {
        buf: &'a mut [u8; 256],
        pos: usize,
    }

    impl<'a> Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let remaining = 256 - self.pos;
            // Truncate at a UTF-8 character boundary to avoid splitting
            // a multibyte codepoint (which would make from_utf8_unchecked UB).
            let to_copy = s.floor_char_boundary(bytes.len().min(remaining));
            self.buf[self.pos..self.pos + to_copy]
                .copy_from_slice(&bytes[..to_copy]);
            self.pos += to_copy;
            Ok(())
        }
    }

    let mut writer = BufWriter { buf, pos: 0 };
    let _ = core::fmt::write(&mut writer, args);
    writer.pos
}

// ── USB state formatting ────────────────────────────────────────────────

fn format_usb_state(bits: i32) -> &'static str {
    // Known flags: ACTIVATED=0x200, CONNECTED=0x020, ESTABLISHED=0x002
    // Undocumented: bit0=0x001, bit4=0x010, bit8=0x100 (seen as 0x111 on PSP-3000)
    match bits {
        0x000 => "NONE",
        0x111 => "HW_IDLE",
        0x200 => "ACT",
        0x311 => "HW_IDLE+ACT",
        0x220 => "ACT+CON",
        0x331 => "HW_IDLE+ACT+CON",
        0x222 => "ACT+CON+EST",
        0x333 => "HW_IDLE+ACT+CON+EST",
        0x020 => "CON",
        0x131 => "HW_IDLE+CON",
        0x022 => "CON+EST",
        0x002 => "EST",
        _ => "OTHER",
    }
}

// ── Controller input ────────────────────────────────────────────────────

/// Wait for a specific button press (edge detection).
fn wait_button_press(mask: CtrlButtons) {
    let mut pad = SceCtrlData::default();

    // Wait for button to be released first
    loop {
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceCtrlPeekBufferPositive(&mut pad, 1) };
        if !pad.buttons.intersects(mask) {
            break;
        }
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceKernelDelayThread(16_000) };
    }

    // Wait for button to be pressed
    loop {
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceCtrlPeekBufferPositive(&mut pad, 1) };
        if pad.buttons.intersects(mask) {
            return;
        }
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceKernelDelayThread(16_000) };
    }
}

/// Check if a button is currently pressed (no edge detection).
fn is_pressed(buttons: CtrlButtons) -> bool {
    let mut pad = SceCtrlData::default();
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceCtrlPeekBufferPositive(&mut pad, 1) };
    pad.buttons.intersects(buttons)
}

// ── Battery info ────────────────────────────────────────────────────────

fn print_battery_status() {
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe {
        let charging = sys::scePowerIsBatteryCharging();
        let percent = sys::scePowerGetBatteryLifePercent();
        let voltage = sys::scePowerGetBatteryVolt();
        let ac = sys::scePowerIsPowerOnline();

        log_println!(
            "Battery: {}%  {}mV  AC:{}  Chg:{}",
            percent,
            voltage,
            if ac != 0 { "Y" } else { "N" },
            if charging != 0 { "Y" } else { "N" },
        );
    }
}

// ── USB experiment steps ────────────────────────────────────────────────

/// Step 1: Load USB modules via sceUtilityLoadUsbModule.
fn step_load_modules() {
    log_println!("--- Step 1: Load USB modules ---");

    // Load USBPspCm (base USB communication)
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret = unsafe { sys::sceUtilityLoadUsbModule(sys::UsbModule::UsbPspCm) };
    log_println!("  LoadUsbModule(PspCm)  = 0x{:08X}", ret as u32);

    // Load USBAcc (accessory framework, required before camera)
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret = unsafe { sys::sceUtilityLoadUsbModule(sys::UsbModule::UsbAcc) };
    log_println!("  LoadUsbModule(Acc)    = 0x{:08X}", ret as u32);

    // Load USBCam (camera driver — triggers host mode init)
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret = unsafe { sys::sceUtilityLoadUsbModule(sys::UsbModule::UsbCam) };
    log_println!("  LoadUsbModule(Cam)    = 0x{:08X}", ret as u32);

    // Small delay for module initialization
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(100_000) };
}

/// Step 2: Start USB bus driver.
fn step_start_bus() -> i32 {
    log_println!("--- Step 2: Start USB bus driver ---");

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret = unsafe {
        sceUsbStart(
            b"USBBusDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        )
    };
    log_println!("  sceUsbStart(USBBusDriver) = 0x{:08X}", ret as u32);

    // Check driver state
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let drv_state = unsafe { sceUsbGetDrvState(b"USBBusDriver\0".as_ptr()) };
    log_println!("  USBBusDriver state       = 0x{:08X}", drv_state as u32);

    // Check USB state after bus start
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let bits = unsafe { sceUsbGetState() }.bits();
    log_println!(
        "  USB state: 0x{:03X} [{}]",
        bits,
        format_usb_state(bits),
    );

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(100_000) };
    ret
}

/// Step 3: Start USB camera driver.
fn step_start_cam() -> i32 {
    log_println!("--- Step 3: Start USB camera driver ---");

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret = unsafe {
        sceUsbStart(
            b"USBCamDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        )
    };
    log_println!("  sceUsbStart(USBCamDriver) = 0x{:08X}", ret as u32);

    // Also try starting camera mic driver
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret_mic = unsafe {
        sceUsbStart(
            b"USBCamMicDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        )
    };
    log_println!("  sceUsbStart(CamMicDriver) = 0x{:08X}", ret_mic as u32);

    // Check driver states
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let drv_state = unsafe { sceUsbGetDrvState(b"USBCamDriver\0".as_ptr()) };
    log_println!("  USBCamDriver state        = 0x{:08X}", drv_state as u32);

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let bits = unsafe { sceUsbGetState() }.bits();
    log_println!(
        "  USB state: 0x{:03X} [{}]",
        bits,
        format_usb_state(bits),
    );

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(100_000) };
    ret
}

/// Step 4: Activate USB with camera PID.
fn step_activate() -> i32 {
    log_println!("--- Step 4: Activate USB (PID=0x282) ---");

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret = unsafe { sceUsbActivate(0x282) };
    log_println!("  sceUsbActivate(0x282) = 0x{:08X}", ret as u32);

    // Wait a moment for activation
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(500_000) };

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let bits = unsafe { sceUsbGetState() }.bits();
    log_println!(
        "  USB state: 0x{:03X} [{}]",
        bits,
        format_usb_state(bits),
    );

    // Check if cable is connected
    if bits & UsbState::CONNECTED.bits() != 0 {
        log_println!("  >> Cable CONNECTED");
    } else {
        log_println!("  >> No cable detected");
    }

    ret
}

/// Cleanup: deactivate and stop USB drivers.
fn step_cleanup() {
    log_println!("--- Cleanup: Stopping USB ---");

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret = unsafe { sceUsbDeactivate(0x282) };
    log_println!("  sceUsbDeactivate(0x282) = 0x{:08X}", ret as u32);

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(100_000) };

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret = unsafe {
        sceUsbStop(
            b"USBCamMicDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        )
    };
    log_println!("  sceUsbStop(CamMicDriver) = 0x{:08X}", ret as u32);

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret = unsafe {
        sceUsbStop(
            b"USBCamDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        )
    };
    log_println!("  sceUsbStop(USBCamDriver) = 0x{:08X}", ret as u32);

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret = unsafe {
        sceUsbStop(
            b"USBBusDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        )
    };
    log_println!("  sceUsbStop(USBBusDriver) = 0x{:08X}", ret as u32);

    // Unload modules in reverse order
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret = unsafe { sys::sceUtilityUnloadUsbModule(sys::UsbModule::UsbCam) };
    log_println!("  UnloadUsbModule(Cam)  = 0x{:08X}", ret as u32);
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret = unsafe { sys::sceUtilityUnloadUsbModule(sys::UsbModule::UsbAcc) };
    log_println!("  UnloadUsbModule(Acc)  = 0x{:08X}", ret as u32);
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let ret = unsafe { sys::sceUtilityUnloadUsbModule(sys::UsbModule::UsbPspCm) };
    log_println!("  UnloadUsbModule(PspCm)= 0x{:08X}", ret as u32);

    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let bits = unsafe { sceUsbGetState() }.bits();
    log_println!(
        "  USB state: 0x{:03X} [{}]",
        bits,
        format_usb_state(bits),
    );
}

// ── State polling loop ──────────────────────────────────────────────────

/// Poll USB state continuously, displaying on screen and logging changes.
/// Returns when START is pressed.
fn poll_loop() {
    log_println!("");
    log_println!("=== Polling USB state ===");
    log_println!("Check FNB58 for VBUS voltage!");
    log_println!("TRIANGLE=cleanup  START=exit");
    log_println!("");

    let mut prev_state_bits: i32 = -1;
    let mut poll_count: u32 = 0;

    loop {
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        let bits = unsafe { sceUsbGetState() }.bits();

        // Only log when state changes (avoid flooding the log)
        if bits != prev_state_bits {
            log_println!(
                "[{}] USB: 0x{:03X} [{}]",
                poll_count,
                bits,
                format_usb_state(bits),
            );
            prev_state_bits = bits;
        }

        // Check for TRIANGLE (cleanup)
        if is_pressed(CtrlButtons::TRIANGLE) {
            step_cleanup();
            prev_state_bits = -1; // Force re-log after cleanup
            // Wait for button release
            while is_pressed(CtrlButtons::TRIANGLE) {
                // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
                unsafe { sceKernelDelayThread(16_000) };
            }
        }

        // Check for START (exit)
        if is_pressed(CtrlButtons::START) {
            log_println!("START pressed — exiting poll loop");
            return;
        }

        poll_count += 1;
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceKernelDelayThread(500_000) }; // Poll every 500ms
    }
}

// ── Main ────────────────────────────────────────────────────────────────

fn psp_main() {
    psp::callback::setup_exit_callback().unwrap();

    // Set analog stick to digital mode
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceCtrlSetSamplingMode(CtrlMode::Digital) };

    // Open log file
    log_open();

    // Header
    log_println!("================================");
    log_println!(" USB Host Mode — Phase 0");
    log_println!(" Zero-RE VBUS Experiment");
    log_println!("================================");
    log_println!("");

    // Show battery status (important for VBUS experiment)
    print_battery_status();
    log_println!("");

    // Show initial USB state
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    let bits = unsafe { sceUsbGetState() }.bits();
    log_println!(
        "Initial USB state: 0x{:03X} [{}]",
        bits,
        format_usb_state(bits),
    );
    log_println!("");

    // Prompt user
    log_println!("CROSS=step-by-step  O=run all");
    log_println!("START=exit");
    log_println!("");

    // Wait for user choice
    let auto_mode;
    loop {
        if is_pressed(CtrlButtons::CROSS) {
            auto_mode = false;
            break;
        }
        if is_pressed(CtrlButtons::CIRCLE) {
            auto_mode = true;
            break;
        }
        if is_pressed(CtrlButtons::START) {
            log_println!("Exiting without running.");
            log_close();
            return;
        }
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceKernelDelayThread(16_000) };
    }

    // Debounce
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(200_000) };

    if auto_mode {
        log_println!(">> Running all steps...");
    } else {
        log_println!(">> Step-by-step mode");
    }
    log_println!("");

    // Step 1: Load USB modules
    step_load_modules();
    if !auto_mode {
        log_println!("Press CROSS to continue...");
        wait_button_press(CtrlButtons::CROSS);
    }

    // Step 2: Start bus driver
    step_start_bus();
    if !auto_mode {
        log_println!("Press CROSS to continue...");
        wait_button_press(CtrlButtons::CROSS);
    }

    // Step 3: Start camera driver
    step_start_cam();
    if !auto_mode {
        log_println!("Press CROSS to continue...");
        wait_button_press(CtrlButtons::CROSS);
    }

    // Step 4: Activate with camera PID
    step_activate();
    log_println!("");

    // Show battery status again (check for any VBUS power draw changes)
    print_battery_status();

    // Enter polling loop
    poll_loop();

    // Cleanup before exit
    step_cleanup();
    log_println!("");
    log_println!("Phase 0 complete. Check log at:");
    log_println!("  {}", LOG_PATH);

    log_close();

    // Brief pause so user can read final message
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(2_000_000) };
}
