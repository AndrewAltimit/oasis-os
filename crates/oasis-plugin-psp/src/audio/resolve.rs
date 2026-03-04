//! Function pointer statics, NID resolution, and module enumeration.

use super::nids::*;
use super::state::Mp3InitStruct;
use super::{copy_bytes, log_i32, write_hex32};

// ---------------------------------------------------------------------------
// Resolved function pointers
// ---------------------------------------------------------------------------

pub(super) static mut AUDIO_CH_RESERVE_FN: Option<unsafe extern "C" fn(i32, i32, i32) -> i32> =
    None;
pub(super) static mut AUDIO_OUTPUT_BLOCKING_FN: Option<
    unsafe extern "C" fn(i32, i32, *const u8) -> i32,
> = None;
#[allow(dead_code)]
pub(super) static mut AUDIO_CH_RELEASE_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
pub(super) static mut AUDIO_SET_CH_VOL_FN: Option<unsafe extern "C" fn(i32, i32, i32) -> i32> =
    None;

// SRC channel function pointers (preferred -- no conflict with games)
pub(super) static mut AUDIO_SRC_RESERVE_FN: Option<unsafe extern "C" fn(i32, i32, i32) -> i32> =
    None;
pub(super) static mut AUDIO_SRC_OUTPUT_FN: Option<unsafe extern "C" fn(i32, *const u8) -> i32> =
    None;
#[allow(dead_code)]
pub(super) static mut AUDIO_SRC_RELEASE_FN: Option<unsafe extern "C" fn() -> i32> = None;
/// Whether we use SRC output (true) or regular channel (false).
pub(super) static mut USE_SRC_OUTPUT: bool = false;

// Which decoder backend is active: 0=none, 1=sceMp3, 2=sceAudiocodec
pub(super) static mut DECODER_BACKEND: u8 = 0;

// sceMp3 function pointers
pub(super) static mut MP3_INIT_RESOURCE_FN: Option<unsafe extern "C" fn() -> i32> = None;
pub(super) static mut MP3_RESERVE_HANDLE_FN: Option<
    unsafe extern "C" fn(*const Mp3InitStruct) -> i32,
> = None;
pub(super) static mut MP3_RELEASE_HANDLE_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
pub(super) static mut MP3_INIT_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
pub(super) static mut MP3_DECODE_FN: Option<unsafe extern "C" fn(i32, *mut *const i16) -> i32> =
    None;
pub(super) static mut MP3_CHECK_NEED_DATA_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
pub(super) static mut MP3_GET_INFO_TO_ADD_FN: Option<
    unsafe extern "C" fn(i32, *mut *mut u8, *mut i32, *mut i32) -> i32,
> = None;
pub(super) static mut MP3_NOTIFY_ADD_DATA_FN: Option<unsafe extern "C" fn(i32, i32) -> i32> = None;

// sceAudiocodec function pointers
pub(super) static mut CODEC_CHECK_NEED_MEM_FN: Option<unsafe extern "C" fn(*mut u32, i32) -> i32> =
    None;
pub(super) static mut CODEC_INIT_FN: Option<unsafe extern "C" fn(*mut u32, i32) -> i32> = None;
pub(super) static mut CODEC_DECODE_FN: Option<unsafe extern "C" fn(*mut u32, i32) -> i32> = None;
pub(super) static mut CODEC_GET_EDRAM_FN: Option<unsafe extern "C" fn(*mut u32, i32) -> i32> =
    None;
pub(super) static mut CODEC_RELEASE_EDRAM_FN: Option<unsafe extern "C" fn(*mut u32) -> i32> = None;

// sceKernelGetModuleIdList function pointer
pub(super) static mut GET_MODULE_ID_LIST_FN: Option<
    unsafe extern "C" fn(*mut i32, i32, *mut i32) -> i32,
> = None;
// sceKernelQueryModuleInfo function pointer
pub(super) static mut QUERY_MODULE_INFO_FN: Option<unsafe extern "C" fn(i32, *mut u8) -> i32> =
    None;

// Network function pointers (resolved lazily for radio streaming)
pub(super) static mut NET_INIT_FN: Option<
    unsafe extern "C" fn(i32, i32, i32, i32, i32) -> i32,
