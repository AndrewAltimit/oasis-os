//! Video decode thread for TV Guide playback.
//!
//! Uses oasis-video's `demux_lite::Mp4Lite` for lightweight MP4 parsing
//! (no symphonia, no lazy_static, no std::sync::Once -- PPSSPP-safe).
//! Audio AAC samples are forwarded to the audio thread for hardware decode.
//! Video H.264 frames are decoded via `sceMpeg` (Media Engine) on real PSP
//! hardware, with PSMF container wrapping and direct ABGR pixel output.
//!
//! The sceMpeg API is used instead of the lower-level sceVideocodec because
//! sceVideocodec weak imports fail to resolve on many CFW configurations
//! (error 0x806201fe), while sceMpeg is universally available.
//!
//! The ringbuffer callback feeds PSMF-structured data (2048-byte packets):
//! first packet is the PSMF header, subsequent packets are MPEG-PS packs
//! containing H.264 AUs in PES video packets.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};
use psp::sync::SpscQueue;
use psp::thread::ThreadBuilder;
use crate::psmf::{PACKET_SIZE, PsmfMuxer};
use crate::threading::{AudioCmd, send_audio_cmd};

/// File-based debug logging (works from video thread, unlike psp::dprintln).
fn vlog(msg: &str) {
    // SAFETY: sceIo calls with valid path and buffer pointers.
    unsafe {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
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

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Commands for the video decode thread.
pub enum VideoCmd {
    /// Start decoding a downloaded MP4 file.
    Play { path: String, seek_secs: u64 },
    /// Begin streaming mode — I/O thread will push frames via the stream queue.
    StreamStart,
    /// Stop current playback.
    Stop,
    /// Shut down the thread.
    Shutdown,
}

/// A pre-demuxed H.264 access unit pushed by the I/O thread for decode.
pub struct StreamFrame {
    pub data: Vec<u8>,
    pub timestamp_secs: f64,
    pub is_keyframe: bool,
}

/// A decoded video frame ready for texture upload.
///
/// Identical to `oasis_video::h264::DecodedFrame` but defined separately
/// because the PSP backend is excluded from the workspace (different target
/// architecture) and cannot depend on oasis-video's h264 module.
pub struct DecodedFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

// ---------------------------------------------------------------------------
// Static queues and state
// ---------------------------------------------------------------------------

/// Commands: main thread -> video thread.
static VIDEO_CMD_QUEUE: SpscQueue<VideoCmd, 4> = SpscQueue::new();
/// Decoded frames: video thread -> main thread (double-buffered).
static VIDEO_FRAME_QUEUE: SpscQueue<DecodedFrame, 2> = SpscQueue::new();
/// Pre-demuxed H.264 frames: I/O thread -> video thread (streaming mode).
/// 8 slots provide enough buffering to absorb I/O jitter while keeping
/// memory usage bounded (~8 × avg H.264 AU ≈ 200KB for 480p content).
static VIDEO_STREAM_QUEUE: SpscQueue<StreamFrame, 8> = SpscQueue::new();
/// Whether video is currently playing.
static VIDEO_PLAYING: AtomicBool = AtomicBool::new(false);
/// Set by I/O thread to signal video thread to enter streaming mode.
/// The video thread clears it once it starts `play_stream()`.
static STREAM_REQUESTED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Send a command to the video decode thread.
pub fn send_video_cmd(cmd: VideoCmd) {
    let _ = VIDEO_CMD_QUEUE.push(cmd);
}

/// Poll for the next decoded video frame (non-blocking).
pub fn poll_video_frame() -> Option<DecodedFrame> {
    VIDEO_FRAME_QUEUE.pop()
}

/// Check if video is currently playing.
pub fn is_video_playing() -> bool {
    VIDEO_PLAYING.load(Ordering::Relaxed)
}

/// Set the playing flag from outside the video thread.
pub fn set_video_playing(val: bool) {
    VIDEO_PLAYING.store(val, Ordering::Release);
}

/// Request the video thread to enter streaming mode.
/// Called by the I/O thread (avoids SPSC queue two-producer issue).
pub fn request_stream_start() {
    STREAM_REQUESTED.store(true, Ordering::Release);
}

/// Push a pre-demuxed H.264 frame for streaming decode.
/// Returns `Ok(())` on success, or `Err(frame)` if the queue was full
/// (caller should retry after a short sleep).
pub fn try_push_stream_frame(frame: StreamFrame) -> Result<(), StreamFrame> {
    VIDEO_STREAM_QUEUE.push(frame)
}

/// Test sceMpeg decode with a real PMF file from the memory stick.
/// This mirrors exactly how a PMF player works: open file, setup sceMpeg,
/// feed via ringbuffer callback (sceIoRead), decode.
pub fn test_real_pmf() {
    vlog("[PMF-TEST] Starting real PMF test...");

    crate::audio::load_av_modules_once_pub();
    // Reset sceMpeg completely — preinit_mpeg may have left stale state.
    unsafe { psp::sys::sceMpegFinish() };
    let ret = unsafe { psp::sys::sceMpegInit() };
    vlog(&format!("[PMF-TEST] sceMpegFinish+Init = {ret:#x}"));

    // Open the real PMF file
    let fd = unsafe {
        psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/test.pmf\0".as_ptr(),
            psp::sys::IoOpenFlags::RD_ONLY,
            0,
        )
    };
    if fd < psp::sys::SceUid(0) {
        vlog("[PMF-TEST] Failed to open test.pmf");
        return;
    }
    vlog("[PMF-TEST] Opened test.pmf");

    // Read the PSMF header (first 2048 bytes)
    let mut header = [0u8; 2048];
    let n = unsafe {
        psp::sys::sceIoRead(fd, header.as_mut_ptr() as *mut _, 2048)
    };
    vlog(&format!("[PMF-TEST] Read header: {n} bytes"));

    // Store fd in a static for the callback
    static mut PMF_FD: psp::sys::SceUid = psp::sys::SceUid(-1);
    unsafe { PMF_FD = fd; }

    // DON'T seek to 0 — the callback must read MPEG-PS data only,
    // NOT the PSMF header. The header is parsed by QueryStreamOffset.
    // We'll seek to stream_offset (2048) after QueryStreamOffset.

    // Ringbuffer callback: reads from the PMF file (exactly like PMF players)
    unsafe extern "C" fn pmf_callback(
        data: *mut core::ffi::c_void,
        num_packets: i32,
        _param: *mut core::ffi::c_void,
    ) -> i32 {
        if num_packets <= 0 {
            return 0;
        }
        let bytes = num_packets * 2048;
        let n = psp::sys::sceIoRead(PMF_FD, data, bytes as u32);
        if n < 0 {
            return -1;
        }
        n / 2048
    }

    // Setup sceMpeg
    let mem_size = unsafe { psp::sys::sceMpegQueryMemSize(0) };
    let mut mpeg_data = vec![0u8; mem_size as usize + 64];
    let mpeg_data_aligned = {
        let p = mpeg_data.as_mut_ptr();
        unsafe { p.add(p.align_offset(64)) }
    };

    let rb_size = unsafe { psp::sys::sceMpegRingbufferQueryMemSize(32) };
    let mut rb_data = vec![0u8; rb_size as usize];

    let mut ringbuffer = Box::new(unsafe {
        core::mem::zeroed::<psp::sys::SceMpegRingbuffer>()
    });
    let ret = unsafe {
        psp::sys::sceMpegRingbufferConstruct(
            &mut *ringbuffer,
            32,
            rb_data.as_mut_ptr() as *mut core::ffi::c_void,
            rb_size,
            Some(pmf_callback),
            core::ptr::null_mut(),
        )
    };
    vlog(&format!("[PMF-TEST] RingbufferConstruct = {ret:#x}"));

    let mpeg_storage = Box::into_raw(Box::new(core::ptr::null_mut::<core::ffi::c_void>()));
    let mpeg: psp::sys::SceMpeg = unsafe {
        core::mem::transmute(mpeg_storage as *mut *mut core::ffi::c_void)
    };

    let ret = unsafe {
        psp::sys::sceMpegCreate(
            mpeg,
            mpeg_data_aligned as *mut core::ffi::c_void,
            mem_size as i32,
            &mut *ringbuffer,
            512,
            0,
            0,
        )
    };
    vlog(&format!("[PMF-TEST] sceMpegCreate = {ret:#x}"));
    if ret < 0 {
        unsafe {
            psp::sys::sceIoClose(fd);
            let _ = Box::from_raw(mpeg_storage);
            psp::sys::sceMpegRingbufferDestruct(&mut *ringbuffer);
        }
        return;
    }

    // CRITICAL: QueryStreamOffset MUST be called before any Put.
    // It calls AnalyzeMpeg internally, initializing the kernel's demuxer.
    let mut stream_offset: i32 = 0;
    let ret = unsafe {
        psp::sys::sceMpegQueryStreamOffset(
            mpeg,
            header.as_ptr() as *mut core::ffi::c_void,
            &mut stream_offset,
        )
    };
    vlog(&format!("[PMF-TEST] QueryStreamOffset = {ret:#x}, offset={stream_offset}"));

    let mut stream_size: i32 = 0;
    let ret = unsafe {
        psp::sys::sceMpegQueryStreamSize(
            header.as_ptr() as *mut core::ffi::c_void,
            &mut stream_size,
        )
    };
    vlog(&format!("[PMF-TEST] QueryStreamSize = {ret:#x}, size={stream_size}"));

    // Seek file past PSMF header to stream data.
    // The ringbuffer callback must read ONLY MPEG-PS data, not the header.
    unsafe { psp::sys::sceIoLseek(fd, stream_offset as i64, psp::sys::IoWhence::Set) };
    vlog(&format!("[PMF-TEST] Seeked to offset {stream_offset}"));

    // Register video stream AFTER QueryStreamOffset (order matters!)
    let stream = unsafe { psp::sys::sceMpegRegistStream(mpeg, 0, 0) };
    vlog("[PMF-TEST] Stream registered (channel 0 = 0xE0)");

    // Allocate ES buffer + init AU
    let es_buf = unsafe { psp::sys::sceMpegMallocAvcEsBuf(mpeg) };
    vlog(&format!("[PMF-TEST] MallocAvcEsBuf = {:#x}", es_buf as usize));
    let mut au = unsafe { core::mem::zeroed::<psp::sys::SceMpegAu>() };
    let ret = unsafe { psp::sys::sceMpegInitAu(mpeg, es_buf, &mut au) };
    vlog(&format!("[PMF-TEST] InitAu = {ret:#x}"));

    // Feed + decode loop: 1 packet at a time, GetAvcAu after each.
    let mut total_put = 0i32;
    let mut decode_init = 0i32;
    let mut output_buf = vec![0u8; 512 * 288 * 4]; // 512 stride × 288 (rounded 272)
    let mut frames_decoded = 0u32;

    for round in 0..100 {
        // Feed 1 packet
        let avail = unsafe {
            psp::sys::sceMpegRingbufferAvailableSize(&mut *ringbuffer)
        };
        if avail > 0 {
            vlog(&format!("[PMF-TEST] round {round}: Put(1), avail={avail}"));
            let ret = unsafe {
                psp::sys::sceMpegRingbufferPut(&mut *ringbuffer, 1, avail)
            };
            vlog(&format!("[PMF-TEST] Put returned {ret}"));
            if ret > 0 {
                total_put += ret;
            }
            if ret < 0 {
                vlog(&format!("[PMF-TEST] Put error, stopping"));
                break;
            }
        }

        // Try to get AU
        let au_ret = unsafe {
            psp::sys::sceMpegGetAvcAu(mpeg, stream, &mut au, core::ptr::null_mut())
        };
        if au_ret >= 0 {
            vlog(&format!("[PMF-TEST] GetAvcAu OK! round {round}, total_put={total_put}"));

            let mut out_ptr = output_buf.as_mut_ptr() as *mut core::ffi::c_void;
            let buf_addr = &mut out_ptr as *mut *mut core::ffi::c_void
                as *mut core::ffi::c_void;

            // Flush output buffer
            unsafe {
                psp::sys::sceKernelDcacheWritebackInvalidateRange(
                    output_buf.as_ptr() as *const core::ffi::c_void,
                    output_buf.len() as u32,
                );
            }

            let dec_ret = unsafe {
                psp::sys::sceMpegAvcDecode(
                    mpeg, &mut au, 512, buf_addr, &mut decode_init,
                )
            };
            vlog(&format!(
                "[PMF-TEST] AvcDecode = {dec_ret:#x}, init = {decode_init}"
            ));
            if dec_ret >= 0 && decode_init > 0 {
                frames_decoded += 1;
                vlog(&format!(
                    "[PMF-TEST] FRAME DECODED! ({frames_decoded} total)"
                ));
                if frames_decoded >= 3 {
                    break; // enough to prove it works
                }
            }
        }
    }

    vlog(&format!(
        "[PMF-TEST] Done: {total_put} packets fed, {frames_decoded} frames decoded"
    ));

    // Cleanup
    unsafe {
        psp::sys::sceMpegFreeAvcEsBuf(mpeg, es_buf);
        psp::sys::sceMpegUnRegistStream(mpeg, stream);
        psp::sys::sceMpegDelete(mpeg);
        let _ = Box::from_raw(mpeg_storage);
        psp::sys::sceMpegRingbufferDestruct(&mut *ringbuffer);
        psp::sys::sceIoClose(fd);
    }
    vlog("[PMF-TEST] Test complete!");
}

/// Pre-initialize the MPEG subsystem before any audio modules load.
///
/// Must be called from the main thread before spawning the audio thread.
/// This ensures `sceMpegInit` succeeds before `sceUtilityLoadModule(AvMpegBase)`
/// is called, which would otherwise put the MPEG library in a state where
/// `sceMpegInit` returns `0x8002013a` (library already exists).
pub fn preinit_mpeg() {
    crate::audio::load_av_modules_once_pub();
    // SAFETY: sceMpegInit is idempotent.
    let ret = unsafe { psp::sys::sceMpegInit() };
    vlog(&format!("[VIDEO] preinit_mpeg: sceMpegInit = {ret:#x}"));

    // Dump loaded mpeg/codec modules from memory for Ghidra analysis.
    dump_loaded_modules();

    // Extract ME firmware images from flash0.
    extract_me_firmware();
}

/// Enumerate all loaded kernel modules and dump mpeg/codec-related ones
/// to ms0 for Ghidra RE analysis.
fn dump_loaded_modules() {
    let mut mod_ids = [psp::sys::SceUid(0); 128];
    let mut count: i32 = 0;
    let ret = unsafe {
        psp::sys::sceKernelGetModuleIdList(
            mod_ids.as_mut_ptr(),
            128,
            &mut count,
        )
    };
    vlog(&format!("[DUMP] GetModuleIdList = {ret:#x}, count = {count}"));
    if ret < 0 || count <= 0 {
        return;
    }

    for i in 0..count as usize {
        let mid = mod_ids[i];
        let mut info = unsafe {
            core::mem::zeroed::<psp::sys::SceKernelModuleInfo>()
        };
        info.size = core::mem::size_of::<psp::sys::SceKernelModuleInfo>();
        let ret = unsafe { psp::sys::sceKernelQueryModuleInfo(mid, &mut info) };
        if ret < 0 {
            continue;
        }

        let name_len = info.name.iter().position(|&b| b == 0)
            .unwrap_or(info.name.len());
        let name = unsafe {
            core::str::from_utf8_unchecked(&info.name[..name_len])
        };

        // Only dump mpeg/codec modules.
        let dominated = ["mpeg", "Mpeg", "codec", "Codec", "video", "Video",
                         "avcodec", "memlmd", "libmpeg"];
        let is_target = dominated.iter().any(|t| name.contains(t));
        if !is_target {
            continue;
        }

        let text_addr = info.text_addr as usize;
        let text_size = info.text_size as usize;
        let data_size = info.data_size as usize;
        let total = text_size + data_size;

        vlog(&format!(
            "[DUMP] {name} @{text_addr:#010x} t={text_size} d={data_size}"
        ));

        if total == 0 || text_addr == 0 {
            continue;
        }

        // Build output path.
        let out_path = if name.contains("mpegbase") || name.contains("MpegBase") {
            "ms0:/PSP/GAME/OASISOS/dec_mpegbase.bin\0"
        } else if name.contains("mpeg") || name.contains("Mpeg") {
            "ms0:/PSP/GAME/OASISOS/dec_mpeg.bin\0"
        } else if name.contains("avcodec") {
            "ms0:/PSP/GAME/OASISOS/dec_avcodec.bin\0"
        } else if name.contains("memlmd") {
            "ms0:/PSP/GAME/OASISOS/dec_memlmd.bin\0"
        } else {
            "ms0:/PSP/GAME/OASISOS/dec_other.bin\0"
        };

        // SAFETY: text_addr is the loaded module's base in kernel memory.
        let slice = unsafe {
            core::slice::from_raw_parts(text_addr as *const u8, total)
        };

        // Write to file.
        unsafe {
            let fd = psp::sys::sceIoOpen(
                out_path.as_ptr(),
                psp::sys::IoOpenFlags::WR_ONLY
                    | psp::sys::IoOpenFlags::CREAT
                    | psp::sys::IoOpenFlags::TRUNC,
                0o777,
            );
            if fd >= psp::sys::SceUid(0) {
                psp::sys::sceIoWrite(
                    fd,
                    slice.as_ptr() as *const core::ffi::c_void,
                    total,
                );
                psp::sys::sceIoClose(fd);
                vlog(&format!("[DUMP] wrote {total} bytes to {out_path}"));
            } else {
                vlog(&format!("[DUMP] failed to open {out_path}"));
            }
        }
    }
}

/// Extract ME firmware images from flash0 to memory stick.
///
/// These are the encrypted firmware binaries that sceMeCodecWrapper loads
/// onto the Media Engine coprocessor during codec initialization.
/// First lists flash0 directories to find the actual paths, then copies.
fn extract_me_firmware() {
    // ARK-4 CFW intercepts flash0:/kd/ — try multiple device paths.
    // The ME firmware may be on the raw flash (lflash0:) or a different
    // partition that ARK doesn't redirect.
    list_flash0_dir(b"flash0:/kd\0", 0);
    list_flash0_dir(b"flash1:/\0", 0);
    list_flash0_dir(b"flash2:/\0", 0);
    list_flash0_dir(b"flash3:/\0", 0);

    // Try all known paths for ME firmware images.
    let files: &[(&str, &[u8], &[u8])] = &[
        // Standard flash0 paths (Sony firmware)
        ("meimg.img", b"flash0:/kd/resource/meimg.img\0", b"ms0:/PSP/GAME/OASISOS/meimg.img\0"),
        ("me_blimg.img", b"flash0:/kd/resource/me_blimg.img\0", b"ms0:/PSP/GAME/OASISOS/me_blimg.img\0"),
        ("me_sdimg.img", b"flash0:/kd/resource/me_sdimg.img\0", b"ms0:/PSP/GAME/OASISOS/me_sdimg.img\0"),
        ("me_t2img.img", b"flash0:/kd/resource/me_t2img.img\0", b"ms0:/PSP/GAME/OASISOS/me_t2img.img\0"),
        // Try without /resource/ subdirectory
        ("meimg_kd", b"flash0:/kd/meimg.img\0", b"ms0:/PSP/GAME/OASISOS/meimg.img\0"),
        // Try flash3 (some FW versions store ME images separately)
        ("meimg_f3", b"flash3:/meimg.img\0", b"ms0:/PSP/GAME/OASISOS/meimg.img\0"),
    ];

    for &(name, src, dst) in files {
        vlog(&format!("[ME-FW] extracting {name}..."));

        let src_fd = unsafe {
            psp::sys::sceIoOpen(src.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0)
        };
        if src_fd < psp::sys::SceUid(0) {
            vlog(&format!("[ME-FW] open failed: {name} (fd={:#x})", src_fd.0));
            continue;
        }

        // Get file size.
        let file_size = unsafe {
            let end = psp::sys::sceIoLseek(src_fd, 0, psp::sys::IoWhence::End);
            psp::sys::sceIoLseek(src_fd, 0, psp::sys::IoWhence::Set);
            end as usize
        };
        vlog(&format!("[ME-FW] {name}: {file_size} bytes"));

        if file_size == 0 {
            unsafe { psp::sys::sceIoClose(src_fd) };
            continue;
        }

        let dst_fd = unsafe {
            psp::sys::sceIoOpen(
                dst.as_ptr(),
                psp::sys::IoOpenFlags::WR_ONLY
                    | psp::sys::IoOpenFlags::CREAT
                    | psp::sys::IoOpenFlags::TRUNC,
                0o777,
            )
        };
        if dst_fd < psp::sys::SceUid(0) {
            unsafe { psp::sys::sceIoClose(src_fd) };
            vlog(&format!("[ME-FW] dst open failed: {name}"));
            continue;
        }

        // Copy in 16KB chunks.
        let mut buf = vec![0u8; 16384];
        let mut total: usize = 0;
        loop {
            let n = unsafe {
                psp::sys::sceIoRead(src_fd, buf.as_mut_ptr() as *mut _, buf.len() as u32)
            };
            if n <= 0 {
                break;
            }
            let w = unsafe {
                psp::sys::sceIoWrite(dst_fd, buf.as_ptr() as *const _, n as usize)
            };
            if w <= 0 {
                break;
            }
            total += w as usize;
        }

        unsafe {
            psp::sys::sceIoClose(src_fd);
            psp::sys::sceIoClose(dst_fd);
        }
        vlog(&format!("[ME-FW] wrote {name}: {total} bytes"));
    }
}

/// List contents of a flash0 directory via sceIoDopen/Dread/Dclose.
fn list_flash0_dir(path: &[u8], depth: u8) {
    let dir_fd = unsafe {
        psp::sys::sceIoDopen(path.as_ptr())
    };
    let path_str = core::str::from_utf8(&path[..path.len() - 1]).unwrap_or("?");
    if dir_fd < psp::sys::SceUid(0) {
        vlog(&format!("[ME-FW] dir open failed: {path_str} ({:#x})", dir_fd.0));
        return;
    }
    vlog(&format!("[ME-FW] listing {path_str}"));

    let mut count = 0;
    loop {
        let mut entry: psp::sys::SceIoDirent = unsafe { core::mem::zeroed() };
        let ret = unsafe { psp::sys::sceIoDread(dir_fd, &mut entry) };
        if ret <= 0 {
            break;
        }
        let name_len = entry.d_name.iter().position(|&b| b == 0)
            .unwrap_or(entry.d_name.len());
        let name = unsafe {
            core::str::from_utf8_unchecked(
                &*(&entry.d_name[..name_len] as *const [u8])
            )
        };
        let is_dir = entry.d_stat.st_attr.contains(psp::sys::IoStatAttr::IFDIR);
        let size = entry.d_stat.st_size as u64;
        let prefix = if depth == 0 { "" } else if depth == 1 { "  " } else { "    " };
        if is_dir {
            vlog(&format!("[ME-FW] {prefix}{name}/"));
        } else {
            vlog(&format!("[ME-FW] {prefix}{name} ({size})"));
        }
        count += 1;
        if count > 100 {
            vlog("[ME-FW] (truncated at 100 entries)");
            break;
        }
    }
    unsafe { psp::sys::sceIoDclose(dir_fd) };
}

/// Spawn the video decode thread (priority 24, between audio=16 and I/O=32).
pub fn spawn_video_thread() {
    if let Ok(handle) = ThreadBuilder::new(b"oasis_video\0")
        .priority(24)
        .spawn(move || {
            video_thread_fn();
            0
        })
    {
        core::mem::forget(handle);
    }
}

// ---------------------------------------------------------------------------
// PSP file reader wrapper for demux_lite
// ---------------------------------------------------------------------------

/// Adapter implementing `Read + Seek` over PSP `sceIo*` file I/O.
struct PspFileReader {
    fd: psp::sys::SceUid,
}

impl PspFileReader {
    fn open(path: &str) -> Option<Self> {
        let mut path_bytes: Vec<u8> = path.as_bytes().to_vec();
        path_bytes.push(0);
        // SAFETY: path_bytes is a null-terminated byte string.
        let fd =
            unsafe { psp::sys::sceIoOpen(path_bytes.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0) };
        if fd < psp::sys::SceUid(0) {
            return None;
        }
        Some(Self { fd })
    }
}

impl std::io::Read for PspFileReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // SAFETY: self.fd is a valid file descriptor opened above.
        // buf pointer and len are valid.
        let n =
            unsafe { psp::sys::sceIoRead(self.fd, buf.as_mut_ptr() as *mut _, buf.len() as u32) };
        if n < 0 {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "sceIoRead failed",
            ))
        } else {
            Ok(n as usize)
        }
    }
}

