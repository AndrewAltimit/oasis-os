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
    catalog::poll_catalog_fetch(state);
    catalog::start_catalog_fetch_if_needed(state);
    tune::handle_tune_requests(state, backend, vfs);
    player::tick_video_player(state, backend);
    player::detect_untune(state, backend);
    player::auto_advance_episode(state, backend);
}

/// The open TV Guide window's runner, if any.
fn find_tv_guide_runner(open_runners: &mut [(String, AppRunner)]) -> Option<&mut AppRunner> {
    open_runners
        .iter_mut()
        .map(|(_, runner)| runner)
        .find(|runner| runner.title == "TV Guide")
}

#[cfg(all(test, feature = "_video"))]
mod tests;

#[cfg(all(test, feature = "_video"))]
mod e2e_tests;
