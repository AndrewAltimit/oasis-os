use oasis_skin::legacy_theme as theme;

use super::*;

/// Bars taller than the screen (and a 0x0 dashboard grid) used to
/// overflow `screen_h - bottombar_height` and friends: a debug-build
/// panic, and bars / icons at y ~ 4 billion in release. Every bar,
/// the dashboard, the start menu, the terminal and the assemble
/// transition must lay out such a skin without panicking.
#[test]
fn extreme_skin_geometry_does_not_overflow() {
    use crate::dashboard::{AppEntry, DashboardConfig, DashboardState};
    use crate::skin::SkinFeatures;
    use crate::startmenu::StartMenuState;
    use crate::statusbar::StatusBar;
    use crate::taskbar::Taskbar;
    use crate::transition::{TransitionEffect, TransitionState, apply_assemble};
    use crate::wm::window::{Window, WindowConfig, WindowType, WmTheme};

    let mut at = ActiveTheme::default();
    at.screen_w = 8;
    at.screen_h = 10;
    at.statusbar_height = 200;
    at.tab_row_height = 40;
    at.bottombar_height = 300;
    at.grid_padding_x = 500;
    at.grid_padding_y = 500;
    let features = SkinFeatures {
        grid_cols: 0,
        grid_rows: 0,
        ..SkinFeatures::default()
    };

    assert_eq!(bottom_bar_y(&at), 0);
    let mut sdi = SdiRegistry::new();

    let config = DashboardConfig::from_features(&features, &at);
    assert_eq!((config.grid_w, config.grid_h), (0, 0));
    let apps = vec![
        AppEntry::for_title("Terminal"),
        AppEntry::for_title("Paint"),
    ];
    let mut dash = DashboardState::new(config, apps);
    dash.update_sdi(&mut sdi, &at);

    StatusBar::new().update_sdi(&mut sdi, &at, &features);
    let mut bar = BottomBar::new();
    bar.update_sdi(&mut sdi, &at, &features);
    bar.next_tab();
    bar.update_sdi(&mut sdi, &at, &features);
    let mut dock = features.clone();
    dock.bottombar_style = "media_dock".to_string();
    bar.update_sdi(&mut sdi, &at, &dock);
    assert!(sdi.get("bar_bottom").is_ok_and(|o| o.y == 0));

    let win = Window::new(
        &WindowConfig {
            id: "w".into(),
            title: "Window".into(),
            x: None,
            y: None,
            width: 100,
            height: 80,
            window_type: WindowType::AppWindow,
            always_on_top: false,
            modal: false,
        },
        0,
        0,
        &WmTheme::default(),
    );
    Taskbar::new().update_sdi(&mut sdi, &at, &[win], Some("w"), true);

    let menu = StartMenuState::new_with_theme(StartMenuState::default_items(&at), &at);
    menu.update_sdi(&mut sdi, &at);

    let lines = vec!["line".to_string(); 50];
    crate::terminal_sdi::setup_terminal_objects(&mut sdi, &lines, "/", "ls", 0, &at, true);

    let trans = TransitionState::new(TransitionEffect::Assemble, 10, at.screen_w, at.screen_h);
    apply_assemble(&mut sdi, &at, &trans);
}

#[test]
fn media_tab_cycle() {
    let mut bar = BottomBar::new();
    assert_eq!(bar.active_tab, MediaTab::None);
    bar.next_tab();
    assert_eq!(bar.active_tab, MediaTab::Audio);
    bar.next_tab();
    assert_eq!(bar.active_tab, MediaTab::Video);
    bar.next_tab();
    assert_eq!(bar.active_tab, MediaTab::Image);
    bar.next_tab();
    assert_eq!(bar.active_tab, MediaTab::File);
    bar.next_tab();
    assert_eq!(bar.active_tab, MediaTab::None);
}

#[test]
fn update_sdi_creates_objects() {
    let bar = BottomBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures {
        show_media_tabs: true,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);
    assert!(sdi.contains("bar_bottom"));
    // bar_url is not created when bar_url_text is empty (default).
    assert!(sdi.contains("bar_btab_0"));
    assert!(sdi.contains("bar_btab_1"));
    assert!(sdi.contains("bar_btab_2"));
    assert!(sdi.contains("bar_btab_3"));
    assert!(sdi.contains("bar_bpipe_0"));
    assert!(sdi.contains("bar_bpipe_1"));
    assert!(sdi.contains("bar_bpipe_2"));
}

#[test]
fn page_dots_visibility() {
    let mut bar = BottomBar::new();
    bar.total_pages = 3;
    bar.current_page = 1;
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);

    assert!(sdi.get("bar_page_0").unwrap().visible);
    assert!(sdi.get("bar_page_1").unwrap().visible);
    assert!(sdi.get("bar_page_2").unwrap().visible);
}

#[test]
fn bar_is_overlay() {
    let bar = BottomBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);
    assert!(sdi.get("bar_bottom").unwrap().overlay);
}

