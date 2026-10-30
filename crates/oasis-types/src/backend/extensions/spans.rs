//! Adapter from the shared span generators to `SdiCore::fill_rect`.

use crate::backend::{Color, SdiCore};
use crate::error::Result;

/// Turns half-open row spans into `fill_rect` calls for the trait
/// defaults.
///
/// A span that continues the previous one straight down (same columns,
/// next row) grows that rectangle instead of starting a new one, so the
/// straight parts of a shape (a rounded rect's body, a vertical line, a
/// polygon's rectangular middle) cost one `fill_rect` rather than one per
/// row. The covered pixels are exactly the spans' union and no pixel is
/// covered twice, as long as the generator itself reports each pixel once.
///
/// The first error from `fill_rect` is kept and later spans are dropped.
pub(crate) struct SpanFill<'a, B: SdiCore + ?Sized> {
    backend: &'a mut B,
    color: Color,
    /// Pending rectangle `(x0, x1, y, rows)`, columns half-open.
    pending: Option<(i32, i32, i32, u32)>,
    result: Result<()>,
}

impl<'a, B: SdiCore + ?Sized> SpanFill<'a, B> {
    pub(crate) fn new(backend: &'a mut B, color: Color) -> Self {
        Self {
            backend,
            color,
            pending: None,
            result: Ok(()),
        }
    }

    /// Add the span covering columns `x0..x1` of row `y`.
    pub(crate) fn span(&mut self, y: i32, x0: i32, x1: i32) {
        if x1 <= x0 || self.result.is_err() {
            return;
        }
        if let Some((px0, px1, py, rows)) = &mut self.pending
            && *px0 == x0
            && *px1 == x1
            && *py + *rows as i32 == y
        {
            *rows += 1;
            return;
        }
        self.flush();
        self.pending = Some((x0, x1, y, 1));
    }

    fn flush(&mut self) {
        if let Some((x0, x1, y, rows)) = self.pending.take()
            && self.result.is_ok()
        {
            self.result = self
                .backend
                .fill_rect(x0, y, (x1 - x0) as u32, rows, self.color);
        }
    }

    /// Emit the last pending rectangle and return the first error, if any.
    pub(crate) fn finish(mut self) -> Result<()> {
        self.flush();
        self.result
    }
}