impl std::io::Seek for PspFileReader {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        let (offset, whence) = match pos {
            std::io::SeekFrom::Start(n) => (n as i64, psp::sys::IoWhence::Set),
            std::io::SeekFrom::End(n) => (n, psp::sys::IoWhence::End),
            std::io::SeekFrom::Current(n) => (n, psp::sys::IoWhence::Cur),
        };
        // SAFETY: self.fd is a valid file descriptor.
        let result = unsafe { psp::sys::sceIoLseek(self.fd, offset, whence) };
        if result < 0 {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "sceIoLseek failed",
            ))
        } else {
            Ok(result as u64)
        }
    }
}

impl Drop for PspFileReader {
    fn drop(&mut self) {
        if self.fd >= psp::sys::SceUid(0) {
            // SAFETY: fd is valid; close after use.
            unsafe { psp::sys::sceIoClose(self.fd) };
        }
    }
}

/// Round up to the next power of 2 (minimum 512 for PSP ME alignment).
fn next_power_of_2(v: u32) -> u32 {
    let min = 512;
    let mut n = v.max(min);
    n -= 1;
    n |= n >> 1;
    n |= n >> 2;
    n |= n >> 4;
    n |= n >> 8;
    n |= n >> 16;
    n + 1
}

// ---------------------------------------------------------------------------
// Ringbuffer callback context (static, single video thread)
// ---------------------------------------------------------------------------

