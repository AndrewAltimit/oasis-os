use super::glyphs::{GLYPH_SLOT_POWER, GLYPH_SLOT_WIFI};
use super::*;

#[test]
fn tab_cycle() {
    let mut bar = StatusBar::new();
    assert_eq!(bar.active_tab, TopTab::Apps);
    bar.next_tab();
    assert_eq!(bar.active_tab, TopTab::Mods);
    bar.next_tab();
    assert_eq!(bar.active_tab, TopTab::Net);
    bar.next_tab();
    assert_eq!(bar.active_tab, TopTab::Apps);
}

#[test]
fn update_info_clock() {
    let mut bar = StatusBar::new();
    let time = SystemTime {
        year: 2025,
        month: 6,
        day: 15,
        hour: 14,
        minute: 30,
        second: 0,
    };
    bar.update_info(Some(&time), None);
    assert_eq!(bar.clock_text, "14:30");
}

#[test]
fn update_info_battery() {
    let mut bar = StatusBar::new();
    let power = PowerInfo {
        battery_percent: Some(75),
        battery_minutes: None,
        state: BatteryState::Discharging,
        cpu: crate::platform::CpuClock {
            current_mhz: 333,
            max_mhz: 333,
        },
    };
    bar.update_info(None, Some(&power));
    assert!(bar.battery_text.contains("75%"));
    assert_eq!(bar.cpu_text, "333MHz");
}

#[test]
fn update_info_no_battery() {
    let mut bar = StatusBar::new();
    let power = PowerInfo {
        battery_percent: None,
        battery_minutes: None,
        state: BatteryState::NoBattery,
        cpu: crate::platform::CpuClock {
            current_mhz: 0,
            max_mhz: 0,
        },
    };
    bar.update_info(None, Some(&power));
    assert_eq!(bar.battery_text, "AC");
    assert!(bar.cpu_text.is_empty());
}

#[test]
fn update_sdi_creates_objects() {
    let bar = StatusBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures {
        show_tabs: true,
        show_version: true,
        clock_in_bottombar: false,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);
    assert!(sdi.contains("bar_top"));
    assert!(sdi.contains("bar_version"));
    assert!(sdi.contains("bar_clock"));
    assert!(sdi.contains("bar_tab_0"));
    assert!(sdi.contains("bar_tab_1"));
    assert!(sdi.contains("bar_tab_2"));
}

#[test]
fn bar_top_is_overlay() {
    let bar = StatusBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);
    assert!(sdi.get("bar_top").unwrap().overlay);
}

#[test]
fn toptab_labels() {
    assert_eq!(TopTab::Apps.label(), "APPS");
    assert_eq!(TopTab::Mods.label(), "MODS");
    assert_eq!(TopTab::Net.label(), "NET");
}

#[test]
fn toptab_next_from_apps() {
    assert_eq!(TopTab::Apps.next(), TopTab::Mods);
}

#[test]
fn toptab_next_from_net_wraps() {
    assert_eq!(TopTab::Net.next(), TopTab::Apps);
}

#[test]
fn statusbar_default_state() {
    let bar = StatusBar::new();
    assert_eq!(bar.active_tab, TopTab::Apps);
    assert_eq!(bar.clock_text, "00:00");
    assert!(bar.date_text.is_empty());
    assert!(bar.battery_text.is_empty());
    assert!(bar.cpu_text.is_empty());
}

#[test]
fn statusbar_default_trait() {
    let bar = StatusBar::default();
    assert_eq!(bar.active_tab, TopTab::Apps);
}

#[test]
fn update_info_date_formatting() {
    let mut bar = StatusBar::new();
    let time = SystemTime {
        year: 2025,
        month: 12,
        day: 25,
        hour: 0,
        minute: 0,
        second: 0,
    };
    bar.update_info(Some(&time), None);
    assert_eq!(bar.date_text, "December 25, 2025");
}

