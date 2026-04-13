//! Engine-agnostic value and error types.
//!
//! Both the rquickjs-backed [`crate::JsEngine`] and the boa-backed
//! [`crate::BoaJsEngine`] return `JsValue` from their `eval` methods
//! and `JsError` on failure. Keeping the types here means callers can
//! write engine-independent code that works under either backend
//! without paying for the other backend's transitive dependencies.

use core::fmt;

/// A simple representation of a JavaScript return value.
///
/// Engines collapse complex JS values (objects, arrays, BigInts,
/// symbols) to their `String(value)` form so callers always see
/// something printable. Numbers that fit losslessly in `i32` are
/// reported as [`JsValue::Int`]; anything else is [`JsValue::Float`].
#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Int(i32),
    Float(f64),
    String(String),
}

impl fmt::Display for JsValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undefined => f.write_str("undefined"),
            Self::Null => f.write_str("null"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int(n) => write!(f, "{n}"),
            Self::Float(v) => write!(f, "{v}"),
            Self::String(s) => f.write_str(s),
        }
    }
}

/// An error produced by JavaScript execution.
///
/// `stack` is `None` on backends that don't expose a stack-trace
/// string (currently boa). The rquickjs backend populates it with
/// QuickJS's error object `.stack` property when available.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}{}", stack.as_ref().map(|s| format!("\n{s}")).unwrap_or_default())]
pub struct JsError {
    pub message: String,
    pub stack: Option<String>,
}
