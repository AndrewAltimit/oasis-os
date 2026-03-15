//! Background MP3 playback via runtime NID resolution.
//!
//! User-mode imports cause PRX load failure, so we use
//! `psp::hook::find_function()` (wrapping `sctrlHENFindFunction`) to resolve
//! all audio NIDs at runtime.
//!
//! ## Strategy
//!
//! 1. Resolve `sceUtilityLoadModule` from `sceUtility_Driver` -- this is the
//!    official PSP API for loading optional system modules and it properly
//!    registers them so `find_function` can discover their exports.
//! 2. Use it to load `PSP_MODULE_AV_AVCODEC` (0x0300) and `PSP_MODULE_AV_MP3`
//!    (0x0302).
//! 3. Resolve sceMp3 NIDs (preferred -- higher-level streaming API).
//! 4. If sceMp3 fails, try sceAudiocodec NIDs (lower-level codec API).
//! 5. If a named module search fails, retry with NULL module name (searches
//!    all loaded modules on PRO/ME/ARK CFW).

mod decode;
mod network;
mod nids;
mod radio;
mod resolve;
mod state;

use core::sync::atomic::Ordering;

use crate::overlay;

use state::*;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn current_track_name() -> &'static [u8] {
    // SAFETY: TRACK_NAME is a valid 48-byte static buffer.
    unsafe { core::slice::from_raw_parts((&raw const TRACK_NAME).cast::<u8>(), 48) }
}

pub fn audio_state() -> u8 {
    AUDIO_STATE.load(Ordering::Relaxed)
}

pub fn toggle_playback() {
    if AUDIO_AVAILABLE.load(Ordering::Relaxed) == 0 {
        overlay::show_osd(b"Audio: not available");
        return;
    }
    AUDIO_CMD.store(1, Ordering::Relaxed);
}

pub fn next_track() {
    if AUDIO_AVAILABLE.load(Ordering::Relaxed) == 0 {
        overlay::show_osd(b"Audio: not available");
        return;
    }
    AUDIO_CMD.store(2, Ordering::Relaxed);
}

pub fn prev_track() {
    if AUDIO_AVAILABLE.load(Ordering::Relaxed) == 0 {
        overlay::show_osd(b"Audio: not available");
        return;
    }
    AUDIO_CMD.store(3, Ordering::Relaxed);
}

pub fn volume_up() {
    let cur = AUDIO_VOLUME.load(Ordering::Relaxed);
    let new = cur.saturating_add(16);
    AUDIO_VOLUME.store(new, Ordering::Relaxed);
    let mut buf = [0u8; 24];
    let mut p = copy_bytes(&mut buf, 0, b"Vol: ");
    p = write_u32_decimal(&mut buf, p, new as u32);
    overlay::show_osd(&buf[..p]);
}

pub fn volume_down() {
    let cur = AUDIO_VOLUME.load(Ordering::Relaxed);
    let new = cur.saturating_sub(16);
    AUDIO_VOLUME.store(new, Ordering::Relaxed);
    let mut buf = [0u8; 24];
    let mut p = copy_bytes(&mut buf, 0, b"Vol: ");
    p = write_u32_decimal(&mut buf, p, new as u32);
    overlay::show_osd(&buf[..p]);
}

pub fn toggle_radio() {
    if AUDIO_AVAILABLE.load(Ordering::Relaxed) == 0 {
        overlay::show_osd(b"Audio: not available");
        return;
    }
    AUDIO_CMD.store(4, Ordering::Relaxed);
}

pub fn next_station() {
    AUDIO_CMD.store(5, Ordering::Relaxed);
}

pub fn is_radio_active() -> bool {
    RADIO_ACTIVE.load(Ordering::Relaxed)
}

pub fn radio_station_name() -> &'static [u8] {
    let idx = RADIO_STATION_IDX.load(Ordering::Relaxed) as usize;
    if idx < RADIO_STATIONS.len() {
        RADIO_STATIONS[idx].name
    } else {
        b"Unknown"
    }
}

pub fn radio_meta() -> &'static [u8] {
    // SAFETY: RADIO_META is a valid 48-byte static buffer.
    unsafe { core::slice::from_raw_parts((&raw const RADIO_META).cast::<u8>(), 48) }
}