#[test]
fn media_tab_labels() {
    assert_eq!(MediaTab::None.label(), "");
    assert_eq!(MediaTab::Audio.label(), "AUDIO");
    assert_eq!(MediaTab::Video.label(), "VIDEO");
    assert_eq!(MediaTab::Image.label(), "IMAGE");
    assert_eq!(MediaTab::File.label(), "FILE");
}

#[test]
fn media_tab_next_from_none() {
    assert_eq!(MediaTab::None.next(), MediaTab::Audio);
}

#[test]
fn media_tab_next_from_file_wraps() {
    assert_eq!(MediaTab::File.next(), MediaTab::None);
}

#[test]
fn bottombar_default_state() {
    let bar = BottomBar::new();
    assert_eq!(bar.active_tab, MediaTab::None);
    assert_eq!(bar.current_page, 0);
    assert_eq!(bar.total_pages, 1);
    assert!(!bar.l_pressed);
    assert!(!bar.r_pressed);
}

#[test]
fn bottombar_default_trait() {
    let bar = BottomBar::default();
    assert_eq!(bar.active_tab, MediaTab::None);
}

#[test]
fn next_tab_cycles_correctly() {
    let mut bar = BottomBar::new();
    bar.next_tab();
    assert_eq!(bar.active_tab, MediaTab::Audio);
    bar.next_tab();
    assert_eq!(bar.active_tab, MediaTab::Video);
    bar.next_tab();
    assert_eq!(bar.active_tab, MediaTab::Image);
    bar.next_tab();
    assert_eq!(bar.active_tab, MediaTab::File);
    bar.next_tab();
    assert_eq!(bar.active_tab, MediaTab::None);
}

#[test]
fn page_dots_hidden_when_disabled() {
    let mut bar = BottomBar::new();
    bar.total_pages = 3;
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();

    // First enable to create objects.
    let mut feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);

    // Now disable and verify they're hidden.
    feat.show_page_dots = false;
    bar.update_sdi(&mut sdi, &at, &feat);

    assert!(!sdi.get("bar_page_0").unwrap().visible);
    assert!(!sdi.get("bar_page_1").unwrap().visible);
    assert!(!sdi.get("bar_page_2").unwrap().visible);
}

#[test]
fn media_tabs_hidden_when_disabled() {
    let bar = BottomBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();

    // First enable to create objects.
    let mut feat = crate::skin::SkinFeatures {
        show_media_tabs: true,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);

    // Now disable and verify they're hidden.
    feat.show_media_tabs = false;
    bar.update_sdi(&mut sdi, &at, &feat);

    assert!(!sdi.get("bar_btab_0").unwrap().visible);
}

#[test]
fn hide_sdi_hides_all_objects() {
    let bar = BottomBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);

    BottomBar::hide_sdi(&mut sdi);

    assert!(!sdi.get("bar_bottom").unwrap().visible);
    // bar_url and bar_usb are not created when URL text is empty.
}

#[test]
fn active_tab_color_differs_from_inactive() {
    let mut bar = BottomBar::new();
    bar.active_tab = MediaTab::Audio;
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures {
        show_media_tabs: true,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);

    let audio_tab = sdi.get("bar_btab_0").unwrap();
    let video_tab = sdi.get("bar_btab_1").unwrap();
    assert_ne!(audio_tab.text_color, video_tab.text_color);
}

#[test]
fn bottom_clock_renders_when_enabled() {
    let mut bar = BottomBar::new();
    bar.update_info(Some(&crate::platform::SystemTime {
        year: 2026,
        month: 5,
        day: 1,
        hour: 14,
        minute: 32,
        second: 0,
    }));
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures {
        clock_in_bottombar: true,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);

    let clock = sdi.get("bar_bottom_clock").unwrap();
    let text = clock.text.as_ref().unwrap();
    assert!(text.contains("14:32"));
    assert!(text.contains("May"));
    assert!(clock.visible);
}

#[test]
fn bottom_clock_hidden_when_opted_out() {
    let bar = BottomBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures {
        clock_in_bottombar: false,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);
    // Object is not created when the feature is opted out.
    assert!(!sdi.contains("bar_bottom_clock"));
}

#[test]
fn url_label_hidden_when_empty() {
    let bar = BottomBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);

    // Default theme has empty bar_url_text, so URL label is not created.
    assert!(!sdi.contains("bar_url"));
}

#[test]
fn url_label_shown_when_set() {
    let bar = BottomBar::new();
    let mut sdi = SdiRegistry::new();
    let mut at = crate::active_theme::ActiveTheme::default();
    at.bar.url_text = "HTTP://EXAMPLE".to_string();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);

    let url = sdi.get("bar_url").unwrap();
    assert_eq!(url.text, Some("HTTP://EXAMPLE".to_string()));
    assert!(url.visible);
}

#[test]
fn usb_hidden_when_url_empty() {
    let bar = BottomBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);

    // USB indicator is hidden when URL text is empty.
    assert!(!sdi.contains("bar_usb"));
}

