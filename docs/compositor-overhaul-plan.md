# Compositor Overhaul — Design & Implementation Plan

**Status:** Draft (design doc, pre-implementation)
**Tracking branch:** `feat/compositor-overhaul-plan`
**Backlog item:** [`docs/browser-backlog.md` → "Epic: Compositor overhaul"](browser-backlog.md)
**Effort estimate:** 2–3 weeks of focused work, split across ~6 PRs.

---

## 1. Problem statement

Several CSS properties are parsed today but cannot be painted because
`SdiBackend` is purely immediate-mode — every primitive lands directly on
the framebuffer with no intermediate buffer. The properties below all
need the *same* missing primitive (render-to-texture + composite-back),
so a single architectural change unlocks them together:

| Property | Parsing state | Paint state |
|---|---|---|
| `mix-blend-mode` (16 modes) | parsed → `ComputedStyle.mix_blend_mode` | ignored |
| `background-blend-mode` | parsed | ignored |
| `backdrop-filter` | parsed → `ComputedStyle.backdrop_filters` | ignored (no readback) |
| `isolation: isolate` | parsed → `ComputedStyle.isolation` enum | does not force a stacking context yet |
| `filter` on layout boxes | parsed | only `paint/filters.rs` per-color approximation; no real Gaussian blur, drop-shadow, etc. |
| `will-change: transform/opacity/filter` | parsed → `will_change_transform` bool | hint only; does not promote to a layer |
| `mask-*` (8 longhands) | **not parsed at all** | n/a |

Today's "compositor" (`crates/oasis-browser/src/paint/display_list.rs`)
is a display-list player that batches rects/text and merges strips. It
already has `PushLayer { opacity }` / `PopLayer` items
(`display_list.rs:136-138`) and an opacity stack
(`display_list.rs:641`), but those just modulate alpha during replay —
no offscreen surface is ever created.

### Why one epic, not seven small ones

Every property in the table needs the same primitive: *render the
contained subtree into an offscreen surface, then composite that surface
back into the parent with a blend mode and/or a filter chain and/or a
mask*. Adding the primitive once and then enabling each property in a
follow-up PR is dramatically cheaper than seven parallel attempts.

---

## 2. Existing architecture (relevant pieces)

### 2.1 Backend traits

`crates/oasis-types/src/backend/` is split into:

- `sdi_core.rs` — `SdiCore`, 13 required methods (init, clear, blit,
  fill_rect, draw_text, swap_buffers, load_texture, destroy_texture,
  set_clip_rect, reset_clip_rect, measure_text, read_pixels, shutdown).
- `sdi_backend.rs:30-41` — `SdiBackend`, marker super-trait that
  combines `SdiCore` with eight focused extension traits via blanket
  bounds.
- `extensions.rs` — the eight extension traits (`SdiShapes`,
  `SdiGradients`, `SdiAlpha`, `SdiText`, `SdiTextures`,
  `SdiClipTransform`, `SdiVector`, `SdiBatch`). Each method has a
  default implementation that delegates to `SdiCore` primitives;
  backends override for acceleration. `SdiTextures` is the closest
  prior art for what we're adding (`extensions.rs:474-522`).
- `types.rs:179-180` — `pub struct TextureId(pub u64);` opaque handle
  pattern. New `RenderTargetId(pub u64)` will mirror it.
- `stacks.rs` — `ClipStack` (`stacks.rs:109-150`) and `TranslateStack`
  (`stacks.rs:19-84`) already encode the "scoped state with push/pop"
  pattern that render targets need.

### 2.2 Browser paint pipeline

- `crates/oasis-browser/src/paint/record.rs` walks the layout tree and
  emits `DisplayItem`s into a `DisplayList` (no backend calls during
  record).
- `crates/oasis-browser/src/paint/display_list.rs` defines the
  `DisplayItem` enum (`display_list.rs:28-157`) — 14+ variants
  including `PushClip`/`PopClip`, `PushLayer`/`PopLayer`,
  `PushSticky`/`PopSticky`, `BlurHint`, plus the drawables (`FillRect`,
  `DrawText`, `Blit`, `Gradient`, `BorderEdge`, `Shadow`, …).
- `display_list.rs:632-850` — `replay()` walks items, maintains
  opacity/clip/sticky stacks, batches consecutive rects via
  `SdiBatch::submit_rect_batch` and consecutive text via
  `submit_text_batch`. **This is the integration point for render
  targets.**
