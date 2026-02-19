# ADR-002: Virtual File System Abstraction

**Status:** Accepted
**Date:** 2025-02-12

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
  `MemoryVfs` cannot escape its virtual root. `RealVfs` normalizes and rejects
  `..` traversal.
- **Testability.** All file operations are testable with `MemoryVfs` -- no temp
  directories, no cleanup, deterministic behavior.
- **Embedding.** FFI consumers populate `MemoryVfs` via `oasis_add_vfs_file()`
  before any OS code runs. No host filesystem exposure.

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
