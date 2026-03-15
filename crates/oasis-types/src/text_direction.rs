//! Text direction types for RTL/LTR layout support.
//!
//! Provides the foundational `TextDirection` enum used across the UI,
//! browser engine, and skin/theme systems to control inline text flow
//! and logical-to-physical property mapping.

/// Text direction for inline content flow.
///
/// Controls whether inline content flows left-to-right or right-to-left.
/// `Auto` defers to the Unicode Bidi algorithm or the inherited direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextDirection {
    /// Left-to-right (default for Latin, Cyrillic, etc.).
    #[default]
    Ltr,
    /// Right-to-left (Arabic, Hebrew, etc.).
    Rtl,
    /// Automatic detection based on content or inheritance.
    Auto,
}

impl TextDirection {
    /// Returns `true` if this direction is RTL.
    ///
    /// `Auto` is treated as LTR (the CSS default).
    pub fn is_rtl(self) -> bool {
        self == Self::Rtl
    }

    /// Returns `true` if this direction is LTR or Auto.
    pub fn is_ltr(self) -> bool {
        !self.is_rtl()
    }

    /// Resolve `Auto` to the given fallback direction.
    pub fn resolve(self, fallback: TextDirection) -> TextDirection {
        if self == Self::Auto { fallback } else { self }
    }

    /// Map logical "start" to a physical alignment.
    ///
    /// In LTR, "start" means left. In RTL, "start" means right.
    pub fn start_is_left(self) -> bool {
        self.is_ltr()
    }

    /// Swap left/right values for RTL contexts.
    ///
    /// Returns `(physical_left, physical_right)` from logical
    /// `(inline_start, inline_end)`.
    pub fn map_inline_edges(self, start: f32, end: f32) -> (f32, f32) {
        if self.is_rtl() {
            (end, start)
        } else {
            (start, end)
        }
    }
}

impl core::fmt::Display for TextDirection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ltr => write!(f, "ltr"),
            Self::Rtl => write!(f, "rtl"),
            Self::Auto => write!(f, "auto"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_ltr() {
        assert_eq!(TextDirection::default(), TextDirection::Ltr);
    }

    #[test]
    fn is_rtl() {
        assert!(TextDirection::Rtl.is_rtl());
        assert!(!TextDirection::Ltr.is_rtl());
        assert!(!TextDirection::Auto.is_rtl());
    }

    #[test]
    fn is_ltr() {
        assert!(TextDirection::Ltr.is_ltr());
        assert!(TextDirection::Auto.is_ltr());
        assert!(!TextDirection::Rtl.is_ltr());
    }

    #[test]
    fn resolve_auto_uses_fallback() {
        assert_eq!(
            TextDirection::Auto.resolve(TextDirection::Rtl),
            TextDirection::Rtl,
        );
        assert_eq!(
            TextDirection::Auto.resolve(TextDirection::Ltr),
            TextDirection::Ltr,
        );
    }

    #[test]
    fn resolve_explicit_ignores_fallback() {
        assert_eq!(
            TextDirection::Rtl.resolve(TextDirection::Ltr),
            TextDirection::Rtl,
        );
        assert_eq!(
            TextDirection::Ltr.resolve(TextDirection::Rtl),
            TextDirection::Ltr,
        );
    }

    #[test]
    fn start_is_left_ltr() {
        assert!(TextDirection::Ltr.start_is_left());
        assert!(!TextDirection::Rtl.start_is_left());
    }

    #[test]
    fn map_inline_edges_ltr() {
        let (l, r) = TextDirection::Ltr.map_inline_edges(10.0, 20.0);
        assert_eq!(l, 10.0);
        assert_eq!(r, 20.0);
    }

    #[test]
    fn map_inline_edges_rtl() {
        let (l, r) = TextDirection::Rtl.map_inline_edges(10.0, 20.0);
        assert_eq!(l, 20.0);
        assert_eq!(r, 10.0);
    }

    #[test]
    fn display_format() {
        assert_eq!(format!("{}", TextDirection::Ltr), "ltr");
        assert_eq!(format!("{}", TextDirection::Rtl), "rtl");
        assert_eq!(format!("{}", TextDirection::Auto), "auto");
    }
}