- `display_list.rs:360-451` — `compact()` / `optimize()` /
  `merge_vertical_strips()` / `eliminate_occluded()` strip-merge and
  occlusion-eliminate the display list before replay.
- `crates/oasis-browser/src/paint/mod.rs:679-708` —
  `creates_stacking_context()` triggers: positioned + non-auto z-index,
  `opacity < 1.0`, non-empty transforms, non-empty filters,
  `will-change: transform/opacity/filter`. **`isolation: isolate` is
  not currently in this list and must be added.**
- `crates/oasis-browser/src/paint/filters.rs:15-108` — `apply_filters()`
  is a per-*color* approximation: it dims/desaturates pixels of solid
  fills and never reaches per-pixel post-processing. Real Gaussian
  blur, drop-shadow, hue-rotate fidelity, etc. all need readback.

### 2.3 Backend prior art

| Backend | Texture storage | Render-target headroom |
|---|---|---|
| SDL3 (`crates/oasis-backend-sdl/src/lib.rs:83`) | `HashMap<u64, Texture<'static>>` | Trivial — `SDL_TEXTUREACCESS_TARGET` is built into SDL3, just wire it. |
| WASM (`crates/oasis-backend-wasm/src/renderer.rs:51-67`) | `HashMap<u64, TextureData>` where each entry already wraps an `HtmlCanvasElement` | Trivial — every texture is already an offscreen canvas. Drawing into one is a `getContext('2d')` away. |
| UE5 (`crates/oasis-backend-ue5/src/renderer.rs:38-42`) | `Vec<Option<Texture>>` of `Rc<Vec<u8>>` over an RGBA `SoftwareBuffer` | Easy — allocate additional `SoftwareBuffer` instances. Memory-friendly via `Rc`. |
| test-backend (`crates/oasis-test-backend/src/lib.rs`) | none | Trivial — record commands, assert ordering. |
| PSP (`crates/oasis-backend-psp/src/textures.rs:11-88`) | `VolatileAllocator` bump-allocates from heap or volatile (4MB extra region on PSP-2000+); textures 16-byte aligned, power-of-two-padded | **Hard.** ~900KB free VRAM after framebuffers + 1MB GU command buffer; main-RAM render targets + GU blit-back is the only realistic path. |

### 2.4 CSS parsing state

Confirmed via `crates/oasis-browser/src/css/values/`:

- `mix_blend_mode: BlendMode` — parsed, stored. `BlendMode` enum
  (`css/values/types.rs:858`) covers Normal/Multiply/Screen/Overlay/
  Darken/Lighten/ColorDodge/ColorBurn/HardLight/SoftLight/Difference/
  Exclusion/Hue/Saturation/Color/Luminosity.
- `backdrop_filters: Vec<FilterFunction>` — parsed, stored. Same
  filter-function vocabulary as `filter`.
- `isolation: Isolation` — parsed, stored (`css/values/types.rs:818`,
  values `Auto`/`Isolate`).
- `will_change_transform: bool` — parsed, stored.
- `mask-*` (`mask-image`, `mask-position`, `mask-size`, `mask-repeat`,
  `mask-clip`, `mask-origin`, `mask-composite`, `mask-mode`) — **not
  parsed**. Confirmed via grep on `css/`: zero matches.

---

## 3. Proposed design

### 3.1 New trait: `SdiRenderTargets`

Add a new extension trait next to `SdiTextures` in
`crates/oasis-types/src/backend/extensions.rs`, and add it to the
`SdiBackend` super-trait bounds in `sdi_backend.rs:30-41`.

