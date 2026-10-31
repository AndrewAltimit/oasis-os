//! Dashboard, fullscreen terminal and on-screen keyboard input.

use oasis_core::input::{Button, InputEvent, Trigger};
use oasis_core::osk::{OskConfig, OskState};
use oasis_core::sdi::SdiRegistry;
use oasis_core::startmenu::StartMenuAction;
use oasis_core::transition;
use oasis_core::vfs::MemoryVfs;

use crate::app_state::{AppState, Mode};
use crate::launch::LaunchOpts;
use crate::ui_sound::UiSound;
use oasis_core::terminal_sdi;

use crate::{icon_drag, launch, terminal_input};

use super::{
    InputResult, handle_start_menu_action, home_mode, launch_dashboard_icon, start_menu_button,
    update_dashboard_hover,
};

/// Overlay a fade on dashboard page changes for skins with
/// `[transition] page_style = "fade"` (the icon slide is suppressed by
/// `DashboardConfig::page_style`). Default "slide" skins are untouched.
fn apply_page_change_fade(state: &mut AppState) {
    if state.active_theme.transition_page_style == "fade" {
        state.active_transition = Some(transition::fade_in_custom(
            state.config.screen_width,
            state.config.screen_height,
            state.active_theme.page_slide_duration.max(1),
        ));
    }
}

/// Handle input in OSK mode.
pub fn handle_osk_input(
    event: &InputEvent,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
) -> InputResult {
    if let Some(ref mut osk_state) = state.osk {
        match event {
            InputEvent::Quit => return InputResult::Quit,
            InputEvent::Backspace => {
                osk_state.buffer.pop();
            },
            InputEvent::ButtonPress(btn) => {
                osk_state.handle_input(btn);
                let for_terminal = osk_state.config.title == TERMINAL_OSK_TITLE;
                let back_to = if for_terminal {
                    Mode::Terminal
                } else {
                    Mode::Dashboard
                };
                if let Some(text) = osk_state.confirmed_text() {
                    if for_terminal {
                        // The OSK edited the prompt line: put it back.
                        state.terminal.session.set_line(text);
                    } else {
                        state.terminal.print(format!("[OSK] Input: {text}"));
                        terminal_sdi::trim_scrollback(&mut state.terminal.output_lines);
                    }
                    osk_state.hide_sdi(sdi);
                    state.osk = None;
                    state.mode = back_to;
                } else if osk_state.is_cancelled() {
                    if !for_terminal {
                        state.terminal.print("[OSK] Cancelled");
                        terminal_sdi::trim_scrollback(&mut state.terminal.output_lines);
                    }
                    osk_state.hide_sdi(sdi);
                    state.osk = None;
                    state.mode = back_to;
                }
            },
            _ => {},
        }
    }
    InputResult::Continue
}

/// OSK title for the general-purpose keyboard (result echoed to the
/// terminal scrollback).
const DEFAULT_OSK_TITLE: &str = "On-Screen Keyboard";
/// OSK title when it edits the terminal prompt (result returns to it).
const TERMINAL_OSK_TITLE: &str = "Terminal Input";

