# Refactor Plan: Backend Traits, Test Gaps, Browser JS Runtime

**Branch**: `refactor/backend-tests-js-plan`
**Status**: Phase 1 implemented — extension traits defined, ~200 tests added, JS event dispatch working, ASAN fix applied

---

## 1. Backend Trait Surface Area Reduction

### Problem

`oasis-types/src/backend.rs` is 2,903 lines with 77 total methods across traits.
`SdiBackend` alone has 48 optional methods with default implementations. While the
separation of `SdiCore` (13 required) + `SdiBackend` (48 extended) is sound, the
monolithic trait will be harder to maintain as backends diverge further.

### Current Backend Override Coverage

| Backend | Methods Overridden | Coverage |
|---------|--------------------|----------|
| WASM    | 45/53              | 85%      |
| SDL     | 41/53              | 77%      |
| UE5     | 34/53              | 64%      |
| PSP     | 19/53              | 36%      |

### Proposed: Split SdiBackend into Extension Traits

Split the 48 optional `SdiBackend` methods into focused extension traits.
Backends opt-in by implementing only the traits they need. Fallback behavior
preserved via blanket default impls.

```
SdiCore (13 required — unchanged)
│
├── SdiShapes (7 methods)
│   fill_rounded_rect, stroke_rect, draw_line, fill_circle, fill_triangle,
│   stroke_circle, fill_ellipse
│
├── SdiGradients (2 methods)
│   fill_rect_gradient, fill_rounded_rect_gradient
│
├── SdiAlpha (3 methods)
│   fill_rect_alpha, viewport_size, dim_screen
│
├── SdiText (8 methods)
│   measure_text_height, font_ascent, draw_text_styled, draw_text_wrapped,
│   draw_text_ellipsis, draw_text_centered, draw_text_right_aligned,
│   draw_text_weighted
│
├── SdiTextures (4 methods)
│   blit_sub, blit_tinted, blit_sub_tinted, blit_flipped
│
├── SdiClipTransform (8 methods)
│   push_clip_rect, pop_clip_rect, push_translate, pop_translate,
│   push_clip_rounded, clip_stack_depth, translate_stack_depth,
│   current_translate_offset
│
├── SdiVector (6 methods)
│   fill_polygon, stroke_polygon, fill_arc, stroke_arc,
│   stroke_line_dashed, fill_polygon_gradient
│
└── SdiBatch (2 methods)
    begin_batch, flush_batch
```

### Dead Code Candidates

These methods are either never overridden or never called in production:

| Method | Issue | Action |
|--------|-------|--------|
| `fill_polygon_gradient` | Zero call sites | Remove |
| `draw_text_weighted` | Never overridden, weight ignored | Remove or defer |
| `begin_batch` / `flush_batch` | No-op placeholders | Keep as `SdiBatch` but document as reserved |

### Migration Strategy

1. **Phase 1**: Define the 8 extension traits alongside `SdiBackend` (no breaking changes)
2. **Phase 2**: Implement extension traits for each backend, delegating to existing methods
3. **Phase 3**: Migrate call sites from `dyn SdiBackend` to specific extension trait bounds
4. **Phase 4**: Remove methods from `SdiBackend`, replace with blanket impls
5. **Phase 5**: Remove dead methods (`fill_polygon_gradient`, `draw_text_weighted`)

### Risk Mitigation

- Each phase is a standalone PR — revert granularity per trait group
- `SdiCore` is **never touched** — all 4 backends keep working throughout
- Extension traits use default impls identical to current `SdiBackend` defaults
- No changes to PSP backend (it already relies on defaults for 34/48 methods)

---

## 2. Test Gaps in Platform-Critical Code

### Current State

| Crate | Lines | Tests | Coverage |
|-------|-------|-------|----------|
| oasis-backend-psp | 14,026 | 0 | 0% |
| oasis-plugin-psp | 2,449 | 0 | 0% |
| oasis-backend-wasm | 4,808 | 0 | 0% |
| oasis-backend-sdl | 3,401 | 33 (all `#[ignore]`) | ~1% |
| oasis-backend-ue5 | 1,886 | 53 | ~15% |
| oasis-video | 3,758 | 37 | ~10% |
| tv_controller.rs | 2,394 | 0 | 0% |

