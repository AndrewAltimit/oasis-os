# ADR-002: Virtual File System Abstraction

**Status:** Accepted
**Date:** 2026-02-09 (initial release; recorded 2026-02-19)
**Last reviewed:** 2026-10-29 — still current. `MemoryVfs`, `RealVfs`, and `GameAssetVfs` ship today in `crates/oasis-vfs/`. Since the original decision the trait gained a shared `validate_rename` invariant check and a cross-implementation conformance suite (see "Shared invariants" below).

## Context

OASIS_OS runs on three very different platforms:

- **Desktop (SDL):** Full filesystem access via `std::fs`.
- **Unreal Engine 5:** Read-only game assets with overlay writes.
- **PSP:** Memory stick I/O via `sceIo*` syscalls.

Terminal commands like `ls`, `cat`, `mkdir` need to work identically across all
platforms. The browser engine needs to load pages from virtual paths. Apps need
to save/load state.

## Decision

We use a **`Vfs` trait** (`oasis-vfs`) as the sole file system interface.
Core code never calls `std::fs` directly. Three implementations exist:

- **`MemoryVfs`** -- in-RAM tree. Default for testing and UE5/FFI embedding.
- **`RealVfs`** -- delegates to `std::fs`. Used by the SDL desktop backend.
- **`GameAssetVfs`** -- read-only base layer + writable overlay. Used by UE5.

## Rationale

- **Platform independence.** Terminal commands work on PSP, desktop, and UE5
  without `#[cfg]` conditionals in business logic.
- **Security.** Path traversal attacks are blocked at the trait boundary. The
  `MemoryVfs` cannot escape its virtual root. `RealVfs` resolves `..`
  lexically (a VFS path cannot climb above `/`), then canonicalizes the
  deepest ancestor that exists on disk and requires it to lie inside the
  root before appending the not-yet-existing tail. A symlink inside the root
  that points outside is therefore rejected even when several path levels
  below it do not exist yet (`mkdir /link/new/x`), instead of letting
  `create_dir_all` follow it out of the sandbox.
- **Testability.** All file operations are testable with `MemoryVfs` -- no temp
  directories, no cleanup, deterministic behavior.
- **Embedding.** FFI consumers populate `MemoryVfs` via `oasis_add_vfs_file()`
  before any OS code runs. No host filesystem exposure.

## Shared invariants

Behaviour that is part of the trait contract is enforced once and tested
against every implementation:

- **`Vfs::validate_rename`** (provided method in `oasis-vfs/src/lib.rs`) is
  called by all three `rename` implementations on their normalized paths. It
  rejects renaming `/`, a missing source, a destination inside the source
  (`/a` -> `/a/b`, which would orphan the moved subtree), a missing or
  non-directory destination parent, an existing directory as destination
  (empty or not), and a directory onto an existing file. `from == to` is a
  no-op; a file may replace a file. `RealVfs` additionally compares the
  resolved on-disk paths so a symlink cannot smuggle a move into its own
  subtree, and treats a case-only rename on a case-insensitive filesystem as
  a rename rather than a collision.
- **Permissions.** `MemoryVfs` and `GameAssetVfs` require write permission on
  both parent directories for `rename` (as for `write`, `mkdir` and
  `remove`). `RealVfs` leaves permission enforcement to the OS.
- **`GameAssetVfs` layering.** Only overlay entries can be renamed; a
  directory with visible base-layer descendants cannot move; a base entry
  shadowed by the moved overlay entry stays hidden at the old path.
- **`crates/oasis-vfs/tests/vfs_conformance.rs`** runs one macro-generated
  suite against `MemoryVfs`, `GameAssetVfs` and `RealVfs` (temp dir), plus
  unix-only RealVfs symlink-escape tests (run them on Linux or in the CI
  Docker image). New trait-level behaviour gets a test there.

Callers (`mv`, the file manager, the transfer service) still keep their own
user-facing checks for nicer messages, but no longer need them for tree
integrity.

## Tradeoffs

- **Memory cost for `MemoryVfs`.** All file content is in RAM. Acceptable for
  the typical use case (small utility files, HTML pages, configs).
- **No mmap or streaming.** Large files must be fully read. The 480x272 UI
  rarely handles files larger than a few KB.

## Consequences

- Every crate that does file I/O depends on `oasis-vfs` and accepts `&mut dyn Vfs`.
- The `Environment` struct in the terminal carries `&mut dyn Vfs`.
- The FFI layer wraps a `MemoryVfs` inside the opaque `OasisInstance`.
- Path security is enforced in the `Vfs` implementations, not in callers.
