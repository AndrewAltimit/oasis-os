//! Configuration file parser for `ms0:/seplugins/oasis.ini`.
//!
//! Simple line-by-line INI parser using `sceIoOpen`/`sceIoRead` -- no serde,
//! no allocator. All config values are stored in a static struct.
//!
//! ```ini
//! # Overlay trigger button (default: NOTE)
//! trigger = note
//! # Music directory
//! music_dir = ms0:/MUSIC/
//! # Overlay opacity (0-255)
//! opacity = 180
//! # Auto-start music on game launch
//! autoplay = false
//! ```

use core::sync::atomic::{AtomicU8, Ordering};

/// Maximum path length for config strings.
const MAX_PATH: usize = 64;

/// Config file path on Memory Stick.
const CONFIG_PATH: &[u8] = b"ms0:/seplugins/oasis.ini\0";

/// Trigger button options.
#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum TriggerButton {
    /// NOTE button (0x800000) -- kernel-only, default.
    Note = 0,
    /// SCREEN button (0x400000) -- kernel-only.
    Screen = 1,
}

/// Static plugin configuration.
#[derive(Copy, Clone)]
pub struct PluginConfig {
    /// Which button triggers the overlay.
    pub trigger: TriggerButton,
    /// Music directory path (null-terminated).
    pub music_dir: [u8; MAX_PATH],
    /// Music directory path length (excluding null).
    pub music_dir_len: usize,
    /// Overlay background opacity (0-255).
    pub opacity: u8,
    /// Auto-start music playback on plugin load.
    pub autoplay: bool,
}

impl PluginConfig {
    const fn default() -> Self {
        // "ms0:/MUSIC/" as bytes
        let mut dir = [0u8; MAX_PATH];
        let src = b"ms0:/MUSIC/";
        let mut i = 0;
        while i < src.len() {
            dir[i] = src[i];
            i += 1;
        }
        Self {
            trigger: TriggerButton::Note,
            music_dir: dir,
            music_dir_len: 11,
            opacity: 180,
            autoplay: false,
        }
    }

    /// Get music directory as a byte slice (with null terminator).
    pub fn music_dir_str(&self) -> &[u8] {
        &self.music_dir[..self.music_dir_len + 1]
    }

    /// Get the trigger button mask for controller polling.
    pub fn trigger_mask(&self) -> u32 {
        match self.trigger {
            TriggerButton::Note => 0x00800000,
            TriggerButton::Screen => 0x00400000,
        }
    }
}

/// Atomic opacity (updated from config, read from hook).
static OPACITY: AtomicU8 = AtomicU8::new(180);

/// Static config storage -- written once at startup, read-only after.
static mut CONFIG: PluginConfig = PluginConfig::default();

/// Get the current plugin configuration.
///
/// # Safety
/// Safe to call after `load_config()` has returned. The config is read-only
/// after initialization.
pub fn get_config() -> PluginConfig {
    // SAFETY: CONFIG is only written in load_config() during single-threaded
    // init, then read-only afterwards.
    unsafe { CONFIG }
}

/// Get overlay opacity (atomic, safe from any thread).
pub fn get_opacity() -> u8 {
    OPACITY.load(Ordering::Relaxed)
}

/// Load and parse the configuration file. Falls back to defaults on error.
pub fn load_config() {
    let mut buf = [0u8; 512];

    // SAFETY: sceIoOpen with read-only flags, null-terminated path.
    let fd = unsafe {
        psp::sys::sceIoOpen(
            CONFIG_PATH.as_ptr(),
            psp::sys::IoOpenFlags::RD_ONLY,
            0,
        )
    };
    if fd < psp::sys::SceUid(0) {
        return; // File doesn't exist, use defaults.
    }

    // SAFETY: fd is valid, buf is on stack.
    let bytes_read =
        unsafe { psp::sys::sceIoRead(fd, buf.as_mut_ptr() as *mut _, buf.len() as u32) };
    // SAFETY: Close the file descriptor.
    unsafe {
        psp::sys::sceIoClose(fd);
    }

    if bytes_read <= 0 {
        return;
    }
    let data = &buf[..bytes_read as usize];

    // SAFETY: Single-threaded init, CONFIG not yet shared.
    unsafe {
        parse_config(data, &mut *(&raw mut CONFIG));
        OPACITY.store(CONFIG.opacity, Ordering::Relaxed);
    }
}

/// Parse INI-style config data into a `PluginConfig`.
fn parse_config(data: &[u8], config: &mut PluginConfig) {
    // Process each line
    let mut start = 0;
    while start < data.len() {
        // Find end of line
        let mut end = start;
        while end < data.len() && data[end] != b'\n' && data[end] != b'\r' {
            end += 1;
        }
        let line = &data[start..end];

        // Skip to next line
        start = end;
        while start < data.len() && (data[start] == b'\n' || data[start] == b'\r') {
            start += 1;
        }

        // Skip empty lines and comments
        let line = trim_bytes(line);
        if line.is_empty() || line[0] == b'#' {
            continue;
        }

        // Find '=' separator
        if let Some(eq_pos) = line.iter().position(|&b| b == b'=') {
            let key = trim_bytes(&line[..eq_pos]);
            let val = trim_bytes(&line[eq_pos + 1..]);

            if bytes_eq_ci(key, b"trigger") {
                if bytes_eq_ci(val, b"screen") {
                    config.trigger = TriggerButton::Screen;
                } else {
                    config.trigger = TriggerButton::Note;
                }
            } else if bytes_eq_ci(key, b"music_dir") {
                let len = val.len().min(MAX_PATH - 1);
                let mut i = 0;
                while i < len {
                    config.music_dir[i] = val[i];
                    i += 1;
                }
                config.music_dir[len] = 0;
                config.music_dir_len = len;
            } else if bytes_eq_ci(key, b"opacity") {
                if let Some(n) = parse_u8(val) {
                    config.opacity = n;
                }
            } else if bytes_eq_ci(key, b"autoplay") {
                config.autoplay =
                    bytes_eq_ci(val, b"true") || bytes_eq_ci(val, b"1") || bytes_eq_ci(val, b"yes");
            }
        }
    }
}

/// Trim leading/trailing whitespace from a byte slice.
fn trim_bytes(s: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < s.len() && s[start].is_ascii_whitespace() {
        start += 1;
    }
    let mut end = s.len();
    while end > start && s[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &s[start..end]
}

/// Case-insensitive byte comparison.
fn bytes_eq_ci(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i].to_ascii_lowercase() != b[i].to_ascii_lowercase() {
            return false;
        }
        i += 1;
    }
    true
}

/// Parse a byte slice as a u8 decimal number.
fn parse_u8(s: &[u8]) -> Option<u8> {
    if s.is_empty() {
        return None;
    }
    let mut result: u16 = 0;
    for &b in s {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result * 10 + (b - b'0') as u16;
        if result > 255 {
            return None;
        }
    }
    Some(result as u8)
}
