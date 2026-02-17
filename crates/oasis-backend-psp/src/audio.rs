//! Audio playback (MP3 via sceAudiocodec + psp::audio) and `AudioBackend` trait.
//!
//! Uses the low-level `sceAudiocodec` frame-by-frame decoder instead of the
//! high-level `sceMp3*` API, which crashes on real PSP hardware after ~2-3
//! handle reuse cycles.
//!
//! MP3 data is **streamed from file** using a small fixed-size read buffer
//! (32 KB) to avoid large heap allocations that cause heap fragmentation
//! and crashes on PSP's limited 24 MB user memory.

use psp::audio::{AudioChannel, AudioFormat};
use psp::audiocodec::{AudiocodecDecoder, CodecType};
use psp::mp3::{find_sync, skip_id3v2};

use oasis_core::backend::{AudioBackend, AudioTrackId};
use oasis_core::error::{OasisError, Result};

use crate::threading::{AudioCmd, AudioHandle, send_audio_cmd};

/// Standard MP3 frame size (MPEG1 Layer 3).
const MP3_FRAME_SAMPLES: i32 = 1152;

/// Size of the read buffer for streaming MP3 from file.
/// 32 KB is enough for many MP3 frames and avoids large heap allocations.
const READ_BUF_SIZE: usize = 32 * 1024;

/// Load AV codec modules once (idempotent). Called lazily on first play
/// to avoid conflicts with the PRX overlay at boot time.
fn load_av_modules_once() {
    use core::sync::atomic::{AtomicBool, Ordering};
    static LOADED: AtomicBool = AtomicBool::new(false);
    if LOADED.swap(true, Ordering::Relaxed) {
        return; // Already loaded.
    }
    unsafe {
        psp::sys::sceUtilityLoadModule(psp::sys::Module::AvCodec);
        psp::sys::sceUtilityLoadModule(psp::sys::Module::AvMpegBase);
        psp::sys::sceUtilityLoadModule(psp::sys::Module::AvMp3);
    }
}

// ---------------------------------------------------------------------------
// MP3 frame header parsing
// ---------------------------------------------------------------------------

/// MPEG version bitrate tables (kbps). Index: bitrate_index (1..14).
const BITRATES_V1_L3: [u32; 15] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];
const BITRATES_V2_L3: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

/// Sample rates by MPEG version. [version_index][srate_index]
const SAMPLE_RATES: [[u32; 3]; 4] = [
    [11025, 12000, 8000],  // MPEG 2.5
    [0, 0, 0],             // reserved
    [22050, 24000, 16000], // MPEG 2
    [44100, 48000, 32000], // MPEG 1
];

/// Parsed MP3 frame header.
struct Mp3FrameHeader {
    /// Sample rate in Hz.
    sample_rate: u32,
    /// Bitrate in kbps.
    bitrate: u32,
    /// Number of channels (1 or 2).
    channels: u32,
}

/// Parse an MP3 frame header from 4 bytes starting at a sync position.
fn parse_mp3_header(data: &[u8]) -> Option<Mp3FrameHeader> {
    if data.len() < 4 {
        return None;
    }
    let b1 = data[1];
    let b2 = data[2];
    let b3 = data[3];

    let version_bits = (b1 >> 3) & 0x03;
    let layer_bits = (b1 >> 1) & 0x03;
    let bitrate_idx = (b2 >> 4) & 0x0F;
    let srate_idx = (b2 >> 2) & 0x03;
    let channel_mode = (b3 >> 6) & 0x03;

    if version_bits == 1
        || layer_bits == 0
        || bitrate_idx == 0
        || bitrate_idx == 15
        || srate_idx == 3
    {
        return None;
    }
    // Only Layer III.
    if layer_bits != 1 {
        return None;
    }

    let is_v1 = version_bits == 3;
    let bitrate = if is_v1 {
        BITRATES_V1_L3[bitrate_idx as usize]
    } else {
        BITRATES_V2_L3[bitrate_idx as usize]
    };
    let sample_rate = SAMPLE_RATES[version_bits as usize][srate_idx as usize];
    if sample_rate == 0 || bitrate == 0 {
        return None;
    }
    let channels = if channel_mode == 3 { 1 } else { 2 };

    Some(Mp3FrameHeader {
        sample_rate,
        bitrate,
        channels,
    })
}

