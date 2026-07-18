//! Shadow and elevation system.

use crate::backend::{Color, SdiBackend};
use crate::error::Result;

/// A single shadow layer in a multi-layer shadow effect.
#[derive(Debug, Clone, Copy)]
pub struct ShadowLayer {
    /// Horizontal offset from the element in pixels.
    pub offset_x: i32,
    /// Vertical offset from the element in pixels.
    pub offset_y: i32,
    /// Additional size expansion beyond the element bounds in pixels.
    pub spread: u16,
    /// Shadow opacity (0 = invisible, 255 = fully opaque).
    pub alpha: u8,
    /// Shadow tint color (typically black).
    pub color: Color,
}

/// Shadow specification composed of multiple concentric layers.
///
/// Each layer draws a filled rectangle behind the target element with
/// increasing spread and decreasing alpha, producing a soft drop shadow.
#[derive(Debug, Clone)]
pub struct Shadow {
    /// Ordered list of shadow layers, drawn back to front.
    pub layers: Vec<ShadowLayer>,
}

impl Shadow {
    /// No shadow.
    pub fn none() -> Self {
        Self { layers: vec![] }
    }

    /// Draw the shadow behind a rectangle.
    ///
    /// Call BEFORE drawing the panel itself.
    pub fn draw(
        &self,
        backend: &mut dyn SdiBackend,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
    ) -> Result<()> {
        for layer in &self.layers {
            let sx = x + layer.offset_x - layer.spread as i32;
            let sy = y + layer.offset_y - layer.spread as i32;
            let sw = w + layer.spread as u32 * 2;
            let sh = h + layer.spread as u32 * 2;
            let color = Color::rgba(layer.color.r, layer.color.g, layer.color.b, layer.alpha);
            if radius > 0 {
                backend.fill_rounded_rect(sx, sy, sw, sh, radius + layer.spread, color)?;
            } else {
                backend.fill_rect(sx, sy, sw, sh, color)?;
            }
        }
        Ok(())
    }

    /// Override the color of all shadow layers.
    pub fn with_color(mut self, color: Color) -> Self {
        for layer in &mut self.layers {
            layer.color = color;
        }
        self
    }

    #[cfg(test)]
    fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Create a shadow from a predefined elevation level.
    ///
    /// - Level 0: no shadow
    /// - Level 1: subtle (2 layers, small offset)
    /// - Level 2: medium (3 layers, moderate offset)
    /// - Level 3+: prominent (4 layers, large offset)
    pub fn elevation(level: u8) -> Self {
        match level {
            0 => Shadow::none(),
            1 => Shadow {
                layers: vec![
                    ShadowLayer {
                        offset_x: 1,
                        offset_y: 2,
                        spread: 1,
                        alpha: 30,
                        color: Color::BLACK,
                    },
                    ShadowLayer {
                        offset_x: 1,
                        offset_y: 2,
                        spread: 2,
                        alpha: 15,
                        color: Color::BLACK,
                    },
                ],
            },
            2 => Shadow {
                layers: vec![
                    ShadowLayer {
                        offset_x: 2,
                        offset_y: 3,
                        spread: 1,
                        alpha: 40,
                        color: Color::BLACK,
                    },
                    ShadowLayer {
                        offset_x: 2,
                        offset_y: 3,
                        spread: 2,
                        alpha: 25,
                        color: Color::BLACK,
                    },
                    ShadowLayer {
                        offset_x: 2,
                        offset_y: 3,
                        spread: 4,
                        alpha: 12,
                        color: Color::BLACK,
                    },
                ],
            },
            _ => Shadow {
                layers: vec![
                    ShadowLayer {
                        offset_x: 3,
                        offset_y: 5,
                        spread: 1,
                        alpha: 50,
                        color: Color::BLACK,
                    },
                    ShadowLayer {
                        offset_x: 3,
                        offset_y: 5,
                        spread: 3,
                        alpha: 35,
                        color: Color::BLACK,
                    },
                    ShadowLayer {
                        offset_x: 3,
                        offset_y: 5,
                        spread: 5,
                        alpha: 20,
                        color: Color::BLACK,
                    },
                    ShadowLayer {
                        offset_x: 3,
                        offset_y: 5,
                        spread: 8,
                        alpha: 10,
                        color: Color::BLACK,
                    },
                ],
            },
        }
    }
}

/// A semantic elevation ladder mapping levels 0..=5 to concrete shadows.
///
/// Each level may be overridden with an explicit set of [`ShadowLayer`]s.
/// Levels left unset resolve to the built-in [`Shadow::elevation`] ladder,
/// so a [`Default`] ladder reproduces today's shadow appearance exactly.
///
/// Skins expose per-level overrides through the `[elevation]` TOML table;
/// the resulting ladder is the single source of truth for every
/// `*_shadow_level` field once it is threaded through the theme.
#[derive(Debug, Clone, Default)]
pub struct ElevationLadder {
    /// Per-level overrides for levels 0..=5. `None` => built-in default.
    overrides: [Option<Vec<ShadowLayer>>; 6],
}

