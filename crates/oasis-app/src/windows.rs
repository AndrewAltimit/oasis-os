//! Closing app windows: the one close path behind the titlebar button,
//! Cancel on a non-app window, an app's own `Exit`, `wm close` and MCP
//! `close_window`.
//!
//! Every close exits kiosk fullscreen, closes the WM window, stops the
//! radio / music the app owned, retires the runner (so the shell releases
//! its backend resources next frame, see `ContentLayer::retired_runners`),
//! drops the browser widget for the browser window, plays the close sound
//! and falls back to the dashboard when the last window went away.

use oasis_core::apps::{AppAction, OwnedMedia};
use oasis_core::sdi::SdiRegistry;
use oasis_core::ui_sound::UiSound;
use oasis_core::vfs::MemoryVfs;

use crate::app_state::{AppState, Mode};
use crate::launch::BROWSER_WIN;

/// Who is closing the window, which decides whether the app gets a say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseHow {
    /// The user clicked the titlebar close button (or pressed Cancel on a
    /// window that is not an app). The app is asked first
    /// ([`AppRunner::request_close`](oasis_core::apps::AppRunner::request_close)):
    /// an editor with unsaved changes answers with its prompt instead.
    UserChrome,
    /// The app itself answered `Exit` (Cancel at top level, File > Close,
    /// "Save & close" finished). Not asked again.
    AppExit,
    /// Closed by a program (`wm close <id>`, MCP `close_window`). Without
    /// `force` the app is asked first, exactly like the titlebar button;
    /// with `force` it is closed regardless (unsaved edits are lost).
    Programmatic { force: bool },
}

/// Result of [`close_app_window`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    /// The window is gone.
    Closed,
    /// The app declined (it is showing a confirmation prompt); the window
    /// stays open and focused.
    Declined,
    /// No window with that id.
    NotFound,
}

/// Close window `id` (see the module docs).
pub fn close_app_window(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
    id: &str,
    how: CloseHow,
) -> CloseOutcome {
    // The WM may already have dropped the window (its own titlebar-button
    // handling): the shell side still has to be cleaned up then.
    let has_window = state.wm.get_window(id).is_some();
    let has_content = state.content.open_runners.iter().any(|(rid, _)| rid == id)
        || (id == BROWSER_WIN && state.content.browser.is_some());
    if !has_window && !has_content {
        return CloseOutcome::NotFound;
    }
    let ask = matches!(
        how,
        CloseHow::UserChrome | CloseHow::Programmatic { force: false }
    );
    if ask
        && let Some((_, runner)) = state
            .content
            .open_runners
            .iter_mut()
            .find(|(rid, _)| rid == id)
        && runner.request_close(vfs) != AppAction::Exit
    {
        // Show the app's prompt to the user.
        let _ = state.wm.focus_window(id, sdi);
        return CloseOutcome::Declined;
    }

    if state.content.fullscreen_app.as_deref() == Some(id) {
        if has_window {
            let _ = state.wm.exit_fullscreen(id, sdi);
        }
        state.content.fullscreen_app = None;
    }
    if has_window && let Err(e) = state.wm.close_window(id, sdi) {
        log::warn!("close_window({id}): {e}");
    }
    stop_owned_media(state, id);
    retire_runner(state, id);
    if id == BROWSER_WIN {
        state.content.browser = None;
    }
    state.ui_sounds.push(UiSound::Close);
    // `wm close` typed in the fullscreen terminal must not yank the user
    // out of it: only the desktop falls back to the dashboard.
    if state.wm.window_count() == 0 && state.mode == Mode::Desktop {
        state.mode = Mode::Dashboard;
    }
    CloseOutcome::Closed
}

/// Stop the radio / music when the closing runner's app owns it
/// ([`OwnedMedia`] in its registration: Internet Radio, Music Player):
/// otherwise audio keeps playing with no UI to control it (the apps' own
/// `stop` IPC only runs on their Cancel path).
fn stop_owned_media(state: &mut AppState, id: &str) {
    let media = state
        .content
        .open_runners
        .iter()
        .find(|(rid, _)| rid == id)
        .and_then(|(_, runner)| state.content.apps.owned_media(&runner.title));
    match media {
        Some(OwnedMedia::Radio) => stop_radio(state),
        Some(OwnedMedia::Music) => crate::media_controller::shutdown(state),
        None => {},
    }
}

/// Tear down active radio playback and any pending network work.
/// Idempotent.
pub(crate) fn stop_radio(state: &mut AppState) {
    let _ = state
        .radio
        .manager
        .process_request("stop", state.audio_backend.as_mut());
    state.radio.archive_catalog = None;
    state.radio.pending_catalog_fetch = None;
    state.radio.pending_source_fetch = None;
    if let Some(mut src) = state.radio.source.take() {
        src.disconnect();
    }
}

/// Remove the runner of window `id`, parking it until the shell releases
/// its backend resources (see `ContentLayer::retired_runners`).
fn retire_runner(state: &mut AppState, id: &str) {
    let content = &mut state.content;
    while let Some(idx) = content.open_runners.iter().position(|(rid, _)| rid == id) {
        let (_, runner) = content.open_runners.remove(idx);
        content.retired_runners.push(runner);
    }
}
