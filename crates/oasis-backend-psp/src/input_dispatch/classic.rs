//! Input dispatch for Classic mode views.
//!
//! # Pattern note (D4)
//!
//! This module shares a structural pattern with
//! `oasis-backend-wasm/src/input_dispatch.rs`: both dispatch
//! `InputEvent` variants through a mode/view state machine (Dashboard,
//! Terminal, app views).  However, the concrete types diverge completely
//! (PSP takes 20+ arguments with PSP-specific hardware types; WASM uses
//! `&mut self` on `OasisWasm`), so extracting a shared trait or generic
//! dispatcher would add abstraction without reducing code.  If a third
//! backend appears with the same pattern, consider a shared
//! `InputDispatcher` trait in `oasis-types`.

use oasis_backend_psp::{
    AudioCmd, AudioHandle, Button, InputEvent, IoCmd, PspBackend, SCREEN_HEIGHT, SCREEN_WIDTH,
    SdiRegistry, SfxId, Trigger, WindowManager,
};
use oasis_backend_psp::threading::IoHandle;

use oasis_core::active_theme::ActiveTheme;
use oasis_core::dashboard::DashboardState;
use oasis_core::skin::SkinFeatures;

use crate::app_states::{
    BrowserState, FileManagerState, MusicPlayerState, PhotoViewerState, RadioState, TerminalState,
    TvGuideState,
};
use crate::skins;
use crate::theme::*;
use crate::types::*;

use super::DispatchResult;
use super::helpers::{dispatch_dashboard_confirm, dispatch_terminal_confirm, dispatch_tv_confirm};

