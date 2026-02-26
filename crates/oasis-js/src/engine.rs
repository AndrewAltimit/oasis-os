use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use rquickjs::{Context, Runtime};

use crate::console::{ConsoleBuffer, ConsoleEntry, ConsoleLevel};

/// A simple representation of a JavaScript return value.
#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Int(i32),
    Float(f64),
    String(String),
}

/// An error produced by JavaScript execution.
#[derive(Debug, Clone)]
pub struct JsError {
    pub message: String,
    pub stack: Option<String>,
}

impl fmt::Display for JsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(stack) = &self.stack {
            write!(f, "\n{stack}")?;
        }
        Ok(())
    }
}

impl std::error::Error for JsError {}

/// JavaScript engine wrapping a QuickJS-NG runtime and context.
///
/// Provides `eval` / `eval_all` for executing scripts, and
/// `console_output` / `take_console_output` for reading buffered
/// `console.log` (etc.) output.
pub struct JsEngine {
    _runtime: Runtime,
    context: Context,
    console_buf: ConsoleBuffer,
}

impl JsEngine {
    /// Create a new engine.
    ///
    /// `max_memory_bytes` sets the QuickJS memory limit. Note: with the
    /// `rust-alloc` feature this is a no-op — the Rust global allocator
    /// is used instead.
    pub fn new(max_memory_bytes: usize) -> Result<Self, JsError> {
        let runtime = Runtime::new().map_err(|e| JsError {
            message: format!("failed to create JS runtime: {e}"),
            stack: None,
        })?;
        runtime.set_memory_limit(max_memory_bytes);

        let context = Context::full(&runtime).map_err(|e| JsError {
            message: format!("failed to create JS context: {e}"),
            stack: None,
        })?;

        let console_buf: ConsoleBuffer = Rc::new(RefCell::new(Vec::new()));

        let buf = Rc::clone(&console_buf);
        context
            .with(|ctx| crate::console::install(&ctx, buf))
            .map_err(|e| JsError {
                message: format!("failed to install globals: {e}"),
                stack: None,
            })?;

        Ok(Self {
            _runtime: runtime,
            context,
            console_buf,
        })
    }

    /// Evaluate a JavaScript source string and return the result.
    pub fn eval(&self, script: &str) -> Result<JsValue, JsError> {
        self.context.with(|ctx| {
            let result: Result<rquickjs::Value<'_>, rquickjs::Error> = ctx.eval(script);
            match result {
                Ok(val) => Ok(convert_value(&val)),
                Err(err) => {
                    let js_err = convert_error(&ctx, err);
                    self.console_buf.borrow_mut().push(ConsoleEntry {
                        level: ConsoleLevel::Error,
                        message: js_err.to_string(),
                    });
                    Err(js_err)
                },
            }
        })
    }

    /// Evaluate multiple scripts in document order, returning a result
    /// for each. Execution continues even if an earlier script fails.
    pub fn eval_all(&self, scripts: &[&str]) -> Vec<Result<JsValue, JsError>> {
        scripts.iter().map(|s| self.eval(s)).collect()
    }

    /// Return a clone of the buffered console output.
    pub fn console_output(&self) -> Vec<ConsoleEntry> {
        self.console_buf.borrow().clone()
    }

    /// Take and clear the buffered console output.
    pub fn take_console_output(&self) -> Vec<ConsoleEntry> {
        std::mem::take(&mut self.console_buf.borrow_mut())
    }

