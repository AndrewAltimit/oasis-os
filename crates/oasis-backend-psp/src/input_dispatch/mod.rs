//! Input event dispatch for Classic and Desktop modes.
//!
//! Extracts the large `match event { ... }` blocks from the main loop into
//! dedicated functions, keeping the main loop as a thin orchestrator.

mod classic;
mod helpers;

use psp::sys::CtrlButtons;

use oasis_backend_psp::{
    AudioCmd, Button, InputEvent, PspBackend, SdiRegistry, SfxId, Trigger, WindowManager,
};

use oasis_backend_psp::AudioHandle;

use oasis_core::dashboard::DashboardState;

use crate::app_states::TerminalState;
use crate::desktop;
use crate::types::*;

pub(crate) use classic::dispatch_classic;

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
