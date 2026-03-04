//! Network initialization and utilities for radio streaming.

use super::nids::*;
use super::resolve::*;
use super::state::*;
use super::log_i32;

/// Load network modules and initialize the PSP network stack.
/// Only called on first radio activation.
pub(super) unsafe fn init_network() -> bool {
    // SAFETY: Initializing PSP network stack via resolved kernel driver NIDs.
    // Loading network PRX modules, resolving sceNetInet/sceNetApctl/sceNetResolver
    // function pointers, and calling them to connect WiFi. Volatile reads/writes
    // to statics during audio thread network init (single caller).
    unsafe {
        if core::ptr::read_volatile(&raw const NET_INITIALIZED) {
            return true;
        }
        crate::debug_log(b"[OASIS] init_network...");

        // Load network modules via sceUtilityLoadModule.
        let load_fn: Option<unsafe extern "C" fn(i32) -> i32> =
            resolve_nid(UTILITY_MODULES, NID_UTILITY_LOAD_MODULE)
                .map(|ptr| core::mem::transmute(ptr));

        if let Some(load) = load_fn {
            let r1 = load(PSP_MODULE_NET_COMMON);
            log_i32(b"[OASIS] LoadModule NET_COMMON=", r1);
            let r2 = load(PSP_MODULE_NET_INET);
            log_i32(b"[OASIS] LoadModule NET_INET=", r2);
        }

        // Also try kernel loading network PRXs.
        let kprxs: &[&[u8]] = &[
            b"flash0:/kd/pspnet.prx\0",
            b"flash0:/kd/pspnet_inet.prx\0",
            b"flash0:/kd/pspnet_apctl.prx\0",
            b"flash0:/kd/pspnet_resolver.prx\0",
        ];
        for path in kprxs {
            let mod_id = psp::sys::sceKernelLoadModule(path.as_ptr(), 0, core::ptr::null_mut());
            if mod_id.0 >= 0 {
                psp::sys::sceKernelStartModule(
                    mod_id,
                    0,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                );
            }
        }

        // Resolve network NIDs.
        if let Some(ptr) = resolve_nid(NET_MODULES, NID_NET_INIT) {
            core::ptr::write_volatile(&raw mut NET_INIT_FN, Some(core::mem::transmute(ptr)));
        }
        if let Some(ptr) = resolve_nid(INET_MODULES, NID_INET_INIT) {
            core::ptr::write_volatile(&raw mut INET_INIT_FN, Some(core::mem::transmute(ptr)));
        }
        if let Some(ptr) = resolve_nid(INET_MODULES, NID_INET_SOCKET) {
            core::ptr::write_volatile(&raw mut INET_SOCKET_FN, Some(core::mem::transmute(ptr)));
        }
        if let Some(ptr) = resolve_nid(INET_MODULES, NID_INET_CONNECT) {
            core::ptr::write_volatile(&raw mut INET_CONNECT_FN, Some(core::mem::transmute(ptr)));
        }
        if let Some(ptr) = resolve_nid(INET_MODULES, NID_INET_SEND) {
            core::ptr::write_volatile(&raw mut INET_SEND_FN, Some(core::mem::transmute(ptr)));
        }
        if let Some(ptr) = resolve_nid(INET_MODULES, NID_INET_RECV) {
            core::ptr::write_volatile(&raw mut INET_RECV_FN, Some(core::mem::transmute(ptr)));
        }
        if let Some(ptr) = resolve_nid(INET_MODULES, NID_INET_CLOSE) {
            core::ptr::write_volatile(&raw mut INET_CLOSE_FN, Some(core::mem::transmute(ptr)));
        }
        if let Some(ptr) = resolve_nid(APCTL_MODULES, NID_APCTL_INIT) {
            core::ptr::write_volatile(&raw mut APCTL_INIT_FN, Some(core::mem::transmute(ptr)));
        }
        if let Some(ptr) = resolve_nid(APCTL_MODULES, NID_APCTL_CONNECT) {
            core::ptr::write_volatile(&raw mut APCTL_CONNECT_FN, Some(core::mem::transmute(ptr)));
        }
        if let Some(ptr) = resolve_nid(APCTL_MODULES, NID_APCTL_GET_STATE) {
            core::ptr::write_volatile(
                &raw mut APCTL_GET_STATE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(RESOLVER_MODULES, NID_RESOLVER_INIT) {
            core::ptr::write_volatile(&raw mut RESOLVER_INIT_FN, Some(core::mem::transmute(ptr)));
        }
        if let Some(ptr) = resolve_nid(RESOLVER_MODULES, NID_RESOLVER_CREATE) {
            core::ptr::write_volatile(
                &raw mut RESOLVER_CREATE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(RESOLVER_MODULES, NID_RESOLVER_START_N2A) {
            core::ptr::write_volatile(
                &raw mut RESOLVER_START_N2A_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(RESOLVER_MODULES, NID_RESOLVER_DELETE) {
            core::ptr::write_volatile(
                &raw mut RESOLVER_DELETE_FN,
                Some(core::mem::transmute(ptr)),
            );
        }

        // Check critical inet NIDs.
        let have_socket = core::ptr::read_volatile(&raw const INET_SOCKET_FN).is_some();
        let have_connect = core::ptr::read_volatile(&raw const INET_CONNECT_FN).is_some();
        let have_send = core::ptr::read_volatile(&raw const INET_SEND_FN).is_some();
        let have_recv = core::ptr::read_volatile(&raw const INET_RECV_FN).is_some();

        if !have_socket || !have_connect || !have_send || !have_recv {
            crate::debug_log(b"[OASIS] critical inet NIDs missing");
            return false;
        }

        // Check if game already has WiFi up.
        let mut state: i32 = 0;
        if let Some(get_state) = core::ptr::read_volatile(&raw const APCTL_GET_STATE_FN) {
            get_state(&mut state);
        }

        if state == 4 {
            crate::debug_log(b"[OASIS] WiFi already connected");
        } else {
            // Init network stack. If sceNetInit returns < 0, the game
            // likely already initialized it -- skip remaining init calls
            // to avoid crashing on double-init.
            let mut we_initialized = false;
            if let Some(f) = core::ptr::read_volatile(&raw const NET_INIT_FN) {
                let ret = f(0x20000, 0x20, 0x1000, 0x20, 0x1000);
                log_i32(b"[OASIS] sceNetInit=", ret);
                if ret >= 0 {
                    we_initialized = true;
                } else {
                    crate::debug_log(b"[OASIS] net already init by game");
                }
            }
            if we_initialized {
                if let Some(f) = core::ptr::read_volatile(&raw const INET_INIT_FN) {
                    let ret = f();
                    log_i32(b"[OASIS] sceNetInetInit=", ret);
                }
                if let Some(f) = core::ptr::read_volatile(&raw const RESOLVER_INIT_FN) {
                    let ret = f();
                    log_i32(b"[OASIS] sceNetResolverInit=", ret);
                }
                if let Some(f) = core::ptr::read_volatile(&raw const APCTL_INIT_FN) {
                    let ret = f(0x1000, 48);
                    log_i32(b"[OASIS] sceNetApctlInit=", ret);
                }
            }

            // Connect to AP 1.
            if let Some(f) = core::ptr::read_volatile(&raw const APCTL_CONNECT_FN) {
                let ret = f(1);
                log_i32(b"[OASIS] sceNetApctlConnect=", ret);
            }

            // Poll until connected (30s timeout).
            if let Some(get_state) = core::ptr::read_volatile(&raw const APCTL_GET_STATE_FN) {
                let mut attempts = 0;
                while attempts < 60 {
                    state = 0;
                    get_state(&mut state);
                    if state == 4 {
                        break;
                    }
                    psp::sys::sceKernelDelayThread(500_000);
                    attempts += 1;
                }
            }

            if state != 4 {
                crate::debug_log(b"[OASIS] WiFi connect failed");
                return false;
            }
            crate::debug_log(b"[OASIS] WiFi connected");
        }

        core::ptr::write_volatile(&raw mut NET_INITIALIZED, true);
        true
    }
}

/// Resolve hostname to IPv4 address using sceNetResolver.
pub(super) unsafe fn resolve_hostname_raw(host: *const u8) -> Option<[u8; 4]> {
    // SAFETY: Calling resolved sceNetResolver functions with valid parameters.
    // RESOLVER_BUF is a stack-like static buffer used as resolver working memory.
    unsafe {
        let create = core::ptr::read_volatile(&raw const RESOLVER_CREATE_FN)?;
        let start = core::ptr::read_volatile(&raw const RESOLVER_START_N2A_FN)?;
        let delete = core::ptr::read_volatile(&raw const RESOLVER_DELETE_FN)?;

        let mut rid: i32 = 0;
        let buf = &raw mut RESOLVER_BUF;
        let ret = create(&mut rid, (*buf).as_mut_ptr(), 1024);
        if ret < 0 {
            log_i32(b"[OASIS] resolver create=", ret);
            return None;
        }

        let mut addr: u32 = 0;
        let ret = start(rid, host, &mut addr, 5, 3);
        delete(rid);
        if ret < 0 {
            log_i32(b"[OASIS] resolver start=", ret);
            return None;
        }

        Some([
            (addr & 0xFF) as u8,
            ((addr >> 8) & 0xFF) as u8,
            ((addr >> 16) & 0xFF) as u8,
            ((addr >> 24) & 0xFF) as u8,
        ])
    }
}

/// Build a sockaddr_in as raw bytes (BSD layout).
pub(super) fn make_sockaddr_in(ip: [u8; 4], port: u16) -> [u8; 16] {
    let mut sa = [0u8; 16];
    sa[0] = 16; // sa_len
    sa[1] = 2; // AF_INET
    let port_be = port.to_be_bytes();
    sa[2] = port_be[0];
    sa[3] = port_be[1];
    sa[4] = ip[0];
    sa[5] = ip[1];
    sa[6] = ip[2];
    sa[7] = ip[3];
    sa
}

/// Send all bytes on a socket (loop until complete).
pub(super) unsafe fn send_all(fd: i32, data: &[u8]) -> bool {
    // SAFETY: Calling resolved sceNetInetSend with valid socket fd and data buffer.
    unsafe {
        let send = match core::ptr::read_volatile(&raw const INET_SEND_FN) {
            Some(f) => f,
            None => return false,
        };
        let mut sent = 0;
        while sent < data.len() {
            let ret = send(fd, data.as_ptr().add(sent), data.len() - sent, 0);
            if ret <= 0 {
                return false;
            }
            sent += ret as usize;
        }
        true
    }
}
