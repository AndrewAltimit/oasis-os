//! Kernel PRX for remote PSP development automation.
//!
//! File-based command protocol via ms0: (shared with AlwaysUSB).
//! Polls ms0:/seplugins/devloop_cmd.txt every 2 seconds.
//! If non-empty, reads command, truncates file, executes.
//!
//! Commands: reboot, launch <path>, screenshot

#![no_std]
#![no_main]

psp::module_kernel!("OasisDevloop", 1, 0);

use core::ffi::c_void;

const LOG_PATH: *const u8 = b"ms0:/seplugins/devloop.log\0".as_ptr();

fn hex_u32(val: u32, out: &mut [u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for i in 0..4 {
        let b = ((val >> (24 - i * 8)) & 0xFF) as u8;
        out[i * 2] = HEX[(b >> 4) as usize];
        out[i * 2 + 1] = HEX[(b & 0xF) as usize];
    }
}
const CMD_PATH: *const u8 = b"ms0:/seplugins/devloop_cmd.txt\0".as_ptr();

fn log(msg: &[u8]) {
    unsafe {
        let fd = psp::sys::sceIoOpen(
            LOG_PATH,
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

fn take_screenshot() {
    const FB_SIZE: usize = 512 * 272 * 4;
    unsafe {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/seplugins/devloop_screen.raw\0".as_ptr(),
            psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY
                | psp::sys::IoOpenFlags::TRUNC,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            psp::sys::sceIoWrite(fd, 0x44000000u32 as *const c_void, FB_SIZE);
            psp::sys::sceIoClose(fd);
        }
    }
}

fn launch_eboot(path: &[u8]) {
    let mods: &[(&[u8], &[u8])] = &[
        (b"SystemControl\0", b"SystemCtrlForKernel\0"),
        (b"SystemCtrlForKernel\0", b"SystemCtrlForKernel\0"),
        (b"ARKCompatLayer\0", b"SystemCtrlForKernel\0"),
    ];
    for &(m, l) in mods {
        if let Some(fp) = unsafe {
            psp::hook::find_function(m.as_ptr(), l.as_ptr(), 0x1DDDAD0C)
        } {
            #[repr(C)]
            struct P {
                size: u32, args: u32, argp: *const u8, key: *const u8,
                vs_sz: u32, vs_p: *const u8, cfg: *const u8, u0: u32, u1: u32,
            }
            let mut p = P {
                size: core::mem::size_of::<P>() as u32,
                args: path.len() as u32, argp: path.as_ptr(),
                key: b"game\0".as_ptr(),
                vs_sz: 0, vs_p: core::ptr::null(),
                cfg: b"/kd/pspbtcnf_game.txt\0".as_ptr(),
                u0: 0, u1: 0,
            };
            type F = unsafe extern "C" fn(i32, *const u8, *mut P) -> i32;
            let f: F = unsafe { core::mem::transmute(fp) };
            log(b"[DL] launching");
            unsafe { f(0x141, path.as_ptr(), &mut p) };
            return;
        }
    }
    log(b"[DL] LoadExecVSH not found");
}

/// Read command file, return bytes read (0 if empty/missing).
fn read_cmd(buf: &mut [u8]) -> usize {
    unsafe {
        let fd = psp::sys::sceIoOpen(CMD_PATH, psp::sys::IoOpenFlags::RD_ONLY, 0);
        if fd < psp::sys::SceUid(0) { return 0; }
        let n = psp::sys::sceIoRead(fd, buf.as_mut_ptr() as *mut c_void, buf.len() as u32);
        psp::sys::sceIoClose(fd);
        if n <= 0 { return 0; }
        n as usize
    }
}

/// Create/clear the command file (truncate to 0).
fn clear_cmd() {
    unsafe {
        let fd = psp::sys::sceIoOpen(
            CMD_PATH,
            psp::sys::IoOpenFlags::WR_ONLY
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::TRUNC,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            psp::sys::sceIoClose(fd);
        }
    }
}

fn psp_main() {
    // 10s delay — let system settle before any ms0: I/O.
    unsafe { psp::sys::sceKernelDelayThread(10_000_000) };
    log(b"[DL] started");

    // Create command file for file-based IPC (works when not on USB).
    clear_cmd();

    // Spawn TCP server thread (works in game context after EBOOT inits net).
    unsafe {
        let thid = psp::sys::sceKernelCreateThread(
            b"devloop\0".as_ptr(),
            tcp_worker,
            0x20, 4096,
            psp::sys::ThreadAttributes::empty(),
            core::ptr::null_mut(),
        );
        if thid >= psp::sys::SceUid(0) {
            psp::sys::sceKernelStartThread(thid, 0, core::ptr::null_mut());
        }
    }

    // Main thread: poll command file (fallback when WiFi not available).
    loop {
        unsafe { psp::sys::sceKernelDelayThread(2_000_000) };

        let mut buf = [0u8; 256];
        let n = read_cmd(&mut buf);
        if n == 0 { continue; }

        clear_cmd();
        execute_cmd(&buf[..n]);
    }
}

fn execute_cmd(raw: &[u8]) {
    let cmd = raw.split(|&b| b == b'\n' || b == b'\r').next().unwrap_or(raw);

    if cmd == b"reboot" {
        log(b"[DL] cmd: reboot");
        if let Some(fp) = unsafe { psp::hook::find_function(
            b"scePower_Service\0".as_ptr(),
            b"scePower\0".as_ptr(),
            0x0442D852,
        ) } {
            let f: unsafe extern "C" fn(i32) -> i32 = unsafe { core::mem::transmute(fp) };
            unsafe { f(0) };
        }
    } else if cmd == b"screenshot" {
        log(b"[DL] cmd: screenshot");
        take_screenshot();
    } else if cmd.starts_with(b"launch ") && cmd.len() > 7 {
        let path = &cmd[7..];
        let mut pb = [0u8; 128];
        let l = path.len().min(127);
        pb[..l].copy_from_slice(&path[..l]);
        pb[l] = 0;
        log(b"[DL] cmd: launch");
        launch_eboot(&pb[..l + 1]);
    } else {
        log(b"[DL] unknown cmd");
    }
}

unsafe extern "C" fn tcp_worker(_: usize, _: *mut c_void) -> i32 {
    psp::sys::sceKernelDelayThread(3_000_000);

    let inet = &[(b"sceNetInet_Library\0" as &[u8], b"sceNetInet\0" as &[u8])];
    let apctl = &[(b"sceNetApctl_Library\0" as &[u8], b"sceNetApctl\0" as &[u8])];

    macro_rules! find {
        ($nid:expr, $m:expr) => {{
            let mut r: *mut u8 = core::ptr::null_mut();
            for &(m, l) in $m {
                if let Some(fp) = psp::hook::find_function(m.as_ptr(), l.as_ptr(), $nid) {
                    r = fp; break;
                }
            }
            r
        }};
    }

    // Wait for EBOOT to load net modules.
    log(b"[DL] TCP: waiting for net...");
    let mut p_sock: *mut u8 = core::ptr::null_mut();
    for _ in 0..60 {
        p_sock = find!(0x8B7B220F, inet);
        if !p_sock.is_null() { break; }
        psp::sys::sceKernelDelayThread(2_000_000);
    }
    if p_sock.is_null() { log(b"[DL] TCP: net unavailable"); return 0; }

    let p_state = find!(0x5DEAC81B, apctl);
    let p_bind = find!(0x1A33F9AE, inet);
    let p_listen = find!(0xD10A1A7A, inet);
    let p_accept = find!(0xDB094E1B, inet);
    let p_recv = find!(0xCDA85C99, inet);
    let p_send = find!(0x7AA671BC, inet);
    let p_close = find!(0x8D7284EA, inet);

    if p_bind.is_null() || p_accept.is_null() {
        log(b"[DL] TCP: incomplete resolve");
        return 0;
    }
    log(b"[DL] TCP: net resolved");

    type F1 = unsafe extern "C" fn(i32)->i32;
    type Fs = unsafe extern "C" fn(*mut i32)->i32;
    type F2 = unsafe extern "C" fn(i32,i32)->i32;
    type F3 = unsafe extern "C" fn(i32,i32,i32)->i32;
    type Fb = unsafe extern "C" fn(i32,*const u8,u32)->i32;
    type Fa = unsafe extern "C" fn(i32,*mut u8,*mut u32)->i32;
    type Fr = unsafe extern "C" fn(i32,*mut c_void,usize,i32)->i32;
    type Fsd = unsafe extern "C" fn(i32,*const c_void,usize,i32)->i32;

    let socket: F3 = core::mem::transmute(p_sock);
    let bind: Fb = core::mem::transmute(p_bind);
    let listen: F2 = core::mem::transmute(p_listen);
    let accept: Fa = core::mem::transmute(p_accept);
    let recv: Fr = core::mem::transmute(p_recv);
    let send: Fsd = core::mem::transmute(p_send);
    let close: F1 = core::mem::transmute(p_close);

    // Wait for WiFi (state 4 = got IP).
    if !p_state.is_null() {
        let get_state: Fs = core::mem::transmute(p_state);
        log(b"[DL] TCP: waiting WiFi...");
        let mut connected = false;
        for i in 0..60u32 {
            let mut s: i32 = 0;
            let r = get_state(&mut s);
            // Log first few attempts and every 10th.
            if i < 3 || i % 10 == 0 {
                let mut msg = *b"[DL] state=XX ret=XXXXXXXX";
                msg[11] = b'0' + ((s / 10) % 10) as u8;
                msg[12] = b'0' + (s % 10) as u8;
                hex_u32(r as u32, &mut msg[18..26]);
                log(&msg);
            }
            if s == 4 { connected = true; break; }
            psp::sys::sceKernelDelayThread(2_000_000);
        }
        if connected {
            log(b"[DL] TCP: WiFi OK");
        } else {
            log(b"[DL] TCP: WiFi timeout, trying anyway");
        }
    } else {
        log(b"[DL] TCP: no state fn, delay 15s");
        psp::sys::sceKernelDelayThread(15_000_000);
    }

    // Let inet stack settle after WiFi connect.
    psp::sys::sceKernelDelayThread(3_000_000);

    // Retry socket creation a few times.
    let mut sfd = -1i32;
    for attempt in 0..5 {
        sfd = socket(2, 1, 0);
        if sfd >= 0 { break; }
        if attempt == 0 {
            let mut msg = *b"[DL] TCP: sock retry XXXXXXXX";
            hex_u32(sfd as u32, &mut msg[21..29]);
            log(&msg);
        }
        psp::sys::sceKernelDelayThread(2_000_000);
    }
    if sfd < 0 { log(b"[DL] TCP: socket fail final"); return 0; }

    let mut sa = [0u8; 16];
    sa[0] = 16; sa[1] = 2;
    sa[2] = (9293 >> 8) as u8;
    sa[3] = (9293 & 0xFF) as u8;

    if bind(sfd, sa.as_ptr(), 16) < 0 { log(b"[DL] TCP: bind fail"); close(sfd); return 0; }
    if listen(sfd, 1) < 0 { log(b"[DL] TCP: listen fail"); close(sfd); return 0; }
    log(b"[DL] TCP :9293 ready");

    loop {
        let mut ca = [0u8; 16];
        let mut al: u32 = 16;
        let cfd = accept(sfd, ca.as_mut_ptr(), &mut al);
        if cfd < 0 { psp::sys::sceKernelDelayThread(500_000); continue; }

        let mut buf = [0u8; 256];
        let n = recv(cfd, buf.as_mut_ptr() as *mut c_void, 256, 0);
        if n > 0 {
            let cmd = &buf[..n as usize];
            let cmd = cmd.split(|&b| b == b'\n' || b == b'\r').next().unwrap_or(cmd);

            if cmd == b"ping" {
                send(cfd, b"pong\n".as_ptr() as *const c_void, 5, 0);
            } else {
                send(cfd, b"ok\n".as_ptr() as *const c_void, 3, 0);
                close(cfd);
                execute_cmd(&buf[..n as usize]);
                continue;
            }
        }
        close(cfd);
    }
}
