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
| `local_storage()` (engine.rs:150) | Borrow the in-memory `localStorage` area (`LocalStorage`, 5 MiB quota) for snapshot or restore. |
| `install_fetch_handler(Box::new(handler))` (engine.rs:156) | Install an HTTP transport. Replaces any previous handler. See [Fetch](#fetch). |
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

`FetchHandler` (`fetch.rs`) is a synchronous trait:

```rust
pub trait FetchHandler {
    fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, String>;
}
```

`FetchRequest` carries the URL exactly as passed to `fetch()` (possibly
relative — resolving it is the handler's job), the method (standard methods
upper-cased), lower-cased headers and an optional body. `FetchResponse` has
`status`, `headers`, `body`, plus `status_text` and `url` (both may be left
empty: JS then sees the standard reason phrase and the request URL). Build it
with `..FetchResponse::default()`. Returning `Err` rejects the promise with a
`TypeError` ("Failed to fetch: …"), matching a network error on the web.

With no handler installed every `fetch()` rejects with `no fetch handler
installed`. Hosts install one of two ways:

- `JsEngine::install_fetch_handler(Box::new(h))` — engine-wide.
- `oasis_js::fetch::bind_fetch_handler(&ctx, Box::new(h))` — from inside
  `with_context`, rebinding the native hook for that context. `oasis-browser`
  uses this to bind a per-page, origin-aware handler (see below).

`MockFetchHandler` is provided for tests.

### JS surface

`JsEngine::new` installs standard-shaped `fetch(input, init)`, `Response` and
`Headers`:

- `fetch()` returns a **real `Promise`**; `.then()` continuations run as
  microtasks on the next drain (`eval` drains automatically), so
  `fetch(u).then(r => r.json()).then(d => …)` receives the parsed data.
- `input` may be a string, a `URL`-like object (stringified) or a
  `Request`-like object with `url` / `method` / `headers` / `body`.
  `init.headers` may be a plain object, an array of pairs or a `Headers`.
  `GET` / `HEAD` with a body rejects with `TypeError`.
- `Response`: `status`, `ok`, `statusText`, `url`, `redirected`, `type`,
  `bodyUsed`, `headers` (`get` / `has` / `set` / `append` / `delete` /
  `forEach` / iteration, case-insensitive), `text()`, `json()`,
  `arrayBuffer()` (UTF-8 bytes of the body), `clone()`. A body can be
  consumed once; a second read rejects with `TypeError`.

### Synchronous transport

There is **no asynchronous response queue.** The `Promise` executor calls the
native `__oasis_fetch`, which calls `FetchHandler::fetch` and waits for it.
The promise then settles and its callbacks run on the next microtask drain.
`eval` / `tick_timers` therefore never poll: the handler must produce the
response by the time it returns, or fail.

> **Hazard:** `FetchHandler::fetch` is invoked on the JS eval thread.
> Blocking I/O inside it stalls the **entire** JS engine and, in the
> browser, the UI thread. Handlers must bound their I/O with timeouts. The
> watchdog deadline keeps running while the handler blocks, so a request
> that outlives `max_exec_ms` gets the calling script interrupted as soon as
> control returns to JS.

## Storage

`LocalStorage` (`storage.rs`) is one Web Storage area: a sorted
`BTreeMap<String, String>` with a byte quota
(`DEFAULT_STORAGE_QUOTA_BYTES` = 5 MiB, counted as the UTF-8 length of every
key plus value). `try_set_item` enforces the quota and returns
`Err(QuotaExceeded)`; `set_item` is the trusted host-side write and skips the
check. `get_item` returns `Some("")` for a stored empty string. JS
`localStorage.setItem` throws a `DOMException` named `QuotaExceededError`
(code 22) when the quota would be exceeded — `JsEngine::new` installs a
minimal `DOMException` (`DOM_EXCEPTION_SHIM`) because QuickJS-NG has none.

`OriginStorage` holds one `LocalStorage` per origin (`area` / `area_mut`),
each with its own quota. The engine-level `local_storage()` handle is a
single area (the engine has no notion of origin) and does not define
`sessionStorage`. Persistence is the host's job: snapshot the areas on
shutdown and rehydrate on startup.

`oasis-browser` exposes both `localStorage` and `sessionStorage`, each backed
by an `OriginStorage` inside the widget-lifetime `WebStorage`
(`crates/oasis-browser/src/js_dom/storage.rs`: `kind: 0` = local, `kind: 1` =
session). A page only ever sees the area of its own origin
(`scheme://host[:port]`, host lower-cased, default port elided), so two sites
never see each other's keys. Pages with an opaque origin (empty / `about:`
URL) get a private store that lasts only for that page. Both maps live for the
widget (tab) lifetime, so `sessionStorage` survives same-tab navigation;
nothing is written to disk.

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
| `bindings.rs` | Attribute / text / tree-mutation / innerHTML / selector / inline-style / navigation / `getComputedStyle` bindings. |
| `node_api.rs` | Node types, text-inclusive child lists, raw parent / sibling links, detached-safe insertion (fragments, cycle checks), `cloneNode`, fragment parsing, `matches`, `getElementsBy*`, and the freed-slot log that evicts stale wrappers. |
| `fetch.rs` | `BrowserFetchHandler`: origin-aware `FetchHandler` behind `oasis-js`'s `fetch()` (URL resolution, TLS, CSP `connect-src`, private-network and same-origin policy). |
| `storage.rs` | Per-origin, quota'd `localStorage` / `sessionStorage`; page-scoped `document.cookie`. |
| `serialize.rs` | `innerHTML` / `outerHTML` serialization + fragment deep-copy. |
| `canvas.rs` | `__oasis_canvas_*` (feature `canvas`). |
| `compat_shims.rs` | Site-compat helpers (reddit `togglecomment` & co.). |
| `bootstrap.js` / `canvas.js` / `compat_shims.js` | The JS halves, embedded with `include_str!`. |

