//! Input event dispatch for Classic and Desktop modes.
//!
//! Extracts the large `match event { ... }` blocks from the main loop into
//! dedicated functions, keeping the main loop as a thin orchestrator.

use psp::sys::CtrlButtons;

use oasis_backend_psp::{
    AudioCmd, Button, InputEvent, IoCmd, PspBackend, SCREEN_HEIGHT, SCREEN_WIDTH, SdiRegistry,
    SfxId, Trigger, TvCatalogRequest, WindowManager,
};

use oasis_core::dashboard::DashboardState;

use crate::app_states::{
    BrowserState, FileManagerState, MusicPlayerState, PhotoViewerState, RadioState, TerminalState,
    TvGuideState,
};
use crate::commands;
use crate::desktop;
use crate::skins;
use crate::theme::*;
use crate::types::*;

use oasis_backend_psp::AudioHandle;
use oasis_backend_psp::threading::IoHandle;

use oasis_core::active_theme::ActiveTheme;
use oasis_core::dashboard::DashboardConfig;
use oasis_core::skin::SkinFeatures;

/// Return type for input dispatch: whether to `continue` the outer event loop
/// or `return` from `psp_main`.
pub(crate) enum DispatchResult {
    /// Continue processing the next event.
    Continue,
    /// Skip remaining events this frame (used after Desktop mode dispatch).
    SkipRest,
    /// Exit the main loop.
    Quit,
}