// ---------------------------------------------------------------------------
// AudioPlayer — streaming sceAudiocodec-based MP3 playback
// ---------------------------------------------------------------------------

/// MP3 playback engine using sceAudiocodec with file streaming.
///
/// Instead of loading the entire MP3 into a Vec (which fragments the PSP's
/// 24MB heap after 2-3 songs), this streams from an open file descriptor
/// using a fixed 32KB read buffer — matching the PRX plugin's approach.
///
/// The `AudiocodecDecoder` and `AudioChannel` are created once and reused
/// across all songs.
pub struct AudioPlayer {
    pub(crate) decoder: Option<AudiocodecDecoder>,
    pub(crate) channel: Option<AudioChannel>,
    /// Fixed read buffer for streaming MP3 data from file.
    read_buf: Vec<u8>,
    /// Number of valid bytes in `read_buf`.
    buf_valid: usize,
    /// Current read position within `read_buf`.
    buf_pos: usize,
    /// Open file descriptor for the current song (negative = none).
    fd: psp::sys::SceUid,
    /// Total file size in bytes.
    file_size: usize,
    /// Bytes read from file so far.
    file_pos: usize,
    playing: bool,
    paused: bool,
    /// Hardware volume (0x0000..=0x8000).
    hw_volume: i32,
    /// PCM decode buffer (stereo: 1152 * 2 samples).
    pub(crate) pcm_buf: Vec<i16>,
    /// Cached MP3 info from first frame header.
    pub sample_rate: u32,
    pub bitrate: u32,
    pub channels: u32,
    /// Count of decoded MP3 frames (for position tracking).
    pub frames_decoded: u32,
    /// Total MP3 data size in bytes (for duration estimation).
    pub data_size: u32,
    /// Consecutive decode errors (to bail on corrupt data).
    error_count: u32,
}

impl AudioPlayer {
    pub fn new() -> Self {
        Self {
            decoder: None,
            channel: None,
            read_buf: vec![0u8; READ_BUF_SIZE],
            buf_valid: 0,
            buf_pos: 0,
            fd: psp::sys::SceUid(-1),
            file_size: 0,
            file_pos: 0,
            playing: false,
            paused: false,
            hw_volume: 0x8000,
            pcm_buf: vec![0i16; 1152 * 2],
            sample_rate: 0,
            bitrate: 0,
            channels: 0,
            frames_decoded: 0,
            data_size: 0,
            error_count: 0,
        }
    }

    /// Initialize the audio subsystem (no-op, kept for API compat).
    pub fn init(&mut self) -> bool {
        true
    }

    /// Close the current file if open.
    fn close_file(&mut self) {
        if self.fd >= psp::sys::SceUid(0) {
            unsafe { psp::sys::sceIoClose(self.fd) };
            self.fd = psp::sys::SceUid(-1);
        }
    }

