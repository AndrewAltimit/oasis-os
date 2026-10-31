use super::*;

fn test_config() -> WindowConfig {
    WindowConfig {
        id: "test_win".to_string(),
        title: "Test Window".to_string(),
        x: None,
        y: None,
        width: 200,
        height: 150,
        window_type: WindowType::AppWindow,
        always_on_top: false,
        modal: false,
    }
}

#[test]
fn window_new_computes_outer_dimensions() {
    let theme = WmTheme::default();
    let config = test_config();
    let win = Window::new(&config, 10, 20, &theme);
    // outer_w = content_w + 2*border = 200 + 2 = 202
    assert_eq!(win.outer_w, 200 + theme.border_width * 2);
    // outer_h = content_h + titlebar + 2*border = 150 + 24 + 2 = 176
    assert_eq!(
        win.outer_h,
        150 + theme.titlebar_height + theme.border_width * 2
    );
}

#[test]
fn fullscreen_has_no_border_or_titlebar() {
    let theme = WmTheme::default();
    let config = WindowConfig {
        id: "fs".to_string(),
        title: "Full".to_string(),
        x: None,
        y: None,
        width: 480,
        height: 272,
        window_type: WindowType::Fullscreen,
        always_on_top: false,
        modal: false,
    };
    let win = Window::new(&config, 0, 0, &theme);
    assert_eq!(win.outer_w, 480);
    assert_eq!(win.outer_h, 272);
    assert!(win.titlebar_rect(&theme).is_none());
}

#[test]
fn content_rect_app_window() {
    let theme = WmTheme::default();
    let config = test_config();
    let win = Window::new(&config, 10, 20, &theme);
    let (cx, cy, cw, ch) = win.content_rect(&theme);
    assert_eq!(cx, 10 + theme.border_width as i32);
    assert_eq!(
        cy,
        20 + theme.titlebar_height as i32 + theme.border_width as i32
    );
    assert_eq!(cw, 200);
    assert_eq!(ch, 150);
}

#[test]
fn titlebar_rect_inside_border() {
    let theme = WmTheme::default();
    let config = test_config();
    let win = Window::new(&config, 10, 20, &theme);
    let (tx, ty, tw, th) = win.titlebar_rect(&theme).unwrap();
    assert_eq!(tx, 10 + theme.border_width as i32);
    assert_eq!(ty, 20 + theme.border_width as i32);
    assert_eq!(tw, 200); // outer_w - 2*border
    assert_eq!(th, theme.titlebar_height);
}

#[test]
fn close_button_top_right() {
    let theme = WmTheme::default();
    let config = test_config();
    let win = Window::new(&config, 0, 0, &theme);
    let (bx, _by, bw, _bh) = win.close_btn_rect(&theme).unwrap();
    let (tx, _ty, tw, _th) = win.titlebar_rect(&theme).unwrap();
    // Close button is near the right edge of the titlebar.
    assert!(bx + bw as i32 <= tx + tw as i32);
    assert!(bx > tx + tw as i32 / 2); // Right half.
}

#[test]
fn dialog_has_no_minimize_maximize() {
    let theme = WmTheme::default();
    let config = WindowConfig {
        id: "dlg".to_string(),
        title: "Dialog".to_string(),
        x: None,
        y: None,
        width: 300,
        height: 100,
        window_type: WindowType::Dialog,
        always_on_top: false,
        modal: false,
    };
    let win = Window::new(&config, 0, 0, &theme);
    assert!(win.has_close_button());
    assert!(!win.has_minimize_button());
    assert!(!win.has_maximize_button());
    assert!(!win.is_resizable());
}

