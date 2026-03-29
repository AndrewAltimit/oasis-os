//! TCP command server for remote development automation.
//!
//! Listens on port 9293 after WiFi connects. Accepts single-line
//! text commands and responds with "ok\n" or "pong\n".
//!
//! Commands:
//!   ping        → "pong\n"
//!   screenshot  → saves VRAM to ms0:/seplugins/devloop_screen.raw
//!   reboot      → cold reset
//!   log         → responds with last 2KB of eboot.log

use core::ffi::c_void;

/// Start the network auto-connect + command server thread.
/// Call early in EBOOT init — connects WiFi in background without
/// blocking the UI, then starts TCP server.
pub fn spawn() {
    if let Ok(handle) = psp::thread::ThreadBuilder::new(b"cmd_srv\0")
        .priority(40)
        .stack_size(8192)
        .spawn(move || {
            // Auto-connect WiFi in background before server starts.
            auto_connect_wifi();
            server_main();
            0
        })
    {
        core::mem::forget(handle);
    }
}

/// Try to auto-connect to saved WiFi profile without dialog.
/// Initializes the full network stack if not already done.
fn auto_connect_wifi() {
    // Check if already connected.
    if psp::net::is_connected() {
        return;
    }

    // Load net modules + init stack if not done yet.
    crate::network::load_net_modules_once();
    let _ = psp::net::init(0x20000);

    // Try profiles 1 and 0.
    for profile in [1i32, 0] {
        let ret = unsafe { psp::sys::sceNetApctlConnect(profile) };
        if ret < 0 { continue; }

        let mut connected = false;
        for _ in 0..30 {
            let mut state = psp::sys::ApctlState::Disconnected;
            unsafe { psp::sys::sceNetApctlGetState(&mut state) };
            if matches!(state, psp::sys::ApctlState::GotIp) {
                connected = true;
                break;
            }
            psp::thread::sleep_ms(500);
        }

        if connected {
            log_msg("[CMD] WiFi auto-connected");
            return;
        }
        unsafe { psp::sys::sceNetApctlDisconnect() };
        psp::thread::sleep_ms(500);
    }

    log_msg("[CMD] WiFi auto-connect failed, will retry on demand");
}

fn log_msg(msg: &str) {
    crate::video::vlog_force(msg);
}

fn server_main() {
    // Wait for network to be fully ready.
    for _ in 0..30 {
        if psp::net::is_connected() {
            break;
        }
        psp::thread::sleep_ms(2000);
    }

    if !psp::net::is_connected() {
        log_msg("[CMD] no network, server not started");
        return;
    }

    // Create TCP server socket.
    let fd = unsafe {
        psp::sys::sceNetInetSocket(2, 1, 0) // AF_INET, SOCK_STREAM
    };
    if fd < 0 {
        log_msg("[CMD] socket failed");
        return;
    }

    // Bind to 0.0.0.0:9293.
    let sa = crate::network::make_sockaddr_in_pub([0, 0, 0, 0], 9293);
    let ret = unsafe {
        psp::sys::sceNetInetBind(
            fd,
            &sa as *const _ as *const psp::sys::sockaddr,
            core::mem::size_of::<psp::sys::sockaddr>() as u32,
        )
    };
    if ret < 0 {
        log_msg("[CMD] bind failed");
        unsafe { psp::sys::sceNetInetClose(fd) };
        return;
    }

    let ret = unsafe { psp::sys::sceNetInetListen(fd, 2) };
    if ret < 0 {
        log_msg("[CMD] listen failed");
        unsafe { psp::sys::sceNetInetClose(fd) };
        return;
    }

    log_msg("[CMD] TCP :9293 ready");

    // Accept loop.
    loop {
        let mut client_addr: psp::sys::sockaddr = unsafe { core::mem::zeroed() };
        let mut addr_len: u32 = core::mem::size_of::<psp::sys::sockaddr>() as u32;
        let cfd = unsafe {
            psp::sys::sceNetInetAccept(fd, &mut client_addr, &mut addr_len)
        };
        if cfd < 0 {
            psp::thread::sleep_ms(500);
            continue;
        }

        handle_client(cfd);
    }
}

fn handle_client(cfd: i32) {
    let mut buf = [0u8; 256];
    let n = unsafe {
        psp::sys::sceNetInetRecv(
            cfd,
            buf.as_mut_ptr() as *mut c_void,
            256,
            0,
        )
    };

    if n <= 0 {
        unsafe { psp::sys::sceNetInetClose(cfd) };
        return;
    }

    let raw = &buf[..n as usize];
    let cmd = raw.split(|&b| b == b'\n' || b == b'\r')
        .next()
        .unwrap_or(raw);

    if cmd == b"ping" {
        send_response(cfd, b"pong\n");
    } else if cmd == b"screenshot" {
        take_screenshot();
        send_response(cfd, b"ok\n");
        log_msg("[CMD] screenshot");
    } else if cmd == b"exit" {
        send_response(cfd, b"ok\n");
        unsafe { psp::sys::sceNetInetClose(cfd) };
        log_msg("[CMD] exiting to XMB");
        psp::thread::sleep_ms(500);
        unsafe { psp::sys::sceKernelExitGame() };
        return;
    } else if cmd == b"reboot" {
        send_response(cfd, b"ok\n");
        unsafe { psp::sys::sceNetInetClose(cfd) };
        log_msg("[CMD] rebooting PSP");
        psp::thread::sleep_ms(500);
        // Restart the EBOOT (reloads plugins, refreshes everything).
        unsafe {
            psp::sys::sceKernelLoadExec(
                b"ms0:/PSP/GAME/OASISOS/EBOOT.PBP\0".as_ptr(),
                core::ptr::null_mut(),
            );
        }
        return;
    } else if cmd == b"log" {
        // Read last 2KB of eboot.log and send it.
        send_log(cfd);
    } else {
        send_response(cfd, b"err: unknown command\n");
    }

    unsafe { psp::sys::sceNetInetClose(cfd) };
}

fn send_response(cfd: i32, data: &[u8]) {
    unsafe {
        psp::sys::sceNetInetSend(
            cfd,
            data.as_ptr() as *const c_void,
            data.len(),
            0,
        );
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
            psp::sys::sceIoWrite(
                fd,
                0x44000000u32 as *const c_void,
                FB_SIZE,
            );
            psp::sys::sceIoClose(fd);
        }
    }
}

fn send_log(cfd: i32) {
    let fd = unsafe {
        psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::RD_ONLY,
            0,
        )
    };
    if fd < psp::sys::SceUid(0) {
        send_response(cfd, b"err: no log\n");
        return;
    }

    // Seek to last 2KB.
    let size = unsafe {
        psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::End)
    };
    let offset = if size > 2048 { size - 2048 } else { 0 };
    unsafe {
        psp::sys::sceIoLseek(fd, offset, psp::sys::IoWhence::Set);
    }

    let mut buf = [0u8; 2048];
    let n = unsafe {
        psp::sys::sceIoRead(fd, buf.as_mut_ptr() as *mut c_void, 2048)
    };
    unsafe { psp::sys::sceIoClose(fd) };

    if n > 0 {
        send_response(cfd, &buf[..n as usize]);
    } else {
        send_response(cfd, b"(empty)\n");
    }
}
