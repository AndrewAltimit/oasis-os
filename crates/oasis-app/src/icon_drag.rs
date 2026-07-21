//! Desktop-icon drag & drop and position persistence.
//!
//! In free icon layout (`features.icon_layout = "free"`), a pointer press
//! on an icon arms a potential drag. The drag activates once the pointer
//! moves ≥ [`DRAG_THRESHOLD_PX`] or the button is held ≥
//! [`DRAG_HOLD_FRAMES`]; until then a release is treated as a click
//! (select / launch). Dropped positions are committed through
//! [`DashboardState::set_icon_position`] (snap + clamp) and persisted per
//! skin in the settings store under `icon_positions.<skin>.<app path>`.
//!
//! [`DashboardState::set_icon_position`]:
//!     oasis_core::dashboard::DashboardState::set_icon_position

use oasis_core::dashboard::DashboardState;
use oasis_core::settings::SettingsStore;
use oasis_core::vfs::MemoryVfs;

use crate::app_state::AppState;

/// Pointer movement (pixels) that promotes a press into a drag.
pub const DRAG_THRESHOLD_PX: i32 = 4;
/// Hold duration (frames, ~60fps) that promotes a press into a drag.
pub const DRAG_HOLD_FRAMES: u64 = 15;

/// An armed or active icon drag.
#[derive(Debug, Clone, Copy)]
pub struct IconDrag {
    /// Page index of the pressed icon.
    pub index: usize,
    /// Pointer offset from the icon's cell origin at press time.
    grab_dx: i32,
    grab_dy: i32,
    /// Press position (for the movement threshold).
    start_x: i32,
    start_y: i32,
    /// Frame counter at press time (for the hold threshold).
    press_frame: u64,
    /// Whether the icon was already selected before this press
    /// (two-click launch when `launch_on_single_click = false`).
    was_selected: bool,
    /// True once a threshold was crossed and the icon follows the pointer.
    pub moved: bool,
}

/// What a pointer release resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseAction {
    /// Nothing further (drop committed, or click only selected).
    None,
    /// Launch the app at this page index.
    Launch(usize),
}

/// Arm a drag for the icon at page index `index`, selecting it.
pub fn begin(state: &mut AppState, index: usize, x: i32, y: i32) {
    let dash = &mut state.ui.dashboard;
    let was_selected = dash.selected == index;
    dash.selected = index;
    let (ox, oy) = match dash.icon_rect(index) {
        Some((ox, oy, _, _)) => (ox, oy),
        None => return,
    };
    state.icon_drag = Some(IconDrag {
        index,
        grab_dx: x - ox,
        grab_dy: y - oy,
        start_x: x,
        start_y: y,
        press_frame: state.frame_counter,
        was_selected,
        moved: false,
    });
}

/// Whether an icon drag is armed or active.
pub fn active(state: &AppState) -> bool {
    state.icon_drag.is_some()
}

/// Track pointer movement: promote an armed drag past its threshold and
/// move the icon live (unsnapped — snapping happens on drop).
pub fn on_move(state: &mut AppState, x: i32, y: i32) {
    let Some(mut drag) = state.icon_drag else {
        return;
    };
    if !drag.moved {
        let dist = (x - drag.start_x).abs().max((y - drag.start_y).abs());
        let held = state.frame_counter.saturating_sub(drag.press_frame);
        if dist < DRAG_THRESHOLD_PX && held < DRAG_HOLD_FRAMES {
            return;
        }
        drag.moved = true;
        state.ui.dashboard.drag_index = Some(drag.index);
    }
    let dash = &mut state.ui.dashboard;
    if let Some(app) = dash.current_page_apps().get(drag.index) {
        let path = app.path.clone();
        let pos = dash.clamp_free_position(x - drag.grab_dx, y - drag.grab_dy);
        dash.positions.insert(path, pos);
    }
    state.icon_drag = Some(drag);
}

