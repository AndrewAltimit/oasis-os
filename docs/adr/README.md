# Architecture Decision Records

Short records of the load-bearing architectural decisions in OASIS_OS: the
context, the decision, its tradeoffs or rejected alternatives, and the consequences. Each
ADR carries a **Last reviewed** line noting when it was last checked against
the source.

| ADR | Title | Status | Decided |
|-----|-------|--------|---------|
| [001](001-arena-based-dom.md) | Arena-based DOM (`NodeId` indices into a node arena in `oasis-browser`) | Accepted | 2025-02-12 |
| [002](002-vfs-abstraction.md) | Virtual file system abstraction (`Vfs` trait; `MemoryVfs`, `RealVfs`, `GameAssetVfs`) | Accepted | 2025-02-12 |
| [003](003-backend-trait-design.md) | Backend trait design (`SdiCore` + extension traits, input / network / audio traits) | Accepted | 2025-02-12 |
| [004](004-psp-two-binary-architecture.md) | PSP two-binary architecture (EBOOT shell + kernel-mode PRX overlay) | Accepted | 2025-06-01 |
| [005](005-toml-skin-system.md) | TOML skin system (manifest / layout / features / theme files, built-in + external skins) | Accepted | 2025-02-12 |

## Adding an ADR

Copy the structure of an existing record (`# ADR-00N: Title`, then
**Status**, **Date**, **Last reviewed**, and the Context / Decision /
Rationale / Tradeoffs or Alternatives Considered / Consequences sections), number it
sequentially, and add a row to the table above and to
[docs/README.md](../README.md). A decision that is replaced later keeps its
file with status `Superseded by ADR-00M`.
