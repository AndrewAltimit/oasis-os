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

use oasis_types::backend::{SdiBackend, TextureId};
use oasis_types::bitmap_font::glyph_advance_scaled;
use oasis_types::error::Result;
use oasis_vector::icons::IconDef;
use oasis_vector::status_glyphs;

use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::platform::{BatteryState, PowerInfo, SystemTime, WifiInfo};
use crate::sdi::SdiRegistry;
use crate::sdi::helpers::{ensure_border, ensure_pill, ensure_text, hide_objects};

/// Measure the pixel width of a text string using proportional glyph metrics.
fn text_px(s: &str, font_size: u16) -> i32 {
    s.chars()
        .map(|c| glyph_advance_scaled(c, font_size) as i32)
        .sum()
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

/// Power indicator state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerGlyph {
    /// Wall power, no battery.
    AcPlug,
    /// Battery with `level` of [`status_glyphs::BATTERY_LEVELS`] cells.
    Battery { level: u8, charging: bool },
}

/// A status indicator drawn as a vector glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusGlyph {
    /// Battery / AC power.
    Power(PowerGlyph),
    /// Wi-Fi radio (only shown when the hardware is available).
    Wifi { connected: bool },
}

impl StatusGlyph {
    /// Stable text key stored in the glyph slot object.
    fn key(self) -> String {
        match self {
            Self::Power(PowerGlyph::AcPlug) => "ac".to_string(),
            Self::Power(PowerGlyph::Battery { level, charging }) => {
                format!("battery:{level}:{}", u8::from(charging))
            },
            Self::Wifi { connected } => format!("wifi:{}", u8::from(connected)),
        }
    }

    /// Parse a key written by [`Self::key`].
    fn from_key(key: &str) -> Option<Self> {
        let mut parts = key.split(':');
        match parts.next()? {
            "ac" => Some(Self::Power(PowerGlyph::AcPlug)),
            "battery" => {
                let level = parts.next()?.parse().ok()?;
                let charging = parts.next()? == "1";
                Some(Self::Power(PowerGlyph::Battery { level, charging }))
            },
            "wifi" => Some(Self::Wifi {
                connected: parts.next()? == "1",
            }),
            _ => None,
        }
    }

    /// Screen-reader description.
    fn describe(self) -> String {
        match self {
            Self::Power(PowerGlyph::AcPlug) => "AC power".to_string(),
            Self::Power(PowerGlyph::Battery { level, charging }) => {
                let pct = level as u32 * 100 / status_glyphs::BATTERY_LEVELS as u32;
                let state = if charging { ", charging" } else { "" };
                format!("Battery about {pct}%{state}")
            },
            Self::Wifi { connected: true } => "Wi-Fi connected".to_string(),
            Self::Wifi { connected: false } => "Wi-Fi disconnected".to_string(),
        }
    }

    /// Design size of the glyph.
    fn design_size(self) -> (u32, u32) {
        match self {
            Self::Power(PowerGlyph::AcPlug) => status_glyphs::AC_PLUG_SIZE,
            Self::Power(PowerGlyph::Battery { .. }) => status_glyphs::BATTERY_SIZE,
            Self::Wifi { .. } => status_glyphs::WIFI_SIZE,
        }
    }

    /// Build the glyph's vector icon in `color`.
    pub fn icon(self, color: Color) -> IconDef {
        match self {
            Self::Power(PowerGlyph::AcPlug) => status_glyphs::ac_plug(color),
            Self::Power(PowerGlyph::Battery { level, charging }) => {
                status_glyphs::battery(level, charging, color)
            },
            Self::Wifi { connected } => status_glyphs::wifi(connected, color),
        }
    }
}

/// Glyph slot objects, in left-to-right order (Wi-Fi, then power).
const GLYPH_SLOT_WIFI: &str = "bar_glyph_wifi";
const GLYPH_SLOT_POWER: &str = "bar_glyph_power";
const GLYPH_SLOTS: [&str; 2] = [GLYPH_SLOT_WIFI, GLYPH_SLOT_POWER];
/// Horizontal gap (px) after each glyph.
const GLYPH_GAP: i32 = 4;

/// Paint the status bar's vector indicator glyphs.
///
/// Call after the SDI scene has been drawn (the glyphs sit on the status
/// bar, which is an overlay object). Reads only the glyph slot objects
/// that [`StatusBar::update_sdi`] maintains, so hidden bars (terminal
/// mode, fullscreen apps, text indicator style) draw nothing.
pub fn render_status_glyphs(backend: &mut dyn SdiBackend, sdi: &SdiRegistry) -> Result<()> {
    for slot in GLYPH_SLOTS {
        let Ok(obj) = sdi.get(slot) else {
            continue;
        };
        if !obj.visible || obj.h == 0 {
            continue;
        }
        let Some(glyph) = obj.text.as_deref().and_then(StatusGlyph::from_key) else {
            continue;
        };
        let icon = glyph.icon(obj.text_color);
        let scale = obj.h as f32 / icon.height.max(1) as f32;
        let mut scene = oasis_vector::VectorScene::new(obj.w, obj.h);
        for mut op in icon.ops {
            if scale != 1.0 {
                op.scale(scale);
            }
            scene.push(op);
        }
        oasis_vector::render::render_scene_at(backend, &scene, obj.x, obj.y, 255)?;
    }
    Ok(())
}

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

