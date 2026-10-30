//! Boot-time QuickJS bring-up probe, triggered by the `ms0:/JSTEST`
//! sentinel (see `psp_main`).
//!
//! Every step writes a log line to `eboot.log` before and after the call,
//! so a crash inside any of them leaves the name of the last-attempted
//! call on disk. The QuickJS calls go through the raw FFI
//! (`oasis_js::rquickjs::qjs::*`) to keep the Rust glue out of the failure
//! surface.

use core::ffi::c_char;

use oasis_js::rquickjs::{self, qjs};

fn log(step: &str) {
    oasis_backend_psp::vlog_force(&format!("[JSTEST] {step}"));
}

fn free_kb() -> i32 {
    oasis_backend_psp::heap_kb().0
}

/// Run the probes. Returns early (after logging) if the runtime or
/// context cannot be created.
pub(crate) fn run() {
    log("Runtime::new begin");
    let runtime = match rquickjs::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            log(&format!("Runtime::new err: {e}"));
            return;
        },
    };
    log("Context::base begin");
    let context = match rquickjs::Context::base(&runtime) {
        Ok(c) => c,
        Err(e) => {
            log(&format!("Context::base err: {e}"));
            return;
        },
    };

    log("context.with begin");
    context.with(|ctx| {
        let raw_ctx = ctx.as_raw().as_ptr();
        log(&format!("raw ctx ptr = {raw_ctx:?}"));

        log("JS_GetGlobalObject begin");
        // SAFETY: `raw_ctx` is the live context of this `with` scope; the
        // returned value is owned and freed exactly once.
        unsafe {
            let gval = qjs::JS_GetGlobalObject(raw_ctx);
            log("JS_GetGlobalObject ok");
            qjs::JS_FreeValue(raw_ctx, gval);
        }
        log("JS_FreeValue(global) ok");

        let fname = c"<jstest>".as_ptr();
        let global_flag = qjs::JS_EVAL_TYPE_GLOBAL as i32;
        let compile_only = global_flag | qjs::JS_EVAL_FLAG_COMPILE_ONLY as i32;

        // Probe A: JS_Eval2 takes a JSEvalOptions pointer instead of the
        // 5-argument form JS_Eval uses. All 4 args fit in $a0-$a3 (no
        // stack-passed args), so if this works and JS_Eval doesn't, the
        // crash is the MIPS o32 shadow-space ABI mismatch between rustc
        // and GCC for 5-arg calls.
        log(&format!(
            "A: free={}KB, JS_Eval2 \"0\" (4-arg form) begin",
            free_kb()
        ));
        let mut opts = qjs::JSEvalOptions {
            // QuickJS rejects version != 1 with JS_ThrowInternalError,
            // whose variadic error formatter path crashes under our stub
            // `vsnprintf` on real hardware.
            version: 1,
            eval_flags: compile_only,
            filename: fname,
            line_num: 1,
        };
        // SAFETY: as above; the source is a static NUL-terminated string of
        // the stated length and `opts` outlives the call.
        unsafe {
            let va = qjs::JS_Eval2(raw_ctx, c"0".as_ptr(), 1, &mut opts as *mut _);
            log("A: JS_Eval2 returned");
            qjs::JS_FreeValue(raw_ctx, va);
        }
        log("A: freed");

        // Probe B: compile-only "0" (lexer + parser + codegen, no
        // execution). Probe C: full eval of "0" (adds the interpreter).
        // Probes D-F: previously-passing PPSSPP cases, for reference.
        let probes: [(&str, &[u8], i32); 5] = [
            ("B=\"0\" COMPILE_ONLY", b"0\0", compile_only),
            ("C=\"0\" FULL", b"0\0", global_flag),
            ("D=1+2+3", b"1+2+3\0", global_flag),
            ("E='hi'+'!'", b"'hi'+'!'\0", global_flag),
            (
                "F=(fn(){return 42})()",
                b"(function(){return 42})()\0",
                global_flag,
            ),
        ];
        for (label, src, flags) in probes {
            log(&format!("{label}: free={}KB, begin", free_kb()));
            // SAFETY: `src` is NUL-terminated and the length passed
            // excludes the NUL; `fname` is a static C string; the result
            // is freed exactly once.
            unsafe {
                let val = qjs::JS_Eval(
                    raw_ctx,
                    src.as_ptr() as *const c_char,
                    (src.len() - 1) as _,
                    fname,
                    flags,
                );
                log(&format!("{label}: returned"));
                qjs::JS_FreeValue(raw_ctx, val);
            }
            log(&format!("{label}: freed"));
        }
    });
    log("context.with ok");
}
