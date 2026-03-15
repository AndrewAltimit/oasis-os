use std::cell::RefCell;
use std::rc::Rc;

use rquickjs::function::Rest;
use rquickjs::{Ctx, Function, Object, Result as JsResult, Value};

use crate::timers::TimerQueue;

/// Severity level of a console message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleLevel {
    Log,
    Info,
    Warn,
    Error,
}

/// A single console output entry produced by JavaScript code.
#[derive(Debug, Clone)]
pub struct ConsoleEntry {
    pub level: ConsoleLevel,
    pub message: String,
}

/// Shared buffer for console output, accessible from JS closures and Rust.
pub(crate) type ConsoleBuffer = Rc<RefCell<Vec<ConsoleEntry>>>;

/// Format a single JS value as a human-readable string.
fn fmt_value(val: &Value<'_>) -> String {
    if val.is_undefined() {
        "undefined".into()
    } else if val.is_null() {
        "null".into()
    } else if let Some(b) = val.as_bool() {
        b.to_string()
    } else if let Some(i) = val.as_int() {
        i.to_string()
    } else if let Some(f) = val.as_float() {
        if f.fract() == 0.0 && f.abs() < i64::MAX as f64 {
            format!("{}", f as i64)
        } else {
            format!("{f}")
        }
    } else if let Some(s) = val.as_string() {
        s.to_string().unwrap_or_default()
    } else {
        "[object]".into()
    }
}

/// Format variadic JS args as a space-separated string.
fn fmt_args(args: &Rest<Value<'_>>) -> String {
    args.0
        .iter()
        .map(|v| fmt_value(v))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Shared timer queue reference for closures.
pub(crate) type SharedTimerQueue = Rc<RefCell<TimerQueue>>;

/// Install `console`, `alert`, `setTimeout`, `setInterval`,
/// `clearTimeout`, and `clearInterval` into the given JS context.
pub(crate) fn install(
    ctx: &Ctx<'_>,
    buf: ConsoleBuffer,
    timer_queue: SharedTimerQueue,
) -> JsResult<()> {
    let globals = ctx.globals();

    // -- console object ------------------------------------------------
    let console = Object::new(ctx.clone())?;

    let b = Rc::clone(&buf);
    console.set(
        "log",
        Function::new(ctx.clone(), move |args: Rest<Value<'_>>| {
            b.borrow_mut().push(ConsoleEntry {
                level: ConsoleLevel::Log,
                message: fmt_args(&args),
            });
        })?,
    )?;

    let b = Rc::clone(&buf);
    console.set(
        "info",
        Function::new(ctx.clone(), move |args: Rest<Value<'_>>| {
            b.borrow_mut().push(ConsoleEntry {
                level: ConsoleLevel::Info,
                message: fmt_args(&args),
            });
        })?,
    )?;

    let b = Rc::clone(&buf);
    console.set(
        "warn",
        Function::new(ctx.clone(), move |args: Rest<Value<'_>>| {
            b.borrow_mut().push(ConsoleEntry {
                level: ConsoleLevel::Warn,
                message: fmt_args(&args),
            });
        })?,
    )?;

    let b = Rc::clone(&buf);
    console.set(
        "error",
        Function::new(ctx.clone(), move |args: Rest<Value<'_>>| {
            b.borrow_mut().push(ConsoleEntry {
                level: ConsoleLevel::Error,
                message: fmt_args(&args),
            });
        })?,
    )?;

    globals.set("console", console)?;

    // -- alert() -------------------------------------------------------
    let b = Rc::clone(&buf);
    globals.set(
        "alert",
        Function::new(ctx.clone(), move |args: Rest<Value<'_>>| {
            b.borrow_mut().push(ConsoleEntry {
                level: ConsoleLevel::Log,
                message: fmt_args(&args),
            });
        })?,
    )?;

    // -- Timer internals (Rust side) ------------------------------------
    // Expose low-level helpers that only accept typed primitives
    // (no Value<'_> + Ctx<'_> mixing). The JS wrappers below store
    // the callback on globalThis themselves.

    let tq = Rc::clone(&timer_queue);
    globals.set(
        "__oasis_add_timeout",
        Function::new(ctx.clone(), move |delay: f64| -> i32 {
            let mut q = tq.borrow_mut();
            let id = q.add_timeout(String::new(), delay);
            let gn = format!("__oasis_timer_cb_{id}");
            if let Some(t) = q.timers_mut().iter_mut().find(|t| t.id() == id) {
                t.set_callback_global(gn);
            }
            id
        })?,
    )?;

    let tq = Rc::clone(&timer_queue);
    globals.set(
        "__oasis_add_interval",
        Function::new(ctx.clone(), move |delay: f64| -> i32 {
            let mut q = tq.borrow_mut();
            let id = q.add_interval(String::new(), delay);
            let gn = format!("__oasis_timer_cb_{id}");
            if let Some(t) = q.timers_mut().iter_mut().find(|t| t.id() == id) {
                t.set_callback_global(gn);
            }
            id
        })?,
    )?;

    let tq = Rc::clone(&timer_queue);
    globals.set(
        "__oasis_clear_timer",
        Function::new(ctx.clone(), move |id: i32| {
            tq.borrow_mut().clear(id);
        })?,
    )?;

    // -- JS wrappers for setTimeout / setInterval / clear* ------------
    // Callback storage happens on the JS side so we avoid the
    // rquickjs lifetime issue of mixing Ctx<'a> with Value<'b>.
    ctx.eval::<(), _>(
        br#"
globalThis.setTimeout = function(cb, delay) {
    var d = (typeof delay === 'number') ? delay : 0;
    var id = __oasis_add_timeout(d);
    var gn = '__oasis_timer_cb_' + id;
    if (typeof cb === 'function') {
        globalThis[gn] = cb;
    } else if (typeof cb === 'string') {
        globalThis[gn] = new Function(cb);
    }
    return id;
};
globalThis.setInterval = function(cb, delay) {
    var d = (typeof delay === 'number') ? delay : 0;
    var id = __oasis_add_interval(d);
    var gn = '__oasis_timer_cb_' + id;
    if (typeof cb === 'function') {
        globalThis[gn] = cb;
    } else if (typeof cb === 'string') {
        globalThis[gn] = new Function(cb);
    }
    return id;
};
globalThis.clearTimeout = function(id) {
    __oasis_clear_timer(id);
};
globalThis.clearInterval = function(id) {
    __oasis_clear_timer(id);
};
"#,
    )?;

    Ok(())
}