```rust
/// Opaque handle for an offscreen render target. Mirrors `TextureId`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct RenderTargetId(pub u64);

pub trait SdiRenderTargets: SdiCore {
    /// Allocate an offscreen RGBA8 surface. Returns `Err` if the
    /// backend cannot satisfy the request (e.g. PSP out of VRAM).
    fn create_render_target(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<RenderTargetId, BackendError> {
        let _ = (width, height);
        Err(BackendError::Unsupported("render targets"))
    }

    /// Redirect subsequent draw calls into the given target. Saves
    /// (and clears) clip + scissor state. Nestable; backends maintain
    /// their own bind stack.
    fn bind_render_target(&mut self, id: RenderTargetId) -> Result<(), BackendError> {
        let _ = id;
        Err(BackendError::Unsupported("render targets"))
    }

    /// Pop the most recent bind. After the outermost pop, draws go
    /// back to the framebuffer.
    fn unbind_render_target(&mut self) -> Result<(), BackendError> {
        Err(BackendError::Unsupported("render targets"))
    }

    /// Composite the given render target into the currently bound
    /// surface (framebuffer or another target).
    fn composite_render_target(
        &mut self,
        id: RenderTargetId,
        dst: Rect,
        blend: BlendMode,
        opacity: f32,
    ) -> Result<(), BackendError> {
        let _ = (id, dst, blend, opacity);
        Err(BackendError::Unsupported("render targets"))
    }

    /// Read pixels back from a render target into a caller-supplied
    /// RGBA8 buffer. Required for `backdrop-filter`. Backends that
    /// cannot read back (PSP for some configurations) return Err and
    /// the caller falls back to a static-blur shim.
    fn read_render_target(
        &mut self,
        id: RenderTargetId,
        dst: &mut [u8],
    ) -> Result<(), BackendError> {
        let _ = (id, dst);
        Err(BackendError::Unsupported("render-target readback"))
    }

    fn destroy_render_target(&mut self, id: RenderTargetId) -> Result<(), BackendError>;

    /// Capability probe used by the browser to decide between full
    /// readback (true) and a static-blur shim (false) on a per-frame
    /// basis. Default: false.
    fn supports_render_target_readback(&self) -> bool {
        false
    }
}
```

Design notes:

- Default impls return `BackendError::Unsupported`; the *only* required
  method is `destroy_render_target` (so a backend that opts in can't
  forget cleanup). All other methods are overridable.
- `BlendMode` already exists in `oasis-types` — re-use it; do not
  invent a backend-private blend enum.
- The trait deliberately exposes a *bind stack* rather than a
  one-render-target-at-a-time API. `mix-blend-mode` on a child of a
  `backdrop-filter` parent needs nesting.
- Readback is its own capability flag because PSP can do offscreen
  rendering but cannot afford a full-frame `read_pixels` per frame for
  backdrop-filter. The browser will gracefully fall back.

### 3.2 New `DisplayItem` variants

Extend the enum in `crates/oasis-browser/src/paint/display_list.rs:28-157`:

```rust
PushCompositingLayer {
    bounds: Rect,         // pixel rect on the parent surface
    opacity: f32,
    blend: BlendMode,
    needs_backdrop: bool, // true for backdrop-filter / mix-blend-mode != Normal
    filters: Vec<FilterFunction>,         // applied to the layer at composite time
    backdrop_filters: Vec<FilterFunction>,// applied to the readback before src is drawn
    mask: Option<MaskParams>,
},
PopCompositingLayer,
```

`PushLayer { opacity }` / `PopLayer` (the existing variants at
`display_list.rs:136-138`) will be **kept as the fast path** for the
common "opacity < 1.0, no blend mode, no filter" case so we don't pay
for an offscreen surface when we don't need one. The recorder picks
between the two based on the computed style.

`MaskParams` is a small struct holding `image: TextureId`,
`position`/`size`/`repeat`/`clip`/`origin`/`composite`/`mode` enums —
mirroring the CSS longhands.

### 3.3 Recording: when to push a compositing layer

In `crates/oasis-browser/src/paint/record.rs`, the existing per-box
paint function (`record_box`, ~`record.rs:110+`) currently emits
`PushLayer { opacity }` for `opacity < 1.0`. Replace that branch with a
dispatch:

```text
needs_layer  = opacity < 1.0
            || mix_blend_mode != Normal
            || !backdrop_filters.is_empty()
            || isolation == Isolate
            || !filters.is_empty()         // box-level filter, not color-only
            || mask_image.is_some()
            || will_change_transform       // promote-to-layer hint

if needs_layer && (mix_blend_mode != Normal || backdrop_filters || filters || mask) {
    emit PushCompositingLayer { ... }
} else if needs_layer {
    emit PushLayer { opacity }   // existing fast path
}
```

`creates_stacking_context()` in `paint/mod.rs:679-708` must be updated
in lock-step to add `isolation == Isolate` and to make the
`will-change` branch promote (it already does, but the comment should
note the layer promotion).