/// Start playing a companion MP3 for PIP video.
/// Interrupts current playback and switches to the video's audio track.
/// When the MP3 finishes, it loops until `stop_video_mp3()` is called.
pub fn play_video_mp3(path: &[u8]) {
    // Copy path to shared buffer.
    // SAFETY: VIDEO_MP3_PATH is a 128-byte static; len is clamped to 127.
    // Written before VIDEO_MP3_ACTIVE is set (Release ordering ensures visibility).
    unsafe {
        let dst = (&raw mut VIDEO_MP3_PATH).cast::<u8>();
        let len = path.len().min(127);
        let mut i = 0;
        while i < len {
            *dst.add(i) = path[i];
            i += 1;
        }
        *dst.add(len) = 0;
    }
    // Release ensures VIDEO_MP3_PATH writes above are visible before the
    // audio thread observes the flag via Acquire.
    VIDEO_MP3_ACTIVE.store(true, Ordering::Release);
    // Ensure audio is in "playing" state so the thread picks it up.
    AUDIO_STATE.store(1, Ordering::Relaxed);
    // Cmd 7 interrupts any current decode loop without advancing the playlist.
    AUDIO_CMD.store(7, Ordering::Relaxed);
}

/// Stop PIP video audio and resume normal playlist playback.
///
/// Only interrupts the audio thread if a video MP3 was actually active;
/// otherwise this is a no-op so normal music is not disturbed.
pub fn stop_video_mp3() {
    if VIDEO_MP3_ACTIVE.swap(false, Ordering::AcqRel) {
        // Cmd 7 interrupts the video MP3 decode loop.
        AUDIO_CMD.store(7, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Thread start
// ---------------------------------------------------------------------------

pub fn start_audio_thread() {
    crate::debug_log(b"[OASIS] starting audio thread...");

    // SAFETY: Creating and starting a kernel thread for audio playback.
    // sceKernelCreateThread with valid name, entry point, and stack size.
    // sceKernelStartThread with valid thread ID returned from create.
    unsafe {
        let thid = psp::sys::sceKernelCreateThread(
            b"OasisAudio\0".as_ptr(),
            decode::audio_thread_entry,
            0x18, // priority 24: above default (30) for smooth playback
            0x4000,
            psp::sys::ThreadAttributes::empty(),
            core::ptr::null_mut(),
        );
        if thid.0 >= 0 {
            psp::sys::sceKernelStartThread(thid, 0, core::ptr::null_mut());
            crate::debug_log(b"[OASIS] audio thread started");
        } else {
            crate::debug_log(b"[OASIS] audio thread create FAILED");
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers (used across sub-modules)
// ---------------------------------------------------------------------------

pub(crate) fn copy_bytes(buf: &mut [u8], pos: usize, s: &[u8]) -> usize {
    let mut p = pos;
    let mut i = 0;
    while i < s.len() && p < buf.len() {
        buf[p] = s[i];
        p += 1;
        i += 1;
    }
    p
}

pub(crate) fn write_u32_decimal(buf: &mut [u8], pos: usize, val: u32) -> usize {
    if val == 0 {
        if pos < buf.len() {
            buf[pos] = b'0';
            return pos + 1;
        }
        return pos;
    }
    let mut digits = [0u8; 10];
    let mut n = val;
    let mut count = 0;
    while n > 0 {
        digits[count] = b'0' + (n % 10) as u8;
        n /= 10;
        count += 1;
    }
    let mut p = pos;
    while count > 0 {
        count -= 1;
        if p >= buf.len() {
            break;
        }
        buf[p] = digits[count];
        p += 1;
    }
    p
}

pub(crate) fn write_hex32(buf: &mut [u8], pos: usize, val: u32) -> usize {
    let hex = b"0123456789ABCDEF";
    let mut p = pos;
    let mut i = 0;
    while i < 8 && p < buf.len() {
        let nibble = (val >> (28 - i * 4)) & 0xF;
        buf[p] = hex[nibble as usize];
        p += 1;
        i += 1;
    }
    p
}

pub(crate) fn log_i32(prefix: &[u8], val: i32) {
    let mut buf = [0u8; 64];
    let mut p = copy_bytes(&mut buf, 0, prefix);
    if val < 0 {
        if p < buf.len() {
            buf[p] = b'-';
            p += 1;
        }
        p = write_u32_decimal(&mut buf, p, (-(val as i64)) as u32);
    } else {
        p = write_u32_decimal(&mut buf, p, val as u32);
    }
    crate::debug_log(&buf[..p]);
}

/// Parse icy-metaint from raw HTTP response headers (no_std).
pub(crate) fn parse_icy_metaint_raw(headers: &[u8]) -> Option<usize> {
    let needle = b"icy-metaint:";
    let mut i = 0;
    while i + needle.len() <= headers.len() {
        let mut matched = true;
        let mut k = 0;
        while k < needle.len() {
            if headers[i + k].to_ascii_lowercase() != needle[k] {
                matched = false;
                break;
            }
            k += 1;
        }
        if matched {
            let mut j = i + needle.len();
            while j < headers.len() && headers[j] == b' ' {
                j += 1;
            }
            let mut val: usize = 0;
            while j < headers.len() && headers[j].is_ascii_digit() {
                val = val * 10 + (headers[j] - b'0') as usize;
                j += 1;
            }
            if val > 0 {
                return Some(val);
            }
        }
        i += 1;
    }
    None
}

pub(crate) fn skip_id3v2(data: &[u8]) -> usize {
    if data.len() < 10 {
        return 0;
    }
    if data[0] != b'I' || data[1] != b'D' || data[2] != b'3' {
        return 0;
    }
    let size = ((data[6] as u32) << 21)
        | ((data[7] as u32) << 14)
        | ((data[8] as u32) << 7)
        | (data[9] as u32);
    10 + size as usize
}

pub(crate) fn find_mp3_sync(data: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < data.len() {
        if data[i] == 0xFF && (data[i + 1] & 0xE0) == 0xE0 {
            let version = (data[i + 1] >> 3) & 0x03;
            let layer = (data[i + 1] >> 1) & 0x03;
            if version != 1 && layer != 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // copy_bytes
    // -----------------------------------------------------------------------

    #[test]
    fn copy_bytes_basic() {
        let mut buf = [0u8; 32];
        let p = copy_bytes(&mut buf, 0, b"hello");
        assert_eq!(p, 5);
        assert_eq!(&buf[..5], b"hello");
    }

    #[test]
    fn copy_bytes_at_offset() {
        let mut buf = [0u8; 32];
        let p = copy_bytes(&mut buf, 10, b"ABC");
        assert_eq!(p, 13);
        assert_eq!(&buf[10..13], b"ABC");
    }

    #[test]
    fn copy_bytes_truncation() {
        let mut buf = [0u8; 3];
        let p = copy_bytes(&mut buf, 0, b"hello");
        assert_eq!(p, 3);
        assert_eq!(&buf, b"hel");
    }

    #[test]
    fn copy_bytes_empty() {
        let mut buf = [0u8; 8];
        let p = copy_bytes(&mut buf, 0, b"");
        assert_eq!(p, 0);
    }

    // -----------------------------------------------------------------------
    // write_u32_decimal
    // -----------------------------------------------------------------------

    #[test]
    fn write_u32_decimal_zero() {
        let mut buf = [0u8; 16];
        let p = write_u32_decimal(&mut buf, 0, 0);
        assert_eq!(p, 1);
        assert_eq!(buf[0], b'0');
    }

    #[test]
    fn write_u32_decimal_small() {
        let mut buf = [0u8; 16];
        let p = write_u32_decimal(&mut buf, 0, 42);
        assert_eq!(p, 2);
        assert_eq!(&buf[..2], b"42");
    }

    #[test]
    fn write_u32_decimal_large() {
        let mut buf = [0u8; 16];
        let p = write_u32_decimal(&mut buf, 0, 1000000);
        assert_eq!(p, 7);
        assert_eq!(&buf[..7], b"1000000");
    }

    #[test]
    fn write_u32_decimal_max() {
        let mut buf = [0u8; 16];
        let p = write_u32_decimal(&mut buf, 0, u32::MAX);
        assert_eq!(&buf[..p], b"4294967295");
    }

    // -----------------------------------------------------------------------
    // write_hex32
    // -----------------------------------------------------------------------

    #[test]
    fn write_hex32_zero() {
        let mut buf = [0u8; 16];
        let p = write_hex32(&mut buf, 0, 0);
        assert_eq!(p, 8);
        assert_eq!(&buf[..8], b"00000000");
    }

    #[test]
    fn write_hex32_deadbeef() {
        let mut buf = [0u8; 16];
        let p = write_hex32(&mut buf, 0, 0xDEADBEEF);
        assert_eq!(p, 8);
        assert_eq!(&buf[..8], b"DEADBEEF");
    }

    #[test]
    fn write_hex32_all_nibbles() {
        let mut buf = [0u8; 16];
        let p = write_hex32(&mut buf, 0, 0x01234567);
        assert_eq!(p, 8);
        assert_eq!(&buf[..8], b"01234567");
    }

    #[test]
    fn write_hex32_max() {
        let mut buf = [0u8; 16];
        let p = write_hex32(&mut buf, 0, 0xFFFFFFFF);
        assert_eq!(p, 8);
        assert_eq!(&buf[..8], b"FFFFFFFF");
    }

    #[test]
    fn write_hex32_at_offset() {
        let mut buf = [0u8; 16];
        let p1 = copy_bytes(&mut buf, 0, b"0x");
        let p2 = write_hex32(&mut buf, p1, 0xCAFE);
        assert_eq!(&buf[..p2], b"0x0000CAFE");
    }

    // -----------------------------------------------------------------------
    // parse_icy_metaint_raw
    // -----------------------------------------------------------------------

    #[test]
    fn parse_icy_metaint_basic() {
        let headers = b"HTTP/1.1 200 OK\r\nicy-metaint:8192\r\n\r\n";
        assert_eq!(parse_icy_metaint_raw(headers), Some(8192));
    }

    #[test]
    fn parse_icy_metaint_with_space() {
        let headers = b"icy-metaint: 16000\r\n";
        assert_eq!(parse_icy_metaint_raw(headers), Some(16000));
    }

    #[test]
    fn parse_icy_metaint_case_insensitive() {
        let headers = b"ICY-METAINT:4096\r\n";
        assert_eq!(parse_icy_metaint_raw(headers), Some(4096));
    }

    #[test]
    fn parse_icy_metaint_mixed_case() {
        let headers = b"Icy-MetaInt: 32768\r\n";
        assert_eq!(parse_icy_metaint_raw(headers), Some(32768));
    }

    #[test]
    fn parse_icy_metaint_not_found() {
        let headers = b"HTTP/1.1 200 OK\r\ncontent-type: audio/mpeg\r\n\r\n";
        assert_eq!(parse_icy_metaint_raw(headers), None);
    }

    #[test]
    fn parse_icy_metaint_zero_value() {
        let headers = b"icy-metaint:0\r\n";
        assert_eq!(parse_icy_metaint_raw(headers), None);
    }

    #[test]
    fn parse_icy_metaint_empty() {
        assert_eq!(parse_icy_metaint_raw(b""), None);
    }

    // -----------------------------------------------------------------------
    // skip_id3v2
    // -----------------------------------------------------------------------

    #[test]
    fn skip_id3v2_no_tag() {
        let data = [0xFF, 0xFB, 0x90, 0x00]; // MP3 sync
        assert_eq!(skip_id3v2(&data), 0);
    }

    #[test]
    fn skip_id3v2_valid_tag() {
        // ID3v2 header: "ID3" + version(2) + flags(1) + size(4)
        // Size uses syncsafe encoding: each byte's MSB is 0
        let mut data = [0u8; 20];
        data[0] = b'I';
        data[1] = b'D';
        data[2] = b'3';
        data[3] = 4; // version major
        data[4] = 0; // version minor
        data[5] = 0; // flags
        // Size: 0x00 0x00 0x01 0x00 = 128 bytes (syncsafe)
        data[6] = 0x00;
        data[7] = 0x00;
        data[8] = 0x01;
        data[9] = 0x00;
        assert_eq!(skip_id3v2(&data), 10 + 128);
    }

    #[test]
    fn skip_id3v2_too_short() {
        assert_eq!(skip_id3v2(b"ID3"), 0);
        assert_eq!(skip_id3v2(b""), 0);
    }

    #[test]
    fn skip_id3v2_wrong_magic() {
        let data = [b'X', b'Y', b'Z', 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(skip_id3v2(&data), 0);
    }

    // -----------------------------------------------------------------------
    // find_mp3_sync
    // -----------------------------------------------------------------------

    #[test]
    fn find_mp3_sync_at_start() {
        // Valid MPEG1 Layer3 sync: 0xFF 0xFB (version=3, layer=1)
        let data = [0xFF, 0xFB, 0x90, 0x00];
        assert_eq!(find_mp3_sync(&data, 0), Some(0));
    }

    #[test]
    fn find_mp3_sync_offset() {
        // Some junk, then sync
        let mut data = vec![0x00; 10];
        data.push(0xFF);
        data.push(0xFB); // MPEG1 Layer3
        assert_eq!(find_mp3_sync(&data, 0), Some(10));
    }

    #[test]
    fn find_mp3_sync_not_found() {
        let data = [0x00, 0x00, 0x00, 0x00];
        assert_eq!(find_mp3_sync(&data, 0), None);
    }

    #[test]
    fn find_mp3_sync_with_start_offset() {
        let mut data = vec![0xFF, 0xFB]; // sync at 0
        data.extend_from_slice(&[0x00; 5]);
        data.push(0xFF);
        data.push(0xFB); // sync at 7
        assert_eq!(find_mp3_sync(&data, 1), Some(7));
    }

    #[test]
    fn find_mp3_sync_rejects_invalid_version() {
        // version=1 (reserved) should be rejected
        // 0xFF 0xE8 = 11111111 11101000 -> version=01 (reserved)
        let data = [0xFF, 0xE8, 0x00, 0x00];
        assert_eq!(find_mp3_sync(&data, 0), None);
    }

    #[test]
    fn find_mp3_sync_rejects_invalid_layer() {
        // layer=0 (reserved) should be rejected
        // 0xFF 0xE1 = 11111111 11100001 -> layer=00 (reserved)
        let data = [0xFF, 0xE1, 0x00, 0x00];
        assert_eq!(find_mp3_sync(&data, 0), None);
    }

    #[test]
    fn find_mp3_sync_too_short() {
        assert_eq!(find_mp3_sync(&[0xFF], 0), None);
        assert_eq!(find_mp3_sync(&[], 0), None);
    }
}
