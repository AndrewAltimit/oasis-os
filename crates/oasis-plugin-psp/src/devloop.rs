//! Remote development loop via WiFi TCP server.
//!
//! Dev only, and off unless `oasis.ini` sets both `devloop = true` and a
//! `devloop_psk` of at least [`MIN_PSK_LEN`] bytes. Connects to saved WiFi
//! profile 1 and listens on TCP port 9293. Every connection must first pass
//! the PSK HMAC challenge-response in [`crate::auth`] (`AUTH_REQUIRED
//! <nonce>` / `AUTH <hmac>` / `AUTH_OK`, the remote terminal's protocol);
//! then one command line is read and answered with `OK`.
//! Commands: "screenshot", "reboot", "launch <path>"

use core::ffi::c_void;

use crate::auth;

/// Shortest `devloop_psk` accepted; shorter keys leave the server off.
pub const MIN_PSK_LEN: usize = 8;

const LOG_FILE: &[u8] = b"ms0:/seplugins/.devloop_log\0";

// -----------------------------------------------------------------------
// Hex formatting + logging (no alloc)
// -----------------------------------------------------------------------

fn hex_nibble(n: u8) -> u8 {
    if n < 10 { b'0' + n } else { b'a' + n - 10 }
}
fn hex_u32(val: u32, out: &mut [u8]) {
    for i in 0..4 {
        let b = ((val >> (24 - i * 8)) & 0xFF) as u8;
        out[i * 2] = hex_nibble(b >> 4);
        out[i * 2 + 1] = hex_nibble(b & 0xF);
    }
}