### 3.4 Replay: composite path

In `display_list.rs:632-850`, `replay()`:

1. On `PushCompositingLayer`, if the backend supports render targets:
   - Allocate an `RGBA8` target sized to `bounds`.
   - If `needs_backdrop`, copy the parent's pixels under `bounds` into
     the new target via `read_render_target` + a CPU-side filter chain
     pass (for the backdrop only).
   - `bind_render_target(id)` and translate the coordinate system so
     the contained items draw at the right offset.
   - Push onto a `Vec<ActiveLayer>` so nested layers compose correctly.
2. Draw contained items normally — they go into the offscreen.
3. On `PopCompositingLayer`:
   - `unbind_render_target()`.
   - Apply the layer's `filters` chain (CPU pass over the offscreen
     pixels for now; SDL3 may grow a shader path later).
   - `composite_render_target(id, bounds, blend, opacity)` to write the
     result back to the parent.
   - `destroy_render_target(id)` (or recycle into a per-frame pool —
     see §3.7).
4. **Fallback**: if `supports_render_target_readback()` is false and
   `needs_backdrop`, replace `backdrop-filter: blur(N)` with a static
   tinted overlay (the same approximation `paint/filters.rs:15-108`
   already uses for color-level blur). `mix-blend-mode` falls back to
   `Normal`. The page still renders; just without the effect.

### 3.5 Mask painting

Once the compositor exists, `mask-image` is a destination-in composite:
draw the masked content into a render target, draw the mask alpha into
a second target (or the alpha channel of the first), composite into the
parent. The eight `mask-*` longhands map onto existing background
machinery — `mask-position`/`-size`/`-repeat`/`-clip`/`-origin` reuse
the same parsing helpers as `background-position` etc.

This work is gated behind compositor landing and ships in its own PR
(see §6).

### 3.6 PSP plan

The PSP is the only backend where this is genuinely hard. Plan:

- **Phase A — VRAM-only small targets.** Allocate render targets out of
  the existing `VolatileAllocator`
  (`crates/oasis-backend-psp/src/textures.rs:35-88`). Cap individual
  target dimensions at 256×256 (131KB at 16bpp) and refuse anything
  larger. This is enough for typical "blurred card" effects.
- **Phase B — Main-RAM targets with GU blit-back.** When VRAM is
  exhausted, fall back to allocating in main heap RAM, doing software
  composition, and blitting the final result back via
  `sceGuCopyImage`. Slower but unbounded in size.
- **Backdrop-filter:** report `supports_render_target_readback() =
  false` for the time being. The browser falls back to the static
  tinted overlay. We can revisit once the streaming-video work proves
  out a reliable readback pipeline.