#[test]
fn update_info_invalid_month() {
    let mut bar = StatusBar::new();
    let time = SystemTime {
        year: 2025,
        month: 13,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    };
    bar.update_info(Some(&time), None);
    assert!(bar.date_text.contains("???"));
}

#[test]
fn update_info_battery_low() {
    let mut bar = StatusBar::new();
    let power = PowerInfo {
        battery_percent: Some(10),
        battery_minutes: None,
        state: BatteryState::Discharging,
        cpu: crate::platform::CpuClock {
            current_mhz: 0,
            max_mhz: 0,
        },
    };
    bar.update_info(None, Some(&power));
    assert!(bar.battery_text.contains("10%"));
    assert!(bar.battery_text.contains("[|    ]"));
}

#[test]
fn update_info_battery_full_state() {
    let mut bar = StatusBar::new();
    let power = PowerInfo {
        battery_percent: Some(100),
        battery_minutes: None,
        state: BatteryState::Full,
        cpu: crate::platform::CpuClock {
            current_mhz: 0,
            max_mhz: 0,
        },
    };
    bar.update_info(None, Some(&power));
    assert_eq!(bar.battery_text, "FULL");
}

#[test]
fn update_info_battery_icons() {
    let mut bar = StatusBar::new();
    // Test different battery percentage ranges.
    for (pct, expected_icon) in &[
        (5, "[|    ]"),
        (25, "[||   ]"),
        (50, "[|||  ]"),
        (70, "[|||| ]"),
        (95, "[|||||]"),
    ] {
        let power = PowerInfo {
            battery_percent: Some(*pct),
            battery_minutes: None,
            state: BatteryState::Discharging,
            cpu: crate::platform::CpuClock {
                current_mhz: 0,
                max_mhz: 0,
            },
        };
        bar.update_info(None, Some(&power));
        assert!(bar.battery_text.contains(expected_icon));
    }
}

#[test]
fn update_info_cpu_text() {
    let mut bar = StatusBar::new();
    let power = PowerInfo {
        battery_percent: Some(50),
        battery_minutes: None,
        state: BatteryState::Discharging,
        cpu: crate::platform::CpuClock {
            current_mhz: 222,
            max_mhz: 333,
        },
    };
    bar.update_info(None, Some(&power));
    assert_eq!(bar.cpu_text, "222MHz");
}

#[test]
fn update_info_cpu_zero_clears() {
    let mut bar = StatusBar::new();
    bar.cpu_text = "333MHz".to_string();
    let power = PowerInfo {
        battery_percent: Some(50),
        battery_minutes: None,
        state: BatteryState::Discharging,
        cpu: crate::platform::CpuClock {
            current_mhz: 0,
            max_mhz: 333,
        },
    };
    bar.update_info(None, Some(&power));
    assert!(bar.cpu_text.is_empty());
}

#[test]
fn hide_sdi_hides_all_objects() {
    let bar = StatusBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures {
        clock_in_bottombar: false,
        show_version: true,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);

    StatusBar::hide_sdi(&mut sdi);

    assert!(!sdi.get("bar_top").unwrap().visible);
    assert!(!sdi.get("bar_version").unwrap().visible);
    assert!(!sdi.get("bar_clock").unwrap().visible);
    assert!(!sdi.get("bar_battery").unwrap().visible);
}

#[test]
fn tabs_hidden_when_disabled() {
    let bar = StatusBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();

    // First enable to create objects.
    let mut feat = crate::skin::SkinFeatures {
        show_tabs: true,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);

    // Now disable and verify they're hidden.
    feat.show_tabs = false;
    bar.update_sdi(&mut sdi, &at, &feat);

    assert!(!sdi.get("bar_tab_0").unwrap().visible);
    assert!(!sdi.get("bar_mso").unwrap().visible);
}

#[test]
fn battery_hidden_when_disabled() {
    let bar = StatusBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();

    // First enable to create objects.
    let mut feat = crate::skin::SkinFeatures {
        show_version: true,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);

    // Now disable and verify they're hidden.
    feat.show_battery = false;
    bar.update_sdi(&mut sdi, &at, &feat);

    assert!(!sdi.get("bar_battery").unwrap().visible);
}