/// Handle a single input event in Desktop mode.
///
/// Returns `DispatchResult` to tell the caller what to do next.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_desktop(
    event: &InputEvent,
    backend: &mut PspBackend,
    app_mode: &mut AppMode,
    classic_view: &mut ClassicView,
    dashboard: &mut DashboardState,
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    term: &mut TerminalState,
    audio: &AudioHandle,
) -> DispatchResult {
    match event {
        InputEvent::ButtonPress(Button::Confirm) => {
            let (cx, cy) = backend.cursor_pos();
            let ptr_event = InputEvent::PointerClick { x: cx, y: cy };
            let wm_event = wm.handle_input(&ptr_event, sdi);
            desktop::handle_wm_event(
                &wm_event,
                &mut term.lines,
                classic_view,
                app_mode,
                wm,
                sdi,
                dashboard.page,
            );
        },
        InputEvent::ButtonRelease(Button::Confirm) => {
            let (cx, cy) = backend.cursor_pos();
            let ptr_event = InputEvent::PointerRelease { x: cx, y: cy };
            wm.handle_input(&ptr_event, sdi);
        },
        InputEvent::CursorMove { x, y } => {
            let move_event = InputEvent::CursorMove { x: *x, y: *y };
            wm.handle_input(&move_event, sdi);
        },
        InputEvent::ButtonPress(Button::Select) => {
            *app_mode = AppMode::Classic;
            *classic_view = ClassicView::Dashboard;
        },
        InputEvent::ButtonPress(Button::Triangle) => {
            if let Some(app) = dashboard.selected_app() {
                let title = app.title.clone();
                if let Some(psp_app) = APPS.iter().find(|a| a.title == title.as_str()) {
                    desktop::open_app_window(wm, sdi, psp_app.id, psp_app.title);
                }
            }
        },
        InputEvent::ButtonPress(Button::Start) => {
            desktop::open_app_window(wm, sdi, "terminal", "Terminal");
        },
        InputEvent::ButtonPress(
            btn @ (Button::Up | Button::Down | Button::Left | Button::Right),
        ) => {
            let old_sel = dashboard.selected;
            dashboard.handle_input(btn);
            if dashboard.selected != old_sel {
                audio.send(AudioCmd::PlaySfx(SfxId::Click));
            }
        },
        InputEvent::TriggerPress(Trigger::Left) => {
            if backend.is_button_held(CtrlButtons::RTRIGGER) {
                wm.close_all(sdi);
            } else {
                wm.cycle_focus(false, sdi);
                audio.send(AudioCmd::PlaySfx(SfxId::Click));
            }
        },
        InputEvent::TriggerPress(Trigger::Right) => {
            if backend.is_button_held(CtrlButtons::LTRIGGER) {
                wm.close_all(sdi);
            } else {
                wm.cycle_focus(true, sdi);
                audio.send(AudioCmd::PlaySfx(SfxId::Click));
            }
        },
        InputEvent::Quit => return DispatchResult::Quit,
        _ => {},
    }
    DispatchResult::SkipRest
}

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
                Ok(None) | Err(_) => {},
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

        // -- Browser input --
        InputEvent::ButtonPress(Button::Square) if *classic_view == ClassicView::Browser => {
            match psp::osk::OskBuilder::new("Enter URL")
                .max_chars(256)
                .initial_text(&br.url)
                .show()
            {
                Ok(Some(text)) => {
                    br.url = text;
                    br.status_msg = String::from("Press X to load");
                },
                Ok(None) | Err(_) => {},
            }
            backend.reinit_gu_frame();
        },
        InputEvent::ButtonPress(Button::Confirm) if *classic_view == ClassicView::Browser => {
            if !oasis_backend_psp::network::is_net_initialized() {
                if let Err(e) = oasis_backend_psp::network::ensure_net_init_pub() {
                    br.status_msg = format!("Net error: {e}");
                    backend.reinit_gu_frame();
                    return DispatchResult::Continue;
                }
                backend.reinit_gu_frame();
            }
            br.loading = true;
            br.status_msg = String::from("Loading...");
            br.content_lines.clear();
            io.send(IoCmd::HttpGet {
                url: br.url.clone(),
                tag: 0xBEEF,
            });
        },
        InputEvent::ButtonPress(Button::Up) if *classic_view == ClassicView::Browser => {
            br.scroll = br.scroll.saturating_sub(3);
        },
        InputEvent::ButtonPress(Button::Down) if *classic_view == ClassicView::Browser => {
            if br.scroll + 3 < br.content_lines.len() {
                br.scroll += 3;
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

// ---------------------------------------------------------------------------
// Helper: Dashboard Confirm (app launch)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn dispatch_dashboard_confirm(
    title: &str,
    classic_view: &mut ClassicView,
    app_mode: &mut AppMode,
    _dashboard: &mut DashboardState,
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    _audio: &AudioHandle,
    io: &IoHandle,
    fm: &mut FileManagerState,
    pv: &mut PhotoViewerState,
    mp: &mut MusicPlayerState,
    br: &mut BrowserState,
    radio: &mut RadioState,
    tv: &mut TvGuideState,
    backend: &mut PspBackend,
    dbg_log: &dyn Fn(&str),
) {
    match title {
        "Terminal" => {
            *classic_view = ClassicView::Terminal;
        },
        "File Manager" => {
            *classic_view = ClassicView::FileManager;
            fm.left.path = String::from("ms0:/");
            fm.left.loaded = false;
            fm.right.path = fm.left.path.clone();
            fm.right.loaded = false;
            fm.active_panel = 0;
        },
        "Photo Viewer" => {
            *classic_view = ClassicView::PhotoViewer;
            pv.viewing = false;
            pv.loaded = false;
        },
        "Music Player" => {
            *classic_view = ClassicView::MusicPlayer;
            mp.loaded = false;
        },
        "Browser" => {
            *classic_view = ClassicView::Browser;
            br.content_lines.clear();
            br.scroll = 0;
            br.loading = false;
            br.status_msg = String::from("Press [] to enter URL");
        },
        "Radio" => {
            *classic_view = ClassicView::Radio;
            radio.selected = 0;
            radio.scroll = 0;
        },
        "TV Guide" => {
            dbg_log("[TV] entering TV Guide view");
            *classic_view = ClassicView::TvGuide;
            if tv.channels.is_empty() {
                if !oasis_backend_psp::network::is_net_initialized() {
                    dbg_log("[TV] init network...");
                    if let Err(e) = oasis_backend_psp::network::ensure_net_init_pub() {
                        dbg_log(&format!("[TV] net init failed: {e}"));
                        backend.reinit_gu_frame();
                    } else {
                        dbg_log("[TV] net init OK");
                        backend.reinit_gu_frame();
                    }
                }
                dbg_log("[TV] parsing channel TOML...");
                if let Ok(config) = oasis_core::apps::tv_guide::ChannelConfig::from_toml(
                    oasis_core::apps::tv_guide::channel::DEFAULT_CHANNELS_TOML,
                ) {
                    dbg_log(&format!("[TV] parsed {} channels", config.channel.len()));
                    tv.channels = config.channel;
                    tv.catalogs = vec![None; tv.channels.len()];
                    let mut batch = Vec::new();
                    for (i, ch) in tv.channels.iter().enumerate() {
                        for src in &ch.source {
                            let api_path =
                                oasis_core::apps::tv_guide::ChannelCatalog::files_api_path(
                                    &src.item_id,
                                );
                            batch.push(TvCatalogRequest {
                                url: format!("http://archive.org{}", api_path,),
                                ch_idx: i,
                                item_id: src.item_id.clone(),
                                subfolder: src.subfolder.clone(),
                            });
                        }
                    }
                    io.send(IoCmd::TvCatalogFetchBatch { requests: batch });
                    dbg_log("[TV] catalog batch sent");
                } else {
                    dbg_log("[TV] TOML parse failed");
                }
            }
            tv.selected = 0;
            tv.scroll = 0;
        },
        _ => {
            // Apps without a Classic view: open in Desktop mode.
            if let Some(app) = APPS.iter().find(|a| a.title == title) {
                *app_mode = AppMode::Desktop;
                desktop::open_app_window(wm, sdi, app.id, app.title);
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Helper: Terminal Confirm (command execution)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn dispatch_terminal_confirm(
    backend: &mut PspBackend,
    term: &mut TerminalState,
    audio: &AudioHandle,
    mp: &mut MusicPlayerState,
    usb_storage: &mut Option<psp::usb::UsbStorageMode>,
    config: &mut psp::config::Config,
    current_preset: &mut skins::PspSkinPreset,
    active_theme: &mut ActiveTheme,
    skin_features: &SkinFeatures,
    dashboard: &mut DashboardState,
) {
    let cmd = term.input.clone();
    term.lines.push(format!("> {}", cmd));
    let (output, used_dialog) = match cmd.trim() {
        "sfx click" => {
            audio.send(AudioCmd::PlaySfx(SfxId::Click));
            (vec!["SFX: click".into()], false)
        },
        "sfx nav" => {
            audio.send(AudioCmd::PlaySfx(SfxId::Navigate));
            (vec!["SFX: navigate".into()], false)
        },
        "sfx error" => {
            audio.send(AudioCmd::PlaySfx(SfxId::Error));
            (vec!["SFX: error".into()], false)
        },
        "save" => match commands::save_terminal_history(&term.lines) {
            Ok(()) => (vec!["State saved.".into()], true),
            Err(e) => (vec![format!("Save failed: {e}")], true),
        },
        "load" => match commands::load_terminal_history() {
            Ok(lines) => {
                term.lines.clear();
                term.lines.extend(lines);
                (vec!["State restored.".into()], true)
            },
            Err(e) => (vec![format!("Load failed: {e}")], true),
        },
        "usb mount" => {
            if usb_storage.is_some() {
                (vec!["USB storage already active.".into()], false)
            } else {
                match psp::usb::start_bus() {
                    Ok(()) => match psp::usb::UsbStorageMode::activate() {
                        Ok(handle) => {
                            *usb_storage = Some(handle);
                            (
                                vec!["USB storage mode active. Connect cable to PC.".into()],
                                false,
                            )
                        },
                        Err(e) => (vec![format!("USB activate failed: {e}")], false),
                    },
                    Err(e) => (vec![format!("USB bus start failed: {e}")], false),
                }
            }
        },
        "usb unmount" | "usb eject" => {
            if usb_storage.take().is_some() {
                (vec!["USB storage mode deactivated.".into()], false)
            } else {
                (vec!["USB storage not active.".into()], false)
            }
        },
        "usb" | "usb status" => {
            let connected = psp::usb::is_connected();
            let established = psp::usb::is_established();
            let active = usb_storage.is_some();
            (
                vec![
                    format!(
                        "USB cable: {}",
                        if connected {
                            "connected"
                        } else {
                            "disconnected"
                        }
                    ),
                    format!(
                        "Storage mode: {}",
                        if active { "ACTIVE" } else { "inactive" }
                    ),
                    format!("Host mounted: {}", if established { "yes" } else { "no" },),
                ],
                false,
            )
        },
        _ if cmd.trim().starts_with("play ") => {
            let path = cmd.trim().strip_prefix("play ").unwrap().trim();
            audio.send(AudioCmd::LoadAndPlay(path.to_string()));
            mp.file_name = path.to_string();
            (vec![format!("Playing: {}", path)], false)
        },
        "pause" => {
            audio.send(AudioCmd::Pause);
            (vec!["Paused.".into()], false)
        },
        "resume" => {
            audio.send(AudioCmd::Resume);
            (vec!["Resumed.".into()], false)
        },
        "stop" => {
            audio.send(AudioCmd::Stop);
            (vec!["Stopped.".into()], false)
        },
        "skin" => {
            let names: Vec<String> = skins::PspSkinPreset::ALL
                .iter()
                .map(|p| {
                    let marker = if *p == *current_preset { ">" } else { " " };
                    format!("{} {}", marker, p.name())
                })
                .collect();
            let mut out = vec!["Skins (use 'skin NAME'):".into()];
            out.extend(names);
            (out, false)
        },
        _ if cmd.trim().starts_with("skin ") => {
            let key = cmd.trim().strip_prefix("skin ").unwrap().trim();
            let preset = skins::PspSkinPreset::from_key(key);
            if preset == *current_preset {
                (vec![format!("Already using '{}'.", key)], false)
            } else {
                *current_preset = preset;
                *active_theme = preset.to_active_theme();
                dashboard.config = DashboardConfig::from_features(skin_features, active_theme);
                config.set(
                    "skin",
                    psp::config::ConfigValue::Str(preset.key().to_string()),
                );
                let _ = config.save(CONFIG_PATH);
                (vec![format!("Skin changed to '{}'.", preset.name())], false)
            }
        },
        _ => {
            let r = commands::execute_command(&cmd, config);
            (r.lines, r.used_dialog)
        },
    };
    if used_dialog {
        backend.reinit_gu_frame();
    }
    for line in output {
        term.lines.push(line);
    }
    term.input.clear();
    term.scroll = 0;
    while term.lines.len() > 200 {
        term.lines.remove(0);
    }
}

// ---------------------------------------------------------------------------
// Helper: TV Guide Confirm (tune channel)
// ---------------------------------------------------------------------------

fn dispatch_tv_confirm(
    tv: &mut TvGuideState,
    io: &IoHandle,
    backend: &mut PspBackend,
    dbg_log: &dyn Fn(&str),
) {
    if tv.tuned.is_some() || tv.downloading {
        return;
    }
    dbg_log(&format!(
        "[TV] X pressed, tuning ch {} (catalogs={})",
        tv.selected,
        tv.catalogs.len()
    ));
    if tv.selected < tv.catalogs.len() {
        if let Some(catalog) = &tv.catalogs[tv.selected] {
            dbg_log(&format!(
                "[TV] catalog has {} episodes",
                catalog.episodes.len()
            ));
            let best =
                oasis_core::apps::tv_guide::select_smallest_for(&catalog.episodes, 20_000_000, 320);
            if let Some(ep) = best {
                dbg_log(&format!("[TV] episode: {} ({}B)", ep.title, ep.width));
                if !oasis_backend_psp::network::is_net_initialized() {
                    dbg_log("[TV] calling ensure_net_init_pub...");
                    if let Err(e) = oasis_backend_psp::network::ensure_net_init_pub() {
                        dbg_log(&format!("[TV] net init failed: {e}"));
                        tv.error_msg = format!("Net: {e}");
                        backend.reinit_gu_frame();
                        return;
                    }
                    dbg_log("[TV] net init OK");
                    backend.reinit_gu_frame();
                }
                let url = oasis_core::apps::tv_guide::ChannelCatalog::download_url(ep);
                dbg_log(&format!("[TV] starting download: {url}"));
                tv.now_playing = ep.title.clone();
                tv.downloading = true;
                tv.download_progress = 0.0;
                tv.error_msg.clear();
                tv.tuned = Some(tv.selected);
                io.send(IoCmd::VideoDownload {
                    url,
                    dest: String::from("ms0:/PSP/GAME/OASISOS/tv_cache.mp4"),
                    tag: 0xBB00,
                });
            } else {
                dbg_log("[TV] no suitable video found");
                tv.error_msg = String::from("No suitable video found");
            }
        } else {
            dbg_log("[TV] catalog still loading");
            tv.error_msg = String::from("Loading channel catalog...");
        }
    } else {
        dbg_log("[TV] tv_selected out of range");
    }
}
