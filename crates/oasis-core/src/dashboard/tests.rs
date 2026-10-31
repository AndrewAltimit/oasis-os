use super::*;
use crate::backend::Color;
use crate::input::Button;
use crate::sdi::SdiRegistry;
use crate::ui::flex::GridLayout;

pub(crate) fn test_config() -> DashboardConfig {
    DashboardConfig {
        grid_cols: 2,
        grid_rows: 2,
        icons_per_page: 4,
        grid_x: 16,
        grid_y: 48,
        cell_w: 110,
        cell_h: 95,
        cursor_pad: 4,
        grid_layout: GridLayout::new(2),
        grid_w: 220,
        grid_h: 190,
        page_slide_duration: 12,
        press_flash_duration: 6,
        free_layout: false,
        free_cell_w: 80,
        free_cell_h: 80,
        free_cols: None,
        left_align_icons: false,
        snap_to_grid: true,
        page_style: "slide".to_string(),
    }
}

pub(crate) fn test_apps(n: usize) -> Vec<AppEntry> {
    (0..n)
        .map(|i| AppEntry {
            title: format!("App {i}"),
            path: format!("/apps/app{i}"),
            icon_png: Vec::new(),
            color: Color::rgb(100, 100, 100),
        })
        .collect()
}

#[test]
fn idle_dashboard_frames_leave_scene_clean() {
    // The Dashboard-mode per-frame SDI pass: hide the terminal pool,
    // then sync the dashboard. Once the first frame has built the
    // scene, an idle frame must not report a change (idle-frame
    // elision depends on it).
    let at = ActiveTheme::default();
    let mut dash = DashboardState::new(test_config(), test_apps(6));
    let mut sdi = SdiRegistry::new();
    let lines: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
    crate::terminal_sdi::setup_terminal_objects(&mut sdi, &lines, "/", "", 0, &at, true);
    let frame = |dash: &mut DashboardState, sdi: &mut SdiRegistry| {
        crate::terminal_sdi::set_terminal_visible(sdi, false);
        dash.tick_animation();
        dash.update_sdi(sdi, &at);
    };
    frame(&mut dash, &mut sdi);
    assert!(sdi.take_scene_dirty(), "first frame builds the scene");
    frame(&mut dash, &mut sdi);
    assert!(!sdi.take_scene_dirty(), "idle frame 2 must be clean");
    frame(&mut dash, &mut sdi);
    assert!(!sdi.is_scene_dirty(), "idle frame 3 must be clean");
    // A real change still registers.
    dash.next_page();
    frame(&mut dash, &mut sdi);
    assert!(sdi.is_scene_dirty(), "page change must redraw");
}

#[test]
fn page_count_single() {
    let dash = DashboardState::new(test_config(), test_apps(3));
    assert_eq!(dash.page_count(), 1);
}

#[test]
fn page_count_multiple() {
    let dash = DashboardState::new(test_config(), test_apps(6));
    assert_eq!(dash.page_count(), 2);
}

#[test]
fn page_count_exact() {
    let dash = DashboardState::new(test_config(), test_apps(4));
    assert_eq!(dash.page_count(), 1);
}

#[test]
fn page_count_empty() {
    let dash = DashboardState::new(test_config(), vec![]);
    assert_eq!(dash.page_count(), 1);
}

#[test]
fn navigate_right_wraps() {
    let mut dash = DashboardState::new(test_config(), test_apps(3));
    dash.handle_input(&Button::Right);
    assert_eq!(dash.selected, 1);
    dash.handle_input(&Button::Right);
    assert_eq!(dash.selected, 2);
    dash.handle_input(&Button::Right);
    assert_eq!(dash.selected, 0); // Wraps.
}

#[test]
fn navigate_left_wraps() {
    let mut dash = DashboardState::new(test_config(), test_apps(3));
    dash.handle_input(&Button::Left);
    assert_eq!(dash.selected, 2); // Wraps to last.
}