#[test]
fn clock_hidden_when_disabled() {
    let bar = StatusBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();

    // First enable to create objects (top-right rendering path).
    let mut feat = crate::skin::SkinFeatures {
        clock_in_bottombar: false,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);

    // Now disable and verify they're hidden.
    feat.show_clock = false;
    bar.update_sdi(&mut sdi, &at, &feat);

    assert!(!sdi.get("bar_clock").unwrap().visible);
}

#[test]
fn version_hidden_when_disabled() {
    let bar = StatusBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();

    // First enable to create objects.
    let mut feat = crate::skin::SkinFeatures {
        show_version: true,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);

    // Now disable and verify they're hidden.
    feat.show_version = false;
    bar.update_sdi(&mut sdi, &at, &feat);

    assert!(!sdi.get("bar_version").unwrap().visible);
}

#[test]
fn active_tab_has_different_color() {
    let mut bar = StatusBar::new();
    bar.active_tab = TopTab::Apps;
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures {
        show_tabs: true,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);

    let apps_tab = sdi.get("bar_tab_0").unwrap();
    let mods_tab = sdi.get("bar_tab_1").unwrap();
    assert_ne!(apps_tab.text_color, mods_tab.text_color);
}

#[test]
fn tab_textures_stamped_by_state() {
    let mut bar = StatusBar::new();
    bar.tab_texture_active = Some(TextureId(7));
    bar.tab_texture_inactive = Some(TextureId(8));
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures {
        show_tabs: true,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);

    assert_eq!(sdi.get("bar_tab_bg_0").unwrap().texture, Some(TextureId(7)));
    assert_eq!(sdi.get("bar_tab_bg_1").unwrap().texture, Some(TextureId(8)));

    // Switching tabs swaps the textures on the pills.
    bar.next_tab();
    bar.update_sdi(&mut sdi, &at, &feat);
    assert_eq!(sdi.get("bar_tab_bg_0").unwrap().texture, Some(TextureId(8)));
    assert_eq!(sdi.get("bar_tab_bg_1").unwrap().texture, Some(TextureId(7)));
}

#[test]
fn tab_pills_untouched_without_texture_slots() {
    let bar = StatusBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures {
        show_tabs: true,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);
    // Simulate a layout.toml `texture =` assignment on the pill.
    sdi.get_mut("bar_tab_bg_0").unwrap().texture = Some(TextureId(42));
    bar.update_sdi(&mut sdi, &at, &feat);
    assert_eq!(
        sdi.get("bar_tab_bg_0").unwrap().texture,
        Some(TextureId(42)),
        "pill texture from layout must survive when no tab slots are set"
    );
}

#[test]
fn top_clock_hidden_when_clock_in_bottombar() {
    let bar = StatusBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures {
        clock_in_bottombar: true,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);
    // Either not created, or created and hidden.
    if let Ok(obj) = sdi.get("bar_clock") {
        assert!(!obj.visible);
    }
}

/// The clock is created on the first frame showing "00:00" and only
/// later gets the date; it must stay right-aligned as it grows
/// (it used to keep the x it was created at and run off the screen).
#[test]
fn clock_stays_right_aligned_when_text_grows() {
    let mut bar = StatusBar::new();
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures {
        clock_in_bottombar: false,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);
    let first_x = sdi.get("bar_clock").unwrap().x;
    let time = SystemTime {
        year: 2026,
        month: 10,
        day: 30,
        hour: 0,
        minute: 57,
        second: 0,
    };
    bar.update_info(Some(&time), None);
    bar.update_sdi(&mut sdi, &at, &feat);
    let obj = sdi.get("bar_clock").unwrap();
    let w = text_px(obj.text.as_deref().unwrap(), at.font_small);
    assert!(obj.x < first_x);
    assert_eq!(obj.x + w, at.screen_w as i32 - 6);
}