### Tier 1: High-Value, Low-Effort (pure logic, no mocks needed)

**Target: ~120 new tests**

#### 2.1 Input Mapping Tests (all backends)

Test button/key→InputEvent mapping, coordinate scaling, deadzone math.

- **PSP** (`input.rs`, 87 lines): analog deadzone (0.31 threshold), cursor speed (5.0x),
  button-to-InputEvent mapping. ~15 tests.
- **SDL** (`input.rs`, 98 lines): `map_key_down`/`map_key_up` (11 mappings each), wheel
  delta sign. ~20 tests.
- **WASM** (`input.rs` + `input_dispatch.rs`, 769 lines): `scale_point()` letterbox math,
  touch vs mouse routing. ~25 tests.
- **UE5**: Already has 3 tests; add coordinate clamping edge cases. ~5 tests.

#### 2.2 Color/Pixel Format Conversion

RGBA↔ABGR, RGB565↔RGBA8888 — pure math across UE5 and PSP renderers. ~20 tests.

#### 2.3 Video Demux Error Paths

Extend existing `demux_lite.rs` fixtures: truncated box headers, invalid box sizes,
missing required atoms (ftyp/moov/mdat). ~15 tests.

#### 2.4 StreamingBuffer Throttle Logic

Test `should_throttle()` with mocked `decoder_pos`, `bytes_received`, `has_moov` states.
Test MAX_LOOKAHEAD (16MB) boundary. Test `maybe_evict()` window sliding. ~15 tests.

#### 2.5 Seek Interpolation

Test `(seek_secs / duration) * file_size` at boundaries (0%, 50%, 100%, beyond-end). ~8 tests.

### Tier 2: Medium-Effort (needs mock traits)

**Target: ~80 new tests**

#### 2.6 MockSdiCore Test Backend

Create a `MockSdiCore` in a shared test utility crate (`oasis-test-utils`) that records
all calls for assertion:

```rust
pub struct MockSdiCore {
    pub calls: Vec<RenderCall>,
    pub textures: HashMap<u64, TextureInfo>,
}
```

This unblocks shape rasterization tests, font rendering tests, and widget rendering tests
across all backends.

#### 2.7 Video Decode Pipeline

- `aac.rs` (82 lines, 0 tests): AAC frame validation, ADTS header parsing. ~10 tests.
- `h264.rs` (88 lines, 0 tests): NAL unit type detection, SPS/PPS extraction. ~10 tests.
- `ffmpeg_decoder.rs` error paths: malformed AVIO callbacks, seek past EOF. ~15 tests
  (requires test fixture).

#### 2.8 PSP Audio Mixer

Mock audio sources (silence, impulse, chirp), test mixing algorithm and overflow
saturation. ~20 tests.

#### 2.9 WASM Audio Queue

Queue push/drain logic, sample rate conversion boundaries. ~10 tests.

### Tier 3: High-Effort (architectural, integration)

**Target: ~50 new tests, longer term**

#### 2.10 WASM Backend with Mocked web-sys

Use `wasm-bindgen-test` with `#[wasm_bindgen_test]` attribute. Mock `CanvasRenderingContext2d`
for renderer tests, `AudioContext` for audio tests. Requires wasm-pack test runner.

#### 2.11 SDL Backend Display Tests

Convert existing 33 `#[ignore]` tests to run in CI via Xvfb (virtual framebuffer).
Already have `try_create_backend()` helper.

#### 2.12 PSP Thread Synchronization

SpscQueue correctness under contention, mutex ordering, deadlock detection. Requires
careful safety review and PSP-specific thread primitives.

### Implementation Order

```
Phase A (Tier 1):  Input mapping + color conversion + demux errors + throttle logic
Phase B (Tier 2a): MockSdiCore crate + video decode unit tests
Phase C (Tier 2b): PSP audio + WASM audio queue tests
Phase D (Tier 3):  WASM wasm-bindgen-test + SDL Xvfb + PSP threading
```