#[test]
fn navigate_down() {
    let mut dash = DashboardState::new(test_config(), test_apps(4));
    dash.handle_input(&Button::Down);
    assert_eq!(dash.selected, 2); // Moved down one row (2 cols).
}

#[test]
fn navigate_up() {
    let mut dash = DashboardState::new(test_config(), test_apps(4));
    dash.selected = 3;
    dash.handle_input(&Button::Up);
    assert_eq!(dash.selected, 1);
}

#[test]
fn zero_frame_page_slide_is_complete_not_nan() {
    let anim = PageSlideAnim {
        frame: 0,
        duration: 0,
        direction: -1,
    };
    assert_eq!(anim.eased_progress(), 1.0);
    let half = PageSlideAnim {
        frame: 5,
        duration: 10,
        direction: 1,
    };
    assert!((half.eased_progress() - 0.5).abs() < 1e-6);
}

#[test]
fn page_style_slide_animates_fade_does_not() {
    let mut dash = DashboardState::new(test_config(), test_apps(6));
    dash.next_page();
    assert!(dash.page_anim.is_some(), "slide style animates");

    let mut cfg = test_config();
    cfg.page_style = "fade".to_string();
    let mut dash = DashboardState::new(cfg, test_apps(6));
    dash.next_page();
    assert!(dash.page_anim.is_none(), "fade style suppresses the slide");
    assert_eq!(dash.page, 1);
    dash.prev_page();
    assert!(dash.page_anim.is_none());
    assert_eq!(dash.page, 0);
}

#[test]
fn next_page_wraps() {
    let mut dash = DashboardState::new(test_config(), test_apps(6));
    assert_eq!(dash.page, 0);
    dash.next_page();
    assert_eq!(dash.page, 1);
    dash.next_page();
    assert_eq!(dash.page, 0); // Wraps (2 pages).
}

#[test]
fn prev_page_wraps() {
    let mut dash = DashboardState::new(test_config(), test_apps(6));
    dash.prev_page();
    assert_eq!(dash.page, 1); // Wraps to last.
}

#[test]
fn selected_app() {
    let dash = DashboardState::new(test_config(), test_apps(3));
    let app = dash.selected_app().unwrap();
    assert_eq!(app.title, "App 0");
}

#[test]
fn update_sdi_creates_objects() {
    let mut dash = DashboardState::new(test_config(), test_apps(3));
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    dash.update_sdi(&mut sdi, &at);
    assert!(sdi.contains("icon_0"));
    assert!(sdi.contains("icon_1"));
    assert!(sdi.contains("icon_2"));
    assert!(sdi.contains("icon_label_0"));
    assert!(sdi.contains("icon_label_1"));
    assert!(sdi.contains("cursor_highlight"));
}

#[test]
fn grid_layout_shows_selection_highlight() {
    let mut dash = DashboardState::new(test_config(), test_apps(3));
    let mut sdi = SdiRegistry::new();
    let mut at = crate::active_theme::ActiveTheme::default();
    dash.selected = 2;
    dash.update_sdi(&mut sdi, &at);
    let cursor = sdi.get("cursor_highlight").unwrap();
    assert!(
        cursor.visible,
        "d-pad selection must be visible in grid mode"
    );
    // The highlight surrounds the selected icon's cell, not icon 0's.
    let (cx, cy, cw, ch) = dash.icon_rect(2).unwrap();
    assert!(cursor.x >= cx && cursor.x < cx + cw as i32);
    assert!(cursor.y >= cy && cursor.y < cy + ch as i32);
    // It sits below the icon bodies.
    assert!(cursor.z < sdi.get("icon_2").unwrap().z);
    // `cursor_style = "none"` still disables it.
    at.icon.cursor_style = "none".to_string();
    dash.update_sdi(&mut sdi, &at);
    assert!(!sdi.get("cursor_highlight").unwrap().visible);
}

