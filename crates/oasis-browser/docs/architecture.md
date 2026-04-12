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

## Top-level modules

```text
src/
├── lib.rs              BrowserWidget + the public surface
├── widget_paint.rs     paint(): display list record + replay
├── widget_input.rs     handle_input(): event routing
├── widget_pipeline.rs  navigate -> parse -> cascade -> layout glue
├── widget_images.rs    async image decode plumbing
├── config.rs           viewport, zoom, cache caps, feature toggles
│
├── html/               HTML5 tokenizer, tree builder, arena DOM
├── css/                CSS tokenizer, parser, cascade, values, shorthand
├── layout/             block / inline / flex / grid / table layout
├── paint/              display list recording + per-item painters
├── nav.rs              navigation history (back / forward stacks)
├── loader/             resource cache, IO thread, HTTP, Gemini, VFS, CSP
├── forms/              <input>, <textarea>, <select>, validation, submission
├── image.rs            format detection + decode dispatch
├── image_atlas.rs      texture atlas packing for many small images
├── gemini/             Gemini protocol parser + .gmi → HTML rasteriser
├── js_dom.rs           QuickJS DOM bindings (feature = "javascript")
├── plugin.rs           per-URL-scheme plugin trait
└── reader.rs           reader-mode extraction
```

The `widget_*.rs` files exist purely to keep `lib.rs` readable — they
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
`IoThread` in `loader/io_thread.rs`. It processes `IoWork` items
sequentially (HTTP fetch, Gemini fetch, TLS handshake, response decode)
and posts `IoResult` back through a channel. Image decoding spawns
short-lived worker threads from a small pool — see `widget_images.rs`.

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
