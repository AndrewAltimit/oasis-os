use std::cell::RefCell;
use std::rc::Rc;

use rquickjs::function::Rest;
use rquickjs::{Ctx, Function, Object, Result as JsResult, Value};

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

/// Install `console`, `alert`, `setTimeout`, `setInterval`, `clearTimeout`,
/// and `clearInterval` into the given JS context.
pub(crate) fn install(ctx: &Ctx<'_>, buf: ConsoleBuffer) -> JsResult<()> {
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

    // -- setTimeout / setInterval stubs --------------------------------
    let b = Rc::clone(&buf);
    globals.set(
        "setTimeout",
        Function::new(ctx.clone(), move |_args: Rest<Value<'_>>| -> i32 {
            b.borrow_mut().push(ConsoleEntry {
                level: ConsoleLevel::Warn,
                message: "setTimeout is not supported".into(),
            });
            0
        })?,
    )?;

    let b = Rc::clone(&buf);
    globals.set(
        "setInterval",
        Function::new(ctx.clone(), move |_args: Rest<Value<'_>>| -> i32 {
            b.borrow_mut().push(ConsoleEntry {
                level: ConsoleLevel::Warn,
                message: "setInterval is not supported".into(),
            });
            0
        })?,
    )?;

    // -- clearTimeout / clearInterval (no-op) --------------------------
    globals.set(
        "clearTimeout",
        Function::new(ctx.clone(), |_args: Rest<Value<'_>>| {})?,
    )?;
    globals.set(
        "clearInterval",
        Function::new(ctx.clone(), |_args: Rest<Value<'_>>| {})?,
    )?;

    Ok(())
}
