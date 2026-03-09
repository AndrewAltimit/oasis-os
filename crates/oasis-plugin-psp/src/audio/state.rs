//! Audio state, playlist, and helper structures.

use core::sync::atomic::{AtomicBool, AtomicU8};

use super::{copy_bytes, log_i32, write_u32_decimal};

// ---------------------------------------------------------------------------
// Audio state (atomics for cross-thread communication)
// ---------------------------------------------------------------------------

pub(super) static AUDIO_CMD: AtomicU8 = AtomicU8::new(0);
pub(super) static AUDIO_VOLUME: AtomicU8 = AtomicU8::new(128);
pub(super) static AUDIO_STATE: AtomicU8 = AtomicU8::new(0);
pub(super) static AUDIO_AVAILABLE: AtomicU8 = AtomicU8::new(0);
pub(super) static mut TRACK_NAME: [u8; 48] = [0u8; 48];

// ---------------------------------------------------------------------------
// Internet radio state
// ---------------------------------------------------------------------------

pub(super) static RADIO_ACTIVE: AtomicBool = AtomicBool::new(false);
pub(super) static RADIO_STATION_IDX: AtomicU8 = AtomicU8::new(0);
/// ICY "now playing" metadata (written by audio thread, read by overlay).
pub(super) static mut RADIO_META: [u8; 48] = [0u8; 48];

pub(super) struct RadioStation {
    pub(super) name: &'static [u8],
    pub(super) host: &'static [u8],
    pub(super) port: u16,
    pub(super) path: &'static [u8],
}

pub(super) const RADIO_STATIONS: [RadioStation; 8] = [
    RadioStation {
        name: b"Drone Zone",
        host: b"ice2.somafm.com\0",
        port: 80,
        path: b"/dronezone-128-mp3\0",
    },
    RadioStation {
        name: b"DEF CON",
        host: b"ice2.somafm.com\0",
        port: 80,
        path: b"/defcon-128-mp3\0",
    },
    RadioStation {
        name: b"Groove Salad",
        host: b"ice2.somafm.com\0",
        port: 80,
        path: b"/groovesalad-128-mp3\0",
    },
    RadioStation {
        name: b"Space Station",
        host: b"ice2.somafm.com\0",
        port: 80,
        path: b"/spacestation-128-mp3\0",
    },
    RadioStation {
        name: b"Secret Agent",
        host: b"ice2.somafm.com\0",
        port: 80,
        path: b"/secretagent-128-mp3\0",
    },
    RadioStation {
        name: b"Lush",
        host: b"ice2.somafm.com\0",
        port: 80,
        path: b"/lush-128-mp3\0",
    },
    RadioStation {
        name: b"Metal Detector",
        host: b"ice2.somafm.com\0",
        port: 80,
        path: b"/metal-128-mp3\0",
    },
    RadioStation {
        name: b"Boot Liquor",
        host: b"ice2.somafm.com\0",
        port: 80,
        path: b"/bootliquor-128-mp3\0",
    },
];

/// TCP receive staging buffer.
pub(super) static mut RECV_BUF: [u8; 4096] = [0u8; 4096];
/// HTTP request builder buffer.
pub(super) static mut HTTP_BUF: [u8; 512] = [0u8; 512];
/// DNS resolver working buffer.
pub(super) static mut RESOLVER_BUF: [u8; 1024] = [0u8; 1024];

pub(super) struct IcyDemuxer {
    metaint: usize,
    audio_count: usize,
    in_meta: bool,
    meta_remaining: usize,
    meta_buf: [u8; 512],
    meta_buf_len: usize,
}

impl IcyDemuxer {
    pub(super) fn new(metaint: usize) -> Self {
        Self {
            metaint,
            audio_count: 0,
            in_meta: false,
            meta_remaining: 0,
            meta_buf: [0u8; 512],
            meta_buf_len: 0,
        }
    }