### Node wrappers and identity

Each DOM node id maps to exactly **one** JS wrapper object, cached in
`bootstrap.js`, so identity comparisons work (`a.parentNode === b`,
`list.children[0] === el`) and expando properties stick (`el._state = …`).
`new Element(nid)` returns the cached wrapper too. Wrappers follow the
standard hierarchy: `Node` → `CharacterData` → `Text` / `Comment`, plus
`Element` (aliased as `HTMLElement`), `DocumentFragment` and `Document`
(`HTMLDocument`); `instanceof` checks behave as in browsers.

Node removal (`remove()`, `removeChild`, `replaceChild`, moving a node with
`appendChild`/`insertBefore`) **detaches** the subtree without freeing it, so
script can re-insert it later. Subtrees replaced by an `innerHTML`,
`outerHTML` or `textContent` write are freed; their ids are logged
(`__oasis_take_freed`) and the JS side drops the matching wrappers and
listeners so a reused arena slot never resolves to a stale object. Removed
nodes stay allocated until the page unloads.

### Document

| API | Notes |
| --- | --- |
| `getElementById(id)` | Connected elements only (detached nodes and clones are skipped). |
| `getElementsByTagName(tag)` / `getElementsByClassName(names)` / `getElementsByName(name)` | Static lists (`item()` supported), `'*'` matches all tags. |
| `querySelector(sel)` / `querySelectorAll(sel)` | Searched from the document root, so `head` content (e.g. `title`) is reachable. |
| `createElement(tag)` / `createElementNS(ns, name)` | Detached element; tag lower-cased. |
| `createTextNode(text)` / `createComment(text)` / `createDocumentFragment()` | Appending a fragment moves its children. |
| `createEvent(kind)` | Legacy; pair with `initEvent` / `initCustomEvent`. |
| `documentElement`, `head`, `body`, `activeElement` | Getters (`activeElement` is always `body`). |
| `title` | Getter and setter. |
| `readyState` | `loading` while parser-inserted scripts run, then `interactive` → `complete`. |
| `URL`, `documentURI`, `location`, `defaultView`, `characterSet`, `compatMode`, `visibilityState`, `hidden` | Getters. |
| `cookie` | Getter and setter; raw string (storage.rs). Page-scoped jar, fresh per page load (never shared across origins, not sent on requests). |
| Node / ParentNode API | `childNodes`, `children`, `firstElementChild`, `append`, `prepend`, `replaceChildren`, `contains`, events, … as on elements. |

### Node (all node kinds)

- `nodeType` (plus the `Node.ELEMENT_NODE` … constants), `nodeName`,
  `nodeValue`, `textContent` (get/set; setting `''` empties the node),
  `isConnected`, `ownerDocument`.
- Navigation including text and comment nodes: `parentNode`, `parentElement`,
  `childNodes`, `firstChild`, `lastChild`, `nextSibling`, `previousSibling`,
  `getRootNode()`, `hasChildNodes()`, `contains(other)`, `isSameNode()`.
- Mutation: `appendChild`, `insertBefore` (a `null` or foreign reference
  appends), `removeChild` / `replaceChild` (throw `NotFoundError` when the
  node is not a child), `cloneNode(deep)`. Inserting an ancestor into its own
  descendant throws `HierarchyRequestError`.
- `Text` / `Comment` add `data`, `length`, `wholeText` (Text) and the
  ChildNode methods below.

### Element

- Element navigation: `children`, `firstElementChild`, `lastElementChild`,
  `childElementCount`, `nextElementSibling`, `previousElementSibling`.
