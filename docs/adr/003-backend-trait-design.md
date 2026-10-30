# ADR-003: Backend Trait Design (Split Backend Traits)

**Status:** Accepted
**Date:** 2025-02-12
**Last reviewed:** 2026-10-29 — decision still current; counts, the removed
opt-in traits and the shared span rasterizer updated below to match current
source. See `crates/oasis-types/src/backend/`.

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
