//! PSP PRX Decryptor — decrypt flash0 firmware modules using Kirk hardware.
//!
//! Reads encrypted PRX files from flash0:/kd/, decrypts them in RAM using
//! the PSP's built-in Kirk crypto engine (via memlmd), and writes the
//! decrypted ELF files to ms0:/PSP/GAME/PRXDEC/dec/.
//!
//! Must run as kernel-mode EBOOT on real PSP hardware (Kirk is not
//! available in PPSSPP).
//!
//! Controls:
//!   X        — Decrypt all target PRXs
//!   TRIANGLE — Exit

#![no_std]
#![no_main]

use core::ffi::c_void;
use psp::sys::{
    self, CtrlButtons, CtrlMode, IoOpenFlags, SceCtrlData, SceUid,
    sceCtrlPeekBufferPositive, sceCtrlSetSamplingMode,
    sceIoClose, sceIoOpen, sceIoRead, sceIoWrite,
    sceKernelDelayThread,
};

psp::module_kernel!("PRXDecrypt", 1, 0);

// ── Logging ─────────────────────────────────────────────────────────────

static LOG_PATH: &[u8] = b"ms0:/PSP/GAME/PRXDEC/decrypt.log\0";

fn log(s: &str) {
    psp::dprintln!("{}", s);
    let fd = unsafe {
        sceIoOpen(
            LOG_PATH.as_ptr(),
            IoOpenFlags::WR_ONLY | IoOpenFlags::CREAT | IoOpenFlags::APPEND,
            0o777,
        )
    };
    if fd.0 >= 0 {
        unsafe {
            sceIoWrite(fd, s.as_ptr() as *const c_void, s.len());
            sceIoWrite(fd, b"\n".as_ptr() as *const c_void, 1);
            sceIoClose(fd);
        }
    }
}

fn log_init() {
    let fd = unsafe {
        sceIoOpen(
            LOG_PATH.as_ptr(),
            IoOpenFlags::WR_ONLY | IoOpenFlags::CREAT | IoOpenFlags::TRUNC,
            0o777,
        )
    };
    if fd.0 >= 0 {
        unsafe { sceIoClose(fd) };
    }
}

// ── Hex formatting ──────────────────────────────────────────────────────

struct Fmt {
    buf: [u8; 256],
    pos: usize,
}

impl Fmt {
    fn new() -> Self {
        Self { buf: [0u8; 256], pos: 0 }
    }
    fn s(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.pos]) }
    }
    fn p(&mut self, s: &str) {
        let n = s.len().min(self.buf.len() - self.pos);
        self.buf[self.pos..self.pos + n].copy_from_slice(&s.as_bytes()[..n]);
        self.pos += n;
    }
    fn h8(&mut self, v: u32) {
        let hex = b"0123456789ABCDEF";
        for i in (0..8).rev() {
            if self.pos < self.buf.len() {
                self.buf[self.pos] = hex[((v >> (i * 4)) & 0xF) as usize];
                self.pos += 1;
            }
        }
    }
    fn decimal(&mut self, mut v: u32) {
        if v == 0 {
            if self.pos < self.buf.len() {
                self.buf[self.pos] = b'0';
                self.pos += 1;
            }
            return;
        }
        let mut digits = [0u8; 10];
        let mut n = 0usize;
        while v > 0 {
            digits[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
        }
        for i in (0..n).rev() {
            if self.pos < self.buf.len() {
                self.buf[self.pos] = digits[i];
                self.pos += 1;
            }
        }
    }
}

// ── memlmd decryption NID resolution ────────────────────────────────────

/// memlmd_EF73E85B — decrypt PRX buffer in-place
/// int memlmd_EF73E85B(u8 *buf, int size, int *newsize);
const NID_MEMLMD_DECRYPT_1: u32 = 0xEF73E85B;

/// memlmd_6192F715 — alternate decrypt
const NID_MEMLMD_DECRYPT_2: u32 = 0x6192F715;

