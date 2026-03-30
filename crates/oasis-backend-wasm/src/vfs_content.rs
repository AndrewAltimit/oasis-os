//! VFS population and helper functions for the WASM backend.

use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::AppRunner;
use oasis_core::terminal::{populate_man_pages, populate_motd, populate_profile};
use oasis_core::terminal_sdi;
use oasis_core::vfs::{MemoryVfs, Vfs};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Truncate output lines to `MAX_OUTPUT_LINES`.
pub(crate) fn trim_output(output_lines: &mut Vec<String>) {
    while output_lines.len() > terminal_sdi::MAX_OUTPUT_LINES {
        output_lines.remove(0);
    }
}

// ---------------------------------------------------------------------------
// VFS population
// ---------------------------------------------------------------------------

/// Find a TV Guide runner in either the full-screen or windowed runners.
pub(crate) fn find_tv_guide_runner_wasm<'a>(
    app_runner: &'a mut Option<AppRunner>,
    open_runners: &'a mut [(String, AppRunner)],
) -> Option<&'a mut AppRunner> {
    if let Some(ref mut runner) = *app_runner
        && runner.title == "TV Guide"
    {
        return Some(runner);
    }
    open_runners
        .iter_mut()
        .map(|(_, runner)| runner)
        .find(|runner| runner.title == "TV Guide")
}

/// Compute the TV video capture dimensions `(x, y, w, h)`.
///
/// Captures at full screen resolution so the video looks sharp in both
/// PIP and expanded (fullscreen) modes. The backend handles scaling
/// when blitting to the smaller PIP area.
pub(crate) fn tv_preview_rect(at: &ActiveTheme) -> (i32, i32, u32, u32) {
    let usable_h = at
        .screen_h
        .saturating_sub(at.statusbar_height + at.bottombar_height);
    (0, at.statusbar_height as i32, at.screen_w, usable_h)
}

/// Check if a runner's pending request is a TV Guide tune_url (should not be
/// consumed by the generic VFS handler).
pub(crate) fn is_tv_tune_request_wasm(runner: &AppRunner) -> bool {
    runner.peek_pending_request().is_some_and(|req| {
        req.0 == oasis_core::apps::tv_guide::TV_REQUEST_PATH && req.1.starts_with("tune_url ")
    })
}