Each phase is a separate PR. Phase A has zero new dependencies.

---

## 3. Browser JavaScript Runtime Completion

### Current State

| Feature | Status | Location |
|---------|--------|----------|
| `<script>` inline execution | Working | widget_pipeline.rs:121-224 |
| `console.log/warn/error/info` | Working | console.rs:68-108 |
| `document.getElementById` | Working | js_dom.rs:381-383 |
| `document.createElement` | Working | js_dom.rs:385-386 |
| `element.textContent` (get/set) | Working | js_dom.rs:292-326 |
| `element.getAttribute/setAttribute` | Working | js_dom.rs:328-341 |
| `element.appendChild` | Working | js_dom.rs:340-341 |
| `addEventListener` | Stores listeners but **never fires** | js_dom.rs:346-351 |
| `removeEventListener` | Works (removes by ref) | js_dom.rs:352-360 |
| `dispatchEvent` | Manual only, no auto-dispatch | js_dom.rs:361-368 |
| `setTimeout` / `setInterval` | **Stub — logs warning, returns 0** | console.rs:125-147 |
| `fetch` / `Promise` / `async` | **Not implemented** | — |
| `<script src="...">` | **Silently skipped** | widget_pipeline.rs:287 |
| Event bubbling/capturing | **Not implemented** | — |
| `querySelector`/`querySelectorAll` | **Not implemented** | — |

### Root Cause: No Event Loop

The JS engine is created, all `<script>` tags execute synchronously, then the engine
is dropped. There is no mechanism to:
1. Fire callbacks after page load (timers, events)
2. Dispatch DOM events from user input
3. Process async operations (fetch, promises)

### Proposed Architecture: Retained JS Engine + Task Queue

Currently the engine lifecycle is:
```
page load → create engine → eval all scripts → drop engine
```

Proposed:
```
page load → create engine → eval all scripts → retain engine
               ↓
         input events → dispatch to JS listeners
               ↓
         timer tick → fire pending setTimeout/setInterval callbacks
               ↓
         page unload → drop engine
```

### Phase 1: Event Dispatch (connect existing infrastructure)

The listener map (`__oasis_listeners`) and dispatch function (`__oasis_dispatch_event`)
already exist. The missing piece is calling dispatch from Rust input handling.

**Changes:**

1. **Retain JsEngine beyond page load** — store `Option<JsEngine>` in browser widget state
   instead of dropping after script execution.

2. **Bridge input events to JS** — in `widget_input.rs`, after handling a click/keypress,
   call `__oasis_dispatch_event(nid, "click", detail)` on the clicked element's node ID.

3. **Event propagation (bubbling)** — walk parent chain from target to root, dispatching
   at each level. Add `event.stopPropagation()` flag.

   ```javascript
   function __oasis_dispatch_with_propagation(nid, type, detail) {
       var evt = { type: type, detail: detail, target: new Element(nid),
                   _stopped: false,
                   stopPropagation: function() { this._stopped = true; } };
       // Bubble: target → parent → ... → body
       var current = nid;
       while (current > 0 && !evt._stopped) {
           var key = current + ":" + type;
           var arr = __oasis_listeners[key];
           if (arr) {
               evt.currentTarget = new Element(current);
               for (var i = 0; i < arr.length; i++) arr[i].call(evt.currentTarget, evt);
           }
           current = __oasis_parent(current);
       }
   }
   ```

4. **Supported events**: `click`, `input`, `change`, `keydown`, `keyup`, `focus`, `blur`

**Estimated scope**: ~200 lines changed across `widget_input.rs`, `js_dom.rs`, and
browser widget state management.

### Phase 2: Timers (setTimeout / setInterval)

Replace stubs with a frame-driven timer queue.

**Design:**

