# JavaScript

JavaScript support is **feature-gated** behind `feature = "javascript"`
and lives almost entirely in `src/js_dom.rs` (~110 KB). The runtime is
[QuickJS-NG](https://github.com/quickjs-ng/quickjs) wrapped by
[`rquickjs`](https://docs.rs/rquickjs), re-exported through the sibling
crate `oasis-js`.

If you build with `--no-default-features --features ""` you get a
browser with no JS at all (smaller binary, no DOM mutation from
script). The PSP backend currently builds without JS.

## Architecture

`rquickjs` closures cannot easily return rich `Object<'js>` values
across the FFI boundary, so the bindings use a **two-layer** trick:

1. **Rust functions** named `__oasis_*` operate on integer `NodeId`s
   and return primitives (numbers, strings, bools).
2. **JavaScript shim code**, evaluated once at engine startup, defines
   the user-facing constructors (`Element`, `HTMLCollection`, …) and
   `document` global. The shim wraps every Rust call so user code sees
   ordinary JS objects.

This keeps the Rust↔JS boundary minimal and avoids per-call lifetime
gymnastics.

## Shared state

The bindings need to mutate browser state from inside JS callbacks, so
several pieces of state are stored in `Rc<RefCell<…>>` and cloned into
the engine on creation:

```text
SharedDoc            mutable Document for getElementById, createElement, ...
SharedStyles         computed styles for getComputedStyle()
SharedNavActions     queue of JS-initiated nav (location.assign, history.back)
SharedLocalStorage   per-origin localStorage map (persistent across navs)
SharedSessionStorage per-tab sessionStorage map (cleared on tab close)
SharedCanvasMap      <canvas> 2D drawing state shared with bindings
SharedTimers         setTimeout / setInterval queue
```

After the engine runs a callback, `BrowserWidget::tick()` drains the
nav action queue and any pending timer callbacks, then schedules a
relayout if any DOM mutation happened.

## DOM APIs exposed

### Selectors
- `document.getElementById(id)`
- `document.querySelector(selectors)`
- `document.querySelectorAll(selectors)` → returns a `NodeList`
- `element.querySelector(...)` / `querySelectorAll(...)`

### Element properties
- `.tagName`, `.id`, `.className`, `.classList` (`add`, `remove`,
  `toggle`, `contains`)
- `.textContent` (read + write)
- `.innerHTML` (read + write — write triggers a re-parse of the
  fragment)
- `.getAttribute(name)`, `.setAttribute(name, value)`,
  `.removeAttribute(name)`, `.hasAttribute(name)`
- `.style` — proxy that maps to inline `style="…"` declarations
- `.parentNode`, `.children`, `.firstChild`, `.lastChild`,
  `.nextSibling`, `.previousSibling`
- `.appendChild(child)`, `.removeChild(child)`,
  `.insertBefore(child, ref)`, `.replaceChild(new, old)`
- `.cloneNode(deep)`

### Events
- `addEventListener(type, handler, options)` — `options` honors
  `{once, capture, passive}`
- `removeEventListener(type, handler)`
- Three-phase dispatch (capture → target → bubble) via
  `__oasis_dispatch_with_bubbling`
- Event objects expose `.type`, `.target`, `.currentTarget`,
  `.clientX`, `.clientY`, `.key`, `.code`, `.detail`,
  `.stopPropagation()`, `.preventDefault()`
- Supported event types: `click`, `dblclick`, `mousedown`, `mouseup`,
  `mousemove`, `mouseover`, `mouseout`, `keydown`, `keyup`, `keypress`,
  `submit`, `change`, `input`, `focus`, `blur`, `load`, `DOMContentLoaded`

### Navigation & history
- `window.location.href` (getter + setter)
- `window.location.assign(url)`, `.replace(url)`, `.reload()`
- `window.history.back()`, `.forward()`, `.go(n)`
- `window.history.pushState(state, title, url)`,
  `.replaceState(state, title, url)`

### Storage
- `localStorage.getItem / setItem / removeItem / clear / length / key`
- `sessionStorage.getItem / setItem / removeItem / clear / length / key`
- `document.cookie` getter / setter (delegates to the loader cookie jar)

### Network
- `fetch(url, init)` returns a `Promise<Response>` (basic JSON / text
  support; CSP `connect-src` is enforced)
- `Response.text()`, `.json()`, `.status`, `.ok`, `.headers`

### Timers
- `setTimeout(fn, ms)`, `clearTimeout(id)`
- `setInterval(fn, ms)`, `clearInterval(id)`

### Console
- `console.log`, `.warn`, `.error`, `.info`, `.debug` — output goes
  through the `log` crate.

### Canvas 2D
- `canvas.getContext('2d')` returns a context with:
  - Path API: `beginPath`, `moveTo`, `lineTo`, `bezierCurveTo`,
    `quadraticCurveTo`, `arc`, `arcTo`, `rect`, `closePath`
  - Fill / stroke: `fill`, `stroke`, `fillStyle`, `strokeStyle`,
    `lineWidth`, `lineCap`, `lineJoin`
  - Text: `fillText`, `strokeText`, `font`, `textAlign`, `textBaseline`,
    `measureText`
  - Transform: `save`, `restore`, `translate`, `rotate`, `scale`,
    `transform`, `setTransform`
  - Pixel ops: `getImageData`, `putImageData`, `createImageData`
  - Compositing: `globalAlpha`, `globalCompositeOperation`

## Engine lifecycle

1. The engine is constructed lazily on the first inline `<script>` or
   `addEventListener` call.
2. The shim JS is evaluated once. After that the engine is "warm".
3. Each script block is evaluated as it is encountered during HTML
   parse. `<script async>` is queued and run when downloaded;
   `<script defer>` is queued and run after parsing finishes.
4. Event handlers run for the lifetime of the document.
5. On navigation, the engine is dropped and a fresh one is created for
   the new document. `localStorage` survives because it lives in the
   `BrowserWidget`, not in the engine.

## What is not supported

- ES2024 features beyond what QuickJS-NG ships (which is most of
  ES2020).
- WebAssembly.
- WebGL / WebGL2 (only Canvas 2D).
- Workers (Web / Service / Shared).
- IndexedDB.
- WebSockets, WebRTC.
- The full `MutationObserver` interface (a stub exists but does not
  fire).

## Tests

- `js_dom` itself does not contain unit tests — it is exercised through
  `tests/browser_integration.rs` and through running real pages.
- The `oasis-js` crate has its own test suite for the runtime layer.