/// Handle a single input event in Classic mode.
///
/// Returns `DispatchResult` to tell the caller what to do next.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_classic(
    event: &InputEvent,
    backend: &mut PspBackend,
    app_mode: &mut AppMode,
    classic_view: &mut ClassicView,
    dashboard: &mut DashboardState,
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    audio: &AudioHandle,
    io: &IoHandle,
    term: &mut TerminalState,
    fm: &mut FileManagerState,
    pv: &mut PhotoViewerState,
    mp: &mut MusicPlayerState,
    br: &mut BrowserState,
    radio: &mut RadioState,
    tv: &mut TvGuideState,
    icons_hidden: &mut bool,
    usb_storage: &mut Option<psp::usb::UsbStorageMode>,
    config: &mut psp::config::Config,
    current_preset: &mut skins::PspSkinPreset,
    active_theme: &mut ActiveTheme,
    skin_features: &SkinFeatures,
    dbg_log: &dyn Fn(&str),
) -> DispatchResult {
    match event {
        InputEvent::Quit => return DispatchResult::Quit,

        InputEvent::ButtonPress(Button::Start) => {
            if *classic_view == ClassicView::FileManager && fm.umd_activated {
                // SAFETY: deactivate UMD drive on exit.
                unsafe {
                    psp::sys::sceUmdDeactivate(1, b"disc0:\0".as_ptr());
                }
                fm.umd_activated = false;
            }
            *classic_view = match *classic_view {
                ClassicView::Dashboard => ClassicView::Terminal,
                ClassicView::Terminal => ClassicView::Dashboard,
                _ => ClassicView::Dashboard,
            };
        },

        InputEvent::ButtonPress(Button::Select) if *classic_view == ClassicView::Dashboard => {
            *app_mode = AppMode::Desktop;
        },

        // -- Dashboard input (via DashboardState) --
        InputEvent::ButtonPress(
            btn @ (Button::Up | Button::Down | Button::Left | Button::Right),
        ) if *classic_view == ClassicView::Dashboard => {
            let old_sel = dashboard.selected;
            dashboard.handle_input(btn);
            if dashboard.selected != old_sel {
                audio.send(AudioCmd::PlaySfx(SfxId::Click));
            }
        },
        InputEvent::ButtonPress(Button::Confirm) if *classic_view == ClassicView::Dashboard => {
            audio.send(AudioCmd::PlaySfx(SfxId::Navigate));
            dashboard.trigger_press_flash();
            let app_title = dashboard.selected_app().map(|a| a.title.clone());
            if let Some(ref title) = app_title {
                dispatch_dashboard_confirm(
                    title,
                    classic_view,
                    app_mode,
                    dashboard,
                    wm,
                    sdi,
                    audio,
                    io,
                    fm,
                    pv,
                    mp,
                    br,
                    radio,
                    tv,
                    backend,
                    dbg_log,
                );
            }
        },
        InputEvent::ButtonPress(Button::Cancel) if *classic_view == ClassicView::Dashboard => {
            *icons_hidden = !*icons_hidden;
        },

        // Trigger cycling through open windows (z-order).
        InputEvent::TriggerPress(Trigger::Left) if *classic_view == ClassicView::Dashboard => {
            wm.cycle_focus(false, sdi);
            audio.send(AudioCmd::PlaySfx(SfxId::Click));
        },
        InputEvent::TriggerPress(Trigger::Right) if *classic_view == ClassicView::Dashboard => {
            wm.cycle_focus(true, sdi);
            audio.send(AudioCmd::PlaySfx(SfxId::Click));
        },

        // -- Terminal input --
        InputEvent::ButtonPress(Button::Confirm) if *classic_view == ClassicView::Terminal => {
            dispatch_terminal_confirm(
                backend,
                term,
                audio,
                mp,
                usb_storage,
                config,
                current_preset,
                active_theme,
                skin_features,
                dashboard,
            );
        },
        InputEvent::ButtonPress(Button::Square) if *classic_view == ClassicView::Terminal => {
            match psp::osk::OskBuilder::new("Enter command")
                .max_chars(256)
                .initial_text(&term.input)
                .show()
            {
                Ok(Some(text)) => {
                    term.input = text;
                },
                Ok(None) => {},
                Err(e) => {
                    psp::dprintln!("OSK dialog error: {e}");
                },
            }
            backend.reinit_gu_frame();
        },
        InputEvent::ButtonPress(Button::Up) if *classic_view == ClassicView::Terminal => {
            let max_scroll = term.lines.len().saturating_sub(MAX_OUTPUT_LINES);
            if term.scroll < max_scroll {
                term.scroll += 3;
                if term.scroll > max_scroll {
                    term.scroll = max_scroll;
                }
            }
        },
        InputEvent::ButtonPress(Button::Down) if *classic_view == ClassicView::Terminal => {
            term.scroll = term.scroll.saturating_sub(3);
        },

        // -- File manager input (dual-panel) --
        InputEvent::ButtonPress(Button::Left) if *classic_view == ClassicView::FileManager => {
            fm.active_panel = 0;
            audio.send(AudioCmd::PlaySfx(SfxId::Click));
        },
        InputEvent::ButtonPress(Button::Right) if *classic_view == ClassicView::FileManager => {
            fm.active_panel = 1;
            audio.send(AudioCmd::PlaySfx(SfxId::Click));
        },
        InputEvent::ButtonPress(Button::Up) if *classic_view == ClassicView::FileManager => {
            let panel = fm.active_panel_mut();
            if panel.selected > 0 {
                panel.selected -= 1;
                if panel.selected < panel.scroll {
                    panel.scroll = panel.selected;
                }
            }
        },
        InputEvent::ButtonPress(Button::Down) if *classic_view == ClassicView::FileManager => {
            let panel = fm.active_panel_mut();
            if panel.selected + 1 < panel.entries.len() {
                panel.selected += 1;
                if panel.selected >= panel.scroll + FM_VISIBLE_ROWS {
                    panel.scroll = panel.selected - FM_VISIBLE_ROWS + 1;
                }
            }
        },
        InputEvent::ButtonPress(Button::Confirm) if *classic_view == ClassicView::FileManager => {
            let panel = fm.active_panel_mut();
            if panel.selected < panel.entries.len() && panel.entries[panel.selected].is_dir {
                let dir_name = panel.entries[panel.selected].name.clone();
                if panel.path.ends_with('/') {
                    panel.path = format!("{}{}", panel.path, dir_name);
                } else {
                    panel.path = format!("{}/{}", panel.path, dir_name);
                }
                panel.loaded = false;
            }
        },
        InputEvent::ButtonPress(Button::Cancel) if *classic_view == ClassicView::FileManager => {
            // Work on the active panel's path. Use index to avoid borrow issues.
            let path = if fm.active_panel == 0 {
                &fm.left.path
            } else {
                &fm.right.path
            };
            let rfind_pos = path.rfind('/');
            let path_len = path.len();
            let before_colon = rfind_pos.map(|pos| {
                (
                    pos,
                    pos > 0 && !path[..pos].ends_with(':'),
                    path_len > pos + 1,
                )
            });
            if let Some((pos, can_truncate_parent, can_truncate_root)) = before_colon {
                if can_truncate_parent {
                    fm.active_panel_mut().path.truncate(pos);
                } else if can_truncate_root {
                    fm.active_panel_mut().path.truncate(pos + 1);
                } else {
                    if fm.umd_activated {
                        // SAFETY: deactivate UMD drive on exit.
                        unsafe {
                            psp::sys::sceUmdDeactivate(1, b"disc0:\0".as_ptr());
                        }
                        fm.umd_activated = false;
                    }
                    *classic_view = ClassicView::Dashboard;
                }
                fm.active_panel_mut().loaded = false;
            } else {
                if fm.umd_activated {
                    // SAFETY: deactivate UMD drive on exit.
                    unsafe {
                        psp::sys::sceUmdDeactivate(1, b"disc0:\0".as_ptr());
                    }
                    fm.umd_activated = false;
                }
                *classic_view = ClassicView::Dashboard;
            }
        },
        InputEvent::ButtonPress(Button::Square) if *classic_view == ClassicView::FileManager => {
            let panel = fm.active_panel_ref();
            // UMD is read-only, skip delete.
            if panel.path.starts_with("disc0:") {
                term.lines.push("UMD is read-only.".into());
            } else if panel.selected < panel.entries.len() && !panel.entries[panel.selected].is_dir
            {
                let name = panel.entries[panel.selected].name.clone();
                let full_path = if panel.path.ends_with('/') {
                    format!("{}{}", panel.path, name)
                } else {
                    format!("{}/{}", panel.path, name)
                };
                let msg = format!("Delete {}?", name);
                match psp::dialog::confirm_dialog(&msg) {
                    Ok(psp::dialog::DialogResult::Confirm) => {
                        match psp::io::remove_file(&full_path) {
                            Ok(()) => {
                                term.lines.push(format!("Deleted: {}", full_path));
                                fm.active_panel_mut().loaded = false;
                            },
                            Err(e) => {
                                let _ = psp::dialog::error_dialog(e.0 as u32);
                            },
                        }
                    },
                    _ => {},
                }
                backend.reinit_gu_frame();
            }
        },
        InputEvent::ButtonPress(Button::Triangle) if *classic_view == ClassicView::FileManager => {
            if fm.umd_activated {
                // SAFETY: deactivate UMD drive on exit.
                unsafe {
                    psp::sys::sceUmdDeactivate(1, b"disc0:\0".as_ptr());
                }
                fm.umd_activated = false;
            }
            *classic_view = ClassicView::Dashboard;
        },

        // -- Photo viewer input --
        InputEvent::ButtonPress(Button::Up)
            if *classic_view == ClassicView::PhotoViewer && !pv.viewing =>
        {
            if pv.selected > 0 {
                pv.selected -= 1;
                if pv.selected < pv.scroll {
                    pv.scroll = pv.selected;
                }
            }
        },
        InputEvent::ButtonPress(Button::Down)
            if *classic_view == ClassicView::PhotoViewer && !pv.viewing =>
        {
            if pv.selected + 1 < pv.entries.len() {
                pv.selected += 1;
                if pv.selected >= pv.scroll + FM_VISIBLE_ROWS {
                    pv.scroll = pv.selected - FM_VISIBLE_ROWS + 1;
                }
            }
        },
        InputEvent::ButtonPress(Button::Confirm)
            if *classic_view == ClassicView::PhotoViewer && !pv.viewing =>
        {
            if pv.selected < pv.entries.len() {
                let entry = &pv.entries[pv.selected];
                if entry.is_dir {
                    let dir_name = entry.name.clone();
                    if pv.path.ends_with('/') {
                        pv.path = format!("{}{}", pv.path, dir_name);
                    } else {
                        pv.path = format!("{}/{}", pv.path, dir_name);
                    }
                    pv.loaded = false;
                } else {
                    let file_path = if pv.path.ends_with('/') {
                        format!("{}{}", pv.path, entry.name)
                    } else {
                        format!("{}/{}", pv.path, entry.name)
                    };
                    io.send(IoCmd::LoadTexture {
                        path: file_path,
                        max_w: SCREEN_WIDTH as i32,
                        max_h: SCREEN_HEIGHT as i32,
                    });
                    pv.loading = true;
                }
            }
        },
        InputEvent::ButtonPress(Button::Cancel) if *classic_view == ClassicView::PhotoViewer => {
            if pv.viewing {
                pv.viewing = false;
            } else if let Some(pos) = pv.path.rfind('/') {
                if pos > 0 && !pv.path[..pos].ends_with(':') {
                    pv.path.truncate(pos);
                } else if pv.path.len() > pos + 1 {
                    pv.path.truncate(pos + 1);
                } else {
                    *classic_view = ClassicView::Dashboard;
                }
                pv.loaded = false;
            } else {
                *classic_view = ClassicView::Dashboard;
            }
        },
        InputEvent::ButtonPress(Button::Triangle) if *classic_view == ClassicView::PhotoViewer => {
            if pv.viewing {
                pv.viewing = false;
            } else {
                *classic_view = ClassicView::Dashboard;
            }
        },

        // -- Music player input --
        InputEvent::ButtonPress(Button::Up)
            if *classic_view == ClassicView::MusicPlayer && !audio.is_playing() =>
        {
            if mp.selected > 0 {
                mp.selected -= 1;
                if mp.selected < mp.scroll {
                    mp.scroll = mp.selected;
                }
            }
        },
        InputEvent::ButtonPress(Button::Down)
            if *classic_view == ClassicView::MusicPlayer && !audio.is_playing() =>
        {
            if mp.selected + 1 < mp.entries.len() {
                mp.selected += 1;
                if mp.selected >= mp.scroll + FM_VISIBLE_ROWS {
                    mp.scroll = mp.selected - FM_VISIBLE_ROWS + 1;
                }
            }
        },
        InputEvent::ButtonPress(Button::Confirm) if *classic_view == ClassicView::MusicPlayer => {
            if audio.is_playing() {
                if audio.is_paused() {
                    audio.send(AudioCmd::Resume);
                } else {
                    audio.send(AudioCmd::Pause);
                }
            } else if mp.selected < mp.entries.len() {
                let entry = &mp.entries[mp.selected];
                if entry.is_dir {
                    let dir_name = entry.name.clone();
                    if mp.path.ends_with('/') {
                        mp.path = format!("{}{}", mp.path, dir_name);
                    } else {
                        mp.path = format!("{}/{}", mp.path, dir_name);
                    }
                    mp.loaded = false;
                } else {
                    let file_path = if mp.path.ends_with('/') {
                        format!("{}{}", mp.path, entry.name)
                    } else {
                        format!("{}/{}", mp.path, entry.name)
                    };
                    mp.file_name = entry.name.clone();
                    audio.send(AudioCmd::LoadAndPlay(file_path));
                    term.lines.push(format!("Playing: {}", entry.name));
                }
            }
        },
        InputEvent::ButtonPress(Button::Square) if *classic_view == ClassicView::MusicPlayer => {
            audio.send(AudioCmd::Stop);
        },
        InputEvent::ButtonPress(Button::Cancel) if *classic_view == ClassicView::MusicPlayer => {
            audio.send(AudioCmd::Stop);
            if let Some(pos) = mp.path.rfind('/') {
                if pos > 0 && !mp.path[..pos].ends_with(':') {
                    mp.path.truncate(pos);
                } else if mp.path.len() > pos + 1 {
                    mp.path.truncate(pos + 1);
                } else {
                    *classic_view = ClassicView::Dashboard;
                }
                mp.loaded = false;
            } else {
                *classic_view = ClassicView::Dashboard;
            }
        },
        InputEvent::ButtonPress(Button::Triangle) if *classic_view == ClassicView::MusicPlayer => {
            *classic_view = ClassicView::Dashboard;
        },

        // -- Browser input (full oasis-browser engine) --
        InputEvent::ButtonPress(Button::Square) if *classic_view == ClassicView::Browser => {
            // Open OSK for URL entry.
            let current_url = br.url().to_string();
            match psp::osk::OskBuilder::new("Enter URL")
                .max_chars(256)
                .initial_text(&current_url)
                .show()
            {
                Ok(Some(text)) => {
                    // Ensure network is up.
                    if !oasis_backend_psp::network::is_net_initialized() {
                        if let Err(e) = oasis_backend_psp::network::ensure_net_init_pub() {
                            br.status_msg = format!("Net error: {e}");
                            backend.reinit_gu_frame();
                            return DispatchResult::Continue;
                        }
                    }
                    br.loading = true;
                    br.status_msg = String::from("Loading...");
                    dbg_log(&format!("[Browser] navigating to: {text}"));
                    // Ensure widget exists, then navigate.
                    br.ensure_widget();
                    if let Some(ref mut w) = br.widget {
                        w.navigate_vfs(&text, &br.vfs);
                    }
                    dbg_log("[Browser] navigate_vfs returned");
                    br.loading = false;
                    let url_display = br.url().to_string();
                    br.status_msg = format!("Loaded: {url_display}");
                },
                Ok(None) => {},
                Err(e) => {
                    psp::dprintln!("OSK dialog error: {e}");
                },
            }
            backend.reinit_gu_frame();
        },
        InputEvent::ButtonPress(Button::Confirm) if *classic_view == ClassicView::Browser => {
            dbg_log("[Browser] Confirm pressed");
            // Ensure network is up.
            if !oasis_backend_psp::network::is_net_initialized() {
                if let Err(e) = oasis_backend_psp::network::ensure_net_init_pub() {
                    br.status_msg = format!("Net error: {e}");
                    backend.reinit_gu_frame();
                    return DispatchResult::Continue;
                }
                backend.reinit_gu_frame();
            }
            br.ensure_widget();
            // Forward X press to BrowserWidget (follows focused link).
            let input_event =
                oasis_backend_psp::InputEvent::ButtonPress(oasis_backend_psp::Button::Confirm);
            let handled = br
                .widget
                .as_mut()
                .is_some_and(|w| w.handle_input(&input_event, &br.vfs));
            if !handled {
                // No link selected -- navigate to home URL.
                let home = br
                    .widget
                    .as_ref()
                    .map(|w| w.config.features.home_url.clone())
                    .unwrap_or_else(|| "https://info.cern.ch".to_string());
                br.loading = true;
                br.status_msg = String::from("Loading...");
                dbg_log(&format!("[Browser] navigating to home: {home}"));
                if let Some(ref mut w) = br.widget {
                    w.navigate_vfs(&home, &br.vfs);
                }
                dbg_log("[Browser] navigate_vfs returned");
                br.loading = false;
                let url_display = br.url().to_string();
                br.status_msg = format!("Loaded: {url_display}");
            }
        },
        InputEvent::ButtonPress(Button::Up) if *classic_view == ClassicView::Browser => {
            if let Some(ref mut w) = br.widget {
                for _ in 0..3 {
                    w.scroll_mut().scroll_up();
                }
            }
        },
        InputEvent::ButtonPress(Button::Down) if *classic_view == ClassicView::Browser => {
            if let Some(ref mut w) = br.widget {
                for _ in 0..3 {
                    w.scroll_mut().scroll_down();
                }
            }
        },
        InputEvent::ButtonPress(Button::Left) if *classic_view == ClassicView::Browser => {
            if let Some(ref mut w) = br.widget {
                let event =
                    oasis_backend_psp::InputEvent::ButtonPress(oasis_backend_psp::Button::Left);
                w.handle_input(&event, &br.vfs);
            }
        },
        InputEvent::ButtonPress(Button::Right) if *classic_view == ClassicView::Browser => {
            if let Some(ref mut w) = br.widget {
                let event =
                    oasis_backend_psp::InputEvent::ButtonPress(oasis_backend_psp::Button::Right);
                w.handle_input(&event, &br.vfs);
            }
        },
        InputEvent::TriggerPress(Trigger::Left) if *classic_view == ClassicView::Browser => {
            if let Some(ref mut w) = br.widget {
                w.go_back(&br.vfs);
            }
        },
        InputEvent::TriggerPress(Trigger::Right) if *classic_view == ClassicView::Browser => {
            if let Some(ref mut w) = br.widget {
                w.go_forward(&br.vfs);
            }
        },
        InputEvent::ButtonPress(Button::Triangle) if *classic_view == ClassicView::Browser => {
            *classic_view = ClassicView::Dashboard;
        },
        InputEvent::ButtonPress(Button::Cancel) if *classic_view == ClassicView::Browser => {
            *classic_view = ClassicView::Dashboard;
        },

        // -- Radio input --
        InputEvent::ButtonPress(Button::Up)
            if *classic_view == ClassicView::Radio && radio.status == RadioStatus::Stopped =>
        {
            if radio.selected > 0 {
                radio.selected -= 1;
                if radio.selected < radio.scroll {
                    radio.scroll = radio.selected;
                }
            }
        },
        InputEvent::ButtonPress(Button::Down)
            if *classic_view == ClassicView::Radio && radio.status == RadioStatus::Stopped =>
        {
            if radio.selected + 1 < RADIO_STATIONS.len() {
                radio.selected += 1;
                if radio.selected >= radio.scroll + FM_VISIBLE_ROWS {
                    radio.scroll = radio.selected - FM_VISIBLE_ROWS + 1;
                }
            }
        },
        InputEvent::ButtonPress(Button::Confirm) if *classic_view == ClassicView::Radio => {
            if radio.status == RadioStatus::Stopped || radio.status == RadioStatus::Error {
                if radio.selected < RADIO_STATIONS.len() {
                    if !oasis_backend_psp::network::is_net_initialized() {
                        if let Err(e) = oasis_backend_psp::network::ensure_net_init_pub() {
                            radio.error_msg = format!("Net error: {e}");
                            radio.status = RadioStatus::Error;
                            backend.reinit_gu_frame();
                            return DispatchResult::Continue;
                        }
                        backend.reinit_gu_frame();
                    }
                    let station = &RADIO_STATIONS[radio.selected];
                    radio.station_name = String::from(station.name);
                    radio.now_playing.clear();
                    radio.status = RadioStatus::Connecting;
                    io.send(IoCmd::RadioConnect {
                        url: String::from(station.url),
                    });
                }
            }
        },
        InputEvent::ButtonPress(Button::Square) if *classic_view == ClassicView::Radio => {
            if radio.status != RadioStatus::Stopped {
                audio.send(AudioCmd::RadioStop);
                radio.status = RadioStatus::Stopped;
                radio.now_playing.clear();
            }
        },
        InputEvent::ButtonPress(Button::Triangle) if *classic_view == ClassicView::Radio => {
            *classic_view = ClassicView::Dashboard;
        },
        InputEvent::ButtonPress(Button::Cancel) if *classic_view == ClassicView::Radio => {
            if radio.status != RadioStatus::Stopped {
                audio.send(AudioCmd::RadioStop);
                radio.status = RadioStatus::Stopped;
                radio.now_playing.clear();
            }
            *classic_view = ClassicView::Dashboard;
        },

        // -- TV Guide input --
        InputEvent::ButtonPress(Button::Up)
            if *classic_view == ClassicView::TvGuide && tv.tuned.is_none() =>
        {
            if tv.selected > 0 {
                tv.selected -= 1;
                if tv.selected < tv.scroll {
                    tv.scroll = tv.selected;
                }
            }
        },
        InputEvent::ButtonPress(Button::Down)
            if *classic_view == ClassicView::TvGuide && tv.tuned.is_none() =>
        {
            if tv.selected + 1 < tv.channels.len() {
                tv.selected += 1;
                if tv.selected >= tv.scroll + FM_VISIBLE_ROWS {
                    tv.scroll = tv.selected - FM_VISIBLE_ROWS + 1;
                }
            }
        },
        InputEvent::ButtonPress(Button::Confirm) if *classic_view == ClassicView::TvGuide => {
            dispatch_tv_confirm(tv, io, backend, dbg_log);
        },
        InputEvent::ButtonPress(Button::Cancel) if *classic_view == ClassicView::TvGuide => {
            if tv.tuned.is_some() || tv.downloading {
                oasis_backend_psp::threading::cancel_video_download();
                oasis_backend_psp::video::send_video_cmd(oasis_backend_psp::video::VideoCmd::Stop);
                audio.send(AudioCmd::VideoAudioStop);
                if let Some(old) = tv.preview_tex.take() {
                    backend.destroy_texture_inner(old);
                }
                tv.tuned = None;
                tv.downloading = false;
                tv.now_playing.clear();
                tv.error_msg.clear();
            } else {
                *classic_view = ClassicView::Dashboard;
            }
        },
        InputEvent::ButtonPress(Button::Triangle) if *classic_view == ClassicView::TvGuide => {
            *classic_view = ClassicView::Dashboard;
        },

        _ => {},
    }
    DispatchResult::Continue
}