> = None;
pub(super) static mut INET_INIT_FN: Option<unsafe extern "C" fn() -> i32> = None;
pub(super) static mut INET_SOCKET_FN: Option<unsafe extern "C" fn(i32, i32, i32) -> i32> = None;
pub(super) static mut INET_CONNECT_FN: Option<unsafe extern "C" fn(i32, *const u8, u32) -> i32> =
    None;
pub(super) static mut INET_SEND_FN: Option<unsafe extern "C" fn(i32, *const u8, usize, i32) -> i32> =
    None;
pub(super) static mut INET_RECV_FN: Option<unsafe extern "C" fn(i32, *mut u8, usize, i32) -> i32> =
    None;
pub(super) static mut INET_CLOSE_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
pub(super) static mut APCTL_INIT_FN: Option<unsafe extern "C" fn(i32, i32) -> i32> = None;
pub(super) static mut APCTL_CONNECT_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
pub(super) static mut APCTL_GET_STATE_FN: Option<unsafe extern "C" fn(*mut i32) -> i32> = None;
pub(super) static mut RESOLVER_INIT_FN: Option<unsafe extern "C" fn() -> i32> = None;
pub(super) static mut RESOLVER_CREATE_FN: Option<
    unsafe extern "C" fn(*mut i32, *mut u8, u32) -> i32,
> = None;
pub(super) static mut RESOLVER_START_N2A_FN: Option<
    unsafe extern "C" fn(i32, *const u8, *mut u32, u32, i32) -> i32,
> = None;
pub(super) static mut RESOLVER_DELETE_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;

/// Whether the network stack has been initialized.
pub(super) static mut NET_INITIALIZED: bool = false;

/// Text addresses of discovered MP3/codec modules (from enumeration).
pub(super) static mut MP3_TEXT_ADDR: u32 = 0;
pub(super) static mut CODEC_TEXT_ADDR: u32 = 0;

// ---------------------------------------------------------------------------
// NID resolution helpers
// ---------------------------------------------------------------------------