    /// Load an MP3 file from the Memory Stick and start streaming playback.
    pub fn load_and_play(&mut self, path: &str) -> bool {
        self.playing = false;
        self.paused = false;
        self.close_file();

        load_av_modules_once();

        // Open file.
        let mut path_bytes: Vec<u8> = path.as_bytes().to_vec();
        path_bytes.push(0); // null-terminate
        let fd =
            unsafe { psp::sys::sceIoOpen(path_bytes.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0) };
        if fd < psp::sys::SceUid(0) {
            return false;
        }

        // Get file size.
        let size = unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::End) } as usize;
        unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::Set) };
        if size == 0 {
            unsafe { psp::sys::sceIoClose(fd) };
            return false;
        }

        self.fd = fd;
        self.file_size = size;
        self.file_pos = 0;
        self.data_size = size as u32;

        // Initial read into buffer.
        let read = unsafe {
            psp::sys::sceIoRead(
                self.fd,
                self.read_buf.as_mut_ptr() as *mut _,
                READ_BUF_SIZE as u32,
            )
        };
        if read <= 0 {
            self.close_file();
            return false;
        }
        self.buf_valid = read as usize;
        self.buf_pos = 0;
        self.file_pos = self.buf_valid;

        // Skip ID3v2 tag. If the tag is larger than the read buffer
        // (common with embedded album art), seek past it in the file
        // and re-read from the first audio frame.
        let id3_skip = skip_id3v2(&self.read_buf[..self.buf_valid]);
        if id3_skip > 0 {
            // Adjust data_size to exclude the tag for duration estimation.
            self.data_size = self.data_size.saturating_sub(id3_skip as u32);
            if id3_skip < self.buf_valid {
                self.buf_pos = id3_skip;
            } else {
                // Tag exceeds buffer — seek past it in the file.
                unsafe {
                    psp::sys::sceIoLseek(self.fd, id3_skip as i64, psp::sys::IoWhence::Set);
                }
                self.file_pos = id3_skip;
                let re_read = unsafe {
                    psp::sys::sceIoRead(
                        self.fd,
                        self.read_buf.as_mut_ptr() as *mut _,
                        READ_BUF_SIZE as u32,
                    )
                };
                if re_read <= 0 {
                    self.close_file();
                    return false;
                }
                self.buf_valid = re_read as usize;
                self.buf_pos = 0;
                self.file_pos = id3_skip + self.buf_valid;
            }
        }

        // Parse first frame header for metadata.
        let buf_slice = &self.read_buf[self.buf_pos..self.buf_valid];
        if let Some(sync_off) = find_sync(buf_slice, 0) {
            if let Some(hdr) = parse_mp3_header(&buf_slice[sync_off..]) {
                self.sample_rate = hdr.sample_rate;
                self.bitrate = hdr.bitrate;
                self.channels = hdr.channels;
            }
        }

        self.frames_decoded = 0;
        self.error_count = 0;

        // Create the audiocodec decoder once, reuse forever.
        if self.decoder.is_none() {
            match AudiocodecDecoder::new(CodecType::Mp3) {
                Ok(dec) => self.decoder = Some(dec),
                Err(_) => {
                    self.close_file();
                    return false;
                },
            }
        }

        // Reuse audio channel if we already have one.
        if self.channel.is_none() {
            match AudioChannel::reserve(MP3_FRAME_SAMPLES, AudioFormat::Stereo) {
                Ok(ch) => self.channel = Some(ch),
                Err(_) => {
                    self.close_file();
                    return false;
                },
            }
        }

        self.playing = true;
        self.paused = false;
        true
    }

    /// Start playback from in-memory MP3 data.
    ///
    /// Writes data to a temp file and streams from it, keeping the same
    /// streaming architecture. Falls back to the file path approach.
    pub fn load_and_play_owned(&mut self, data: Vec<u8>) -> bool {
        // Write data to a temp file, then stream from it.
        // This avoids keeping the full MP3 in heap memory.
        let temp_path = "ms0:/PSP/GAME/oasis_os/__temp_audio.mp3";
        let temp_path_c = b"ms0:/PSP/GAME/oasis_os/__temp_audio.mp3\0";

        let fd = unsafe {
            psp::sys::sceIoOpen(
                temp_path_c.as_ptr(),
                psp::sys::IoOpenFlags::WR_ONLY
                    | psp::sys::IoOpenFlags::CREAT
                    | psp::sys::IoOpenFlags::TRUNC,
                0o777,
            )
        };
        if fd < psp::sys::SceUid(0) {
            return false;
        }
        let written = unsafe { psp::sys::sceIoWrite(fd, data.as_ptr() as *const _, data.len()) };
        unsafe { psp::sys::sceIoClose(fd) };

        // Free the input data immediately — we don't need it anymore.
        drop(data);

        if written <= 0 {
            return false;
        }

        self.load_and_play(temp_path)
    }

    /// Start playback from a borrowed slice (copies data internally).
    pub fn load_and_play_data(&mut self, data: &[u8]) -> bool {
        self.load_and_play_owned(data.to_vec())
    }

    /// Refill the read buffer: compact consumed data and read more from file.
    ///
    /// Uses the PRX plugin's compact+stream-refill pattern: when more than
    /// half the buffer is consumed, shift remaining data to the front and
    /// top up in small chunks (4KB) to avoid blocking audio output.
    fn refill_buffer(&mut self) {
        // Compact when more than half consumed.
        if self.buf_pos > READ_BUF_SIZE / 2 && self.file_pos < self.file_size {
            let remaining = self.buf_valid - self.buf_pos;
            if remaining > 0 {
                // SAFETY: Manual byte-by-byte copy (LLVM memcpy can recurse
                // on MIPS). src and dst overlap so we copy forward.
                let ptr = self.read_buf.as_mut_ptr();
                for i in 0..remaining {
                    unsafe { *ptr.add(i) = *ptr.add(self.buf_pos + i) };
                }
            }
            self.buf_valid = remaining;
            self.buf_pos = 0;
        }

        // Top up buffer if there's room (small reads to avoid audio stalls).
        if self.buf_valid < READ_BUF_SIZE && self.file_pos < self.file_size {
            let room = READ_BUF_SIZE - self.buf_valid;
            let chunk = room.min(4096);
            let read = unsafe {
                psp::sys::sceIoRead(
                    self.fd,
                    self.read_buf.as_mut_ptr().add(self.buf_valid) as *mut _,
                    chunk as u32,
                )
            };
            if read > 0 {
                self.buf_valid += read as usize;
                self.file_pos += read as usize;
            }
        }
    }

    /// Pump decoded audio to the output channel. Call each frame.
    pub fn update(&mut self) {
        if !self.playing || self.paused {
            return;
        }

        if self.decoder.is_none() || self.channel.is_none() {
            return;
        }

        // Refill buffer from file (before borrowing decoder/channel).
        self.refill_buffer();

        let avail = self.buf_valid - self.buf_pos;
        if avail < 4 {
            self.playing = false;
            self.close_file();
            return;
        }

        // Find next MP3 frame sync in the buffer.
        let sync_pos = match find_sync(&self.read_buf[..self.buf_valid], self.buf_pos) {
            Some(p) => p,
            None => {
                self.buf_pos = self.buf_valid;
                return;
            },
        };
        self.buf_pos = sync_pos;

        if self.buf_valid - self.buf_pos < 8 {
            return;
        }

        // Decode one frame via sceAudiocodec.
        // We use split borrows: decoder, channel, read_buf, and pcm_buf are
        // all separate fields, so borrowck allows simultaneous access.
        for s in &mut self.pcm_buf {
            *s = 0;
        }

        let decoder = self.decoder.as_mut().unwrap();
        let buf_pos = self.buf_pos;
        let buf_valid = self.buf_valid;
        let result = decoder.decode(&self.read_buf[buf_pos..buf_valid], &mut self.pcm_buf);

        match result {
            Ok(consumed) => {
                if consumed == 0 {
                    self.error_count += 1;
                    self.buf_pos += 1;
                    if self.error_count > 100 {
                        self.playing = false;
                        self.close_file();
                    }
                    return;
                }
                self.error_count = 0;
                self.buf_pos += consumed;
                self.frames_decoded += 1;
                let channel = self.channel.as_ref().unwrap();
                let _ = channel.output_blocking(self.hw_volume, &self.pcm_buf);
            },
            Err(_) => {
                self.error_count += 1;
                self.buf_pos += 1;
                if self.error_count > 100 {
                    self.playing = false;
                    self.close_file();
                }
            },
        }
    }

    /// Stop playback. Decoder and channel are kept alive for reuse.
    pub fn stop(&mut self) {
        self.playing = false;
        self.paused = false;
        self.close_file();
    }

    /// Toggle pause/resume.
    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    /// Set volume (0..=100) mapped to PSP hardware range (0x0000..=0x8000).
    pub fn set_volume(&mut self, volume: u8) {
        let v = volume.min(100) as i32;
        self.hw_volume = v * 0x8000 / 100;
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Estimated playback position in milliseconds.
    pub fn position_ms(&self) -> u64 {
        if self.sample_rate == 0 {
            return 0;
        }
        (self.frames_decoded as u64 * MP3_FRAME_SAMPLES as u64 * 1000) / self.sample_rate as u64
    }

    /// Estimated total duration in milliseconds (from bitrate + file size).
    pub fn duration_ms(&self) -> u64 {
        if self.bitrate == 0 {
            return 0;
        }
        (self.data_size as u64 * 8) / self.bitrate as u64
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.close_file();
    }
}

