//! Pure-Rust JavaScript engine backed by [`boa_engine`].
//!
//! This module provides a minimal `BoaJsEngine` that implements just
//! enough of the rquickjs-backed [`crate::JsEngine`] surface (engine
//! init, `eval`, value conversion) to power the PSP backend. The
//! larger `JsEngine` API (console buffering, fetch handler, local
//! storage, timer queue, raw `with_context` escape hatch) is **not**
//! mirrored here — it is rquickjs-specific by design and would not
//! port without extensive rework. Desktop and WASM keep the full
//! `JsEngine`; PSP gets `BoaJsEngine`.
//!
//! The split exists because building rquickjs (C source via the `cc`
//! crate) for the `mipsel-sony-psp` target requires a full pspdev /
//! newlib cross-compile toolchain, which we do not currently have on
//! the build host. `boa_engine` is pure safe Rust — once
//! `std::time::Instant`, `HashMap`, and the global allocator work on
//! PSP (verified on real hardware, see the rust-psp branch
//! `fix/psp-hardware-std-overlay-alignment-and-time`), boa drops in
//! cleanly with no native dependencies.
//!
//! ## Performance expectations
//!
//! Boa is an interpreted reference implementation rather than a JIT,
//! and PSP Allegrex is a 333 MHz MIPS without SIMD. Realistic ballpark
//! is **~10× slower than QuickJS** for the same script, which is
//! itself ~500–1000× slower than a desktop V8. Inert pages with small
//! bootstrap scripts will work; React SPAs will be unusable. That
//! tradeoff is consistent with the original "PSP JavaScript
//! integration" epic in `docs/browser-backlog.md` — degraded is
//! better than dead.

use boa_engine::{Context, JsValue as BoaValue, Source};

use crate::types::{JsError, JsValue};

/// JavaScript engine backed by `boa_engine`.
///
/// One `BoaJsEngine` owns one boa `Context`. `eval` runs the script
/// to completion and converts the return value into the same
/// [`JsValue`] enum the rquickjs-backed engine uses, so callers can
/// share the value-handling code path across both backends.
///
/// Errors are surfaced as the same [`JsError`] type. The boa context
/// does not expose a stack-trace string in a stable form, so
/// `JsError::stack` is left `None` for now — the message field carries
/// boa's `JsError::Display` output, which is usually enough to find
/// the offending line.
pub struct BoaJsEngine {
    context: Context,
}

impl BoaJsEngine {
    /// Create a new engine with default boa settings.
    ///
    /// `_max_memory_bytes` is accepted for API parity with
    /// [`crate::JsEngine::new`] but is currently ignored — boa uses
    /// the global Rust allocator and does not expose a runtime-level
    /// memory cap. PSP code paths can rely on the global allocator's
    /// arena bound to limit total JS heap.
    pub fn new(_max_memory_bytes: usize) -> Result<Self, JsError> {
        // `Context::default` is infallible in current boa; `?`-style
        // error propagation is reserved for future versions that may
        // surface init errors.
        let context = Context::default();
        Ok(Self { context })
    }

    /// Evaluate a JavaScript source string and return the result.
    ///
    /// Unlike the rquickjs backend this does not currently install a
    /// time-based interrupt handler — boa exposes a different
    /// instruction-counting cancellation mechanism that we can wire
    /// up in a follow-up if the PSP browser ever runs untrusted
    /// scripts. For now, scripts that run forever will hang the
    /// engine; callers should treat `eval` as blocking and avoid
    /// running anything that isn't known to terminate.
    pub fn eval(&mut self, script: &str) -> Result<JsValue, JsError> {
        let source = Source::from_bytes(script.as_bytes());
        match self.context.eval(source) {
            Ok(value) => Ok(boa_value_to_js(&value, &mut self.context)),
            Err(err) => Err(JsError {
                message: format!("{err}"),
                stack: None,
            }),
        }
    }
}

/// Convert a `boa_engine::JsValue` into the engine-agnostic
/// [`JsValue`] enum.
///
/// Values that don't fit any of the six primitive variants are
/// rendered as their `to_string` form so callers always get *some*
/// readable result rather than a panic.
fn boa_value_to_js(value: &BoaValue, context: &mut Context) -> JsValue {
    if value.is_undefined() {
        return JsValue::Undefined;
    }
    if value.is_null() {
        return JsValue::Null;
    }
    if let Some(b) = value.as_boolean() {
        return JsValue::Bool(b);
    }
    if let Some(n) = value.as_number() {
        // Mirror rquickjs's behaviour: prefer `Int` when the value is
        // an exact 32-bit integer, otherwise fall back to `Float`.
        if n.is_finite()
            && n.fract() == 0.0
            && !(n == 0.0 && n.is_sign_negative())
            && n >= i32::MIN as f64
            && n <= i32::MAX as f64
        {
            return JsValue::Int(n as i32);
        }
        return JsValue::Float(n);
    }
    if let Some(s) = value.as_string() {
        return JsValue::String(s.to_std_string_escaped());
    }
    // Objects, arrays, BigInts, symbols, etc. — render via JS
    // `String(value)` so the caller sees a meaningful textual form.
    match value.to_string(context) {
        Ok(s) => JsValue::String(s.to_std_string_escaped()),
        Err(_) => JsValue::Undefined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_arithmetic_returns_int() {
        let mut engine = BoaJsEngine::new(0).expect("engine init");
        let value = engine.eval("1 + 2 + 3").expect("eval");
        assert_eq!(value, JsValue::Int(6));
    }

    #[test]
    fn eval_string_concat() {
        let mut engine = BoaJsEngine::new(0).expect("engine init");
        let value = engine.eval("'foo' + 'bar'").expect("eval");
        assert_eq!(value, JsValue::String("foobar".to_string()));
    }

    #[test]
    fn eval_float_returns_float() {
        let mut engine = BoaJsEngine::new(0).expect("engine init");
        // Use a value with a non-integer fractional part so the int-folding
        // branch in `boa_value_to_js` doesn't downcast it to `Int`.
        let value = engine.eval("1.5 * 2.5").expect("eval");
        assert_eq!(value, JsValue::Float(3.75));
    }

    #[test]
    fn eval_undefined() {
        let mut engine = BoaJsEngine::new(0).expect("engine init");
        let value = engine.eval("void 0").expect("eval");
        assert_eq!(value, JsValue::Undefined);
    }

    #[test]
    fn eval_function_call() {
        let mut engine = BoaJsEngine::new(0).expect("engine init");
        let value = engine
            .eval("function add(a, b) { return a + b; } add(40, 2)")
            .expect("eval");
        assert_eq!(value, JsValue::Int(42));
    }

    #[test]
    fn eval_syntax_error_returns_js_error() {
        let mut engine = BoaJsEngine::new(0).expect("engine init");
        let err = engine.eval("function (").expect_err("should fail");
        assert!(!err.message.is_empty());
    }
}
