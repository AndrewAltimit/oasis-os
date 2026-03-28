//! Remote development loop: automated EBOOT deployment and testing.
//!
//! Enables Claude Code to iterate on PSP builds without human intervention:
//! 1. Plugin activates USB storage on XMB
//! 2. Host copies EBOOT + writes command file, then ejects USB
//! 3. Plugin detects USB disconnect → reads command → launches EBOOT
//! 4. EBOOT runs (with watchdog) → exits or is killed → back to XMB
//! 5. Plugin re-activates USB → host reads logs → repeat
//!
//! Also supports remote screenshots via framebuffer capture.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// USB Mass Storage PID.
const USB_STOR_PID: u32 = 0x1C8;

/// Command file path (host writes this, plugin reads it).
const CMD_FILE: &[u8] = b"ms0:/seplugins/.devloop_cmd\0";

/// Status file path (plugin writes this, host reads it).
const STATUS_FILE: &[u8] = b"ms0:/seplugins/.devloop_status\0";

/// Log file path (append-mode event log).
const LOG_FILE: &[u8] = b"ms0:/seplugins/.devloop_log\0";

/// Screenshot output path.
const SCREENSHOT_FILE: &[u8] = b"ms0:/seplugins/.devloop_screenshot.raw\0";

/// Whether the devloop is enabled.
static DEVLOOP_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Watchdog timestamp — EBOOT must update this to avoid being killed.
static WATCHDOG_DEADLINE: AtomicU32 = AtomicU32::new(0);

// -----------------------------------------------------------------------
// Logging
// -----------------------------------------------------------------------

fn devlog(msg: &[u8]) {
    unsafe {
        let fd = psp::sys::sceIoOpen(
            LOG_FILE.as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            psp::sys::sceIoWrite(fd, msg.as_ptr() as *const _, msg.len());
            psp::sys::sceIoWrite(fd, b"\n".as_ptr() as *const _, 1);
            psp::sys::sceIoClose(fd);
        }
    }
}

fn write_status(status: &[u8]) {
    unsafe {
        let fd = psp::sys::sceIoOpen(
            STATUS_FILE.as_ptr(),
            psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY
                | psp::sys::IoOpenFlags::TRUNC,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            psp::sys::sceIoWrite(fd, status.as_ptr() as *const _, status.len());
            psp::sys::sceIoClose(fd);
        }
    }
}

// -----------------------------------------------------------------------
// USB Storage Management
// -----------------------------------------------------------------------

// USB storage is managed by AlwaysUSB plugin — no USB code needed here.

// -----------------------------------------------------------------------
// Command File Parsing
// -----------------------------------------------------------------------

/// Parsed command from the host.
enum DevCmd {
    /// Launch an EBOOT at the given path with timeout in seconds.
    Launch {
        path: [u8; 128],
        path_len: usize,
        timeout_secs: u32,
        wifi: bool,
    },
    /// Take a screenshot and save to .devloop_screenshot.raw.
    Screenshot,
    /// Reboot the PSP.
    Reboot,
    /// No-op (empty or unrecognized command).
    None,
}