    /// Process raw stream data, separating audio from ICY metadata.
    /// Writes audio bytes to `audio_out`.
    /// Returns (audio_written, meta_updated).
    pub(super) fn process(&mut self, data: &[u8], audio_out: &mut [u8]) -> (usize, bool) {
        let mut audio_len = 0;
        let mut meta_updated = false;
        let mut i = 0;

        while i < data.len() {
            if self.in_meta {
                if self.meta_remaining == 0 {
                    let meta_len = data[i] as usize * 16;
                    i += 1;
                    if meta_len == 0 {
                        self.in_meta = false;
                        continue;
                    }
                    self.meta_remaining = meta_len;
                    self.meta_buf_len = 0;
                } else {
                    let take = self.meta_remaining.min(data.len() - i);
                    let copy = take.min(512 - self.meta_buf_len);
                    let mut j = 0;
                    while j < copy {
                        self.meta_buf[self.meta_buf_len + j] = data[i + j];
                        j += 1;
                    }
                    self.meta_buf_len += copy;
                    self.meta_remaining -= take;
                    i += take;
                    if self.meta_remaining == 0 {
                        extract_stream_title(&self.meta_buf[..self.meta_buf_len]);
                        meta_updated = true;
                        self.in_meta = false;
                    }
                }
            } else {
                let remaining = self.metaint - self.audio_count;
                let take = remaining.min(data.len() - i);
                let copy = take.min(audio_out.len() - audio_len);
                let mut j = 0;
                while j < copy {
                    audio_out[audio_len + j] = data[i + j];
                    j += 1;
                }
                audio_len += copy;
                self.audio_count += take;
                i += take;
                if self.audio_count >= self.metaint {
                    self.audio_count = 0;
                    self.in_meta = true;
                    self.meta_remaining = 0;
                }
            }
        }

        (audio_len, meta_updated)
    }
}

/// Extract StreamTitle from ICY metadata into RADIO_META.
fn extract_stream_title(meta: &[u8]) {
    let needle = b"StreamTitle='";
    let mut start = 0;
    while start + needle.len() < meta.len() {
        let mut matched = true;
        let mut k = 0;
        while k < needle.len() {
            if meta[start + k] != needle[k] {
                matched = false;
                break;
            }
            k += 1;
        }
        if matched {
            let title_start = start + needle.len();
            let mut title_end = title_start;
            while title_end < meta.len() {
                if meta[title_end] == b'\''
                    && title_end + 1 < meta.len()
                    && meta[title_end + 1] == b';'
                {
                    break;
                }
                if meta[title_end] == 0 {
                    break;
                }
                title_end += 1;
            }
            let len = (title_end - title_start).min(47);
            // SAFETY: RADIO_META written only from audio thread.
            unsafe {
                let mut j = 0;
                while j < len {
                    (*(&raw mut RADIO_META))[j] = meta[title_start + j];
                    j += 1;
                }
                while j < 48 {
                    (*(&raw mut RADIO_META))[j] = 0;
                    j += 1;
                }
            }
            return;
        }
        start += 1;
    }
}

// ---------------------------------------------------------------------------
// Structures and constants
// ---------------------------------------------------------------------------

/// sceMp3 init structure.
#[repr(C)]
pub(super) struct Mp3InitStruct {
    pub(super) mp3_stream_start: i32,
    pub(super) _unk1: i32,
    pub(super) mp3_stream_end: i32,
    pub(super) _unk2: i32,
    pub(super) mp3_buf: *mut u8,
    pub(super) mp3_buf_size: i32,
    pub(super) pcm_buf: *mut u8,
    pub(super) pcm_buf_size: i32,
}

pub(super) const AUDIO_FORMAT_STEREO: i32 = 0;
pub(super) const MP3_SAMPLES_PER_FRAME: i32 = 1152;
pub(super) const MAX_PLAYLIST: usize = 32;
pub(super) const MAX_FILENAME: usize = 128;
pub(super) const MAX_SCAN_DEPTH: usize = 4;