// ---------------------------------------------------------------------------
// RadioStreamer — internet radio via raw TCP socket + ICY protocol
// ---------------------------------------------------------------------------

/// Internet radio streamer that reads MP3 data from a connected TCP socket.
///
/// Handles ICY metadata demuxing inline: every `icy_metaint` bytes of audio
/// data, a metadata block appears (1-byte length * 16 = metadata bytes).
/// Decoded titles are buffered for the main thread to poll.
///
/// Shares the `AudiocodecDecoder` and `AudioChannel` from `AudioPlayer`
/// (only one active at a time).
pub struct RadioStreamer {
    socket_fd: i32,
    read_buf: Vec<u8>,
    /// Number of valid bytes in the read buffer.
    pub buf_valid: usize,
    buf_pos: usize,
    /// ICY metadata interval (0 = no metadata).
    icy_metaint: usize,
    /// Bytes of audio data since last metadata block.
    icy_audio_count: usize,
    /// Whether we're currently inside a metadata block.
    icy_in_meta: bool,
    /// Remaining bytes in the current metadata block.
    icy_meta_remaining: usize,
    /// Accumulated metadata bytes for current block.
    icy_meta_buf: Vec<u8>,
    /// Latest ICY title extracted from metadata.
    pending_meta: Option<String>,
    /// Whether the stream is still buffering initial data.
    pub buffering: bool,
    /// Hardware volume (0x0000..=0x8000).
    hw_volume: i32,
    /// Consecutive decode errors.
    error_count: u32,
}