/// sceUtilsBufferCopyWithRange — raw Kirk interface (fallback)
const NID_KIRK_COPY: u32 = 0x4C537C72;

type DecryptFn = unsafe extern "C" fn(*mut u8, i32, *mut i32) -> i32;

static mut DECRYPT_FN: Option<DecryptFn> = None;
static mut DECRYPT_NAME: &str = "none";

/// Resolve memlmd decrypt function.
unsafe fn resolve_decrypt() -> bool {
    let pairs: [(&[u8], &[u8], u32, &str); 6] = [
        (
            b"sceMesgLed\0", b"memlmd\0",
            NID_MEMLMD_DECRYPT_1, "memlmd_EF73E85B",
        ),
        (
            b"sceMesgLed\0", b"memlmd\0",
            NID_MEMLMD_DECRYPT_2, "memlmd_6192F715",
        ),
        (
            b"sceMemlmd\0", b"memlmd\0",
            NID_MEMLMD_DECRYPT_1, "sceMemlmd_EF73E85B",
        ),
        (
            b"sceMemlmd\0", b"memlmd\0",
            NID_MEMLMD_DECRYPT_2, "sceMemlmd_6192F715",
        ),
        (
            b"sceLowIO_Driver\0", b"sceNand_driver\0",
            NID_MEMLMD_DECRYPT_1, "lowio_EF73E85B",
        ),
        (
            b"sceMesgLed\0", b"sceMemlmd\0",
            NID_MEMLMD_DECRYPT_1, "mesgLed_sceMemlmd",
        ),
    ];

    for (module, library, nid, name) in &pairs {
        if let Some(addr) = psp::hook::find_function(
            module.as_ptr(), library.as_ptr(), *nid,
        ) {
            let mut f = Fmt::new();
            f.p("  Found ");
            f.p(name);
            f.p(" at ");
            f.h8(addr as u32);
            log(f.s());

            core::ptr::write_volatile(
                &raw mut DECRYPT_FN,
                Some(core::mem::transmute(addr)),
            );
            core::ptr::write_volatile(&raw mut DECRYPT_NAME, name);
            return true;
        }
    }

    false
}

// ── File I/O helpers ────────────────────────────────────────────────────

// 512KB static buffer — largest flash0 PRX is ~161KB
const BUF_SIZE: usize = 512 * 1024;
static mut PRX_BUF: [u8; BUF_SIZE] = [0u8; BUF_SIZE];

/// Read file into static buffer. Returns file size or 0 on failure.
fn read_file_static(path: &[u8]) -> usize {
    let fd = unsafe {
        sceIoOpen(path.as_ptr(), IoOpenFlags::RD_ONLY, 0)
    };
    if fd.0 < 0 {
        return 0;
    }

    let size = unsafe { sys::sceIoLseek(fd, 0, sys::IoWhence::End) };
    if size <= 0 || size as usize > BUF_SIZE {
        unsafe { sceIoClose(fd) };
        return 0;
    }
    unsafe { sys::sceIoLseek(fd, 0, sys::IoWhence::Set) };

    let size = size as usize;
    // Zero the buffer first
    unsafe {
        let ptr = (&raw mut PRX_BUF) as *mut u8;
        for i in 0..BUF_SIZE {
            *ptr.add(i) = 0;
        }
    }

    let read = unsafe {
        let ptr = (&raw mut PRX_BUF) as *mut c_void;
        sceIoRead(fd, ptr, size as u32)
    };
    unsafe { sceIoClose(fd) };

    if read as usize != size {
        return 0;
    }
    size
}

/// Write a buffer to a file. Returns true on success.
fn write_file(path: &[u8], data: &[u8]) -> bool {
    let fd = unsafe {
        sceIoOpen(
            path.as_ptr(),
            IoOpenFlags::WR_ONLY | IoOpenFlags::CREAT | IoOpenFlags::TRUNC,
            0o777,
        )
    };
    if fd.0 < 0 {
        return false;
    }

    let written = unsafe {
        sceIoWrite(fd, data.as_ptr() as *const c_void, data.len())
    };
    unsafe { sceIoClose(fd) };

    written as usize == data.len()
}

