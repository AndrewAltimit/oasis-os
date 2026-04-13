use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use rquickjs::{Context, Runtime};

use crate::console::{ConsoleBuffer, ConsoleEntry, ConsoleLevel};
use crate::fetch::{FetchHandler, SharedFetchHandler};
use crate::storage::{LocalStorage, SharedStorage};
use crate::timers::TimerQueue;

/// Default maximum JS execution time per eval call (5 seconds).
const DEFAULT_MAX_EXEC_MS: u64 = 5_000;

// `JsValue` and `JsError` live in `crate::types` so the boa-backed
// engine can return the same types without pulling in rquickjs. The
// re-export here keeps existing `use crate::engine::{JsValue, JsError}`
// paths working without churn inside this file.
pub use crate::types::{JsError, JsValue};

/// JavaScript engine wrapping a QuickJS-NG runtime and context.
///
/// Provides `eval` / `eval_all` for executing scripts, and
/// `console_output` / `take_console_output` for reading buffered
/// `console.log` (etc.) output.
pub struct JsEngine {
    runtime: Runtime,
    context: Context,
    console_buf: ConsoleBuffer,
    timer_queue: Rc<RefCell<TimerQueue>>,
    storage: SharedStorage,
    fetch_handler: SharedFetchHandler,
    /// Maximum execution time per eval call in milliseconds.
    max_exec_ms: u64,
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
        let timer_queue = Rc::new(RefCell::new(TimerQueue::new()));
        let storage: SharedStorage = Rc::new(RefCell::new(LocalStorage::new()));
        let fetch_handler: SharedFetchHandler = Rc::new(RefCell::new(None));

        let buf = Rc::clone(&console_buf);
        let tq = Rc::clone(&timer_queue);
        let st = Rc::clone(&storage);
        let fh = Rc::clone(&fetch_handler);
        context
            .with(|ctx| -> rquickjs::Result<()> {
                crate::console::install(&ctx, buf, tq)?;
                crate::storage::install(&ctx, st)?;
                crate::fetch::install(&ctx, fh)?;
                Ok(())
            })
            .map_err(|e| JsError {
                message: format!("failed to install globals: {e}"),
                stack: None,
            })?;

