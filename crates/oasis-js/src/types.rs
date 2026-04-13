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
            // Match JS number-to-string semantics for the corner cases
            // Rust's default `f64` formatter gets wrong:
            //   - `-0.0`                  -> JS "0"         (Rust "-0")
            //   - `f64::INFINITY`         -> JS "Infinity"  (Rust "inf")
            //   - `f64::NEG_INFINITY`     -> JS "-Infinity" (Rust "-inf")
            //   - `f64::NAN`              -> JS "NaN"       (Rust "NaN" — already correct)
            Self::Float(v) => {
                if *v == 0.0 && v.is_sign_negative() {
                    f.write_str("0")
                } else if v.is_infinite() {
                    let sign = if v.is_sign_negative() { "-" } else { "" };
                    write!(f, "{sign}Infinity")
                } else {
                    write!(f, "{v}")
                }
            },
            Self::String(s) => f.write_str(s),
        }
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn display_primitives() {
        assert_eq!(JsValue::Undefined.to_string(), "undefined");
        assert_eq!(JsValue::Null.to_string(), "null");
        assert_eq!(JsValue::Bool(true).to_string(), "true");
        assert_eq!(JsValue::Bool(false).to_string(), "false");
        assert_eq!(JsValue::Int(42).to_string(), "42");
        assert_eq!(JsValue::Int(-7).to_string(), "-7");
        assert_eq!(JsValue::String("hi".to_string()).to_string(), "hi");
    }

    #[test]
    fn display_float_finite() {
        assert_eq!(JsValue::Float(3.25).to_string(), "3.25");
        assert_eq!(JsValue::Float(-0.5).to_string(), "-0.5");
    }

    #[test]
    fn display_float_negative_zero() {
        // JS: String(-0) === "0" — Rust default formats as "-0".
        assert_eq!(JsValue::Float(-0.0).to_string(), "0");
        assert_eq!(JsValue::Float(0.0).to_string(), "0");
    }

    #[test]
    fn display_float_infinity_matches_javascript() {
        // JS: String(Infinity)  === "Infinity"
        // JS: String(-Infinity) === "-Infinity"
        // Rust default: "inf" / "-inf" — that's the bug this test pins.
        assert_eq!(JsValue::Float(f64::INFINITY).to_string(), "Infinity");
        assert_eq!(JsValue::Float(f64::NEG_INFINITY).to_string(), "-Infinity");
    }

    #[test]
    fn display_float_nan_matches_javascript() {
        // Rust's default NaN formatter already matches JS ("NaN"), but
        // pinning it keeps us honest if the default ever changes.
        assert_eq!(JsValue::Float(f64::NAN).to_string(), "NaN");
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