#[test]
fn clock_includes_date_when_present() {
    let mut bar = StatusBar::new();
    let time = SystemTime {
        year: 2025,
        month: 6,
        day: 15,
        hour: 14,
        minute: 30,
        second: 0,
    };
    bar.update_info(Some(&time), None);
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures {
        clock_in_bottombar: false,
        ..Default::default()
    };
    bar.update_sdi(&mut sdi, &at, &feat);

    let clock_obj = sdi.get("bar_clock").unwrap();
    let clock_str = clock_obj.text.as_ref().unwrap();
    assert!(clock_str.contains("14:30"));
    assert!(clock_str.contains("June"));
}

#[test]
fn battery_merges_with_cpu() {
    let mut bar = StatusBar::new();
    let power = PowerInfo {
        battery_percent: Some(75),
        battery_minutes: None,
        state: BatteryState::Discharging,
        cpu: crate::platform::CpuClock {
            current_mhz: 222,
            max_mhz: 333,
        },
    };
    bar.update_info(None, Some(&power));
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);

    let battery_obj = sdi.get("bar_battery").unwrap();
    let text = battery_obj.text.as_ref().unwrap();
    assert!(text.contains("75%"));
    assert!(text.contains("222MHz"));
}

fn power(state: BatteryState, pct: Option<u8>, mhz: u32) -> PowerInfo {
    PowerInfo {
        battery_percent: pct,
        battery_minutes: None,
        state,
        cpu: crate::platform::CpuClock {
            current_mhz: mhz,
            max_mhz: 333,
        },
    }
}

#[test]
fn battery_glyph_levels_and_charging() {
    let mut bar = StatusBar::new();
    for (pct, level) in [(5, 1), (25, 2), (50, 3), (70, 4), (95, 5)] {
        bar.update_info(None, Some(&power(BatteryState::Discharging, Some(pct), 0)));
        assert_eq!(
            bar.power_glyph,
            Some(PowerGlyph::Battery {
                level,
                charging: false
            }),
            "{pct}%"
        );
        assert_eq!(bar.glyph_display, format!("{pct}%"));
    }
    bar.update_info(None, Some(&power(BatteryState::Charging, Some(42), 0)));
    assert_eq!(
        bar.power_glyph,
        Some(PowerGlyph::Battery {
            level: 3,
            charging: true
        })
    );
    bar.update_info(None, Some(&power(BatteryState::Full, None, 0)));
    assert_eq!(
        bar.power_glyph,
        Some(PowerGlyph::Battery {
            level: 5,
            charging: false
        })
    );
    assert_eq!(bar.glyph_display, "FULL");
}

#[test]
fn ac_power_shows_plug_glyph_not_text() {
    let mut bar = StatusBar::new();
    bar.update_info(None, Some(&power(BatteryState::NoBattery, None, 0)));
    assert_eq!(bar.power_glyph, Some(PowerGlyph::AcPlug));
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);
    let slot = sdi.get(GLYPH_SLOT_POWER).unwrap();
    assert!(slot.visible);
    assert_eq!(slot.alpha, 0, "slot itself never paints");
    assert_eq!(slot.text.as_deref(), Some("ac"));
    assert_eq!(slot.aria_label.as_deref(), Some("AC power"));
    assert_eq!(slot.text_color, at.bar.battery_color);
    // No "AC" text any more; the (empty) label sits after the glyph.
    let label = sdi.get("bar_battery").unwrap();
    assert_eq!(label.text.as_deref(), Some(""));
    assert!(label.x >= slot.x + slot.w as i32);
}

#[test]
fn glyph_mode_label_has_no_ascii_battery() {
    let mut bar = StatusBar::new();
    bar.update_info(None, Some(&power(BatteryState::Discharging, Some(75), 333)));
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);
    let text = sdi.get("bar_battery").unwrap().text.clone().unwrap();
    assert_eq!(text, "75%  333MHz");
    let slot = sdi.get(GLYPH_SLOT_POWER).unwrap();
    assert_eq!(slot.text.as_deref(), Some("battery:4:0"));
    // Glyph is vertically centred in the bar and keeps its aspect.
    assert!(slot.y >= 0 && slot.y + slot.h as i32 <= at.statusbar_height as i32);
    assert_eq!(slot.w, 20 * slot.h / 10);
}