/// Context for the ringbuffer callback. Since there's only one video
/// decoder thread, we use static state.
struct RingbufCtx {
    /// Pointer to PSMF packet data to copy into the ringbuffer.
    ptr: *const u8,
    /// Total bytes available.
    len: usize,
    /// Bytes consumed so far.
    offset: usize,
}

// SAFETY: Only accessed from the video thread (single-threaded context).
static mut RINGBUF_CTX: RingbufCtx = RingbufCtx {
    ptr: core::ptr::null(),
    len: 0,
    offset: 0,
};

/// Ringbuffer callback invoked by `sceMpegRingbufferPut`.
///
/// Copies PSMF packet data from `RINGBUF_CTX` into the ringbuffer's
/// data buffer. Each packet is 2048 bytes.
///
/// # Safety
///
/// Called by the PSP firmware from within `sceMpegRingbufferPut`.
/// `data` points into the ringbuffer's pre-allocated data buffer.
unsafe extern "C" fn ringbuffer_callback(
    data: *mut c_void,
    num_packets: i32,
    _param: *mut c_void,
) -> i32 {
    // Count callback invocations to distinguish header vs AU data.
    static mut CB_COUNT: u32 = 0;
    let cb_num = unsafe { CB_COUNT };
    unsafe { CB_COUNT += 1; }

    // Log callback invocation.
    {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            let msg = b"[VIDEO] ringbuf_cb entered\n";
            psp::sys::sceIoWrite(fd, msg.as_ptr() as *const _, msg.len());
            psp::sys::sceIoClose(fd);
        }
    }

    if num_packets <= 0 {
        return 0;
    }

    // SAFETY: Single-threaded access from video thread only.
    // Use raw pointer access (Rust 2024 forbids &mut to mutable statics).
    let ctx = unsafe { &raw mut RINGBUF_CTX };
    let ctx_ptr = unsafe { (*ctx).ptr };
    let ctx_len = unsafe { (*ctx).len };
    let ctx_offset = unsafe { (*ctx).offset };

    if ctx_ptr.is_null() || ctx_offset >= ctx_len {
        return 0;
    }

    let remaining = ctx_len - ctx_offset;
    let bytes_requested = num_packets as usize * PACKET_SIZE;
    let bytes_to_copy = remaining.min(bytes_requested);
    let packets_to_copy = bytes_to_copy / PACKET_SIZE;

    if packets_to_copy == 0 {
        return 0;
    }

    let src = ctx_ptr.add(ctx_offset);
    let dst = data as *mut u8;
    let copy_len = packets_to_copy * PACKET_SIZE;

    // Log pointers and hex-dump src data BEFORE copy (first AU only).
    // Log pointers, hex-dump src, and test dst write (first AU only).
    if cb_num == 1 {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            // Format hex dump manually (no alloc in callback).
            let hex = b"0123456789abcdef";
            let mut buf = [0u8; 200];

            // Log pointers.
            let prefix = b"[VIDEO] src=0x";
            buf[..prefix.len()].copy_from_slice(prefix);
            let mut p = prefix.len();
            let src_addr = src as usize;
            for shift in (0..8).rev() {
                let nibble = (src_addr >> (shift * 4)) & 0xF;
                buf[p] = hex[nibble];
                p += 1;
            }
            buf[p..p + 5].copy_from_slice(b" dst=");
            p += 5;
            buf[p..p + 2].copy_from_slice(b"0x");
            p += 2;
            let dst_addr = dst as usize;
            for shift in (0..8).rev() {
                let nibble = (dst_addr >> (shift * 4)) & 0xF;
                buf[p] = hex[nibble];
                p += 1;
            }
            buf[p] = b'\n';
            p += 1;
            psp::sys::sceIoWrite(fd, buf.as_ptr() as *const _, p);

            // Hex dump first 48 bytes of src.
            let mut buf2 = [0u8; 200];
            let prefix2 = b"[VIDEO] pkt: ";
            buf2[..prefix2.len()].copy_from_slice(prefix2);
            let mut p2 = prefix2.len();
            let dump_len = 48usize.min(copy_len);
            for i in 0..dump_len {
                let b = *src.add(i);
                buf2[p2] = hex[(b >> 4) as usize];
                buf2[p2 + 1] = hex[(b & 0xF) as usize];
                buf2[p2 + 2] = b' ';
                p2 += 3;
            }
            buf2[p2] = b'\n';
            p2 += 1;
            psp::sys::sceIoWrite(fd, buf2.as_ptr() as *const _, p2);

            // Test: write a single byte to dst to check if writable.
            let test_msg = b"[VIDEO] testing dst write...\n";
            psp::sys::sceIoWrite(fd, test_msg.as_ptr() as *const _, test_msg.len());
            *dst = 0xAA;
            let ok_msg = b"[VIDEO] dst write OK\n";
            psp::sys::sceIoWrite(fd, ok_msg.as_ptr() as *const _, ok_msg.len());

            psp::sys::sceIoClose(fd);
        }
    }

    // DEBUG: Only copy 1 packet at a time to isolate freeze.
    let actual_copy = 1usize.min(packets_to_copy);
    let actual_len = actual_copy * PACKET_SIZE;

    // Copy data into ringbuffer.
    for i in 0..actual_len {
        unsafe { *dst.add(i) = *src.add(i); }
    }
    unsafe { (*ctx).offset = ctx_offset + actual_len; }

    // Log after copy (first AU only).
    if cb_num <= 2 {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            let msg = b"[VIDEO] copy done, flushing dcache...\n";
            psp::sys::sceIoWrite(fd, msg.as_ptr() as *const _, msg.len());
            psp::sys::sceIoClose(fd);
        }
    }

    // Flush D-cache on the destination buffer.
    unsafe {
        psp::sys::sceKernelDcacheWritebackInvalidateRange(
            dst as *const c_void,
            actual_len as u32,
        );
    }

    if cb_num <= 2 {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            let msg = b"[VIDEO] returning from callback\n";
            psp::sys::sceIoWrite(fd, msg.as_ptr() as *const _, msg.len());
            psp::sys::sceIoClose(fd);
        }
    }

    actual_copy as i32
}

