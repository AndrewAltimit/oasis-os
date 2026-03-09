//! Internet radio streaming via TCP + ICY demuxing + sceAudiocodec.

use core::sync::atomic::Ordering;

use super::nids::*;
use super::resolve::*;
use super::state::*;
use super::{copy_bytes, find_mp3_sync, write_u32_decimal};

/// Stream internet radio via TCP + ICY demuxing + sceAudiocodec.
///
/// Connects to the given station, parses ICY headers, and streams
/// MP3 audio through the codec decoder. Structurally mirrors
/// `play_track_codec()` but replaces file I/O with TCP recv + ICY
/// demuxing.
///
/// # Safety
/// Caller must ensure codec user memory is allocated and network is
/// initialized.
pub(super) unsafe fn play_radio_stream(station_idx: u8, channel: i32) -> i32 {
    if station_idx as usize >= RADIO_STATIONS.len() {
        return -1;
    }
    let station = &RADIO_STATIONS[station_idx as usize];

    // Set track name to station name.
    // SAFETY: Writing to TRACK_NAME static buffer; called from audio thread only.
    unsafe {
        let len = station.name.len().min(47);
        let mut j = 0;
        while j < len {
            (*(&raw mut TRACK_NAME))[j] = station.name[j];
            j += 1;
        }
        while j < 48 {
            (*(&raw mut TRACK_NAME))[j] = 0;
            j += 1;
        }
    }

    // DNS resolve.
    // SAFETY: resolve_hostname_raw calls resolved sceNetResolver fn pointers.
    let ip = match unsafe { super::network::resolve_hostname_raw(station.host.as_ptr()) } {
        Some(ip) => ip,
        None => {
            crate::debug_log(b"[OASIS] radio: DNS failed");
            return -1;
        },
    };
    {
        let mut buf = [0u8; 48];
        let mut p = copy_bytes(&mut buf, 0, b"[OASIS] radio IP: ");
        p = write_u32_decimal(&mut buf, p, ip[0] as u32);
        p = copy_bytes(&mut buf, p, b".");
        p = write_u32_decimal(&mut buf, p, ip[1] as u32);
        p = copy_bytes(&mut buf, p, b".");
        p = write_u32_decimal(&mut buf, p, ip[2] as u32);
        p = copy_bytes(&mut buf, p, b".");
        p = write_u32_decimal(&mut buf, p, ip[3] as u32);
        crate::debug_log(&buf[..p]);
    }

    // TCP socket + connect.
    // SAFETY: Volatile reads of resolved sceNetInet fn pointers (socket, connect, recv, close).
    let socket_fn = unsafe {
        match core::ptr::read_volatile(&raw const INET_SOCKET_FN) {
            Some(f) => f,
            None => return -1,
        }
    };
    // SAFETY: Volatile read of resolved sceNetInetConnect fn pointer.
    let connect_fn = unsafe {
        match core::ptr::read_volatile(&raw const INET_CONNECT_FN) {
            Some(f) => f,
            None => return -1,
        }
    };
    // SAFETY: Volatile read of resolved sceNetInetRecv fn pointer.
    let recv_fn = unsafe {
        match core::ptr::read_volatile(&raw const INET_RECV_FN) {
            Some(f) => f,
            None => return -1,
        }
    };
    // SAFETY: Volatile read of resolved sceNetInetClose fn pointer.
    let close_fn = unsafe { core::ptr::read_volatile(&raw const INET_CLOSE_FN) };

    // AF_INET=2, SOCK_STREAM=1
    // SAFETY: Calling resolved sceNetInetSocket to create a TCP socket.
    let sock = unsafe { socket_fn(2, 1, 0) };
    if sock < 0 {
        crate::debug_log(b"[OASIS] radio: socket failed");
        return -1;
    }

    let sa = super::network::make_sockaddr_in(ip, station.port);
    // SAFETY: Calling resolved sceNetInetConnect with valid socket and sockaddr.
    let ret = unsafe { connect_fn(sock, sa.as_ptr(), 16) };
    if ret < 0 {
        crate::debug_log(b"[OASIS] radio: connect failed");
        if let Some(f) = close_fn {
            // SAFETY: Closing socket on error path.
            unsafe { f(sock) };
        }
        return -1;
    }

    // Build HTTP GET request with ICY metadata header.
    // SAFETY: Accessing HTTP_BUF static; called from single audio thread only.
    let http_len = unsafe {
        let buf = &raw mut HTTP_BUF;
        let b = &mut *buf;
        let plen = station.path.len().saturating_sub(1);
        let hlen = station.host.len().saturating_sub(1);
        let mut p = copy_bytes(b, 0, b"GET ");
        p = copy_bytes(b, p, &station.path[..plen]);
        p = copy_bytes(b, p, b" HTTP/1.0\r\n");
        p = copy_bytes(b, p, b"Host: ");
        p = copy_bytes(b, p, &station.host[..hlen]);
        p = copy_bytes(b, p, b"\r\n");
        p = copy_bytes(b, p, b"Icy-MetaData: 1\r\n");
        p = copy_bytes(b, p, b"User-Agent: OASIS/1.0\r\n");
        p = copy_bytes(b, p, b"\r\n");
        p
    };

    // SAFETY: send_all sends data over valid socket; HTTP_BUF accessed from audio thread.
    if !unsafe { super::network::send_all(sock, &(&(*(&raw const HTTP_BUF)))[..http_len]) } {
        crate::debug_log(b"[OASIS] radio: send request failed");
        if let Some(f) = close_fn {
            // SAFETY: Closing socket on error path.
            unsafe { f(sock) };
        }
        return -1;
    }

    // Read response headers.
    let mut header_len: usize = 0;
    let mut header_end: usize = 0;
    // SAFETY: Calling resolved recv fn with valid socket; writing into RECV_BUF
    // static accessed from audio thread only. Scanning for \r\n\r\n header end.
    unsafe {
        let recv_buf = &raw mut RECV_BUF;
        loop {
            if header_len >= 4096 {
                break;
            }
            let n = recv_fn(
                sock,
                (*recv_buf).as_mut_ptr().add(header_len),
                4096 - header_len,
                0,
            );
            if n <= 0 {
                break;
            }
            header_len += n as usize;
            // Look for \r\n\r\n.
            if header_len >= 4 {
                let mut k = 0;
                while k + 3 < header_len {
                    if (*recv_buf)[k] == b'\r'
                        && (*recv_buf)[k + 1] == b'\n'
                        && (*recv_buf)[k + 2] == b'\r'
                        && (*recv_buf)[k + 3] == b'\n'
                    {
                        header_end = k + 4;
                        break;
                    }
                    k += 1;
                }
            }
            if header_end > 0 {
                break;
            }
        }
    }

    if header_end == 0 {
        crate::debug_log(b"[OASIS] radio: no header end");
        if let Some(f) = close_fn {
            // SAFETY: Closing socket on error path.
            unsafe { f(sock) };
        }
        return -1;
    }

    // Parse icy-metaint from headers.
    // SAFETY: Accessing RECV_BUF static with header_end bytes of valid header data.
    let metaint =
        unsafe { super::parse_icy_metaint_raw(&(&(*(&raw const RECV_BUF)))[..header_end]) };
    let metaint = metaint.unwrap_or(0);
    {
        let mut buf = [0u8; 48];
        let mut p = copy_bytes(&mut buf, 0, b"[OASIS] icy-metaint=");
        p = write_u32_decimal(&mut buf, p, metaint as u32);
        crate::debug_log(&buf[..p]);
    }

    // Initialize codec (reuse user-memory buffers).
    // SAFETY: Volatile reads of UMEM_* pointers; set during init, read-only after.
    let codec = unsafe { core::ptr::read_volatile(&raw const UMEM_CODEC) };
    let pcm_buf = unsafe { core::ptr::read_volatile(&raw const UMEM_PCM) };
    let read_buf = unsafe { core::ptr::read_volatile(&raw const UMEM_READ) };
    if codec.is_null() || pcm_buf.is_null() || read_buf.is_null() {
        crate::debug_log(b"[OASIS] radio: codec bufs null");
        if let Some(f) = close_fn {
            // SAFETY: Closing socket on error path.
            unsafe { f(sock) };
        }
        return -1;
    }

    // SAFETY: Zeroing codec buffer via pointer arithmetic; codec points to
    // CODEC_BUF_WORDS words of allocated user memory.
    unsafe {
        let mut i = 0;
        while i < CODEC_BUF_WORDS {
            *codec.add(i) = 0;
            i += 1;
        }
    }

    #[allow(unused_assignments)]
    let mut edram_allocated = false;
    // SAFETY: Volatile reads of resolved sceAudiocodec fn pointers
    // (CheckNeedMem, GetEDRAM, Init); calling with valid codec buffer.
    // Closing socket on error paths.
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const CODEC_CHECK_NEED_MEM_FN) {
            f(codec, CODEC_TYPE_MP3);
        }
        if let Some(f) = core::ptr::read_volatile(&raw const CODEC_GET_EDRAM_FN) {
            let ret = f(codec, CODEC_TYPE_MP3);
            if ret >= 0 {
                edram_allocated = true;
            } else {
                if let Some(cl) = close_fn {
                    cl(sock);
                }
                return -1;
            }
        } else {
            if let Some(cl) = close_fn {
                cl(sock);
            }
            return -1;
        }
        if let Some(f) = core::ptr::read_volatile(&raw const CODEC_INIT_FN) {
            let ret = f(codec, CODEC_TYPE_MP3);
            if ret < 0 {
                if edram_allocated {
                    if let Some(rel) = core::ptr::read_volatile(&raw const CODEC_RELEASE_EDRAM_FN) {
                        rel(codec);
                    }
                }
                if let Some(cl) = close_fn {
                    cl(sock);
                }
                return -1;
            }
        }
    }

    // SAFETY: Volatile read of resolved sceAudiocodecDecode fn pointer;
    // releasing EDRAM and closing socket on error path.
    let decode_fn = unsafe {
        match core::ptr::read_volatile(&raw const CODEC_DECODE_FN) {
            Some(f) => f,
            None => {
                if edram_allocated {
                    if let Some(rel) = core::ptr::read_volatile(&raw const CODEC_RELEASE_EDRAM_FN) {
                        rel(codec);
                    }
                }
                if let Some(cl) = close_fn {
                    cl(sock);
                }
                return -1;
            },
        }
    };

    // Initialize ICY demuxer.
    let mut demuxer = if metaint > 0 {
        Some(IcyDemuxer::new(metaint))
    } else {
        None
    };

    // Seed read_buf with any leftover data after headers.
    let mut buf_valid: usize = 0;
    let mut buf_pos: usize = 0;
    // SAFETY: Copying leftover header data from RECV_BUF into read_buf via
    // ICY demuxer or raw byte copy. Both buffers are allocated user memory.
    unsafe {
        let leftover = header_len - header_end;
        if leftover > 0 {
            let recv_ptr = (*(&raw const RECV_BUF)).as_ptr();
            if let Some(ref mut d) = demuxer {
                let out = core::slice::from_raw_parts_mut(read_buf, READ_BUF_SIZE);
                let (written, _) = d.process(
                    core::slice::from_raw_parts(recv_ptr.add(header_end), leftover),
                    out,
                );
                buf_valid = written;
            } else {
                let copy = leftover.min(READ_BUF_SIZE);
                let mut j = 0;
                while j < copy {
                    *read_buf.add(j) = *recv_ptr.add(header_end + j);
                    j += 1;
                }
                buf_valid = copy;
            }
        }
    }

    let mut result = 0i32;
    let mut frame_count: u32 = 0;
    let mut zero_consumed: u32 = 0;

    loop {
        // Check commands.
        let cmd = AUDIO_CMD.load(Ordering::Relaxed);
        if cmd >= 2 {
            // 2=next, 3=prev, 4=toggle, 5=next_sta, 6=prev_sta
            break;
        }
        if cmd == 1 {
            AUDIO_CMD.store(0, Ordering::Relaxed);
            let state = AUDIO_STATE.load(Ordering::Relaxed);
            if state == 1 {
                AUDIO_STATE.store(2, Ordering::Relaxed);
                crate::overlay::show_osd(b"Radio paused");
            } else {
                AUDIO_STATE.store(1, Ordering::Relaxed);
                crate::overlay::show_osd(b"Radio playing");
            }
        }
        if AUDIO_STATE.load(Ordering::Relaxed) != 1 {
            // SAFETY: PSP kernel syscall to sleep thread while paused.
            unsafe { psp::sys::sceKernelDelayThread(50_000) };
            continue;
        }

        // Compact buffer when half consumed.
        if buf_pos > READ_BUF_SIZE / 2 {
            let remaining = buf_valid - buf_pos;
            if remaining > 0 {
                // SAFETY: Manual byte copy within allocated read_buf;
                // source and dest ranges are within bounds.
                unsafe {
                    let mut i = 0;
                    while i < remaining {
                        *read_buf.add(i) = *read_buf.add(buf_pos + i);
                        i += 1;
                    }
                }
            }
            buf_valid = remaining;
            buf_pos = 0;
        }

        // Fill buffer from network.
        while buf_valid < READ_BUF_SIZE - 4096 {
            // SAFETY: Calling resolved recv fn with valid socket into RECV_BUF static.
            let n = unsafe { recv_fn(sock, (*(&raw mut RECV_BUF)).as_mut_ptr(), 4096, 0) };
            if n <= 0 {
                if n == 0 {
                    crate::debug_log(b"[OASIS] radio: connection closed");
                }
                break;
            }
            // SAFETY: Creating slice from RECV_BUF with n bytes of valid recv data.
            let recv_data = unsafe {
                core::slice::from_raw_parts((*(&raw const RECV_BUF)).as_ptr(), n as usize)
            };
            if let Some(ref mut d) = demuxer {
                // SAFETY: Creating mutable slice from read_buf at offset buf_valid;
                // remaining capacity is READ_BUF_SIZE - buf_valid.
                let out = unsafe {
                    core::slice::from_raw_parts_mut(
                        read_buf.add(buf_valid),
                        READ_BUF_SIZE - buf_valid,
                    )
                };
                let (written, _) = d.process(recv_data, out);
                buf_valid += written;
            } else {
                let copy = (n as usize).min(READ_BUF_SIZE - buf_valid);
                // SAFETY: Copying recv_data bytes into read_buf at buf_valid offset;
                // copy is bounded by remaining buffer capacity.
                unsafe {
                    let mut j = 0;
                    while j < copy {
                        *read_buf.add(buf_valid + j) = recv_data[j];
                        j += 1;
                    }
                }
                buf_valid += copy;
            }
            // Don't block too long filling -- decode some frames.
            if buf_valid >= 8192 {
                break;
            }
        }

        if buf_valid.saturating_sub(buf_pos) < 4 {
            if buf_valid == 0 {
                result = -1;
                break;
            }
            // SAFETY: PSP kernel syscall to sleep thread while waiting for data.
            unsafe { psp::sys::sceKernelDelayThread(10_000) };
            continue;
        }

        // Find MP3 sync and decode.
        // SAFETY: read_buf points to buf_valid bytes of valid data.
        let slice = unsafe { core::slice::from_raw_parts(read_buf, buf_valid) };
        let sync_pos = match find_mp3_sync(slice, buf_pos) {
            Some(pos) => pos,
            None => {
                buf_pos = buf_valid.saturating_sub(1);
                continue;
            },
        };
        buf_pos = sync_pos;
        if buf_valid - buf_pos < 8 {
            continue;
        }

        let avail = buf_valid - buf_pos;
        // SAFETY: Setting codec buffer fields via pointer arithmetic;
        // codec points to CODEC_BUF_WORDS words of allocated user memory.
        unsafe {
            *codec.add(6) = read_buf.add(buf_pos) as u32;
            *codec.add(7) = avail as u32;
            *codec.add(8) = pcm_buf as u32;
            *codec.add(9) = (1152 * 4) as u32;
            *codec.add(10) = avail as u32;
        }

        // SAFETY: Calling resolved sceAudiocodecDecode with valid codec buffer.
        let ret = unsafe { decode_fn(codec, CODEC_TYPE_MP3) };
        if ret < 0 {
            frame_count += 1;
            if frame_count > 100 {
                crate::debug_log(b"[OASIS] radio: too many decode errors");
                break;
            }
            buf_pos += 1;
            continue;
        }

        // SAFETY: Reading consumed byte count from codec buffer field 7.
        let consumed = unsafe { *codec.add(7) } as usize;
        if consumed == 0 {
            zero_consumed += 1;
            if zero_consumed > 100 {
                break;
            }
            buf_pos += 1;
            continue;
        }
        zero_consumed = 0;
        buf_pos += consumed;

        // Output decoded audio.
        let vol = (AUDIO_VOLUME.load(Ordering::Relaxed) as i32 * 0x8000) / 255;
        // SAFETY: Volatile read of USE_SRC_OUTPUT flag; set during init.
        let use_src = unsafe { core::ptr::read_volatile(&raw const USE_SRC_OUTPUT) };
        // SAFETY: Volatile reads of resolved audio output fn pointers;
        // calling with valid channel, volume, and decoded PCM buffer.
        unsafe {
            if use_src {
                if let Some(f) = core::ptr::read_volatile(&raw const AUDIO_SRC_OUTPUT_FN) {
                    let ret = f(vol, pcm_buf as *const u8);
                    if ret < 0 {
                        result = ret;
                        break;
                    }
                }
            } else {
                if let Some(f) = core::ptr::read_volatile(&raw const AUDIO_SET_CH_VOL_FN) {
                    f(channel, vol, vol);
                }
                if let Some(f) = core::ptr::read_volatile(&raw const AUDIO_OUTPUT_BLOCKING_FN) {
                    let ret = f(channel, vol, pcm_buf as *const u8);
                    if ret < 0 {
                        result = ret;
                        break;
                    }
                }
            }
        }
        frame_count += 1;
    }

    // Cleanup.
    // SAFETY: Releasing EDRAM allocation and closing socket on cleanup.
    unsafe {
        if edram_allocated {
            if let Some(f) = core::ptr::read_volatile(&raw const CODEC_RELEASE_EDRAM_FN) {
                f(codec);
            }
        }
        if let Some(f) = close_fn {
            f(sock);
        }
    }
    result
}