        Ok(Self {
            runtime,
            context,
            console_buf,
            timer_queue,
            storage,
            fetch_handler,
            max_exec_ms: DEFAULT_MAX_EXEC_MS,
        })
    }

    /// Set the maximum execution time per eval call in milliseconds.
    ///
    /// Scripts exceeding this limit are interrupted with an error.
    /// Default is 5000ms (5 seconds).
    pub fn set_max_exec_ms(&mut self, ms: u64) {
        self.max_exec_ms = ms;
    }

    /// Evaluate a JavaScript source string and return the result.
    ///
    /// Execution is interrupted if it exceeds the configured time limit
    /// (default 5s), preventing infinite loops from freezing the host.
    pub fn eval(&self, script: &str) -> Result<JsValue, JsError> {
        // Install a time-based interrupt handler.
        let deadline = Instant::now() + std::time::Duration::from_millis(self.max_exec_ms);
        self.runtime
            .set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));

        let result = self.context.with(|ctx| {
            let result: Result<rquickjs::Value<'_>, rquickjs::Error> = ctx.eval(script);
            // Drain microtask queue (promise callbacks) after eval.
            while ctx.execute_pending_job() {}
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
        });

        // Clear the interrupt handler after execution.
        self.runtime.set_interrupt_handler(None);
        result
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

    /// Return a shared reference to the in-memory `localStorage` backing
    /// store.  Useful for snapshotting or persisting storage externally.
    pub fn local_storage(&self) -> &SharedStorage {
        &self.storage
    }

    /// Install a [`FetchHandler`] that will service `fetch()` calls from
    /// JavaScript.  Replaces any previously installed handler.
    pub fn install_fetch_handler(&self, handler: Box<dyn FetchHandler>) {
        *self.fetch_handler.borrow_mut() = Some(handler);
    }

    /// Advance timers by `dt_ms` and execute any callbacks that fire.
    ///
    /// Call this once per frame from the host (e.g. browser widget
    /// tick).  Returns the number of callbacks that fired.
    pub fn tick_timers(&self, dt_ms: f64) -> usize {
        let callbacks = self.timer_queue.borrow_mut().tick(dt_ms);
        let count = callbacks.len();
        for cb in callbacks {
            // Errors in timer callbacks are logged to the console
            // buffer by `eval`, so we can ignore them here.
            let _ = self.eval(&cb);
        }
        // Drain the promise microtask queue after timer callbacks.
        self.drain_microtasks();
        count
    }

    /// Execute all pending microtasks (promise continuations).
    ///
    /// QuickJS buffers resolved-promise `.then()` callbacks internally.
    /// Call this after any JS execution that may have created or
    /// resolved promises to ensure they run synchronously.
    pub fn drain_microtasks(&self) {
        self.context.with(|ctx| while ctx.execute_pending_job() {});
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
    fn settimeout_returns_id() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let val = engine.eval("setTimeout(() => {}, 100)").unwrap();
        // Timer IDs start at 1.
        assert_eq!(val, JsValue::Int(1));
        // No warning — stubs are gone.
        let out = engine.console_output();
        assert!(!out.iter().any(|e| e.level == ConsoleLevel::Warn));
    }

    #[test]
    fn setinterval_returns_id() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let val = engine.eval("setInterval(() => {}, 100)").unwrap();
        assert_eq!(val, JsValue::Int(1));
    }

    #[test]
    fn settimeout_fires_on_tick() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval("setTimeout(function(){ console.log('fired'); }, 50)")
            .unwrap();
        // Not enough time yet.
        engine.tick_timers(30.0);
        let out = engine.take_console_output();
        assert!(
            !out.iter().any(|e| e.message == "fired"),
            "should not fire before delay"
        );

        // Now exceed the delay.
        engine.tick_timers(30.0);
        let out = engine.take_console_output();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].message, "fired");
    }

    #[test]
    fn setinterval_fires_multiple() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval(
                "var _iv_count = 0; \
                 setInterval(function(){ \
                     _iv_count++; \
                     console.log('tick' + _iv_count); \
                 }, 100)",
            )
            .unwrap();

        engine.tick_timers(100.0);
        let out = engine.take_console_output();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].message, "tick1");

        engine.tick_timers(100.0);
        let out = engine.take_console_output();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].message, "tick2");

        engine.tick_timers(100.0);
        let out = engine.take_console_output();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].message, "tick3");
    }

    #[test]
    fn cleartimeout_prevents_fire() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval(
                "var _tid = setTimeout(\
                     function(){ console.log('nope'); }, 50); \
                 clearTimeout(_tid);",
            )
            .unwrap();
        engine.tick_timers(100.0);
        let out = engine.take_console_output();
        assert!(
            !out.iter().any(|e| e.message == "nope"),
            "cleared timeout must not fire"
        );
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

    #[test]
    fn eval_object_returns_string() {
        // Objects are converted to String("[object]") by our JsValue mapping.
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let val = engine.eval("({})").unwrap();
        assert_eq!(val, JsValue::String("[object]".into()));
    }

    #[test]
    fn eval_array_returns_string() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let val = engine.eval("[1,2,3]").unwrap();
        assert_eq!(val, JsValue::String("[object]".into()));
    }

    #[test]
    fn eval_large_integer() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        assert_eq!(engine.eval("2147483647").unwrap(), JsValue::Int(i32::MAX));
    }

    #[test]
    fn eval_negative_integer() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        assert_eq!(engine.eval("-42").unwrap(), JsValue::Int(-42));
    }

    #[test]
    fn console_log_no_args() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.eval("console.log()").unwrap();
        let out = engine.console_output();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].message, "");
    }

    #[test]
    fn clear_timeout_noop() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let val = engine.eval("clearTimeout(0)").unwrap();
        assert_eq!(val, JsValue::Undefined);
    }

    #[test]
    fn js_error_display() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let err = engine.eval("throw new Error('oops')").unwrap_err();
        assert!(err.to_string().contains("oops"));
    }

    #[test]
    fn js_error_is_error_trait() {
        let err = JsError {
            message: "test".into(),
            stack: None,
        };
        let dyn_err: &dyn std::error::Error = &err;
        assert!(!dyn_err.to_string().is_empty());
    }

    #[test]
    fn function_definition_and_call() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.eval("function add(a, b) { return a + b; }").unwrap();
        assert_eq!(engine.eval("add(3, 4)").unwrap(), JsValue::Int(7));
    }

    #[test]
    fn closures_work() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval("var counter = (function() { var n = 0; return function() { return ++n; }; })()")
            .unwrap();
        assert_eq!(engine.eval("counter()").unwrap(), JsValue::Int(1));
        assert_eq!(engine.eval("counter()").unwrap(), JsValue::Int(2));
    }

    // -- Promise / microtask tests --

    #[test]
    fn promise_then_runs_synchronously() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval(
                "Promise.resolve(42).then(function(v) { \
                 console.log('resolved ' + v); \
                 })",
            )
            .unwrap();
        let out = engine.console_output();
        assert!(
            out.iter().any(|e| e.message == "resolved 42"),
            "promise .then should run after eval drains microtasks"
        );
    }

    #[test]
    fn promise_chain() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval(
                "Promise.resolve(1)\
                 .then(function(v) { return v + 1; })\
                 .then(function(v) { console.log('chain ' + v); })",
            )
            .unwrap();
        let out = engine.console_output();
        assert!(
            out.iter().any(|e| e.message == "chain 2"),
            "chained promises should execute"
        );
    }

    #[test]
    fn promise_catch() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval(
                "Promise.reject('err')\
                 .catch(function(e) { console.log('caught ' + e); })",
            )
            .unwrap();
        let out = engine.console_output();
        assert!(
            out.iter().any(|e| e.message == "caught err"),
            "promise .catch should run"
        );
    }

    #[test]
    fn infinite_loop_interrupted() {
        let mut engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.set_max_exec_ms(100); // 100ms limit
        let start = std::time::Instant::now();
        let result = engine.eval("while(true) {}");
        let elapsed = start.elapsed();
        assert!(result.is_err(), "infinite loop should be interrupted");
        assert!(
            elapsed.as_millis() < 2000,
            "should interrupt within reasonable time, took {}ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn normal_code_unaffected_by_limit() {
        let mut engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.set_max_exec_ms(5000);
        assert_eq!(engine.eval("1 + 2").unwrap(), JsValue::Int(3));
        // Engine is still usable after a previous eval.
        assert_eq!(engine.eval("3 + 4").unwrap(), JsValue::Int(7));
    }
}
