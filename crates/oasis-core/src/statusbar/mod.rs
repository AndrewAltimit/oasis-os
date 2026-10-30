//! PSIX-style status bar -- top bar with version, clock, battery, and tabs.
//!
//! Occupies the top 24 pixels of the 480x272 screen. Creates and updates
//! SDI objects to display system status and top-level navigation tabs.
//!
//! Power and Wi-Fi indicators are vector glyphs
//! ([`oasis_vector::status_glyphs`]) by default. Glyphs are not SDI
//! primitives, so `update_sdi` reserves an invisible *glyph slot* object
//! per indicator (position, size, colour, and the glyph state as its
//! text, which keeps SDI dirty tracking exact) and the shell paints them
//! after the SDI pass with [`render_status_glyphs`]. The legacy text form
//! (`AC`, `75% [|||| ]`) remains available via [`IndicatorStyle::Text`].
//!
//! Layout: `glyphs` (indicator glyphs + slots), `tabs` (the top tab
//! row); this module owns the cached state and the bar strip itself.

mod glyphs;
mod tabs;

pub use glyphs::{PowerGlyph, StatusGlyph, render_status_glyphs};
pub use tabs::TopTab;

use oasis_types::backend::TextureId;
use oasis_types::bitmap_font::glyph_advance_scaled;

use crate::active_theme::ActiveTheme;
use crate::platform::{BatteryState, PowerInfo, SystemTime, WifiInfo};
use crate::sdi::helpers::{ensure_border, ensure_text, hide_objects};
use crate::sdi::{SdiObject, SdiRegistry};
use glyphs::{GLYPH_SLOTS, layout_glyph_slots};
use tabs::{TabTextures, hide_tab_row, update_tab_row};

/// Measure the pixel width of a text string using proportional glyph metrics.
fn text_px(s: &str, font_size: u16) -> i32 {
    s.chars()
        .map(|c| glyph_advance_scaled(c, font_size) as i32)
        .sum()
}

/// Apply the skin's status-bar text shadow (if any) to a text object.
fn apply_text_shadow(obj: &mut SdiObject, at: &ActiveTheme) {
    if at.bar.text_shadow {
        obj.text_shadow_offset = Some((1, 1));
        obj.text_shadow_color = Some(at.bar.text_shadow_color);
    }
}

/// How the status bar presents power / network indicators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndicatorStyle {
    /// Vector glyphs (battery with level/charging, AC plug, Wi-Fi) next to
    /// a short percentage label.
    #[default]
    Glyphs,
    /// Legacy text: `AC`, `FULL`, `75% [|||| ]`.
    Text,
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