/// Resolve a NID trying each module/library pair, then null-module fallback.
pub(super) unsafe fn resolve_nid(modules: &[(&[u8], &[u8])], nid: u32) -> Option<*mut u8> {
    // Try each named module/library pair.
    for &(module, library) in modules {
        // SAFETY: find_function calls sctrlHENFindFunction (CFW kernel API)
        // with valid null-terminated module/library name pointers.
        if let Some(ptr) =
            unsafe { psp::hook::find_function(module.as_ptr(), library.as_ptr(), nid) }
        {
            return Some(ptr);
        }
    }
    // Fallback: NULL module name (searches all loaded modules on PRO/ME/ARK).
    for &(_, library) in modules {
        // SAFETY: find_function with null module name searches all loaded modules.
        if let Some(ptr) =
            unsafe { psp::hook::find_function(core::ptr::null(), library.as_ptr(), nid) }
        {
            return Some(ptr);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Module enumeration and export walking
// ---------------------------------------------------------------------------

/// Check if a pointer looks like a valid PSP memory address.
pub(super) fn is_valid_ptr(addr: u32) -> bool {
    if addr == 0 || addr & 3 != 0 {
        return false;
    }
    // User space:  0x08800000 - 0x0BFFFFFF
    // Kernel KSEG0: 0x80000000 - 0x8BFFFFFF
    // Kernel KSEG1: 0xA0000000 - 0xABFFFFFF
    (addr >= 0x0800_0000 && addr < 0x0C00_0000)
        || (addr >= 0x8000_0000 && addr < 0x8C00_0000)
        || (addr >= 0xA000_0000 && addr < 0xAC00_0000)
}

/// Resolve module enumeration APIs.
pub(super) unsafe fn init_module_enum() -> bool {
    // SAFETY: Resolving kernel module manager NIDs via sctrlHENFindFunction;
    // transmuting raw pointers to typed fn pointers. Single-threaded init.
    unsafe {
        if let Some(ptr) = resolve_nid(MOD_MGR_MODULES, NID_GET_MODULE_ID_LIST) {
            core::ptr::write_volatile(
                &raw mut GET_MODULE_ID_LIST_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        if let Some(ptr) = resolve_nid(MOD_MGR_MODULES, NID_QUERY_MODULE_INFO) {
            core::ptr::write_volatile(
                &raw mut QUERY_MODULE_INFO_FN,
                Some(core::mem::transmute(ptr)),
            );
        }
        let have_list = core::ptr::read_volatile(&raw const GET_MODULE_ID_LIST_FN).is_some();
        let have_query = core::ptr::read_volatile(&raw const QUERY_MODULE_INFO_FN).is_some();
        if have_list && have_query {
            crate::debug_log(b"[OASIS] module enum APIs resolved");
            true
        } else {
            crate::debug_log(b"[OASIS] module enum APIs NOT found");
            false
        }
    }
}

/// Enumerate all loaded modules and log their names.
/// Stores text_addr for MP3/codec modules when found.
pub(super) unsafe fn enumerate_modules() {
    // SAFETY: Calling resolved kernel APIs (sceKernelGetModuleIdList,
    // sceKernelQueryModuleInfo) with properly-sized stack buffers.
    // Reading module info structs at documented offsets.
    unsafe {
        let get_list = match core::ptr::read_volatile(&raw const GET_MODULE_ID_LIST_FN) {
            Some(f) => f,
            None => return,
        };
        let query = match core::ptr::read_volatile(&raw const QUERY_MODULE_INFO_FN) {
            Some(f) => f,
            None => return,
        };

        let mut ids = [0i32; 128];
        let mut count: i32 = 0;
        let ret = get_list(ids.as_mut_ptr(), (128 * 4) as i32, &mut count);
        if ret < 0 {
            log_i32(b"[OASIS] GetModuleIdList err=", ret);
            return;
        }
        log_i32(b"[OASIS] loaded modules: ", count);

        let n = (count as usize).min(128);
        let mut i = 0;
        while i < n {
            let mut info = [0u8; 96];
            // Set size field at offset 0.
            let size_ptr = info.as_mut_ptr() as *mut u32;
            *size_ptr = MODULE_INFO_SIZE;

            let qr = query(ids[i], info.as_mut_ptr());
            if qr < 0 {
                i += 1;
                continue;
            }

            // text_addr at offset 0x30.
            let text_addr = *(info.as_ptr().add(MODINFO_TEXT_ADDR) as *const u32);
            // name at offset 0x44 (28 bytes, null-terminated).
            let name = &info[MODINFO_NAME..MODINFO_NAME + 28];

            // Log: "mod: <name> @XXXXXXXX"
            let mut buf = [0u8; 64];
            let mut p = copy_bytes(&mut buf, 0, b"[OASIS] mod: ");
            let mut k = 0;
            while k < 28 && name[k] != 0 && p < buf.len() - 12 {
                buf[p] = name[k];
                p += 1;
                k += 1;
            }
            p = copy_bytes(&mut buf, p, b" @");
            p = write_hex32(&mut buf, p, text_addr);
            crate::debug_log(&buf[..p]);

            // Check if this is an MP3 or codec module.
            if contains_pattern(name, MP3_NAME_PATTERNS) {
                core::ptr::write_volatile(&raw mut MP3_TEXT_ADDR, text_addr);
                crate::debug_log(b"[OASIS] => MP3 module!");
            }
            if contains_pattern(name, CODEC_NAME_PATTERNS) {
                core::ptr::write_volatile(&raw mut CODEC_TEXT_ADDR, text_addr);
                crate::debug_log(b"[OASIS] => CODEC module!");
            }

            i += 1;
        }
    }
}

/// Check if `name` contains any of the given patterns.
pub(super) fn contains_pattern(name: &[u8], patterns: &[&[u8]]) -> bool {
    for &pat in patterns {
        if byte_contains(name, pat) {
            return true;
        }
    }
    false
}

pub(super) fn byte_contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    let hlen = {
        let mut l = 0;
        while l < haystack.len() && haystack[l] != 0 {
            l += 1;
        }
        l
    };
    if needle.len() > hlen {
        return false;
    }
    let mut i = 0;
    while i + needle.len() <= hlen {
        let mut matched = true;
        let mut j = 0;
        while j < needle.len() {
            if haystack[i + j] != needle[j] {
                matched = false;
                break;
            }
            j += 1;
        }
        if matched {
            return true;
        }
        i += 1;
    }
    false
}

/// Walk exports starting from a module's text segment address.
///
/// The SceModuleInfo header is at the start of the text segment for
/// PRX modules. It contains ent_top (+0x24) and ent_end (+0x28).
pub(super) unsafe fn walk_exports_from_text(text_addr: u32, nid: u32) -> Option<*mut u8> {
    if !is_valid_ptr(text_addr) {
        return None;
    }
    // SAFETY: text_addr validated by is_valid_ptr; reading SceModuleInfo header
    // at documented offsets (ent_top, ent_end) from the module's text segment.
    unsafe {
        let base = text_addr as *const u8;
        let ent_top_val = *(base.add(SCEMODINFO_ENT_TOP) as *const u32);
        let ent_end_val = *(base.add(SCEMODINFO_ENT_END) as *const u32);

        if !is_valid_ptr(ent_top_val) || !is_valid_ptr(ent_end_val) {
            return None;
        }
        if ent_end_val <= ent_top_val {
            return None;
        }
        let ent_size = (ent_end_val - ent_top_val) as usize;
        if ent_size > 0x10000 {
            return None;
        }

        walk_export_table(ent_top_val as *const u8, ent_size, nid)
    }
}

/// Walk an export table (array of SceLibraryEntryTable entries).
pub(super) unsafe fn walk_export_table(
    ent_top: *const u8,
    ent_size: usize,
    nid: u32,
) -> Option<*mut u8> {
    // SAFETY: ent_top validated by is_valid_ptr; reading SceLibraryEntryTable
    // entries at documented field offsets. Pointer reads validated before use.
    unsafe {
        let mut offset = 0usize;
        while offset < ent_size {
            let entry = ent_top.add(offset);

            // SceLibraryEntryTable:
            //   +0x00: name (char*)
            //   +0x04: version (u16) | attribute (u16)
            //   +0x08: entLen (u8) | varCount (u8) | funcCount (u16)
            //   +0x0C: entrytable (u32*)
            let ent_len = *entry.add(8) as usize;
            if ent_len < 4 || ent_len > 16 {
                break;
            }

            let var_count = *entry.add(9) as usize;
            let func_count = *(entry.add(10) as *const u16) as usize;
            let entrytable = *(entry.add(12) as *const u32) as *const u32;

            if !entrytable.is_null()
                && func_count > 0
                && func_count < 256
                && is_valid_ptr(entrytable as u32)
            {
                let mut i = 0;
                while i < func_count {
                    if *entrytable.add(i) == nid {
                        let func_ptr = *entrytable.add(func_count + var_count + i);
                        if func_ptr != 0 {
                            return Some(func_ptr as *mut u8);
                        }
                    }
                    i += 1;
                }
            }

            offset += ent_len * 4;
        }
    }
    None
}

/// Resolve a NID: try sctrlHENFindFunction, then export walking.
pub(super) unsafe fn resolve_nid_any(
    modules: &[(&[u8], &[u8])],
    text_addr_ptr: *const u32,
    nid: u32,
) -> Option<*mut u8> {
    // Fast path: sctrlHENFindFunction (kernel modules).
    // SAFETY: resolve_nid calls sctrlHENFindFunction with valid module/library names.
    if let Some(ptr) = unsafe { resolve_nid(modules, nid) } {
        return Some(ptr);
    }
    // Slow path: walk exports from discovered text_addr.
    // SAFETY: Volatile read of text_addr_ptr; caller ensures it points to valid memory.
    let text_addr = unsafe { core::ptr::read_volatile(text_addr_ptr) };
    if text_addr != 0 {
        // SAFETY: walk_exports_from_text reads module export tables at text_addr.
        if let Some(ptr) = unsafe { walk_exports_from_text(text_addr, nid) } {
            return Some(ptr);
        }
    }
    None
}
