//! Test assertion helpers for command output matching.
//!
//! These macros reduce boilerplate in tests that match on [`CommandOutput`] variants.
//! Instead of:
//!
//! ```ignore
//! assert!(matches!(&result, CommandOutput::Text(_)), "expected text, got {result:?}");
//! let CommandOutput::Text(s) = result else { unreachable!() };
//! assert!(s.contains("expected"));
//! ```
//!
//! Write:
//!
//! ```ignore
//! let s = assert_text!(result);
//! assert!(s.contains("expected"));
//! ```
//!
//! Some match/panic patterns remain in commands.rs, interpreter tests,
//! and control_flow.rs that could still be converted.

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
        assert!(
            matches!(&__val, $crate::CommandOutput::Text(_)),
            "expected CommandOutput::Text, got {__val:?}"
        );
        let $crate::CommandOutput::Text(s) = __val else {
            unreachable!()
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