#[test]
fn set_hover_tracks_pointer_target() {
    let mut dash = DashboardState::new(test_config(), test_apps(3));
    assert!(dash.set_hover(Some(1)));
    assert_eq!(dash.hover_index, Some(1));
    assert!(!dash.set_hover(Some(1)), "same target is not a change");
    // Slots without an app on this page never hover.
    assert!(dash.set_hover(Some(3)));
    assert_eq!(dash.hover_index, None);
    dash.set_hover(Some(0));
    dash.next_page();
    assert_eq!(dash.hover_index, None, "page change drops hover");
}

#[test]
fn hover_shows_faint_highlight_and_lifts_icon() {
    let mut dash = DashboardState::new(test_config(), test_apps(3));
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    dash.tick_animation();
    dash.update_sdi(&mut sdi, &at);
    let rest_y = sdi.get("icon_1").unwrap().y;
    let rest_label_y = sdi.get("icon_label_1").unwrap().y;
    assert!(!sdi.get("hover_highlight").unwrap().visible);

    dash.set_hover(Some(1));
    assert!(dash.has_active_animation(&at), "lift must request frames");
    let mut prev = rest_y;
    for _ in 0..10 {
        dash.tick_animation();
        dash.update_sdi(&mut sdi, &at);
        let y = sdi.get("icon_1").unwrap().y;
        assert!(y <= prev, "lift eases monotonically");
        prev = y;
    }
    let lifted = rest_y - sdi.get("icon_1").unwrap().y;
    assert!((2..=3).contains(&lifted), "lift {lifted}px");
    assert_eq!(sdi.get("icon_label_1").unwrap().y, rest_label_y);
    assert!(!dash.has_active_animation(&at), "settled lift is idle");
    let hover = sdi.get("hover_highlight").unwrap();
    assert!(hover.visible);
    let sel = sdi.get("cursor_highlight").unwrap();
    let hover_a = hover.stroke_color.unwrap().a;
    assert!(hover_a < sel.stroke_color.unwrap().a, "hover is fainter");

    // Pointer leaves: highlight hides and the icon settles back.
    dash.set_hover(None);
    for _ in 0..10 {
        dash.tick_animation();
    }
    dash.update_sdi(&mut sdi, &at);
    assert_eq!(sdi.get("icon_1").unwrap().y, rest_y);
    assert!(!sdi.get("hover_highlight").unwrap().visible);
}

#[test]
fn hover_does_not_lift_under_reduced_motion() {
    let mut dash = DashboardState::new(test_config(), test_apps(3));
    let mut sdi = SdiRegistry::new();
    let mut at = crate::active_theme::ActiveTheme::default();
    at.ui_theme.reduced_motion = true;
    dash.update_sdi(&mut sdi, &at);
    let rest_y = sdi.get("icon_1").unwrap().y;
    dash.set_hover(Some(1));
    assert!(!dash.has_active_animation(&at));
    for _ in 0..10 {
        dash.tick_animation();
        dash.update_sdi(&mut sdi, &at);
        assert_eq!(sdi.get("icon_1").unwrap().y, rest_y);
    }
    // The static hover highlight still shows.
    assert!(sdi.get("hover_highlight").unwrap().visible);
}

#[test]
fn free_layout_shows_selection_highlight() {
    let mut config = test_config();
    config.free_layout = true;
    let mut dash = DashboardState::new(config, test_apps(3));
    dash.selected = 1;
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    dash.update_sdi(&mut sdi, &at);
    let cursor = sdi.get("cursor_highlight").unwrap();
    assert!(cursor.visible);
    // Default cursor_style is "stroke": outline only, transparent fill.
    assert_eq!(cursor.stroke_width, Some(at.icon.cursor_stroke_width));
    // Hidden again while a drag is in flight.
    dash.drag_index = Some(1);
    dash.update_sdi(&mut sdi, &at);
    assert!(!sdi.get("cursor_highlight").unwrap().visible);
}

#[test]
fn selected_clamps_on_page_switch() {
    let mut dash = DashboardState::new(test_config(), test_apps(5));
    // 5 apps, 4 per page: page 0 has 4, page 1 has 1.
    dash.selected = 3; // Last on page 0.
    dash.next_page();
    // Page 1 has only 1 app, so selected should clamp to 0.
    assert_eq!(dash.selected, 0);
}

