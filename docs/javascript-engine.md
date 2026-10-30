# JavaScript Engine (`oasis-js`)

Single-backend QuickJS-NG integration via `rquickjs`. Used on every
target (desktop / WASM / UE5 / PSP).

The DOM bindings themselves are documented in the "JavaScript DOM
bindings" section of [`browser-engine.md`](browser-engine.md); this
file is about the engine plumbing — the console API, script
execution, and especially the PSP cross-compile arrangement.

## APIs available to page scripts

- **`console`** — `log`, `warn`, `error`, `info`.
- **Inline `<script>` execution.**
- **Timers** — `setTimeout`, `setInterval`.
- **Storage** — `localStorage` (persistent across navigations),
  `sessionStorage`.
- **Cookies** — `document.cookie` getter/setter.
- **History + location** — `history.pushState`, `history.replaceState`,
  `window.location.assign/replace/reload`.
- **Retained engine** with event dispatch (click, keydown, keyup,
  mousedown, mouseup, mousemove) via `__oasis_dispatch_with_bubbling`
  for three-phase capture/target/bubble dispatch.
  `addEventListener({once, capture, passive})`, detail properties
  (`clientX`, `clientY`, `key`, `code`), `stopPropagation`,
  `preventDefault`.

## Execution and memory limits

The browser builds each page's engine from two `BrowserConfig` fields
(full details in [`oasis-js.md` §Watchdog](oasis-js.md#watchdog)):

| Field | Desktop | PSP | Meaning |
| --- | --- | --- | --- |
| `js_max_exec_ms` | 2 000 ms | 500 ms | Limit per inline script and per event dispatch, and the budget of each frame's timer tick (timers still due when it runs out wait for the next frame). Runaway JS gets an uncatchable `InternalError: interrupted`. |
| `js_memory_limit_bytes` | 64 MiB | 8 MiB | QuickJS heap limit; allocations past it throw `InternalError: out of memory`. |

The heap limit is real on every target, including PSP. The workspace
enables `rquickjs/rust-alloc` (all JS allocations go through Rust's
global allocator, on PSP the rust-psp heap), and rquickjs' docs say
`set_memory_limit` is a no-op then. That is out of date for QuickJS-NG,
which checks `malloc_limit` in `js_malloc_rt` itself, before it calls
the allocator. `oasis-js` pins this with the `memory_limit_is_enforced`
test, so no counting allocator is needed. The few buffers QuickJS
allocates straight from libc (for example through
`dbuf_default_realloc`, served on PSP by the shim below) are outside
the limit.

## PSP build

The PSP backend ships the same QuickJS-NG engine as desktop — the
earlier pure-Rust `boa_engine` PSP fallback has been retired. Four
knots to be aware of:

1. **`cc` crate is wired to pspdev's cross toolchain.**
   `CC_mipsel_sony_psp_std=/opt/pspdev/bin/psp-gcc` in
   `crates/oasis-backend-psp/.cargo/config.toml` makes QuickJS's C
   sources compile through pspdev.

2. **`-msingle-float` is mandatory on the C side.** PSP Allegrex has
   a single-precision FPU only and rust-psp's target spec declares
   `"features": "+single-float"`. Compiling QuickJS C with
   `-mdouble-float` links successfully but crashes on real hardware
   the first time `JS_Eval` reaches `dtoa`, because the Rust and C
   halves disagree about how double-precision helpers are called.
   PPSSPP silently fixes this up; real hardware does not. This was
   the primary blocker during bring-up.

3. **Final link through `psp-ld`**, not `rust-lld`. GCC 15 /
   binutils 2.43 emit a `.symtab` layout that rust-lld rejects with
   "invalid binding: 0". The workaround lives in `.cargo/config.toml`:
   `linker = /opt/pspdev/bin/psp-ld`, `linker-flavor = gnu` under
   `-Z unstable-options`, replayed target-spec pre-link args, plus a
   supplementary link script (`tools/psp-linkscript.ld`) that
   synthesises `_gp` (rust-psp's target script omits it; rust-lld
   computes `_gp` internally).

4. **No pspdev `libc.a` / `libm.a`.** Those archives are
   eabi32/msingle-float/abicalls and can't merge with Rust's
   o32/mdouble-float/non-abicalls code. Instead
   `crates/oasis-backend-psp/src/quickjs_shim.rs` provides the
   ~40 libc/libm symbols QuickJS references: math via the `libm`
   crate; string/memory routines hand-written; `calloc`/`realloc`
   forwarding to libpsp's `malloc`/`free` with libpsp's size-header
   decoding; non-variadic stdio stubs; a 256-byte `_impure_ptr`
   static; RTC-backed `clock_gettime` family.

## Lazy init on PSP

`BrowserState::widget: Option<BrowserWidget>` in
`crates/oasis-backend-psp/src/app_states.rs` means the JS engine is
only constructed the first time the user opens the browser app — so
boot-time cost is zero. DOM bindings are shared code with the other
backends (`oasis-browser/src/js_dom/`) enabled via the `javascript`
feature.
