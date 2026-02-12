//! Avatar widget: circular image with fallback initial.

use crate::backend::{Color, TextureId};
use crate::error::Result;
use crate::ui::context::DrawContext;
use crate::ui::layout;
use crate::ui::widget::Widget;

/// A circular avatar with either an image or a text initial.
pub struct Avatar {
    pub image: Option<TextureId>,
    pub initial: char,
    pub bg_color: Option<Color>,
    pub size: u32,
}

impl Avatar {
    pub fn new(initial: char, size: u32) -> Self {
        Self {
            image: None,
            initial,
            bg_color: None,
            size,
        }
    }

    pub fn with_image(image: TextureId, size: u32) -> Self {
        Self {
            image: Some(image),
            initial: ' ',
            bg_color: None,
            size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let a = Avatar::new('A', 32);
        assert_eq!(a.initial, 'A');
        assert_eq!(a.size, 32);
        assert!(a.image.is_none());
        assert!(a.bg_color.is_none());
    }

    #[test]
    fn with_image_constructor() {
        let a = Avatar::with_image(TextureId(5), 48);
        assert_eq!(a.image, Some(TextureId(5)));
        assert_eq!(a.size, 48);
        assert_eq!(a.initial, ' ');
    }

    #[test]
    fn custom_bg_color() {
        let mut a = Avatar::new('Z', 24);
        a.bg_color = Some(Color::rgb(100, 200, 50));
        assert_eq!(a.bg_color.unwrap(), Color::rgb(100, 200, 50));
    }

    #[test]
    fn unicode_initial() {
        let a = Avatar::new('\u{1F600}', 32);
        assert_eq!(a.initial, '\u{1F600}');
    }

    #[test]
    fn zero_size() {
        let a = Avatar::new('X', 0);
        assert_eq!(a.size, 0);
    }
}

impl Widget for Avatar {
    fn measure(&self, _ctx: &DrawContext<'_>, _available_w: u32, _available_h: u32) -> (u32, u32) {
        (self.size, self.size)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, _w: u32, _h: u32) -> Result<()> {
        let r = self.size as u16 / 2;
        let cx = x + r as i32;
        let cy = y + r as i32;

        if self.image.is_some() {
            // Draw circular clip with image.
            let bg = self.bg_color.unwrap_or(ctx.theme.surface_variant);
            ctx.backend.fill_circle(cx, cy, r, bg)?;
            // Blit image into the bounding rect (no circular clip in software).
            if let Some(tex) = self.image {
                ctx.backend.blit(tex, x, y, self.size, self.size)?;
            }
        } else {
            // Fallback: colored circle with initial.
            let bg = self.bg_color.unwrap_or(ctx.theme.accent);
            ctx.backend.fill_circle(cx, cy, r, bg)?;

            let text = self.initial.to_uppercase().to_string();
            let fs = ctx.theme.font_size_md;
            let tw = ctx.backend.measure_text(&text, fs);
            let th = ctx.backend.measure_text_height(fs);
            let tx = x + layout::center(self.size, tw);
            let ty = y + layout::center(self.size, th);
            ctx.backend
                .draw_text(&text, tx, ty, fs, ctx.theme.text_on_accent)?;
        }
        Ok(())
    }
}