- **Stencil/shader-free blend:** the 16 blend modes get implemented as
  a small CPU compositor that operates on `ARGB8888` buffers in main
  RAM (the same data we'd be feeding to GU anyway). PSP-1000 will pay
  the CPU cost; PSP-2000+ uses volatile RAM to keep allocations off
  the main heap.
- **`mpeg_vsh370.prx` / video co-existence:** the compositor must not
  touch VRAM regions reserved for the streaming video frame buffer
  during TV Guide playback. Add an assertion in the volatile allocator
  that fails fast if a render-target alloc would overlap.

If Phase A turns out to be too constrained (i.e. real pages routinely
need targets larger than 256×256), we revisit and may simply disable
the compositor on PSP and ship the no-op fallback. **That is an
acceptable outcome** — PSP correctness should not block desktop
launch.

### 3.7 Render-target pool

A per-frame pool keyed on `(width, height)` lives on the
`PaintContext` and recycles targets across frames. Without it, every
`mix-blend-mode` element thrashes `create_render_target` /
`destroy_render_target` per frame. The pool releases targets that
weren't requested for N frames.

This is not in the trait — it's a browser-side helper that calls
through to the trait. Backends don't need to know.

---

## 4. Risks and open questions

1. **Coordinate-system gotchas.** Drawing into an offscreen and then
   compositing back means the contained items have to be re-emitted
   with translated coordinates. The `TranslateStack`
   (`stacks.rs:19-84`) is the right tool but every backend's clip
   handling needs to be re-checked for "what state is saved/restored
   on bind/unbind".
2. **Strip-merge / occlusion elimination interaction.** `compact()` /
   `optimize()` / `merge_vertical_strips()` /
   `eliminate_occluded()` (`display_list.rs:360-499`) currently assume
   a single output surface. They MUST NOT merge across
   `PushCompositingLayer` / `PopCompositingLayer` boundaries — items
   inside a layer have a different destination. Add a unit test that
   asserts a `FillRect` inside a layer is never merged with one
   outside.
3. **Hover patching.** The browser patches display-item colors in
   place when hover state changes (it's how transitions stay cheap).
   When an item is inside a render target, patching the item still
   works, but the *target* has to be re-rendered. Decision: mark the
   layer dirty on any contained-item patch and re-rasterize the
   target on the next frame. Already cheap because the layer is
   small.
4. **WASM Canvas2D blend modes** — the Canvas2D
   `globalCompositeOperation` set covers all 16 CSS blend modes by
   name (`multiply`, `screen`, `overlay`, …) so the WASM mapping is
   essentially free. Confirm during implementation.
5. **SDL3 blend modes** — SDL3 only ships 5 built-in blend modes
   (`NONE`, `BLEND`, `ADD`, `MOD`, `MUL`). The remaining 11 modes need
   either a software fallback (read pixels, blend in CPU, write back)
   or a custom `SDL_ComposeCustomBlendMode`. Plan: software path
   first, custom blend mode as a follow-up if it proves too slow.
6. **`backdrop-filter` blur kernel cost.** A real Gaussian blur on a
   1024×768 region is expensive on CPU. Use a separable two-pass
   box blur (3 iterations ≈ Gaussian) and downsample the readback to
   1/4 resolution before blurring. This is the standard browser
   trick.

---

## 5. Test strategy

- **`oasis-test-backend` ordering tests.** The mock backend records
  every call. Add tests that build a tiny display list with nested
  `PushCompositingLayer` and assert the exact sequence of
  `create_render_target` / `bind_render_target` / drawing /
  `unbind_render_target` / `composite_render_target` /
  `destroy_render_target` calls.
- **Pixel goldens via SDL backend.** Add fixtures under
  `crates/oasis-browser/tests/fixtures/`:
  - `mix_blend_mode_basic.html` — 16 small panels, one per blend mode.
  - `backdrop_filter_blur.html` — fixed-position blur card over
    gradient text.
  - `mask_radial.html` — radial-gradient mask.
  - `isolation_isolate.html` — `isolation: isolate` containing a
    `mix-blend-mode` child; verify the blend stays inside the
    isolation root.
  - `filter_box_blur.html` — `filter: blur(8px)` on a box with mixed
    text/borders.
  Reuses the display-list golden harness shipped on
  `feat/browser-realworld-compat-epic`
  ([done epic](browser-backlog.md#-done-real-world-compatibility-measurement)):
  drop the compositor fixtures under
  `crates/oasis-browser/tests/fixtures/` and add them to the
  `FIXTURES` list in `tests/visual_regression.rs`, then
  `UPDATE_GOLDENS=1` to seed the expected draw-call streams.
- **PSP smoke test.** PPSSPP headless run that loads
  `mix_blend_mode_basic.html` and asserts no panic / no GU command
  buffer overflow / no VRAM exhaustion. Visual fidelity not asserted.
- **Performance microbench.** `benches/compositor.rs` measures
  per-frame cost of a page with 50 nested `mix-blend-mode` cards on
  desktop. Gate CI on regression > 30%.

---

## 6. PR breakdown

Each PR is independently mergeable and CI-green.

### PR 1 — Trait surface (1–2 days)

- Add `SdiRenderTargets` trait + `RenderTargetId` to
  `crates/oasis-types/src/backend/extensions.rs`.
- Add to `SdiBackend` super-trait bounds in `sdi_backend.rs`.
- All backends get a default impl (via the trait defaults) — no
  behavior change.
- Add `BackendError::Unsupported(&'static str)` if it doesn't exist.
- Tests: trait compiles and is object-safe; existing backends still
  build clean.

### PR 2 — test-backend implementation (1 day)

- Implement render-target tracking in `crates/oasis-test-backend` so
  unit tests can assert ordering.
- Add ordering tests in `crates/oasis-test-backend/tests/`.

### PR 3 — Display-list variants + recorder (3–4 days)

- Add `PushCompositingLayer` / `PopCompositingLayer` to
  `display_list.rs:28-157`.
- Add `creates_compositing_layer()` helper in `paint/mod.rs` next to
  `creates_stacking_context()`.
- Update `record_box` in `paint/record.rs` to dispatch between
  `PushLayer` (fast path) and `PushCompositingLayer` (slow path).
- `replay()` in `display_list.rs:632-850` handles the new variants
  with the trait's default-Err implementations — items inside an
  unsupported layer fall through to "draw without effect" so behavior
  is unchanged on backends that haven't opted in yet.
- Add `isolation: isolate` to `creates_stacking_context()`.
- Update `compact()` / `optimize()` / `merge_vertical_strips()` /
  `eliminate_occluded()` to NOT cross compositing-layer boundaries.
  Add the regression test.
- Add the per-frame render-target pool helper.

### PR 4 — SDL + WASM + UE5 backend implementations (3–4 days)

- SDL: `SDL_TEXTUREACCESS_TARGET` + `SDL_SetRenderTarget`. Software
  fallback for the 11 blend modes SDL3 doesn't ship natively.
- WASM: store render targets as additional `HtmlCanvasElement`s in the
  same `HashMap`; binding switches the active 2D context. Use
  `globalCompositeOperation` for blend modes.
- UE5: allocate additional `SoftwareBuffer`s; CPU compositor in Rust.
- Test-backend gains a "real" mode that captures pixel output for
  visual goldens.
- Land the `mix-blend-mode_basic.html` and `isolation_isolate.html`
  fixtures with goldens.

### PR 5 — Box-level filter + backdrop-filter (2–3 days)

- Wire `filters: Vec<FilterFunction>` from `ComputedStyle` through to
  `PushCompositingLayer.filters`.
- CPU filter chain implementation: blur (separable two-pass box,
  3 iterations), drop-shadow, hue-rotate, brightness, contrast,
  saturate, grayscale, sepia, invert, opacity. Reuse formulas from
  the existing color-level `paint/filters.rs:15-108`.
- `backdrop-filter` path: `read_render_target` of parent under
  `bounds` → filter chain → draw into the layer → contained items on
  top.
- Land `backdrop_filter_blur.html` and `filter_box_blur.html` goldens.

### PR 6 — Mask properties (2–3 days)

- Add CSS parsing for the eight `mask-*` longhands in
  `crates/oasis-browser/src/css/values/`. Reuse background-position /
  background-size / background-repeat helpers.
- Add `MaskParams` struct and wire through `PushCompositingLayer`.
- Implement destination-in composite path in `replay()`.
- Land `mask_radial.html` golden.

### PR 7 — PSP backend (1 week, optional for desktop launch)

- Phase A small VRAM targets in
  `crates/oasis-backend-psp/src/textures.rs`.
- 16 blend modes via CPU compositor on ARGB8888 main-RAM buffers.
- Backdrop-filter remains disabled (returns `false` from
  `supports_render_target_readback`).
- PPSSPP smoke test.
- If Phase A proves too constrained, fall back to leaving PSP on the
  no-op default impl and document it as a known limitation.

---

## 7. Out of scope

- **GPU shader paths.** SDL3 has `SDL_GPUShader` but we're not
  introducing a shader pipeline as part of this epic. CPU paths are
  fine for the property set we care about.
- **`will-change` heuristics.** We promote on `will-change:
  transform/opacity/filter` only when the developer explicitly asked.
  No auto-promotion based on animation detection.
- **Tile-based compositing.** Compositing layers are full-rect; no
  tiling, no dirty-rect partial updates inside a layer.
- **Hardware video overlay interactions.** PSP video playback and
  compositor coexist via the assertion in §3.6; they do not share
  surfaces.

---

## 8. Definition of done

The epic is complete when:

1. All five `tests/fixtures/` goldens above pass on SDL3, WASM, and
   UE5 backends.
2. `mix-blend-mode`, `background-blend-mode`, `backdrop-filter`,
   `isolation`, box-level `filter`, and the eight `mask-*` longhands
   are listed in the supported-properties section of
   `crates/oasis-browser/src/lib.rs`.
3. CI gates on the new visual goldens and the new performance bench.
4. PSP either ships Phase A or has a documented "compositor disabled
   on PSP" note in CLAUDE.md.
5. `docs/browser-backlog.md` "Epic: Compositor overhaul" section is
   removed or marked complete.
