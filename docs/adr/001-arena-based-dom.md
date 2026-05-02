# ADR-001: Arena-Based DOM

**Status:** Accepted
**Date:** 2025-02-12
**Last reviewed:** 2026-05-02 — still current. The browser engine uses an arena-based DOM today (`crates/oasis-browser/src/html/dom.rs`) with `NodeId` indices, exactly as described.

## Context

The browser engine needs a DOM representation for HTML parsing, CSS cascade,
layout, and painting. Two common approaches exist:

1. **Reference-counted nodes** (`Rc<RefCell<Node>>`) -- traditional tree with
   parent/child pointers. Each node owns its children. Traversal follows pointers.
2. **Arena-based nodes** -- all nodes stored in a flat `Vec<Node>`. Relationships
   expressed via indices (`NodeId = usize`). No reference counting.

## Decision

We use an **arena-based DOM** (`Vec<Node>`) with index-based relationships.

`NodeId` is a plain `usize` index. `Document` owns `Vec<Node>`. Each `Node` has
`parent: Option<NodeId>` and `children: Vec<NodeId>`.

## Rationale

- **No reference counting overhead.** On PSP (MIPS, 333 MHz), `Rc`/`RefCell`
  costs are significant. Arena indexing is a single array access.
- **Cache-friendly layout.** Nodes are contiguous in memory. Sequential traversal
  (CSS cascade, layout) benefits from cache locality.
- **Simple lifetime model.** All nodes live as long as the `Document`. No cycles,
  no weak references, no `RefCell` borrow panics.
- **Easy serialization.** The flat structure maps directly to indices in the
  styles array (`Vec<Option<ComputedStyle>>`), avoiding the need for a HashMap.
- **Predictable allocation.** A single `Vec::with_capacity` at parse time avoids
  many small allocations.

## Tradeoffs

- **Node removal is O(n)** since we don't compact the arena. In practice, our
  browser engine builds the DOM once and never removes nodes.
- **Parent/child navigation requires indexing** rather than following pointers.
  This is a minor ergonomic cost.

## Consequences

- `Document::add_node()` appends to the arena and returns a `NodeId`.
- CSS styles are stored as `Vec<Option<ComputedStyle>>` indexed by `NodeId`.
- Layout boxes store `Option<NodeId>` to link back to DOM nodes.
- No garbage collection or cycle detection needed.