/// sceMp3 stream buffer (64KB).
pub(super) const MP3_BUF_SIZE: usize = 64 * 1024;
/// sceMp3 PCM decode buffer.
pub(super) const PCM_BUF_SIZE: usize = MP3_SAMPLES_PER_FRAME as usize * 4 * 4;
/// sceAudiocodec read buffer (64KB for fewer I/O stalls).
pub(super) const READ_BUF_SIZE: usize = 64 * 1024;
/// sceAudiocodec codec buffer (65 u32 = 260 bytes, must be 64-byte aligned).
pub(super) const CODEC_BUF_WORDS: usize = 65;
/// sceAudiocodec working memory size (CheckNeedMem reports ~15208, round up).
pub(super) const CODEC_WORK_SIZE: usize = 16 * 1024;

/// Total user-memory allocation for codec buffers.
/// Layout: [64-byte pad] [codec: 260] [pcm: 4608] [work: 16384] [read: 32768]
pub(super) const UMEM_CODEC_SIZE: usize =
    64 + (CODEC_BUF_WORDS * 4) + (1152 * 2 * 2) + CODEC_WORK_SIZE + READ_BUF_SIZE;

/// UID of user-memory block, 0 = not allocated.
pub(super) static mut UMEM_BLOCK_ID: psp::sys::SceUid = psp::sys::SceUid(0);
/// Pointer to codec buffer in user memory (64-byte aligned).
pub(super) static mut UMEM_CODEC: *mut u32 = core::ptr::null_mut();
/// Pointer to PCM buffer in user memory.
pub(super) static mut UMEM_PCM: *mut i16 = core::ptr::null_mut();
/// Pointer to codec working memory (replaces sceAudiocodecGetEDRAM).
#[allow(dead_code)]
pub(super) static mut UMEM_WORK: *mut u8 = core::ptr::null_mut();
/// Pointer to read buffer in user memory.
pub(super) static mut UMEM_READ: *mut u8 = core::ptr::null_mut();

/// Allocate codec buffers in user memory partition (partition 2).
/// Required because syscall stubs validate that pointers are in user range.
pub(super) unsafe fn alloc_codec_user_mem() -> bool {
    // SAFETY: sceKernelAllocPartitionMemory with valid partition ID and size.
    let block = unsafe {
        psp::sys::sceKernelAllocPartitionMemory(
            psp::sys::SceSysMemPartitionId::SceKernelPrimaryUserPartition,
            b"oasis_codec\0".as_ptr(),
            psp::sys::SceSysMemBlockTypes::Low,
            UMEM_CODEC_SIZE as u32,
            core::ptr::null_mut(),
        )
    };
    if block < psp::sys::SceUid(0) {
        crate::debug_log(b"[OASIS] user mem alloc failed");
        return false;
    }
    // SAFETY: block is a valid memory block ID returned by sceKernelAllocPartitionMemory.
    let base = unsafe { psp::sys::sceKernelGetBlockHeadAddr(block) } as *mut u8;
    if base.is_null() {
        crate::debug_log(b"[OASIS] user mem addr null");
        return false;
    }
    // SAFETY: base is valid; pointer arithmetic stays within the allocated block.
    // Single-threaded init, statics written once.
    unsafe {
        UMEM_BLOCK_ID = block;
        // Align codec buffer to 64 bytes.
        let codec_off = (64 - (base as usize % 64)) % 64;
        UMEM_CODEC = base.add(codec_off) as *mut u32;
        let pcm_off = codec_off + CODEC_BUF_WORDS * 4;
        UMEM_PCM = base.add(pcm_off) as *mut i16;
        let work_off = pcm_off + 1152 * 2 * 2;
        UMEM_WORK = base.add(work_off) as *mut u8;
        let read_off = work_off + CODEC_WORK_SIZE;
        UMEM_READ = base.add(read_off) as *mut u8;
    }
    log_i32(b"[OASIS] user mem @", base as i32);
    true
}

/// Free user-memory block if allocated.
#[allow(dead_code)]
pub(super) unsafe fn free_codec_user_mem() {
    // SAFETY: UMEM_BLOCK_ID is valid if >= SceUid(0); freeing the partition memory.
    unsafe {
        if UMEM_BLOCK_ID >= psp::sys::SceUid(0) && UMEM_BLOCK_ID != psp::sys::SceUid(0) {
            psp::sys::sceKernelFreePartitionMemory(UMEM_BLOCK_ID);
            UMEM_BLOCK_ID = psp::sys::SceUid(0);
        }
    }
}

