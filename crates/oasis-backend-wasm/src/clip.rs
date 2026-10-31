//! Clip and translate state of one Canvas 2D surface, kept in step with
//! the context's `save()` / `restore()` stack.
//!
//! Canvas 2D can only *intersect* clips (`clip()`), never replace or remove
//! one, except by `restore()`-ing to a state saved before it. So an applied
//! clip is exactly one `save()` level on the context, entered by
//! [`SurfaceClip::apply`]; every clip change (`set_clip_rect`,
//! `reset_clip_rect`, `push_clip_rect`, `pop_clip_rect`, `clear`) restores
//! that level and, if a clip remains, saves and clips once again. Nothing
//! else may leave a `save()` unbalanced.
//!
//! The logic only needs three context calls, so it is written against
//! [`ClipCanvas`] and unit-tested on the host against a model of Canvas 2D
//! clip semantics (see the tests): the WASM backend has no headless-browser
//! test job, and this state machine is where its rendering bugs used to be
//! (a nested browser clip never widening again, `reset_clip_rect` popping
//! a `push_clip_rect` level).

use oasis_types::backend::stacks::{ClipStack, TranslateStack};
use oasis_types::geometry::ClipRect;

/// The part of a Canvas 2D context the clip logic drives.
pub(crate) trait ClipCanvas {
    /// `ctx.save()`.
    fn save(&self);
    /// `ctx.restore()`.
    fn restore(&self);
    /// `ctx.beginPath(); ctx.rect(..); ctx.clip()`: intersect the current
    /// clip with `c`.
    fn clip_to(&self, c: ClipRect);
}

impl ClipCanvas for web_sys::CanvasRenderingContext2d {
    fn save(&self) {
        web_sys::CanvasRenderingContext2d::save(self);
    }

    fn restore(&self) {
        web_sys::CanvasRenderingContext2d::restore(self);
    }

    fn clip_to(&self, c: ClipRect) {
        self.begin_path();
        self.rect(
            f64::from(c.x),
            f64::from(c.y),
            f64::from(c.w),
            f64::from(c.h),
        );
        self.clip();
    }
}

/// Clip + translate stacks of one surface and the clip applied on its
/// context.
#[derive(Debug, Default)]
pub(crate) struct SurfaceClip {
    /// `push_clip_rect` / `pop_clip_rect` stack (surface pixels).
    pub(crate) stack: ClipStack,
    /// `push_translate` / `pop_translate` stack.
    pub(crate) translate: TranslateStack,
    /// The clip currently applied on the context (one `save()` level), or
    /// `None`.
    applied: Option<ClipRect>,
}

impl SurfaceClip {
    /// The clip currently applied on the context.
    #[cfg(test)]
    pub(crate) fn applied(&self) -> Option<ClipRect> {
        self.applied
    }

    /// Make `clip` (surface pixels, `None` = no clip) the one clip applied
    /// on `ctx`, replacing the previous one.
    pub(crate) fn apply(&mut self, ctx: &impl ClipCanvas, clip: Option<ClipRect>) {
        if self.applied.take().is_some() {
            ctx.restore();
        }
        if let Some(c) = clip {
            ctx.save();
            ctx.clip_to(c);
            self.applied = Some(c);
        }
    }

    /// `set_clip_rect`: absolute (untranslated) and replacing.
    pub(crate) fn set(&mut self, ctx: &impl ClipCanvas, c: ClipRect) {
        self.apply(ctx, Some(c));
    }

    /// `reset_clip_rect`.
    pub(crate) fn reset(&mut self, ctx: &impl ClipCanvas) {
        self.apply(ctx, None);
    }

    /// `push_clip_rect`: translated, intersected with the current stack
    /// clip.
    pub(crate) fn push(&mut self, ctx: &impl ClipCanvas, x: i32, y: i32, w: u32, h: u32) {
        let applied = self.stack.push_translated(&self.translate, x, y, w, h);
        self.apply(ctx, Some(applied));
    }

    /// `pop_clip_rect`: back to the clip before the matching push.
    pub(crate) fn pop(&mut self, ctx: &impl ClipCanvas) {
        let prev = self.stack.pop();
        self.apply(ctx, prev);
    }

    /// Run `f` with the clip lifted (`clear` is clip-independent), then
    /// put it back.
    pub(crate) fn unclipped<R>(&mut self, ctx: &impl ClipCanvas, f: impl FnOnce() -> R) -> R {
        let clip = self.applied;
        self.apply(ctx, None);
        let r = f();
        self.apply(ctx, clip);
        r
    }