/// Create directory (ignore errors if exists).
fn mkdir(path: &[u8]) {
    unsafe { sys::sceIoMkdir(path.as_ptr(), 0o777) };
}

// ── PRX decryption ──────────────────────────────────────────────────────

/// Try to decrypt a PRX buffer in-place using memlmd.
/// Returns decrypted size on success, negative error on failure.
unsafe fn decrypt_prx(buf: &mut [u8], file_size: usize) -> i32 {
    let func = match core::ptr::read_volatile(&raw const DECRYPT_FN) {
        Some(f) => f,
        None => return -1,
    };

    let mut new_size: i32 = 0;
    // SAFETY: calling kernel memlmd decrypt function with valid buffer.
    let ret = unsafe { func(buf.as_mut_ptr(), file_size as i32, &mut new_size) };

    if ret >= 0 && new_size > 0 {
        new_size
    } else {
        // Return the error code
        if ret < 0 { ret } else { -2 }
    }
}

/// Load a PRX via sceKernelLoadModule (which decrypts it), then find
/// the decrypted module in memory and dump it. Returns decrypted size.
unsafe fn load_and_dump(flash_path: &[u8], out_path: &[u8]) -> i32 {
    // Load the module — this triggers Kirk decryption internally
    let mod_id = sys::sceKernelLoadModule(
        flash_path.as_ptr(),
        0,
        core::ptr::null_mut(),
    );

    if mod_id.0 < 0 {
        let mut f = Fmt::new();
        f.p("  LoadModule err: ");
        f.h8(mod_id.0 as u32);
        log(f.s());
        return mod_id.0;
    }

    let mut f = Fmt::new();
    f.p("  Module loaded, id=");
    f.h8(mod_id.0 as u32);
    log(f.s());

    // Query module info to find its base address and size
    let mut info: sys::SceKernelModuleInfo = core::mem::zeroed();
    info.size = core::mem::size_of::<sys::SceKernelModuleInfo>();
    let ret = sys::sceKernelQueryModuleInfo(mod_id, &mut info);

    if ret < 0 {
        let mut f = Fmt::new();
        f.p("  QueryModuleInfo err: ");
        f.h8(ret as u32);
        log(f.s());
        sys::sceKernelUnloadModule(mod_id);
        return ret;
    }

    // Get text segment info
    let text_addr = info.text_addr as *const u8;
    let text_size = info.text_size as usize;
    let data_size = info.data_size as usize;
    let total = text_size + data_size;

    let mut f = Fmt::new();
    f.p("  Addr=");
    f.h8(text_addr as u32);
    f.p(" text=");
    f.decimal(text_size as u32);
    f.p(" data=");
    f.decimal(data_size as u32);
    log(f.s());

    if total > 0 && !text_addr.is_null() {
        // Dump decrypted module memory (text + data) to file
        let slice = core::slice::from_raw_parts(text_addr, total);
        if write_file(out_path, slice) {
            log("  Dumped!");
        } else {
            log("  Write failed");
        }
    }

    // Unload the module
    sys::sceKernelUnloadModule(mod_id);

    total as i32
}

// ── Target PRX list ─────────────────────────────────────────────────────

struct PrxTarget {
    flash_path: &'static [u8],
    output_name: &'static [u8],
    label: &'static str,
}

