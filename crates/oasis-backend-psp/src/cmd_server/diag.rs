//! On-hardware diagnostics: `timetest` and `js <expr>`.

use core::ffi::{CStr, c_char};

use oasis_js::rquickjs::{self, qjs};

use super::log_msg;
use super::wire::send_response;

/// `timetest`: regression check that `std::time::Instant` works on real
/// hardware.
///
/// rust-psp's std overlay once had no `target_os = "psp"` arm in
/// `sys/time`, so `Instant::now` fell through to the panicking
/// `unsupported` shim. Each step is logged first so a regression shows up
/// in `eboot.log` as the last step reached.
pub(super) fn run_instant_timetest(cfd: i32) {
    log_msg("[timetest] Instant::now");
    let t0 = std::time::Instant::now();
    psp::thread::sleep_ms(10);
    let t1 = std::time::Instant::now();
    log_msg("[timetest] duration_since");
    let us = t1.duration_since(t0).as_micros();
    log_msg("[timetest] elapsed");
    let el_us = t0.elapsed().as_micros();
    let reply = format!("timetest: ok elapsed={us}us t0_elapsed={el_us}us\n");
    log_msg(reply.trim_end());
    send_response(cfd, reply.as_bytes());
}

/// `js <script>`: evaluate one expression on a fresh QuickJS-NG runtime and
/// reply `js: <value>\n` (or `js: error: <reason>\n`).
///
/// This deliberately bypasses `oasis_js::JsEngine` and drives the raw
/// QuickJS FFI with a log line between every step, so a hard crash on
/// real hardware pinpoints the exact call in `eboot.log`. Nothing is
/// shared between invocations.
///
/// ```text
/// $ echo "js 1 + 2 + 3" | nc -w 5 192.168.0.249 9293
/// js: 6
/// ```
pub(super) fn run_js_eval(cfd: i32, script: &[u8]) {
    let Ok(script) = core::str::from_utf8(script).map(str::trim) else {
        send_response(cfd, b"js: error: script is not valid UTF-8\n");
        return;
    };
    log_msg(&format!(
        "[js] eval ({} bytes) source={script:?}",
        script.len()
    ));

    log_msg("[js] Runtime::new");
    let runtime = match rquickjs::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            log_msg(&format!("[js] Runtime::new err: {e}"));
            send_response(cfd, b"js: error: runtime init\n");
            return;
        },
    };
    log_msg("[js] Context::base");
    let context = match rquickjs::Context::base(&runtime) {
        Ok(c) => c,
        Err(e) => {
            log_msg(&format!("[js] Context::base err: {e}"));
            send_response(cfd, b"js: error: context init\n");
            return;
        },
    };

    log_msg("[js] context.with");
    let mut reply: Vec<u8> = Vec::new();
    context.with(|ctx| {
        let raw_ctx = ctx.as_raw().as_ptr();
        let mut src = Vec::with_capacity(script.len() + 1);
        src.extend_from_slice(script.as_bytes());
        src.push(0);

        // SAFETY: `raw_ctx` is the live context of this `with` scope; `src`
        // is NUL-terminated and `script.len()` excludes the NUL; the file
        // name is a static C string.
        let val = unsafe {
            qjs::JS_Eval(
                raw_ctx,
                src.as_ptr() as *const c_char,
                script.len() as _,
                c"<tcp>".as_ptr(),
                qjs::JS_EVAL_TYPE_GLOBAL as i32,
            )
        };
        log_msg("[js] JS_Eval returned");

        // SAFETY: `val` is the owned result of `JS_Eval` on `raw_ctx`. It
        // is inspected, stringified (the C string is freed right after
        // being copied) and freed exactly once.
        unsafe {
            if qjs::JS_IsException(val) {
                reply.extend_from_slice(b"js: error: exception\n");
            } else {
                let cstr = qjs::JS_ToCString(raw_ctx, val);
                if cstr.is_null() {
                    reply.extend_from_slice(b"js: undefined\n");
                } else {
                    reply.extend_from_slice(b"js: ");
                    reply.extend_from_slice(CStr::from_ptr(cstr).to_bytes());
                    reply.push(b'\n');
                    qjs::JS_FreeCString(raw_ctx, cstr);
                }
            }
            qjs::JS_FreeValue(raw_ctx, val);
        }
    });
    log_msg("[js] done");
    send_response(cfd, &reply);
}
