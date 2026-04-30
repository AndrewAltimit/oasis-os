//! Input dispatch for kiosk app views.
//!
//! Routes input events to the appropriate app handler based on the current
//! `KioskApp` state.  Each app's handlers are identical to the former Classic
//! mode, but keyed by `KioskApp` variant instead of `ClassicView`.

use oasis_backend_psp::threading::IoHandle;
use oasis_backend_psp::{
    AudioCmd, AudioHandle, Button, InputEvent, IoCmd, PspBackend, SCREEN_HEIGHT, SCREEN_WIDTH,
    SdiRegistry, SfxId, Trigger, WindowManager,
};

use oasis_core::active_theme::ActiveTheme;
use oasis_core::dashboard::DashboardState;
use oasis_core::skin::SkinFeatures;

use crate::app_states::{
    BrowserState, FileManagerState, MusicPlayerState, PhotoViewerState, RadioState, SettingsState,
    TerminalState, TvGuideState,
};
use crate::skins;
use crate::theme::*;
use crate::types::*;

use super::DispatchResult;
use super::helpers::{dispatch_terminal_confirm, dispatch_tv_confirm};

/// Close the current kiosk app and return to dashboard.
fn close_kiosk(
    kiosk_app: &mut KioskApp,
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
) {
    if let Some(wid) = kiosk_app.window_id() {
        let _ = wm.exit_fullscreen(wid, sdi);
        let _ = wm.close_window(wid, sdi);
    }
    *kiosk_app = KioskApp::None;
}