const TARGETS: &[PrxTarget] = &[
    PrxTarget {
        flash_path: b"flash0:/kd/usb.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/usb.prx\0",
        label: "usb.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/usbcam.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/usbcam.prx\0",
        label: "usbcam.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/usbpspcm.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/usbpspcm.prx\0",
        label: "usbpspcm.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/usbacc.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/usbacc.prx\0",
        label: "usbacc.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/lowio.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/lowio.prx\0",
        label: "lowio.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/syscon.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/syscon.prx\0",
        label: "syscon.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/loadexec_09g.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/loadexec_09g.prx\0",
        label: "loadexec_09g.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/usbstor.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/usbstor.prx\0",
        label: "usbstor.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/usbstormgr.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/usbstormgr.prx\0",
        label: "usbstormgr.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/usbstorms.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/usbstorms.prx\0",
        label: "usbstorms.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/usbstorboot.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/usbstorboot.prx\0",
        label: "usbstorboot.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/usbmic.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/usbmic.prx\0",
        label: "usbmic.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/usbgps.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/usbgps.prx\0",
        label: "usbgps.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/usbdmb.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/usbdmb.prx\0",
        label: "usbdmb.prx",
    },
    PrxTarget {
        flash_path: b"flash0:/kd/usb1seg.prx\0",
        output_name: b"ms0:/PSP/GAME/PRXDEC/dec/usb1seg.prx\0",
        label: "usb1seg.prx",
    },
];

// ── Input helpers ───────────────────────────────────────────────────────

const NAV_BUTTONS: CtrlButtons = CtrlButtons::from_bits_truncate(
    CtrlButtons::CROSS.bits()
    | CtrlButtons::TRIANGLE.bits()
    | CtrlButtons::CIRCLE.bits()
    | CtrlButtons::START.bits()
);

fn wait_button() -> CtrlButtons {
    let mut p = SceCtrlData::default();
    loop {
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if !p.buttons.intersects(NAV_BUTTONS) { break; }
        unsafe { sceKernelDelayThread(16_000) };
    }
    loop {
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        let pressed = p.buttons & NAV_BUTTONS;
        if !pressed.is_empty() { return pressed; }
        unsafe { sceKernelDelayThread(16_000) };
    }
}

// ── Main ────────────────────────────────────────────────────────────────