impl ElevationLadder {
    /// Number of semantic levels in the ladder (0..=5).
    pub const LEVELS: u8 = 6;

    /// Override the layers for a single level (clamped to 0..=5).
    pub fn set_level(&mut self, level: u8, layers: Vec<ShadowLayer>) {
        let idx = level.min(Self::LEVELS - 1) as usize;
        self.overrides[idx] = Some(layers);
    }

    /// Builder variant of [`set_level`](Self::set_level).
    pub fn with_level(mut self, level: u8, layers: Vec<ShadowLayer>) -> Self {
        self.set_level(level, layers);
        self
    }

    /// Returns `true` if no level has been customized (pure default ladder).
    pub fn is_default(&self) -> bool {
        self.overrides.iter().all(Option::is_none)
    }

    /// Resolve a semantic level to a concrete [`Shadow`].
    ///
    /// Falls back to [`Shadow::elevation`] for levels without an override,
    /// so a default ladder is byte-for-byte identical to the built-in one.
    pub fn resolve(&self, level: u8) -> Shadow {
        let idx = level.min(Self::LEVELS - 1) as usize;
        match &self.overrides[idx] {
            Some(layers) => Shadow {
                layers: layers.clone(),
            },
            None => Shadow::elevation(level),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_has_no_layers() {
        let s = Shadow::none();
        assert_eq!(s.layer_count(), 0);
    }

    #[test]
    fn elevation_0_is_none() {
        let s = Shadow::elevation(0);
        assert_eq!(s.layer_count(), 0);
    }

    #[test]
    fn elevation_1_has_two_layers() {
        let s = Shadow::elevation(1);
        assert_eq!(s.layer_count(), 2);
    }

    #[test]
    fn elevation_2_has_three_layers() {
        let s = Shadow::elevation(2);
        assert_eq!(s.layer_count(), 3);
    }

    #[test]
    fn elevation_3_has_four_layers() {
        let s = Shadow::elevation(3);
        assert_eq!(s.layer_count(), 4);
    }

    #[test]
    fn elevation_high_same_as_3() {
        let s = Shadow::elevation(255);
        assert_eq!(s.layer_count(), 4);
    }

    #[test]
    fn with_color_changes_all_layers() {
        let s = Shadow::elevation(2).with_color(Color::rgb(255, 0, 0));
        for layer in &s.layers {
            assert_eq!(layer.color, Color::rgb(255, 0, 0));
        }
    }

    #[test]
    fn higher_elevation_larger_offsets() {
        let s1 = Shadow::elevation(1);
        let s3 = Shadow::elevation(3);
        let max_offset_1 = s1.layers.iter().map(|l| l.offset_y).max().unwrap();
        let max_offset_3 = s3.layers.iter().map(|l| l.offset_y).max().unwrap();
        assert!(max_offset_3 > max_offset_1);
    }

    #[test]
    fn shadow_is_debug() {
        let s = Shadow::elevation(1);
        let _ = format!("{s:?}");
    }

    #[test]
    fn shadow_clone() {
        let s = Shadow::elevation(2);
        let s2 = s.clone();
        assert_eq!(s.layer_count(), s2.layer_count());
    }

    // -- ElevationLadder tests --

    #[test]
    fn default_ladder_reproduces_builtin_elevation() {
        let ladder = ElevationLadder::default();
        assert!(ladder.is_default());
        for level in 0u8..=6 {
            let from_ladder = ladder.resolve(level);
            let builtin = Shadow::elevation(level);
            assert_eq!(from_ladder.layer_count(), builtin.layer_count());
            for (a, b) in from_ladder.layers.iter().zip(builtin.layers.iter()) {
                assert_eq!(a.offset_x, b.offset_x);
                assert_eq!(a.offset_y, b.offset_y);
                assert_eq!(a.spread, b.spread);
                assert_eq!(a.alpha, b.alpha);
                assert_eq!(a.color, b.color);
            }
        }
    }

    #[test]
    fn ladder_override_replaces_level() {
        let custom = vec![ShadowLayer {
            offset_x: 9,
            offset_y: 9,
            spread: 3,
            alpha: 200,
            color: Color::rgb(255, 0, 0),
        }];
        let ladder = ElevationLadder::default().with_level(2, custom);
        assert!(!ladder.is_default());
        let s = ladder.resolve(2);
        assert_eq!(s.layer_count(), 1);
        assert_eq!(s.layers[0].alpha, 200);
        // Non-overridden levels still fall back to the built-in ladder.
        assert_eq!(
            ladder.resolve(1).layer_count(),
            Shadow::elevation(1).layer_count()
        );
    }

    #[test]
    fn ladder_clamps_high_levels() {
        let ladder = ElevationLadder::default().with_level(5, vec![]);
        // Level 200 clamps to level 5 (empty override).
        assert_eq!(ladder.resolve(200).layer_count(), 0);
    }
}
