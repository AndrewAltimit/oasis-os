//! Shadow and elevation system.

use crate::backend::{Color, SdiBackend};
use crate::error::Result;

/// A single shadow layer.
#[derive(Debug, Clone, Copy)]
pub struct ShadowLayer {
    pub offset_x: i32,
    pub offset_y: i32,
    pub spread: u16,
    pub alpha: u8,
    pub color: Color,
}

/// Shadow specification composed of multiple layers.
#[derive(Debug, Clone)]
pub struct Shadow {
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

    /// Predefined elevation levels.
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
}