fn psp_main() {
    let _ = psp::callback::setup_exit_callback();
    unsafe { sceCtrlSetSamplingMode(CtrlMode::Digital) };
    log_init();

    log("=== PSP PRX Decryptor ===");
    log("FW 6.61 / ARK-4 / Kirk HW");
    log("");

    // Create output directory
    mkdir(b"ms0:/PSP/GAME/PRXDEC/dec\0");

    // Resolve memlmd decrypt function
    log("Resolving memlmd decrypt NID...");
    let found = unsafe { resolve_decrypt() };

    if !found {
        log("ERROR: Could not resolve memlmd decrypt function!");
        log("Trying alternative: sceKernelCheckExecFile...");

        // Try alternative NIDs
        let alt_nids: [(&[u8], &[u8], u32, &str); 4] = [
            (b"sceLoadExec\0", b"LoadExecForKernel\0",
             0xD8320A28, "sceKernelCheckExecFile"),
            (b"sceMesgLed\0", b"sceMesgLed_driver\0",
             0xEF73E85B, "sceMesgLed_driver_EF73"),
            (b"sceMesgLed\0", b"sceMesgLed_driver\0",
             0x6192F715, "sceMesgLed_driver_6192"),
            (b"sceMemlmd\0", b"sceMesgLed_driver\0",
             0xEF73E85B, "sceMemlmd_mesgLed_EF73"),
        ];

        for (module, library, nid, name) in &alt_nids {
            unsafe {
                if let Some(addr) = psp::hook::find_function(
                    module.as_ptr(), library.as_ptr(), *nid,
                ) {
                    let mut f = Fmt::new();
                    f.p("  Found alt: ");
                    f.p(name);
                    f.p(" at ");
                    f.h8(addr as u32);
                    log(f.s());

                    core::ptr::write_volatile(
                        &raw mut DECRYPT_FN,
                        Some(core::mem::transmute(addr)),
                    );
                    core::ptr::write_volatile(&raw mut DECRYPT_NAME, name);
                    break;
                }
            }
        }

        if unsafe { core::ptr::read_volatile(&raw const DECRYPT_FN) }.is_none() {
            log("FATAL: No decrypt function found.");
            log("Press any button to exit.");
            wait_button();
            return;
        }
    }

    let name = unsafe { core::ptr::read_volatile(&raw const DECRYPT_NAME) };
    let mut f = Fmt::new();
    f.p("Using: ");
    f.p(name);
    log(f.s());
    log("");

    log("X=decrypt all, TRI=exit");
    let btn = wait_button();
    if btn.intersects(CtrlButtons::TRIANGLE) {
        return;
    }

    log("");
    let mut ok_count = 0u32;
    let mut fail_count = 0u32;

    for target in TARGETS {
        let mut f = Fmt::new();
        f.p("--- ");
        f.p(target.label);
        f.p(" ---");
        log(f.s());

        // Read encrypted PRX into static buffer
        log("  Reading...");
        let file_size = read_file_static(target.flash_path);
        if file_size == 0 {
            log("  SKIP: cannot read file");
            fail_count += 1;
            continue;
        }

        let mut f = Fmt::new();
        f.p("  Size: ");
        f.decimal(file_size as u32);
        log(f.s());

        // Log PRX header tag
        if file_size >= 0xD4 {
            let ptr = unsafe { (&raw const PRX_BUF) as *const u8 };
            let tag = u32::from_le_bytes(unsafe { [
                *ptr.add(0xD0), *ptr.add(0xD1),
                *ptr.add(0xD2), *ptr.add(0xD3),
            ] });
            let mut f = Fmt::new();
            f.p("  Tag: ");
            f.h8(tag);
            log(f.s());
        }

        // Try to decrypt
        log("  Decrypting...");
        let result = unsafe {
            let buf = core::slice::from_raw_parts_mut(
                (&raw mut PRX_BUF) as *mut u8, BUF_SIZE,
            );
            decrypt_prx(buf, file_size)
        };

        if result > 0 {
            let dec_size = result as usize;
            let mut f = Fmt::new();
            f.p("  OK! Size: ");
            f.decimal(dec_size as u32);
            log(f.s());

            // Check magic
            let ptr = unsafe { (&raw const PRX_BUF) as *const u8 };
            if dec_size >= 4 {
                let magic = u32::from_le_bytes(unsafe { [
                    *ptr.add(0), *ptr.add(1),
                    *ptr.add(2), *ptr.add(3),
                ] });
                let mut f = Fmt::new();
                f.p("  Magic: ");
                f.h8(magic);
                if magic == 0x464C457F {
                    f.p(" (ELF!)");
                } else if magic == 0x5053507E {
                    f.p(" (~PSP encrypted)");
                }
                log(f.s());
            }

            // Write decrypted file
            let out_slice = unsafe {
                core::slice::from_raw_parts(ptr, dec_size)
            };
            if write_file(target.output_name, out_slice) {
                log("  Saved!");
                ok_count += 1;
            } else {
                log("  Write failed");
                fail_count += 1;
            }
        } else {
            let mut f = Fmt::new();
            f.p("  memlmd failed: ");
            f.h8(result as u32);
            log(f.s());

            // Fallback: load module and dump from memory
            log("  Trying LoadModule+dump...");
            let dump_size = unsafe {
                load_and_dump(target.flash_path, target.output_name)
            };
            if dump_size > 0 {
                let mut f = Fmt::new();
                f.p("  Dumped ");
                f.decimal(dump_size as u32);
                f.p(" bytes");
                log(f.s());
                ok_count += 1;
            } else {
                let mut f = Fmt::new();
                f.p("  LoadModule also failed: ");
                f.h8(dump_size as u32);
                log(f.s());
                fail_count += 1;
            }
        }

        log("");
        unsafe { sceKernelDelayThread(100_000) };
    }

    // Summary
    log("=== Summary ===");
    let mut f = Fmt::new();
    f.p("Decrypted: ");
    f.decimal(ok_count);
    f.p("  Failed: ");
    f.decimal(fail_count);
    log(f.s());
    log("");
    log("Output: ms0:/PSP/GAME/PRXDEC/dec/");
    log("Log: ms0:/PSP/GAME/PRXDEC/decrypt.log");
    log("");
    log("Press any button to exit.");
    wait_button();

    unsafe { sys::sceKernelExitDeleteThread(0) };
}