// ---------------------------------------------------------------------------
// SceMpeg H.264 video decoder
// ---------------------------------------------------------------------------

/// PSP hardware H.264 video decoder using sceMpeg (Media Engine).
///
/// Uses the PSMF container format to feed H.264 AUs through the sceMpeg
/// ringbuffer pipeline. The ME decodes H.264 and outputs ABGR pixels
/// directly (via `sceMpegAvcDecodeMode` with `Psm8888`).
struct SceMpegDecoder {
    /// sceMpeg handle (pointer-to-pointer).
    mpeg_storage: *mut c_void,
    /// Ringbuffer for feeding PSMF data to the ME (heap-allocated for
    /// stable address — sceMpegCreate stores an internal pointer to it).
    ringbuffer: Box<psp::sys::SceMpegRingbuffer>,
    /// Working memory for sceMpeg (64-byte aligned).
    mpeg_data: Vec<u8>,
    /// Ringbuffer packet data memory.
    ringbuf_data: Vec<u8>,
    /// Registered video stream handle.
    video_stream: psp::sys::SceMpegStream,
    /// ES buffer handle from sceMpegMallocAvcEsBuf.
    es_buf: *mut c_void,
    /// Access unit descriptor.
    au: psp::sys::SceMpegAu,
    /// PSMF muxer for wrapping H.264 AUs.
    muxer: PsmfMuxer,
    /// Output pixel buffer (512 × height × 4 ABGR).
    output_buf: Vec<u8>,
    /// Decode init flag (passed to sceMpegAvcDecode).
    decode_init: i32,
    /// Video dimensions.
    width: u32,
    height: u32,
    /// Output stride (power-of-2 >= width).
    frame_width: u32,
}

impl SceMpegDecoder {
    /// Number of ringbuffer packets. 256 packets = 512KB, enough for
    /// TV Guide H.264 AUs which are typically 10-50KB.
    const RB_PACKETS: i32 = 256;

