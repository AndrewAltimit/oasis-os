//! PSIX-style status bar -- top bar with version, clock, battery, and tabs.
//!
//! Occupies the top 24 pixels of the 480x272 screen. Creates and updates
//! SDI objects to display system status and top-level navigation tabs.

use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::platform::{BatteryState, PowerInfo, SystemTime};
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::{ensure_border, ensure_pill, ensure_text, hide_objects};
use crate::theme;

/// Top-level tabs (cycled with L trigger).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopTab {
    /// Main dashboard (app grid).
    Apps,
    /// Module / plugin manager.
    Mods,
    /// Network status.
    Net,
}

impl TopTab {
    /// Cycle to the next tab.
    pub fn next(self) -> Self {
        match self {
            Self::Apps => Self::Mods,
            Self::Mods => Self::Net,
            Self::Net => Self::Apps,
        }
    }

    /// Display label for the tab.
    pub fn label(self) -> &'static str {
        match self {
            Self::Apps => "APPS",
            Self::Mods => "MODS",
            Self::Net => "NET",
        }
    }

    /// All tabs in order.
    pub const ALL: &[TopTab] = &[TopTab::Apps, TopTab::Mods, TopTab::Net];
}

/// Month names for date display.
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Runtime state for the top status bar.
#[derive(Debug)]
pub struct StatusBar {
    /// Currently selected top tab.
    pub active_tab: TopTab,
    /// Cached clock string (updated each frame).
    clock_text: String,
    /// Cached date string.
    date_text: String,
    /// Cached battery string.
    battery_text: String,
    /// Cached CPU frequency string.
    cpu_text: String,
}

impl StatusBar {
    /// Create a new status bar with default state.
    pub fn new() -> Self {
        Self {
            active_tab: TopTab::Apps,
            clock_text: "00:00".to_string(),
            date_text: String::new(),
            battery_text: String::new(),
            cpu_text: String::new(),
        }
    }

    /// Cycle to the next top tab.
    pub fn next_tab(&mut self) {
        self.active_tab = self.active_tab.next();
    }

    /// Update cached system info strings.
    pub fn update_info(&mut self, time: Option<&SystemTime>, power: Option<&PowerInfo>) {
        if let Some(t) = time {
            self.clock_text = format!("{:02}:{:02}", t.hour, t.minute);
            let month_name = if t.month >= 1 && t.month <= 12 {
                MONTHS[(t.month - 1) as usize]
            } else {
                "???"
            };
            self.date_text = format!("{month_name} {}, {}", t.day, t.year);
        }
        if let Some(p) = power {
            self.battery_text = match p.state {
                BatteryState::NoBattery => "AC".to_string(),
                BatteryState::Full => "FULL".to_string(),
                _ => {
                    let pct = p.battery_percent.unwrap_or(0);
                    let icon = match pct {
                        0..=20 => "[|    ]",
                        21..=40 => "[||   ]",
                        41..=60 => "[|||  ]",
                        61..=80 => "[|||| ]",
                        _ => "[|||||]",
                    };
                    format!("{pct}% {icon}")
                },
            };
            if p.cpu.current_mhz > 0 {
                self.cpu_text = format!("{}MHz", p.cpu.current_mhz);
            } else {
                self.cpu_text.clear();
            }
        }
    }

