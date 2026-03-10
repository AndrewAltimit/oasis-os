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
        let fixed = [
            "app_bg",
            "app_title_bg",
            "app_title_text",
            "app_scroll",
            "app_divider",
            "app_sel_bg",
            "app_sel_accent",
        ];
        for name in &fixed {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
        // Hide up to a generous upper bound (handles all resolutions).
        for i in 0..100 {
            let name = format!("app_line_{i}");
            if !sdi.contains(&name) {
                break;
            }
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = false;
            }
        }
        for i in 0..100 {
            let lp = format!("app_lp_line_{i}");
            if !sdi.contains(&lp) {
                break;
            }
            let rp = format!("app_rp_line_{i}");
            if let Ok(obj) = sdi.get_mut(&lp) {
                obj.visible = false;
            }
            if let Ok(obj) = sdi.get_mut(&rp) {
                obj.visible = false;
            }
        }

        // Hide TV Guide objects.
        oasis_app_tv_guide::TvGuideState::hide_sdi(sdi);
    }
}
