# JavaScript Engine — Desktop API

This is the contributor reference for `oasis-js` and the DOM bindings exposed by
`oasis-browser`. For the PSP cross-compile story (pspdev toolchain, FPU mode,
hand-rolled libc shim, kernel-mode quirks) see
[`docs/javascript-engine.md`](javascript-engine.md).

## Crate at a glance

`oasis-js` wraps QuickJS-NG via the `rquickjs` bindings. The same crate runs on
desktop, WASM, UE5, and PSP — there is no per-target backend split. The crate
provides a single-threaded, non-reentrant JS context plus a small set of
host-side primitives (console, timers, fetch, storage) that the browser layer
extends with DOM bindings.

- Cargo features: `rquickjs-engine` (default, the only supported backend).
  `psp-bindgen` regenerates QuickJS C bindings at build time and is enabled
  only by the PSP backend; desktop never sets it.
- All public types live at the crate root and are re-exported from
  `crates/oasis-js/src/lib.rs`.

## Engine lifecycle

```rust
use oasis_js::JsEngine;

let engine = JsEngine::new(8 * 1024 * 1024)?; // 8 MiB heap budget
engine.set_max_exec_ms(5_000);

let value = engine.eval("1 + 2")?;       // returns JsValue::Int(3)
let _ = engine.tick_timers(16.0);        // advance pending setTimeout/setInterval

for entry in engine.take_console_output() {
    eprintln!("[js {:?}] {}", entry.level, entry.message);
}
```

Key entry points (paths are `crates/oasis-js/src/engine.rs`):

