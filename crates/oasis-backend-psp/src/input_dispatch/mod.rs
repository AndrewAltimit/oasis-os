//! Unified input event dispatch.
//!
//! Single dispatch function handles all input based on the current WM state:
//! - **Dashboard** (no kiosk app): cursor-based icon selection + WM windows.
//! - **Kiosk app**: Input routed to app-specific handler by window ID.

mod app_input;
mod helpers;

use psp::sys::CtrlButtons;

use oasis_backend_psp::threading::IoHandle;
use oasis_backend_psp::{
    AudioCmd, AudioHandle, Button, InputEvent, PspBackend, SdiRegistry, SfxId, Trigger,
    WindowManager,
};

use oasis_core::active_theme::ActiveTheme;
use oasis_core::dashboard::DashboardState;
use oasis_core::skin::SkinFeatures;

use crate::app_states::{
    BrowserState, FileManagerState, MusicPlayerState, PhotoViewerState, RadioState, TerminalState,
    TvGuideState,
};
use crate::desktop;
use crate::skins;
use crate::types::*;

pub(crate) use app_input::dispatch_app_input;

/// Return type for input dispatch.
pub(crate) enum DispatchResult {
    /// Continue processing the next event.
    Continue,
    /// Skip remaining events this frame.
    SkipRest,
    /// Exit the main loop.
    Quit,
}

/// Handle a single input event in the unified desktop mode.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_unified(
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

        // -- Select: minimize focused window, return to dashboard --
        InputEvent::ButtonPress(Button::Select) => {
            if *kiosk_app != KioskApp::None {
                if let Some(wid) = kiosk_app.window_id() {
                    let _ = wm.exit_fullscreen(wid, sdi);
                    let _ = wm.minimize_window(wid, sdi);
                }
                *kiosk_app = KioskApp::None;
            }
        },

        // -- Start: toggle kiosk/windowed for focused app, or open terminal --
        InputEvent::ButtonPress(Button::Start) => {
            if *kiosk_app != KioskApp::None {
                // Kiosk → windowed: exit fullscreen but keep window visible.
                if let Some(wid) = kiosk_app.window_id() {
                    let _ = wm.exit_fullscreen(wid, sdi);
                }
                *kiosk_app = KioskApp::None;
            } else {
                // Try active window first, then cursor hit-test, then
                // topmost visible window.
                let target = wm
                    .active_window()
                    .map(|s| s.to_string())
                    .or_else(|| {
                        let (cx, cy) = backend.cursor_pos();
                        wm.window_at(cx, cy).map(|s| s.to_string())
                    })
                    .or_else(|| wm.topmost_visible().map(|s| s.to_string()));

                if let Some(win_id) = target {
                    let ka = KioskApp::from_window_id(&win_id);
                    if ka != KioskApp::None {
                        desktop::open_app_window(wm, sdi, &win_id, "", true);
                        *kiosk_app = ka;
                    } else {
                        let _ = wm.enter_fullscreen(&win_id, sdi);
                    }
                } else {
                    // No windows at all: open terminal in kiosk.
                    desktop::open_app_window(wm, sdi, "terminal", "Terminal", true);
                    *kiosk_app = KioskApp::Terminal;
                }
            }
        },

        // -- L/R triggers: cycle window focus --
        InputEvent::TriggerPress(Trigger::Left) => {
            if backend.is_button_held(CtrlButtons::RTRIGGER) {
                wm.close_all(sdi);
                *kiosk_app = KioskApp::None;
            } else {
                wm.cycle_focus(false, sdi);
                audio.send(AudioCmd::PlaySfx(SfxId::Click));
            }
        },
        InputEvent::TriggerPress(Trigger::Right) => {
            if backend.is_button_held(CtrlButtons::LTRIGGER) {
                wm.close_all(sdi);
                *kiosk_app = KioskApp::None;
            } else {
                wm.cycle_focus(true, sdi);
                audio.send(AudioCmd::PlaySfx(SfxId::Click));
            }
        },

        // -- Cursor movement forwarded to WM --
        InputEvent::CursorMove { x, y } => {
            let move_event = InputEvent::CursorMove { x: *x, y: *y };
            wm.handle_input(&move_event, sdi);
        },

        // All other events: route based on kiosk state.
        _ => {
            if *kiosk_app != KioskApp::None {
                return dispatch_app_input(
                    event,
                    backend,
                    kiosk_app,
                    dashboard,
                    wm,
                    sdi,
                    audio,
                    io,
                    term,
                    fm,
                    pv,
                    mp,
                    br,
                    radio,
                    tv,
                    usb_storage,
                    config,
                    current_preset,
                    active_theme,
                    skin_features,
                    dbg_log,
                );
            }
            // -- Dashboard input (cursor-based) --
            return dispatch_dashboard(
                event,
                backend,
                kiosk_app,
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
                icons_hidden,
                dbg_log,
            );
        },
    }
    DispatchResult::Continue
}

/// Handle dashboard-level input (cursor-based icon clicks + WM windows).
///
/// Z-order: WM windows get priority. If the cursor click doesn't hit a
/// window, fall through to desktop icon hit testing.
#[allow(clippy::too_many_arguments)]
fn dispatch_dashboard(
    event: &InputEvent,
    backend: &mut PspBackend,
    kiosk_app: &mut KioskApp,
    dashboard: &mut DashboardState,
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    audio: &AudioHandle,
    io: &IoHandle,
    fm: &mut FileManagerState,
    pv: &mut PhotoViewerState,
    mp: &mut MusicPlayerState,
    br: &mut BrowserState,
    radio: &mut RadioState,
    tv: &mut TvGuideState,
    _icons_hidden: &mut bool,
    dbg_log: &dyn Fn(&str),
) -> DispatchResult {
    match event {
        // Confirm = click at cursor position.
        InputEvent::ButtonPress(Button::Confirm) => {
            let (cx, cy) = backend.cursor_pos();

            // 1) Try WM windows first (z-order priority).
            let ptr_event = InputEvent::PointerClick { x: cx, y: cy };
            let wm_event = wm.handle_input(&ptr_event, sdi);
            match &wm_event {
                oasis_backend_psp::WmEvent::None
                | oasis_backend_psp::WmEvent::DesktopClick(_, _) => {
                    // Click didn't hit a window — try desktop icons.
                    if let Some(idx) =
                        desktop::hit_test_dashboard_icon(cx, cy, dashboard.page)
                    {
                        if idx < APPS.len() {
                            audio.send(AudioCmd::PlaySfx(SfxId::Navigate));
                            helpers::dispatch_dashboard_confirm(
                                APPS[idx].title,
                                kiosk_app,
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
                    }
                },
                _ => {
                    // Window was clicked — handle WM event (close, content click, etc.).
                    desktop::handle_wm_event(
                        &wm_event,
                        &mut Vec::new(), // term_lines not needed here
                        wm,
                        sdi,
                        dashboard.page,
                    );
                },
            }
        },
        InputEvent::ButtonRelease(Button::Confirm) => {
            let (cx, cy) = backend.cursor_pos();
            let ptr_event = InputEvent::PointerRelease { x: cx, y: cy };
            wm.handle_input(&ptr_event, sdi);
        },

        // D-pad pages the dashboard when no windows are focused.
        InputEvent::ButtonPress(Button::Left) => {
            if dashboard.page > 0 {
                dashboard.page -= 1;
            }
        },
        InputEvent::ButtonPress(Button::Right) => {
            if dashboard.page + 1 < dashboard.page_count() {
                dashboard.page += 1;
            }
        },

        _ => {},
    }
    DispatchResult::Continue
}
