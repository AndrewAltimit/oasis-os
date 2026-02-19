# ADR-003: Backend Trait Design (Four Traits)

**Status:** Accepted
**Date:** 2025-02-12

## Context

OASIS_OS must render to SDL2, Unreal Engine 5, and PSP hardware. Each platform
has different rendering APIs, input mechanisms, networking stacks, and audio
subsystems. We need an abstraction that:

- Keeps core code platform-agnostic
- Allows backends to implement only what they support
- Minimizes trait object overhead on PSP (333 MHz, 32 MB RAM)

## Decision

We define **four separate backend traits** in `oasis-types`:

1. **`SdiBackend`** -- Rendering: `clear`, `fill_rect`, `draw_text`, `blit`,
   `load_texture`, `swap_buffers`, `read_pixels`, plus 14 extended methods.
2. **`InputBackend`** -- Input: `poll_events() -> Vec<InputEvent>`.
3. **`NetworkBackend`** -- Networking: `listen`, `accept`, `connect`,
   `tls_provider`.
4. **`AudioBackend`** -- Audio: `load_track`, `play`, `pause`, `stop`, `volume`,
   `stream`, plus queries.

## Rationale

- **Granularity.** Not all platforms support all subsystems. UE5 has no audio.
  PSP user-mode has no network. By splitting traits, backends implement only
  what applies. Core code accepts `Option<&dyn AudioBackend>` where audio is
  optional.
- **Single responsibility.** Each trait is focused. `SdiBackend` is about
  drawing. `InputBackend` is about events. They can be implemented by different
  structs if needed.
- **Default methods.** `SdiBackend` provides default implementations for
  extended rendering (rounded rects, circles, gradients) that fall back to
  `fill_rect`. Backends can override for hardware acceleration.
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
- New backends only need to implement `SdiBackend` + `InputBackend` at minimum.
