//! Minimal SGR (Select Graphic Rendition) escape-sequence support for
//! terminal output coloring.
//!
//! ## Scope
//!
//! This is deliberately **not** a VT100 emulator. Only foreground color
//! SGR sequences are recognized:
//!
//! - `ESC[30m`..`ESC[37m` — normal foreground colors (palette slots 0-7)
//! - `ESC[90m`..`ESC[97m` — bright foreground colors (palette slots 8-15)
//! - `ESC[39m` — reset foreground to the default color
//! - `ESC[0m` / `ESC[m` — full reset (treated as foreground reset)
//!
//! Multi-parameter sequences (`ESC[0;31m`) apply each parameter in order.
//! Any other parameter (bold, underline, background colors, 256/truecolor
//! extensions) is ignored, and any other escape sequence is stripped.
//!
//! Palette indices map into [`AnsiPalette`](crate::active_theme::AnsiPalette),
//! so themed skins control the actual rendered colors.

/// A run of text with an optional palette color.
///
/// `color` is a palette slot index (0-15); `None` means "default color"
/// (the theme's terminal output color).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SgrRun<'a> {
    /// Palette slot (0-15), or `None` for the default foreground.
    pub color: Option<u8>,
    /// The text of this run (contains no escape sequences).
    pub text: &'a str,
}

/// Quick check: does this line contain an escape character at all?
///
/// Lets render paths skip SGR parsing (and its allocations) for the
/// common plain-text case.
#[inline]
pub fn has_sgr(line: &str) -> bool {
    line.as_bytes().contains(&0x1b)
}

/// Map a single SGR parameter to a foreground palette state change.
///
/// - `Some(Some(slot))` — set foreground to palette slot 0-15
/// - `Some(None)` — reset foreground to the default color
/// - `None` — parameter is out of scope; ignore it
fn apply_sgr_param(param: &str) -> Option<Option<u8>> {
    match param.parse::<u8>() {
        Ok(0) | Ok(39) => Some(None),              // reset / default fg
        Ok(n @ 30..=37) => Some(Some(n - 30)),     // normal colors
        Ok(n @ 90..=97) => Some(Some(n - 90 + 8)), // bright colors
        _ => None,                                 // ignored (bold, bg, ...)
    }
}

/// Parse a line into colored runs.
///
/// Escape sequences are removed; the text between them is returned as
/// borrowed slices tagged with the active palette slot. Lines without
/// escapes yield a single default-colored run. Unterminated escape
/// sequences at end-of-line are dropped.
pub fn parse_runs(line: &str) -> Vec<SgrRun<'_>> {
    let mut runs = Vec::new();
    let mut color: Option<u8> = None;
    let bytes = line.as_bytes();
    let mut seg_start = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == 0x1b {
            // Emit the text segment before this escape.
            if i > seg_start {
                runs.push(SgrRun {
                    color,
                    text: &line[seg_start..i],
                });
            }
            // CSI sequence: ESC [ params 'm' (only SGR is recognized).
            if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                let mut j = i + 2;
                while j < bytes.len() && !bytes[j].is_ascii_alphabetic() {
                    j += 1;
                }
                if j < bytes.len() {
                    if bytes[j] == b'm' {
                        let params = &line[i + 2..j];
                        if params.is_empty() {
                            color = None; // ESC[m == full reset
                        } else {
                            for p in params.split(';') {
                                if let Some(new_state) = apply_sgr_param(p) {
                                    color = new_state;
                                }
                            }
                        }
                    }
                    // Skip the whole sequence (SGR or otherwise).
                    i = j + 1;
                } else {
                    // Unterminated sequence: drop the rest of the line.
                    i = bytes.len();
                }
            } else {
                // Lone ESC (or ESC + non-'['): skip the ESC byte.
                i += 1;
                if i < bytes.len() {
                    i += 1;
                }
            }
            seg_start = i;
        } else {
            i += 1;
        }
    }

    if seg_start < bytes.len() {
        runs.push(SgrRun {
            color,
            text: &line[seg_start..],
        });
    }
    if runs.is_empty() {
        runs.push(SgrRun {
            color: None,
            text: "",
        });
    }
    runs
}