    /// Attempt to initialize the sceMpeg H.264 hardware decoder.
    ///
    /// Returns `Err` on PPSSPP (sceMpeg not fully emulated for streaming)
    /// or if the MPEG modules are unavailable.
    fn try_init(width: u32, height: u32) -> Result<Self, String> {
        vlog("[VIDEO] SceMpegDecoder::try_init start");
        crate::audio::load_av_modules_once_pub();
        vlog("[VIDEO] AV modules loaded");

        // Step 1: Init MPEG subsystem (may already be done by preinit_mpeg).
        // SAFETY: sceMpegInit is idempotent; ignore "already init" errors.
        // Known "already initialized" codes:
        //   0x80618003 = SCE_MPEG_ERROR_ALREADY_INIT
        //   0x80618005 = firmware-specific "already init" variant (PSP-3001)
        let ret = unsafe { psp::sys::sceMpegInit() };
        if ret < 0
            && ret != 0x80618003_u32 as i32
            && ret != 0x80618005_u32 as i32
        {
            return Err(format!("sceMpegInit failed: {ret:#x}"));
        }
        vlog(&format!("[VIDEO] sceMpegInit = {ret:#x}"));

        // Step 2: Query and allocate working memory.
        let mem_size = unsafe { psp::sys::sceMpegQueryMemSize(0) };
        if mem_size <= 0 {
            return Err(format!("sceMpegQueryMemSize = {mem_size}"));
        }
        vlog(&format!("[VIDEO] mpeg mem size = {mem_size}"));

        // Allocate 64-byte aligned working memory.
        let mut mpeg_data = vec![0u8; mem_size as usize + 64];
        let mpeg_data_aligned = {
            let p = mpeg_data.as_mut_ptr();
            let off = p.align_offset(64);
            // SAFETY: offset is within the over-allocated buffer.
            unsafe { p.add(off) }
        };

        // Step 3: Query and allocate ringbuffer memory.
        let rb_size = unsafe { psp::sys::sceMpegRingbufferQueryMemSize(Self::RB_PACKETS) };
        if rb_size <= 0 {
            return Err(format!("sceMpegRingbufferQueryMemSize = {rb_size}"));
        }
        vlog(&format!("[VIDEO] ringbuf mem size = {rb_size}"));
        let mut ringbuf_data = vec![0u8; rb_size as usize];

        // Step 4: Construct ringbuffer (heap-allocated for stable address).
        // sceMpegCreate stores an internal pointer to the ringbuffer, so it
        // must not move after creation.
        // SAFETY: ringbuf_data is valid for rb_size bytes.
        let mut ringbuffer = Box::new(unsafe {
            core::mem::zeroed::<psp::sys::SceMpegRingbuffer>()
        });
        let ret = unsafe {
            psp::sys::sceMpegRingbufferConstruct(
                &mut *ringbuffer,
                Self::RB_PACKETS,
                ringbuf_data.as_mut_ptr() as *mut c_void,
                rb_size,
                Some(ringbuffer_callback),
                core::ptr::null_mut(),
            )
        };
        if ret < 0 {
            return Err(format!("sceMpegRingbufferConstruct = {ret:#x}"));
        }
        vlog("[VIDEO] ringbuffer constructed");

        // Step 5: Create MPEG handle.
        // SceMpeg is *mut *mut c_void — must point to valid heap storage.
        let mpeg_storage_box = Box::into_raw(Box::new(core::ptr::null_mut::<c_void>()));
        let mpeg: psp::sys::SceMpeg = unsafe {
            core::mem::transmute(mpeg_storage_box as *mut *mut c_void)
        };
        let mpeg_storage = mpeg_storage_box as *mut c_void;

        // Frame width must be >= video width, rounded up to next power of 2.
        // The ME uses this as the output stride for decoded frames.
        let frame_width = next_power_of_2(width);
        vlog(&format!(
            "[VIDEO] frame_width = {frame_width} (video = {width}x{height})"
        ));

        let ret = unsafe {
            psp::sys::sceMpegCreate(
                mpeg,
                mpeg_data_aligned as *mut c_void,
                mem_size as i32,
                &mut *ringbuffer,
                frame_width as i32,
                0,
                0,
            )
        };
        if ret < 0 {
            // SAFETY: Clean up allocated storage.
            unsafe {
                let _ = Box::from_raw(mpeg_storage_box);
                psp::sys::sceMpegRingbufferDestruct(&mut *ringbuffer);
            }
            return Err(format!("sceMpegCreate = {ret:#x}"));
        }
        vlog("[VIDEO] sceMpegCreate OK");

        // Step 6: Register video stream (stream_id=0 for video).
        let video_stream = unsafe { psp::sys::sceMpegRegistStream(mpeg, 0, 0) };
        vlog("[VIDEO] stream registered");

        // Step 7: Set decode mode to ABGR 8888 for direct pixel output.
        let mut mode = psp::sys::SceMpegAvcMode {
            unk0: -1,
            pixel_format: psp::sys::DisplayPixelFormat::Psm8888,
        };
        let ret = unsafe { psp::sys::sceMpegAvcDecodeMode(mpeg, &mut mode) };
        vlog(&format!("[VIDEO] sceMpegAvcDecodeMode = {ret:#x}"));

        // Step 8: Allocate ES buffer.
        let es_buf = unsafe { psp::sys::sceMpegMallocAvcEsBuf(mpeg) };
        if es_buf.is_null() {
            unsafe {
                psp::sys::sceMpegUnRegistStream(mpeg, video_stream);
                psp::sys::sceMpegDelete(mpeg);
                let _ = Box::from_raw(mpeg_storage_box);
                psp::sys::sceMpegRingbufferDestruct(&mut *ringbuffer);
            }
            return Err("sceMpegMallocAvcEsBuf returned null".to_string());
        }
        vlog("[VIDEO] ES buffer allocated");

        // Step 9: Init AU descriptor.
        let mut au = unsafe { core::mem::zeroed::<psp::sys::SceMpegAu>() };
        let ret = unsafe { psp::sys::sceMpegInitAu(mpeg, es_buf, &mut au) };
        vlog(&format!("[VIDEO] sceMpegInitAu = {ret:#x}"));

        // Step 10: Init hardware CSC (color space converter).
        let ret = unsafe { psp::sys::sceMpegBaseCscInit(frame_width as i32) };
        vlog(&format!("[VIDEO] sceMpegBaseCscInit = {ret:#x}"));

        // Allocate output pixel buffer (frame_width stride × height × 4 ABGR).
        let out_h = ((height + 15) / 16) * 16; // round up to 16-pixel boundary
        let output_buf = vec![0u8; frame_width as usize * out_h as usize * 4];

        let muxer = PsmfMuxer::new(width as u16, height as u16);

        // Validate PSMF header via sceMpegQueryStreamOffset.
        // This confirms the firmware can parse our generated header.
        let header = muxer.peek_header();
        let mut stream_offset: i32 = 0;
        let qso_ret = unsafe {
            psp::sys::sceMpegQueryStreamOffset(
                mpeg,
                header.as_ptr() as *mut c_void,
                &mut stream_offset,
            )
        };
        vlog(&format!(
            "[VIDEO] sceMpegQueryStreamOffset = {qso_ret:#x}, offset = {stream_offset}"
        ));

        let mut stream_size: i32 = 0;
        let qss_ret = unsafe {
            psp::sys::sceMpegQueryStreamSize(
                header.as_ptr() as *mut c_void,
                &mut stream_size,
            )
        };
        vlog(&format!(
            "[VIDEO] sceMpegQueryStreamSize = {qss_ret:#x}, size = {stream_size}"
        ));

        vlog(&format!(
            "[VIDEO] SceMpegDecoder ready: {width}x{height}, \
             output buf = {} bytes",
            output_buf.len()
        ));

        Ok(Self {
            mpeg_storage,
            ringbuffer,
            mpeg_data,
            ringbuf_data,
            video_stream,
            es_buf,
            au,
            muxer,
            output_buf,
            decode_init: 0,
            width,
            height,
            frame_width,
        })
    }

    /// Get the sceMpeg handle from the storage pointer.
    fn mpeg(&self) -> psp::sys::SceMpeg {
        // SAFETY: mpeg_storage was allocated by Box and cast to *mut c_void;
        // it originally points to a *mut *mut c_void (SceMpeg layout).
        unsafe { core::mem::transmute(self.mpeg_storage) }
    }

    /// Feed PSMF packet data to the ringbuffer via the callback.
    ///
    /// Sets up the static `RINGBUF_CTX` and calls `sceMpegRingbufferPut`.
    /// Returns the number of packets actually consumed.
    fn feed_packets(&mut self, data: &[u8]) -> i32 {
        let num_packets = (data.len() / PACKET_SIZE) as i32;
        if num_packets == 0 {
            return 0;
        }

        // SAFETY: Single-threaded access; set context before Put triggers callback.
        unsafe {
            RINGBUF_CTX.ptr = data.as_ptr();
            RINGBUF_CTX.len = data.len();
            RINGBUF_CTX.offset = 0;
        }

        // Check available space in ringbuffer.
        let avail = unsafe {
            psp::sys::sceMpegRingbufferAvailableSize(&mut *self.ringbuffer)
        };
        if avail <= 0 {
            // SAFETY: Clear context.
            unsafe {
                RINGBUF_CTX.ptr = core::ptr::null();
                RINGBUF_CTX.len = 0;
            }
            return 0;
        }

        let packets_to_put = num_packets.min(avail);

        // Flush D-cache on the data to ensure DMA coherency.
        // SAFETY: data pointer and length are valid.
        unsafe {
            psp::sys::sceKernelDcacheWritebackInvalidateRange(
                data.as_ptr() as *const c_void,
                data.len() as u32,
            );
        }

        // Log immediately before Put (last chance before potential crash).
        vlog(&format!(
            "[VIDEO] sceMpegRingbufferPut: put={packets_to_put}, avail={avail}"
        ));

        // SAFETY: sceMpegRingbufferPut invokes our callback which copies data.
        let ret = unsafe {
            psp::sys::sceMpegRingbufferPut(
                &mut *self.ringbuffer,
                packets_to_put,
                avail,
            )
        };

        // SAFETY: Clear context after Put returns.
        unsafe {
            RINGBUF_CTX.ptr = core::ptr::null();
            RINGBUF_CTX.len = 0;
        }

        ret
    }

