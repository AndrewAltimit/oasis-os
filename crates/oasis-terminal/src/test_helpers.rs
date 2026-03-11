//! Test assertion helpers for command output matching.
//!
//! These macros reduce boilerplate in tests that match on [`CommandOutput`] variants.
//! Instead of:
//!
//! ```ignore
//! match result {
//!     CommandOutput::Text(s) => assert!(s.contains("expected")),
//!     _ => panic!("expected text"),
//! }
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
    ($expr:expr) => {
        match $expr {
            $crate::CommandOutput::Text(s) => s,
            other => panic!("expected CommandOutput::Text, got {other:?}"),
        }
    };
}

/// Assert that a [`CommandOutput`] is `Clear`.
#[allow(unused_macros)]
macro_rules! assert_clear {
    ($expr:expr) => {
        match $expr {
            $crate::CommandOutput::Clear => {},
            other => panic!("expected CommandOutput::Clear, got {other:?}"),
        }
    };
}

/// Assert that a [`CommandOutput`] is `None` (no visible output).
#[allow(unused_macros)]
macro_rules! assert_none_output {
    ($expr:expr) => {
        match $expr {
            $crate::CommandOutput::None => {},
            other => panic!("expected CommandOutput::None, got {other:?}"),
        }
    };
}

#[allow(unused_imports)]
pub(crate) use assert_clear;
#[allow(unused_imports)]
pub(crate) use assert_none_output;
pub(crate) use assert_text;