#[test]
fn floating_widget_draggable_no_resize() {
    let theme = WmTheme::default();
    let config = WindowConfig {
        id: "widget".to_string(),
        title: "Clock".to_string(),
        x: None,
        y: None,
        width: 80,
        height: 40,
        window_type: WindowType::FloatingWidget,
        always_on_top: false,
        modal: false,
    };
    let win = Window::new(&config, 0, 0, &theme);
    assert!(win.is_draggable());
    assert!(!win.is_resizable());
    assert!(win.has_close_button());
    assert!(!win.has_minimize_button());
}

#[test]
fn sdi_suffixes_by_type() {
    let theme = WmTheme::default();
    let config = test_config();
    let win = Window::new(&config, 0, 0, &theme);
    let suffixes = win.sdi_suffixes();
    assert!(suffixes.contains(&"frame"));
    assert!(suffixes.contains(&"titlebar"));
    assert!(suffixes.contains(&"btn_close"));
    assert!(suffixes.contains(&"btn_minimize"));
    assert!(suffixes.contains(&"btn_maximize"));
    assert!(suffixes.contains(&"content"));
}

#[test]
fn sdi_name_formatting() {
    let theme = WmTheme::default();
    let config = test_config();
    let win = Window::new(&config, 0, 0, &theme);
    assert_eq!(win.sdi_name("frame"), "test_win.frame");
    assert_eq!(win.sdi_name("titlebar"), "test_win.titlebar");
}

#[test]
fn panel_not_draggable() {
    let config = WindowConfig {
        id: "taskbar".to_string(),
        title: "Taskbar".to_string(),
        x: None,
        y: None,
        width: 480,
        height: 32,
        window_type: WindowType::Panel,
        always_on_top: false,
        modal: false,
    };
    let theme = WmTheme::default();
    let win = Window::new(&config, 0, 0, &theme);
    assert!(!win.is_draggable());
    assert!(!win.is_resizable());
    assert!(!win.has_close_button());
}

#[test]
fn default_theme_reasonable() {
    let theme = WmTheme::default();
    assert!(theme.titlebar_height > 0);
    assert!(theme.button_size > 0);
    assert!(theme.button_size <= theme.titlebar_height);
    assert!(theme.resize_handle_size > 0);
}

#[test]
fn kiosk_mode_content_rect_is_full_bounds() {
    let theme = WmTheme::default();
    let config = test_config();
    let mut win = Window::new(&config, 0, 0, &theme);
    win.fullscreen_kiosk = true;
    win.x = 0;
    win.y = 0;
    win.outer_w = 480;
    win.outer_h = 272;
    let (cx, cy, cw, ch) = win.content_rect(&theme);
    assert_eq!((cx, cy, cw, ch), (0, 0, 480, 272));
}

#[test]
fn kiosk_mode_titlebar_is_none() {
    let theme = WmTheme::default();
    let config = test_config();
    let mut win = Window::new(&config, 10, 20, &theme);
    assert!(win.titlebar_rect(&theme).is_some());
    win.fullscreen_kiosk = true;
    assert!(win.titlebar_rect(&theme).is_none());
}

fn macos_theme() -> WmTheme {
    WmTheme {
        button_side: "left".to_string(),
        title_align: "center".to_string(),
        button_size: 12,
        button_spacing: 6,
        ..WmTheme::default()
    }
}

fn rect_right(r: (i32, i32, u32, u32)) -> i32 {
    r.0 + r.2 as i32
}

#[test]
fn left_side_order_is_close_minimize_maximize() {
    let theme = macos_theme();
    let win = Window::new(&test_config(), 0, 0, &theme);
    let close = win.close_btn_rect(&theme).unwrap();
    let min = win.minimize_btn_rect(&theme).unwrap();
    let max = win.maximize_btn_rect(&theme).unwrap();
    assert!(close.0 < min.0 && min.0 < max.0);
    // Buttons don't overlap each other.
    assert!(rect_right(close) <= min.0 && rect_right(min) <= max.0);
}

