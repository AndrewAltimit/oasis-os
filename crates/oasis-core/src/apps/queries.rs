//! App-specific query and interaction methods for `AppRunner`.

use super::app_trait::AppAction;
use super::runner::AppRunner;

use crate::vfs::Vfs;

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
            .as_mut()
            .and_then(|app| {
                app.as_any_mut()
                    .downcast_mut::<oasis_app_tv_guide::TvGuideApp>()
            })
            .map(|app| &mut app.guide)
    }

    /// Peek at a pending VFS IPC request without consuming it.
    pub fn peek_pending_request(&self) -> Option<&(String, String)> {
        if let Some(ref app) = self.delegate {
            return app.peek_pending_request();
        }
        self.pending_vfs_request.as_ref()
    }

    /// Take any pending VFS IPC request (returns path and data if present).
    pub fn take_pending_request(&mut self) -> Option<(String, String)> {
        if let Some(ref mut app) = self.delegate {
            return app.take_pending_request();
        }
        self.pending_vfs_request.take()
    }

    /// Set a pending VFS IPC request (used for auto-tune in tests).
    pub fn set_pending_request(&mut self, path: String, data: String) {
        // For TV Guide, set the request on the TvGuideApp delegate so
        // take_pending_request() can find it.
        if let Some(tv) = self.delegate_as_mut::<oasis_app_tv_guide::TvGuideApp>() {
            tv.pending_request = Some((path, data));
        } else {
            self.pending_vfs_request = Some((path, data));
        }
    }

    /// Refresh radio display from VFS status (called each frame when visible).
    pub fn refresh_radio(&mut self, vfs: &dyn Vfs) {
        if self.title != "Internet Radio" {
            return;
        }
        if let Some(ref mut app) = self.delegate {
            app.refresh(vfs);
            self.sync_from_delegate();
        }
    }

    /// Refresh the Video Embed app from VFS-published search results.
    pub fn refresh_video_embed(&mut self, vfs: &dyn Vfs) {
        if self.title != "Video Embed" {
            return;
        }
        if let Some(ref mut app) = self.delegate {
            app.refresh(vfs);
            self.sync_from_delegate();
        }
    }

    /// Refresh TV Guide text display after catalog changes.
    ///
    /// Delegates to `TvGuideApp::refresh_text()` through the delegate,
    /// then syncs the runner's cached fields.
    pub fn refresh_tv_text(&mut self) {
        if let Some(tv) = self.delegate_as_mut::<oasis_app_tv_guide::TvGuideApp>() {
            tv.refresh_text();
        }
        self.sync_from_delegate();
    }

    /// Handle a content-area click for the current app.
    pub fn handle_click(
        &mut self,
        lx: i32,
        ly: i32,
        cw: u32,
        ch: u32,
        fullscreen: bool,
    ) -> AppAction {
        if let Some(ref mut app) = self.delegate {
            let action = app.handle_click(lx, ly, cw, ch, fullscreen);
            self.redraw_pending = true;
            self.sync_from_delegate();
            return action;
        }

        AppAction::None
    }

    /// Let the delegate apply its queued VFS mutations (see
    /// [`crate::apps::App::apply_vfs_ops`]). Hosts call this once per frame
    /// for every open runner.
    pub fn apply_vfs_ops(&mut self, vfs: &mut dyn Vfs) {
        if let Some(ref mut app) = self.delegate
            && app.apply_vfs_ops(vfs)
        {
            self.redraw_pending = true;
            self.sync_from_delegate();
        }
    }

    /// Forward an `App::refresh` call to the delegate. Apps that don't
    /// override `refresh` get the no-op default; apps like the file
    /// manager use this to apply pending vfs work queued from
    /// `handle_click` (which doesn't get a vfs reference).
    pub fn refresh_app(&mut self, vfs: &dyn Vfs) {
        if let Some(ref mut app) = self.delegate {
            app.refresh(vfs);
            self.sync_from_delegate();
        }
    }
}
