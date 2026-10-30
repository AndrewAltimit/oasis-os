# ADR-003: Backend Trait Design (Split Backend Traits)

**Status:** Accepted
**Date:** 2026-02-09 (initial release; recorded 2026-02-19)
**Last reviewed:** 2026-10-30. The decision is still current. Counts, the
removed opt-in traits and the shared span rasterizer were updated to match
the source, and the drawing-state contract every backend follows was added.
See `crates/oasis-types/src/backend/`.

## Context

OASIS_OS must render to SDL3, WebAssembly (Canvas 2D), Unreal Engine 5, and PSP
hardware. Each platform has different rendering APIs, input mechanisms, networking
stacks, and audio subsystems. We need an abstraction that:

- Keeps core code platform-agnostic
- Allows backends to implement only what they support
- Minimizes trait object overhead on PSP (333 MHz, 32 MB RAM)

## Decision

We define **separate backend traits** in `oasis-types` (five main ones,
plus `ClipboardBackend` and the `NetworkStream` connection trait):

1. **`SdiCore`** -- Required rendering: 13 methods (`init`, `clear`, `blit`,
   `fill_rect`, `draw_text`, `swap_buffers`, `load_texture`, `destroy_texture`,
   `set_clip_rect`, `reset_clip_rect`, `measure_text`, `read_pixels`, `shutdown`).
2. **`SdiBackend`** -- A marker super-trait with no methods of its own. It
   bundles `SdiCore` plus nine extension traits via a blanket impl, so any
   type that satisfies `SdiCore` and all nine extensions automatically
   implements `SdiBackend`. The nine extensions are `SdiShapes`,
   `SdiGradients`, `SdiAlpha`, `SdiText`, `SdiTextures`, `SdiClipTransform`,
   `SdiVector`, `SdiBatch`, and `SdiRenderTarget`, totalling 53
   default-implemented methods that fall back to `SdiCore` primitives
   (shapes 8, gradients 2, alpha 3, text 8, textures 4, clip/transform 8,
   vector 7, batch 4, render target 9).
   `SdiBatch` provides `begin_batch`/`flush_batch` (reserved hooks, no-ops
   on every backend today) plus `submit_rect_batch`/`submit_text_batch` for
   batched rect and text geometry submission (used by the browser's light
   compositor to reduce draw call count). The former opt-in traits
   `SdiBlendMode` and `SdiGeometry` had no implementation or caller outside
   tests and were removed in 2026-10; blend modes are expressed through
   `SdiRenderTarget::composite_render_target`.
3. **`InputBackend`** -- Input: `poll_events() -> Vec<InputEvent>`.
4. **`NetworkBackend`** -- Networking: `listen`, `accept`, `connect`,
   `tls_provider`.
5. **`AudioBackend`** -- Audio: `load_track`, `play`, `pause`, `stop`, `volume`,
   `stream`, plus queries.

### Drawing-state contract (added 2026-10)

The traits leave no room for backends to disagree on clip, translation and
render-target state. Until 2026-10 they did: SDL and UE5 ignored the
translation in `set_clip_rect` while WASM and PSP applied it, WASM
intersected repeated `set_clip_rect` calls, and no backend saved the clip
and translate stacks across a render-target bind although the trait doc
promised it. Every backend (SDL, UE5, WASM, PSP, the headless harness
wrapper) now implements the following, documented as rustdoc on the traits:

| Operation | Contract |
|---|---|
| Draw calls | Coordinates are offset by the current translation (`push_translate`) on the bound surface |
| `set_clip_rect` | Absolute: in surface pixels, **not** translated. **Replaces** the active clip; does not intersect. Zero area rejects every draw |
| `reset_clip_rect` | Removes the clip; always safe, also with no clip set |
| `push_clip_rect` / `pop_clip_rect` | Translated and intersected with the previously pushed clip; pop re-applies the previous one (or none). `current_clip_rect` reports the top of this stack |
| Mixing the two | One active clip, last call wins: `set_clip_rect` does not touch the push stack, the next push/pop re-applies the stack's clip |
| `clear` | Writes the color, alpha included, to every pixel of the bound surface, **ignoring the clip and without blending** |
| `bind_render_target` | The target starts with no clip and no translation (its own coordinates); `viewport_size` reports its size |
| `unbind_render_target` | Restores the parent's clip stack, translate stack and applied clip exactly as at the bind |
| `composite_render_target[_premultiplied]` | Into the bound surface, under its clip; the destination is untranslated surface pixels; the target is **scaled** to `dst_w` x `dst_h` |
| `read_render_target` | `dst.len()` must equal `w * h * 4` of the target; anything else is an error |

Why these choices:

- **Absolute, replacing `set_clip_rect`.** Its main caller, the browser
  display list, computes absolute clips itself, intersects them with the
  parent, and calls `set_clip_rect` again on every `PopClip` to widen the
  clip back. That only works with replace semantics. The translated,
  nesting variant already exists as `push_clip_rect`, so the two calls cover
  both needs without overlapping.
