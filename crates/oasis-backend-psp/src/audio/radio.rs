//! RadioStreamer — internet radio via raw TCP socket + ICY protocol.

use psp::audio::{AudioChannel, AudioFormat};
use psp::audiocodec::{AudiocodecDecoder, CodecType};
use psp::mp3::find_sync;

use super::player::AudioPlayer;
use super::{load_av_modules_once, MP3_FRAME_SAMPLES};

/// Internet radio streamer that reads MP3 data from a connected TCP socket.
///
/// Handles ICY metadata demuxing inline: every `icy_metaint` bytes of audio
/// data, a metadata block appears (1-byte length * 16 = metadata bytes).
/// Decoded titles are buffered for the main thread to poll.
///
/// Shares the `AudiocodecDecoder` and `AudioChannel` from `AudioPlayer`
/// (only one active at a time).
pub(crate) struct RadioStreamer {
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
    /// Consecutive recv failures (negative returns).
    recv_fail_count: u32,
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
            recv_fail_count: 0,
        }
    }

    /// Seed the read buffer with initial data received during header parsing.
    pub fn seed_buffer(&mut self, data: &[u8]) {
        let n = data.len().min(Self::BUF_SIZE);
        // SAFETY: Manual byte copy to avoid LLVM memcpy recursion on MIPS.
        let ptr = self.read_buf.as_mut_ptr();
        for i in 0..n {
            unsafe { *ptr.add(i) = data[i] };
        }
        self.buf_valid = n;
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
                // Clamp to prevent overflow if recv returns more than asked.
                self.buf_valid = (self.buf_valid + n as usize).min(Self::BUF_SIZE);
                self.recv_fail_count = 0;
            } else if n == 0 {
                // EOF: server closed the connection.
                self.error_count = 201;
            } else {
                // Negative: check errno to distinguish EAGAIN from fatal.
                // SAFETY: sceNetInetGetErrno returns the last socket error.
                let errno = unsafe { psp::sys::sceNetInetGetErrno() };
                // EAGAIN = 0x0B (11) on PSP: no data available yet.
                if errno == 0x0B || errno == 35 {
                    self.recv_fail_count += 1;
                    if self.recv_fail_count > 3000 {
                        self.error_count = 201;
                    }
                } else {
                    // Fatal socket error (ECONNRESET, ENOTCONN, etc.).
                    self.error_count = 201;
                }
            }
        } else {
            // ICY metadata enabled: receive into temp buffer, demux.
            let mut tmp = [0u8; 4096];
            let chunk = room.min(tmp.len());
            // SAFETY: Non-blocking recv into temp buffer.
            let n = unsafe {
                psp::sys::sceNetInetRecv(self.socket_fd, tmp.as_mut_ptr() as *mut _, chunk, 0x80)
            };
            if n == 0 {
                // EOF: server closed the connection.
                self.error_count = 201;
            } else if n < 0 {
                // SAFETY: sceNetInetGetErrno returns the last socket error.
                let errno = unsafe { psp::sys::sceNetInetGetErrno() };
                if errno == 0x0B || errno == 35 {
                    // EAGAIN: no data available yet.
                    self.recv_fail_count += 1;
                    if self.recv_fail_count > 3000 {
                        self.error_count = 201;
                    }
                } else {
                    // Fatal socket error.
                    self.error_count = 201;
                }
            } else {
                self.recv_fail_count = 0;
                let received = n as usize;
                let mut i = 0;
                while i < received {
                    if self.icy_in_meta {
                        if self.icy_meta_remaining == 0 {
                            // Read the metadata length byte.
                            let meta_len = tmp[i] as usize * 16;
                            i += 1;
                            if meta_len == 0 {
                                // Empty metadata block.
                                self.icy_in_meta = false;
                                continue;
                            }
                            self.icy_meta_remaining = meta_len;
                            self.icy_meta_buf.clear();
                        }
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
                            // Enter metadata mode; the length byte will
                            // be consumed by the meta branch (handles
                            // both inline and cross-recv boundaries).
                            self.icy_audio_count = 0;
                            self.icy_in_meta = true;
                            self.icy_meta_remaining = 0;
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
                Err(e) => {
                    psp::dprintln!("Radio: AudiocodecDecoder init failed: {e}");
                    return;
                },
            }
        }
        if player.channel.is_none() {
            match AudioChannel::reserve(MP3_FRAME_SAMPLES, AudioFormat::Stereo) {
                Ok(ch) => player.channel = Some(ch),
                Err(e) => {
                    psp::dprintln!("Radio: AudioChannel reserve failed: {e}");
                    return;
                },
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

        let Some(decoder) = player.decoder.as_mut() else { return };
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
                let Some(channel) = player.channel.as_ref() else { return };
                let _ = channel.output_blocking(self.hw_volume, &player.pcm_buf);
            },
            Err(e) => {
                if self.error_count == 0 {
                    psp::dprintln!("Radio: MP3 decode error: {e}");
                }
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
