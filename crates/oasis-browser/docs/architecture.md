# Architecture

## `BrowserWidget`

`BrowserWidget` (in `src/lib.rs`) is the only public entry point. It owns
every piece of mutable per-tab state: the parsed `Document`, the cascaded
styles, the layout tree, the recorded display list, the navigation
history, the IO thread handle, the JavaScript engine (when enabled), and
the form manager.

The window manager creates one `BrowserWidget` per browser tab and drives
it through three methods every frame:

```text
loop {
    widget.tick(vfs);                  // process IO, animations, deferred scripts
    if event_pending { widget.handle_input(event); }
    widget.paint(backend, viewport);   // record + replay display list
}
```

`BrowserWidget` is **not** `Send` or `Sync` — every field is owned, the
JavaScript engine holds non-`Send` `Rc<RefCell<>>` shared state, and the
IO thread is the only piece that crosses thread boundaries (it owns its
own clones).

## Per-frame lifecycle

1. **`tick(vfs)`**
   - Drains completed `IoResult`s from the IO thread, parses and dispatches
     them (HTML → tree builder, CSS → cascade, image → decode pool).
   - Advances any running CSS transitions / animations and marks affected
     subtrees dirty.
   - Runs deferred `<script>` blocks that were queued during HTML parsing
     once their dependencies (`async` images, `defer` scripts) resolve.
   - Resolves `setTimeout` / `setInterval` callbacks scheduled by JS.

2. **`handle_input(event)`**
   - Routes scroll events to the nearest scroll container (nested
     `overflow: auto` is supported via per-element scroll offsets).
   - Routes click / key events to the focused form element first, then
     to JS event listeners (capture → target → bubble), then to the
     navigation handler for plain `<a>` clicks.
   - Updates `:hover` / `:focus` / `:active` state and marks the affected
     subtree dirty so the next paint picks up transitions.

3. **`paint(backend, viewport)`**
   - Calls `relayout_if_dirty()` which walks the dirty layout subtrees,
     remeasures text against the cached `SharedTextCache`, and rebuilds
     just the affected `LayoutBox` ranges.
   - Records a fresh `DisplayList` if anything changed, otherwise replays
     the cached one with adjusted scroll offsets.
   - Replays the display list to the `SdiBackend`, intersecting against
     dirty rects so the painter only touches changed pixels.

The pipeline is intentionally one-shot: a navigation kicks off
`navigate_url(url)` which produces a fresh `Document` and `LayoutBox`
tree. Incremental updates after that point happen at the cascade and
layout level — the document itself is replaced wholesale.

## Crates and top-level modules

The engine is four crates. The three parsers/loaders below have no
dependency on layout or paint, so they build and test on their own
(`cargo test -p oasis-css` compiles neither layout nor QuickJS):

```text
oasis-html   tokenizer, tree builder, arena DOM, embedder Hook<F>   (deps: log)
   ↑
oasis-css    CSS tokenizer, parser, cascade, computed values        (deps: oasis-html, oasis-types)
   ↑
oasis-browser  layout, paint, widget, forms, js_dom, gemini, image, font, svg, canvas
   ↓
oasis-http   Url, HTTP/1.1 + HTTP/2, Gemini framing, cookies, cache, CSP, IO thread
             (deps: oasis-net, oasis-vfs, oasis-types; feature psp)
```

`oasis-browser` keeps the old module paths: `src/lib.rs` aliases the
crates as private `html` / `css` modules (`pub(crate) use oasis_html as
html;`) and re-exports `pub use oasis_http as loader;`, so
`crate::css::cascade::...` inside the crate and
`oasis_browser::loader::...` / `oasis_browser::internals::...` for
embedders are unchanged. Items the browser needs from the other crates
are `pub`; their APIs are engine-internal and may change without notice.

```text
crates/oasis-browser/src/
├── lib.rs              BrowserWidget + the public surface
├── widget/
│   ├── paint.rs        paint(): display list record (cached) + replay
│   ├── input/          handle_input(): event routing (click, focus, forms,
│   │                   history, hover + transitions, JS events, scroll)
│   ├── pipeline/       navigate -> parse -> cascade -> layout glue
│   │                   (navigation, document, stylesheets, scripts, gemini_page)
│   └── images.rs       async image decode plumbing
├── config.rs           viewport, zoom, cache + image caps, feature toggles
│
├── layout/             block / inline / flex / grid / table layout,
│                       @container lookup (container_query.rs)
├── paint/              display list record / optimize / replay (the only renderer)
├── transform.rs        2D / 3D transform matrices for layout + paint
├── nav.rs              navigation history (back / forward stacks)
├── forms/              <input>, <textarea>, <select>, validation, submission
├── image.rs            format detection, decode + downscale to ImageLimits
├── image_atlas.rs      texture atlas packing for many small images
├── gemini/             .gmi parser (protocol in oasis-http; HTML translation in widget/)
├── js_dom/             QuickJS DOM bindings + JS bootstrap (feature = "javascript")
└── reader.rs           reader-mode extraction
```

The `widget/*.rs` files exist purely to keep `lib.rs` readable — they
contain `impl BrowserWidget` blocks that implement different aspects of
the same struct. Treat them as one logical unit.

## Public surface

The crate exports very little publicly. From `src/lib.rs`:

- `BrowserWidget`, `Focus`, `LoadingState`
- `BrowserConfig` (viewport, cache size, scroll speed, feature toggles)
- A handful of types under `#[doc(hidden)]` for fuzz / bench harnesses
  (`Tokenizer`, `TreeBuilder`, `Stylesheet`, `ComputedStyle`, `LayoutBox`).

Embedders normally only ever construct `BrowserWidget`, drive it via the
three methods above, and read `widget.title()`, `widget.url()`, and
`widget.loading_state()` for chrome.

## Threads

There is exactly **one** background thread per `BrowserWidget`: the
`IoThread` in `oasis-http/src/io_thread.rs`. It processes `IoWork` items
sequentially (HTTP fetch, Gemini fetch, TLS handshake, response decode)
and posts `IoResult` back through a channel. Image decoding runs on a
second, lazily spawned `img-decode` thread — see `widget/images.rs`.

Everything else runs on the main thread. The JavaScript engine is single
threaded by design.

## Backends

`oasis-browser` only knows about `SdiBackend` (re-exported from
`oasis-types`). The painter calls `fill_rect`, `draw_text`, `blit`,
`blit_sub`, `submit_rect_batch`, `submit_text_batch`, `push_clip`,
`pop_clip`, `set_blend_mode`. Backends decide how those map to the GPU,
software framebuffer, Canvas 2D, or PSP GU.

When porting to a new backend, the only thing that matters is whether
those calls are honored. The crate has no other platform dependencies.
