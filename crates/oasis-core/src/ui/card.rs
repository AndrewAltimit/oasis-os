//! Card widget: content card with optional image, title, subtitle, body.

use crate::backend::TextureId;
use crate::error::Result;
use crate::ui::context::DrawContext;
use crate::ui::shadow::Shadow;
use crate::ui::widget::Widget;

/// A content card combining image, title, subtitle, and body text.
pub struct Card {
    pub image: Option<TextureId>,
    pub image_height: u32,
    pub title: String,
    pub subtitle: Option<String>,
    pub body: Option<String>,
    pub elevation: u8,
    pub radius: u16,
}

impl Card {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            image: None,
            image_height: 0,
            title: title.into(),
            subtitle: None,
            body: None,
            elevation: 1,
            radius: 0,
        }
    }

    pub fn themed(title: impl Into<String>, ctx: &DrawContext<'_>) -> Self {
        Self {
            radius: ctx.theme.border_radius_lg,
            ..Self::new(title)
        }
    }
}

impl Widget for Card {
    fn measure(&self, ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        let padding = ctx.theme.spacing_md as u32;
        let mut h = padding;

        if self.image.is_some() {
            h += self.image_height;
        }

        let fs_title = ctx.theme.font_size_lg;
        h += ctx.backend.measure_text_height(fs_title) + padding;

        if self.subtitle.is_some() {
            let fs = ctx.theme.font_size_sm;
            h += ctx.backend.measure_text_height(fs) + 2;
        }

        if self.body.is_some() {
            let fs = ctx.theme.font_size_md;
            h += ctx.backend.measure_text_height(fs) * 2 + padding;
        }

        (available_w, h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let padding = ctx.theme.spacing_md as i32;

        // Shadow + background.
        if self.elevation > 0 {
            let shadow = Shadow::elevation(self.elevation);
            shadow.draw(ctx.backend, x, y, w, h, self.radius)?;
        }
        ctx.backend
            .fill_rounded_rect(x, y, w, h, self.radius, ctx.theme.surface)?;

        let mut cy = y;

        // Image.
        if let Some(tex) = self.image {
            ctx.backend.blit(tex, x, cy, w, self.image_height)?;
            cy += self.image_height as i32;
        }

        cy += padding;

        // Title.
        let fs_title = ctx.theme.font_size_lg;
        let content_w = w.saturating_sub(padding as u32 * 2);
        ctx.backend.draw_text_ellipsis(
            &self.title,
            x + padding,
            cy,
            fs_title,
            ctx.theme.text_primary,
            content_w,
        )?;
        cy += ctx.backend.measure_text_height(fs_title) as i32 + 2;

        // Subtitle.
        if let Some(sub) = &self.subtitle {
            let fs = ctx.theme.font_size_sm;
            ctx.backend.draw_text_ellipsis(
                sub,
                x + padding,
                cy,
                fs,
                ctx.theme.text_secondary,
                content_w,
            )?;
            cy += ctx.backend.measure_text_height(fs) as i32 + 4;
        }

        // Body.
        if let Some(body) = &self.body {
            let fs = ctx.theme.font_size_md;
            ctx.backend.draw_text_wrapped(
                body,
                x + padding,
                cy,
                fs,
                ctx.theme.text_secondary,
                content_w,
                0,
            )?;
        }

        Ok(())
    }
}
