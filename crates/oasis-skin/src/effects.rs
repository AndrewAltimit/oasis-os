//! Pluggable visual effect system for skins.
//!
//! Effects modify the SDI scene graph each frame to create visual aesthetics
//! like corruption, scanlines, or CRT warping. Each effect implements the
//! `SkinEffect` trait and is applied in sequence during rendering.

use std::fmt::Debug;

use oasis_sdi::SdiRegistry;

use crate::corrupted::{CorruptedModifiers, SimpleRng};

/// A pluggable visual effect that modifies the SDI scene each frame.
pub trait SkinEffect: Debug {
    /// Human-readable name of this effect.
    fn name(&self) -> &str;

    /// Current intensity (0.0 = off, 1.0 = full).
    fn intensity(&self) -> f32;

    /// Set the intensity (clamped to 0.0..=1.0).
    fn set_intensity(&mut self, intensity: f32);

    /// Apply the effect to the SDI scene for one frame.
    fn apply(&mut self, sdi: &mut SdiRegistry);
}

// ---------------------------------------------------------------------------
// CorruptedEffect: adapter wrapping CorruptedModifiers
// ---------------------------------------------------------------------------

/// Adapter wrapping `CorruptedModifiers` as a `SkinEffect`.
#[derive(Debug, Clone)]
pub struct CorruptedEffect {
    modifiers: CorruptedModifiers,
    rng: SimpleRng,
}

impl CorruptedEffect {
    pub fn new(modifiers: CorruptedModifiers) -> Self {
        Self {
            modifiers,
            rng: SimpleRng::new(42),
        }
    }
}

impl SkinEffect for CorruptedEffect {
    fn name(&self) -> &str {
        "corrupted"
    }

    fn intensity(&self) -> f32 {
        self.modifiers.intensity
    }

    fn set_intensity(&mut self, intensity: f32) {
        self.modifiers.intensity = intensity.clamp(0.0, 1.0);
    }

    fn apply(&mut self, sdi: &mut SdiRegistry) {
        self.modifiers.apply(sdi, &mut self.rng);
    }
}

// ---------------------------------------------------------------------------
// ScanlineEffect: proof-of-concept effect
// ---------------------------------------------------------------------------

/// A simple scanline overlay effect that dims alternating rows
/// by toggling the alpha of a set of horizontal bar objects.
#[derive(Debug, Clone)]
pub struct ScanlineEffect {
    intensity: f32,
    /// Number of scanline overlay objects created.
    line_count: u32,
    /// Whether the scanline objects have been created.
    initialized: bool,
}

impl ScanlineEffect {
    pub fn new(intensity: f32) -> Self {
        Self {
            intensity: intensity.clamp(0.0, 1.0),
            line_count: 0,
            initialized: false,
        }
    }

    const PREFIX: &str = "_fx_scanline_";
}

impl SkinEffect for ScanlineEffect {
    fn name(&self) -> &str {
        "scanlines"
    }

    fn intensity(&self) -> f32 {
        self.intensity
    }

    fn set_intensity(&mut self, intensity: f32) {
        self.intensity = intensity.clamp(0.0, 1.0);
    }

    fn apply(&mut self, sdi: &mut SdiRegistry) {
        if self.intensity <= 0.0 {
            // Hide all scanline objects if intensity is zero.
            for i in 0..self.line_count {
                let name = format!("{}{i}", Self::PREFIX);
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
            }
            return;
        }

        // Create scanline objects on first use (every 2 pixels).
        if !self.initialized {
            let screen_h = 272u32; // PSP native height
            let screen_w = 480u32;
            let mut count = 0u32;
            for y in (0..screen_h).step_by(2) {
                let name = format!("{}{count}", Self::PREFIX);
                if !sdi.contains(&name) {
                    let obj = sdi.create(&name);
                    obj.x = 0;
                    obj.y = y as i32;
                    obj.w = screen_w;
                    obj.h = 1;
                    obj.color = oasis_types::backend::Color::rgba(0, 0, 0, 0);
                    obj.overlay = true;
                    obj.z = 950;
                }
                count += 1;
            }
            self.line_count = count;
            self.initialized = true;
        }

        let alpha = (self.intensity * 40.0) as u8;
        for i in 0..self.line_count {
            let name = format!("{}{i}", Self::PREFIX);
            if let Ok(obj) = sdi.get_mut(&name) {
                obj.visible = true;
                obj.color = oasis_types::backend::Color::rgba(0, 0, 0, alpha);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupted_effect_adapter() {
        let mods = CorruptedModifiers::default();
        let mut effect = CorruptedEffect::new(mods);
        assert_eq!(effect.name(), "corrupted");
        assert!((effect.intensity() - 1.0).abs() < f32::EPSILON);
        effect.set_intensity(0.5);
        assert!((effect.intensity() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn corrupted_effect_apply() {
        let mods = CorruptedModifiers {
            position_jitter: 10,
            intensity: 1.0,
            alpha_flicker_chance: 0.0,
            text_garble_chance: 0.0,
            ..Default::default()
        };
        let mut effect = CorruptedEffect::new(mods);
        let mut sdi = SdiRegistry::new();
        {
            let obj = sdi.create("test");
            obj.x = 100;
            obj.y = 200;
        }
        effect.apply(&mut sdi);
        let obj = sdi.get("test").unwrap();
        assert!(obj.x != 100 || obj.y != 200);
    }

    #[test]
    fn scanline_effect_creates_objects() {
        let mut effect = ScanlineEffect::new(0.5);
        assert_eq!(effect.name(), "scanlines");
        let mut sdi = SdiRegistry::new();
        effect.apply(&mut sdi);
        assert!(effect.initialized);
        assert!(effect.line_count > 0);
        assert!(sdi.contains(&format!("{}0", ScanlineEffect::PREFIX)));
    }

    #[test]
    fn scanline_zero_intensity_hides() {
        let mut effect = ScanlineEffect::new(0.5);
        let mut sdi = SdiRegistry::new();
        effect.apply(&mut sdi);
        assert!(
            sdi.get(&format!("{}0", ScanlineEffect::PREFIX))
                .unwrap()
                .visible
        );

        effect.set_intensity(0.0);
        effect.apply(&mut sdi);
        assert!(
            !sdi.get(&format!("{}0", ScanlineEffect::PREFIX))
                .unwrap()
                .visible
        );
    }
}