/// Populate the WASM VFS with demo content.
pub(crate) fn populate_wasm_vfs(vfs: &mut MemoryVfs) {
    // Core directory structure.
    let _ = vfs.mkdir("/home");
    let _ = vfs.mkdir("/home/user");
    let _ = vfs.mkdir("/etc");
    let _ = vfs.mkdir("/tmp");
    let _ = vfs.mkdir("/var");
    let _ = vfs.mkdir("/var/oasis");
    let _ = vfs.mkdir("/var/log");

    // Use the terminal's built-in content populators.
    populate_motd(vfs);
    populate_profile(vfs);
    populate_man_pages(vfs);

    // System metadata.
    let _ = vfs.write("/etc/hostname", b"oasis-wasm");
    let _ = vfs.write("/etc/version", b"1.0.0-wasm");

    // Demo user files.
    let _ = vfs.write(
        "/home/user/readme.txt",
        b"OASIS_OS is running in your browser!\n\
          \n\
          This is a retro operating system shell originally built for the PSP.\n\
          It now runs on desktop (SDL2), Unreal Engine 5, and WebAssembly.\n\
          \n\
          Try these commands:\n\
            help        Show available commands\n\
            ls          List files\n\
            cat <file>  Read a file\n\
            skin list   Show available skins\n\
            fortune     Random fortune\n\
            tutorial    Interactive terminal tutorial\n\
            man ls      Manual page for a command\n",
    );

    let _ = vfs.write(
        "/home/user/notes.txt",
        b"Shopping list:\n- Milk\n- Bread\n- Memory Stick PRO Duo\n",
    );

    // Demo app directories (discovered by the dashboard).
    // Names must match the title strings in AppRunner::init_content().
    let _ = vfs.mkdir("/apps");
    let _ = vfs.mkdir("/apps/File Manager");
    let _ = vfs.mkdir("/apps/Settings");
    let _ = vfs.mkdir("/apps/Browser");
    let _ = vfs.mkdir("/apps/Music Player");
    let _ = vfs.mkdir("/apps/Terminal");
    let _ = vfs.mkdir("/apps/TV Guide");

    // TV Guide configuration.
    let _ = vfs.mkdir("/etc/tv");
    let _ = vfs.mkdir("/var/tv");
    let _ = vfs.mkdir("/var/tv/cache");
    let _ = vfs.write(
        "/etc/tv/channels.toml",
        oasis_core::apps::tv_guide::channel::DEFAULT_CHANNELS_TOML.as_bytes(),
    );

    // Browser home page content.
    let _ = vfs.mkdir("/sites");
    let _ = vfs.mkdir("/sites/home");
    let _ = vfs.write(
        "/sites/home/index.html",
        br#"<html><head><title>OASIS Home</title>
<style>
body { color: #e0e0e0; background-color: #1a1a2e; }
h1 { color: #64c8ff; }
h2 { color: #80d0a0; }
a { color: #64c8ff; }
code { background-color: rgba(100,200,255,30); }
pre { background-color: rgba(100,200,255,15); border: 1px solid rgba(100,200,255,30); }
blockquote { border-left-color: #64c8ff; color: #a0a0c0; }
table { border-collapse: collapse; }
th { background-color: rgba(100,200,255,20); border: 1px solid rgba(255,255,255,30); }
td { border: 1px solid rgba(255,255,255,20); }
</style>
</head><body>
<h1>Welcome to OASIS Browser</h1>
<p>A lightweight <strong>HTML/CSS</strong> rendering engine for
<em>OASIS_OS</em>. Supports block, inline, flex, and table layout.</p>

<h2>Features</h2>
<ul>
<li>CSS cascade with <code>specificity</code></li>
<li>Block, inline, flex, and table layout</li>
<li>Text wrapping and decoration</li>
<li>Smooth scrolling with mouse wheel</li>
</ul>

<h2>Shortcuts</h2>
<table>
<tr><th>Key</th><th>Action</th></tr>
<tr><td>Tab</td><td>Focus URL bar</td></tr>
<tr><td>Left/Right</td><td>Navigate links</td></tr>
<tr><td>Up/Down</td><td>Scroll page</td></tr>
</table>

<blockquote>Built from scratch in Rust (2026), inspired by PSP homebrew shells like PSIX.</blockquote>

<h2>Links</h2>
<ol>
<li><a href="/sites/home/about.html">About OASIS Browser</a></li>
<li><a href="/sites/home/features.html">CSS Feature Test</a></li>
</ol>
</body></html>"#,
    );
    let _ = vfs.write(
        "/sites/home/about.html",
        br#"<html><head><title>About OASIS Browser</title>
<style>
body { color: #e0e0e0; background-color: #1a1a2e; }
h1 { color: #64c8ff; }
a { color: #64c8ff; }
</style>
</head><body>
<h1>About OASIS Browser</h1>
<p>A lightweight HTML/CSS engine for embedded systems:</p>
<ul>
<li><strong>HTML</strong> -- WHATWG tokenizer, 70+ tags</li>
<li><strong>CSS</strong> -- cascade, specificity, media queries</li>
<li><strong>Layout</strong> -- block, inline, flex, table, float</li>
<li><strong>Gemini</strong> -- lightweight text protocol</li>
</ul>
<p><a href="/sites/home/index.html">Back to home</a></p>
</body></html>"#,
    );
    let _ = vfs.write(
        "/sites/home/features.html",
        br#"<html><head><title>CSS Features</title>
<style>
body { color: #e0e0e0; background-color: #1a1a2e; }
h1 { color: #64c8ff; }
h2 { color: #80d0a0; font-size: 1.2em; }
a { color: #64c8ff; }
</style>
</head><body>
<h1>CSS Feature Test</h1>
<h2>Text Formatting</h2>
<p><strong>Bold</strong>, <em>italic</em>, <u>underline</u>,
<s>strikethrough</s>, <code>inline code</code>,
<mark>highlighted</mark>, <small>small</small>.</p>
<h2>Blockquote</h2>
<blockquote>Blockquote with left border.</blockquote>
<h2>Ordered List</h2>
<ol><li>First</li><li>Second</li><li>Third</li></ol>
<h2>Preformatted</h2>
<pre>fn main() {
    println!("Hello!");
}</pre>
<p><a href="/sites/home/index.html">Back to home</a></p>
</body></html>"#,
    );

    // Demo startup script.
    let _ = vfs.write(
        "/home/user/startup.sh",
        b"# OASIS_OS startup script\necho Welcome back!\nls /apps\n",
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    // -----------------------------------------------------------------------
    // trim_output
    // -----------------------------------------------------------------------

    #[test]
    fn trim_output_below_limit_unchanged() {
        let mut lines: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();
        trim_output(&mut lines);
        assert_eq!(lines.len(), 10);
    }

    #[test]
    fn trim_output_at_limit_unchanged() {
        let mut lines: Vec<String> = (0..terminal_sdi::MAX_OUTPUT_LINES)
            .map(|i| format!("line {i}"))
            .collect();
        trim_output(&mut lines);
        assert_eq!(lines.len(), terminal_sdi::MAX_OUTPUT_LINES);
    }

    #[test]
    fn trim_output_over_limit_removes_oldest() {
        let count = terminal_sdi::MAX_OUTPUT_LINES + 5;
        let mut lines: Vec<String> = (0..count).map(|i| format!("line {i}")).collect();
        trim_output(&mut lines);
        assert_eq!(lines.len(), terminal_sdi::MAX_OUTPUT_LINES);
        // The first 5 lines should have been removed.
        assert_eq!(lines[0], "line 5");
    }

    #[test]
    fn trim_output_empty_vec() {
        let mut lines: Vec<String> = Vec::new();
        trim_output(&mut lines);
        assert!(lines.is_empty());
    }

    #[test]
    fn trim_output_one_over_limit() {
        let count = terminal_sdi::MAX_OUTPUT_LINES + 1;
        let mut lines: Vec<String> = (0..count).map(|i| format!("line {i}")).collect();
        trim_output(&mut lines);
        assert_eq!(lines.len(), terminal_sdi::MAX_OUTPUT_LINES);
        assert_eq!(lines[0], "line 1");
    }

    // -----------------------------------------------------------------------
    // tv_preview_rect
    // -----------------------------------------------------------------------

    fn make_theme(screen_w: u32, screen_h: u32, sb_h: u32, bb_h: u32) -> ActiveTheme {
        let mut at = ActiveTheme::default();
        at.screen_w = screen_w;
        at.screen_h = screen_h;
        at.statusbar_height = sb_h;
        at.bottombar_height = bb_h;
        at
    }

    #[test]
    fn tv_preview_rect_default_psp() {
        let at = make_theme(480, 272, 16, 16);
        let (x, y, w, h) = tv_preview_rect(&at);
        assert_eq!(x, 0);
        assert_eq!(y, 16);
        assert_eq!(w, 480);
        assert_eq!(h, 272 - 16 - 16);
    }

    #[test]
    fn tv_preview_rect_no_bars() {
        let at = make_theme(800, 600, 0, 0);
        let (x, y, w, h) = tv_preview_rect(&at);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
        assert_eq!(w, 800);
        assert_eq!(h, 600);
    }

    #[test]
    fn tv_preview_rect_large_bars() {
        let at = make_theme(1024, 768, 30, 40);
        let (x, y, w, h) = tv_preview_rect(&at);
        assert_eq!(x, 0);
        assert_eq!(y, 30);
        assert_eq!(w, 1024);
        assert_eq!(h, 768 - 30 - 40);
    }

    #[test]
    fn tv_preview_rect_bars_exceed_height() {
        // Degenerate case: bars taller than the screen.
        let at = make_theme(480, 20, 15, 15);
        let (_, _, _, h) = tv_preview_rect(&at);
        // saturating_sub prevents underflow.
        assert_eq!(h, 0);
    }

    // -----------------------------------------------------------------------
    // populate_wasm_vfs (smoke test)
    // -----------------------------------------------------------------------

    #[test]
    fn populate_creates_expected_directories_and_files() {
        let mut vfs = MemoryVfs::new();
        populate_wasm_vfs(&mut vfs);

        // Check key files exist.
        assert!(vfs.read("/etc/hostname").is_ok());
        assert!(vfs.read("/etc/version").is_ok());
        assert!(vfs.read("/home/user/readme.txt").is_ok());
        assert!(vfs.read("/home/user/notes.txt").is_ok());
        assert!(vfs.read("/home/user/startup.sh").is_ok());

        // Check hostname content.
        let hostname = vfs.read("/etc/hostname").unwrap();
        assert_eq!(hostname, b"oasis-wasm");

        // Check version content.
        let version = vfs.read("/etc/version").unwrap();
        assert_eq!(version, b"1.0.0-wasm");
    }

    #[test]
    fn populate_creates_app_directories() {
        let mut vfs = MemoryVfs::new();
        populate_wasm_vfs(&mut vfs);

        // The VFS should have app directories (readdir returns entries).
        let entries = vfs.readdir("/apps").unwrap_or_default();
        assert!(!entries.is_empty());
    }

    #[test]
    fn populate_creates_browser_pages() {
        let mut vfs = MemoryVfs::new();
        populate_wasm_vfs(&mut vfs);

        assert!(vfs.read("/sites/home/index.html").is_ok());
        assert!(vfs.read("/sites/home/about.html").is_ok());
        assert!(vfs.read("/sites/home/features.html").is_ok());
    }

    #[test]
    fn populate_creates_tv_config() {
        let mut vfs = MemoryVfs::new();
        populate_wasm_vfs(&mut vfs);

        assert!(vfs.read("/etc/tv/channels.toml").is_ok());
    }
}
