//! Shared clip-rect and translate stacks for backend implementations.
//!
//! SDL, WASM, and UE5 backends all maintain clip and translate stacks with
//! nearly identical logic. This module provides reusable helpers so each
//! backend only needs to wire up platform-specific calls (e.g. setting the
//! SDL clip rect or calling `ctx.clip()` in WASM).
//!
//! [`DrawState`] bundles both stacks so a backend can save them on
//! `bind_render_target` and restore them on `unbind_render_target`, as the
//! [`SdiRenderTarget`](crate::backend::SdiRenderTarget) contract requires.

use crate::geometry::ClipRect;

// ---------------------------------------------------------------------------
// DrawState
// ---------------------------------------------------------------------------

/// The clip and translate stacks of one drawing surface.
///
/// A render-target bind starts the target with no clip and no
/// translation, and the unbind brings the parent's stacks back unchanged:
///
/// ```
/// use oasis_types::backend::stacks::{ClipStack, DrawState, TranslateStack};
/// use oasis_types::geometry::ClipRect;
///
/// let mut clip = ClipStack::default();
/// let mut translate = TranslateStack::new();
/// clip.push(ClipRect { x: 1, y: 2, w: 3, h: 4 });
/// translate.push(10, 20);
///
/// // bind_render_target
/// let saved = DrawState::take(&mut clip, &mut translate);
/// assert_eq!(clip.current(), None);
/// assert_eq!(translate.current(), (0, 0));
///
/// // unbind_render_target
/// saved.restore(&mut clip, &mut translate);
/// assert_eq!(clip.current(), Some(ClipRect { x: 1, y: 2, w: 3, h: 4 }));
/// assert_eq!(translate.current(), (10, 20));
/// ```
#[derive(Debug, Clone, Default)]
pub struct DrawState {
    /// The surface's clip stack.
    pub clip: ClipStack,
    /// The surface's translate stack.
    pub translate: TranslateStack,
}

impl DrawState {
    /// Move both stacks out, leaving them empty (no clip, no translation).
    #[must_use]
    pub fn take(clip: &mut ClipStack, translate: &mut TranslateStack) -> Self {
        Self {
            clip: std::mem::take(clip),
            translate: std::mem::take(translate),
        }
    }

    /// Put stacks saved by [`take`](Self::take) back, discarding the
    /// current ones.
    pub fn restore(self, clip: &mut ClipStack, translate: &mut TranslateStack) {
        *clip = self.clip;
        *translate = self.translate;
    }
}

// ---------------------------------------------------------------------------
// TranslateStack
// ---------------------------------------------------------------------------

/// Manages a stack of cumulative (dx, dy) translation offsets.
///
/// Every `push` saves the current offset and adds the delta.
/// Every `pop` restores the previous offset.
#[derive(Debug, Clone)]
pub struct TranslateStack {
    stack: Vec<(i32, i32)>,
    cumulative: (i32, i32),
}

impl Default for TranslateStack {
    fn default() -> Self {
        Self::new()
    }
}

impl TranslateStack {
    /// Create a new translate stack with zero offset.
    #[must_use]
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            cumulative: (0, 0),
        }
    }

    /// Save the current offset and add `(dx, dy)`.
    pub fn push(&mut self, dx: i32, dy: i32) {
        self.stack.push(self.cumulative);
        self.cumulative.0 = self.cumulative.0.saturating_add(dx);
        self.cumulative.1 = self.cumulative.1.saturating_add(dy);
    }

    /// Restore the previous offset.  Returns `true` if there was a frame to
    /// pop, `false` if the stack was already empty (offset stays at zero).
    pub fn pop(&mut self) -> bool {
        if let Some(prev) = self.stack.pop() {
            self.cumulative = prev;
            true
        } else {
            false
        }
    }

    /// Current cumulative translation offset.
    #[must_use]
    pub fn current(&self) -> (i32, i32) {
        self.cumulative
    }

    /// Apply the current translation to a point.
    ///
    /// Saturates instead of overflowing, so a far off-screen point stays
    /// off-screen on the same side (a wrapping add would move it to the
    /// opposite edge, and a plain `+` panics in debug builds).
    #[must_use]
    pub fn translate(&self, x: i32, y: i32) -> (i32, i32) {
        (
            x.saturating_add(self.cumulative.0),
            y.saturating_add(self.cumulative.1),
        )
    }

    /// Apply the current translation returning `f64` (useful for WASM canvas).
    #[must_use]
    pub fn translate_f64(&self, x: i32, y: i32) -> (f64, f64) {
        (
            x as f64 + self.cumulative.0 as f64,
            y as f64 + self.cumulative.1 as f64,
        )
    }

    /// Reset to empty.
    pub fn clear(&mut self) {
        self.stack.clear();
        self.cumulative = (0, 0);
    }
}