    /// Decode a single H.264 access unit (Annex B format).
    ///
    /// Feeds PSMF header (first call only), then spawns a helper thread to
    /// call `sceMpegRingbufferPut` (which blocks until the ME consumes data),
    /// while the main thread calls `sceMpegGetAvcAu` + `sceMpegAvcDecode`
    /// to kick the ME into consuming.
    fn decode(&mut self, au_data: &[u8], pts_secs: f64) -> Option<DecodedFrame> {
        if au_data.is_empty() {
            return None;
        }

        static DECODE_CALL_COUNT: core::sync::atomic::AtomicU32 =
            core::sync::atomic::AtomicU32::new(0);
        let call_num = DECODE_CALL_COUNT.fetch_add(1, Ordering::Relaxed);
        let verbose = call_num < 10;

        let pts_90khz = (pts_secs * 90_000.0) as u64;
        let mpeg = self.mpeg();

        // If header hasn't been sent yet, send it first (non-blocking).
        if let Some(header_pkt) = self.muxer.take_header_packet() {
            if verbose {
                vlog("[VIDEO] feeding PSMF header...");
            }
            let put = self.feed_packets(&header_pkt);
            vlog(&format!("[VIDEO] fed PSMF header: {put} packets"));
        }

        // Wrap the AU in MPEG-PS PES packets.
        let packets = self.muxer.wrap_au(au_data, pts_90khz);
        if packets.is_empty() {
            return None;
        }

        if verbose {
            vlog(&format!(
                "[VIDEO] decode#{call_num}: au={} bytes, pts={pts_secs:.2}s, \
                 wrapped into {} packets",
                au_data.len(),
                packets.len(),
            ));
        }

        // Flatten packets into a contiguous buffer for the callback.
        let total_bytes = packets.len() * PACKET_SIZE;
        let mut flat = vec![0u8; total_bytes];
        for (i, pkt) in packets.iter().enumerate() {
            flat[i * PACKET_SIZE..(i + 1) * PACKET_SIZE].copy_from_slice(pkt);
        }

        // Set up static context for the ringbuffer callback.
        unsafe {
            let ctx = &raw mut RINGBUF_CTX;
            (*ctx).ptr = flat.as_ptr();
            (*ctx).len = flat.len();
            (*ctx).offset = 0;
        }

        let avail = unsafe {
            psp::sys::sceMpegRingbufferAvailableSize(&mut *self.ringbuffer)
        };
        if avail <= 0 {
            unsafe {
                let ctx = &raw mut RINGBUF_CTX;
                (*ctx).ptr = core::ptr::null();
                (*ctx).len = 0;
            }
            return None;
        }

        let packets_to_put = (packets.len() as i32).min(avail);

        if verbose {
            vlog(&format!(
                "[VIDEO] feeding {packets_to_put} packets (avail={avail}), \
                 spawning feeder thread..."
            ));
        }

        // Flush D-cache on the data.
        unsafe {
            psp::sys::sceKernelDcacheWritebackInvalidateRange(
                flat.as_ptr() as *const c_void,
                flat.len() as u32,
            );
        }

        // Spawn a helper thread for sceMpegRingbufferPut (it blocks).
        // We use a raw pointer to the ringbuffer since it's heap-allocated
        // and lives for the duration of the decoder.
        // Use statics to pass parameters to the feeder thread (avoids Send issues).
        static FEED_DONE: AtomicBool = AtomicBool::new(false);
        static FEED_RESULT: core::sync::atomic::AtomicI32 =
            core::sync::atomic::AtomicI32::new(0);
        static mut FEED_RB_PTR: *mut psp::sys::SceMpegRingbuffer = core::ptr::null_mut();
        static mut FEED_NUM_PACKETS: i32 = 0;
        static mut FEED_AVAIL: i32 = 0;

        // SAFETY: Set before spawning thread; thread reads after spawn.
        unsafe {
            FEED_RB_PTR = &mut *self.ringbuffer as *mut psp::sys::SceMpegRingbuffer;
            FEED_NUM_PACKETS = packets_to_put;
            FEED_AVAIL = avail;
        }
        FEED_DONE.store(false, Ordering::Release);

        let feed_handle = ThreadBuilder::new(b"mpeg_feed\0")
            .priority(20) // higher priority than video thread (24)
            .stack_size(4096)
            .spawn(move || {
                // SAFETY: Statics set by caller before thread spawn.
                let ret = unsafe {
                    psp::sys::sceMpegRingbufferPut(
                        FEED_RB_PTR,
                        FEED_NUM_PACKETS,
                        FEED_AVAIL,
                    )
                };
                FEED_RESULT.store(ret, Ordering::Release);
                FEED_DONE.store(true, Ordering::Release);
                0
            });

        if feed_handle.is_err() {
            vlog("[VIDEO] failed to spawn feeder thread");
            unsafe {
                let ctx = &raw mut RINGBUF_CTX;
                (*ctx).ptr = core::ptr::null();
                (*ctx).len = 0;
            }
            return None;
        }
        let feed_handle = feed_handle.ok();

        // Poll: repeatedly call sceMpegGetAvcAu while waiting for the
        // feeder thread to complete. GetAvcAu tells the ME to consume data,
        // which unblocks Put.
        if verbose {
            vlog("[VIDEO] polling GetAvcAu to unblock feeder...");
        }

        let mut au_ret = -1i32;
        for attempt in 0..200 {
            // 200 × 5ms = 1 second timeout
            if FEED_DONE.load(Ordering::Acquire) {
                break;
            }

            // Call GetAvcAu to kick the ME into consuming ringbuffer data.
            let ret = unsafe {
                psp::sys::sceMpegGetAvcAu(
                    mpeg,
                    self.video_stream,
                    &mut self.au,
                    core::ptr::null_mut(),
                )
            };

            if ret >= 0 {
                au_ret = ret;
                if verbose {
                    vlog(&format!(
                        "[VIDEO] sceMpegGetAvcAu OK on attempt {attempt}"
                    ));
                }
                break;
            }

            if verbose && attempt < 3 {
                vlog(&format!(
                    "[VIDEO] GetAvcAu attempt {attempt} = {:#x}",
                    ret as u32
                ));
            }

            unsafe { psp::sys::sceKernelDelayThread(5_000) }; // 5ms
        }

        // Wait for feeder thread to finish after GetAvcAu unblocked it.
        for _ in 0..100 {
            if FEED_DONE.load(Ordering::Acquire) {
                break;
            }
            unsafe { psp::sys::sceKernelDelayThread(1_000) }; // 1ms
        }

        // Clean up context.
        unsafe {
            let ctx = &raw mut RINGBUF_CTX;
            (*ctx).ptr = core::ptr::null();
            (*ctx).len = 0;
        }

        if let Some(h) = feed_handle {
            core::mem::forget(h); // Thread already exited.
        }

        if !FEED_DONE.load(Ordering::Acquire) {
            vlog("[VIDEO] feeder thread timed out");
            return None;
        }

        let put = FEED_RESULT.load(Ordering::Acquire);
        if verbose {
            vlog(&format!("[VIDEO] feeder done, put={put}"));
        }
        if put <= 0 {
            return None;
        }

        // If GetAvcAu didn't succeed during polling, try once more.
        if au_ret < 0 {
            au_ret = unsafe {
                psp::sys::sceMpegGetAvcAu(
                    mpeg,
                    self.video_stream,
                    &mut self.au,
                    core::ptr::null_mut(),
                )
            };
        }

        if au_ret < 0 {
            if verbose {
                vlog(&format!(
                    "[VIDEO] sceMpegGetAvcAu final = {:#x}",
                    au_ret as u32
                ));
            }
            return None;
        }

        if verbose {
            vlog("[VIDEO] calling sceMpegAvcDecode...");
        }

        // Decode the H.264 AU — output is ABGR pixels written to output_buf.
        // `buffer` is a pointer-to-pointer: the ME writes the output address.
        let mut output_ptr = self.output_buf.as_mut_ptr() as *mut c_void;
        let buffer_addr = &mut output_ptr as *mut *mut c_void as *mut c_void;

        // SAFETY: sceMpegAvcDecode with valid handles and aligned output buffer.
        let ret = unsafe {
            psp::sys::sceMpegAvcDecode(
                mpeg,
                &mut self.au,
                self.frame_width as i32,
                buffer_addr,
                &mut self.decode_init,
            )
        };

        if ret < 0 {
            static DEC_ERR_COUNT: core::sync::atomic::AtomicU32 =
                core::sync::atomic::AtomicU32::new(0);
            let c = DEC_ERR_COUNT.fetch_add(1, Ordering::Relaxed);
            if c < 10 {
                vlog(&format!(
                    "[VIDEO] sceMpegAvcDecode = {:#x}, init = {}",
                    ret as u32, self.decode_init
                ));
            }
            return None;
        }

        // decode_init > 0 means a frame was produced.
        if self.decode_init <= 0 {
            return None;
        }

        // The ME may have written pixels to a different address than our buffer.
        // `output_ptr` now points to the actual decoded pixel data (may be EDRAM).
        // We need to copy from that address into our owned buffer.
        let w = self.width as usize;
        let h = self.height as usize;
        let stride = self.frame_width as usize;
        let mut rgba = vec![0u8; w * h * 4];

        // SAFETY: output_ptr is set by sceMpegAvcDecode to valid decoded data.
        // Use uncached address for EDRAM coherency.
        let src = (output_ptr as usize | 0x4000_0000) as *const u8;
        for row in 0..h {
            let src_offset = row * stride * 4;
            let dst_offset = row * w * 4;
            // SAFETY: src + src_offset is valid for stride*4 bytes (decoded frame).
            unsafe {
                core::ptr::copy_nonoverlapping(
                    src.add(src_offset),
                    rgba.as_mut_ptr().add(dst_offset),
                    w * 4,
                );
            }
        }

        Some(DecodedFrame {
            rgba,
            width: self.width,
            height: self.height,
        })
    }
}

impl Drop for SceMpegDecoder {
    fn drop(&mut self) {
        let mpeg = self.mpeg();
        // SAFETY: Cleanup sceMpeg resources in reverse order.
        unsafe {
            if !self.es_buf.is_null() {
                psp::sys::sceMpegFreeAvcEsBuf(mpeg, self.es_buf);
            }
            psp::sys::sceMpegUnRegistStream(mpeg, self.video_stream);
            psp::sys::sceMpegDelete(mpeg);
            psp::sys::sceMpegRingbufferDestruct(&mut *self.ringbuffer);
            // Reclaim the Box we leaked for mpeg_storage.
            let _ = Box::from_raw(self.mpeg_storage as *mut *mut c_void);
            // Don't call sceMpegFinish — allow reuse on channel switch.
        }
        vlog("[VIDEO] SceMpegDecoder dropped");
    }
}

// ---------------------------------------------------------------------------
// H.264 SPS parsing (minimal, for extracting width/height)
// ---------------------------------------------------------------------------

