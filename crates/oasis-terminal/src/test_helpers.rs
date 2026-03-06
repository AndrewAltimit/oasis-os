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
//! TODO: Apply these macros to the remaining ~170 match/panic patterns across all
//! test modules (commands.rs, interpreter.rs, text_commands.rs, file_commands.rs,
//! dev_commands.rs, fun_commands.rs, system_commands.rs, network_commands.rs,
//! skin_commands.rs, ui_commands.rs, audio_commands.rs, doc_commands.rs,
//! radio_commands.rs, etc.).

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