fn read_command() -> DevCmd {
    let mut buf = [0u8; 512];
    let fd = unsafe {
        psp::sys::sceIoOpen(
            CMD_FILE.as_ptr(),
            psp::sys::IoOpenFlags::RD_ONLY,
            0,
        )
    };
    if fd < psp::sys::SceUid(0) {
        return DevCmd::None;
    }
    let n = unsafe {
        psp::sys::sceIoRead(fd, buf.as_mut_ptr() as *mut c_void, 512)
    };
    unsafe { psp::sys::sceIoClose(fd) };

    // Delete command file after reading (one-shot).
    unsafe { psp::sys::sceIoRemove(CMD_FILE.as_ptr()) };

    if n <= 0 {
        return DevCmd::None;
    }

    let text = &buf[..n as usize];

    // Simple line parser: key = value
    let mut cmd_type = b"" as &[u8];
    let mut path = [0u8; 128];
    let mut path_len = 0usize;
    let mut timeout = 60u32;
    let mut wifi = false;

    // Parse lines without allocation.
    let mut pos = 0usize;
    while pos < n as usize {
        // Find end of line.
        let line_start = pos;
        while pos < n as usize && text[pos] != b'\n' {
            pos += 1;
        }
        let line_end = if pos > line_start && text[pos - 1] == b'\r' {
            pos - 1
        } else {
            pos
        };
        pos += 1; // skip newline

        let line = &text[line_start..line_end];

        // Find '=' separator.
        let eq_pos = match line.iter().position(|&b| b == b'=') {
            Some(p) => p,
            None => continue,
        };

        // Extract key (trim spaces).
        let mut key_end = eq_pos;
        while key_end > 0 && line[key_end - 1] == b' ' {
            key_end -= 1;
        }
        let key = &line[..key_end];

        // Extract value (trim leading spaces).
        let mut val_start = eq_pos + 1;
        while val_start < line.len() && line[val_start] == b' ' {
            val_start += 1;
        }
        let val = &line[val_start..];

        if key == b"cmd" {
            cmd_type = if val == b"launch" {
                b"launch"
            } else if val == b"screenshot" {
                b"screenshot"
            } else if val == b"reboot" {
                b"reboot"
            } else {
                b""
            };
        } else if key == b"path" {
            let take = val.len().min(127);
            path[..take].copy_from_slice(&val[..take]);
            path[take] = 0;
            path_len = take;
        } else if key == b"timeout" {
            timeout = 0;
            for &b in val {
                if b >= b'0' && b <= b'9' {
                    timeout = timeout * 10 + (b - b'0') as u32;
                }
            }
        } else if key == b"wifi" {
            wifi = val == b"true" || val == b"1";
        }
    }

    match cmd_type {
        b"launch" => DevCmd::Launch { path, path_len, timeout_secs: timeout, wifi },
        b"screenshot" => DevCmd::Screenshot,
        b"reboot" => DevCmd::Reboot,
        _ => DevCmd::None,
    }
}

// -----------------------------------------------------------------------
// Screenshot
// -----------------------------------------------------------------------

fn take_screenshot() {
    // Read the current framebuffer (VRAM at 0x04000000, 512x272 ABGR).
    const FB_WIDTH: usize = 512;
    const FB_HEIGHT: usize = 272;
    const FB_BPP: usize = 4;
    const FB_SIZE: usize = FB_WIDTH * FB_HEIGHT * FB_BPP;

    let fb_ptr = 0x44000000u32 as *const u8; // uncached VRAM

    unsafe {
        let fd = psp::sys::sceIoOpen(
            SCREENSHOT_FILE.as_ptr(),
            psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY
                | psp::sys::IoOpenFlags::TRUNC,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            psp::sys::sceIoWrite(fd, fb_ptr as *const c_void, FB_SIZE);
            psp::sys::sceIoClose(fd);
            devlog(b"[DEV] screenshot saved");
        }
    }
}

// -----------------------------------------------------------------------
// EBOOT Launcher
// -----------------------------------------------------------------------

/// Launch an EBOOT using sctrlKernelLoadExecVSHWithApitype.
/// Resolves the function at runtime from SystemCtrlForKernel.
fn launch_eboot(path: &[u8]) -> bool {
    // NID for sctrlKernelLoadExecVSHWithApitype in ARK-4.
    const NID_LOAD_EXEC_VSH: u32 = 0x1DDDAD0C;

    let func_ptr = unsafe {
        psp::hook::find_function(
            b"SystemControl\0".as_ptr(),
            b"SystemCtrlForKernel\0".as_ptr(),
            NID_LOAD_EXEC_VSH,
        )
    };

    // Also try alternative module names.
    let func_ptr = func_ptr.or_else(|| unsafe {
        psp::hook::find_function(
            b"SystemCtrlForKernel\0".as_ptr(),
            b"SystemCtrlForKernel\0".as_ptr(),
            NID_LOAD_EXEC_VSH,
        )
    });

    let Some(fp) = func_ptr else {
        devlog(b"[DEV] LoadExecVSH not found");
        return false;
    };

    // SceKernelLoadExecVSHParam struct.
    #[repr(C)]
    struct LoadExecParam {
        size: u32,
        args: u32,
        argp: *mut c_void,
        key: *const u8,
        vshmain_args_size: u32,
        vshmain_args: *mut c_void,
        configfile: *const u8,
        unk4: u32,
        unk5: u32,
    }

    let mut param = LoadExecParam {
        size: core::mem::size_of::<LoadExecParam>() as u32,
        args: path.len() as u32 + 1,
        argp: path.as_ptr() as *mut c_void,
        key: b"game\0".as_ptr(),
        vshmain_args_size: 0,
        vshmain_args: core::ptr::null_mut(),
        configfile: b"/kd/pspbtcnf_game.txt\0".as_ptr(),
        unk4: 0,
        unk5: 0,
    };

    type LoadExecFn = unsafe extern "C" fn(
        i32, *const u8, *mut LoadExecParam,
    ) -> i32;
    let load_exec: LoadExecFn = unsafe { core::mem::transmute(fp) };

    devlog(b"[DEV] launching EBOOT...");
    let ret = unsafe { load_exec(0x141, path.as_ptr(), &mut param) };
    devlog(b"[DEV] LoadExec returned (shouldn't happen)");
    ret >= 0
}