#[test]
fn page_dot_count_limited_to_max() {
    let mut bar = BottomBar::new();
    bar.total_pages = 20; // More than MAX_PAGE_DOTS.
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);

    // Only MAX_PAGE_DOTS (typically 8) should be visible.
    let max_dots = theme::MAX_PAGE_DOTS;
    assert!(
        sdi.get(&format!("bar_page_{}", max_dots - 1))
            .unwrap()
            .visible
    );
}

fn dock_features() -> SkinFeatures {
    SkinFeatures {
        bottombar_style: "media_dock".to_string(),
        ..Default::default()
    }
}

/// `true` when `name` is absent or hidden.
fn hidden(sdi: &SdiRegistry, name: &str) -> bool {
    sdi.get(name).map_or(true, |o| !o.visible)
}

/// Labels placed after other content used to be positioned only when
/// first created, so a skin swap that changed the URL text (or the bar
/// style) left the USB indicator at its old x.
#[test]
fn usb_label_follows_url_text_across_skin_swaps() {
    let bar = BottomBar::new();
    let mut sdi = SdiRegistry::new();
    let feat = SkinFeatures::default();
    let mut at = crate::active_theme::ActiveTheme::default();
    at.bar.url_text = "A".to_string();
    bar.update_sdi(&mut sdi, &at, &feat);
    let short_x = sdi.get("bar_usb").unwrap().x;

    at.bar.url_text = "HTTP://A-MUCH-LONGER-URL".to_string();
    bar.update_sdi(&mut sdi, &at, &feat);
    let long_x = sdi.get("bar_usb").unwrap().x;
    assert!(
        long_x > short_x,
        "USB x {long_x} should move past {short_x}"
    );
    let url = sdi.get("bar_url").unwrap();
    assert!(long_x > url.x + text_px(&at.bar.url_text, at.font_small));
}

/// The right-aligned media tab strip must track the bottom clock's width
/// (the date arrives after the first frame).
#[test]
fn media_tabs_follow_bottom_clock_width() {
    let mut bar = BottomBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = SkinFeatures {
        show_media_tabs: true,
        clock_in_bottombar: true,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);
    let before = sdi.get("bar_btab_0").unwrap().x;
    bar.update_info(Some(&crate::platform::SystemTime {
        year: 2026,
        month: 10,
        day: 30,
        hour: 12,
        minute: 34,
        second: 0,
    }));
    bar.update_sdi(&mut sdi, &at, &feat);
    let after = sdi.get("bar_btab_0").unwrap().x;
    assert!(after < before, "tab x {after} should move left of {before}");
}

#[test]
fn media_dock_draws_transport_and_tracks() {
    let bar = BottomBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    bar.update_sdi(&mut sdi, &at, &dock_features());
    for name in ["bar_dock_prev", "bar_dock_play", "bar_dock_next", "bar_usb"] {
        assert!(sdi.get(name).unwrap().visible, "{name} visible");
    }
    for name in ["bar_dock_progress_fill", "bar_dock_vol_fill"] {
        let fill = sdi.get(name).unwrap();
        assert!(fill.visible && fill.z == 902, "{name} above its track");
    }
    // Forced media tabs without the classic bezel; no page dots.
    assert!(sdi.get("bar_btab_0").unwrap().visible);
    assert!(hidden(&sdi, "bar_tab_bezel"));
    assert!(hidden(&sdi, "bar_page_0"));
}

#[test]
fn switching_dock_to_classic_hides_dock_objects() {
    let bar = BottomBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    bar.update_sdi(&mut sdi, &at, &dock_features());
    assert!(sdi.get("bar_usb").unwrap().visible);
    bar.update_sdi(&mut sdi, &at, &SkinFeatures::default());
    for name in [
        "bar_dock_prev",
        "bar_dock_play_g_0",
        "bar_dock_vol_track",
        "bar_btab_0",
        // No URL text: the classic footer has no USB indicator.
        "bar_usb",
    ] {
        assert!(hidden(&sdi, name), "{name} hidden");
    }
}

/// Glyph scanlines past the active row count are hidden when a shorter
/// bar shrinks the transport glyphs.
#[test]
fn dock_glyph_rows_shrink_with_the_bar() {
    let bar = BottomBar::new();
    let mut sdi = SdiRegistry::new();
    let mut at = crate::active_theme::ActiveTheme::default();
    let visible_rows = |sdi: &SdiRegistry| {
        (0..9)
            .filter(|r| !hidden(sdi, &format!("bar_dock_play_g_{r}")))
            .count()
    };
    at.bottombar_height = 60;
    bar.update_sdi(&mut sdi, &at, &dock_features());
    let tall_rows = visible_rows(&sdi);
    at.bottombar_height = 24;
    bar.update_sdi(&mut sdi, &at, &dock_features());
    let short_rows = visible_rows(&sdi);
    assert!(short_rows < tall_rows, "{short_rows} rows vs {tall_rows}");
}