    /// Synchronize SDI objects to reflect current status bar state.
    ///
    /// Accepts an `ActiveTheme` for skin-driven colors and `SkinFeatures`
    /// for content visibility toggles. Pass `&ActiveTheme::default()` and
    /// `&SkinFeatures::default()` for legacy behaviour.
    pub fn update_sdi(
        &self,
        sdi: &mut SdiRegistry,
        at: &ActiveTheme,
        features: &crate::skin::SkinFeatures,
    ) {
        let bar_h = at.statusbar_height;
        let font_small = at.font_small;
        let screen_w = theme::SCREEN_W;

        // Semi-transparent background bar.
        if !sdi.contains("bar_top") {
            let obj = sdi.create("bar_top");
            obj.x = 0;
            obj.y = 0;
            obj.w = screen_w;
            obj.h = bar_h;
            obj.color = at.statusbar_bg;
            obj.overlay = true;
            obj.z = 900;
        }
        if let Ok(obj) = sdi.get_mut("bar_top") {
            obj.color = at.statusbar_bg;
            obj.h = bar_h;
            obj.visible = true;
            obj.gradient_top = at.statusbar_gradient_top;
            obj.gradient_bottom = at.statusbar_gradient_bottom;
        }

        // Thin line separator below status bar.
        ensure_border(
            sdi,
            "bar_top_line",
            0,
            bar_h as i32 - 1,
            screen_w,
            1,
            at.separator_color,
        );

        // Vertically center text within the bar.
        let text_y = (bar_h as i32 - font_small as i32) / 2;
        let char_w = font_small.max(8) as i32 / 8 * 8;

        // Battery + CPU info (left side).
        if features.show_battery {
            ensure_text(sdi, "bar_battery", 6, text_y, font_small, at.battery_color);
            if let Ok(obj) = sdi.get_mut("bar_battery") {
                let mut info = self.battery_text.clone();
                if !self.cpu_text.is_empty() {
                    info = format!("{info}  {}", self.cpu_text);
                }
                obj.text = Some(info);
                obj.visible = true;
            }
        } else if let Ok(obj) = sdi.get_mut("bar_battery") {
            obj.visible = false;
        }

        // Clock + date (right side, right-aligned).  Compute first so
        // version can check for overlap.
        let clock_x = if features.show_clock {
            let clock_str = if self.date_text.is_empty() {
                self.clock_text.clone()
            } else {
                format!("{} {}", self.clock_text, self.date_text)
            };
            let clock_w = clock_str.len() as i32 * char_w;
            let cx = screen_w as i32 - clock_w - 6;
            ensure_text(sdi, "bar_clock", cx, text_y, font_small, at.clock_color);
            if let Ok(obj) = sdi.get_mut("bar_clock") {
                obj.text = Some(clock_str);
                obj.visible = true;
            }
            cx
        } else {
            if let Ok(obj) = sdi.get_mut("bar_clock") {
                obj.visible = false;
            }
            screen_w as i32
        };

        // Version label (center area) -- hidden when it would overlap clock.
        if features.show_version {
            let ver = "Version 0.1";
            let ver_w = ver.len() as i32 * char_w;
            let ver_x = (screen_w as i32 - ver_w) / 2;
            if ver_x + ver_w <= clock_x {
                ensure_text(
                    sdi,
                    "bar_version",
                    ver_x,
                    text_y,
                    font_small,
                    at.version_color,
                );
                if let Ok(obj) = sdi.get_mut("bar_version") {
                    obj.text = Some(ver.to_string());
                    obj.visible = true;
                }
            } else if let Ok(obj) = sdi.get_mut("bar_version") {
                obj.visible = false;
            }
        } else if let Ok(obj) = sdi.get_mut("bar_version") {
            obj.visible = false;
        }

        // Category label before tabs (PSIX: "MSO").
        if features.show_tabs {
            let mso_y = bar_h as i32 + (theme::TAB_H - font_small as i32) / 2;
            ensure_text(
                sdi,
                "bar_mso",
                6,
                mso_y,
                font_small,
                at.category_label_color,
            );
            if let Ok(obj) = sdi.get_mut("bar_mso") {
                obj.text = Some("OSS".to_string());
                obj.visible = true;
            }
        } else if let Ok(obj) = sdi.get_mut("bar_mso") {
            obj.visible = false;
        }

        // Tab row: single pill-shaped SDI objects (replaces 4-edge borders).
        let tab_y = bar_h as i32;
        for (i, tab) in TopTab::ALL.iter().enumerate() {
            let name = format!("bar_tab_{i}");
            let bg_name = format!("bar_tab_bg_{i}");

            if !features.show_tabs {
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
                if let Ok(obj) = sdi.get_mut(&bg_name) {
                    obj.visible = false;
                }
                continue;
            }

            let x = theme::TAB_START_X + (i as i32) * (theme::TAB_W + theme::TAB_GAP);
            let tw = theme::TAB_W as u32;
            let th = theme::TAB_H as u32;

            let is_active = *tab == self.active_tab;

            // Hide legacy 4-edge border objects.
            for suffix in &["bt", "bb", "bl", "br"] {
                let edge_name = format!("bar_tab_{suffix}_{i}");
                if let Ok(obj) = sdi.get_mut(&edge_name) {
                    obj.visible = false;
                }
            }

            // Single pill tab background (replaces fill + 4 borders).
            if is_active {
                ensure_pill(
                    sdi,
                    &bg_name,
                    x,
                    tab_y,
                    tw,
                    th,
                    at.tab_active_fill,
                    Color::rgba(255, 255, 255, at.tab_active_alpha),
                );
            } else {
                // Inactive: transparent fill, dim stroke.
                ensure_pill(
                    sdi,
                    &bg_name,
                    x,
                    tab_y,
                    tw,
                    th,
                    at.tab_inactive_fill,
                    Color::rgba(255, 255, 255, at.tab_inactive_alpha),
                );
            }

            // Tab text (centered in tab).
            let tx = x + (theme::TAB_W - (tab.label().len() as i32 * char_w)) / 2;
            let tab_text_y = tab_y + (theme::TAB_H - font_small as i32) / 2;
            ensure_text(
                sdi,
                &name,
                tx.max(x + 2),
                tab_text_y,
                font_small,
                Color::WHITE,
            );
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.text = Some(tab.label().to_string());
                obj.text_color = if is_active {
                    at.media_tab_active
                } else {
                    at.media_tab_inactive
                };
            }
        }

        // Hide legacy CPU text object (merged into battery display).
        if let Ok(obj) = sdi.get_mut("bar_cpu") {
            obj.visible = false;
        }
    }

    /// Hide all status bar SDI objects.
    pub fn hide_sdi(sdi: &mut SdiRegistry) {
        hide_objects(
            sdi,
            &[
                "bar_top",
                "bar_top_line",
                "bar_version",
                "bar_clock",
                "bar_battery",
                "bar_cpu",
                "bar_mso",
            ],
        );
        for i in 0..TopTab::ALL.len() {
            for prefix in &[
                "bar_tab_",
                "bar_tab_bg_",
                "bar_tab_bt_",
                "bar_tab_bb_",
                "bar_tab_bl_",
                "bar_tab_br_",
            ] {
                let name = format!("{prefix}{i}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
        }
    }
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
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
        let feat = crate::skin::SkinFeatures::default();
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
}
