//! AudioPlayer — streaming sceAudiocodec-based MP3 playback.

use psp::audio::{AudioChannel, AudioFormat};
use psp::audiocodec::{AudiocodecDecoder, CodecType};
use psp::mp3::{find_sync, skip_id3v2};

use super::frame_parser::parse_mp3_header;
use super::{load_av_modules_once, MP3_FRAME_SAMPLES, READ_BUF_SIZE};

/// MP3 playback engine using sceAudiocodec with file streaming.
///
/// Instead of loading the entire MP3 into a Vec (which fragments the PSP's
/// 24MB heap after 2-3 songs), this streams from an open file descriptor
/// using a fixed 32KB read buffer — matching the PRX plugin's approach.
///
/// The `AudiocodecDecoder` and `AudioChannel` are created once and reused
/// across all songs.
pub(crate) struct AudioPlayer {
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
            // SAFETY: fd is a valid open file descriptor (checked >= 0).
            // After closing, we immediately invalidate it to -1.
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
        // SAFETY: path_bytes is a null-terminated byte string on the stack.
        // sceIoOpen returns a valid fd or a negative error code.
        let fd =
            unsafe { psp::sys::sceIoOpen(path_bytes.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0) };
        if fd < psp::sys::SceUid(0) {
            return false;
        }

        // Get file size.
        // SAFETY: fd is a valid file descriptor returned by sceIoOpen above.
        // Seeking to End then back to Set is the standard way to measure
        // file size on PSP.
        let size = unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::End) } as usize;
        unsafe { psp::sys::sceIoLseek(fd, 0, psp::sys::IoWhence::Set) };
        if size == 0 {
            // SAFETY: fd is valid; closing an empty file.
            unsafe { psp::sys::sceIoClose(fd) };
            return false;
        }

        self.fd = fd;
        self.file_size = size;
        self.file_pos = 0;
        self.data_size = size as u32;

        // Initial read into buffer.
        // SAFETY: self.fd is a valid open file descriptor. read_buf is a
        // Vec<u8> of READ_BUF_SIZE bytes, so the pointer and length are valid.
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
                // SAFETY: self.fd is a valid open file descriptor.
                // Seeking past the ID3v2 tag to the first audio frame.
                unsafe {
                    psp::sys::sceIoLseek(self.fd, id3_skip as i64, psp::sys::IoWhence::Set);
                }
                self.file_pos = id3_skip;
                // SAFETY: self.fd is valid; read_buf pointer and size are correct.
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
                Err(e) => {
                    psp::dprintln!("AudiocodecDecoder init failed: {e}");
                    self.close_file();
                    return false;
                },
            }
        }

        // Reuse audio channel if we already have one.
        if self.channel.is_none() {
            match AudioChannel::reserve(MP3_FRAME_SAMPLES, AudioFormat::Stereo) {
                Ok(ch) => self.channel = Some(ch),
                Err(e) => {
                    psp::dprintln!("AudioChannel reserve failed: {e}");
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

        // SAFETY: temp_path_c is a null-terminated byte string literal.
        // sceIoOpen creates/truncates the temp file for writing.
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
        // SAFETY: fd is valid; data.as_ptr() and data.len() describe a valid byte slice.
        let written = unsafe { psp::sys::sceIoWrite(fd, data.as_ptr() as *const _, data.len()) };
        // SAFETY: fd is valid and will not be used after this close.
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
            // SAFETY: self.fd is a valid open file descriptor. The write
            // target is read_buf[buf_valid..buf_valid+chunk], which is within
            // the Vec's allocated capacity (READ_BUF_SIZE).
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

        let Some(decoder) = self.decoder.as_mut() else { return };
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
                let Some(channel) = self.channel.as_ref() else { return };
                let _ = channel.output_blocking(self.hw_volume, &self.pcm_buf);
            },
            Err(e) => {
                if self.error_count == 0 {
                    psp::dprintln!("MP3 decode error: {e}");
                }
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

    /// Output raw PCM i16 samples from the video decode thread.
    ///
    /// Ensures an audio channel exists and outputs the samples directly.
    /// Uses 44100 Hz stereo (most IA videos). The PSP hardware channel
    /// is configured for 1152 samples (standard MP3 frame size).
    pub fn output_video_pcm(&mut self, pcm_i16: &[i16]) {
        use psp::audio::{AudioChannel, AudioFormat};

        // Ensure we have a hardware channel.
        if self.channel.is_none() {
            self.channel = AudioChannel::reserve(MP3_FRAME_SAMPLES, AudioFormat::Stereo).ok();
        }
        let channel = match &self.channel {
            Some(ch) => ch,
            None => return,
        };

        // Output in MP3_FRAME_SAMPLES-sized chunks (1152 stereo samples = 2304 i16s).
        let chunk_size = (MP3_FRAME_SAMPLES * 2) as usize; // stereo
        for chunk in pcm_i16.chunks(chunk_size) {
            // Pad to expected size if needed.
            if chunk.len() == chunk_size {
                let _ = channel.output_blocking(self.hw_volume, chunk);
            } else {
                // Pad with silence for the last partial chunk.
                let mut padded = vec![0i16; chunk_size];
                padded[..chunk.len()].copy_from_slice(chunk);
                let _ = channel.output_blocking(self.hw_volume, &padded);
            }
        }
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.close_file();
    }
}
