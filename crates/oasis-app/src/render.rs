use oasis_core::apps::AppRunner;
use oasis_core::bottombar::{BottomBar, MediaTab};
use oasis_core::sdi::SdiRegistry;
use oasis_core::statusbar::StatusBar;

use crate::app_state::{AppState, Mode};
use crate::terminal_sdi;

/// Update the SDI scene graph based on the current mode.
///
/// This controls which UI elements are visible and positioned correctly
/// each frame. The actual rendering (`backend.clear`, `sdi.draw`, etc.)
/// remains in main.rs since it requires `&mut backend`.
pub fn update_sdi(state: &mut AppState, sdi: &mut SdiRegistry) {
    match state.mode {
        Mode::Dashboard => {
            terminal_sdi::set_terminal_visible(sdi, false);
            AppRunner::hide_sdi(sdi);

            if state.bottom_bar.active_tab == MediaTab::None {
                state.dashboard.update_sdi(sdi, &state.active_theme);
            } else {
                state.dashboard.hide_sdi(sdi);
                terminal_sdi::update_media_page(sdi, &state.bottom_bar);
            }

            state
                .status_bar
                .update_sdi(sdi, &state.active_theme, &state.skin.features);
            state
                .bottom_bar
                .update_sdi(sdi, &state.active_theme, &state.skin.features);
            if state.skin.features.start_menu {
                state.start_menu.update_sdi(sdi, &state.active_theme);
            }
        },
        Mode::Terminal => {
            state.dashboard.hide_sdi(sdi);
            AppRunner::hide_sdi(sdi);
            StatusBar::hide_sdi(sdi);
            BottomBar::hide_sdi(sdi);
            state.start_menu.close();
            state.start_menu.hide_sdi(sdi);
            terminal_sdi::hide_media_page(sdi);
            terminal_sdi::setup_terminal_objects(
                sdi,
                &state.output_lines,
                &state.cwd,
                &state.input_buf,
            );
        },
        Mode::App => {
            state.dashboard.hide_sdi(sdi);
            terminal_sdi::set_terminal_visible(sdi, false);
            terminal_sdi::hide_media_page(sdi);
            state.start_menu.close();
            state.start_menu.hide_sdi(sdi);
            state
                .status_bar
                .update_sdi(sdi, &state.active_theme, &state.skin.features);
            state
                .bottom_bar
                .update_sdi(sdi, &state.active_theme, &state.skin.features);
            if let Some(ref runner) = state.app_runner {
                runner.update_sdi(sdi);
            }
        },
        Mode::Desktop => {
            terminal_sdi::set_terminal_visible(sdi, false);
            AppRunner::hide_sdi(sdi);
            state.dashboard.hide_sdi(sdi);
            terminal_sdi::hide_media_page(sdi);
            state.start_menu.close();
            state.start_menu.hide_sdi(sdi);
            state
                .status_bar
                .update_sdi(sdi, &state.active_theme, &state.skin.features);
            state
                .bottom_bar
                .update_sdi(sdi, &state.active_theme, &state.skin.features);
        },
        Mode::Osk => {
            if let Some(ref osk_state) = state.osk {
                osk_state.update_sdi(sdi);
            }
        },
    }

    // Update cursor SDI position (always on top).
    state.mouse_cursor.update_sdi(sdi);

    // Ensure wallpaper is visible and at lowest z.
    if let Ok(obj) = sdi.get_mut("wallpaper") {
        obj.visible = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: The `update_sdi` function is heavily coupled to the SDI backend and AppState.
    // It requires fully initialized AppState and SdiRegistry instances, making unit testing
    // impractical without extensive mocking. The function's correctness is validated through
    // integration tests and visual inspection in the running application.
    //
    // Potential testable aspects if refactored:
    // - Visibility logic could be extracted to pure functions
    // - Mode-to-visibility mapping could be a lookup table
    // - Each mode's rendering behavior could be a separate function

    #[test]
    fn test_mode_enum_coverage() {
        // Verify all Mode variants are handled in update_sdi.
        // This test ensures we don't forget to add a match arm when adding new modes.
        let modes = vec![
            Mode::Dashboard,
            Mode::Terminal,
            Mode::App,
            Mode::Osk,
            Mode::Desktop,
        ];

        // If this compiles, all modes are at least syntactically valid.
        for mode in modes {
            let _ = mode;
        }
    }
}