- **Clip-independent, unblended `clear`.** The vector-layer bake cache
  clears a freshly bound target to `Color::TRANSPARENT`. A blended clear of
  a transparent color is a no-op, so on WASM that clear only worked because
  new canvases happen to start transparent. SDL's `SDL_RenderClear` and
  UE5's `SoftwareBuffer::clear` already behaved this way.
- **Fresh state per bind.** Render targets have their own coordinate space
  starting at `(0, 0)`. An inherited translation or clip would be in the
  parent's coordinates and therefore wrong. Restoring on unbind means a
  `pop_clip_rect` after the unbind pops the parent's clip, not a stale one.
  `oasis_types::backend::stacks::DrawState` is the shared save/restore
  helper.
- **Scaling composites.** SDL and WASM scale through the GPU / Canvas
  `drawImage`, so UE5 now resamples (nearest-neighbour, like its blits)
  instead of asserting `dst == src` in debug builds and drawing 1:1 in
  release builds.

SDL scales render targets with its default linear filter and the software
rasterizer samples nearest, so a scaled composite differs at colour seams.
`oasis_test_backend::conformance` records that as a documented tolerance.
The conformance scenarios `clip_set_is_absolute_and_replaces`,
`clip_set_widens_again`, `clear_ignores_clip_and_alpha`,
`render_target_bind_resets_and_restores_state`,
`render_target_composite_scales` and
`render_target_premultiplied_matches_direct` pin the contract on SDL and UE5
(`cargo test -p oasis-backend-sdl --test backend_parity`). WASM has no
automated pixel test yet. Its clip handling keeps exactly one canvas
`save()` level per applied clip (`WasmBackend::apply_clip`), because Canvas
2D can only intersect clips, never replace them.

## Rationale

- **Granularity.** Not all platforms support all subsystems (PSP user-mode
  has no network, for instance; UE5 does have audio through
  `Ue5AudioBackend` and the FFI's `oasis_audio_*` exports). By splitting traits, backends implement only
  what applies. Core code accepts `Option<&dyn AudioBackend>` where audio is
  optional.
- **Single responsibility.** Each trait is focused. `SdiCore` is about
  required drawing. `SdiBackend` adds optional accelerated methods. `InputBackend`
  is about events. They can be implemented by different structs if needed.
- **Two-tier rendering.** `SdiCore` has 13 required methods that every backend
  must implement. `SdiBackend`'s nine extensions add 53 default-implemented
  methods for shapes, gradients, text styling, textures, clip/transform
  stacks, vector graphics, batching, and render targets — all falling back
  to `SdiCore` calls. The shape defaults emit spans from the same generators
  the SDL and UE5 rasterizers use, so they are pixel-exact, not just
  approximate. Backends progressively override individual extensions for
  hardware acceleration without having to stub the rest.
- **Organisation, not vtables.** Consumers take `&mut dyn SdiBackend`,
  whose vtable holds every supertrait method, and no extension trait is used
  as a separate bound or trait object. The split groups the methods by
  capability; it has no vtable-size benefit.

## Alternatives Considered

- **Single `Backend` trait** with all methods. Rejected: forces backends to
  stub out irrelevant methods (UE5 audio, PSP networking).
- **Capability queries** (`has_audio() -> bool`). Rejected: runtime checks are
  error-prone. Type system (trait presence) is better.
- **No traits, conditional compilation only.** Rejected: prevents testing core
  code without a real backend (see the mocks in `oasis-test-backend`).

## Consequences

- Core code uses `&mut dyn SdiBackend` for rendering, never SDL/PSP directly.
- Because the traits live in `oasis-types`, the SDL and UE5 backend crates
  depend only on foundation crates (`oasis-types`, plus `oasis-net`,
  `oasis-audio`, `oasis-rasterize` and `oasis-shader` for SDL), not on
  `oasis-core`. Linking a backend therefore does not pull in the browser or
  QuickJS, and `oasis-app`'s `javascript` feature really is optional.
- `oasis-app` composes all four traits into a single application state.
- Recording and mock backends live in `oasis-test-backend` (`MockSdiCore`,
  `RecordingBackend` with its `DrawCommand` log, `ClipAuditBackend`, and the
  `conformance` scenarios).
- `SdiBackend` cannot be implemented directly (blanket impl). A new backend
  implements `SdiCore`, the nine extension traits (empty impls allowed;
  `oasis_types::impl_sdi_defaults!(T)` writes them, `; except A, B` skips the
  ones it overrides) and `InputBackend`.
- Four backends implemented: SDL3 (desktop), WASM (browser), UE5 (game engine), PSP (hardware).
- A sixth trait, `ClipboardBackend`, handles copy/paste with an `InMemoryClipboard` fallback.
- Shared rasterization in `oasis_types::rasterize`: the span generators for
  circles, rings, rounded rects, thick lines and even-odd polygons (used by
  the trait defaults, the SDL backend and `oasis-rasterize::SoftwareBuffer`,
  which re-exports them) and `PixelSink` + `rasterize_triangle` (used by UE5
  and `SoftwareBuffer`).
