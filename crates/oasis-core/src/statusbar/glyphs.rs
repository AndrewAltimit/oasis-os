//! Vector indicator glyphs (power, Wi-Fi) and their SDI glyph slots.
//!
//! Glyphs are not SDI primitives, so the status bar reserves an invisible
//! *glyph slot* object per indicator (position, size, colour, and the glyph
//! state as its text, which keeps SDI dirty tracking exact) and the shell
//! paints them after the SDI pass with [`render_status_glyphs`].

use oasis_types::backend::SdiBackend;
use oasis_types::error::Result;
use oasis_vector::icons::IconDef;
use oasis_vector::status_glyphs;

use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::platform::{BatteryState, PowerInfo};
use crate::sdi::SdiRegistry;

/// Power indicator state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerGlyph {
    /// Wall power, no battery.
    AcPlug,
    /// Battery with `level` of [`status_glyphs::BATTERY_LEVELS`] cells.
    Battery { level: u8, charging: bool },
}

impl PowerGlyph {
    /// The glyph for a power reading.
    pub(super) fn from_power(p: &PowerInfo) -> Self {
        let level = || status_glyphs::battery_level(p.battery_percent.unwrap_or(0));
        match p.state {
            BatteryState::NoBattery => Self::AcPlug,
            BatteryState::Full => Self::Battery {
                level: status_glyphs::BATTERY_LEVELS,
                charging: false,
            },
            BatteryState::Charging => Self::Battery {
                level: level(),
                charging: true,
            },
            BatteryState::Discharging => Self::Battery {
                level: level(),
                charging: false,
            },
        }
    }
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
    pub(super) fn key(self) -> String {
        match self {
            Self::Power(PowerGlyph::AcPlug) => "ac".to_string(),
            Self::Power(PowerGlyph::Battery { level, charging }) => {
                format!("battery:{level}:{}", u8::from(charging))
            },
            Self::Wifi { connected } => format!("wifi:{}", u8::from(connected)),
        }
    }

    /// Parse a key written by [`Self::key`].
    pub(super) fn from_key(key: &str) -> Option<Self> {
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
pub(super) const GLYPH_SLOT_WIFI: &str = "bar_glyph_wifi";
pub(super) const GLYPH_SLOT_POWER: &str = "bar_glyph_power";
pub(super) const GLYPH_SLOTS: [&str; 2] = [GLYPH_SLOT_WIFI, GLYPH_SLOT_POWER];
/// Horizontal gap (px) after each glyph.
const GLYPH_GAP: i32 = 4;

/// Glyph pixel height for the bar: a touch taller than the small font,
/// never taller than the bar allows.
fn glyph_height(at: &ActiveTheme) -> u32 {
    (at.font_small as u32 + 2)
        .min(at.statusbar_height.saturating_sub(4))
        .max(6)
}

/// Lay out the indicator glyph slots from `x` (Wi-Fi, then power; a `None`
/// state hides its slot); returns the x where the indicator label starts.
pub(super) fn layout_glyph_slots(
    sdi: &mut SdiRegistry,
    at: &ActiveTheme,
    mut x: i32,
    wifi: Option<bool>,
    power: Option<PowerGlyph>,
) -> i32 {
    let gh = glyph_height(at);
    let gy = (at.statusbar_height as i32 - gh as i32) / 2;
    let glyphs = [
        (
            GLYPH_SLOT_WIFI,
            wifi.map(|connected| StatusGlyph::Wifi { connected }),
        ),
        (GLYPH_SLOT_POWER, power.map(StatusGlyph::Power)),
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

/// Paint the status bar's vector indicator glyphs.
///
/// Call after the SDI scene has been drawn (the glyphs sit on the status
/// bar, which is an overlay object). Reads only the glyph slot objects
/// that [`StatusBar::update_sdi`](super::StatusBar::update_sdi) maintains,
/// so hidden bars (terminal mode, fullscreen apps, text indicator style)
/// draw nothing.
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