impl RadioStreamer {
    /// Minimum bytes buffered before starting decode (32KB).
    pub const BUFFER_THRESHOLD: usize = 32 * 1024;
    /// Read buffer size (64KB for network streaming).
    const BUF_SIZE: usize = 64 * 1024;

    pub fn new(socket_fd: i32, icy_metaint: usize) -> Self {
        load_av_modules_once();
        Self {
            socket_fd,
            read_buf: vec![0u8; Self::BUF_SIZE],
            buf_valid: 0,
            buf_pos: 0,
            icy_metaint,
            icy_audio_count: 0,
            icy_in_meta: false,
            icy_meta_remaining: 0,
            icy_meta_buf: Vec::with_capacity(256),
            pending_meta: None,
            buffering: true,
            hw_volume: 0x8000,
            error_count: 0,
        }
    }

    /// Set volume (0..=100) mapped to PSP hardware range.
    pub fn set_volume(&mut self, volume: u8) {
        let v = volume.min(100) as i32;
        self.hw_volume = v * 0x8000 / 100;
    }

    /// Non-blocking receive from the socket into the read buffer.
    ///
    /// Handles ICY metadata demuxing inline: receives into a temporary
    /// buffer, then copies only audio bytes (stripping metadata blocks)
    /// into the main read buffer.
    pub fn recv_data(&mut self) {
        // Compact buffer if more than half consumed.
        if self.buf_pos > Self::BUF_SIZE / 2 {
            let remaining = self.buf_valid - self.buf_pos;
            if remaining > 0 {
                // SAFETY: Manual byte copy to avoid LLVM memcpy recursion on MIPS.
                let ptr = self.read_buf.as_mut_ptr();
                for i in 0..remaining {
                    unsafe { *ptr.add(i) = *ptr.add(self.buf_pos + i) };
                }
            }
            self.buf_valid = remaining;
            self.buf_pos = 0;
        }

        let room = Self::BUF_SIZE - self.buf_valid;
        if room == 0 {
            return;
        }

        if self.icy_metaint == 0 {
            // No ICY metadata: receive directly into read buffer.
            let chunk = room.min(4096);
            // SAFETY: Non-blocking recv (MSG_DONTWAIT = 0x80 on PSP).
            let n = unsafe {
                psp::sys::sceNetInetRecv(
                    self.socket_fd,
                    self.read_buf.as_mut_ptr().add(self.buf_valid) as *mut _,
                    chunk,
                    0x80,
                )
            };
            if n > 0 {
                self.buf_valid += n as usize;
            }
        } else {
            // ICY metadata enabled: receive into temp buffer, demux.
            let mut tmp = [0u8; 4096];
            let chunk = room.min(tmp.len());
            // SAFETY: Non-blocking recv into temp buffer.
            let n = unsafe {
                psp::sys::sceNetInetRecv(self.socket_fd, tmp.as_mut_ptr() as *mut _, chunk, 0x80)
            };
            if n > 0 {
                let received = n as usize;
                let mut i = 0;
                while i < received {
                    if self.icy_in_meta {
                        // Consuming metadata bytes.
                        let avail = received - i;
                        let take = avail.min(self.icy_meta_remaining);
                        for j in 0..take {
                            self.icy_meta_buf.push(tmp[i + j]);
                        }
                        i += take;
                        self.icy_meta_remaining -= take;
                        if self.icy_meta_remaining == 0 {
                            self.parse_icy_meta();
                            self.icy_meta_buf.clear();
                            self.icy_in_meta = false;
                        }
                    } else {
                        let until_meta = self.icy_metaint - self.icy_audio_count;
                        let avail = received - i;
                        let take = avail.min(until_meta).min(Self::BUF_SIZE - self.buf_valid);
                        if take == 0 {
                            break; // Buffer full.
                        }
                        // Copy audio bytes into read buffer.
                        for j in 0..take {
                            self.read_buf[self.buf_valid + j] = tmp[i + j];
                        }
                        self.buf_valid += take;
                        self.icy_audio_count += take;
                        i += take;
                        if self.icy_audio_count >= self.icy_metaint {
                            self.icy_audio_count = 0;
                            if i < received {
                                let meta_len = tmp[i] as usize * 16;
                                i += 1;
                                if meta_len > 0 {
                                    self.icy_in_meta = true;
                                    self.icy_meta_remaining = meta_len;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Extract `StreamTitle='...'` from ICY metadata.
    fn parse_icy_meta(&mut self) {
        let meta_str = String::from_utf8_lossy(&self.icy_meta_buf);
        if let Some(start) = meta_str.find("StreamTitle='") {
            let rest = &meta_str[start + 13..];
            if let Some(end) = rest.find('\'') {
                let title = rest[..end].to_string();
                if !title.is_empty() {
                    self.pending_meta = Some(title);
                }
            }
        }
    }

    /// Take the latest ICY metadata title (if any).
    pub fn take_meta(&mut self) -> Option<String> {
        self.pending_meta.take()
    }

    /// Decode one MP3 frame from the buffer and output to audio channel.
    ///
    /// Borrows the `AudioPlayer`'s decoder and channel (only one source
    /// is active at a time).
    pub fn update(&mut self, player: &mut AudioPlayer) {
        // Ensure decoder and channel exist.
        if player.decoder.is_none() {
            match AudiocodecDecoder::new(CodecType::Mp3) {
                Ok(dec) => player.decoder = Some(dec),
                Err(_) => return,
            }
        }
        if player.channel.is_none() {
            match AudioChannel::reserve(MP3_FRAME_SAMPLES, AudioFormat::Stereo) {
                Ok(ch) => player.channel = Some(ch),
                Err(_) => return,
            }
        }

        let avail = self.buf_valid - self.buf_pos;
        if avail < 4 {
            return; // Need more data.
        }

        // Find next sync.
        let sync_pos = match find_sync(&self.read_buf[..self.buf_valid], self.buf_pos) {
            Some(p) => p,
            None => {
                self.buf_pos = self.buf_valid;
                return;
            },
        };
        self.buf_pos = sync_pos;

        if self.buf_valid - self.buf_pos < 8 {
            return;
        }

        // Decode one frame.
        for s in &mut player.pcm_buf {
            *s = 0;
        }

        let decoder = player.decoder.as_mut().unwrap();
        let buf_pos = self.buf_pos;
        let buf_valid = self.buf_valid;
        let result = decoder.decode(&self.read_buf[buf_pos..buf_valid], &mut player.pcm_buf);

        match result {
            Ok(consumed) => {
                if consumed == 0 {
                    self.error_count += 1;
                    self.buf_pos += 1;
                    return;
                }
                self.error_count = 0;
                self.buf_pos += consumed;
                let channel = player.channel.as_ref().unwrap();
                let _ = channel.output_blocking(self.hw_volume, &player.pcm_buf);
            },
            Err(_) => {
                self.error_count += 1;
                self.buf_pos += 1;
            },
        }
    }

    /// Check if the stream has hit too many consecutive errors.
    pub fn is_error(&self) -> bool {
        self.error_count > 200
    }

    /// Stop streaming and close the socket.
    pub fn stop(&mut self) {
        if self.socket_fd >= 0 {
            // SAFETY: Close the radio streaming socket.
            unsafe { psp::sys::sceNetInetClose(self.socket_fd) };
            self.socket_fd = -1;
        }
    }
}

impl Drop for RadioStreamer {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// AudioBackend trait implementation (delegates to worker thread)
// ---------------------------------------------------------------------------

/// PSP audio backend that delegates to the audio worker thread.
///
/// Track data is moved (not cloned) to the audio thread on play to
/// minimize peak memory. Only one track's data lives in this struct
/// at a time — previous tracks are freed on load.
pub struct PspAudioBackend {
    audio: AudioHandle,
    tracks: Vec<Option<Vec<u8>>>,
    current_track: Option<u64>,
    volume: u8,
}

impl PspAudioBackend {
    /// Create a new PSP audio backend.
    pub fn new() -> Self {
        Self {
            audio: AudioHandle,
            tracks: Vec::new(),
            current_track: None,
            volume: 80,
        }
    }
}

impl AudioBackend for PspAudioBackend {
    fn init(&mut self) -> Result<()> {
        Ok(())
    }

    fn load_track(&mut self, data: &[u8]) -> Result<AudioTrackId> {
        let id = self.tracks.len() as u64;
        // Free all previous track data to conserve PSP memory.
        for slot in &mut self.tracks {
            *slot = None;
        }
        self.tracks.push(Some(data.to_vec()));
        Ok(AudioTrackId(id))
    }

    fn play(&mut self, track: AudioTrackId) -> Result<()> {
        let idx = track.0 as usize;
        let data = self
            .tracks
            .get_mut(idx)
            .and_then(|slot| slot.take())
            .ok_or_else(|| OasisError::Backend(format!("track {} not loaded", track.0)))?;
        send_audio_cmd(AudioCmd::LoadAndPlayData(data));
        self.current_track = Some(track.0);
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        send_audio_cmd(AudioCmd::Pause);
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        send_audio_cmd(AudioCmd::Resume);
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        send_audio_cmd(AudioCmd::Stop);
        Ok(())
    }

    fn set_volume(&mut self, volume: u8) -> Result<()> {
        self.volume = volume.min(100);
        send_audio_cmd(AudioCmd::SetVolume(self.volume));
        Ok(())
    }

    fn get_volume(&self) -> u8 {
        self.volume
    }

    fn is_playing(&self) -> bool {
        self.audio.is_playing()
    }

    fn position_ms(&self) -> u64 {
        self.audio.position_ms()
    }

    fn duration_ms(&self) -> u64 {
        self.audio.duration_ms()
    }

    fn unload_track(&mut self, track: AudioTrackId) -> Result<()> {
        let idx = track.0 as usize;
        if self.current_track == Some(track.0) {
            self.stop()?;
            self.current_track = None;
        }
        if let Some(slot) = self.tracks.get_mut(idx) {
            *slot = None;
        }
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        self.stop()?;
        self.tracks.clear();
        Ok(())
    }
}
