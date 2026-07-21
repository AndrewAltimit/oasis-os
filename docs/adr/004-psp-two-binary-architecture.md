# ADR-004: PSP Two-Binary Architecture (EBOOT + PRX)

**Status:** Accepted
**Date:** 2025-06-01
**Last reviewed:** 2026-05-02 — still current. `oasis-backend-psp` (EBOOT) and `oasis-plugin-psp` (PRX) both ship and are built independently as documented.

## Context

The PlayStation Portable has a unique execution environment:

- **User-mode EBOOT.PBP** -- standard application, full access to user APIs,
  runs in its own memory space, cannot coexist with games.
- **Kernel-mode PRX** -- loaded by custom firmware (ARK-4/PRO) via
  `PLUGINS.TXT`, stays resident in kernel memory alongside games, can hook
  system calls.

We want OASIS_OS to work both as a standalone shell (replacing XMB) and as a
game overlay (visible while playing games).

## Decision

Deploy as **two separate binaries**:

1. **`oasis-backend-psp`** (EBOOT.PBP) -- full OASIS_OS shell with all apps,
   browser, terminal, file manager. Uses `oasis-core` and all workspace crates.
   Runs standalone.

2. **`oasis-plugin-psp`** (PRX) -- lightweight overlay companion. Direct
   framebuffer rendering only (no `oasis-core` dependency). Hooks
   `sceDisplaySetFrameBuf` to draw UI into the game's framebuffer. Claims a PSP
   audio channel for background MP3 playback. Under 64 KB binary.

## Rationale

- **No shared memory complexity.** The PRX is self-contained. It doesn't need
  IPC with the EBOOT. Both are separate use cases.
- **Minimal kernel footprint.** The PRX includes only rendering primitives,
  audio, config, and font code. No DOM, no CSS, no VFS, no terminal. This keeps
  the kernel memory footprint under 64 KB.
- **Separate crate graph.** The PRX crate (`oasis-plugin-psp`) uses
  `psp = { features = ["kernel"] }` while the EBOOT uses `features = ["std"]`.
  These are incompatible feature sets -- separate crates avoid feature unification
  issues.
- **Display hook context.** The PRX's display hook runs in interrupt context
  where syscalls (file I/O, input polling) don't work. A separate kernel thread
  handles config loading and input. This architecture is natural with a small,
  focused crate.

## Tradeoffs

- **Code duplication.** Font rendering and basic draw primitives exist in both
  the EBOOT backend and the PRX overlay. This is intentional -- the PRX needs
  to be self-contained for kernel stability.
- **No dynamic linking.** The PRX cannot call into the EBOOT or vice versa.
  Features added to one don't automatically appear in the other.

## Consequences

- Both crates are excluded from the workspace (`exclude` in root `Cargo.toml`)
  since they require nightly + `cargo-psp` to build.
- CI builds both independently with separate `cargo psp` commands.
- The PRX uses `psp::module_kernel!()` for kernel-mode module declaration.
- The PRX uses `psp::hook::SyscallHook` to intercept display buffer swaps.
- Lock-free communication (`psp::sync::SpscQueue`) bridges the display hook
  thread and the config/input thread.