// ---------------------------------------------------------------------------
// Playlist data
// ---------------------------------------------------------------------------

pub(super) static mut PLAYLIST: [[u8; MAX_FILENAME]; MAX_PLAYLIST] =
    [[0u8; MAX_FILENAME]; MAX_PLAYLIST];
pub(super) static mut PLAYLIST_LEN: usize = 0;
pub(super) static mut CURRENT_TRACK: usize = 0;

/// Path to companion MP3 for PIP video (set by video module).
pub(super) static mut VIDEO_MP3_PATH: [u8; 128] = [0u8; 128];

/// Whether PIP video audio is active (video module sets, audio thread reads).
pub(super) static VIDEO_MP3_ACTIVE: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Playlist scanning
// ---------------------------------------------------------------------------

pub(super) unsafe fn scan_playlist() {
    let config = crate::config::get_config();
    // SAFETY: Volatile write/read of PLAYLIST_LEN; called only from audio thread.
    // scan_dir_recursive accesses PLAYLIST statics from this thread only.
    unsafe {
        core::ptr::write_volatile(&raw mut PLAYLIST_LEN, 0);
        scan_dir_recursive(&config.music_dir, config.music_dir_len, 0);
    }
    // SAFETY: Volatile read of PLAYLIST_LEN after scan_dir_recursive populated it.
    let count = unsafe { core::ptr::read_volatile(&raw const PLAYLIST_LEN) };
    let mut buf = [0u8; 48];
    let mut p = copy_bytes(&mut buf, 0, b"[OASIS] found ");
    p = write_u32_decimal(&mut buf, p, count as u32);
    p = copy_bytes(&mut buf, p, b" mp3 files");
    crate::debug_log(&buf[..p]);
}

pub(super) unsafe fn scan_dir_recursive(dir_path: &[u8], dir_len: usize, depth: usize) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    // SAFETY: Volatile read of PLAYLIST_LEN; accessed only from audio thread.
    let pl_len = unsafe { core::ptr::read_volatile(&raw const PLAYLIST_LEN) };
    if pl_len >= MAX_PLAYLIST {
        return;
    }

    // SAFETY: sceIoDopen with valid null-terminated directory path.
    let dfd = unsafe { psp::sys::sceIoDopen(dir_path.as_ptr()) };
    if dfd.0 < 0 {
        if depth == 0 {
            crate::debug_log(b"[OASIS] music dir not found");
        }
        return;
    }

    // SAFETY: sceIoDread/sceIoDclose with valid directory fd. SceIoDirent is
    // repr(C) and zero-initialization is valid. Pointer arithmetic on d_name
    // stays within the 256-byte name buffer. PLAYLIST entries written within
    // bounds (PLAYLIST_LEN < MAX_PLAYLIST).
    unsafe {
        let mut dirent = core::mem::zeroed::<psp::sys::SceIoDirent>();
        loop {
            let ret = psp::sys::sceIoDread(dfd, &mut dirent);
            if ret <= 0 {
                break;
            }
            let pl_len = core::ptr::read_volatile(&raw const PLAYLIST_LEN);
            if pl_len >= MAX_PLAYLIST {
                break;
            }

            let name_ptr = dirent.d_name.as_ptr() as *const u8;
            let mut name_len = 0usize;
            while name_len < 256 && *name_ptr.add(name_len) != 0 {
                name_len += 1;
            }
            if name_len == 0 {
                continue;
            }
            // Skip "." and ".."
            if name_len == 1 && *name_ptr == b'.' {
                continue;
            }
            if name_len == 2 && *name_ptr == b'.' && *name_ptr.add(1) == b'.' {
                continue;
            }

            let is_dir = (dirent.d_stat.st_attr.bits() & 0x0010) != 0;

            if is_dir {
                let sub_len = dir_len + name_len + 1;
                if sub_len + 1 > MAX_FILENAME {
                    continue;
                }
                let mut sub_path = [0u8; MAX_FILENAME];
                let mut j = 0;
                while j < dir_len {
                    sub_path[j] = dir_path[j];
                    j += 1;
                }
                let mut k = 0;
                while k < name_len {
                    sub_path[j + k] = *name_ptr.add(k);
                    k += 1;
                }
                sub_path[j + name_len] = b'/';
                sub_path[j + name_len + 1] = 0;
                scan_dir_recursive(&sub_path, sub_len, depth + 1);
            } else {
                if name_len < 5 {
                    continue;
                }
                let e = name_len - 4;
                if *name_ptr.add(e) != b'.'
                    || (*name_ptr.add(e + 1)).to_ascii_lowercase() != b'm'
                    || (*name_ptr.add(e + 2)).to_ascii_lowercase() != b'p'
                    || (*name_ptr.add(e + 3)).to_ascii_lowercase() != b'3'
                {
                    continue;
                }
                let total_len = dir_len + name_len;
                if total_len + 1 > MAX_FILENAME {
                    continue;
                }
                let entry = &mut (*(&raw mut PLAYLIST))[pl_len];
                let mut j = 0;
                while j < dir_len {
                    entry[j] = dir_path[j];
                    j += 1;
                }
                let mut k = 0;
                while k < name_len {
                    entry[j + k] = *name_ptr.add(k);
                    k += 1;
                }
                entry[j + k] = 0;
                core::ptr::write_volatile(&raw mut PLAYLIST_LEN, pl_len + 1);
            }
        }
        psp::sys::sceIoDclose(dfd);
    }
}

