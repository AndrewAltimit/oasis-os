//! App-specific query methods for `AppRunner`.

use super::runner::AppRunner;

impl AppRunner {
    /// Get mutable reference to the TV guide state.
    ///
    /// Accesses the `TvGuideApp` delegate and returns a reference to
    /// its inner `TvGuideState`. Used by external code (tv_controller,
    /// WASM backend) to inject catalogs and update fetch status.
    ///
    /// Unlike [`AppRunner::delegate_as_mut`] this does not schedule a
    /// redraw: hosts poll it every frame, and the guide's visible changes
    /// go through [`AppRunner::refresh_tv_text`] (or happen while video
    /// playback already forces redraws).
    pub fn tv_guide_state(&mut self) -> Option<&mut oasis_app_tv_guide::guide::TvGuideState> {
        self.delegate
            .as_any_mut()
            .downcast_mut::<oasis_app_tv_guide::TvGuideApp>()
            .map(|app| &mut app.guide)
    }

    /// Refresh TV Guide text display after catalog changes.
    ///
    /// Delegates to `TvGuideApp::refresh_text()` and schedules a redraw.
    pub fn refresh_tv_text(&mut self) {
        if let Some(tv) = self.delegate_as_mut::<oasis_app_tv_guide::TvGuideApp>() {
            tv.refresh_text();
        }
    }
}