#[test]
fn right_side_order_is_minimize_maximize_close() {
    let theme = WmTheme::default();
    let win = Window::new(&test_config(), 0, 0, &theme);
    let close = win.close_btn_rect(&theme).unwrap();
    let min = win.minimize_btn_rect(&theme).unwrap();
    let max = win.maximize_btn_rect(&theme).unwrap();
    assert!(min.0 < max.0 && max.0 < close.0);
}

#[test]
fn centered_title_is_centered_on_titlebar() {
    let theme = macos_theme();
    let mut cfg = test_config();
    cfg.title = "Terminal".to_string();
    let win = Window::new(&cfg, 0, 0, &theme);
    let (tx, _, tw, _) = win.titlebar_rect(&theme).unwrap();
    let layout = win.title_layout(&theme).unwrap();
    assert_eq!(layout.text, "Terminal");
    assert_eq!(
        layout.text_w as i32,
        text_px("Terminal", theme.titlebar_font_size)
    );
    let center = layout.x + layout.text_w as i32 / 2;
    let bar_center = tx + tw as i32 / 2;
    assert!((center - bar_center).abs() <= 1, "{center} vs {bar_center}");
}

#[test]
fn centered_title_never_intersects_buttons() {
    let theme = macos_theme();
    // Narrow window: centering would otherwise push text under buttons.
    for width in [60u32, 90, 120, 160, 200, 400] {
        let mut cfg = test_config();
        cfg.width = width;
        cfg.title = "A fairly long terminal window title".to_string();
        let win = Window::new(&cfg, 5, 0, &theme);
        let layout = win.title_layout(&theme).unwrap();
        let text_end = layout.x + layout.text_w as i32;
        for btn in [
            win.close_btn_rect(&theme),
            win.minimize_btn_rect(&theme),
            win.maximize_btn_rect(&theme),
        ]
        .into_iter()
        .flatten()
        {
            let overlaps = layout.text_w > 0 && layout.x < rect_right(btn) && btn.0 < text_end;
            assert!(!overlaps, "width {width}: title overlaps button {btn:?}");
        }
    }
}

#[test]
fn left_aligned_title_starts_after_left_buttons() {
    let theme = WmTheme {
        title_align: "left".to_string(),
        ..macos_theme()
    };
    let win = Window::new(&test_config(), 0, 0, &theme);
    let layout = win.title_layout(&theme).unwrap();
    let max = win.maximize_btn_rect(&theme).unwrap();
    assert!(layout.x >= rect_right(max));
}

#[test]
fn long_title_gets_ellipsis() {
    let theme = WmTheme::default();
    let mut cfg = test_config();
    cfg.width = 120;
    cfg.title = "An Extremely Long Window Title That Cannot Possibly Fit".to_string();
    let win = Window::new(&cfg, 0, 0, &theme);
    let layout = win.title_layout(&theme).unwrap();
    assert!(layout.text.ends_with("..."), "{}", layout.text);
    assert!(layout.text_w <= layout.avail_w);
    assert!(layout.text.len() > 3);
}

#[test]
fn short_title_not_truncated() {
    let theme = WmTheme::default();
    let win = Window::new(&test_config(), 0, 0, &theme);
    assert_eq!(win.title_layout(&theme).unwrap().text, "Test Window");
}

#[test]
fn truncate_to_width_edge_cases() {
    assert_eq!(truncate_to_width("Hi", 8, 200), "Hi");
    assert_eq!(truncate_to_width("Hello World", 8, 0), "");
    let t = truncate_to_width("Hello World", 8, 30);
    assert!(t.ends_with("...") && text_px(&t, 8) <= 30, "{t}");
    // Multi-byte characters never split.
    let t = truncate_to_width("日本語のタイトルです", 8, 30);
    assert!(t.is_empty() || t.ends_with("..."));
}

#[test]
fn kiosk_mode_default_false() {
    let theme = WmTheme::default();
    let config = test_config();
    let win = Window::new(&config, 0, 0, &theme);
    assert!(!win.fullscreen_kiosk);
}