fn devlog(msg: &[u8]) {
    // SAFETY: LOG_FILE is NUL-terminated; writes pass live slices' ptr/len; the fd
    // is used and closed only after a successful open.
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

// -----------------------------------------------------------------------
// Screenshot
// -----------------------------------------------------------------------

fn take_screenshot() {
    const FB_SIZE: usize = 512 * 272 * 4;
    let fb_ptr = 0x44000000u32 as *const u8;
    // SAFETY: 0x44000000 is the uncached VRAM mirror (2 MiB, always mapped in
    // kernel mode) and FB_SIZE (one 512-stride 32bpp frame) stays inside it;
    // sceIoWrite only reads from it.
    unsafe {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/seplugins/.devloop_screenshot.raw\0".as_ptr(),
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

fn launch_eboot(path: &[u8]) {
    const NID_LOAD_EXEC_VSH: u32 = 0x1DDDAD0C;
    let modules: &[(&[u8], &[u8])] = &[
        (b"SystemControl\0", b"SystemCtrlForKernel\0"),
        (b"SystemCtrlForKernel\0", b"SystemCtrlForKernel\0"),
    ];
    for &(m, l) in modules {
        // SAFETY: NUL-terminated name literals; find_function only reads them.
        if let Some(fp) =
            unsafe { psp::hook::find_function(m.as_ptr(), l.as_ptr(), NID_LOAD_EXEC_VSH) }
        {
            #[repr(C)]
            struct Param {
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
            let mut param = Param {
                size: core::mem::size_of::<Param>() as u32,
                args: path.len() as u32,
                argp: path.as_ptr() as *mut c_void,
                key: b"game\0".as_ptr(),
                vshmain_args_size: 0,
                vshmain_args: core::ptr::null_mut(),
                configfile: b"/kd/pspbtcnf_game.txt\0".as_ptr(),
                unk4: 0,
                unk5: 0,
            };
            type F = unsafe extern "C" fn(i32, *const u8, *mut Param) -> i32;
            // SAFETY: `fp` is the non-null sctrlKernelLoadExecVSH export, C signature
            // `int (int apitype, const char *file, SceKernelLoadExecVSHParam *)` = `F`.
            let f: F = unsafe { core::mem::transmute(fp) };
            devlog(b"[DEV] launching...");
            // SAFETY: `path` is NUL-terminated (built by the caller); `param` is a
            // correctly sized param block whose pointers outlive the call.
            unsafe { f(0x141, path.as_ptr(), &mut param) };
            return;
        }
    }
    devlog(b"[DEV] LoadExecVSH not found");
}

// -----------------------------------------------------------------------
// Main thread
// -----------------------------------------------------------------------

pub fn start() {
    // SAFETY: NUL-terminated thread name, `devloop_thread` has the thread-entry
    // ABI, and it is started with an empty argument block.
    unsafe {
        let thid = psp::sys::sceKernelCreateThread(
            b"OasisDev\0".as_ptr(),
            devloop_thread,
            0x20,
            8192,
            psp::sys::ThreadAttributes::empty(),
            core::ptr::null_mut(),
        );
        if thid >= psp::sys::SceUid(0) {
            psp::sys::sceKernelStartThread(thid, 0, core::ptr::null_mut());
        }
    }
}

unsafe extern "C" fn devloop_thread(_: usize, _: *mut c_void) -> i32 {
    // SAFETY: this is the devloop kernel thread's entry point, started once by `start`. It is the
    // only user of its sockets and buffers; every sceIo / sceNet call gets NUL-terminated paths and
    // buffers passed with their real lengths.
    unsafe {
        psp::sys::sceKernelDelayThread(5_000_000);
        let config = crate::config::get_config();
        let psk = config.devloop_psk();
        if psk.len() < MIN_PSK_LEN {
            devlog(b"[DEV] devloop_psk unset or < 8 bytes; server disabled");
            return 0;
        }
        devlog(b"[DEV] starting...");

        // Load net modules.
        psp::sys::sceUtilityLoadNetModule(psp::sys::NetModule::NetCommon);
        psp::sys::sceUtilityLoadNetModule(psp::sys::NetModule::NetInet);
        psp::sys::sceKernelDelayThread(500_000);

        // Resolve net functions by NID (import stubs don't work from kernel PRX).
        let net = &[(b"sceNet_Library\0" as &[u8], b"sceNet\0" as &[u8])];
        let inet = &[(b"sceNetInet_Library\0" as &[u8], b"sceNetInet\0" as &[u8])];
        let apctl = &[(b"sceNetApctl_Library\0" as &[u8], b"sceNetApctl\0" as &[u8])];

        macro_rules! resolve {
            ($nid:expr, $mods:expr) => {{
                let mut r: *mut u8 = core::ptr::null_mut();
                for &(m, l) in $mods {
                    if let Some(fp) = psp::hook::find_function(m.as_ptr(), l.as_ptr(), $nid) {
                        r = fp;
                        break;
                    }
                }
                r
            }};
        }

        let p_net_init = resolve!(0x39AF39A6, net);
        let p_inet_init = resolve!(0x17943399, inet);
        let p_apctl_init = resolve!(0xE2F91F9B, apctl);
        let p_connect = resolve!(0xCFB957C6, apctl);
        let p_get_state = resolve!(0x5DEAC81B, apctl);
        let p_socket = resolve!(0x8B7B220F, inet);
        let p_bind = resolve!(0x1A33F9AE, inet);
        let p_listen = resolve!(0xD10A1A7A, inet);
        let p_accept = resolve!(0xDB094E1B, inet);
        let p_recv = resolve!(0xCDA85C99, inet);
        let p_send = resolve!(0x7AA671BC, inet);
        let p_close = resolve!(0x8D7284EA, inet);
        // Optional: without it a silent peer can hold the server.
        let p_setsockopt = resolve!(0x2FE71FE7, inet);

        // Every pointer is transmuted to a non-nullable fn pointer below, so all
        // of them must have resolved (a null fn pointer is UB, not a no-op).
        let ok = [
            p_net_init,
            p_inet_init,
            p_apctl_init,
            p_connect,
            p_get_state,
            p_socket,
            p_bind,
            p_listen,
            p_accept,
            p_recv,
            p_send,
            p_close,
        ]
        .iter()
        .all(|p| !p.is_null());

        if !ok {
            devlog(b"[DEV] net resolve FAILED");
            return 0;
        }
        devlog(b"[DEV] net functions resolved");

        // Init net stack.
        type F5 = unsafe extern "C" fn(i32, i32, i32, i32, i32) -> i32;
        type F0 = unsafe extern "C" fn() -> i32;
        type F2 = unsafe extern "C" fn(i32, i32) -> i32;
        type F1 = unsafe extern "C" fn(i32) -> i32;
        type F1p = unsafe extern "C" fn(*mut i32) -> i32;
        type F3 = unsafe extern "C" fn(i32, i32, i32) -> i32;
        type Fbind = unsafe extern "C" fn(i32, *const u8, u32) -> i32;
        type Faccept = unsafe extern "C" fn(i32, *mut u8, *mut u32) -> i32;
        type Frecv = unsafe extern "C" fn(i32, *mut c_void, usize, i32) -> i32;
        type Fsend = unsafe extern "C" fn(i32, *const c_void, usize, i32) -> i32;
        type Fsockopt = unsafe extern "C" fn(i32, i32, i32, *const c_void, u32) -> i32;

        let net_init: F5 = core::mem::transmute(p_net_init);
        let inet_init: F0 = core::mem::transmute(p_inet_init);
        let apctl_init: F2 = core::mem::transmute(p_apctl_init);
        let connect: F1 = core::mem::transmute(p_connect);
        let get_state: F1p = core::mem::transmute(p_get_state);
        let socket: F3 = core::mem::transmute(p_socket);
        let bind: Fbind = core::mem::transmute(p_bind);
        let listen: F2 = core::mem::transmute(p_listen);
        let accept: Faccept = core::mem::transmute(p_accept);
        let recv: Frecv = core::mem::transmute(p_recv);
        let send: Fsend = core::mem::transmute(p_send);
        let close: F1 = core::mem::transmute(p_close);
        let setsockopt: Option<Fsockopt> =
            (!p_setsockopt.is_null()).then(|| core::mem::transmute(p_setsockopt));

        let mut r = *b"[DEV] init: XXXXXXXX";
        let rv = net_init(0x20000, 0x20, 0x1000, 0x20, 0x1000);
        hex_u32(rv as u32, &mut r[12..20]);
        devlog(&r);

        inet_init();
        apctl_init(0x1800, 42);

        // Connect WiFi profile 1.
        devlog(b"[DEV] WiFi connecting...");
        connect(1);
        let mut connected = false;
        for _ in 0..30 {
            let mut state: i32 = 0;
            get_state(&mut state);
            if state == 4 {
                connected = true;
                break;
            }
            psp::sys::sceKernelDelayThread(500_000);
        }
        if !connected {
            devlog(b"[DEV] WiFi TIMEOUT");
            return 0;
        }
        devlog(b"[DEV] WiFi OK");

        // TCP server on port 9293.
        let sfd = socket(2, 1, 0);
        if sfd < 0 {
            devlog(b"[DEV] socket fail");
            return 0;
        }

        let mut sa = [0u8; 16];
        sa[0] = 16;
        sa[1] = 2;
        sa[2] = (9293 >> 8) as u8;
        sa[3] = (9293 & 0xFF) as u8;

        if bind(sfd, sa.as_ptr(), 16) < 0 {
            devlog(b"[DEV] bind fail");
            return 0;
        }
        if listen(sfd, 1) < 0 {
            devlog(b"[DEV] listen fail");
            return 0;
        }
        devlog(b"[DEV] TCP :9293 ready");

        // Seed for the next nonce: previous nonce | clock | counter | peer.
        let mut seed = [0u8; auth::NONCE_HEX_LEN + 8 + 4 + 16];
        let mut counter: u32 = 0;
        loop {
            let mut ca = [0u8; 16];
            let mut al: u32 = 16;
            let cfd = accept(sfd, ca.as_mut_ptr(), &mut al);
            if cfd < 0 {
                psp::sys::sceKernelDelayThread(1_000_000);
                continue;
            }
            if let Some(setsockopt) = setsockopt {
                // SO_RCVTIMEO = 10 s (struct timeval of two 32-bit longs).
                let tv: [i32; 2] = [10, 0];
                setsockopt(cfd, 0xFFFF, 0x1006, tv.as_ptr() as *const c_void, 8);
            }

            // Challenge-response before any command is read.
            counter = counter.wrapping_add(1);
            let n = auth::NONCE_HEX_LEN;
            let now = psp::sys::sceKernelGetSystemTimeWide();
            seed[n..n + 8].copy_from_slice(&now.to_le_bytes());
            seed[n + 8..n + 12].copy_from_slice(&counter.to_le_bytes());
            seed[n + 12..].copy_from_slice(&ca);
            let nonce = auth::nonce_from_seed(&seed);
            seed[..n].copy_from_slice(&nonce);
            let mut challenge = *b"AUTH_REQUIRED 00000000000000000000000000000000\n";
            challenge[14..14 + n].copy_from_slice(&nonce);
            send(cfd, challenge.as_ptr() as *const c_void, challenge.len(), 0);

            let mut line = [0u8; 128];
            let mut len = 0usize;
            while len < line.len() && !line[..len].contains(&b'\n') {
                let got = recv(cfd, line[len..].as_mut_ptr() as *mut c_void, line.len() - len, 0);
                if got <= 0 {
                    break;
                }
                len += got as usize;
            }
            if !auth::verify_auth_line(psk, &nonce, &line[..len]) {
                send(cfd, b"AUTH_FAIL\n".as_ptr() as *const c_void, 10, 0);
                close(cfd);
                devlog(b"[DEV] auth failed");
                // Single-threaded accept loop: this throttles PSK guessing.
                psp::sys::sceKernelDelayThread(2_000_000);
                continue;
            }
            send(cfd, b"AUTH_OK\n".as_ptr() as *const c_void, 8, 0);

            let mut buf = [0u8; 512];
            let n = recv(cfd, buf.as_mut_ptr() as *mut c_void, 512, 0);
            if n > 0 {
                send(cfd, b"OK\n".as_ptr() as *const c_void, 3, 0);
            }
            close(cfd);
            if n <= 0 {
                continue;
            }

            let cmd = &buf[..n as usize];
            let cmd = if let Some(e) = cmd.iter().position(|&b| b == b'\n' || b == b'\r') {
                &cmd[..e]
            } else {
                cmd
            };

            devlog(b"[DEV] cmd received");

            if cmd == b"screenshot" {
                take_screenshot();
            } else if cmd == b"reboot" {
                devlog(b"[DEV] rebooting");
                if let Some(fp) = psp::hook::find_function(
                    b"scePower_Service\0".as_ptr(),
                    b"scePower\0".as_ptr(),
                    0x0442D852,
                ) {
                    let f: unsafe extern "C" fn(i32) -> i32 = core::mem::transmute(fp);
                    f(0);
                }
            } else if cmd.starts_with(b"launch ") {
                let path = &cmd[7..];
                let mut pb = [0u8; 128];
                let l = path.len().min(127);
                pb[..l].copy_from_slice(&path[..l]);
                pb[l] = 0;
                launch_eboot(&pb[..l + 1]);
            }
        }
    }
}
