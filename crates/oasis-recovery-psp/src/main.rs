//! PSP Network Recovery EBOOT
//!
//! Minimal recovery application for ARK-4 CFW that provides a WiFi TCP
//! file server for remote EBOOT/PRX replacement when the main application
//! is bricked. Replaces ARK's default recovery menu.
//!
//! ## How It Works
//!
//! 1. User holds R-trigger during PSP boot → ARK loads this instead of OASIS OS
//! 2. Recovery connects to saved WiFi profile (no dialog needed)
//! 3. TCP server on port 9293 accepts file upload commands
//! 4. Host pushes fixed EBOOT/PRX files over WiFi
//! 5. User reboots normally → fixed OASIS OS loads
//!
//! ## TCP Commands (same protocol as cmd_server.rs)
//!
//!   ping                      → "pong\n"
//!   upload <size> <path>      → receive file and write to ms0:
//!   readfile <path>           → stream file contents back: "<size>\n<bytes...>"
//!   delete <path>             → remove file from ms0:
//!   ls <path>                 → list directory contents
//!   reboot                    → cold hardware reset
//!   status                    → JSON with free memory, WiFi state
//!
//! ## Deploy
//!
//!   cp EBOOT.PBP ms0:/PSP/SAVEDATA/ARK_01234/RECOVERY.PBP
//!
//! ## Trigger
//!
//!   Hold R-trigger during PSP power-on

#![feature(restricted_std)]
#![no_main]

psp::module!("OasisRecovery", 1, 0);

use core::ffi::c_void;

const PORT: u16 = 9293;

fn psp_main() {
    // Show banner via dprintln (debug screen).
    psp::dprintln!("=== OASIS Network Recovery ===");
    psp::dprintln!("");

    // Load network modules.
    print("Loading net modules...\n");
    let ret = unsafe { psp::sys::sceUtilityLoadModule(psp::sys::Module::NetCommon) };
    print(&format!("  NetCommon: {ret:#x}\n"));
    let ret = unsafe { psp::sys::sceUtilityLoadModule(psp::sys::Module::NetInet) };
    print(&format!("  NetInet: {ret:#x}\n"));

    // Init network stack.
    print("Initializing network...\n");
    match psp::net::init(0x20000) {
        Ok(_) => print("  Net stack OK\n"),
        Err(e) => {
            print(&format!("  Net init failed: {e}\n"));
            print("  (Will retry after WiFi connect)\n");
        }
    }

    // Check WLAN switch.
    let wlan = unsafe { psp::sys::sceWlanGetSwitchState() };
    if wlan == 0 {
        print("\n** WLAN switch is OFF! **\n");
        print("Turn on the WiFi switch and reboot into recovery.\n");
        park();
    }

    // Auto-connect to saved WiFi profile.
    print("Connecting WiFi...\n");
    let connected = auto_connect_wifi();
    if !connected {
        print("\n** WiFi connection failed **\n");
        print("Ensure a WiFi profile is saved in PSP settings.\n");
        print("Falling back to USB mode...\n\n");
        start_usb();
        park();
    }

    // Show IP address.
    if let Ok(ip) = psp::net::get_ip_address() {
        let ip_str = core::str::from_utf8(&ip)
            .unwrap_or("?")
            .trim_end_matches('\0');
        print(&format!("\nWiFi connected! IP: {ip_str}\n"));
        print(&format!("TCP server on port {PORT}\n\n"));
    }

    // Start TCP server.
    print("Commands:\n");
    print("  ping                    - test connection\n");
    print("  upload <size> <path>    - write file to ms0:\n");
    print("  readfile <path>         - stream file back to host\n");
    print("  delete <path>           - remove file from ms0:\n");
    print("  ls <path>               - list directory\n");
    print("  reboot                  - cold restart\n");
    print("  status                  - system info\n\n");
    print("Waiting for connections...\n\n");

    server_main();
}

// ---------------------------------------------------------------------------
// WiFi auto-connect (same logic as cmd_server.rs)
// ---------------------------------------------------------------------------

fn auto_connect_wifi() -> bool {
    for profile in [1i32, 0] {
        print(&format!("  Trying profile {profile}... "));
        let ret = unsafe { psp::sys::sceNetApctlConnect(profile) };
        if ret < 0 {
            print(&format!("err {ret:#x}\n"));
            continue;
        }

        for _ in 0..40 {
            let mut state = psp::sys::ApctlState::Disconnected;
            unsafe { psp::sys::sceNetApctlGetState(&mut state) };
            if matches!(state, psp::sys::ApctlState::GotIp) {
                print("connected!\n");
                return true;
            }
            psp::thread::sleep_ms(500);
        }

        print("timeout\n");
        unsafe { psp::sys::sceNetApctlDisconnect() };
        psp::thread::sleep_ms(500);
    }
    false
}

// ---------------------------------------------------------------------------
// TCP server
// ---------------------------------------------------------------------------