// -----------------------------------------------------------------------
// PSP 4x3 grid navigation tests
//
// The PSP backend uses a 4-column, 3-row grid (12 icons per page).
// These tests exercise navigation patterns specific to that layout.
// -----------------------------------------------------------------------

fn psp_config() -> DashboardConfig {
    DashboardConfig {
        grid_cols: 4,
        grid_rows: 3,
        icons_per_page: 12,
        grid_x: 8,
        grid_y: 20,
        cell_w: 116,
        cell_h: 72,
        cursor_pad: 2,
        grid_layout: GridLayout::new(4),
        grid_w: 464,
        grid_h: 218,
        page_slide_duration: 12,
        press_flash_duration: 6,
        free_layout: false,
        free_cell_w: 68,
        free_cell_h: 68,
        free_cols: None,
        left_align_icons: false,
        snap_to_grid: true,
        page_style: "slide".to_string(),
    }
}

#[test]
fn psp_grid_navigate_right_wraps_at_row_end() {
    // PSP has 11 apps (one short of filling the 4x3 grid).
    let mut dash = DashboardState::new(psp_config(), test_apps(11));
    // Start at 0, go right to the end of all apps.
    for _ in 0..10 {
        dash.handle_input(&Button::Right);
    }
    assert_eq!(dash.selected, 10);
    // One more right wraps to 0.
    dash.handle_input(&Button::Right);
    assert_eq!(dash.selected, 0);
}

#[test]
fn psp_grid_navigate_down_across_rows() {
    let mut dash = DashboardState::new(psp_config(), test_apps(12));
    // Start at position 0 (row 0, col 0).
    assert_eq!(dash.selected, 0);
    // Down moves to row 1, col 0 = position 4.
    dash.handle_input(&Button::Down);
    assert_eq!(dash.selected, 4);
    // Down again to row 2, col 0 = position 8.
    dash.handle_input(&Button::Down);
    assert_eq!(dash.selected, 8);
    // Down at bottom row: stays put.
    dash.handle_input(&Button::Down);
    assert_eq!(dash.selected, 8);
}

#[test]
fn psp_grid_navigate_up_at_top_stays() {
    let mut dash = DashboardState::new(psp_config(), test_apps(12));
    dash.selected = 2; // Row 0, col 2.
    dash.handle_input(&Button::Up);
    assert_eq!(dash.selected, 2); // Can't go up from row 0.
}

#[test]
fn psp_grid_navigate_left_wraps_to_last() {
    let mut dash = DashboardState::new(psp_config(), test_apps(11));
    assert_eq!(dash.selected, 0);
    dash.handle_input(&Button::Left);
    assert_eq!(dash.selected, 10); // Wraps to last app.
}

#[test]
fn psp_grid_full_page_count() {
    // 11 apps, 12 per page => 1 page.
    let dash = DashboardState::new(psp_config(), test_apps(11));
    assert_eq!(dash.page_count(), 1);
    // 13 apps, 12 per page => 2 pages.
    let dash = DashboardState::new(psp_config(), test_apps(13));
    assert_eq!(dash.page_count(), 2);
    // 24 apps, 12 per page => 2 pages (exact fill).
    let dash = DashboardState::new(psp_config(), test_apps(24));
    assert_eq!(dash.page_count(), 2);
}

#[test]
fn psp_grid_page_switch_clamps_selected() {
    let mut dash = DashboardState::new(psp_config(), test_apps(14));
    // Page 0 has 12 apps, page 1 has 2.
    dash.selected = 11; // Last on page 0.
    dash.next_page();
    // Page 1 has only 2 apps; selected should clamp to 0 or 1.
    assert!(dash.selected < 2);
}

#[test]
fn psp_grid_down_blocked_by_incomplete_row() {
    // 9 apps in a 4-col grid: rows are [0..3], [4..7], [8].
    let mut dash = DashboardState::new(psp_config(), test_apps(9));
    dash.selected = 5; // Row 1, col 1.
    // Down to row 2: only position 8 exists, and 5+4=9 >= 9 apps, stays.
    dash.handle_input(&Button::Down);
    assert_eq!(dash.selected, 5); // Can't go down -- row 2 col 1 doesn't exist.
}

