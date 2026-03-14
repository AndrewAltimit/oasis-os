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
    /// Initial radio station index (0-7).
    pub radio_station: u8,
    /// Start in radio mode on plugin load.
    pub radio_mode: bool,
    /// Video directory path (null-terminated).
    pub video_dir: [u8; MAX_PATH],
    /// Video directory path length (excluding null).
    pub video_dir_len: usize,
    /// Enable PIP video on plugin load.
    pub pip_enabled: bool,
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
        // "ms0:/VIDEO/" as bytes
        let mut vdir = [0u8; MAX_PATH];
        let vsrc = b"ms0:/VIDEO/";
        let mut j = 0;
        while j < vsrc.len() {
            vdir[j] = vsrc[j];
            j += 1;
        }
        Self {
            trigger: TriggerButton::Note,
            music_dir: dir,
            music_dir_len: 11,
            opacity: 180,
            autoplay: false,
            radio_station: 0,
            radio_mode: false,
            video_dir: vdir,
            video_dir_len: 11,
            pip_enabled: false,
        }
    }

    /// Get music directory as a byte slice (with null terminator).
    #[allow(dead_code)]
    pub fn music_dir_str(&self) -> &[u8] {
        &self.music_dir[..self.music_dir_len + 1]
    }

    /// Get video directory as a byte slice (with null terminator).
    pub fn video_dir_str(&self) -> &[u8] {
        &self.video_dir[..self.video_dir_len + 1]
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
    let fd =
        unsafe { psp::sys::sceIoOpen(CONFIG_PATH.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0) };
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
            } else if bytes_eq_ci(key, b"radio_station") {
                if let Some(n) = parse_u8(val) {
                    if n < 8 {
                        config.radio_station = n;
                    }
                }
            } else if bytes_eq_ci(key, b"radio_mode") {
                config.radio_mode =
                    bytes_eq_ci(val, b"true") || bytes_eq_ci(val, b"1") || bytes_eq_ci(val, b"yes");
            } else if bytes_eq_ci(key, b"video_dir") {
                let len = val.len().min(MAX_PATH - 1);
                let mut i = 0;
                while i < len {
                    config.video_dir[i] = val[i];
                    i += 1;
                }
                config.video_dir[len] = 0;
                config.video_dir_len = len;
            } else if bytes_eq_ci(key, b"pip_enabled") {
                config.pip_enabled =
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

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // trim_bytes
    // -----------------------------------------------------------------------

    #[test]
    fn trim_bytes_no_whitespace() {
        assert_eq!(trim_bytes(b"hello"), b"hello");
    }

    #[test]
    fn trim_bytes_leading() {
        assert_eq!(trim_bytes(b"  hello"), b"hello");
    }

    #[test]
    fn trim_bytes_trailing() {
        assert_eq!(trim_bytes(b"hello  "), b"hello");
    }

    #[test]
    fn trim_bytes_both() {
        assert_eq!(trim_bytes(b"  hello  "), b"hello");
    }

    #[test]
    fn trim_bytes_only_whitespace() {
        assert_eq!(trim_bytes(b"   "), b"");
    }

    #[test]
    fn trim_bytes_empty() {
        assert_eq!(trim_bytes(b""), b"");
    }

    #[test]
    fn trim_bytes_tabs_and_newlines() {
        assert_eq!(trim_bytes(b"\t hello \n"), b"hello");
    }

    // -----------------------------------------------------------------------
    // bytes_eq_ci
    // -----------------------------------------------------------------------

    #[test]
    fn bytes_eq_ci_same() {
        assert!(bytes_eq_ci(b"trigger", b"trigger"));
    }

    #[test]
    fn bytes_eq_ci_mixed_case() {
        assert!(bytes_eq_ci(b"Trigger", b"trigger"));
        assert!(bytes_eq_ci(b"TRIGGER", b"trigger"));
        assert!(bytes_eq_ci(b"tRiGgEr", b"TrIgGeR"));
    }

    #[test]
    fn bytes_eq_ci_different_lengths() {
        assert!(!bytes_eq_ci(b"trigger", b"trigg"));
        assert!(!bytes_eq_ci(b"tri", b"trigger"));
    }

    #[test]
    fn bytes_eq_ci_different_content() {
        assert!(!bytes_eq_ci(b"trigger", b"opacity"));
    }

    #[test]
    fn bytes_eq_ci_empty() {
        assert!(bytes_eq_ci(b"", b""));
    }

    // -----------------------------------------------------------------------
    // parse_u8
    // -----------------------------------------------------------------------

    #[test]
    fn parse_u8_zero() {
        assert_eq!(parse_u8(b"0"), Some(0));
    }

    #[test]
    fn parse_u8_typical() {
        assert_eq!(parse_u8(b"180"), Some(180));
    }

    #[test]
    fn parse_u8_max() {
        assert_eq!(parse_u8(b"255"), Some(255));
    }

    #[test]
    fn parse_u8_overflow() {
        assert_eq!(parse_u8(b"256"), None);
        assert_eq!(parse_u8(b"999"), None);
    }

    #[test]
    fn parse_u8_non_digit() {
        assert_eq!(parse_u8(b"12a"), None);
        assert_eq!(parse_u8(b"abc"), None);
    }

    #[test]
    fn parse_u8_empty() {
        assert_eq!(parse_u8(b""), None);
    }

    #[test]
    fn parse_u8_leading_zeros() {
        assert_eq!(parse_u8(b"007"), Some(7));
        assert_eq!(parse_u8(b"00"), Some(0));
    }

    // -----------------------------------------------------------------------
    // PluginConfig defaults
    // -----------------------------------------------------------------------

    #[test]
    fn config_default_values() {
        let cfg = PluginConfig::default();
        assert_eq!(cfg.trigger, TriggerButton::Note);
        assert_eq!(cfg.opacity, 180);
        assert!(!cfg.autoplay);
        assert_eq!(cfg.radio_station, 0);
        assert!(!cfg.radio_mode);
        assert!(!cfg.pip_enabled);
        assert_eq!(cfg.music_dir_len, 11);
        assert_eq!(&cfg.music_dir[..11], b"ms0:/MUSIC/");
        assert_eq!(cfg.video_dir_len, 11);
        assert_eq!(&cfg.video_dir[..11], b"ms0:/VIDEO/");
    }

    #[test]
    fn config_music_dir_str() {
        let cfg = PluginConfig::default();
        // music_dir_str returns len+1 bytes (includes null terminator)
        let s = cfg.music_dir_str();
        assert_eq!(s.len(), 12);
        assert_eq!(&s[..11], b"ms0:/MUSIC/");
        assert_eq!(s[11], 0);
    }

    #[test]
    fn config_video_dir_str() {
        let cfg = PluginConfig::default();
        let s = cfg.video_dir_str();
        assert_eq!(s.len(), 12);
        assert_eq!(&s[..11], b"ms0:/VIDEO/");
        assert_eq!(s[11], 0);
    }

    #[test]
    fn config_trigger_mask_note() {
        let cfg = PluginConfig::default();
        assert_eq!(cfg.trigger_mask(), 0x00800000);
    }

    #[test]
    fn config_trigger_mask_screen() {
        let mut cfg = PluginConfig::default();
        cfg.trigger = TriggerButton::Screen;
        assert_eq!(cfg.trigger_mask(), 0x00400000);
    }

    // -----------------------------------------------------------------------
    // parse_config
    // -----------------------------------------------------------------------

    #[test]
    fn parse_config_trigger_note() {
        let mut cfg = PluginConfig::default();
        parse_config(b"trigger = note\n", &mut cfg);
        assert_eq!(cfg.trigger, TriggerButton::Note);
    }

    #[test]
    fn parse_config_trigger_screen() {
        let mut cfg = PluginConfig::default();
        parse_config(b"trigger = screen\n", &mut cfg);
        assert_eq!(cfg.trigger, TriggerButton::Screen);
    }

    #[test]
    fn parse_config_trigger_case_insensitive() {
        let mut cfg = PluginConfig::default();
        parse_config(b"TRIGGER = SCREEN\n", &mut cfg);
        assert_eq!(cfg.trigger, TriggerButton::Screen);
    }

    #[test]
    fn parse_config_opacity() {
        let mut cfg = PluginConfig::default();
        parse_config(b"opacity = 200\n", &mut cfg);
        assert_eq!(cfg.opacity, 200);
    }

    #[test]
    fn parse_config_opacity_invalid() {
        let mut cfg = PluginConfig::default();
        parse_config(b"opacity = abc\n", &mut cfg);
        // Should keep default
        assert_eq!(cfg.opacity, 180);
    }

    #[test]
    fn parse_config_autoplay_true() {
        let mut cfg = PluginConfig::default();
        parse_config(b"autoplay = true\n", &mut cfg);
        assert!(cfg.autoplay);
    }

    #[test]
    fn parse_config_autoplay_yes() {
        let mut cfg = PluginConfig::default();
        parse_config(b"autoplay = yes\n", &mut cfg);
        assert!(cfg.autoplay);
    }

    #[test]
    fn parse_config_autoplay_one() {
        let mut cfg = PluginConfig::default();
        parse_config(b"autoplay = 1\n", &mut cfg);
        assert!(cfg.autoplay);
    }

    #[test]
    fn parse_config_autoplay_false() {
        let mut cfg = PluginConfig::default();
        parse_config(b"autoplay = false\n", &mut cfg);
        assert!(!cfg.autoplay);
    }

    #[test]
    fn parse_config_music_dir() {
        let mut cfg = PluginConfig::default();
        parse_config(b"music_dir = ms0:/MP3/\n", &mut cfg);
        assert_eq!(cfg.music_dir_len, 10);
        assert_eq!(&cfg.music_dir[..10], b"ms0:/MP3/\0"[..10].as_ref());
    }

    #[test]
    fn parse_config_video_dir() {
        let mut cfg = PluginConfig::default();
        parse_config(b"video_dir = ms0:/MOVIES/\n", &mut cfg);
        assert_eq!(cfg.video_dir_len, 12);
        assert_eq!(&cfg.video_dir[..12], b"ms0:/MOVIES/");
    }

    #[test]
    fn parse_config_radio_station() {
        let mut cfg = PluginConfig::default();
        parse_config(b"radio_station = 5\n", &mut cfg);
        assert_eq!(cfg.radio_station, 5);
    }

    #[test]
    fn parse_config_radio_station_out_of_range() {
        let mut cfg = PluginConfig::default();
        parse_config(b"radio_station = 8\n", &mut cfg);
        // Should keep default (0) because 8 >= 8
        assert_eq!(cfg.radio_station, 0);
    }

    #[test]
    fn parse_config_radio_mode() {
        let mut cfg = PluginConfig::default();
        parse_config(b"radio_mode = true\n", &mut cfg);
        assert!(cfg.radio_mode);
    }

    #[test]
    fn parse_config_pip_enabled() {
        let mut cfg = PluginConfig::default();
        parse_config(b"pip_enabled = yes\n", &mut cfg);
        assert!(cfg.pip_enabled);
    }

    #[test]
    fn parse_config_comments_skipped() {
        let mut cfg = PluginConfig::default();
        parse_config(b"# this is a comment\nopacity = 42\n", &mut cfg);
        assert_eq!(cfg.opacity, 42);
    }

    #[test]
    fn parse_config_empty_lines_skipped() {
        let mut cfg = PluginConfig::default();
        parse_config(b"\n\nopacity = 99\n\n", &mut cfg);
        assert_eq!(cfg.opacity, 99);
    }

    #[test]
    fn parse_config_crlf_line_endings() {
        let mut cfg = PluginConfig::default();
        parse_config(b"opacity = 50\r\nautoplay = true\r\n", &mut cfg);
        assert_eq!(cfg.opacity, 50);
        assert!(cfg.autoplay);
    }

    #[test]
    fn parse_config_multiple_options() {
        let mut cfg = PluginConfig::default();
        let ini = b"trigger = screen\nopacity = 100\nautoplay = true\n\
                     radio_station = 3\nradio_mode = yes\npip_enabled = 1\n";
        parse_config(ini, &mut cfg);
        assert_eq!(cfg.trigger, TriggerButton::Screen);
        assert_eq!(cfg.opacity, 100);
        assert!(cfg.autoplay);
        assert_eq!(cfg.radio_station, 3);
        assert!(cfg.radio_mode);
        assert!(cfg.pip_enabled);
    }

    #[test]
    fn parse_config_whitespace_around_equals() {
        let mut cfg = PluginConfig::default();
        parse_config(b"  opacity  =  42  \n", &mut cfg);
        assert_eq!(cfg.opacity, 42);
    }

    #[test]
    fn parse_config_unknown_key_ignored() {
        let mut cfg = PluginConfig::default();
        parse_config(b"unknown_key = whatever\nopacity = 77\n", &mut cfg);
        assert_eq!(cfg.opacity, 77);
    }

    #[test]
    fn parse_config_no_equals_sign() {
        let mut cfg = PluginConfig::default();
        // Line without '=' should be silently ignored
        parse_config(b"this has no equals\nopacity = 33\n", &mut cfg);
        assert_eq!(cfg.opacity, 33);
    }

    #[test]
    fn parse_config_empty_input() {
        let mut cfg = PluginConfig::default();
        parse_config(b"", &mut cfg);
        // All defaults preserved
        assert_eq!(cfg.opacity, 180);
        assert_eq!(cfg.trigger, TriggerButton::Note);
    }

    #[test]
    fn parse_config_music_dir_long_truncated() {
        let mut cfg = PluginConfig::default();
        // Create a path longer than MAX_PATH - 1 (63 chars)
        let mut long_ini = b"music_dir = ".to_vec();
        for _ in 0..70 {
            long_ini.push(b'x');
        }
        long_ini.push(b'\n');
        parse_config(&long_ini, &mut cfg);
        // Should be truncated to MAX_PATH - 1 = 63
        assert_eq!(cfg.music_dir_len, 63);
    }
}