fn server_main() {
    let fd = unsafe { psp::sys::sceNetInetSocket(2, 1, 0) };
    if fd < 0 {
        print("Socket creation failed!\n");
        park();
    }

    // Bind to 0.0.0.0:9293.
    let mut sa: psp::sys::sockaddr = unsafe { core::mem::zeroed() };
    sa.sa_family = 2; // AF_INET
    // Port at offset 2-3 (network byte order).
    sa.sa_data[0] = (PORT >> 8) as u8;
    sa.sa_data[1] = (PORT & 0xFF) as u8;
    // IP 0.0.0.0 at offset 4-7 (already zeroed).

    let ret = unsafe {
        psp::sys::sceNetInetBind(
            fd,
            &sa as *const psp::sys::sockaddr,
            core::mem::size_of::<psp::sys::sockaddr>() as u32,
        )
    };
    if ret < 0 {
        print(&format!("Bind failed: {ret:#x}\n"));
        park();
    }

    let ret = unsafe { psp::sys::sceNetInetListen(fd, 2) };
    if ret < 0 {
        print(&format!("Listen failed: {ret:#x}\n"));
        park();
    }

    // Accept loop.
    loop {
        let mut client_addr: psp::sys::sockaddr = unsafe { core::mem::zeroed() };
        let mut addr_len: u32 = core::mem::size_of::<psp::sys::sockaddr>() as u32;
        let cfd = unsafe {
            psp::sys::sceNetInetAccept(fd, &mut client_addr, &mut addr_len)
        };
        if cfd < 0 {
            psp::thread::sleep_ms(100);
            continue;
        }

        handle_client(cfd);
    }
}

fn handle_client(cfd: i32) {
    let mut buf = [0u8; 512];
    let n = unsafe {
        psp::sys::sceNetInetRecv(cfd, buf.as_mut_ptr() as *mut c_void, 512, 0)
    };
    if n <= 0 {
        unsafe { psp::sys::sceNetInetClose(cfd) };
        return;
    }

    let raw = &buf[..n as usize];
    let cmd = raw.split(|&b| b == b'\n' || b == b'\r').next().unwrap_or(raw);

    if cmd == b"ping" {
        send(cfd, b"pong\n");
        print("[<] ping -> pong\n");
    } else if cmd == b"reboot" {
        send(cfd, b"ok\n");
        print("[<] rebooting...\n");
        unsafe { psp::sys::sceNetInetClose(cfd) };
        psp::thread::sleep_ms(500);
        unsafe { psp::sys::scePowerRequestColdReset(0) };
        return;
    } else if cmd == b"status" {
        let free = unsafe { psp::sys::sceKernelTotalFreeMemSize() } / 1024;
        let max_blk = unsafe { psp::sys::sceKernelMaxFreeMemSize() } / 1024;
        let mut state = psp::sys::ApctlState::Disconnected;
        unsafe { psp::sys::sceNetApctlGetState(&mut state) };
        let wifi = matches!(state, psp::sys::ApctlState::GotIp);
        let resp = format!(
            "{{\"mode\":\"recovery\",\"free_kb\":{free},\"max_blk_kb\":{max_blk},\"wifi\":{wifi}}}\n"
        );
        send(cfd, resp.as_bytes());
    } else if cmd.starts_with(b"upload ") {
        let args = &cmd[7..];
        let parts: Vec<&[u8]> = args.splitn(2, |&b| b == b' ').collect();
        if parts.len() >= 2 {
            let size = parse_u32(parts[0]);
            let path = parts[1];
            if size > 0 && size < 16_000_000 {
                let header_end = raw.iter().position(|&b| b == b'\n')
                    .map(|p| p + 1).unwrap_or(n as usize);
                let leftover = &buf[header_end..n as usize];
                let path_str = core::str::from_utf8(path).unwrap_or("?");
                print(&format!("[<] upload {} bytes -> {}\n", size, path_str));
                receive_file(cfd, size, path, leftover);
            } else {
                send(cfd, b"err: bad size\n");
            }
        } else {
            send(cfd, b"err: usage: upload <size> <path>\n");
        }
    } else if cmd.starts_with(b"ls ") {
        let path = &cmd[3..];
        list_directory(cfd, path);
    } else if cmd.starts_with(b"readfile ") {
        let path = &cmd[9..];
        read_file(cfd, path);
    } else if cmd.starts_with(b"delete ") {
        let path = &cmd[7..];
        delete_file(cfd, path);
    } else {
        send(cfd, b"err: unknown command\n");
    }

    unsafe { psp::sys::sceNetInetClose(cfd) };
}

// ---------------------------------------------------------------------------
// File operations
// ---------------------------------------------------------------------------