| Method | Purpose |
| --- | --- |
| `JsEngine::new(max_memory_bytes)` (engine.rs:51) | Allocate the QuickJS runtime, install console / storage / fetch / timer globals. |
| `set_max_exec_ms(ms)` (engine.rs:99) | Execution budget per guarded entry (eval, timer callback, microtask drain, guarded event dispatch). Default 5 s. See [Watchdog](#watchdog). |
| `eval(script)` (engine.rs:114) | Evaluate a single script. Returns `Result<JsValue, JsError>`. Drains the promise microtask queue (bounded) afterwards. |
| `eval_all(&[scripts])` (engine.rs:134) | Evaluate each script in document order and collect a `Vec<Result<JsValue, JsError>>` with one entry per input. A failed script does not halt the loop — subsequent scripts still run, and the returned vector preserves index alignment with the input slice. |
| `tick_timers(dt_ms)` (engine.rs:170) | Advance the timer queue by `dt_ms`, fire due callbacks (each under its own deadline), drain microtasks between callbacks. Call once per host frame. |
| `drain_microtasks()` (engine.rs:209) | Run pending promise jobs, bounded by the deadline and `MAX_MICROTASKS_PER_DRAIN`. Returns the number of jobs run. |
| `console_output()` / `take_console_output()` (engine.rs:139, 144) | Snapshot or drain the buffered console. |
| `local_storage()` (engine.rs:150) | Borrow the in-memory `localStorage` map for snapshot or restore. |
| `install_fetch_handler(Box::new(handler))` (engine.rs:156) | Install an HTTP transport. Replaces any previous handler. |
| `with_context_guarded(\|ctx\| ...)` (engine.rs:227) | Raw `Ctx` access with the watchdog armed. Use whenever the closure calls into page JS (event dispatch). |
| `with_context(\|ctx\| ...)` (engine.rs:284) | Unguarded escape hatch for raw `rquickjs::Ctx<'_>` access. Used by `oasis-browser` to register DOM globals; do **not** call page JS through it. |

There is no explicit shutdown; dropping the `JsEngine` runs the QuickJS
finalizers and releases the runtime.

## Value bridge

`JsValue` (`types.rs:16`) is the engine-agnostic surface that crosses the FFI
boundary. It collapses to one of `Undefined | Null | Bool | Int | Float |
String`. Objects and arrays appear as `JsValue::String("[object]")`. Code that
needs structured data should use `with_context` to operate on `rquickjs::Value`
directly.

`JsError` (`types.rs:104`) carries a message and an optional stack. Display
formats as `"<message>\n<stack>"` when a stack is present; non-object throws
(e.g. `throw "boom"`) coerce to a string message.

## Console buffer

`JsEngine` installs a JS `console` global with `log` / `info` / `warn` /
`error`. Each call appends a `ConsoleEntry { level, message }` to an internal
ring (`console.rs:9`). Eval errors and unhandled exceptions are also pushed at
`ConsoleLevel::Error`. The host owns drain semantics: call
`take_console_output()` per frame and forward to whatever surface is
appropriate (terminal pane, browser devtools, log file).

## Timers

`TimerQueue` (`timers.rs:63`) backs `setTimeout`, `setInterval`, `clearTimeout`,
`clearInterval`. Timers are advanced and fired only inside `tick_timers(dt_ms)`
— the engine never spawns its own thread. Microtasks are drained between
callbacks, so a timer that resolves a promise will run the `.then` continuation
before the next timer fires.

Callbacks are stored once, as functions on `globalThis.__oasis_timer_cb_<id>`,
and fired through a pre-compiled, non-writable `__oasis_fire_timer(id, repeat)`
dispatcher (`TimerQueue::tick_fired` reports `FiredTimer { id, repeat }`), so a
firing never re-parses JS source. The legacy `TimerQueue::tick`, which returns
eval strings, is kept for API compatibility.

Limits (see also [Watchdog](#watchdog)):

- Delays that are NaN, infinite or negative are treated as `0`; delays above
  `i32::MAX` ms are clamped.
- `setInterval` periods are clamped to at least `MIN_INTERVAL_MS` (4 ms), an
  approximation of the HTML spec's nested-timer clamp. An interval fires at
  most once per `tick_timers` call and is rescheduled one period after "now",
  so a long frame never causes a catch-up burst.
- At most `MAX_LIVE_TIMERS` (1000) timers may be pending. Further
  `setTimeout` / `setInterval` calls return `0`, register nothing, and a single
  warning is logged to the console until a registration succeeds again.
- A NaN / negative `dt_ms` passed to `tick_timers` is ignored (the timer clock
  does not move backwards or become NaN).

`requestAnimationFrame` is **not** implemented. Use `setInterval(fn, 16)` or
schedule from the host's frame loop.

## Watchdog

QuickJS runs on the host's thread, so any JS that doesn't return freezes the
whole OS. Every entry point that runs page JS is bounded:

| Entry point | Bound |
| --- | --- |
| `eval` / `eval_all` | Interrupt handler armed with a deadline of `max_exec_ms` (default 5 s, `set_max_exec_ms`). The trailing microtask drain shares the same deadline. |
| `tick_timers` | Each fired callback gets its own `max_exec_ms` deadline, followed by a bounded microtask drain. A runaway callback is interrupted and logged; later timers in the same tick still fire. |
| `drain_microtasks` | Fresh `max_exec_ms` deadline **and** a cap of `MAX_MICROTASKS_PER_DRAIN` (10 000) jobs. |
| `with_context_guarded` | Deadline armed for the duration of the closure. `oasis-browser` routes all DOM event dispatch (click, mouse, key, input) through it. |
| `with_context` | **Unbounded.** Only for installing globals / reading state. |

When the deadline passes, QuickJS raises an uncatchable `InternalError:
interrupted`: the current script, callback or handler unwinds (JS `try/catch`
cannot swallow it), the error is logged to the console buffer at
`ConsoleLevel::Error`, and the interrupt handler is cleared so the engine stays
usable for the next event or frame. DOM mutations the handler made before
being interrupted are kept.

The microtask cap exists because a self-perpetuating promise chain
(`function f(){ Promise.resolve().then(f) }`) is made of many tiny jobs; the
time-based interrupt only fires *inside* a job, so without the cap the chain
could spin for the whole budget. When a drain is cut off (cap or deadline) a
console warning is logged and the remaining jobs stay queued for the next
drain — a runaway chain therefore costs at most one bounded slice per drain
instead of freezing the host.

`max_exec_ms` is a per-entry budget, not a per-frame budget: a page that
schedules many slow timers can still use up to `max_exec_ms` per callback.

## Fetch

`FetchHandler` (`fetch.rs:43`) is a synchronous trait:

```rust
pub trait FetchHandler {
    fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, String>;
}
```

The default behaviour returns `error: no fetch handler installed`. Production
code installs an HTTP client via `install_fetch_handler`. `MockFetchHandler`
(`fetch.rs:80`) is provided for tests.

### How the bridge works

There is **no asynchronous response queue.** When JS calls `fetch(url, opts)`
the JS-side wrapper invokes `__oasis_fetch` synchronously; that calls
`FetchHandler::fetch` and waits for it to return. The wrapper then resolves
(or rejects) the JS-visible `Promise` immediately with the result. From the
engine's point of view, by the time `fetch()` returns to the JS caller the
network response is already in hand. The `Promise` exists only to match the
standard `fetch` shape — it is not used to defer work.

This is why `eval` / `tick_timers` do not poll a response queue: there is
nothing to poll. The handler must produce the response by the time it
returns, or fail.

> **Hazard:** `FetchHandler::fetch` is invoked on the JS eval thread.
> Calling blocking I/O inline from the handler stalls the **entire** JS
> engine — every queued microtask, every other in-flight `fetch()`
> promise, every pending timer — not just the one promise being
> resolved. The single-threaded, non-reentrant model means there is no
> background scheduler that can run other JS while one handler waits.
> Always do the actual network call off-thread and return only when the
> bytes are already in hand.

The JS-visible API matches the standard `fetch(url, opts)` shape and resolves
to a `Response` with `status`, `ok`, `headers.get(name)`, `text()`, `json()`.

## Storage

`SharedStorage` (`storage.rs:54`) is an `Rc<RefCell<LocalStorage>>` wrapping a
`BTreeMap<String, String>`. The `oasis-js` crate itself only installs
`localStorage`; it does **not** define `sessionStorage` at the engine level.
Persistence of `localStorage` is the host's job: snapshot the map on shutdown
and rehydrate on startup. There is no quota enforcement.

The `oasis-browser` layer is what actually exposes `sessionStorage` to JS, and
it does so with a **separate** backing map from `localStorage`
(`crates/oasis-browser/src/js_dom/storage.rs`: `kind: 0` = localStorage,
`kind: 1` = sessionStorage). Persistence still differs from the spec —
`sessionStorage` is page-scoped within an `oasis-browser` instance but is not
automatically cleared on navigation events the way a real browser would clear
it. Treat that as an intentional simplification, not a guarantee of
spec-compliant Web Storage semantics.

## Threading and re-entrancy

The engine is **single-threaded and non-reentrant**:

- `JsEngine` holds `Rc<RefCell<...>>` for its shared buffers (`engine.rs:34`).
  It is `!Send` and `!Sync`.
- `eval` and `tick_timers` take `&self` because the shared state hides behind
  `RefCell`. Calling either re-entrantly from inside a native callback is a
  panic.
- All JS callbacks (timer fires, fetch resolutions, event handlers) execute
  synchronously on the host's call stack inside `eval`, `tick_timers` or
  `with_context_guarded`.

If you need to drive multiple JS contexts, hold one per host thread; the
QuickJS runtime cannot migrate between threads.

## DOM bindings (oasis-browser)

DOM globals are installed by `oasis-browser` via `JsEngine::with_context` —
they are not part of `oasis-js` itself. The implementation lives in
the `crates/oasis-browser/src/js_dom/` module. The shape is "thin Rust
functions exposed as `__oasis_*` globals + JS shims that present the standard
API on top":

| File | Contents |
| --- | --- |
| `mod.rs` | Shared handle types, `install_document_global_*` entry points, inline `on*` handler registration. |
| `bindings.rs` | Node / attribute / tree / selector / classList / inline-style / navigation / `getComputedStyle` bindings. |
| `fetch.rs` | `__oasis_fetch` (CSP `connect-src` enforced). |
| `storage.rs` | `localStorage` / `sessionStorage` / `document.cookie`. |
| `serialize.rs` | `innerHTML` serialization + fragment deep-copy. |
| `canvas.rs` | `__oasis_canvas_*` (feature `canvas`). |
| `compat_shims.rs` | Site-compat helpers (reddit `togglecomment` & co.). |
| `bootstrap.js` / `canvas.js` / `compat_shims.js` | The JS halves, embedded with `include_str!`. |

### Document

| API | Source | Notes |
| --- | --- | --- |
| `document.getElementById(id)` | bootstrap.js | Returns `Element \| null`. |
| `document.createElement(tag)` | bootstrap.js | Creates a detached element. |
| `document.createTextNode(text)` | bootstrap.js | Returns a `#text` node. |
| `document.querySelector(sel)` | bootstrap.js | First match in document order. |
| `document.querySelectorAll(sel)` | bootstrap.js | Live `Element[]`. |
| `document.body` | bootstrap.js | Getter only. |
| `document.title` | bootstrap.js | Getter and setter. |
| `document.cookie` | storage.rs | Getter and setter; raw string. |
| `document.addEventListener(type, fn, opts)` | bootstrap.js | |
| `document.removeEventListener(type, fn, opts)` | bootstrap.js | |
| `document.dispatchEvent(evt)` | bootstrap.js | |

### Element

- Tree navigation: `parentElement`, `parentNode`, `firstChild`, `lastChild`,
  `childNodes`, `nextSibling`, `previousSibling`.
- Content: `textContent`, `innerHTML` (getter/setter in bootstrap.js).
- Attributes: `getAttribute`, `setAttribute`, `removeAttribute`
  (bootstrap.js).
- Mutation: `appendChild`, `removeChild`, `insertBefore`
  (bootstrap.js).
- Selectors: `querySelector`, `querySelectorAll` (bootstrap.js).
- Events: `addEventListener`, `removeEventListener`, `dispatchEvent`
  (bootstrap.js). Options accept `{capture, once, passive}` or a bare
  boolean for capture.
- `classList.add / remove / toggle / contains` (bootstrap.js).
- `style.<property>` proxy with camelCase ↔ kebab-case conversion plus
  `getPropertyValue` / `setProperty` (bootstrap.js).

Click / keydown / keyup have a fast path
(`__oasis_dispatch_*_fast`, bootstrap.js) used by the browser's input layer
to avoid a full event dispatch on hot paths.

### Window, location, history

`location.href` getter/setter, `location.assign(url)`, `location.replace(url)`,
`location.reload()`, `history.back()`, `history.forward()`. Navigation actions
are queued and consumed by the host browser layer rather than acted on
synchronously.

### Storage and fetch

`localStorage` and `sessionStorage` both mirror the standard `getItem` /
`setItem` / `removeItem` / `clear` / `key` / `length` API, but they back onto
**separate** maps inside `oasis-browser` — see the Storage section above for
the spec-deviation caveats. `fetch(url, opts)` returns a promise resolved by
the installed `FetchHandler` and yields a `Response` with `status`, `ok`,
`headers.get`, `text`, `json`.

### Canvas 2D

`canvas.getContext('2d')` exposes `fillRect`, `strokeRect`, `clearRect`,
`beginPath`, `arc`, `moveTo`, `lineTo`, `bezierCurveTo`, `quadraticCurveTo`,
`closePath`, `fill`, `stroke`, `fillText`, `save`, `restore`, plus the
`fillStyle`, `strokeStyle`, `font`, `lineWidth` setters
(`js_dom/canvas.rs` + `js_dom/canvas.js`).

### Known gaps vs. browser baseline

These are intentional — file an issue or extend `js_dom/` if you need them:

- `getComputedStyle()` — partially captured in `SharedStyles` but not exposed
  to JS.
- `getBoundingClientRect()` and the `scroll*` family.
- `requestAnimationFrame` — substitute `setInterval(fn, 16)`.

## Adding a host capability

The pattern for new bindings is the one `oasis-browser` already follows.

1. Add a Rust function in `js_dom/bindings.rs` (or the matching file) named
   `__oasis_<verb>` that takes only primitives (`i32`, `String`, `f64`,
   `Vec<i32>`) and returns a primitive or small struct convertible via rquickjs.
2. Mark the DOM dirty via `mark_dirty(&dirty)` if the call mutates the tree
   (`js_dom/mod.rs`).
3. Clone shared `Rc` handles for any move-into-closure capture.
4. Register the function during `with_context`:

   ```rust
   engine.with_context(|ctx| {
       let globals = ctx.globals();
       globals.set("__oasis_my_verb", Function::new(ctx.clone(), my_verb)?)?;
       Ok(())
   })?;
   ```

5. Wrap the raw global in JS to present the public API surface (e.g. add a
   method on `Element.prototype`).

For new fetch transports implement `FetchHandler` and call
`install_fetch_handler`. For new storage backends, swap the `SharedStorage`
contents — the trait is just a `BTreeMap` behind an `Rc<RefCell<>>`.

## Testing

Use `MockFetchHandler` for deterministic fetch responses. Timer-driven code
should advance with `tick_timers(dt)` instead of relying on wall-clock time.
The console buffer is the easiest assertion target — run a script that ends in
`console.log("ok")` and check `take_console_output()`.
