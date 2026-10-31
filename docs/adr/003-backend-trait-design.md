# ADR-003: Backend Trait Design (Split Backend Traits)

**Status:** Accepted
**Date:** 2026-02-09 (initial release; recorded 2026-02-19)
**Last reviewed:** 2026-10-30. The decision is still current. Counts, the
removed opt-in traits and the shared span rasterizer were updated to match
the source, the drawing-state contract every backend follows was added,
and the extension surface was trimmed (see "Surface trim" below). See
`crates/oasis-types/src/backend/`.

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

1. **`SdiCore`** -- Required rendering: 12 required methods (`init`,
   `clear`, `blit`, `fill_rect`, `draw_text`, `swap_buffers`, `load_texture`,
   `destroy_texture`, `set_clip_rect`, `reset_clip_rect`, `read_pixels`,
   `shutdown`) plus two provided ones: `measure_text` (the bitmap font's
   width; SDL overrides it for skin TTF fonts) and `viewport_size` (the
   bound surface's size; defaults to 480x272, every real backend
   overrides it).
2. **`SdiBackend`** -- A marker super-trait with no methods of its own. It
   bundles `SdiCore` plus eight extension traits via a blanket impl, so any
   type that satisfies `SdiCore` and all eight extensions automatically
   implements `SdiBackend`. The eight extensions are `SdiShapes`,
   `SdiGradients`, `SdiText`, `SdiTextures`, `SdiClipTransform`,
   `SdiVector`, `SdiBatch`, and `SdiRenderTarget`, totalling 45
   default-implemented methods that fall back to `SdiCore` primitives
   (shapes 8, gradients 2, text 7, textures 2, clip/transform 9,
   vector 6, batch 2, render target 9). `SdiShapes` builds on
   `SdiClipTransform` (for `draw_bounds`, below), `SdiVector` on
   `SdiShapes` and `SdiBatch` on `SdiText`; every other extension builds
   on `SdiCore` alone. `SdiBatch` is `submit_rect_batch`/`submit_text_batch`,
   batched rect and text geometry submission (used by the browser's
   display-list replay to reduce draw call count); a batch draws exactly
   what its items drawn one by one (`fill_rect`, `draw_text_styled`) would. The former opt-in traits `SdiBlendMode` and
   `SdiGeometry` had no implementation or caller outside tests and were
   removed in 2026-10; blend modes are expressed through
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
| `push_clip_rect` / `pop_clip_rect` | Translated and intersected with the previously pushed clip; pop re-applies the previous one (or none). `current_clip_rect` reports the top of this stack as an `Option<ClipRect>` in surface pixels |
| Mixing the two | One active clip, last call wins: `set_clip_rect` does not touch the push stack, the next push/pop re-applies the stack's clip |
| `clear` | Writes the color, alpha included, to every pixel of the bound surface, **ignoring the clip and without blending** |
| `bind_render_target` | The target starts with no clip and no translation (its own coordinates); `viewport_size` reports its size |
| `unbind_render_target` | Restores the parent's clip stack, translate stack and applied clip exactly as at the bind |
| `composite_render_target[_premultiplied]` | Into the bound surface, under its clip; the destination is untranslated surface pixels; the target is **scaled** to `dst_w` x `dst_h` |
| `read_render_target` | `dst.len()` must equal `w * h * 4` of the target; anything else is an error |
| Using a bound target | Binding a target that is already on the bind chain, compositing it, reading it back or destroying it is an error, and leaves the bind state as it was (UE5 and WASM take a bound target out of storage; SDL checks the chain) |
| Unknown ids | `destroy_texture` of an id that is not live (never loaded, or already destroyed) fails with `BackendError::TextureNotFound`, as `destroy_render_target` fails with `RenderTargetNotFound`; so does a blit of it |
| Off-surface shapes | Shapes cost what their visible part costs: the span generators take the visible bounds (`SdiClipTransform::draw_bounds` for the defaults), so a line from `-1e9` to `1e9` draws as fast as one across the screen |
| Arcs | A sweep beyond one full turn is clamped to one turn (`backend::clamp_sweep`), filled or stroked, default or override |

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
`conformance::render_target_misuse` checks the bound-target rule on SDL,
UE5 and `RecordingBackend`; `conformance::texture_misuse` checks the
unknown-texture rule on SDL and UE5 (and every test double has a unit test
for it). The conformance scenarios
`clip_set_is_absolute_and_replaces`,
`clip_set_widens_again`, `clip_set_and_push_stack_are_independent`,
`clear_ignores_clip_and_alpha`,
`render_target_bind_resets_and_restores_state`,
`render_target_composite_scales` and
`render_target_premultiplied_matches_direct` pin the contract on SDL and UE5
(`cargo test -p oasis-backend-sdl --test backend_parity`). The test
doubles follow the same render-target rule: oasis-test-backend's
`RecordingBackend` saves the clip/translate stacks and viewport size on
bind and restores them on unbind. WASM has no
automated pixel test yet. Its clip handling keeps exactly one canvas
`save()` level per applied clip (`WasmBackend::apply_clip`), because Canvas
2D can only intersect clips, never replace them.

