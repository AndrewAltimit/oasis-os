# ADR-001: Arena-Based DOM

**Status:** Accepted
**Date:** 2026-02-09 (initial release; recorded 2026-02-19)
**Last reviewed:** 2026-10-30 — decision still current. The browser engine uses an arena-based DOM (`crates/oasis-browser/src/html/dom.rs`) with `NodeId` indices. Tradeoffs and Consequences updated: JavaScript mutation removes subtrees and their slots are reused through a free list, and script-built trees are depth-capped.

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

- **Removed slots are not compacted.** JS DOM mutation (`removeChild`,
  `innerHTML`, `textContent`) frees whole subtrees; their slots go on a free
  list (`Document::free_list`) that `add_node` reuses before appending, so the
  arena does not grow without bound under repeated mutation.
- **Reused ids can alias.** A `NodeId` handed to JavaScript can later name a
  different node once its slot is reused. The bindings record freed ids
  (`FreedLog` in `js_dom/node_api.rs`) and drop the stale wrappers.
- **Parent/child navigation requires indexing** rather than following pointers.
  This is a minor ergonomic cost.

## Consequences

- `Document::add_node()` reuses a free slot or appends to the arena and returns
  a `NodeId`.
- Depth is bounded: the tree builder stops nesting at `MAX_NESTING_DEPTH`
  (256), and script insertion (`appendChild`, `insertBefore`, `innerHTML`)
  refuses to place a node below `MAX_DOM_DEPTH`, because cascade, layout and
  paint recurse per level.
- CSS styles are stored as `Vec<Option<ComputedStyle>>` indexed by `NodeId`.
- Layout boxes store `Option<NodeId>` to link back to DOM nodes.
- No garbage collection or cycle detection needed.
