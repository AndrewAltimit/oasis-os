# oasis-browser docs

The `oasis-browser` crate is the embeddable HTML / CSS / Gemini engine that
ships inside OASIS_OS. It is a from-scratch Rust implementation — no servo,
no chromium, no webview. The whole engine compiles for desktop (SDL3), web
(WASM/Canvas), Unreal Engine 5 (FFI), and the PSP (`no_std`-ish, hardware
H.264 decode).

These docs are organised by pipeline stage. Read them in order if you are
new to the crate.

| Doc | What it covers |
| --- | --- |
| [`architecture.md`](architecture.md) | The big picture — `BrowserWidget`, top-level modules, the per-frame lifecycle. **Start here.** |
| [`html-and-dom.md`](html-and-dom.md) | HTML5 tokenizer, tree builder quirks, the arena-backed DOM. |
| [`css-pipeline.md`](css-pipeline.md) | CSS tokenizer, parser, selector matching, cascade, specificity, `@media` / `@supports` / `var()` / `calc()`. |
| [`css-coverage.md`](css-coverage.md) | The authoritative list of supported CSS properties + known gaps. **Update on every CSS change.** |
| [`layout.md`](layout.md) | The layout engine: block / inline / flex / grid / table, text shaping, the incremental layout cache. |
| [`paint.md`](paint.md) | Display list recording, replay, dirty rect tracking, how the painter talks to `SdiBackend`. |
| [`loading-and-navigation.md`](loading-and-navigation.md) | Resource loader, IO thread, cache, cookies, CSP, navigation history, forms. |
| [`javascript.md`](javascript.md) | The QuickJS-NG runtime, DOM bindings, event dispatch, `localStorage`, Canvas 2D. |
| [`gemini.md`](gemini.md) | Gemini protocol support and how it shares the HTML pipeline. |
| [`testing.md`](testing.md) | Unit tests, integration tests, benches, fuzz targets, what to run before pushing. |

## Conventions

- **No allocations in hot paint paths.** The display list is recorded once
  per layout and replayed each frame; per-frame work should be measured in
  cycles, not allocations.
- **Backends only see `SdiBackend`.** The crate never imports SDL, wgpu, or
  PSP types directly. Anything platform-specific is hidden behind the trait.
- **Tests live next to code.** A `#[cfg(test)] mod tests` block at the bottom
  of each module is preferred over a separate `tests/` file. The
  `tests/browser_integration.rs` file is reserved for full-pipeline tests.
- **Property tests are encouraged.** We use `proptest` for parser fuzzing
  and structural invariants — see `cascade::tests::prop_tests`.

## When in doubt

- The pipeline is **HTML → CSS → layout → paint**. If you are not sure
  where a bug lives, dump the DOM, dump the computed style, dump the layout
  tree, and look at the first stage where the wrong value appears.
- The painter never calls platform APIs directly — every draw goes through
  `SdiBackend`. If something does not appear on screen, check whether the
  display item was recorded *and* whether it was replayed (dirty rects can
  filter it out).
- Many properties in `ComputedStyle` are parsed but not yet honored by the
  painter. See the **Storage vs. rendering** section of
  [`css-coverage.md`](css-coverage.md) before assuming a property is "broken".