// ---------------------------------------------------------------------------
// ClipStack
// ---------------------------------------------------------------------------

/// The result of a [`ClipStack::push`] operation, telling the backend what
/// clip rectangle to apply on the platform side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipPush {
    /// Apply this clip rectangle.
    Clip(ClipRect),
    /// The intersection is empty -- apply a zero-area clip.
    Empty,
}

/// Manages a stack of nested clip rectangles with automatic intersection.
///
/// On `push`, the new rectangle is intersected with the current clip.
/// On `pop`, the previous clip is restored.
///
/// The stack tracks whether a clip is currently active.  When no clip is
/// active the first push simply activates the given rectangle.  Subsequent
/// pushes intersect with the active clip.
///
/// "No clip" is stored as `None`, not as a full-viewport rectangle, so a
/// pushed clip that happens to equal the viewport restores correctly and
/// the stack never depends on a viewport size that can go stale on resize.
#[derive(Debug, Clone, Default)]
pub struct ClipStack {
    /// Previous clip states (what to restore on pop); `None` = no clip.
    stack: Vec<Option<ClipRect>>,
    /// The current effective clip, or `None` if clipping is disabled.
    current: Option<ClipRect>,
}

impl ClipStack {
    /// Push a new clip rectangle (already translated by the caller).
    ///
    /// Returns what clip the backend should apply on the platform side.
    pub fn push(&mut self, clip: ClipRect) -> ClipPush {
        self.stack.push(self.current);
        let Some(current) = self.current else {
            self.current = Some(clip);
            return ClipPush::Clip(clip);
        };
        match current.intersect(&clip) {
            Some(isect) => {
                self.current = Some(isect);
                ClipPush::Clip(isect)
            },
            None => {
                self.current = Some(ClipRect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                });
                ClipPush::Empty
            },
        }
    }

    /// Pop the most recent clip.
    ///
    /// Returns `Some(clip)` with the restored clip rectangle, or `None` if
    /// clipping should be disabled (the stack was empty or no clip was
    /// active before the matching push).
    pub fn pop(&mut self) -> Option<ClipRect> {
        self.current = self.stack.pop().flatten();
        self.current
    }

    /// The currently active clip rectangle, or `None` if clipping is disabled.
    #[must_use]
    pub fn current(&self) -> Option<ClipRect> {
        self.current
    }

    /// Reset to empty (no active clip).
    pub fn clear(&mut self) {
        self.stack.clear();
        self.current = None;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- TranslateStack --

    #[test]
    fn translate_saturates_instead_of_overflowing() {
        let mut ts = TranslateStack::new();
        ts.push(i32::MAX, i32::MIN);
        ts.push(10, -10);
        assert_eq!(ts.current(), (i32::MAX, i32::MIN));
        assert_eq!(ts.translate(5, -5), (i32::MAX, i32::MIN));
        assert_eq!(
            ts.translate_f64(-5, 5),
            (i32::MAX as f64 - 5.0, i32::MIN as f64 + 5.0)
        );
        assert!(ts.pop());
        assert_eq!(ts.current(), (i32::MAX, i32::MIN));
    }

    #[test]
    fn translate_push_pop() {
        let mut ts = TranslateStack::new();
        assert_eq!(ts.current(), (0, 0));

        ts.push(5, 10);
        assert_eq!(ts.current(), (5, 10));
        assert_eq!(ts.translate(1, 2), (6, 12));

        ts.push(3, 7);
        assert_eq!(ts.current(), (8, 17));

        assert!(ts.pop());
        assert_eq!(ts.current(), (5, 10));

        assert!(ts.pop());
        assert_eq!(ts.current(), (0, 0));

        // Pop on empty returns false and stays at zero.
        assert!(!ts.pop());
        assert_eq!(ts.current(), (0, 0));
    }

    #[test]
    fn translate_clear() {
        let mut ts = TranslateStack::new();
        ts.push(1, 2);
        ts.push(3, 4);
        ts.clear();
        assert_eq!(ts.current(), (0, 0));
    }

    #[test]
    fn translate_f64() {
        let mut ts = TranslateStack::new();
        ts.push(5, 10);
        assert_eq!(ts.translate_f64(1, 2), (6.0, 12.0));
    }

    // -- ClipStack --

    #[test]
    fn clip_first_push_activates() {
        let mut cs = ClipStack::default();
        assert!(cs.current().is_none());

        let result = cs.push(ClipRect {
            x: 10,
            y: 10,
            w: 50,
            h: 50,
        });
        assert_eq!(
            result,
            ClipPush::Clip(ClipRect {
                x: 10,
                y: 10,
                w: 50,
                h: 50
            })
        );
        assert_eq!(
            cs.current(),
            Some(ClipRect {
                x: 10,
                y: 10,
                w: 50,
                h: 50
            })
        );
    }

    #[test]
    fn clip_nested_intersection() {
        let mut cs = ClipStack::default();
        cs.push(ClipRect {
            x: 10,
            y: 10,
            w: 80,
            h: 80,
        });
        let result = cs.push(ClipRect {
            x: 20,
            y: 20,
            w: 80,
            h: 80,
        });
        // Intersection: (20,20)-(90,90) -> w=70, h=70
        assert_eq!(
            result,
            ClipPush::Clip(ClipRect {
                x: 20,
                y: 20,
                w: 70,
                h: 70
            })
        );
    }

    #[test]
    fn clip_no_overlap_gives_empty() {
        let mut cs = ClipStack::default();
        cs.push(ClipRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        });
        let result = cs.push(ClipRect {
            x: 50,
            y: 50,
            w: 10,
            h: 10,
        });
        assert_eq!(result, ClipPush::Empty);
    }

    #[test]
    fn clip_pop_restores() {
        let mut cs = ClipStack::default();
        cs.push(ClipRect {
            x: 10,
            y: 10,
            w: 50,
            h: 50,
        });
        cs.push(ClipRect {
            x: 20,
            y: 20,
            w: 30,
            h: 30,
        });

        // Pop inner: restores outer clip.
        let restored = cs.pop();
        assert_eq!(
            restored,
            Some(ClipRect {
                x: 10,
                y: 10,
                w: 50,
                h: 50
            })
        );

        // Pop outer: restores to no-clip (full viewport sentinel).
        let restored = cs.pop();
        assert!(restored.is_none());
        assert!(cs.current().is_none());
    }

    #[test]
    fn clip_equal_to_viewport_restores() {
        // A nested clip exactly the size of the viewport used to collide
        // with the "no clip" sentinel and pop to `None`.
        let mut cs = ClipStack::default();
        let outer = ClipRect {
            x: 10,
            y: 10,
            w: 50,
            h: 50,
        };
        let full = ClipRect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
        };
        cs.push(full);
        cs.push(outer);
        assert_eq!(cs.pop(), Some(full), "restores the full-size clip");
        assert_eq!(cs.current(), Some(full));
        assert_eq!(cs.pop(), None);
        assert_eq!(cs.current(), None);
    }

    #[test]
    fn clip_pop_empty_stack() {
        let mut cs = ClipStack::default();
        assert!(cs.pop().is_none());
        assert!(cs.current().is_none());
    }

    #[test]
    fn clip_clear() {
        let mut cs = ClipStack::default();
        cs.push(ClipRect {
            x: 0,
            y: 0,
            w: 50,
            h: 50,
        });
        cs.clear();
        assert!(cs.current().is_none());
    }
}