/// Plain SDI objects owned by the bar strip (tabs and glyph slots are
/// tracked by their modules). `bar_cpu` is a legacy object (CPU text is
/// merged into the battery label) kept hidden.
const BAR_OBJECTS: [&str; 7] = [
    "bar_top",
    "bar_top_line",
    "bar_version",
    "bar_clock",
    "bar_battery",
    "bar_cpu",
    "bar_mso",
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
    /// Cached merged battery+CPU display string (built once per
    /// `update_info` instead of once per frame).
    battery_display: String,
    /// Cached merged clock+date display string.
    clock_display: String,
    /// How power / Wi-Fi indicators are drawn (vector glyphs by default).
    pub indicator_style: IndicatorStyle,
    /// Power glyph state (None until power info arrives).
    power_glyph: Option<PowerGlyph>,
    /// Wi-Fi glyph state: `Some(connected)` while the radio is available.
    wifi_glyph: Option<bool>,
    /// Label shown beside the glyphs: percentage (+ CPU), no ASCII art.
    glyph_display: String,
    /// Texture drawn as the active top-tab pill (from the skin's
    /// `bar_overrides.tab_texture_active` asset; uploaded by the shell on
    /// skin swap). None = plain pill fill.
    pub tab_texture_active: Option<TextureId>,
    /// Texture drawn as inactive top-tab pills.
    pub tab_texture_inactive: Option<TextureId>,
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
            battery_display: String::new(),
            clock_display: "00:00".to_string(),
            indicator_style: IndicatorStyle::Glyphs,
            power_glyph: None,
            wifi_glyph: None,
            glyph_display: String::new(),
            tab_texture_active: None,
            tab_texture_inactive: None,
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
        let mut power_label = String::new();
        if let Some(p) = power {
            self.power_glyph = Some(PowerGlyph::from_power(p));
            power_label = match (p.state, p.battery_percent) {
                (BatteryState::NoBattery, _) => String::new(),
                (BatteryState::Full, None) => "FULL".to_string(),
                (_, pct) => format!("{}%", pct.unwrap_or(0)),
            };
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
        // Rebuild the merged display strings once here so the per-frame
        // `update_sdi` path never allocates for them.
        self.battery_display = if self.cpu_text.is_empty() {
            self.battery_text.clone()
        } else {
            format!("{}  {}", self.battery_text, self.cpu_text)
        };
        if power.is_some() {
            self.glyph_display = match (power_label.is_empty(), self.cpu_text.is_empty()) {
                (_, true) => power_label,
                (true, false) => self.cpu_text.clone(),
                (false, false) => format!("{power_label}  {}", self.cpu_text),
            };
        }
        self.clock_display = if self.date_text.is_empty() {
            self.clock_text.clone()
        } else {
            format!("{} {}", self.clock_text, self.date_text)
        };
    }

    /// Update the Wi-Fi indicator. `None` or unavailable hardware hides it.
    pub fn update_wifi(&mut self, info: Option<&WifiInfo>) {
        self.wifi_glyph = info.filter(|i| i.available).map(|i| i.connected);
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
        let screen_w = at.screen_w;

        // Semi-transparent background bar.
        if !sdi.contains("bar_top") {
            let obj = sdi.create("bar_top");
            obj.x = 0;
            obj.y = 0;
            obj.w = screen_w;
            obj.h = bar_h;
            obj.color = at.bar.statusbar_bg;
            obj.overlay = true;
            obj.z = 900;
        }
        if let Ok(obj) = sdi.get_mut("bar_top") {
            obj.color = at.bar.statusbar_bg;
            obj.h = bar_h;
            obj.visible = true;
            obj.gradient_top = at.bar.statusbar_gradient_top;
            obj.gradient_bottom = at.bar.statusbar_gradient_bottom;
        }

        // Thin line separator below status bar.
        ensure_border(
            sdi,
            "bar_top_line",
            0,
            bar_h as i32 - 1,
            screen_w,
            1,
            at.bar.separator_color,
        );

        // Vertically center text within the bar.
        let text_y = (bar_h as i32 - font_small as i32) / 2;

        // Battery + CPU info (left side): indicator glyphs, then a short
        // label (or the legacy all-text form).
        if features.show_battery {
            let (text_x, text) = if self.indicator_style == IndicatorStyle::Glyphs {
                let x = layout_glyph_slots(sdi, at, 6, self.wifi_glyph, self.power_glyph);
                (x, &self.glyph_display)
            } else {
                hide_objects(sdi, &GLYPH_SLOTS);
                (6, &self.battery_display)
            };
            ensure_text(
                sdi,
                "bar_battery",
                text_x,
                text_y,
                font_small,
                at.bar.battery_color,
            );
            if let Ok(obj) = sdi.get_mut("bar_battery") {
                obj.x = text_x;
                obj.set_text(text);
                obj.visible = true;
                apply_text_shadow(obj, at);
            }
        } else {
            hide_objects(sdi, &GLYPH_SLOTS);
            hide_objects(sdi, &["bar_battery"]);
        }

        // Clock + date (right side, right-aligned).  Compute first so
        // version can check for overlap.  When `clock_in_bottombar` is set
        // the clock is rendered by the bottom bar (XP-style) and we hide
        // the top-right copy here.
        let clock_x = if features.show_clock && !features.clock_in_bottombar {
            let clock_w = text_px(&self.clock_display, font_small);
            let cx = screen_w as i32 - clock_w - 6;
            ensure_text(sdi, "bar_clock", cx, text_y, font_small, at.bar.clock_color);
            if let Ok(obj) = sdi.get_mut("bar_clock") {
                // `ensure_text` places the object only when creating it;
                // the width changes with the text (the date arrives after
                // the first frame, "Oct 9" vs "Oct 30") and with the skin.
                obj.x = cx;
                obj.y = text_y;
                obj.set_text(&self.clock_display);
                obj.visible = true;
                apply_text_shadow(obj, at);
            }
            cx
        } else {
            hide_objects(sdi, &["bar_clock"]);
            screen_w as i32
        };

        // Version label (center area) -- hidden when it would overlap clock.
        let ver = &at.bar.version_text;
        let ver_w = text_px(ver, font_small);
        let ver_x = (screen_w as i32 - ver_w) / 2;
        if features.show_version && ver_x + ver_w <= clock_x {
            ensure_text(
                sdi,
                "bar_version",
                ver_x,
                text_y,
                font_small,
                at.bar.version_color,
            );
            if let Ok(obj) = sdi.get_mut("bar_version") {
                obj.x = ver_x;
                obj.y = text_y;
                obj.set_text(ver);
                obj.visible = true;
                apply_text_shadow(obj, at);
            }
        } else {
            hide_objects(sdi, &["bar_version"]);
        }

        // Category label before tabs (PSIX: "MSO").
        if features.show_tabs {
            let mso_y = bar_h as i32 + (at.tab_h - font_small as i32) / 2;
            ensure_text(
                sdi,
                "bar_mso",
                6,
                mso_y,
                font_small,
                at.bar.category_label_color,
            );
            if let Ok(obj) = sdi.get_mut("bar_mso") {
                obj.set_text(&at.bar.category_label);
                obj.visible = true;
            }
        } else {
            hide_objects(sdi, &["bar_mso"]);
        }

        update_tab_row(
            sdi,
            at,
            features.show_tabs,
            self.active_tab,
            TabTextures {
                active: self.tab_texture_active,
                inactive: self.tab_texture_inactive,
            },
        );

        // Hide legacy CPU text object (merged into battery display).
        hide_objects(sdi, &["bar_cpu"]);
    }

    /// Hide all status bar SDI objects.
    pub fn hide_sdi(sdi: &mut SdiRegistry) {
        hide_objects(sdi, &BAR_OBJECTS);
        hide_objects(sdi, &GLYPH_SLOTS);
        hide_tab_row(sdi);
    }
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
