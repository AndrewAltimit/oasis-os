//! TV Guide subsystem controller.
//!
//! Extracted from the main event loop to reduce complexity. Handles TV catalog
//! fetching, video player ticking, tune/untune requests, and audio streaming.

mod catalog;
mod cdn_failover;
mod download;
mod player;
mod seek;
mod streaming_buffer;
mod tune;

// Re-export public items so existing `use crate::tv_controller::*` paths work.
#[cfg(feature = "_video")]
pub(crate) use streaming_buffer::{MIN_PREBUFFER, StreamingInner};

use crate::app_state::AppState;
use oasis_core::apps::AppRunner;
use oasis_core::backend::SdiBackend;
use oasis_core::vfs::Vfs;

/// Process one frame of TV state: catalog fetching, tune requests, video
/// player ticking, and untune detection.
pub fn tick(state: &mut AppState, backend: &mut impl SdiBackend, vfs: &mut dyn Vfs) {
    publish_guide_status(state, vfs);
    catalog::poll_catalog_fetch(state);
    catalog::start_catalog_fetch_if_needed(state);
    tune::handle_tune_requests(state, backend, vfs);
    player::tick_video_player(state, backend);
    player::detect_untune(state, backend);
    player::auto_advance_episode(state, backend);
}

/// Keep [`TV_STATUS_PATH`] saying whether a TV Guide is open, so `tv tune`
/// can tell the user its request has nothing to tune instead of reporting
/// success. Written only when the answer changes.
///
/// [`TV_STATUS_PATH`]: oasis_core::ipc::TV_STATUS_PATH
fn publish_guide_status(state: &mut AppState, vfs: &mut dyn Vfs) {
    use oasis_core::ipc::TvStatus;
    let open = find_tv_guide_runner(&mut state.content.open_runners).is_some();
    if state.tv.guide_status_published == Some(open) {
        return;
    }
    let status = if open {
        TvStatus::GuideOpen
    } else {
        TvStatus::GuideClosed
    };
    if vfs
        .write(oasis_core::ipc::TV_STATUS_PATH, status.as_line().as_bytes())
        .is_ok()
    {
        state.tv.guide_status_published = Some(open);
    }
}

/// The open TV Guide window's runner, if any.
fn find_tv_guide_runner(open_runners: &mut [(String, AppRunner)]) -> Option<&mut AppRunner> {
    open_runners
        .iter_mut()
        .map(|(_, runner)| runner)
        .find(|runner| runner.title == oasis_apps::titles::TV_GUIDE)
}

#[cfg(all(test, feature = "_video"))]
mod tests;

#[cfg(all(test, feature = "_video"))]
mod e2e_tests;
