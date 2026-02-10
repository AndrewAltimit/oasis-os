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