- ChildNode / ParentNode: `remove()`, `before()`, `after()`,
  `replaceWith()`, `append()`, `prepend()`, `replaceChildren()` (strings
  become text nodes).
- Content: `innerHTML` (get/set), `outerHTML` (get/set), `innerText`
  (alias of `textContent`), `insertAdjacentHTML`, `insertAdjacentElement`,
  `insertAdjacentText` (`beforebegin` / `afterbegin` / `beforeend` /
  `afterend`, case-insensitive; others throw `SyntaxError`).
- Identity: `tagName`, `nodeName`, `localName`, `id`, `className`.
- Attributes: `getAttribute`, `setAttribute`, `removeAttribute`,
  `hasAttribute`, `hasAttributes`, `toggleAttribute(name, force)`,
  `getAttributeNames`, `attributes` (static `{name, value}` list),
  `dataset` (proxy over `data-*`, camelCase ↔ kebab-case, supports
  `in` / `delete` / `Object.keys`).
- Selectors: `querySelector`, `querySelectorAll`, `getElementsByTagName`,
  `getElementsByClassName`, `matches` (+ `webkitMatchesSelector` /
  `msMatchesSelector`), `closest`. Invalid selectors throw `SyntaxError`
  from `matches`/`closest`; `querySelector*` return no match.
- `classList` (a cached `DOMTokenList`): `add` / `remove` (multiple tokens),
  `toggle(token, force)`, `contains`, `replace`, `item`, `length`, `value`,
  `forEach`, iteration. Tokens with whitespace throw
  `InvalidCharacterError`. Only real token changes write the attribute (and
  so trigger a relayout).
- `style.<property>` proxy with camelCase ↔ kebab-case conversion,
  `cssText`, `getPropertyValue` / `setProperty`.
- Form controls reflect onto DOM attributes (not the form manager's live
  edit state): `value` (input / option /
  textarea — which also rewrites its text — and `<select>`, which selects
  the matching option), `checked` (unchecks same-name radios), `selectedIndex`,
  `options`, `type`, `htmlFor`, `name`, `href`, `src`, `title`, `alt`,
  `placeholder`, `rel`, `target`, `lang`, `action`, `method`, and the
  boolean `disabled`, `selected`, `hidden`, `required`, `readOnly`,
  `multiple`.
- `click()` dispatches a bubbling, cancelable `MouseEvent`; `focus()` and
  `blur()` are no-ops.

### Events

- `addEventListener(type, fn, opts)` on every node and on `window`; options
  accept a bare capture boolean or `{capture, once, passive, signal}`
  (`signal` needs an `AbortSignal`-like object with `addEventListener`).
  Listener objects with `handleEvent` work. `on<type>` handler properties
  (`el.onclick = fn`, `window.onload = fn`) run after listeners at the target
  and bubble phases; returning `false` calls `preventDefault()`.
- `dispatchEvent(evt)` runs capture → target → bubble (bubble only when
  `evt.bubbles`), with the path continuing from `document` to `window` for
  connected nodes, and returns `!evt.defaultPrevented`. Plain `{type}`
  objects and type strings are accepted for legacy callers.
- `Event`, `CustomEvent` (`detail`), `UIEvent`, `MouseEvent`,
  `KeyboardEvent`, `FocusEvent`, `InputEvent` constructors with init dicts;
  `stopPropagation`, `stopImmediatePropagation`, `preventDefault` (only on
  `cancelable` events), `composedPath`, `eventPhase`, `currentTarget`,
  `target`.
- A listener that throws is reported through `console.error` and does not
  stop the remaining listeners. Watchdog interrupts are uncatchable and
  still abort the dispatch.
- Host input (`__oasis_dispatch_with_bubbling` and the
  `__oasis_dispatch_*_fast` helpers the input layer calls without re-parsing
  JS) fires trusted, bubbling, cancelable `MouseEvent` / `KeyboardEvent` /
  `Event` objects and reports `defaultPrevented` back to Rust.

### Page lifecycle and animation frames

After parser-inserted scripts, the site-compat shims and inline `on*`
handlers are installed, the host calls `js_dom::fire_document_lifecycle`,
which fires, in order: `readystatechange` (`interactive`),
`DOMContentLoaded` on `document` (bubbling to `window`),
`readystatechange` (`complete`), then `load` on `window`. `<body onload>`
registers a `window` load listener. `requestAnimationFrame(cb)` schedules
`cb(performance.now())` on a 16 ms timer; `cancelAnimationFrame(id)`
clears it. `performance.now()` is provided when the engine lacks one.

### Window, location, history

`location.href` getter/setter, `location.assign(url)`, `location.replace(url)`,
`location.reload()`, `history.back()`, `history.forward()`. Navigation actions
are queued and consumed by the host browser layer rather than acted on
synchronously.

### Storage and fetch