    /// Run a closure with access to the raw rquickjs context.
    ///
    /// This lets external crates (e.g. `oasis-browser`) register
    /// additional globals such as `document` without depending on
    /// rquickjs internals directly.
    pub fn with_context<R, F>(&self, f: F) -> Result<R, JsError>
    where
        F: FnOnce(rquickjs::Ctx<'_>) -> rquickjs::Result<R>,
    {
        self.context.with(|ctx| {
            f(ctx).map_err(|e| JsError {
                message: e.to_string(),
                stack: None,
            })
        })
    }
}

/// Convert a rquickjs `Value` to our public `JsValue` enum.
fn convert_value(val: &rquickjs::Value<'_>) -> JsValue {
    if val.is_undefined() {
        JsValue::Undefined
    } else if val.is_null() {
        JsValue::Null
    } else if let Some(b) = val.as_bool() {
        JsValue::Bool(b)
    } else if let Some(i) = val.as_int() {
        JsValue::Int(i)
    } else if let Some(f) = val.as_float() {
        JsValue::Float(f)
    } else if let Some(s) = val.as_string() {
        JsValue::String(s.to_string().unwrap_or_default())
    } else {
        JsValue::String("[object]".into())
    }
}

/// Extract a `JsError` from a rquickjs error, pulling exception details
/// from the context when available.
fn convert_error(ctx: &rquickjs::Ctx<'_>, err: rquickjs::Error) -> JsError {
    let catch_val = ctx.catch();
    if catch_val.is_null() || catch_val.is_undefined() {
        return JsError {
            message: err.to_string(),
            stack: None,
        };
    }

    if let Some(obj) = catch_val.as_object() {
        let message: String = obj
            .get::<_, String>("message")
            .unwrap_or_else(|_| err.to_string());
        let stack: Option<String> = obj.get::<_, String>("stack").ok();
        JsError { message, stack }
    } else {
        // Non-object throw (e.g. `throw "string"`)
        let message = if let Some(s) = catch_val.as_string() {
            s.to_string().unwrap_or_else(|_| err.to_string())
        } else {
            err.to_string()
        };
        JsError {
            message,
            stack: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConsoleLevel;

    #[test]
    fn eval_literal() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        assert_eq!(engine.eval("1 + 2").unwrap(), JsValue::Int(3));
    }

    #[test]
    fn eval_string() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        assert_eq!(
            engine.eval("'hello'").unwrap(),
            JsValue::String("hello".into())
        );
    }

    #[test]
    fn eval_bool() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        assert_eq!(engine.eval("true").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn eval_float() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        assert_eq!(engine.eval("1.5").unwrap(), JsValue::Float(1.5));
    }

    #[test]
    fn eval_null() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        assert_eq!(engine.eval("null").unwrap(), JsValue::Null);
    }

    #[test]
    fn eval_undefined() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        assert_eq!(engine.eval("undefined").unwrap(), JsValue::Undefined);
    }

    #[test]
    fn console_log() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.eval("console.log('test')").unwrap();
        let out = engine.console_output();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].level, ConsoleLevel::Log);
        assert_eq!(out[0].message, "test");
    }

    #[test]
    fn console_levels() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.eval("console.log('a')").unwrap();
        engine.eval("console.warn('b')").unwrap();
        engine.eval("console.error('c')").unwrap();
        engine.eval("console.info('d')").unwrap();
        let out = engine.console_output();
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].level, ConsoleLevel::Log);
        assert_eq!(out[1].level, ConsoleLevel::Warn);
        assert_eq!(out[2].level, ConsoleLevel::Error);
        assert_eq!(out[3].level, ConsoleLevel::Info);
    }

    #[test]
    fn console_multiple_args() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.eval("console.log('a', 1, true)").unwrap();
        let out = engine.console_output();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].message, "a 1 true");
    }

    #[test]
    fn syntax_error() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let result = engine.eval("{");
        assert!(result.is_err());
    }

    #[test]
    fn runtime_error() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let result = engine.eval("undefined.x");
        assert!(result.is_err());
    }

    #[test]
    fn settimeout_stub() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let val = engine.eval("setTimeout(() => {}, 100)").unwrap();
        assert_eq!(val, JsValue::Int(0));
        // Verify warning was logged.
        let out = engine.console_output();
        assert!(out.iter().any(|e| e.level == ConsoleLevel::Warn));
    }

    #[test]
    fn setinterval_stub() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let val = engine.eval("setInterval(() => {}, 100)").unwrap();
        assert_eq!(val, JsValue::Int(0));
    }

    #[test]
    fn alert_stub() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.eval("alert('hi')").unwrap();
        let out = engine.console_output();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].level, ConsoleLevel::Log);
        assert_eq!(out[0].message, "hi");
    }

    #[test]
    fn take_console_output_clears() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.eval("console.log('x')").unwrap();
        let out = engine.take_console_output();
        assert_eq!(out.len(), 1);
        assert!(engine.console_output().is_empty());
    }

    #[test]
    fn eval_all_continues_after_error() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let results = engine.eval_all(&["1 + 1", "throw 'boom'", "2 + 2"]);
        assert_eq!(results.len(), 3);
        assert!(results[0].is_ok());
        assert!(results[1].is_err());
        assert!(results[2].is_ok());
    }

    #[test]
    fn state_persists_across_evals() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.eval("var x = 42").unwrap();
        assert_eq!(engine.eval("x").unwrap(), JsValue::Int(42));
    }

    #[test]
    fn with_context_register_global() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .with_context(|ctx| {
                ctx.globals().set("MY_CONST", 99)?;
                Ok(())
            })
            .unwrap();
        assert_eq!(engine.eval("MY_CONST").unwrap(), JsValue::Int(99));
    }
}