/// Strip all escape sequences from a line, returning plain text.
pub fn strip_sgr(line: &str) -> String {
    if !has_sgr(line) {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len());
    for run in parse_runs(line) {
        out.push_str(run.text);
    }
    out
}

/// Wrap `text` in the SGR sequence for a normal color code (30-37) or
/// bright code (90-97), with a trailing full reset.
pub fn colorize(text: &str, sgr_code: u8) -> String {
    format!("\u{1b}[{sgr_code}m{text}\u{1b}[0m")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_line_single_run() {
        let runs = parse_runs("hello world");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].color, None);
        assert_eq!(runs[0].text, "hello world");
    }

    #[test]
    fn colored_run_and_reset() {
        let runs = parse_runs("\u{1b}[31merror\u{1b}[0m rest");
        assert_eq!(runs.len(), 2);
        assert_eq!(
            runs[0],
            SgrRun {
                color: Some(1),
                text: "error"
            }
        );
        assert_eq!(
            runs[1],
            SgrRun {
                color: None,
                text: " rest"
            }
        );
    }

    #[test]
    fn bright_colors_map_to_upper_slots() {
        let runs = parse_runs("\u{1b}[94mdir/\u{1b}[0m");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].color, Some(12));
        assert_eq!(runs[0].text, "dir/");
    }

    #[test]
    fn default_foreground_39() {
        let runs = parse_runs("\u{1b}[32mgreen\u{1b}[39mplain");
        assert_eq!(runs[0].color, Some(2));
        assert_eq!(runs[1].color, None);
        assert_eq!(runs[1].text, "plain");
    }

    #[test]
    fn multi_param_sequence() {
        let runs = parse_runs("\u{1b}[0;36mcyan");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].color, Some(6));
    }

    #[test]
    fn empty_params_is_reset() {
        let runs = parse_runs("\u{1b}[31mred\u{1b}[mplain");
        assert_eq!(runs[0].color, Some(1));
        assert_eq!(runs[1].color, None);
    }

    #[test]
    fn ignored_params_keep_state() {
        // Bold (1) and background (41) are out of scope and ignored.
        let runs = parse_runs("\u{1b}[1;32mtext");
        assert_eq!(runs[0].color, Some(2));
        let runs = parse_runs("\u{1b}[41mtext");
        assert_eq!(runs[0].color, None);
    }

    #[test]
    fn non_sgr_csi_stripped() {
        let runs = parse_runs("\u{1b}[2Jcleared");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "cleared");
        assert_eq!(runs[0].color, None);
    }

    #[test]
    fn unterminated_sequence_dropped() {
        let runs = parse_runs("text\u{1b}[31");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "text");
    }

    #[test]
    fn lone_escape_stripped() {
        let runs = parse_runs("a\u{1b}b");
        // ESC swallows the following byte (like a two-char sequence).
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "a");
    }

    #[test]
    fn empty_line_yields_empty_run() {
        let runs = parse_runs("");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "");
    }

    #[test]
    fn strip_sgr_removes_escapes() {
        assert_eq!(strip_sgr("\u{1b}[94mdir/\u{1b}[0m file"), "dir/ file");
        assert_eq!(strip_sgr("plain"), "plain");
    }

    #[test]
    fn colorize_round_trip() {
        let s = colorize("hi", 31);
        assert_eq!(s, "\u{1b}[31mhi\u{1b}[0m");
        assert_eq!(strip_sgr(&s), "hi");
        let runs = parse_runs(&s);
        assert_eq!(runs[0].color, Some(1));
    }

    #[test]
    fn has_sgr_detection() {
        assert!(has_sgr("\u{1b}[31mx"));
        assert!(!has_sgr("plain text"));
    }
}