/// Pre-built SDI object names for the top tab row (D7: avoids per-frame
/// `format!` allocations — indices match `TopTab::ALL`).
const TAB_TEXT_NAMES: [&str; 3] = ["bar_tab_0", "bar_tab_1", "bar_tab_2"];
const TAB_BG_NAMES: [&str; 3] = ["bar_tab_bg_0", "bar_tab_bg_1", "bar_tab_bg_2"];
/// Legacy 4-edge tab border objects (older skins may still define them in
/// layout.toml; kept hidden every frame).
const TAB_EDGE_NAMES: [[&str; 4]; 3] = [
    [
        "bar_tab_bt_0",
        "bar_tab_bb_0",
        "bar_tab_bl_0",
        "bar_tab_br_0",
    ],
    [
        "bar_tab_bt_1",
        "bar_tab_bb_1",
        "bar_tab_bl_1",
        "bar_tab_br_1",
    ],
    [
        "bar_tab_bt_2",
        "bar_tab_bb_2",
        "bar_tab_bl_2",
        "bar_tab_br_2",
    ],
];

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
    /// Cached merged battery+CPU display string (D7: built once per
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
            let pct = p.battery_percent.unwrap_or(0);
            self.power_glyph = Some(match p.state {
                BatteryState::NoBattery => PowerGlyph::AcPlug,
                BatteryState::Full => PowerGlyph::Battery {
                    level: status_glyphs::BATTERY_LEVELS,
                    charging: false,
                },
                BatteryState::Charging => PowerGlyph::Battery {
                    level: status_glyphs::battery_level(pct),
                    charging: true,
                },
                BatteryState::Discharging => PowerGlyph::Battery {
                    level: status_glyphs::battery_level(pct),
                    charging: false,
                },
            });
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

    /// Glyph pixel height for the bar: a touch taller than the small font,
    /// never taller than the bar allows.
    fn glyph_height(at: &ActiveTheme) -> u32 {
        (at.font_small as u32 + 2)
            .min(at.statusbar_height.saturating_sub(4))
            .max(6)
    }

    /// Lay out the indicator glyph slots from `x`; returns the x where the
    /// indicator label starts.
    fn update_glyph_slots(&self, sdi: &mut SdiRegistry, at: &ActiveTheme, mut x: i32) -> i32 {
        let gh = Self::glyph_height(at);
        let gy = (at.statusbar_height as i32 - gh as i32) / 2;
        let glyphs = [
            (
                GLYPH_SLOT_WIFI,
                self.wifi_glyph
                    .map(|connected| StatusGlyph::Wifi { connected }),
            ),
            (GLYPH_SLOT_POWER, self.power_glyph.map(StatusGlyph::Power)),
        ];
        for (slot, glyph) in glyphs {
            if !sdi.contains(slot) {
                let obj = sdi.create(slot);
                obj.overlay = true;
                // Never painted by the SDI pass: the slot only carries the
                // glyph's placement and state for `render_status_glyphs`.
                obj.alpha = 0;
            }
            let Ok(obj) = sdi.get_mut(slot) else {
                continue;
            };
            let Some(glyph) = glyph else {
                obj.visible = false;
                continue;
            };
            let (dw, dh) = glyph.design_size();
            let gw = dw * gh / dh.max(1);
            obj.x = x;
            obj.y = gy;
            obj.w = gw;
            obj.h = gh;
            obj.text_color = at.bar.battery_color;
            // Rewrite the state key only on change (no per-frame allocs).
            if obj.text.as_deref().and_then(StatusGlyph::from_key) != Some(glyph) {
                obj.text = Some(glyph.key());
                obj.aria_label = Some(glyph.describe());
            }
            obj.visible = true;
            x += gw as i32 + GLYPH_GAP;
        }
        x
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
            let glyphs = self.indicator_style == IndicatorStyle::Glyphs;
            let (text_x, text) = if glyphs {
                (self.update_glyph_slots(sdi, at, 6), &self.glyph_display)
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
                if at.bar.text_shadow {
                    obj.text_shadow_offset = Some((1, 1));
                    obj.text_shadow_color = Some(at.bar.text_shadow_color);
                }
            }
        } else {
            hide_objects(sdi, &GLYPH_SLOTS);
            if let Ok(obj) = sdi.get_mut("bar_battery") {
                obj.visible = false;
            }
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
                obj.set_text(&self.clock_display);
                obj.visible = true;
                if at.bar.text_shadow {
                    obj.text_shadow_offset = Some((1, 1));
                    obj.text_shadow_color = Some(at.bar.text_shadow_color);
                }
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
            let ver = &at.bar.version_text;
            let ver_w = text_px(ver, font_small);
            let ver_x = (screen_w as i32 - ver_w) / 2;
            if ver_x + ver_w <= clock_x {
                ensure_text(
                    sdi,
                    "bar_version",
                    ver_x,
                    text_y,
                    font_small,
                    at.bar.version_color,
                );
                if let Ok(obj) = sdi.get_mut("bar_version") {
                    obj.set_text(ver);
                    obj.visible = true;
                    if at.bar.text_shadow {
                        obj.text_shadow_offset = Some((1, 1));
                        obj.text_shadow_color = Some(at.bar.text_shadow_color);
                    }
                }
            } else if let Ok(obj) = sdi.get_mut("bar_version") {
                obj.visible = false;
            }
        } else if let Ok(obj) = sdi.get_mut("bar_version") {
            obj.visible = false;
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
        } else if let Ok(obj) = sdi.get_mut("bar_mso") {
            obj.visible = false;
        }

        // Tab row: single pill-shaped SDI objects (replaces 4-edge borders).
        let tab_y = bar_h as i32;
        for (i, tab) in TopTab::ALL.iter().enumerate() {
            let name = TAB_TEXT_NAMES[i];
            let bg_name = TAB_BG_NAMES[i];

            if !features.show_tabs {
                if let Ok(obj) = sdi.get_mut(name) {
                    obj.visible = false;
                }
                if let Ok(obj) = sdi.get_mut(bg_name) {
                    obj.visible = false;
                }
                continue;
            }

            let x = at.tab_start_x + (i as i32) * (at.tab_w + at.tab_gap);
            let tw = at.tab_w as u32;
            let th = at.tab_h as u32;

            let is_active = *tab == self.active_tab;

            // Hide legacy 4-edge border objects.
            for edge_name in &TAB_EDGE_NAMES[i] {
                if let Ok(obj) = sdi.get_mut(edge_name) {
                    obj.visible = false;
                }
            }

            // Single pill tab background (replaces fill + 4 borders).
            if is_active {
                ensure_pill(
                    sdi,
                    bg_name,
                    x,
                    tab_y,
                    tw,
                    th,
                    at.bar.tab_active_fill,
                    at.bar.tab_active_stroke,
                );
            } else {
                // Inactive: transparent fill, dim stroke.
                ensure_pill(
                    sdi,
                    bg_name,
                    x,
                    tab_y,
                    tw,
                    th,
                    at.bar.tab_inactive_fill,
                    at.bar.tab_inactive_stroke,
                );
            }

            // Tab pill texture slot (B5): shaped tab chrome from the skin's
            // `tab_texture_active` / `tab_texture_inactive` assets. Assigned
            // every frame so tab switches swap textures; a missing state
            // texture clears back to the plain pill fill. Skins without
            // either slot are left alone (a layout.toml `texture =` on the
            // pill object must not be clobbered).
            if (self.tab_texture_active.is_some() || self.tab_texture_inactive.is_some())
                && let Ok(obj) = sdi.get_mut(bg_name)
            {
                obj.texture = if is_active {
                    self.tab_texture_active
                } else {
                    self.tab_texture_inactive
                };
            }

            // Tab text (centered in tab).
            let tx = x + (at.tab_w - text_px(tab.label(), font_small)) / 2;
            let tab_text_y = tab_y + (at.tab_h - font_small as i32) / 2;
            ensure_text(
                sdi,
                name,
                tx.max(x + 2),
                tab_text_y,
                font_small,
                at.bar.tab_text_inactive,
            );
            if let Ok(obj) = sdi.get_mut(name) {
                obj.set_text(tab.label());
                obj.text_color = if is_active {
                    at.bar.tab_text_active
                } else {
                    at.bar.tab_text_inactive
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
        hide_objects(sdi, &GLYPH_SLOTS);
        for i in 0..TopTab::ALL.len() {
            hide_objects(sdi, &[TAB_TEXT_NAMES[i], TAB_BG_NAMES[i]]);
            hide_objects(sdi, &TAB_EDGE_NAMES[i]);
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
        let mut feat = crate::skin::SkinFeatures::default();
        feat.show_tabs = true;
        feat.clock_in_bottombar = false;
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
        let mut feat = crate::skin::SkinFeatures::default();
        feat.clock_in_bottombar = false;
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
        let mut feat = crate::skin::SkinFeatures::default();
        feat.show_tabs = true;
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
        let mut feat = crate::skin::SkinFeatures::default();
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
        let mut feat = crate::skin::SkinFeatures::default();
        feat.clock_in_bottombar = false;
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
        let mut feat = crate::skin::SkinFeatures::default();
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
        let mut feat = crate::skin::SkinFeatures::default();
        feat.show_tabs = true;
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
        let mut feat = crate::skin::SkinFeatures::default();
        feat.show_tabs = true;
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
        let mut feat = crate::skin::SkinFeatures::default();
        feat.show_tabs = true;
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
        let mut feat = crate::skin::SkinFeatures::default();
        feat.clock_in_bottombar = true;
        bar.update_sdi(&mut sdi, &at, &feat);
        // Either not created, or created and hidden.
        if let Ok(obj) = sdi.get("bar_clock") {
            assert!(!obj.visible);
        }
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
        let mut feat = crate::skin::SkinFeatures::default();
        feat.clock_in_bottombar = false;
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
}
