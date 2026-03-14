# ADR-003: Backend Trait Design (Five Traits)

**Status:** Accepted
**Date:** 2025-02-12

## Context

OASIS_OS must render to SDL3, WebAssembly (Canvas 2D), Unreal Engine 5, and PSP
hardware. Each platform has different rendering APIs, input mechanisms, networking
stacks, and audio subsystems. We need an abstraction that:

- Keeps core code platform-agnostic
- Allows backends to implement only what they support
- Minimizes trait object overhead on PSP (333 MHz, 32 MB RAM)

## Decision

We define **five separate backend traits** in `oasis-types`:

1. **`SdiCore`** -- Required rendering: 13 methods (`init`, `clear`, `blit`,
   `fill_rect`, `draw_text`, `swap_buffers`, `load_texture`, `destroy_texture`,
   `set_clip_rect`, `reset_clip_rect`, `measure_text`, `read_pixels`, `shutdown`).
2. **`SdiBackend`** -- Extends `SdiCore` with 39 optional accelerated primitives
   (shapes, gradients, text styling, texture ops, clip/transform stacks, vector
   graphics, batching). Also split into 8 extension traits: `SdiShapes`,
   `SdiGradients`, `SdiAlpha`, `SdiText`, `SdiTextures`, `SdiClipTransform`,
   `SdiVector`, `SdiBatch`.
   All have default implementations that fall back to `SdiCore` methods.
3. **`InputBackend`** -- Input: `poll_events() -> Vec<InputEvent>`.
4. **`NetworkBackend`** -- Networking: `listen`, `accept`, `connect`,
   `tls_provider`.
5. **`AudioBackend`** -- Audio: `load_track`, `play`, `pause`, `stop`, `volume`,
   `stream`, plus queries.

## Rationale

- **Granularity.** Not all platforms support all subsystems. UE5 has no audio.
  PSP user-mode has no network. By splitting traits, backends implement only
  what applies. Core code accepts `Option<&dyn AudioBackend>` where audio is
  optional.
- **Single responsibility.** Each trait is focused. `SdiCore` is about
  required drawing. `SdiBackend` adds optional accelerated methods. `InputBackend`
  is about events. They can be implemented by different structs if needed.
- **Two-tier rendering.** `SdiCore` has 13 required methods that every backend
  must implement. `SdiBackend` extends it with 39 optional methods for shapes,
  gradients, and batching that fall back to `SdiCore` calls. Backends can
  progressively override for hardware acceleration.
- **Minimal vtable.** Four small vtables instead of one large one. On PSP,
  indirect calls through vtables are expensive; smaller vtables mean fewer
  cache misses.

## Alternatives Considered

- **Single `Backend` trait** with all methods. Rejected: forces backends to
  stub out irrelevant methods (UE5 audio, PSP networking).
- **Capability queries** (`has_audio() -> bool`). Rejected: runtime checks are
  error-prone. Type system (trait presence) is better.
- **No traits, conditional compilation only.** Rejected: prevents testing core
  code without a real backend. `MockBackend` implements all four traits.

## Consequences

- Core code uses `&mut dyn SdiBackend` for rendering, never SDL/PSP directly.
- `oasis-app` composes all four traits into a single application state.
- `MockBackend` in `oasis-types` records all draw calls for test assertions.
- New backends only need to implement `SdiCore` + `SdiBackend` + `InputBackend` at minimum.
- Four backends implemented: SDL3 (desktop), WASM (browser), UE5 (game engine), PSP (hardware).
- A sixth trait, `ClipboardBackend`, handles copy/paste with an `InMemoryClipboard` fallback.
- Shared rasterization algorithms in `oasis-types::rasterize` (`PixelSink` trait) reduce
  per-backend implementation effort for shapes, circles, and rounded rectangles.