    /// The context's state was reset behind our back (resizing a canvas
    /// resets its whole 2D state, save stack included): start over.
    pub(crate) fn forget(&mut self) {
        *self = Self::default();
    }

    /// Lift the clip off `ctx` and empty both stacks (`shutdown`).
    pub(crate) fn clear(&mut self, ctx: &impl ClipCanvas) {
        self.apply(ctx, None);
        self.stack.clear();
        self.translate.clear();
    }

    /// `bind_render_target`: move this surface's state out, leaving a fresh
    /// one (unclipped, untranslated) for the target. The parent's context
    /// keeps its clip save level while the target is bound; the target's
    /// context has none (see [`SurfaceClip::unbind`]).
    pub(crate) fn bind(&mut self) -> SurfaceClip {
        std::mem::take(self)
    }

    /// `unbind_render_target`: strip the target's clip level off
    /// `target_ctx` (so its next bind starts unclipped), then restore the
    /// parent state `saved` returned by [`SurfaceClip::bind`].
    pub(crate) fn unbind(&mut self, target_ctx: &impl ClipCanvas, saved: SurfaceClip) {
        self.apply(target_ctx, None);
        *self = saved;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// Canvas 2D clip semantics: the drawing state holds a list of clip
    /// regions (all intersected), `save` pushes a copy, `restore` pops one
    /// (a no-op on an empty stack, like the real API).
    #[derive(Default)]
    struct ModelCanvas {
        clips: RefCell<Vec<ClipRect>>,
        saved: RefCell<Vec<Vec<ClipRect>>>,
        /// `restore()` calls with nothing saved: a desync bug.
        stray_restores: RefCell<u32>,
    }

    impl ClipCanvas for ModelCanvas {
        fn save(&self) {
            let copy = self.clips.borrow().clone();
            self.saved.borrow_mut().push(copy);
        }

        fn restore(&self) {
            match self.saved.borrow_mut().pop() {
                Some(c) => *self.clips.borrow_mut() = c,
                None => *self.stray_restores.borrow_mut() += 1,
            }
        }

        fn clip_to(&self, c: ClipRect) {
            self.clips.borrow_mut().push(c);
        }
    }

    impl ModelCanvas {
        fn depth(&self) -> usize {
            self.saved.borrow().len()
        }

        /// The effective clip: the intersection of every clip region
        /// (`None` = unclipped; an empty intersection is `ClipRect::EMPTY`).
        fn effective(&self) -> Option<ClipRect> {
            let clips = self.clips.borrow();
            let mut it = clips.iter();
            let first = *it.next()?;
            Some(it.fold(first, |acc, c| acc.intersect(c).unwrap_or(ClipRect::EMPTY)))
        }

        fn area(c: Option<ClipRect>) -> Option<u64> {
            c.map(|c| u64::from(c.w) * u64::from(c.h))
        }
    }

    /// The invariant: the context holds exactly the applied clip, as one
    /// save level, and never restored past its base.
    fn check(s: &SurfaceClip, ctx: &ModelCanvas, step: &str) {
        assert_eq!(
            *ctx.stray_restores.borrow(),
            0,
            "{step}: unbalanced restore"
        );
        assert_eq!(
            ctx.depth(),
            usize::from(s.applied().is_some()),
            "{step}: depth"
        );
        let eff = ctx.effective();
        if s.applied().is_some_and(|c| c.w == 0 || c.h == 0) {
            assert_eq!(ModelCanvas::area(eff), Some(0), "{step}: empty clip");
        } else {
            assert_eq!(eff, s.applied(), "{step}: effective clip");
        }
    }

    fn r(x: i32, y: i32, w: u32, h: u32) -> ClipRect {
        ClipRect { x, y, w, h }
    }

    #[test]
    fn nested_push_narrows_and_pop_widens_again() {
        let ctx = ModelCanvas::default();
        let mut s = SurfaceClip::default();
        s.push(&ctx, 0, 0, 100, 100);
        check(&s, &ctx, "outer");
        s.push(&ctx, 50, 50, 100, 100);
        check(&s, &ctx, "inner");
        assert_eq!(ctx.effective(), Some(r(50, 50, 50, 50)));
        s.pop(&ctx);
        check(&s, &ctx, "pop inner");
        // Canvas clips only intersect: a naive save+clip per push would
        // still be 50x50 here.
        assert_eq!(ctx.effective(), Some(r(0, 0, 100, 100)));
        s.pop(&ctx);
        check(&s, &ctx, "pop outer");
        assert_eq!(ctx.effective(), None);
    }

    #[test]
    fn reset_never_restores_past_the_base() {
        let ctx = ModelCanvas::default();
        let mut s = SurfaceClip::default();
        s.reset(&ctx);
        s.reset(&ctx);
        check(&s, &ctx, "reset twice, unclipped");
        s.set(&ctx, r(1, 2, 3, 4));
        s.reset(&ctx);
        s.reset(&ctx);
        check(&s, &ctx, "reset after set");
    }

    #[test]
    fn push_is_translated_and_disjoint_push_is_empty() {
        let ctx = ModelCanvas::default();
        let mut s = SurfaceClip::default();
        s.translate.push(10, 20);
        s.push(&ctx, 0, 0, 30, 30);
        assert_eq!(ctx.effective(), Some(r(10, 20, 30, 30)));
        s.push(&ctx, 500, 500, 5, 5);
        check(&s, &ctx, "disjoint");
        assert_eq!(ModelCanvas::area(ctx.effective()), Some(0));
        s.pop(&ctx);
        assert_eq!(ctx.effective(), Some(r(10, 20, 30, 30)));
    }

    #[test]
    fn clear_lifts_and_restores_the_clip() {
        let ctx = ModelCanvas::default();
        let mut s = SurfaceClip::default();
        s.push(&ctx, 5, 5, 10, 10);
        let during = s.unclipped(&ctx, || ctx.effective());
        assert_eq!(during, None);
        check(&s, &ctx, "after clear");
        assert_eq!(ctx.effective(), Some(r(5, 5, 10, 10)));
    }

    #[test]
    fn render_target_bind_and_unbind_keep_both_contexts_balanced() {
        let page = ModelCanvas::default();
        let target = ModelCanvas::default();
        let mut s = SurfaceClip::default();
        s.translate.push(3, 3);
        s.push(&page, 0, 0, 200, 100);

        let saved = s.bind();
        // The target starts unclipped and untranslated...
        assert_eq!(s.applied(), None);
        assert_eq!(s.translate.current(), (0, 0));
        // ...while the page keeps its clip level.
        assert_eq!(page.effective(), Some(r(3, 3, 200, 100)));
        s.push(&target, 10, 10, 20, 20);
        s.push(&target, 15, 15, 20, 20);
        check(&s, &target, "target nested");

        s.unbind(&target, saved);
        assert_eq!(target.depth(), 0, "target left clipped for its next bind");
        assert_eq!(target.effective(), None);
        check(&s, &page, "page after unbind");
        assert_eq!(s.translate.current(), (3, 3));
        s.pop(&page);
        check(&s, &page, "page pop");
        assert_eq!(page.effective(), None);
    }

    /// Long pseudo-random op sequences keep the invariant and agree with
    /// `ClipStack` (the same stack every other backend uses).
    #[test]
    fn random_op_sequences_keep_the_context_in_step() {
        let mut seed: u64 = 0x5eed_0a51;
        let mut next = move |n: u64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % n
        };
        for _ in 0..50 {
            let ctx = ModelCanvas::default();
            let mut s = SurfaceClip::default();
            let mut pushes = 0u32;
            let mut translates = 0u32;
            for step in 0..200 {
                let x = next(400) as i32 - 50;
                let y = next(300) as i32 - 50;
                let w = next(300) as u32;
                let h = next(200) as u32;
                let op = next(8);
                match op {
                    0 | 1 => {
                        s.push(&ctx, x, y, w, h);
                        pushes += 1;
                        let want = s.stack.current();
                        assert_eq!(s.applied(), want, "push applies the stack clip");
                    },
                    2 | 3 if pushes > 0 => {
                        s.pop(&ctx);
                        pushes -= 1;
                        assert_eq!(s.applied(), s.stack.current(), "pop restores");
                    },
                    4 => s.set(&ctx, r(x, y, w, h)),
                    5 => s.reset(&ctx),
                    6 => {
                        s.unclipped(&ctx, || assert_eq!(ctx.effective(), None));
                    },
                    7 if translates < 4 => {
                        s.translate.push(x / 10, y / 10);
                        translates += 1;
                    },
                    _ => {
                        if translates > 0 {
                            s.translate.pop();
                            translates -= 1;
                        }
                    },
                }
                check(&s, &ctx, &format!("step {step} op {op}"));
            }
            s.clear(&ctx);
            check(&s, &ctx, "clear");
            assert_eq!(ctx.depth(), 0);
        }
    }
}
