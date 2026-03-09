//! Dashboard initialization and SDI rendering orchestration.
//!
//! Wraps `oasis_core::dashboard::DashboardState` with PSP-specific setup:
//! converting the local `APPS` list to `CoreAppEntry`, creating the
//! `DashboardConfig` from skin features, and managing the SDI update cycle
//! (show/hide, animation ticks, vector icon overlays).

use oasis_backend_psp::{PspBackend, SdiRegistry};

use oasis_core::active_theme::ActiveTheme;
use oasis_core::dashboard::{AppEntry as CoreAppEntry, DashboardConfig, DashboardState};
use oasis_core::skin::SkinFeatures;

use crate::types::APPS;

// ---------------------------------------------------------------------------
// Dashboard factory
// ---------------------------------------------------------------------------

/// Create a `DashboardState` populated with the PSP app list.
pub(crate) fn create_dashboard(
    skin_features: &SkinFeatures,
    active_theme: &ActiveTheme,
) -> DashboardState {
    let core_apps: Vec<CoreAppEntry> = APPS
        .iter()
        .map(|a| CoreAppEntry {
            title: a.title.to_string(),
            path: format!("/apps/{}", a.id),
            icon_png: Vec::new(),
            color: a.color,
        })
        .collect();
    let dash_config = DashboardConfig::from_features(skin_features, active_theme);
    DashboardState::new(dash_config, core_apps)
}

// ---------------------------------------------------------------------------
// SDI update helpers
// ---------------------------------------------------------------------------

/// Show dashboard icons in the SDI registry (tick animation + update).
pub(crate) fn show_dashboard_sdi(
    dashboard: &mut DashboardState,
    sdi: &mut SdiRegistry,
    active_theme: &ActiveTheme,
) {
    dashboard.tick_animation();
    dashboard.update_sdi(sdi, active_theme);
}

/// Hide dashboard icons in the SDI registry.
pub(crate) fn hide_dashboard_sdi(dashboard: &mut DashboardState, sdi: &mut SdiRegistry) {
    dashboard.hide_sdi(sdi);
}

/// Render vector icon overlays on top of the SDI-drawn dashboard.
///
/// Only called when `active_theme.icon.style == "vector"` and icons are visible.
pub(crate) fn render_vector_overlays(
    backend: &mut PspBackend,
    dashboard: &mut DashboardState,
    active_theme: &ActiveTheme,
    viz_frame: u32,
) {
    let _ = oasis_core::vector_overlay::render_vector_background(backend, active_theme, viz_frame);
    let _ = dashboard.render_vector_icons(backend, active_theme, viz_frame);
}