/// Parse width and height from an H.264 SPS NAL unit (Annex B format).
///
/// Scans the AU for a NAL type 7 (SPS) and extracts dimensions from the
/// fixed-offset fields. Returns `(width, height)` or `None`.
fn parse_sps_dimensions(au_data: &[u8]) -> Option<(u32, u32)> {
    // Find SPS NAL: 00 00 00 01 67 or 00 00 01 67 (nal_type & 0x1F == 7)
    let mut i = 0;
    while i + 4 < au_data.len() {
        let is_start = (au_data[i] == 0 && au_data[i + 1] == 0 && au_data[i + 2] == 1)
            || (au_data[i] == 0
                && au_data[i + 1] == 0
                && au_data[i + 2] == 0
                && au_data[i + 3] == 1);
        if is_start {
            let nal_offset = if au_data[i + 2] == 1 { i + 3 } else { i + 4 };
            if nal_offset < au_data.len() {
                let nal_type = au_data[nal_offset] & 0x1F;
                if nal_type == 7 {
                    // Found SPS. For TV Guide content (Baseline/Main profile,
                    // no cropping, standard resolution), dimensions are at
                    // predictable bit offsets. Use a simplified parser.
                    return parse_sps_rbsp(&au_data[nal_offset..]);
                }
            }
        }
        i += 1;
    }
    None
}

/// Simplified SPS RBSP parser for Baseline/Main profile.
///
/// Reads exp-golomb coded fields to extract pic_width_in_mbs and
/// pic_height_in_map_units, then computes pixel dimensions.
fn parse_sps_rbsp(sps: &[u8]) -> Option<(u32, u32)> {
    if sps.len() < 5 {
        return None;
    }

    // sps[0] = nal header (already checked)
    let profile_idc = sps[1];
    // sps[2] = constraint flags
    // sps[3] = level_idc

    let mut reader = BitReader::new(&sps[4..]);

    // seq_parameter_set_id
    let _sps_id = reader.read_ue()?;

    // High profile has additional fields
    if profile_idc == 100 || profile_idc == 110 || profile_idc == 122
        || profile_idc == 244 || profile_idc == 44 || profile_idc == 83
        || profile_idc == 86 || profile_idc == 118 || profile_idc == 128
    {
        let chroma_format_idc = reader.read_ue()?;
        if chroma_format_idc == 3 {
            reader.skip(1)?; // separate_colour_plane_flag
        }
        let _bit_depth_luma = reader.read_ue()?;
        let _bit_depth_chroma = reader.read_ue()?;
        reader.skip(1)?; // qpprime_y_zero_transform_bypass_flag
        let seq_scaling_matrix_present = reader.read_bit()?;
        if seq_scaling_matrix_present == 1 {
            let count = if chroma_format_idc != 3 { 8 } else { 12 };
            for _ in 0..count {
                let present = reader.read_bit()?;
                if present == 1 {
                    // Skip scaling list
                    let size = if count < 6 { 16 } else { 64 };
                    let mut last_scale = 8i32;
                    let mut next_scale = 8i32;
                    for _ in 0..size {
                        if next_scale != 0 {
                            let delta = reader.read_se()?;
                            next_scale = (last_scale + delta + 256) % 256;
                        }
                        last_scale = if next_scale == 0 { last_scale } else { next_scale };
                    }
                }
            }
        }
    }

    // log2_max_frame_num_minus4
    let _log2_max_frame_num = reader.read_ue()?;
    // pic_order_cnt_type
    let poc_type = reader.read_ue()?;
    if poc_type == 0 {
        let _log2_max_poc_lsb = reader.read_ue()?;
    } else if poc_type == 1 {
        reader.skip(1)?; // delta_pic_order_always_zero_flag
        let _offset_for_non_ref_pic = reader.read_se()?;
        let _offset_for_top_to_bottom = reader.read_se()?;
        let num_ref_frames_in_poc = reader.read_ue()?;
        for _ in 0..num_ref_frames_in_poc {
            let _offset = reader.read_se()?;
        }
    }

    // max_num_ref_frames
    let _max_ref_frames = reader.read_ue()?;
    // gaps_in_frame_num_allowed
    reader.skip(1)?;

    // pic_width_in_mbs_minus1
    let pic_width_mbs = reader.read_ue()? + 1;
    // pic_height_in_map_units_minus1
    let pic_height_map_units = reader.read_ue()? + 1;

    let width = pic_width_mbs * 16;
    let height = pic_height_map_units * 16;

    Some((width, height))
}

/// Minimal bitstream reader for exp-golomb codes in H.264 SPS.
struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8, // 0-7, MSB first
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, byte_pos: 0, bit_pos: 0 }
    }

    fn read_bit(&mut self) -> Option<u32> {
        if self.byte_pos >= self.data.len() {
            return None;
        }
        let bit = ((self.data[self.byte_pos] >> (7 - self.bit_pos)) & 1) as u32;
        self.bit_pos += 1;
        if self.bit_pos >= 8 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }
        Some(bit)
    }

    fn skip(&mut self, n: u32) -> Option<()> {
        for _ in 0..n {
            self.read_bit()?;
        }
        Some(())
    }

    fn read_bits(&mut self, n: u32) -> Option<u32> {
        let mut val = 0u32;
        for _ in 0..n {
            val = (val << 1) | self.read_bit()?;
        }
        Some(val)
    }

    /// Read unsigned exp-golomb code.
    fn read_ue(&mut self) -> Option<u32> {
        let mut leading_zeros = 0u32;
        loop {
            let bit = self.read_bit()?;
            if bit == 1 {
                break;
            }
            leading_zeros += 1;
            if leading_zeros > 31 {
                return None; // overflow protection
            }
        }
        if leading_zeros == 0 {
            return Some(0);
        }
        let suffix = self.read_bits(leading_zeros)?;
        Some((1 << leading_zeros) - 1 + suffix)
    }

    /// Read signed exp-golomb code.
    fn read_se(&mut self) -> Option<i32> {
        let ue = self.read_ue()?;
        let sign = if ue & 1 == 1 { 1 } else { -1 };
        Some(sign * ((ue + 1) / 2) as i32)
    }
}

// ---------------------------------------------------------------------------
// Thread function
// ---------------------------------------------------------------------------

fn video_thread_fn() {
    loop {
        // Check for streaming mode request (set by I/O thread via atomic).
        if STREAM_REQUESTED.swap(false, Ordering::Acquire) {
            VIDEO_PLAYING.store(true, Ordering::Relaxed);
            if play_stream() {
                break;
            }
        }

        match VIDEO_CMD_QUEUE.pop() {
            Some(VideoCmd::Play { path, seek_secs }) => {
                VIDEO_PLAYING.store(true, Ordering::Relaxed);
                if play_mp4(&path, seek_secs) {
                    break;
                }
            },
            Some(VideoCmd::StreamStart) => {
                // Legacy path — prefer STREAM_REQUESTED atomic.
                VIDEO_PLAYING.store(true, Ordering::Relaxed);
                if play_stream() {
                    break;
                }
            },
            Some(VideoCmd::Stop) => {
                VIDEO_PLAYING.store(false, Ordering::Relaxed);
                while VIDEO_STREAM_QUEUE.pop().is_some() {}
                send_audio_cmd(AudioCmd::VideoAudioStop);
            },
            Some(VideoCmd::Shutdown) => {
                VIDEO_PLAYING.store(false, Ordering::Relaxed);
                break;
            },
            None => {
                // SAFETY: sceKernelDelayThread sleeps the current thread.
                unsafe { psp::sys::sceKernelDelayThread(10_000) };
            },
        }
    }
}