// ---------------------------------------------------------------------------
// Track name
// ---------------------------------------------------------------------------

pub(super) unsafe fn set_track_name(path: &[u8]) {
    // SAFETY: Writing to TRACK_NAME static buffer; called only from audio thread.
    // copy_len is clamped to 47 (buffer is 48 bytes).
    unsafe {
        let mut last_slash = 0;
        let mut i = 0;
        while i < path.len() && path[i] != 0 {
            if path[i] == b'/' {
                last_slash = i + 1;
            }
            i += 1;
        }
        let name = &path[last_slash..];
        let mut len = 0;
        while len < name.len() && name[len] != 0 {
            len += 1;
        }
        if len >= 4
            && name[len - 4] == b'.'
            && name[len - 3].to_ascii_lowercase() == b'm'
            && name[len - 2].to_ascii_lowercase() == b'p'
            && name[len - 1].to_ascii_lowercase() == b'3'
        {
            len -= 4;
        }
        let copy_len = len.min(47);
        let mut j = 0;
        while j < copy_len {
            (*(&raw mut TRACK_NAME))[j] = name[j];
            j += 1;
        }
        while j < 48 {
            (*(&raw mut TRACK_NAME))[j] = 0;
            j += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// OASIS OS detection
// ---------------------------------------------------------------------------

/// Check if the OASIS OS EBOOT is the running application.
///
/// When running alongside OASIS OS, the PRX must NOT use audio codecs
/// because OASIS OS has its own music player (sceAudiocodec) which shares
/// the ME coprocessor's EDRAM. Both cannot be active simultaneously.
///
/// Detection: scan the first 1.5MB of user memory for the "OASIS_OS"
/// string embedded in the EBOOT's SceModuleInfo (generated by
/// `psp::module!("OASIS_OS", ...)`). This works from kernel mode
/// without any kernel APIs (which may not be available on all CFW).
pub(super) unsafe fn is_oasis_running() -> bool {
    let needle = b"OASIS_OS\0";
    let start: u32 = 0x0880_0000;
    let end: u32 = 0x0898_0000; // First 1.5MB of user memory
    let mut addr = start;
    while addr <= end - (needle.len() as u32) {
        let mut matched = true;
        let mut j = 0usize;
        while j < needle.len() {
            // SAFETY: Volatile read from user memory (0x08800000-0x09800000).
            // Kernel mode has full access to user memory range.
            let byte = unsafe { core::ptr::read_volatile((addr + j as u32) as *const u8) };
            if byte != needle[j] {
                matched = false;
                break;
            }
            j += 1;
        }
        if matched {
            return true;
        }
        addr += 4; // word-aligned scan
    }
    false
}