```rust
// In JsEngine or a new TimerQueue struct
struct PendingTimer {
    id: u32,
    callback: rquickjs::Persistent<Function>,
    fire_at: Instant,
    interval: Option<Duration>,  // None = setTimeout, Some = setInterval
}
```

**Integration point**: Browser widget's `update()` method (called each frame) checks
the timer queue and executes any expired callbacks.

**Changes:**

1. Replace `setTimeout` stub in `console.rs` with a closure that stores the callback
   and delay in a shared `TimerQueue`.
2. Replace `setInterval` similarly, with `interval: Some(duration)`.
3. `clearTimeout`/`clearInterval` remove by timer ID.
4. In browser widget frame update: `engine.drain_timers()` fires expired callbacks.

**Estimated scope**: ~150 lines new code (TimerQueue struct + integration).

### Phase 3: Promises + Microtask Queue

QuickJS-NG has built-in Promise support. rquickjs exposes it via `ctx.execute_pending_jobs()`.

**Changes:**

1. Enable Promise support in rquickjs context configuration.
2. Call `ctx.execute_pending_jobs()` after each script eval and after each timer/event callback.
3. This gives us `Promise.resolve()`, `Promise.reject()`, `.then()`, `.catch()`, `async/await`.

**Estimated scope**: ~30 lines (rquickjs already handles the heavy lifting).

### Phase 4: fetch() API

Wrap the existing `load_resource()` infrastructure in a JS-callable `fetch()` that
returns a Promise.

**Design:**

```javascript
// User-facing API
fetch("https://example.com/data.json")
    .then(response => response.text())
    .then(text => { /* use text */ });
```

**Rust-side**: Register `fetch` as a global that:
1. Accepts a URL string
2. Calls `load_resource()` (blocking, on current thread — acceptable for embedded browser)
3. Resolves the Promise with a Response-like object (`{ ok, status, text(), json() }`)

**Estimated scope**: ~100 lines.

### Phase 5: Additional DOM APIs

Once the event/timer/promise infrastructure exists, add commonly-needed DOM APIs:

- `element.innerHTML` (get/set) — requires HTML serializer for get, parser for set
- `element.querySelector(selector)` / `querySelectorAll(selector)` — CSS selector matching
- `element.classList.add/remove/toggle/contains`
- `element.style.setProperty(name, value)` — inline style mutation
- `window.location` (read-only)

These are independent and can be added incrementally per demand.

### Implementation Order

```
Phase 1: Event dispatch + bubbling       (unblocks interactive pages)
Phase 2: setTimeout / setInterval         (unblocks animations, delayed logic)
Phase 3: Promises + microtask queue       (unblocks async patterns)
Phase 4: fetch()                          (unblocks data-driven pages)
Phase 5: Additional DOM APIs              (incremental, on demand)
```

Each phase is independently useful and a separate PR.

---

## Summary: PR Sequence

| PR | Area | Scope | Dependencies |
|----|------|-------|--------------|
| 1 | Backend: define extension traits | ~500 lines | None |
| 2 | Backend: implement extension traits per backend | ~400 lines | PR 1 |
| 3 | Backend: migrate call sites | ~300 lines | PR 2 |
| 4 | Backend: remove dead methods | ~50 lines | PR 3 |
| 5 | Tests: input mapping + color conversion (Tier 1) | ~400 lines | None |
| 6 | Tests: demux errors + throttle + seek (Tier 1) | ~300 lines | None |
| 7 | Tests: MockSdiCore crate (Tier 2) | ~200 lines | None |
| 8 | Tests: video decode unit tests (Tier 2) | ~250 lines | PR 7 |
| 9 | Tests: audio tests (Tier 2) | ~200 lines | None |
| 10 | JS: retained engine + event dispatch + bubbling | ~200 lines | None |
| 11 | JS: setTimeout / setInterval | ~150 lines | PR 10 |
| 12 | JS: Promises + microtask queue | ~30 lines | PR 10 |
| 13 | JS: fetch() API | ~100 lines | PR 12 |

PRs 1-4 (backend), 5-9 (tests), and 10-13 (JS) are **independent tracks** that can
proceed in parallel.