`localStorage` and `sessionStorage` mirror the standard `getItem` /
`setItem` / `removeItem` / `clear` / `key` / `length` API on **separate**,
per-origin, 5 MiB-quota areas — see the Storage section above.

`fetch()` is the `oasis-js` implementation (real promises, full `Response`)
with `BrowserFetchHandler` (`js_dom/fetch.rs`) bound behind it for each page.
The handler:

- resolves relative URLs against the document URL and drops the fragment;
  also serves `data:` URLs;
- sends `https:` through the widget's TLS provider (the same one the page
  loader uses; without one, HTTPS fetches reject) and reports the real status
  and response headers (`Set-Cookie` is never exposed);
- enforces CSP `connect-src` when the page has an active policy;
- drops forbidden request headers (`Host`, `Cookie`, `Origin`, `Referer`,
  `Content-Length`, `Proxy-*`, `Sec-*`, …) and rejects `CONNECT` / `TRACE`;
- for pages with an `http(s)` origin, applies the security policy:
  - **Private network access** — a page may not reach an address class more
    private than its own (public < private < loopback). Private covers
    RFC 1918, link-local (`169.254/16`, `fe80::/10`), CGNAT, `fc00::/7` and
    multicast; loopback covers `127/8`, `0/8`, `::1`, `::` and `localhost` /
    `*.localhost`. IP literals (including `inet_aton` spellings like
    `127.1` or `2130706433` and IPv4-mapped IPv6) are rejected up front;
    DNS names are checked on the exact address dialled and on every
    redirect hop, so a public name rebound to `127.0.0.1` is refused too.
    A public page therefore cannot drive the local MCP server
    (`127.0.0.1:7345`) or LAN devices.
  - **Same-origin** requests may use any method, headers and body; an
    `Origin` header is added to non-`GET`/`HEAD` requests.
  - **Cross-origin** requests must be *simple* (`GET`/`HEAD`, no body, only
    `Accept` / `Accept-Language` / `Content-Language`); they carry `Origin`
    and the response is exposed only when `Access-Control-Allow-Origin` is
    `*` or the page origin. There is no preflight, so non-simple
    cross-origin requests reject. A non-simple request is not allowed to be
    redirected to another origin.

Pages without a web origin (`vfs://` pages, HTML loaded from a string) keep
the permissive behaviour: absolute `http(s)` URLs, no network-class or CORS
checks. Requests are synchronous on the UI thread, bounded by the HTTP
client's timeouts (10 s connect, 15 s read); they carry no cookies. On PSP the
per-connection address check is not available (hosts are resolved inside
`TlsProvider::connect_tcp`), so only IP-literal targets and redirect hops are
checked there.

### Canvas 2D

`canvas.getContext('2d')` exposes `fillRect`, `strokeRect`, `clearRect`,
`beginPath`, `arc`, `moveTo`, `lineTo`, `bezierCurveTo`, `quadraticCurveTo`,
`closePath`, `fill`, `stroke`, `fillText`, `save`, `restore`, plus the
`fillStyle`, `strokeStyle`, `font`, `lineWidth` setters
(`js_dom/canvas.rs` + `js_dom/canvas.js`).

### Known gaps vs. browser baseline

These are intentional — file an issue or extend `js_dom/` if you need them:

- `getComputedStyle()` only offers `getPropertyValue` (styles from the last
  cascade), not property access or live updates.
- `getBoundingClientRect()` and the `scroll*` family.
- `MutationObserver`, `IntersectionObserver`, Shadow DOM, custom elements.
- Removed nodes stay allocated until the page unloads (no GC hook).

## Adding a host capability

The pattern for new bindings is the one `oasis-browser` already follows.

1. Add a Rust function in `js_dom/bindings.rs` (or the matching file) named
   `__oasis_<verb>` that takes only primitives (`i32`, `String`, `f64`,
   `Vec<i32>`) and returns a primitive or small struct convertible via rquickjs.
2. Mark the DOM dirty via `mark_dirty(&dirty)` if the call mutates the tree
   (`js_dom/mod.rs`). If it frees arena slots, go through
   `node_api::free_children_logged` (or push the ids onto the `FreedLog`) and
   call `__sync_freed()` from the JS wrapper so cached wrappers are evicted;
   prefer `detach_node` over `remove_child` for nodes script may still hold.
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
`install_fetch_handler` (or `bind_fetch_handler` from inside `with_context`).
For new storage backends, swap the `LocalStorage` / `OriginStorage` contents
behind the shared `Rc<RefCell<>>`.

## Testing

Use `MockFetchHandler` for deterministic fetch responses. Timer-driven code
should advance with `tick_timers(dt)` instead of relying on wall-clock time.
The console buffer is the easiest assertion target — run a script that ends in
`console.log("ok")` and check `take_console_output()`.
