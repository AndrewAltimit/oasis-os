//! Kernel PRX for remote PSP development automation.
//!
//! Coexists with AlwaysUSB (10s init delay avoids ms0: race).
//! WiFi TCP server on port 9293 for host commands.
//! Logs to ms0: (kernel can write despite USB storage mode).
//!
//! Commands: ping, screenshot, reboot, launch <path>

#![no_std]
#![no_main]

psp::module_kernel!("OasisDevloop", 1, 0);

use core::ffi::c_void;

const LOG_PATH: *const u8 = b"ms0:/seplugins/devloop.log\0".as_ptr();

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

fn psp_main() {
    // 10s delay — let AlwaysUSB fully init before any ms0: I/O.
    unsafe { psp::sys::sceKernelDelayThread(10_000_000) };

    log(b"[DL] starting");
    unsafe {
        let thid = psp::sys::sceKernelCreateThread(
            b"devloop\0".as_ptr(),
            worker,
            0x20, 4096,
            psp::sys::ThreadAttributes::empty(),
            core::ptr::null_mut(),
        );
        if thid >= psp::sys::SceUid(0) {
            psp::sys::sceKernelStartThread(thid, 0, core::ptr::null_mut());
        }
    }
    loop { unsafe { psp::sys::sceKernelDelayThread(60_000_000) }; }
}

unsafe extern "C" fn worker(_: usize, _: *mut c_void) -> i32 {
    psp::sys::sceKernelDelayThread(3_000_000);

    // Resolve net functions by NID (user-mode libs not linked in kernel PRX).
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

    // Wait for EBOOT to load net modules (up to 60s).
    log(b"[DL] waiting for net...");
    let mut p_sock: *mut u8 = core::ptr::null_mut();
    for _ in 0..30 {
        p_sock = find!(0x8B7B220F, inet); // sceNetInetSocket
        if !p_sock.is_null() { break; }
        psp::sys::sceKernelDelayThread(2_000_000);
    }
    if p_sock.is_null() {
        log(b"[DL] net not available");
        return 0;
    }

    let p_state = find!(0x5DEAC81B, apctl);
    let p_bind = find!(0x1A33F9AE, inet);
    let p_listen = find!(0xD10A1A7A, inet);
    let p_accept = find!(0xDB094E1B, inet);
    let p_recv = find!(0xCDA85C99, inet);
    let p_send = find!(0x7AA671BC, inet);
    let p_close = find!(0x8D7284EA, inet);

    if p_bind.is_null() || p_accept.is_null() || p_recv.is_null() {
        log(b"[DL] incomplete net resolve");
        return 0;
    }
    log(b"[DL] net resolved");

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

    // Wait for WiFi connection (EBOOT handles dialog).
    if !p_state.is_null() {
        let get_state: Fs = core::mem::transmute(p_state);
        log(b"[DL] waiting for WiFi...");
        for _ in 0..60 {
            let mut s: i32 = 0;
            get_state(&mut s);
            if s == 4 { break; }
            psp::sys::sceKernelDelayThread(2_000_000);
        }
    } else {
        psp::sys::sceKernelDelayThread(15_000_000);
    }

    // TCP server on port 9293.
    let sfd = socket(2, 1, 0);
    if sfd < 0 { log(b"[DL] socket fail"); return 0; }

    let mut sa = [0u8; 16];
    sa[0] = 16; sa[1] = 2;
    sa[2] = (9293 >> 8) as u8;
    sa[3] = (9293 & 0xFF) as u8;

    if bind(sfd, sa.as_ptr(), 16) < 0 { log(b"[DL] bind fail"); close(sfd); return 0; }
    if listen(sfd, 1) < 0 { log(b"[DL] listen fail"); close(sfd); return 0; }
    log(b"[DL] TCP :9293 ready");

    loop {
        let mut ca = [0u8; 16];
        let mut al: u32 = 16;
        let cfd = accept(sfd, ca.as_mut_ptr(), &mut al);
        if cfd < 0 { psp::sys::sceKernelDelayThread(500_000); continue; }

        let mut buf = [0u8; 256];
        let n = recv(cfd, buf.as_mut_ptr() as *mut c_void, 256, 0);
        if n <= 0 { close(cfd); continue; }

        let cmd = &buf[..n as usize];
        let cmd = cmd.split(|&b| b == b'\n' || b == b'\r').next().unwrap_or(cmd);

        if cmd == b"ping" {
            send(cfd, b"pong\n".as_ptr() as *const c_void, 5, 0);
        } else if cmd == b"screenshot" {
            take_screenshot();
            send(cfd, b"ok\n".as_ptr() as *const c_void, 3, 0);
            log(b"[DL] screenshot taken");
        } else if cmd == b"reboot" {
            send(cfd, b"ok\n".as_ptr() as *const c_void, 3, 0);
            close(cfd);
            log(b"[DL] rebooting");
            if let Some(fp) = psp::hook::find_function(
                b"scePower_Service\0".as_ptr(),
                b"scePower\0".as_ptr(),
                0x0442D852,
            ) {
                let f: unsafe extern "C" fn(i32)->i32 = core::mem::transmute(fp);
                f(0);
            }
            continue;
        } else if cmd.starts_with(b"launch ") {
            send(cfd, b"ok\n".as_ptr() as *const c_void, 3, 0);
            close(cfd);
            let path = &cmd[7..];
            let mut pb = [0u8; 128];
            let l = path.len().min(127);
            pb[..l].copy_from_slice(&path[..l]);
            pb[l] = 0;
            launch_eboot(&pb[..l+1]);
            continue;
        } else {
            send(cfd, b"err\n".as_ptr() as *const c_void, 4, 0);
        }
        close(cfd);
    }
}