fn receive_file(cfd: i32, size: u32, path: &[u8], leftover: &[u8]) {
    let mut path_buf = Vec::with_capacity(path.len() + 1);
    path_buf.extend_from_slice(path);
    path_buf.push(0);

    let fd = unsafe {
        psp::sys::sceIoOpen(
            path_buf.as_ptr(),
            psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY
                | psp::sys::IoOpenFlags::TRUNC,
            0o777,
        )
    };
    if fd < psp::sys::SceUid(0) {
        send(cfd, b"err: can't create file\n");
        print("  ERROR: can't create file\n");
        return;
    }

    let mut received = 0u32;
    if !leftover.is_empty() {
        unsafe {
            psp::sys::sceIoWrite(fd, leftover.as_ptr() as *const c_void, leftover.len());
        }
        received += leftover.len() as u32;
    }

    let mut buf = [0u8; 4096];
    while received < size {
        let n = unsafe {
            psp::sys::sceNetInetRecv(
                cfd, buf.as_mut_ptr() as *mut c_void,
                buf.len().min((size - received) as usize), 0,
            )
        };
        if n <= 0 { break; }
        unsafe {
            psp::sys::sceIoWrite(fd, buf.as_ptr() as *const c_void, n as usize);
        }
        received += n as u32;
    }

    unsafe { psp::sys::sceIoClose(fd) };

    if received == size {
        print(&format!("  OK ({} bytes)\n", received));
        send(cfd, b"ok\n");
    } else {
        print(&format!("  INCOMPLETE ({}/{} bytes)\n", received, size));
        send(cfd, b"err: incomplete\n");
    }
}

fn list_directory(cfd: i32, _path: &[u8]) {
    // TODO: implement directory listing once SceIoDirent types are available.
    send(cfd, b"err: ls not yet implemented\n");
}

/// Read a file from disk and stream its contents back to the client.
///
/// Protocol: `readfile <path>\n` -> `<size>\n<bytes...>` on success
/// or `err: <reason>\n` on failure. Lets the host read crash logs
/// (`ms0:/PSP/GAME/OASISOS/eboot.log`) while the main EBOOT is dead.
fn read_file(cfd: i32, path: &[u8]) {
    let mut path_buf = Vec::with_capacity(path.len() + 1);
    path_buf.extend_from_slice(path);
    path_buf.push(0);

    let fd = unsafe {
        psp::sys::sceIoOpen(
            path_buf.as_ptr(),
            psp::sys::IoOpenFlags::RD_ONLY,
            0,
        )
    };
    if fd < psp::sys::SceUid(0) {
        send(cfd, b"err: open failed\n");
        return;
    }

    let size = unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::End) };
    let seek_ret = unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::Set) };

    if size < 0 || seek_ret < 0 {
        unsafe { psp::sys::sceIoClose(fd) };
        send(cfd, b"err: bad size\n");
        return;
    }

    let header = format!("{}\n", size);
    send(cfd, header.as_bytes());

    let mut remaining = size;
    let mut buf = [0u8; 4096];
    while remaining > 0 {
        let chunk = buf.len().min(remaining as usize);
        let n = unsafe {
            psp::sys::sceIoRead(fd, buf.as_mut_ptr() as *mut c_void, chunk as u32)
        };
        if n <= 0 {
            break;
        }
        unsafe {
            psp::sys::sceNetInetSend(
                cfd,
                buf.as_ptr() as *const c_void,
                n as usize,
                0,
            );
        }
        remaining -= n as i64;
    }

    unsafe { psp::sys::sceIoClose(fd) };
    print(&format!("[<] readfile {} bytes\n", size));
}

/// Delete a file from disk. Useful for clearing eboot.log before a
/// fresh boot attempt so the captured trace contains only the latest
/// run, and for nuking a corrupted config file (config.rcfg).
fn delete_file(cfd: i32, path: &[u8]) {
    let mut path_buf = Vec::with_capacity(path.len() + 1);
    path_buf.extend_from_slice(path);
    path_buf.push(0);

    let ret = unsafe { psp::sys::sceIoRemove(path_buf.as_ptr()) };
    if ret < 0 {
        send(cfd, b"err: delete failed\n");
    } else {
        send(cfd, b"ok\n");
        let path_str = core::str::from_utf8(path).unwrap_or("?");
        print(&format!("[<] deleted {}\n", path_str));
    }
}

// ---------------------------------------------------------------------------
// USB fallback (if WiFi fails)
// ---------------------------------------------------------------------------

fn start_usb() {
    print("Starting USB storage mode...\n");
    print("Connect USB cable and copy files manually.\n");
    // Use psp::usb if available, or just print instructions.
    // The user can still use ARK's built-in USB from the XMB.
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn send(cfd: i32, data: &[u8]) {
    unsafe {
        psp::sys::sceNetInetSend(cfd, data.as_ptr() as *const c_void, data.len(), 0);
    }
}

fn parse_u32(data: &[u8]) -> u32 {
    data.iter().fold(0u32, |acc, &b| {
        if b >= b'0' && b <= b'9' { acc * 10 + (b - b'0') as u32 } else { acc }
    })
}

fn print(msg: &str) {
    // Use dprintln which works in all contexts.
    psp::dprintln!("{}", msg);
}

fn park() -> ! {
    loop {
        psp::thread::sleep_ms(1_000_000);
    }
}
