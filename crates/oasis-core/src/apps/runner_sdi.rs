//! SDI rendering for the app runner (update_sdi / hide_sdi).

use crate::active_theme::ActiveTheme;
use crate::sdi::SdiRegistry;

use super::runner::AppRunner;

impl AppRunner {
    /// Render the app screen as SDI objects (single-display-interface mode).
    pub fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        // Delegate to extracted app.
        if let Some(ref mut app) = self.delegate {
            app.update_sdi(sdi, at);
        }
    }

    /// Hide all app-related SDI objects.
    pub fn hide_sdi(sdi: &mut SdiRegistry) {
        // Generic chrome + line pools (change-detecting: this runs every
        // frame outside app mode and must leave an idle scene clean).
        oasis_app_core::render::hide_app_sdi(sdi);

        // Hide TV Guide objects. Both TV Guide render paths (grid and
        // expanded video) create `tv_hdr_bg` before any other `tv_*`
        // object, so until it exists there is nothing to hide — skip the
        // ~200 name formats + lookups the full pass costs every frame.
        if sdi.contains("tv_hdr_bg") {
            oasis_app_tv_guide::TvGuideState::hide_sdi(sdi);
        }

        // Hide Text Editor Notepad chrome. The authoritative cleanup
        // lives on the text-editor crate, which owns the pool sizes
        // for the menu/dropdown/line slots. `np_menu_bg` is the first
        // `np_*` object its renderer creates (same skip as above).
        if sdi.contains("np_menu_bg") {
            oasis_app_text_editor::hide_notepad_sdi_objects(sdi);
        }
    }
}