// -----------------------------------------------------------------------
// Main Devloop Thread
// -----------------------------------------------------------------------

/// Start the devloop background thread.
pub fn start() {
    if DEVLOOP_ACTIVE.swap(true, Ordering::Relaxed) {
        return; // already started
    }

    unsafe {
        let thid = psp::sys::sceKernelCreateThread(
            b"OasisDev\0".as_ptr(),
            devloop_thread,
            0x20, // priority
            8192, // stack
            psp::sys::ThreadAttributes::empty(),
            core::ptr::null_mut(),
        );
        if thid >= psp::sys::SceUid(0) {
            psp::sys::sceKernelStartThread(thid, 0, core::ptr::null_mut());
            devlog(b"[DEV] devloop thread started");
        } else {
            devlog(b"[DEV] thread create failed");
            DEVLOOP_ACTIVE.store(false, Ordering::Relaxed);
        }
    }
}

unsafe extern "C" fn devloop_thread(
    _args: usize,
    _argp: *mut c_void,
) -> i32 {
    // Initial delay — let the system settle.
    psp::sys::sceKernelDelayThread(3_000_000); // 3 seconds

    // USB storage is managed by AlwaysUSB plugin (Mode=1).
    // We just poll for the command file. When AlwaysUSB is active,
    // ms0: is NOT accessible from PSP (host has exclusive access).
    // We detect USB disconnect (AlwaysUSB deactivates when cable
    // is pulled or host ejects) and then read the command file.
    write_status(b"ready");
    devlog(b"[DEV] waiting for commands...");

    loop {
        // Check if command file exists (only works when USB is not active,
        // i.e., after host ejects or cable disconnected).
        match read_command() {
            DevCmd::Launch { path, path_len, timeout_secs, wifi: _ } => {
                write_status(b"launching");
                devlog(b"[DEV] cmd=launch");

                // Set watchdog deadline.
                let now = psp::sys::sceKernelGetSystemTimeLow();
                WATCHDOG_DEADLINE.store(
                    now.wrapping_add(timeout_secs * 1_000_000),
                    Ordering::Release,
                );

                // Launch the EBOOT (this call doesn't return on success).
                launch_eboot(&path[..path_len + 1]);

                // If we get here, launch failed.
                devlog(b"[DEV] launch failed");
                write_status(b"error_launch");
            }
            DevCmd::Screenshot => {
                devlog(b"[DEV] cmd=screenshot");
                take_screenshot();
                write_status(b"screenshot_done");
            }
            DevCmd::Reboot => {
                devlog(b"[DEV] cmd=reboot");
                let nid: u32 = 0x0442D852;
                if let Some(fp) = psp::hook::find_function(
                    b"scePower_Service\0".as_ptr(),
                    b"scePower\0".as_ptr(),
                    nid,
                ) {
                    let reboot: unsafe extern "C" fn(i32) -> i32 =
                        core::mem::transmute(fp);
                    reboot(0);
                }
            }
            DevCmd::None => {
                // No command — sleep and check again.
            }
        }

        psp::sys::sceKernelDelayThread(2_000_000); // poll every 2 seconds
    }
}
