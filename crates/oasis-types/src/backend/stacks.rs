//! Shared clip-rect and translate stacks for backend implementations.
//!
//! SDL, WASM, and UE5 backends all maintain clip and translate stacks with
//! nearly identical logic. This module provides reusable helpers so each
//! backend only needs to wire up platform-specific calls (e.g. setting the
//! SDL clip rect or calling `ctx.clip()` in WASM).

use crate::geometry::ClipRect;

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
        self.cumulative.0 += dx;
        self.cumulative.1 += dy;
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
    #[must_use]
    pub fn translate(&self, x: i32, y: i32) -> (i32, i32) {
        (x + self.cumulative.0, y + self.cumulative.1)
    }

    /// Apply the current translation returning `f64` (useful for WASM canvas).
    #[must_use]
    pub fn translate_f64(&self, x: i32, y: i32) -> (f64, f64) {
        (
            (x + self.cumulative.0) as f64,
            (y + self.cumulative.1) as f64,
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
#[derive(Debug, Clone)]
pub struct ClipStack {
    /// Previous clip states (what to restore on pop).
    stack: Vec<ClipRect>,
    /// The current effective clip, or `None` if clipping is disabled.
    current: Option<ClipRect>,
    /// Viewport dimensions -- used as the sentinel value for "no clip".
    viewport_w: u32,
    viewport_h: u32,
}

impl ClipStack {
    /// Create a new clip stack for a viewport of the given size.
    #[must_use]
    pub fn new(viewport_w: u32, viewport_h: u32) -> Self {
        Self {
            stack: Vec::new(),
            current: None,
            viewport_w,
            viewport_h,
        }
    }

    /// Push a new clip rectangle (already translated by the caller).
    ///
    /// Returns what clip the backend should apply on the platform side.
    pub fn push(&mut self, clip: ClipRect) -> ClipPush {
        if let Some(current) = self.current {
            self.stack.push(current);
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
        } else {
            // No clip was active -- push a sentinel for the full viewport.
            self.stack.push(ClipRect {
                x: 0,
                y: 0,
                w: self.viewport_w,
                h: self.viewport_h,
            });
            self.current = Some(clip);
            ClipPush::Clip(clip)
        }
    }

    /// Pop the most recent clip.
    ///
    /// Returns `Some(clip)` with the restored clip rectangle, or `None` if
    /// clipping should be disabled (either the stack was empty or the restored
    /// state is the full-viewport sentinel).
    pub fn pop(&mut self) -> Option<ClipRect> {
        if let Some(prev) = self.stack.pop() {
            if prev.x == 0 && prev.y == 0 && prev.w == self.viewport_w && prev.h == self.viewport_h
            {
                self.current = None;
                None
            } else {
                self.current = Some(prev);
                Some(prev)
            }
        } else {
            self.current = None;
            None
        }
    }

    /// The currently active clip rectangle, or `None` if clipping is disabled.
    #[must_use]
    pub fn current(&self) -> Option<ClipRect> {
        self.current
    }

    /// Current clip as an `(x, y, w, h)` tuple, or `None`.
    #[must_use]
    pub fn current_tuple(&self) -> Option<(i32, i32, u32, u32)> {
        self.current.map(|c| (c.x, c.y, c.w, c.h))
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
        let mut cs = ClipStack::new(100, 100);
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
        let mut cs = ClipStack::new(100, 100);
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
        let mut cs = ClipStack::new(100, 100);
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
        let mut cs = ClipStack::new(100, 100);
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
    fn clip_pop_empty_stack() {
        let mut cs = ClipStack::new(100, 100);
        assert!(cs.pop().is_none());
        assert!(cs.current().is_none());
    }

    #[test]
    fn clip_clear() {
        let mut cs = ClipStack::new(100, 100);
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