/// Demux an MP4 file, decode H.264 video via ME, and feed audio to the
/// audio thread.
fn play_mp4(path: &str, seek_secs: u64) -> bool {
    use oasis_video::demux_lite::Mp4Lite;

    let reader = match PspFileReader::open(path) {
        Some(r) => r,
        None => {
            psp::dprintln!("video: failed to open {path}");
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            return false;
        },
    };

    let mut mp4 = match Mp4Lite::open(reader) {
        Ok(m) => m,
        Err(e) => {
            psp::dprintln!("video: failed to parse MP4 {path}: {e}");
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            return false;
        },
    };

    // Seek if requested.
    if seek_secs > 0 {
        if let Err(e) = mp4.seek(seek_secs as f64) {
            psp::dprintln!("video: seek to {seek_secs}s failed: {e}");
        }
    }

    // Get video dimensions from the first video sample's SPS.
    // Default to 480x272 if SPS parsing fails.
    let (vid_w, vid_h) = (480u32, 272u32);

    // Attempt H.264 hardware decoder init.
    vlog("[VIDEO] play_mp4: attempting sceMpeg init...");
    let mut h264 = match SceMpegDecoder::try_init(vid_w, vid_h) {
        Ok(dec) => {
            vlog("[VIDEO] play_mp4: sceMpeg decoder initialized");
            Some(dec)
        },
        Err(e) => {
            vlog(&format!(
                "[VIDEO] play_mp4: sceMpeg disabled ({e}), audio-only"
            ));
            None
        },
    };

    psp::dprintln!(
        "video: MP4 opened, video={}, audio={}",
        mp4.video_track_info().is_some(),
        mp4.audio_track_info().is_some(),
    );

    let mut video_count = 0u32;
    let mut audio_count = 0u32;
    let mut decode_count = 0u32;
    let mut audio_done = mp4.audio_track_info().is_none();
    let mut video_done = mp4.video_track_info().is_none();

    // Track playback start time for frame pacing.
    let start_us = unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;

    loop {
        // Check for stop command.
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
            match cmd {
                VideoCmd::Stop | VideoCmd::Shutdown => {
                    VIDEO_PLAYING.store(false, Ordering::Relaxed);
                    send_audio_cmd(AudioCmd::VideoAudioStop);
                    if matches!(cmd, VideoCmd::Shutdown) {
                        return true;
                    }
                    break;
                },
                VideoCmd::Play { .. } | VideoCmd::StreamStart => {
                    // Ignore nested Play/Stream commands during playback.
                },
            }
        }

        if !VIDEO_PLAYING.load(Ordering::Relaxed) {
            break;
        }

        // Read audio samples and forward raw AAC to the audio thread.
        if !audio_done {
            match mp4.next_audio_sample() {
                Ok(Some(sample)) => {
                    audio_count += 1;
                    send_audio_cmd(AudioCmd::VideoAudioAac { data: sample.data });
                },
                Ok(None) => {
                    audio_done = true;
                },
                Err(oasis_video::demux_lite::LiteError::NoTrack(_)) => {
                    audio_done = true;
                },
                Err(e) => {
                    psp::dprintln!("video: audio read error: {e}");
                    audio_done = true;
                },
            }
        }

        // Read and decode video samples.
        if !video_done {
            match mp4.next_video_sample() {
                Ok(Some(sample)) => {
                    video_count += 1;

                    // Decode H.264 via sceMpeg if available.
                    if let Some(ref mut decoder) = h264 {
                        if let Some(frame) = decoder.decode(
                            &sample.data,
                            sample.timestamp_secs,
                        ) {
                            decode_count += 1;

                            // Frame pacing: wait until the frame's PTS.
                            let pts_us = (sample.timestamp_secs * 1_000_000.0) as u64;
                            let now_us =
                                unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;
                            let elapsed = now_us.wrapping_sub(start_us);
                            if pts_us > elapsed {
                                let wait = (pts_us - elapsed) as u32;
                                if wait < 100_000 {
                                    // SAFETY: Sleep for frame pacing.
                                    unsafe {
                                        psp::sys::sceKernelDelayThread(wait);
                                    }
                                }
                            }

                            let _ = VIDEO_FRAME_QUEUE.push(frame);
                        }
                    }
                },
                Ok(None) => {
                    video_done = true;
                },
                Err(oasis_video::demux_lite::LiteError::NoTrack(_)) => {
                    video_done = true;
                },
                Err(e) => {
                    psp::dprintln!("video: video read error: {e}");
                    video_done = true;
                },
            }
        }

        if audio_done && video_done {
            break;
        }
    }

    // Cleanup on all exit paths.
    psp::dprintln!(
        "video: stream ended -- {} video samples, {} decoded frames, \
         {} audio samples",
        video_count,
        decode_count,
        audio_count,
    );
    VIDEO_PLAYING.store(false, Ordering::Relaxed);
    send_audio_cmd(AudioCmd::VideoAudioStop);
    false
}

/// Streaming playback: receive pre-demuxed H.264 frames from I/O thread
/// and decode them via sceMpeg.
///
/// Returns `true` if Shutdown was received (caller should exit thread).
fn play_stream() -> bool {
    vlog("[VIDEO] play_stream: starting streaming decode");

    // Drain stale commands that may have been queued during moov buffering.
    while let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
        if matches!(cmd, VideoCmd::Shutdown) {
            VIDEO_PLAYING.store(false, Ordering::Relaxed);
            vlog("[VIDEO] play_stream: shutdown during drain");
            return true;
        }
        vlog("[VIDEO] play_stream: drained stale command");
    }

    // We need the first keyframe to extract SPS and get video dimensions
    // before initializing the decoder. Wait for it.
    vlog("[VIDEO] play_stream: waiting for first keyframe...");
    let mut first_frame: Option<StreamFrame> = None;

    for _ in 0..500 {
        // ~5 seconds timeout (500 × 10ms)
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
            if matches!(cmd, VideoCmd::Stop | VideoCmd::Shutdown) {
                VIDEO_PLAYING.store(false, Ordering::Relaxed);
                send_audio_cmd(AudioCmd::VideoAudioStop);
                return matches!(cmd, VideoCmd::Shutdown);
            }
        }

        if let Some(frame) = VIDEO_STREAM_QUEUE.pop() {
            if frame.is_keyframe {
                first_frame = Some(frame);
                break;
            }
            // Skip non-keyframes before decoder init.
        }

        // SAFETY: Sleep while waiting for data.
        unsafe { psp::sys::sceKernelDelayThread(10_000) };
    }

    let first_frame = match first_frame {
        Some(f) => f,
        None => {
            vlog("[VIDEO] play_stream: no keyframe received, audio-only");
            // Continue draining stream queue but don't decode.
            return drain_stream_only();
        },
    };

    // Parse SPS from the first keyframe to get video dimensions.
    let (vid_w, vid_h) = parse_sps_dimensions(&first_frame.data)
        .unwrap_or((480, 272));
    vlog(&format!(
        "[VIDEO] play_stream: SPS dimensions = {vid_w}x{vid_h}"
    ));

    // Initialize sceMpeg decoder.
    let mut h264 = match SceMpegDecoder::try_init(vid_w, vid_h) {
        Ok(dec) => {
            vlog("[VIDEO] play_stream: sceMpeg decoder initialized");
            dec
        },
        Err(e) => {
            vlog(&format!(
                "[VIDEO] play_stream: sceMpeg disabled ({e}), audio-only"
            ));
            return drain_stream_only();
        },
    };

    // Decode the first keyframe.
    let start_us = unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;
    let mut decode_count = 0u32;

    if let Some(decoded) = h264.decode(&first_frame.data, first_frame.timestamp_secs) {
        decode_count += 1;
        let _ = VIDEO_FRAME_QUEUE.push(decoded);
        vlog("[VIDEO] play_stream: first frame decoded!");
    }

    loop {
        // Check for stop/shutdown commands.
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
            match cmd {
                VideoCmd::Stop | VideoCmd::Shutdown => {
                    VIDEO_PLAYING.store(false, Ordering::Relaxed);
                    while VIDEO_STREAM_QUEUE.pop().is_some() {}
                    send_audio_cmd(AudioCmd::VideoAudioStop);
                    if matches!(cmd, VideoCmd::Shutdown) {
                        return true;
                    }
                    vlog(&format!(
                        "[VIDEO] play_stream stopped, {decode_count} frames decoded"
                    ));
                    return false;
                },
                _ => {},
            }
        }

        if !VIDEO_PLAYING.load(Ordering::Relaxed) {
            break;
        }

        // Pop next pre-demuxed H.264 frame from stream queue.
        match VIDEO_STREAM_QUEUE.pop() {
            Some(frame) => {
                if let Some(decoded) = h264.decode(&frame.data, frame.timestamp_secs) {
                    decode_count += 1;

                    // Frame pacing via PTS.
                    let pts_us = (frame.timestamp_secs * 1_000_000.0) as u64;
                    let now_us =
                        unsafe { psp::sys::sceKernelGetSystemTimeWide() } as u64;
                    let elapsed = now_us.wrapping_sub(start_us);
                    if pts_us > elapsed {
                        let wait = (pts_us - elapsed) as u32;
                        if wait < 100_000 {
                            // SAFETY: Sleep for frame pacing.
                            unsafe {
                                psp::sys::sceKernelDelayThread(wait);
                            }
                        }
                    }

                    let _ = VIDEO_FRAME_QUEUE.push(decoded);
                }
            },
            None => {
                // No frame available yet, sleep briefly.
                // SAFETY: sceKernelDelayThread sleeps the current thread.
                unsafe { psp::sys::sceKernelDelayThread(5_000) };
            },
        }
    }

    vlog(&format!(
        "[VIDEO] play_stream ended, {decode_count} frames decoded"
    ));
    VIDEO_PLAYING.store(false, Ordering::Relaxed);
    send_audio_cmd(AudioCmd::VideoAudioStop);
    false
}

/// Drain the stream queue without decoding (audio-only fallback).
/// Keeps the thread alive to handle Stop/Shutdown commands.
fn drain_stream_only() -> bool {
    loop {
        if let Some(cmd) = VIDEO_CMD_QUEUE.pop() {
            match cmd {
                VideoCmd::Stop | VideoCmd::Shutdown => {
                    VIDEO_PLAYING.store(false, Ordering::Relaxed);
                    while VIDEO_STREAM_QUEUE.pop().is_some() {}
                    send_audio_cmd(AudioCmd::VideoAudioStop);
                    return matches!(cmd, VideoCmd::Shutdown);
                },
                _ => {},
            }
        }

        if !VIDEO_PLAYING.load(Ordering::Relaxed) {
            return false;
        }

        // Drain frames to prevent queue backup.
        while VIDEO_STREAM_QUEUE.pop().is_some() {}

        // SAFETY: sceKernelDelayThread sleeps the current thread.
        unsafe { psp::sys::sceKernelDelayThread(50_000) };
    }
}
