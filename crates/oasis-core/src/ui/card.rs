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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let c = Card::new("Hello");
        assert_eq!(c.title, "Hello");
        assert!(c.image.is_none());
        assert_eq!(c.image_height, 0);
        assert!(c.subtitle.is_none());
        assert!(c.body.is_none());
        assert_eq!(c.elevation, 1);
        assert_eq!(c.radius, 0);
    }

    #[test]
    fn new_from_string() {
        let c = Card::new(String::from("World"));
        assert_eq!(c.title, "World");
    }

    #[test]
    fn with_subtitle() {
        let mut c = Card::new("Title");
        c.subtitle = Some("Sub".into());
        assert_eq!(c.subtitle.as_deref(), Some("Sub"));
    }

    #[test]
    fn with_body() {
        let mut c = Card::new("Title");
        c.body = Some("Body text here".into());
        assert_eq!(c.body.as_deref(), Some("Body text here"));
    }

    #[test]
    fn with_image() {
        let mut c = Card::new("Title");
        c.image = Some(TextureId(42));
        c.image_height = 100;
        assert_eq!(c.image, Some(TextureId(42)));
        assert_eq!(c.image_height, 100);
    }

    #[test]
    fn custom_elevation() {
        let mut c = Card::new("Title");
        c.elevation = 3;
        assert_eq!(c.elevation, 3);
    }

    #[test]
    fn zero_elevation_no_shadow() {
        let mut c = Card::new("Title");
        c.elevation = 0;
        assert_eq!(c.elevation, 0);
    }

    // -- Draw / measure tests using MockBackend --

    use crate::browser::test_utils::MockBackend;
    use crate::ui::context::DrawContext;
    use crate::ui::theme::Theme;
    use crate::ui::widget::Widget;

    #[test]
    fn measure_accounts_for_padding() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let card = Card::new("Title");
        let (w, h) = card.measure(&ctx, 200, 100);
        assert!(w > 0);
        assert!(h > 0);
    }

    #[test]
    fn draw_emits_shadow_at_elevation() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut card = Card::new("Title");
            card.elevation = 2;
            card.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        // Shadow + background + text → more than 1 fill_rect.
        assert!(backend.fill_rect_count() > 1);
    }

    #[test]
    fn draw_title_text() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let card = Card::new("Title");
            card.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert!(backend.has_text("Title"));
    }

    #[test]
    fn draw_subtitle() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut card = Card::new("Title");
            card.subtitle = Some("Subtitle text".into());
            card.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert!(backend.has_text("Subtitle text"));
    }

    #[test]
    fn draw_body() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut card = Card::new("Title");
            card.body = Some("Body content here".into());
            card.draw(&mut ctx, 0, 0, 200, 150).unwrap();
        }
        assert!(backend.has_text("Body content here"));
    }

    #[test]
    fn draw_no_image_full_width() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let card = Card::new("NoImage");
            card.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_empty_title() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let card = Card::new("");
            card.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        // Should not panic; background fill still emitted.
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_zero_elevation_no_shadow_rects() {
        let theme = Theme::dark();
        let mut backend_with_shadow = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend_with_shadow, &theme);
            let mut card = Card::new("Title");
            card.elevation = 2;
            card.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        let with_shadow = backend_with_shadow.fill_rect_count();

        let mut backend_no_shadow = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend_no_shadow, &theme);
            let mut card = Card::new("Title");
            card.elevation = 0;
            card.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        let without_shadow = backend_no_shadow.fill_rect_count();

        assert!(without_shadow < with_shadow);
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