#[test]
fn text_indicator_style_keeps_legacy_text() {
    let mut bar = StatusBar::new();
    bar.indicator_style = IndicatorStyle::Text;
    bar.update_info(None, Some(&power(BatteryState::Discharging, Some(10), 0)));
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);
    let label = sdi.get("bar_battery").unwrap();
    assert_eq!(label.text.as_deref(), Some("10% [|    ]"));
    assert_eq!(label.x, 6);
    if let Ok(slot) = sdi.get(GLYPH_SLOT_POWER) {
        assert!(!slot.visible);
    }
}

#[test]
fn wifi_glyph_only_when_radio_available() {
    let mut bar = StatusBar::new();
    bar.update_info(None, Some(&power(BatteryState::Discharging, Some(50), 0)));
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    let wifi = |available, connected| WifiInfo {
        available,
        connected,
        ip_address: None,
        mac_address: [0; 6],
    };
    bar.update_wifi(Some(&wifi(false, false)));
    bar.update_sdi(&mut sdi, &at, &feat);
    assert!(!sdi.get(GLYPH_SLOT_WIFI).unwrap().visible);

    bar.update_wifi(Some(&wifi(true, true)));
    bar.update_sdi(&mut sdi, &at, &feat);
    let w = sdi.get(GLYPH_SLOT_WIFI).unwrap().clone();
    assert!(w.visible);
    assert_eq!(w.text.as_deref(), Some("wifi:1"));
    // Wi-Fi comes first; the battery glyph shifts right of it.
    assert!(sdi.get(GLYPH_SLOT_POWER).unwrap().x >= w.x + w.w as i32);
}

#[test]
fn glyph_state_change_dirties_scene_but_idle_frames_stay_clean() {
    let mut bar = StatusBar::new();
    bar.update_info(None, Some(&power(BatteryState::Discharging, Some(50), 0)));
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);
    sdi.take_scene_dirty();
    bar.update_sdi(&mut sdi, &at, &feat);
    assert!(!sdi.take_scene_dirty(), "idle frame must be clean");
    bar.update_info(None, Some(&power(BatteryState::Charging, Some(50), 0)));
    bar.update_sdi(&mut sdi, &at, &feat);
    assert!(sdi.take_scene_dirty(), "plugging in must redraw the glyph");
}

#[test]
fn render_status_glyphs_paints_only_visible_slots() {
    let mut bar = StatusBar::new();
    bar.update_info(None, Some(&power(BatteryState::Discharging, Some(90), 0)));
    let mut sdi = SdiRegistry::new();
    let at = crate::active_theme::ActiveTheme::default();
    let feat = crate::skin::SkinFeatures::default();
    bar.update_sdi(&mut sdi, &at, &feat);
    let mut backend = oasis_test_backend::RecordingBackend::new(480, 272);
    render_status_glyphs(&mut backend, &sdi).unwrap();
    assert!(
        !backend.commands().is_empty(),
        "battery glyph must paint something"
    );
    // Hidden bar (terminal mode etc.): nothing is painted.
    StatusBar::hide_sdi(&mut sdi);
    backend.clear_commands();
    render_status_glyphs(&mut backend, &sdi).unwrap();
    assert!(backend.commands().is_empty());
}

#[test]
fn status_glyph_keys_round_trip() {
    for g in [
        StatusGlyph::Power(PowerGlyph::AcPlug),
        StatusGlyph::Power(PowerGlyph::Battery {
            level: 2,
            charging: true,
        }),
        StatusGlyph::Wifi { connected: false },
    ] {
        assert_eq!(StatusGlyph::from_key(&g.key()), Some(g));
    }
    assert_eq!(StatusGlyph::from_key("bogus"), None);
}
