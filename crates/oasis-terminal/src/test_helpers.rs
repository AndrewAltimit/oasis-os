//! Test assertion helpers for command output matching.
//!
//! These macros reduce boilerplate in tests that match on [`CommandOutput`] variants.
//! Instead of:
//!
//! ```ignore
//! let CommandOutput::Text(s) = result else {
//!     panic!("expected CommandOutput::Text, got {result:?}");
//! };
//! assert!(s.contains("expected"));
//! ```
//!
//! Write:
//!
//! ```ignore
//! let s = assert_text!(result);
//! assert!(s.contains("expected"));
//! ```

/// Assert that a [`CommandOutput`] is `Text` and return the inner `String`.
///
/// # Examples
///
/// ```ignore
/// let s = assert_text!(result);
/// assert_eq!(s, "hello");
/// assert!(s.contains("substring"));
/// ```
macro_rules! assert_text {
    ($expr:expr) => {{
        let __val = $expr;
        let $crate::CommandOutput::Text(s) = __val else {
            panic!("expected CommandOutput::Text, got {__val:?}");
        };
        s
    }};
}

/// Assert that a [`CommandOutput`] is `Clear`.
#[allow(unused_macros)]
macro_rules! assert_clear {
    ($expr:expr) => {{
        let __val = $expr;
        assert!(
            matches!(__val, $crate::CommandOutput::Clear),
            "expected CommandOutput::Clear, got {__val:?}"
        );
    }};
}

/// Assert that a [`CommandOutput`] is `None` (no visible output).
#[allow(unused_macros)]
macro_rules! assert_none_output {
    ($expr:expr) => {{
        let __val = $expr;
        assert!(
            matches!(__val, $crate::CommandOutput::None),
            "expected CommandOutput::None, got {__val:?}"
        );
    }};
}

#[allow(unused_imports)]
pub(crate) use assert_clear;
#[allow(unused_imports)]
pub(crate) use assert_none_output;
pub(crate) use assert_text;
