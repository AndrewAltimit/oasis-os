//! Desktop mode rendering (windowed WM with floating windows).
//!
//! Draws all registered windows via the window manager's clipped render
//! callback, dispatching each window ID to its content drawing function.

use oasis_backend_psp::{PspBackend, SdiRegistry, StatusBarInfo, SystemInfo, WindowManager};

use crate::app_states::*;
use crate::desktop;

/// Render desktop mode: draw all WM windows with their content.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_desktop(
    backend: &mut PspBackend,
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    config: &psp::config::Config,
    status: &StatusBarInfo,
    sysinfo: &SystemInfo,
    fps: f32,
    usb_active: bool,
    free_kb: i32,
    max_blk_kb: i32,
    term: &TerminalState,
    fm: &FileManagerState,
    pv: &PhotoViewerState,
    mp: &MusicPlayerState,
    audio: &oasis_backend_psp::AudioHandle,
    br: &mut BrowserState,
    tv: &TvGuideState,
) {
    let settings_clock = config.get_i32("clock_mhz").unwrap_or(333);
    let settings_bus = config.get_i32("bus_mhz").unwrap_or(166);
    let current_vol = backend.volatile_mem_info();

    // Grab browser info before the closure borrows everything.
    let br_url = br.url().to_string();
    let br_status = br.status_msg.clone();

    backend.force_bitmap_font = true;
    let _ = wm.draw_with_clips_noalloc(
        sdi,
        backend,
        |window_id, cx, cy, cw, ch, be| match window_id {
            "terminal" => {
                desktop::draw_terminal_windowed(&term.lines, &term.input, cx, cy, cw, ch, be)
            },
            "filemgr" => desktop::draw_filemgr_windowed(
                &fm.left.path,
                &fm.left.entries,
                fm.left.selected,
                fm.left.scroll,
                &fm.right.path,
                &fm.right.entries,
                fm.right.selected,
                fm.right.scroll,
                fm.active_panel,
                cx,
                cy,
                cw,
                ch,
                be,
            ),
            "photos" => desktop::draw_photos_windowed(
                pv.tex, pv.img_w, pv.img_h, pv.viewing, cx, cy, cw, ch, be,
            ),
            "music" => desktop::draw_music_windowed(&mp.file_name, audio, cx, cy, cw, ch, be),
            "settings" => desktop::draw_settings_windowed(
                settings_clock,
                settings_bus,
                current_vol,
                cx,
                cy,
                cw,
                ch,
                be,
            ),
            "network" => desktop::draw_network_windowed(status, cx, cy, cw, ch, be),
            "sysmon" => desktop::draw_sysmon_windowed(
                status,
                sysinfo,
                fps,
                free_kb,
                max_blk_kb,
                current_vol,
                usb_active,
                cx,
                cy,
                cw,
                ch,
                be,
            ),
            "browser" => {
                if let Some(ref mut w) = br.widget {
                    // Resize browser viewport to fit the window content area.
                    w.set_window(cx, cy, cw, ch);
                    let _ = w.paint(be);
                    Ok(())
                } else {
                    desktop::draw_browser_windowed_with_url(
                        &br_url, &br_status, cx, cy, cw, ch, be,
                    )
                }
            },
            "packages" => desktop::draw_packages_windowed(cx, cy, cw, ch, be),
            "radio" => desktop::draw_radio_windowed(audio, cx, cy, cw, ch, be),
            "tvguide" => desktop::draw_tvguide_windowed(
                &tv.channels,
                &tv.catalogs,
                tv.selected,
                tv.scroll,
                &tv.now_playing,
                tv.tuned.is_some(),
                tv.downloading,
                tv.download_progress,
                &tv.error_msg,
                tv.preview_tex,
                cx, cy, cw, ch, be,
            ),
            _ => Ok(()),
        },
    );
    backend.force_bitmap_font = false;
}
