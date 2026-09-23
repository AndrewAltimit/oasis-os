use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::{Context, Runtime};

use crate::console::{ConsoleBuffer, ConsoleEntry, ConsoleLevel};
use crate::fetch::{FetchHandler, SharedFetchHandler};
use crate::storage::{LocalStorage, SharedStorage};
use crate::timers::{FiredTimer, TimerQueue};

/// Default maximum JS execution time per eval call (5 seconds).
const DEFAULT_MAX_EXEC_MS: u64 = 5_000;

/// Maximum number of promise jobs (microtasks) run by a single drain.
///
/// A self-perpetuating chain (`function f(){ Promise.resolve().then(f) }`)
/// never empties the job queue; each job is short, so the time-based
/// interrupt alone would let it spin for the whole deadline. The cap
/// bounds the work per drain; leftover jobs run on the next drain.
pub const MAX_MICROTASKS_PER_DRAIN: usize = 10_000;

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

    /// Current per-call execution budget in milliseconds.
    pub fn max_exec_ms(&self) -> u64 {
        self.max_exec_ms
    }

    /// Evaluate a JavaScript source string and return the result.
    ///
    /// Execution is interrupted if it exceeds the configured time limit
    /// (default 5s), preventing infinite loops from freezing the host.
    /// The microtask drain that follows shares the same deadline and is
    /// additionally capped at [`MAX_MICROTASKS_PER_DRAIN`] jobs.
    pub fn eval(&self, script: &str) -> Result<JsValue, JsError> {
        let deadline = self.install_deadline();

        let result = self.context.with(|ctx| {
            let result: Result<rquickjs::Value<'_>, rquickjs::Error> = ctx.eval(script);
            // Drain microtask queue (promise callbacks) after eval.
            self.drain_jobs(&ctx, deadline);
            match result {
                Ok(val) => Ok(convert_value(&val)),
                Err(err) => Err(self.log_error(&ctx, err)),
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
    ///
    /// Each callback runs under its own execution deadline (see
    /// [`set_max_exec_ms`](Self::set_max_exec_ms)) and is followed by a
    /// bounded microtask drain. Callbacks are invoked through a
    /// pre-compiled dispatcher, so firing a timer never re-parses JS
    /// source.
    pub fn tick_timers(&self, dt_ms: f64) -> usize {
        let fired = self.timer_queue.borrow_mut().tick_fired(dt_ms);
        let count = fired.len();
        for timer in fired {
            self.fire_timer(timer);
        }
        // Drain the promise microtask queue after timer callbacks.
        self.drain_microtasks();
        count
    }

    /// Invoke one fired timer's stored callback under a deadline.
    /// Exceptions are logged to the console buffer.
    fn fire_timer(&self, timer: FiredTimer) {
        let deadline = self.install_deadline();
        self.context.with(|ctx| {
            let res: rquickjs::Result<()> = ctx
                .globals()
                .get::<_, rquickjs::Function>(crate::console::FIRE_TIMER_FN)
                .and_then(|f| f.call((timer.id, timer.repeat)));
            if let Err(err) = res {
                self.log_error(&ctx, err);
            }
            self.drain_jobs(&ctx, deadline);
        });
        self.runtime.set_interrupt_handler(None);
    }

    /// Execute pending microtasks (promise continuations).
    ///
    /// QuickJS buffers resolved-promise `.then()` callbacks internally.
    /// Call this after any JS execution that may have created or
    /// resolved promises to ensure they run synchronously.
    ///
    /// The drain is bounded: it stops after [`MAX_MICROTASKS_PER_DRAIN`]
    /// jobs or when the execution deadline (see
    /// [`set_max_exec_ms`](Self::set_max_exec_ms)) passes, whichever
    /// comes first; the interrupt handler also aborts a single job that
    /// runs past the deadline. Returns the number of jobs executed.
    pub fn drain_microtasks(&self) -> usize {
        let deadline = self.install_deadline();
        let n = self.context.with(|ctx| self.drain_jobs(&ctx, deadline));
        self.runtime.set_interrupt_handler(None);
        n
    }

    /// Run a closure with access to the raw rquickjs context under the
    /// execution watchdog.
    ///
    /// Identical to [`with_context`](Self::with_context) except that the
    /// time-based interrupt handler (see
    /// [`set_max_exec_ms`](Self::set_max_exec_ms)) is armed for the
    /// duration of `f`, so JS invoked from inside `f` (event handlers,
    /// callbacks) cannot hang the host: a runaway handler is aborted with
    /// an uncatchable `InternalError: interrupted`. JS exceptions that
    /// escape `f` are logged to the console buffer and returned as
    /// `Err`.
    pub fn with_context_guarded<R, F>(&self, f: F) -> Result<R, JsError>
    where
        F: FnOnce(rquickjs::Ctx<'_>) -> rquickjs::Result<R>,
    {
        self.install_deadline();
        let result = self
            .context
            .with(|ctx| f(ctx.clone()).map_err(|e| self.log_error(&ctx, e)));
        self.runtime.set_interrupt_handler(None);
        result
    }

    /// Arm the interrupt handler with a deadline `max_exec_ms` from now
    /// and return that deadline.
    fn install_deadline(&self) -> Instant {
        let deadline = Instant::now() + Duration::from_millis(self.max_exec_ms);
        self.runtime
            .set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
        deadline
    }

    /// Run pending jobs until the queue is empty, the job cap is hit, or
    /// `deadline` passes. Logs a console warning when cut off.
    fn drain_jobs(&self, ctx: &rquickjs::Ctx<'_>, deadline: Instant) -> usize {
        let mut n = 0;
        while ctx.execute_pending_job() {
            n += 1;
            if n >= MAX_MICROTASKS_PER_DRAIN || Instant::now() >= deadline {
                // Remaining jobs (if any) stay queued for the next drain.
                self.console_buf.borrow_mut().push(ConsoleEntry {
                    level: ConsoleLevel::Warn,
                    message: format!(
                        "microtask drain cut off after {n} jobs                          (runaway promise chain?)"
                    ),
                });
                break;
            }
        }
        n
    }

    /// Convert an rquickjs error to a [`JsError`] and log it to the
    /// console buffer at error level.
    fn log_error(&self, ctx: &rquickjs::Ctx<'_>, err: rquickjs::Error) -> JsError {
        let js_err = convert_error(ctx, err);
        self.console_buf.borrow_mut().push(ConsoleEntry {
            level: ConsoleLevel::Error,
            message: js_err.to_string(),
        });
        js_err
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

    // -- Watchdog tests --

    #[test]
    fn guarded_context_interrupts_infinite_loop() {
        let mut engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.set_max_exec_ms(50);
        engine.eval("function spin() { while (true) {} }").unwrap();
        let start = Instant::now();
        let result = engine.with_context_guarded(|ctx| {
            let f: rquickjs::Function = ctx.globals().get("spin")?;
            f.call::<_, ()>(())
        });
        assert!(result.is_err(), "runaway handler must be interrupted");
        assert!(
            start.elapsed() < Duration::from_millis(2_000),
            "took {:?}",
            start.elapsed()
        );
        // The interrupt handler is cleared afterwards: the engine is
        // still usable and not permanently interrupted.
        assert_eq!(engine.eval("1 + 1").unwrap(), JsValue::Int(2));
    }

    #[test]
    fn guarded_context_passes_through_results() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let v = engine
            .with_context_guarded(|ctx| ctx.eval::<i32, _>("6 * 7"))
            .unwrap();
        assert_eq!(v, 42);
    }

    #[test]
    fn runaway_promise_chain_is_cut_off() {
        let mut engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.set_max_exec_ms(2_000);
        let start = Instant::now();
        // Never-ending microtask chain: each job queues the next.
        engine
            .eval(
                "var __n = 0; \
                 function f() { __n++; Promise.resolve().then(f); } \
                 f();",
            )
            .unwrap();
        assert!(start.elapsed() < Duration::from_millis(3_000));
        let n = match engine.eval("__n").unwrap() {
            JsValue::Int(n) => n as usize,
            other => panic!("unexpected {other:?}"),
        };
        assert!(n <= MAX_MICROTASKS_PER_DRAIN + 1, "ran {n} jobs");
        assert!(
            engine
                .console_output()
                .iter()
                .any(|e| e.level == ConsoleLevel::Warn && e.message.contains("microtask")),
            "cut-off should be reported"
        );
        // Explicit drains are bounded too.
        let ran = engine.drain_microtasks();
        assert!(ran <= MAX_MICROTASKS_PER_DRAIN, "drained {ran}");
    }

    #[test]
    fn nan_interval_does_not_spin() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval(
                "var __ticks = 0; \
                 setInterval(function(){ __ticks++; }, NaN); \
                 setInterval(function(){ __ticks++; }, -1); \
                 setInterval(function(){ __ticks++; }, 0);",
            )
            .unwrap();
        // One huge frame: every interval fires at most once.
        assert_eq!(engine.tick_timers(10_000.0), 3);
        assert_eq!(engine.eval("__ticks").unwrap(), JsValue::Int(3));
        // Below the 4 ms clamp nothing fires.
        assert_eq!(engine.tick_timers(1.0), 0);
        assert_eq!(engine.tick_timers(3.0), 3);
    }

    #[test]
    fn timer_cap_refuses_extra_timers() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let v = engine
            .eval(
                "var ids = []; \
                 for (var i = 0; i < 1500; i++) ids.push(setTimeout(function(){}, 1e6)); \
                 ids.filter(function(x){ return x === 0; }).length",
            )
            .unwrap();
        assert_eq!(v, JsValue::Int(500));
        let warns = engine
            .console_output()
            .iter()
            .filter(|e| e.level == ConsoleLevel::Warn)
            .count();
        assert_eq!(warns, 1, "cap warning is emitted once");
        // Refused registrations must not leak callback globals.
        assert_eq!(
            engine.eval("typeof globalThis.__oasis_timer_cb_0").unwrap(),
            JsValue::String("undefined".into())
        );
    }

    #[test]
    fn runaway_timer_callback_is_interrupted() {
        let mut engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.set_max_exec_ms(50);
        engine
            .eval("setTimeout(function(){ while(true){} }, 0); var __after = 0;")
            .unwrap();
        engine
            .eval("setTimeout(function(){ __after = 1; }, 0);")
            .unwrap();
        let start = Instant::now();
        assert_eq!(engine.tick_timers(0.0), 2);
        assert!(start.elapsed() < Duration::from_millis(2_000));
        // The second timer still ran after the first was interrupted.
        assert_eq!(engine.eval("__after").unwrap(), JsValue::Int(1));
        assert!(
            engine
                .console_output()
                .iter()
                .any(|e| e.level == ConsoleLevel::Error),
            "interrupt is logged"
        );
    }

    #[test]
    fn timer_callback_exception_logged_and_global_cleaned() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval("var __tid = setTimeout(function(){ throw new Error('kaboom'); }, 0);")
            .unwrap();
        engine.tick_timers(0.0);
        assert!(
            engine
                .console_output()
                .iter()
                .any(|e| e.level == ConsoleLevel::Error && e.message.contains("kaboom"))
        );
        assert_eq!(
            engine
                .eval("typeof globalThis['__oasis_timer_cb_' + __tid]")
                .unwrap(),
            JsValue::String("undefined".into())
        );
    }

    #[test]
    fn fire_timer_dispatcher_is_not_overwritable() {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval(
                "try { globalThis.__oasis_fire_timer = function(){}; } catch (e) {} \
                 setTimeout(function(){ console.log('ran'); }, 0);",
            )
            .unwrap();
        engine.tick_timers(0.0);
        assert!(engine.console_output().iter().any(|e| e.message == "ran"));
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