- **Unknown ids are errors.** `destroy_texture` used to accept any id
  silently while `destroy_render_target` rejected unknown ones. One rule
  for both, and the stricter one: a double destroy is a caller bug (with
  UE5's slot reuse it can even free another owner's texture), and an error
  surfaces it in tests where silence hid it. Callers that release
  best-effort (`let _ = backend.destroy_texture(..)`) are unaffected. The
  test doubles (`RecordingBackend`, `NullBackend`, `MockSdiCore`,
  `ClipAuditBackend`) track live ids so they reject the same calls.
- **Visible-only span generation.** `thick_line_rows`, `polygon_rows` and
  `triangle_rows` take a `ClipRect` of bounds, visit only the rows inside
  it and clip spans to its columns; the thick line computes each row's
  extent from the closed form of the Bresenham walk instead of walking
  every step. The trait defaults pass `draw_bounds()` (the bound surface
  moved back by the current translation; deliberately not the clip, which
  `set_clip_rect` can replace behind the stack's back), SDL and
  `SoftwareBuffer` their surface. SDL draws thin lines through the same
  spans: its own line walk visits every point of the segment.

### Surface trim (2026-10)

A review of every override and call site in the workspace (PSP included)
removed what no backend implemented and no caller used, so the trait
surface describes what backends actually do:

| Removed | Why | Replacement |
|---|---|---|
| `SdiAlpha` (`fill_rect_alpha`, `dim_screen`, `viewport_size`) | The two drawing methods were one-line `fill_rect` wrappers with no production caller; the trait mixed alpha and viewport concerns | `fill_rect(.., color.with_alpha(a))`; `viewport_size` moved to `SdiCore` as a provided method |
| `SdiBatch::begin_batch` / `flush_batch` | Reserved no-op hooks that no backend ever overrode | `submit_rect_batch` / `submit_text_batch` |
| `SdiVector::fill_polygon_gradient` | The only implementation ignored the end colour | `oasis-vector` fills `VectorOp::PolygonGradient` row by row with the interpolated colour |
| `SdiText::text_metrics` + `TextMetrics` | No caller | `measure_text_extents`, `font_ascent` |
| `ArcParams`, `ClipStack::current_tuple` | Internal to one backend / replaced by a typed return | `current_clip_rect() -> Option<ClipRect>` |

The trim also moved more shapes onto shared span generators, so the
defaults and the desktop backends agree by construction:
`rasterize::triangle_rows` (the `fill_triangle` default, SDL and
`SoftwareBuffer`), `rasterize::stroke_rect_parts` (non-overlapping stroke
bands for all three) and a `fill_arc` default that fills one polygon
instead of a triangle fan (the fan double-blended every spoke of a
translucent wedge). Coordinates in the shared helpers (`ClipRect`
intersection, `TranslateStack`, `edge_x`, circle and rounded-rect radii)
are computed in `i64` or saturate, so far off-screen or huge sizes clip
instead of wrapping or panicking in debug builds.

A second pass (overhaul 3) applied the same rule again:

| Change | Why |
|---|---|
| `SdiTextures::blit_sub_tinted` / `blit_flipped` removed | No caller outside tests; SDL, UE5 and WASM each carried an implementation and a conformance scenario for them |
| `SdiBatch` builds on `SdiText`; `submit_text_batch` defaults to one `draw_text_styled` per item | SDL and UE5 both overrode it with exactly that, and WASM double-struck bold where its single-span path drew a real bold glyph, so batched and unbatched text differed. `conformance::batch_matches_singles` pins it |
| `SdiCore::measure_text` provided (bitmap font) | Every backend but SDL, and most test doubles, wrote the same `bitmap_measure_text` body |
| `ClipStack::push` returns the applied `ClipRect` (`ClipRect::EMPTY` for no overlap); `ClipStack::push_translated`; `ClipPush` removed | Every backend mapped `ClipPush::Empty` to the same zero rectangle by hand |
| `backend::unsupported(op)`; the `AudioBackend` streaming defaults return `BackendError::Unsupported` | They returned free-text strings, unlike the `SdiRenderTarget` defaults, so callers could not tell "fall back" from a failure |
| `texture_not_found` takes a `TextureId` | It took a raw `u64` while `render_target_not_found` took the typed id |

Positional `i32`/`u32` arguments were kept: the call sites (browser display
list, chrome, apps) already hold loose coordinates, and a `Rect`-taking
signature would churn hundreds of calls for no behavioural gain. The typed
`ClipRect` return of `current_clip_rect` is the one place a struct was
clearly better, because callers compare and intersect the result.

## Rationale

- **Granularity.** Not all platforms support all subsystems (PSP user-mode
  has no network, for instance; UE5 does have audio through
  `Ue5AudioBackend` and the FFI's `oasis_audio_*` exports). By splitting traits, backends implement only
  what applies. Core code accepts `Option<&dyn AudioBackend>` where audio is
  optional.
- **Single responsibility.** Each trait is focused. `SdiCore` is about
  required drawing. `SdiBackend` adds optional accelerated methods. `InputBackend`
  is about events. They can be implemented by different structs if needed.
- **Two-tier rendering.** `SdiCore` has 12 required methods that every backend
  must implement. `SdiBackend`'s eight extensions add 45 default-implemented
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
  implements `SdiCore`, the eight extension traits (empty impls allowed;
  `oasis_types::impl_sdi_defaults!(T)` writes them, `; except A, B` skips the
  ones it overrides) and `InputBackend`.
- Four backends implemented: SDL3 (desktop), WASM (browser), UE5 (game engine), PSP (hardware).
- A sixth trait, `ClipboardBackend`, handles copy/paste with an `InMemoryClipboard` fallback.
- Shared rasterization in `oasis_types::rasterize`: the span generators for
  circles, rings, rounded rects, thick lines, triangles and even-odd
  polygons plus `stroke_rect_parts` (used by the trait defaults, the SDL
  backend and `oasis-rasterize::SoftwareBuffer`, which re-exports the
  generators) and `PixelSink` + `rasterize_triangle` (used by UE5 and
  `SoftwareBuffer`).

## Update (2026-10-30)

Corrections to statements above that predate the current code (the
decision itself is unchanged):

- `NetworkBackend` also has a provided `listen_loopback(port)`; the remote
  terminal and the FTP-like transfer server use it unless they are given a
  PSK / password, and the MCP server always does.
- `AudioBackend` is `init`, `load_track`, `play`, `pause`, `resume`, `stop`,
  `set_volume` / `get_volume`, `is_playing`, `position_ms`, `duration_ms`,
  `unload_track` and `shutdown`, plus a provided streaming path
  (`load_streaming`, `feed_data`, `streaming_can_accept`,
  `streaming_queued_ms`, `finalize_streaming`, `feed_pcm_f32`);
  `load_streaming`, `feed_data` and `feed_pcm_f32` default to
  `BackendError::Unsupported`.
- The PSP EBOOT does have networking: `PspNetworkBackend` implements
  `NetworkBackend` over `sceNetInet` (WiFi, infrastructure mode). The
  "PSP networking" examples under Rationale and Alternatives describe the
  early state; the platform that cannot open TCP sockets today is WASM
  (`WasmNetworkBackend` returns errors; the browser's own `fetch` is used
  instead).
