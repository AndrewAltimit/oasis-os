# ADR-004: PSP Two-Binary Architecture (EBOOT + PRX)

**Status:** Accepted
**Date:** 2026-02-14 (the overlay PRX landed in #17; recorded 2026-02-19)
**Last reviewed:** 2026-10-30 — still current. `oasis-backend-psp` (EBOOT) and `oasis-plugin-psp` (PRX) both ship and are built independently. Consequences corrected: only the EBOOT crate is in the workspace `exclude` list, the EBOOT now shares the `oasis-types` glyph table, and the PRX shares state through atomics rather than a queue.

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
   audio channel for background MP3 playback. Originally targeted <64 KB; at
   v1.4.0 the stripped PRX is ~153 KB (~212 KB resident), see
   [`psp-plugin.md`](../psp-plugin.md) §Memory Budget.

## Rationale

- **No shared memory complexity.** The PRX is self-contained. It doesn't need
  IPC with the EBOOT. Both are separate use cases.
- **Minimal kernel footprint.** The PRX includes only rendering primitives,
  audio, config, and font code. No DOM, no CSS, no VFS, no terminal. This keeps
  the kernel memory footprint small (the original target was 64 KB; the PRX
  has since grown MP3/AAC/radio playback, PIP video and diagnostics, and CI
  now holds the stripped file to a 192 KiB budget).
- **Separate crate graph.** The PRX crate (`oasis-plugin-psp`) uses
  `psp = { features = ["kernel"] }` while the EBOOT uses `features = ["std"]`.
  These are incompatible feature sets -- separate crates avoid feature unification
  issues.
- **Display hook context.** The PRX's display hook runs in interrupt context
  where syscalls (file I/O, input polling) don't work. A separate kernel thread
  handles config loading and input. This architecture is natural with a small,
  focused crate.

## Tradeoffs

- **Code duplication.** The PRX carries its own copy of the bitmap glyph
  table (`oasis-plugin-psp/src/font.rs`) and basic draw primitives. This is
  intentional -- the PRX needs to be self-contained for kernel stability. The
  EBOOT builds its glyph atlas from the shared `oasis_types::bitmap_font`
  table.
- **No dynamic linking.** The PRX cannot call into the EBOOT or vice versa.
  Features added to one don't automatically appear in the other.

## Consequences

- Neither crate is a workspace member, since both need nightly + `cargo-psp`:
  `oasis-backend-psp` is listed in the root `Cargo.toml` `exclude`, and
  `oasis-plugin-psp` is a standalone package with its own `[workspace]`.
- CI builds both independently with separate `cargo psp` commands (plus the
  recovery EBOOT), all pinned to one rust-psp revision.
- The PRX uses `psp::module_kernel!()` for kernel-mode module declaration.
- The PRX finds syscalls with `psp::hook::find_function` and patches them with
  `sctrlHENPatchSyscall` to intercept display buffer swaps.
- Static atomics (`AtomicU8` / `AtomicBool` command, volume and state cells)
  share state between the display hook and the audio / config threads.