/// Handle a single input event for the currently active kiosk app.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_app_input(
    event: &InputEvent,
    backend: &mut PspBackend,
    kiosk_app: &mut KioskApp,
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
    settings: &mut SettingsState,
    usb_storage: &mut Option<psp::usb::UsbStorageMode>,
    config: &mut psp::config::Config,
    current_preset: &mut skins::PspSkinPreset,
    active_theme: &mut ActiveTheme,
    skin_features: &SkinFeatures,
    dbg_log: &dyn Fn(&str),
) -> DispatchResult {
    match event {
        InputEvent::Quit => return DispatchResult::Quit,

        // -- Terminal input --
        InputEvent::ButtonPress(Button::Confirm) if *kiosk_app == KioskApp::Terminal => {
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
        InputEvent::ButtonPress(Button::Square) if *kiosk_app == KioskApp::Terminal => {
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
        InputEvent::ButtonPress(Button::Up) if *kiosk_app == KioskApp::Terminal => {
            let max_scroll = term.lines.len().saturating_sub(MAX_OUTPUT_LINES);
            if term.scroll < max_scroll {
                term.scroll += 3;
                if term.scroll > max_scroll {
                    term.scroll = max_scroll;
                }
            }
        },
        InputEvent::ButtonPress(Button::Down) if *kiosk_app == KioskApp::Terminal => {
            term.scroll = term.scroll.saturating_sub(3);
        },
        InputEvent::ButtonPress(Button::Triangle) if *kiosk_app == KioskApp::Terminal => {
            close_kiosk(kiosk_app, wm, sdi);
        },
        InputEvent::ButtonPress(Button::Cancel) if *kiosk_app == KioskApp::Terminal => {
            close_kiosk(kiosk_app, wm, sdi);
        },

        // -- File manager input (dual-panel) --
        InputEvent::ButtonPress(Button::Left) if *kiosk_app == KioskApp::FileManager => {
            fm.active_panel = 0;
            audio.send(AudioCmd::PlaySfx(SfxId::Click));
        },
        InputEvent::ButtonPress(Button::Right) if *kiosk_app == KioskApp::FileManager => {
            fm.active_panel = 1;
            audio.send(AudioCmd::PlaySfx(SfxId::Click));
        },
        InputEvent::ButtonPress(Button::Up) if *kiosk_app == KioskApp::FileManager => {
            let panel = fm.active_panel_mut();
            if panel.selected > 0 {
                panel.selected -= 1;
                if panel.selected < panel.scroll {
                    panel.scroll = panel.selected;
                }
            }
        },
        InputEvent::ButtonPress(Button::Down) if *kiosk_app == KioskApp::FileManager => {
            let panel = fm.active_panel_mut();
            if panel.selected + 1 < panel.entries.len() {
                panel.selected += 1;
                if panel.selected >= panel.scroll + FM_VISIBLE_ROWS {
                    panel.scroll = panel.selected - FM_VISIBLE_ROWS + 1;
                }
            }
        },
        InputEvent::ButtonPress(Button::Confirm) if *kiosk_app == KioskApp::FileManager => {
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
        InputEvent::ButtonPress(Button::Cancel) if *kiosk_app == KioskApp::FileManager => {
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
                    close_kiosk(kiosk_app, wm, sdi);
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
                close_kiosk(kiosk_app, wm, sdi);
            }
        },
        InputEvent::ButtonPress(Button::Square) if *kiosk_app == KioskApp::FileManager => {
            let panel = fm.active_panel_ref();
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
        InputEvent::ButtonPress(Button::Triangle) if *kiosk_app == KioskApp::FileManager => {
            if fm.umd_activated {
                // SAFETY: deactivate UMD drive on exit.
                unsafe {
                    psp::sys::sceUmdDeactivate(1, b"disc0:\0".as_ptr());
                }
                fm.umd_activated = false;
            }
            close_kiosk(kiosk_app, wm, sdi);
        },

        // -- Photo viewer input --
        InputEvent::ButtonPress(Button::Up)
            if *kiosk_app == KioskApp::PhotoViewer && !pv.viewing =>
        {
            if pv.selected > 0 {
                pv.selected -= 1;
                if pv.selected < pv.scroll {
                    pv.scroll = pv.selected;
                }
            }
        },
        InputEvent::ButtonPress(Button::Down)
            if *kiosk_app == KioskApp::PhotoViewer && !pv.viewing =>
        {
            if pv.selected + 1 < pv.entries.len() {
                pv.selected += 1;
                if pv.selected >= pv.scroll + FM_VISIBLE_ROWS {
                    pv.scroll = pv.selected - FM_VISIBLE_ROWS + 1;
                }
            }
        },
        InputEvent::ButtonPress(Button::Confirm)
            if *kiosk_app == KioskApp::PhotoViewer && !pv.viewing =>
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
        InputEvent::ButtonPress(Button::Cancel) if *kiosk_app == KioskApp::PhotoViewer => {
            if pv.viewing {
                pv.viewing = false;
            } else if let Some(pos) = pv.path.rfind('/') {
                if pos > 0 && !pv.path[..pos].ends_with(':') {
                    pv.path.truncate(pos);
                } else if pv.path.len() > pos + 1 {
                    pv.path.truncate(pos + 1);
                } else {
                    close_kiosk(kiosk_app, wm, sdi);
                }
                pv.loaded = false;
            } else {
                close_kiosk(kiosk_app, wm, sdi);
            }
        },
        InputEvent::ButtonPress(Button::Triangle) if *kiosk_app == KioskApp::PhotoViewer => {
            if pv.viewing {
                pv.viewing = false;
            } else {
                close_kiosk(kiosk_app, wm, sdi);
            }
        },

        // -- Music player input --
        InputEvent::ButtonPress(Button::Up)
            if *kiosk_app == KioskApp::MusicPlayer && !audio.is_playing() =>
        {
            if mp.selected > 0 {
                mp.selected -= 1;
                if mp.selected < mp.scroll {
                    mp.scroll = mp.selected;
                }
            }
        },
        InputEvent::ButtonPress(Button::Down)
            if *kiosk_app == KioskApp::MusicPlayer && !audio.is_playing() =>
        {
            if mp.selected + 1 < mp.entries.len() {
                mp.selected += 1;
                if mp.selected >= mp.scroll + FM_VISIBLE_ROWS {
                    mp.scroll = mp.selected - FM_VISIBLE_ROWS + 1;
                }
            }
        },
        InputEvent::ButtonPress(Button::Confirm) if *kiosk_app == KioskApp::MusicPlayer => {
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
        InputEvent::ButtonPress(Button::Square) if *kiosk_app == KioskApp::MusicPlayer => {
            audio.send(AudioCmd::Stop);
        },
        InputEvent::ButtonPress(Button::Cancel) if *kiosk_app == KioskApp::MusicPlayer => {
            audio.send(AudioCmd::Stop);
            if let Some(pos) = mp.path.rfind('/') {
                if pos > 0 && !mp.path[..pos].ends_with(':') {
                    mp.path.truncate(pos);
                } else if mp.path.len() > pos + 1 {
                    mp.path.truncate(pos + 1);
                } else {
                    close_kiosk(kiosk_app, wm, sdi);
                }
                mp.loaded = false;
            } else {
                close_kiosk(kiosk_app, wm, sdi);
            }
        },
        InputEvent::ButtonPress(Button::Triangle) if *kiosk_app == KioskApp::MusicPlayer => {
            close_kiosk(kiosk_app, wm, sdi);
        },

        // -- Browser input (full oasis-browser engine) --
        InputEvent::ButtonPress(Button::Square) if *kiosk_app == KioskApp::Browser => {
            let current_url = br.url().to_string();
            match psp::osk::OskBuilder::new("Enter URL")
                .max_chars(256)
                .initial_text(&current_url)
                .show()
            {
                Ok(Some(text)) => {
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
        InputEvent::ButtonPress(Button::Confirm) if *kiosk_app == KioskApp::Browser => {
            dbg_log("[Browser] Confirm pressed");
            if !oasis_backend_psp::network::is_net_initialized() {
                if let Err(e) = oasis_backend_psp::network::ensure_net_init_pub() {
                    br.status_msg = format!("Net error: {e}");
                    backend.reinit_gu_frame();
                    return DispatchResult::Continue;
                }
                backend.reinit_gu_frame();
            }
            let input_event =
                oasis_backend_psp::InputEvent::ButtonPress(oasis_backend_psp::Button::Confirm);
            br.ensure_widget();
            if let Some(ref mut w) = br.widget {
                w.handle_input(&input_event, &br.vfs);
            }
        },
        InputEvent::ButtonPress(Button::Up) if *kiosk_app == KioskApp::Browser => {
            if let Some(ref mut w) = br.widget {
                for _ in 0..3 {
                    w.scroll_mut().scroll_up();
                }
            }
        },
        InputEvent::ButtonPress(Button::Down) if *kiosk_app == KioskApp::Browser => {
            if let Some(ref mut w) = br.widget {
                for _ in 0..3 {
                    w.scroll_mut().scroll_down();
                }
            }
        },
        InputEvent::ButtonPress(Button::Left) if *kiosk_app == KioskApp::Browser => {
            if let Some(ref mut w) = br.widget {
                let event =
                    oasis_backend_psp::InputEvent::ButtonPress(oasis_backend_psp::Button::Left);
                w.handle_input(&event, &br.vfs);
            }
        },
        InputEvent::ButtonPress(Button::Right) if *kiosk_app == KioskApp::Browser => {
            if let Some(ref mut w) = br.widget {
                let event =
                    oasis_backend_psp::InputEvent::ButtonPress(oasis_backend_psp::Button::Right);
                w.handle_input(&event, &br.vfs);
            }
        },
        InputEvent::TriggerPress(Trigger::Left) if *kiosk_app == KioskApp::Browser => {
            if let Some(ref mut w) = br.widget {
                w.go_back(&br.vfs);
            }
        },
        InputEvent::TriggerPress(Trigger::Right) if *kiosk_app == KioskApp::Browser => {
            if let Some(ref mut w) = br.widget {
                w.go_forward(&br.vfs);
            }
        },
        InputEvent::ButtonPress(Button::Triangle) if *kiosk_app == KioskApp::Browser => {
            close_kiosk(kiosk_app, wm, sdi);
        },
        InputEvent::ButtonPress(Button::Cancel) if *kiosk_app == KioskApp::Browser => {
            close_kiosk(kiosk_app, wm, sdi);
        },

        // -- Radio input --
        InputEvent::ButtonPress(Button::Up)
            if *kiosk_app == KioskApp::Radio && radio.status == RadioStatus::Stopped =>
        {
            if radio.selected > 0 {
                radio.selected -= 1;
                if radio.selected < radio.scroll {
                    radio.scroll = radio.selected;
                }
            }
        },
        InputEvent::ButtonPress(Button::Down)
            if *kiosk_app == KioskApp::Radio && radio.status == RadioStatus::Stopped =>
        {
            if radio.selected + 1 < RADIO_STATIONS.len() {
                radio.selected += 1;
                if radio.selected >= radio.scroll + FM_VISIBLE_ROWS {
                    radio.scroll = radio.selected - FM_VISIBLE_ROWS + 1;
                }
            }
        },
        InputEvent::ButtonPress(Button::Confirm) if *kiosk_app == KioskApp::Radio => {
            if radio.status == RadioStatus::Stopped || radio.status == RadioStatus::Error {
                if radio.selected < RADIO_STATIONS.len() {
                    if !oasis_backend_psp::network::is_net_initialized() {
                        // Show the WiFi dialog only if needed. The dialog
                        // takes over GU rendering, so we must reinit our GU
                        // frame after — but only when the dialog actually
                        // ran. Calling `reinit_gu_frame` unconditionally
                        // (incl. on the GotIp early-return path) issues a
                        // second `sceGuStart` inside the active display list
                        // and hangs the next `sceGuSync` on real hardware.
                        match oasis_backend_psp::network::ensure_net_init_pub() {
                            Ok(()) => {
                                // GotIp path is silent; no dialog ran, no
                                // GU reinit needed.
                            },
                            Err(e) => {
                                radio.error_msg = format!("Net error: {e}");
                                radio.status = RadioStatus::Error;
                                backend.reinit_gu_frame();
                                return DispatchResult::Continue;
                            },
                        }
                    }
                    let station = &RADIO_STATIONS[radio.selected];
                    radio.station_name = String::from(station.name);
                    radio.now_playing.clear();
                    radio.status = RadioStatus::Connecting;
                    match station.source_type {
                        "archive" => {
                            io.send(IoCmd::RadioArchive {
                                collection: String::from(station.collection),
                            });
                        },
                        "icecast" => {
                            io.send(IoCmd::RadioConnect {
                                url: String::from(station.url),
                            });
                        },
                        other => {
                            // Defensive: a typo in a station entry's
                            // source_type would otherwise silently fall
                            // through to icecast and connect to whatever
                            // `station.url` happens to contain (often
                            // empty for archive stations).
                            psp::dprintln!(
                                "[RADIO] unknown source_type {:?} for station {:?}; \
                                 falling back to icecast with url={:?}",
                                other,
                                station.name,
                                station.url,
                            );
                            io.send(IoCmd::RadioConnect {
                                url: String::from(station.url),
                            });
                        },
                    }
                }
            }
        },
        InputEvent::ButtonPress(Button::Square) if *kiosk_app == KioskApp::Radio => {
            if radio.status != RadioStatus::Stopped {
                oasis_backend_psp::threading::cancel_radio_stream();
                audio.send(AudioCmd::RadioStop);
                radio.status = RadioStatus::Stopped;
                radio.now_playing.clear();
            }
        },
        InputEvent::ButtonPress(Button::Triangle) if *kiosk_app == KioskApp::Radio => {
            close_kiosk(kiosk_app, wm, sdi);
        },
        InputEvent::ButtonPress(Button::Cancel) if *kiosk_app == KioskApp::Radio => {
            if radio.status != RadioStatus::Stopped {
                oasis_backend_psp::threading::cancel_radio_stream();
                audio.send(AudioCmd::RadioStop);
                radio.status = RadioStatus::Stopped;
                radio.now_playing.clear();
            }
            close_kiosk(kiosk_app, wm, sdi);
        },

        // -- TV Guide input --
        InputEvent::ButtonPress(Button::Up)
            if *kiosk_app == KioskApp::TvGuide && tv.tuned.is_none() =>
        {
            if tv.selected > 0 {
                tv.selected -= 1;
                if tv.selected < tv.scroll {
                    tv.scroll = tv.selected;
                }
            }
        },
        InputEvent::ButtonPress(Button::Down)
            if *kiosk_app == KioskApp::TvGuide && tv.tuned.is_none() =>
        {
            if tv.selected + 1 < tv.channels.len() {
                tv.selected += 1;
                if tv.selected >= tv.scroll + FM_VISIBLE_ROWS {
                    tv.scroll = tv.selected - FM_VISIBLE_ROWS + 1;
                }
            }
        },
        InputEvent::ButtonPress(Button::Confirm) if *kiosk_app == KioskApp::TvGuide => {
            dispatch_tv_confirm(tv, io, backend, dbg_log);
        },
        InputEvent::ButtonPress(Button::Cancel) if *kiosk_app == KioskApp::TvGuide => {
            if tv.tuned.is_some() || tv.downloading {
                // Signal cancellation — I/O and video threads will
                // clean up asynchronously.
                oasis_backend_psp::threading::cancel_video_download();
                oasis_backend_psp::video::send_video_cmd(oasis_backend_psp::video::VideoCmd::Stop);
                audio.send(AudioCmd::VideoAudioStop);
                tv.tuned = None;
                tv.downloading = false;
                tv.now_playing.clear();
                tv.error_msg.clear();
                // Drop the cached texture id too so the post-stream
                // cleanup branch in `main.rs` (which is gated on
                // `tv.tuned.is_some()`) doesn't fire on the *next*
                // tune and free the freshly pre-allocated texture
                // out from under the new stream. The texture itself
                // is reused on the next `dispatch_tv_confirm`.
                tv.preview_tex = None;
            } else {
                close_kiosk(kiosk_app, wm, sdi);
            }
        },
        InputEvent::ButtonPress(Button::Triangle) if *kiosk_app == KioskApp::TvGuide => {
            close_kiosk(kiosk_app, wm, sdi);
        },

        // -- Settings input (theme picker) --
        InputEvent::ButtonPress(Button::Up) if *kiosk_app == KioskApp::Settings => {
            if settings.selected > 0 {
                settings.selected -= 1;
                if settings.selected < settings.scroll {
                    settings.scroll = settings.selected;
                }
                audio.send(AudioCmd::PlaySfx(SfxId::Click));
            }
        },
        InputEvent::ButtonPress(Button::Down) if *kiosk_app == KioskApp::Settings => {
            if settings.selected + 1 < skins::PspSkinPreset::ALL.len() {
                settings.selected += 1;
                if settings.selected >= settings.scroll + FM_VISIBLE_ROWS {
                    settings.scroll = settings.selected - FM_VISIBLE_ROWS + 1;
                }
                audio.send(AudioCmd::PlaySfx(SfxId::Click));
            }
        },
        InputEvent::ButtonPress(Button::Confirm) if *kiosk_app == KioskApp::Settings => {
            let preset = skins::PspSkinPreset::ALL[settings.selected];
            if skins::apply_skin_preset(
                preset,
                current_preset,
                active_theme,
                skin_features,
                dashboard,
                config,
            ) {
                audio.send(AudioCmd::PlaySfx(SfxId::Navigate));
                dbg_log(&format!("[SETTINGS] skin -> '{}'", preset.name()));
            }
        },
        InputEvent::ButtonPress(Button::Cancel) if *kiosk_app == KioskApp::Settings => {
            close_kiosk(kiosk_app, wm, sdi);
        },
        InputEvent::ButtonPress(Button::Triangle) if *kiosk_app == KioskApp::Settings => {
            close_kiosk(kiosk_app, wm, sdi);
        },

        _ => {},
    }
    DispatchResult::Continue
}