/// PSP uses compact grid: 4 cols, 3 rows, 12 icons per page.
/// `DashboardConfig::from_features` with PSP-specific ActiveTheme
/// overrides should produce the correct cell geometry.
#[test]
fn psp_dashboard_config_from_features() {
    let mut psp_at = crate::active_theme::ActiveTheme::from_base_colors(
        Color::rgb(0x1A, 0x1A, 0x2D),
        Color::rgb(0x32, 0x64, 0xC8),
        Color::rgb(0x50, 0x50, 0x50),
        Color::WHITE,
        Color::rgb(0x80, 0x80, 0x80),
        Color::rgb(0x28, 0x3C, 0x5A),
        Color::rgb(0x00, 0xFF, 0x00),
        Color::rgb(0xCC, 0xCC, 0xCC),
        Color::rgb(0xFF, 0x44, 0x44),
    )
    .with_screen_size(480, 272);

    // Simulate PSP overrides from skins.rs::apply_psp_overrides.
    psp_at.statusbar_height = 18;
    psp_at.tab_row_height = 0;
    psp_at.bottombar_height = 32;
    psp_at.icon_width = 34;
    psp_at.icon_height = 34;
    psp_at.grid_padding_x = 8;
    psp_at.grid_padding_y = 2;
    psp_at.cursor_pad = 2;

    let features = crate::skin::SkinFeatures {
        grid_cols: 4,
        grid_rows: 3,
        icons_per_page: 12,
        ..Default::default()
    };

    let config = DashboardConfig::from_features(&features, &psp_at);
    assert_eq!(config.grid_cols, 4);
    assert_eq!(config.grid_rows, 3);
    assert_eq!(config.icons_per_page, 12);
    // Content area: 272 - 18 (statusbar) - 0 (tab) - 32 (bottom) = 222
    // Grid height: 222 - 2*2 = 218, cell height: 218 / 3 = 72
    assert_eq!(config.cell_h, 72);
    // Grid width: 480 - 2*8 = 464, cell width: 464 / 4 = 116
    assert_eq!(config.cell_w, 116);
    assert_eq!(config.cursor_pad, 2);
}

/// Screen x the (single) vector icon was translated to when drawn.
fn vector_icon_x(dash: &DashboardState, at: &ActiveTheme) -> i32 {
    let mut backend = oasis_test_backend::RecordingBackend::new(480, 272);
    dash.render_vector_icons(&mut backend, at, 0).unwrap();
    backend
        .commands()
        .iter()
        .find_map(|c| match c {
            oasis_test_backend::DrawCommand::PushTranslate { dx, .. } => Some(*dx),
            _ => None,
        })
        .expect("vector icon drawn at a translated origin")
}

/// The vector-icon pass used to centre icons in their cell even in the
/// `"column"` layout, while the SDI labels and selection highlight hug
/// the left edge -- the icon floated away from its own label/cursor.
#[test]
fn vector_icons_follow_column_left_alignment() {
    let mut at = ActiveTheme::default();
    at.icon.style = "vector".to_string();
    let config = DashboardConfig {
        free_layout: true,
        free_cols: Some(1),
        free_cell_w: 120,
        ..test_config()
    };
    let centred = DashboardState::new(config.clone(), test_apps(1));
    let left = DashboardState::new(
        DashboardConfig {
            left_align_icons: true,
            ..config
        },
        test_apps(1),
    );
    let (ix, _) = left.icon_origin(0, (16, 48), &at);
    assert_eq!(ix, 16 + COLUMN_LEFT_PAD);
    assert_eq!(vector_icon_x(&left, &at), ix);
    let centred_x = 16 + (120 - at.icon_width as i32) / 2;
    assert_eq!(vector_icon_x(&centred, &at), centred_x);
}