/// Handle input in Dashboard/Terminal modes and global keys.
pub fn handle_default_input(
    event: &InputEvent,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) -> InputResult {
    match event {
        InputEvent::Quit => return InputResult::Quit,
        // An open start menu owns the d-pad, Confirm and Cancel (Cancel
        // closes it): checked before the dashboard's own Confirm (launch
        // the selected icon) and Cancel (quit) bindings.
        InputEvent::ButtonPress(btn)
            if state.mode == Mode::Dashboard && state.ui.start_menu.open =>
        {
            return start_menu_button(*btn, state, sdi, vfs);
        },
        InputEvent::ButtonPress(Button::Cancel) if state.mode == Mode::Dashboard => {
            return InputResult::Quit;
        },

        // Launch app from dashboard as floating window.
        InputEvent::ButtonPress(Button::Confirm) if state.mode == Mode::Dashboard => {
            state.ui.dashboard.trigger_press_flash();
            if crate::render::dashboard_icons_visible(state)
                && let Some(app) = state.ui.dashboard.selected_app().cloned()
            {
                launch::launch(state, sdi, vfs, &app, LaunchOpts::USER);
            }
        },

        // Pointer click on dashboard: start menu takes priority.
        InputEvent::PointerClick { x, y } if state.mode == Mode::Dashboard => {
            // A skin without a start menu never draws it: its button's hit
            // box must not open an invisible menu that takes the d-pad.
            if state.skin.features.start_menu && state.ui.start_menu.hit_test_button(*x, *y) {
                state.ui.start_menu.toggle();
                state.ui_sounds.push(UiSound::Click);
                return InputResult::Continue;
            }
            if state.ui.start_menu.open {
                if let Some(action) = state.ui.start_menu.hit_test_item(*x, *y) {
                    state.ui.start_menu.close();
                    if action == StartMenuAction::Exit {
                        return InputResult::Quit;
                    }
                    handle_start_menu_action(&action, state, sdi, vfs);
                } else {
                    state.ui.start_menu.close();
                }
                return InputResult::Continue;
            }
            if crate::render::dashboard_icons_visible(state)
                && let Some(idx) = state.ui.dashboard.icon_at(*x, *y)
            {
                if state.ui.dashboard.config.free_layout {
                    // Free layout: arm a drag; the release decides between
                    // drop-commit, select, and launch.
                    icon_drag::begin(state, idx, *x, *y);
                } else {
                    launch_dashboard_icon(state, sdi, vfs, idx, true);
                }
            }
        },

        // Free-layout icon drag tracking (no-ops when nothing is armed).
        InputEvent::CursorMove { x, y } if state.mode == Mode::Dashboard => {
            icon_drag::on_move(state, *x, *y);
            update_dashboard_hover(state, *x, *y, true);
            // Hover focus (B6): in free layout the selection follows the
            // pointer, driving the existing focus_scale / focus_glow /
            // selection-highlight micro-motion. Grid skins keep their
            // click/d-pad selection unchanged.
            if state.ui.dashboard.config.free_layout
                && !icon_drag::active(state)
                && let Some(idx) = state.ui.dashboard.icon_at(*x, *y)
            {
                state.ui.dashboard.selected = idx;
            }
        },
        InputEvent::PointerRelease { x, y } if state.mode == Mode::Dashboard => {
            if let icon_drag::ReleaseAction::Launch(idx) = icon_drag::on_release(state, vfs, *x, *y)
            {
                launch_dashboard_icon(state, sdi, vfs, idx, true);
            }
        },

        InputEvent::ButtonPress(Button::Start) => {
            state.mode = match state.mode {
                Mode::Dashboard => Mode::Terminal,
                Mode::Terminal => home_mode(state),
                Mode::Osk => Mode::Osk,
                Mode::Desktop => Mode::Desktop,
            };
        },
        InputEvent::ButtonPress(Button::Select) if state.mode != Mode::Osk => {
            // Opened from the terminal, the OSK edits the prompt line and
            // hands the result back to it (see `handle_osk_input`).
            let from_terminal = state.mode == Mode::Terminal;
            let (title, initial) = if from_terminal {
                (
                    TERMINAL_OSK_TITLE,
                    state.terminal.session.buffer().to_string(),
                )
            } else {
                (DEFAULT_OSK_TITLE, String::new())
            };
            let osk_cfg = OskConfig {
                title: title.to_string(),
                ..OskConfig::for_screen(state.active_theme.screen_w, state.active_theme.screen_h)
            };
            state.osk = Some(OskState::new(osk_cfg, &initial));
            state.mode = Mode::Osk;
            log::info!("OSK opened");
        },

        // L trigger: cycle top tabs (status bar).
        InputEvent::TriggerPress(Trigger::Left) if state.mode == Mode::Dashboard => {
            state.ui.status_bar.next_tab();
            state.ui.bottom_bar.l_pressed = true;
        },
        InputEvent::TriggerRelease(Trigger::Left) => {
            state.ui.bottom_bar.l_pressed = false;
        },

        // R trigger: cycle media category tabs (bottom bar).
        InputEvent::TriggerPress(Trigger::Right) if state.mode == Mode::Dashboard => {
            state.ui.bottom_bar.next_tab();
            state.ui.bottom_bar.r_pressed = true;
            state.active_transition = Some(transition::fade_in_custom(
                state.config.screen_width,
                state.config.screen_height,
                state.skin.features.transition_fade_frames.unwrap_or(15),
            ));
        },
        InputEvent::TriggerRelease(Trigger::Right) => {
            state.ui.bottom_bar.r_pressed = false;
        },

        // Dashboard input: D-pad navigation.
        InputEvent::ButtonPress(btn) if state.mode == Mode::Dashboard => match btn {
            Button::Up | Button::Down | Button::Left | Button::Right
                if crate::render::dashboard_icons_visible(state) =>
            {
                state.ui.dashboard.handle_input(btn);
                state.ui_sounds.push_nav(state.frame_counter);
            },
            Button::Triangle if crate::render::dashboard_icons_visible(state) => {
                state.ui.dashboard.next_page();
                state.ui.bottom_bar.current_page = state.ui.dashboard.page;
                state.ui_sounds.push_nav(state.frame_counter);
                apply_page_change_fade(state);
            },
            Button::Square if crate::render::dashboard_icons_visible(state) => {
                state.ui.dashboard.prev_page();
                state.ui.bottom_bar.current_page = state.ui.dashboard.page;
                state.ui_sounds.push_nav(state.frame_counter);
                apply_page_change_fade(state);
            },
            _ => {},
        },

        // Terminal input: line editing (text, Backspace, Tab, d-pad,
        // Confirm, Square) goes to the shell session.
        InputEvent::TextInput(_)
        | InputEvent::Backspace
        | InputEvent::Tab
        | InputEvent::ButtonPress(_)
            if state.mode == Mode::Terminal
                && terminal_input::handle_event(event, state, sdi, vfs) => {},
        InputEvent::ButtonPress(Button::Cancel) if state.mode == Mode::Terminal => {
            // A terminal-only skin (no dashboard, no window manager) has
            // nowhere to go back to: hiding the terminal would blank the
            // screen until its next change rebuilt it.
            let home = home_mode(state);
            if home != Mode::Terminal {
                terminal_sdi::set_terminal_visible(sdi, false);
                state.mode = home;
                state.ui_sounds.push(UiSound::Close);
            }
        },

        InputEvent::MouseWheel { delta } if state.mode == Mode::Terminal => {
            terminal_input::scroll(state, *delta);
        },

        _ => {},
    }
    InputResult::Continue
}
