//! Mock and recording backends for testing OASIS_OS rendering traits.
//!
//! Provides [`MockSdiCore`] for unit testing UI widgets, browser rendering,
//! and other code that depends on [`SdiCore`](oasis_types::backend::SdiCore)/[`SdiBackend`](oasis_types::backend::SdiBackend) without a real
//! graphics backend.
//!
//! Also provides [`RecordingBackend`] which extends `MockSdiCore` with full
//! [`DrawCommand`] history recording and clip/translate stack support, useful
//! for verifying that `SdiBackend` default method implementations produce the
//! expected sequence of primitive draw calls.

mod mock;
mod recording;

pub use mock::MockSdiCore;
pub use recording::RecordingBackend;

// Re-export commonly used types so test code does not need to depend on
// oasis-types directly for basic assertions.
pub use oasis_types::backend::{Color, DrawCommand, GradientStyle, TextureId};

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_types::backend::{
        SdiAlpha, SdiBatch, SdiClipTransform, SdiCore, SdiGradients, SdiShapes,
    };

    // -----------------------------------------------------------------------
    // MockSdiCore basic tests
    // -----------------------------------------------------------------------

    #[test]
    fn mock_records_calls() {
        let mut mock = MockSdiCore::new(480, 272);
        let color = Color::rgb(255, 0, 0);
        mock.fill_rect(10, 20, 100, 50, color).ok();
        mock.draw_text("hello", 5, 5, 12, Color::WHITE).ok();

        let calls = mock.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "fill_rect");
        assert_eq!(calls[1].0, "draw_text");
    }

    #[test]
    fn mock_configurable_measure_text() {
        let mut mock = MockSdiCore::new(480, 272);
        assert!(mock.measure_text("hello", 12) > 0);

        mock.set_measure_text_fn(|_text, _size| 42);
        assert_eq!(mock.measure_text("anything", 16), 42);
    }

    #[test]
    fn mock_texture_load_destroy() {
        let mut mock = MockSdiCore::new(480, 272);
        let rgba = vec![0u8; 4 * 4 * 4]; // 4x4 RGBA
        let tex = mock.load_texture(4, 4, &rgba).expect("load_texture");
        assert_eq!(tex, TextureId(1));

        let tex2 = mock.load_texture(4, 4, &rgba).expect("load_texture");
        assert_eq!(tex2, TextureId(2));

        mock.destroy_texture(tex).expect("destroy_texture");
    }

    #[test]
    fn mock_init_and_shutdown() {
        let mut mock = MockSdiCore::new(480, 272);
        mock.init(480, 272).expect("init");
        mock.shutdown().expect("shutdown");

        let calls = mock.calls();
        assert_eq!(calls[0].0, "init");
        assert_eq!(calls[1].0, "shutdown");
    }

    #[test]
    fn mock_sdi_backend_defaults() {
        let mut mock = MockSdiCore::new(480, 272);
        // SdiBackend default methods should work via delegation to SdiCore.
        mock.fill_rounded_rect(10, 10, 50, 50, 5, Color::WHITE)
            .expect("fill_rounded_rect");
        // The default falls back to fill_rect.
        let calls = mock.calls();
        assert!(calls.iter().any(|(name, _)| name == "fill_rect"));
    }

    #[test]
    fn mock_read_pixels_returns_zeroed() {
        let mock = MockSdiCore::new(480, 272);
        let pixels = mock.read_pixels(0, 0, 2, 2).expect("read_pixels");
        assert_eq!(pixels.len(), 2 * 2 * 4);
        assert!(pixels.iter().all(|&b| b == 0));
    }

    #[test]
    fn mock_clear_calls() {
        let mut mock = MockSdiCore::new(480, 272);
        mock.fill_rect(0, 0, 10, 10, Color::BLACK).ok();
        assert_eq!(mock.calls().len(), 1);
        mock.clear_calls();
        assert!(mock.calls().is_empty());
    }

    // -----------------------------------------------------------------------
    // RecordingBackend tests
    // -----------------------------------------------------------------------

    #[test]
    fn recording_fill_rounded_rect_falls_back_to_fill_rect() {
        let mut rec = RecordingBackend::new(480, 272);
        rec.fill_rounded_rect(10, 20, 100, 50, 8, Color::rgb(0, 128, 255))
            .expect("fill_rounded_rect");

        let cmds = rec.commands();
        // The default implementation falls back to a single fill_rect.
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            DrawCommand::FillRect {
                x, y, w, h, color, ..
            } => {
                assert_eq!(*x, 10);
                assert_eq!(*y, 20);
                assert_eq!(*w, 100);
                assert_eq!(*h, 50);
                assert_eq!(*color, Color::rgb(0, 128, 255));
            },
            other => panic!("expected FillRect, got {other:?}"),
        }
    }

    #[test]
    fn recording_draw_line_horizontal() {
        let mut rec = RecordingBackend::new(480, 272);
        let color = Color::rgb(255, 0, 0);
        rec.draw_line(10, 50, 100, 50, 2, color).expect("draw_line");

        let cmds = rec.commands();
        // Horizontal line -> single fill_rect.
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            DrawCommand::FillRect { x, y, w, h, .. } => {
                assert_eq!(*x, 10);
                assert_eq!(*y, 50);
                assert_eq!(*w, 90); // |100 - 10| = 90
                assert_eq!(*h, 2); // line width
            },
            other => panic!("expected FillRect, got {other:?}"),
        }
    }

    #[test]
    fn recording_draw_line_vertical() {
        let mut rec = RecordingBackend::new(480, 272);
        let color = Color::rgb(0, 255, 0);
        rec.draw_line(50, 10, 50, 100, 3, color).expect("draw_line");

        let cmds = rec.commands();
        // Vertical line -> single fill_rect.
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            DrawCommand::FillRect { x, y, w, h, .. } => {
                assert_eq!(*x, 50);
                assert_eq!(*y, 10);
                assert_eq!(*w, 3); // line width
                assert_eq!(*h, 90); // |100 - 10| = 90
            },
            other => panic!("expected FillRect, got {other:?}"),
        }
    }

    #[test]
    fn recording_draw_line_diagonal_produces_fill_rects() {
        let mut rec = RecordingBackend::new(480, 272);
        let color = Color::WHITE;
        rec.draw_line(0, 0, 3, 3, 1, color).expect("draw_line");

        let cmds = rec.commands();
        // Diagonal via Bresenham produces pixel-sized fill_rects.
        assert!(cmds.len() >= 4); // at least 4 points for (0,0)->(3,3)
        for cmd in cmds {
            assert!(
                matches!(cmd, DrawCommand::FillRect { .. }),
                "expected FillRect, got {cmd:?}"
            );
        }
    }

    #[test]
    fn recording_stroke_rect_produces_four_fill_rects() {
        let mut rec = RecordingBackend::new(480, 272);
        rec.stroke_rect(10, 10, 100, 80, 2, Color::BLACK)
            .expect("stroke_rect");

        let cmds = rec.commands();
        // stroke_rect default draws 4 fill_rects (top, bottom, left, right).
        assert_eq!(cmds.len(), 4);
        for cmd in cmds {
            assert!(
                matches!(cmd, DrawCommand::FillRect { .. }),
                "expected FillRect, got {cmd:?}"
            );
        }
    }

    #[test]
    fn recording_clip_stack_push_pop() {
        let mut rec = RecordingBackend::new(480, 272);

        rec.push_clip_rect(10, 10, 200, 200)
            .expect("push_clip_rect");
        assert_eq!(rec.current_clip_rect(), Some((10, 10, 200, 200)));

        rec.push_clip_rect(50, 50, 100, 100)
            .expect("push_clip_rect");
        // Intersection of (10,10,200,200) and (50,50,100,100)
        // = (50,50,100,100) since it fits inside the first.
        assert_eq!(rec.current_clip_rect(), Some((50, 50, 100, 100)));

        rec.pop_clip_rect().expect("pop_clip_rect");
        assert_eq!(rec.current_clip_rect(), Some((10, 10, 200, 200)));

        rec.pop_clip_rect().expect("pop_clip_rect");
        assert_eq!(rec.current_clip_rect(), None);

        let cmds = rec.commands();
        assert_eq!(cmds.len(), 4);
        assert!(matches!(cmds[0], DrawCommand::PushClip { .. }));
        assert!(matches!(cmds[1], DrawCommand::PushClip { .. }));
        assert!(matches!(cmds[2], DrawCommand::PopClip));
        assert!(matches!(cmds[3], DrawCommand::PopClip));
    }

    #[test]
    fn recording_translate_stack_push_pop() {
        let mut rec = RecordingBackend::new(480, 272);

        rec.push_translate(10, 20).expect("push_translate");
        assert_eq!(rec.current_translate(), (10, 20));

        rec.push_translate(5, 5).expect("push_translate");
        assert_eq!(rec.current_translate(), (15, 25));

        rec.pop_translate().expect("pop_translate");
        assert_eq!(rec.current_translate(), (10, 20));

        rec.pop_translate().expect("pop_translate");
        assert_eq!(rec.current_translate(), (0, 0));

        let cmds = rec.commands();
        assert_eq!(cmds.len(), 4);
        assert!(matches!(
            cmds[0],
            DrawCommand::PushTranslate { dx: 10, dy: 20 }
        ));
        assert!(matches!(
            cmds[1],
            DrawCommand::PushTranslate { dx: 5, dy: 5 }
        ));
        assert!(matches!(cmds[2], DrawCommand::PopTranslate));
        assert!(matches!(cmds[3], DrawCommand::PopTranslate));
    }

    #[test]
    fn recording_clear_commands() {
        let mut rec = RecordingBackend::new(480, 272);
        rec.fill_rect(0, 0, 10, 10, Color::BLACK).ok();
        assert_eq!(rec.commands().len(), 1);
        rec.clear_commands();
        assert!(rec.commands().is_empty());
    }

    #[test]
    fn recording_viewport_size() {
        let rec = RecordingBackend::new(800, 600);
        assert_eq!(rec.viewport_size(), (800, 600));
    }

    #[test]
    fn recording_dim_screen() {
        let mut rec = RecordingBackend::new(480, 272);
        rec.dim_screen(128).expect("dim_screen");

        let cmds = rec.commands();
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            DrawCommand::FillRect {
                x, y, w, h, color, ..
            } => {
                assert_eq!(*x, 0);
                assert_eq!(*y, 0);
                assert_eq!(*w, 480);
                assert_eq!(*h, 272);
                assert_eq!(color.a, 128);
            },
            other => panic!("expected FillRect, got {other:?}"),
        }
    }

    #[test]
    fn recording_fill_circle_produces_fill_rects() {
        let mut rec = RecordingBackend::new(480, 272);
        rec.fill_circle(100, 100, 10, Color::WHITE)
            .expect("fill_circle");

        // Midpoint circle algorithm produces multiple horizontal spans.
        let cmds = rec.commands();
        assert!(!cmds.is_empty());
        for cmd in cmds {
            assert!(
                matches!(cmd, DrawCommand::FillRect { .. }),
                "expected FillRect, got {cmd:?}"
            );
        }
    }

    #[test]
    fn recording_draw_text_recorded() {
        let mut rec = RecordingBackend::new(480, 272);
        rec.draw_text("hello", 10, 20, 14, Color::WHITE)
            .expect("draw_text");

        let cmds = rec.commands();
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            DrawCommand::DrawText {
                text,
                x,
                y,
                font_size,
                ..
            } => {
                assert_eq!(text, "hello");
                assert_eq!(*x, 10);
                assert_eq!(*y, 20);
                assert_eq!(*font_size, 14);
            },
            other => panic!("expected DrawText, got {other:?}"),
        }
    }

    #[test]
    fn recording_blit_recorded() {
        let mut rec = RecordingBackend::new(480, 272);
        let rgba = vec![0u8; 4 * 4 * 4];
        let tex = rec.load_texture(4, 4, &rgba).expect("load_texture");
        rec.blit(tex, 10, 20, 4, 4).expect("blit");

        let cmds = rec.commands();
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            DrawCommand::Blit { tex: t, x, y, .. } => {
                assert_eq!(*t, tex);
                assert_eq!(*x, 10);
                assert_eq!(*y, 20);
            },
            other => panic!("expected Blit, got {other:?}"),
        }
    }

    #[test]
    fn recording_begin_flush_batch() {
        let mut rec = RecordingBackend::new(480, 272);
        rec.begin_batch().expect("begin_batch");
        rec.fill_rect(0, 0, 10, 10, Color::BLACK).ok();
        rec.flush_batch().expect("flush_batch");
        // begin_batch and flush_batch are no-ops in the default impl,
        // so only the fill_rect is recorded.
        assert_eq!(rec.commands().len(), 1);
    }

    #[test]
    fn recording_fill_rect_gradient() {
        let mut rec = RecordingBackend::new(480, 272);
        let gradient = GradientStyle::Vertical {
            top: Color::WHITE,
            bottom: Color::BLACK,
        };
        rec.fill_rect_gradient(10, 20, 100, 50, &gradient)
            .expect("fill_rect_gradient");

        // Default gradient falls back to fill_rect with primary color.
        let cmds = rec.commands();
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            DrawCommand::FillRect { color, .. } => {
                assert_eq!(*color, Color::WHITE);
            },
            other => panic!("expected FillRect, got {other:?}"),
        }
    }

    #[test]
    fn recording_push_pop_region() {
        let mut rec = RecordingBackend::new(480, 272);
        rec.push_region(10, 20, 200, 100).expect("push_region");
        assert_eq!(rec.current_translate(), (10, 20));
        assert!(rec.current_clip_rect().is_some());

        rec.pop_region().expect("pop_region");
        assert_eq!(rec.current_translate(), (0, 0));
        assert_eq!(rec.current_clip_rect(), None);
    }

    #[test]
    fn recording_nested_clip_intersection() {
        let mut rec = RecordingBackend::new(480, 272);
        rec.push_clip_rect(0, 0, 100, 100).expect("push clip 1");
        rec.push_clip_rect(50, 50, 100, 100).expect("push clip 2");

        // Intersection: (50,50) to min(100,150) x min(100,150) = (50,50,50,50)
        assert_eq!(rec.current_clip_rect(), Some((50, 50, 50, 50)));
    }
}