/// Resolve a pointer release: commit the drop (and persist it), or report
/// the click so the caller can launch.
pub fn on_release(state: &mut AppState, vfs: &mut MemoryVfs, x: i32, y: i32) -> ReleaseAction {
    let Some(drag) = state.icon_drag.take() else {
        return ReleaseAction::None;
    };
    state.ui.dashboard.drag_index = None;

    if drag.moved {
        if let Some((px, py)) =
            state
                .ui
                .dashboard
                .set_icon_position(drag.index, x - drag.grab_dx, y - drag.grab_dy)
        {
            let dash = &state.ui.dashboard;
            if let Some(app) = dash.current_page_apps().get(drag.index) {
                let key = position_key(&state.skin.manifest.name, &app.path);
                state.settings.set_string(key, format!("{px},{py}"));
                state.settings.save(vfs);
            }
        }
        return ReleaseAction::None;
    }

    // Plain click: launch on the first click by default, or on the second
    // click of an already-selected icon when the skin opts out.
    if state.skin.features.launch_on_single_click || drag.was_selected {
        ReleaseAction::Launch(drag.index)
    } else {
        ReleaseAction::None
    }
}

/// Settings key for one icon's stored position.
fn position_key(skin: &str, app_path: &str) -> String {
    format!("icon_positions.{skin}.{app_path}")
}

/// Load persisted icon positions for `skin` into the dashboard.
///
/// Invalid values are skipped; off-screen positions are clamped by the
/// layout pass, and apps without an entry auto-flow.
pub fn load_icon_positions(settings: &SettingsStore, skin: &str, dash: &mut DashboardState) {
    let prefix = format!("icon_positions.{skin}.");
    let keys: Vec<String> = settings
        .keys()
        .filter(|k| k.starts_with(prefix.as_str()))
        .map(String::from)
        .collect();
    for key in keys {
        let path = key[prefix.len()..].to_string();
        if path.is_empty() {
            continue;
        }
        if let Some(val) = settings.get_string(&key)
            && let Some((xs, ys)) = val.split_once(',')
            && let (Ok(x), Ok(y)) = (xs.trim().parse::<i32>(), ys.trim().parse::<i32>())
        {
            dash.positions.insert(path, (x, y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_core::active_theme::ActiveTheme;
    use oasis_core::dashboard::{AppEntry, DashboardConfig};
    use oasis_core::skin::SkinFeatures;

    fn free_dashboard(n: usize) -> DashboardState {
        let mut features = SkinFeatures::default();
        features.icon_layout = "free".to_string();
        let at = ActiveTheme::default();
        let config = DashboardConfig::from_features(&features, &at);
        let apps = (0..n)
            .map(|i| AppEntry {
                title: format!("App {i}"),
                path: format!("/apps/app{i}"),
                icon_png: Vec::new(),
                color: oasis_core::backend::Color::rgb(100, 100, 100),
            })
            .collect();
        DashboardState::new(config, apps)
    }

    #[test]
    fn load_positions_parses_and_filters_by_skin() {
        let mut settings = SettingsStore::new();
        settings.set_string("icon_positions.mytheme./apps/app0", "96, 128");
        settings.set_string("icon_positions.other./apps/app0", "10,10");
        settings.set_string("icon_positions.mytheme./apps/app1", "junk");
        let mut dash = free_dashboard(2);
        load_icon_positions(&settings, "mytheme", &mut dash);
        assert_eq!(dash.positions.get("/apps/app0"), Some(&(96, 128)));
        assert!(!dash.positions.contains_key("/apps/app1"));
        assert_eq!(dash.positions.len(), 1);
    }

    #[test]
    fn position_key_roundtrips_through_settings() {
        let mut vfs = MemoryVfs::new();
        let mut settings = SettingsStore::new();
        let key = position_key("mytheme", "/apps/browser");
        settings.set_string(key, "16,48");
        settings.save(&mut vfs);

        let mut reloaded = SettingsStore::new();
        reloaded.load(&vfs);
        let mut dash = free_dashboard(0);
        dash.apps.push(AppEntry {
            title: "Browser".to_string(),
            path: "/apps/browser".to_string(),
            icon_png: Vec::new(),
            color: oasis_core::backend::Color::rgb(1, 2, 3),
        });
        load_icon_positions(&reloaded, "mytheme", &mut dash);
        assert_eq!(dash.positions.get("/apps/browser"), Some(&(16, 48)));
    }
}
