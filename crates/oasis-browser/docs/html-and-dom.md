# HTML & DOM

## Pipeline

```text
bytes
  │
  ▼
html::tokenizer ──▶ Token stream (StartTag, EndTag, Text, Comment, DocType)
  │
  ▼
html::tree_builder ──▶ html::dom::Document
```

The two stages live in:

- `src/html/tokenizer/` — character-level tokenizer
- `src/html/tree_builder/` — token-level tree construction
- `src/html/dom.rs` — the arena-backed DOM data model

The tree builder is loosely modelled on the WHATWG HTML5 parser
algorithm: insertion modes, the active formatting elements list, foster
parenting for malformed table content, and implicit `<html>` / `<head>`
/ `<body>` insertion. It is *not* a literal spec implementation — we
trade some completeness for code size and binary footprint.

## DOM representation

The DOM is a single arena (`Vec<Node>`) addressed by `NodeId`, not a
tree of `Rc<Node>`. This was an explicit decision recorded in
[`adr/001-arena-based-dom.md`](../../../docs/adr/001-arena-based-dom.md):
flat memory layout, cheap clones (just an integer), trivial parent /
child / sibling traversal.

```rust
pub type NodeId = usize;

pub struct Document {
    nodes: Vec<Node>,
    free_list: Vec<NodeId>,           // recycled slots
    id_index: HashMap<String, NodeId>, // O(1) #id lookup
    // ...
}

pub struct Node {
    kind: NodeKind,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

pub enum NodeKind {
    Document,
    Element(ElementData),
    Text(String),
    Comment(String),
}

pub struct ElementData {
    tag: TagName,
    attributes: Vec<Attribute>,
}
```

`TagName` is an enum with about 50 known HTML tags plus an
`Unknown(String)` fallback. Comparing tags by enum variant is much
faster than string comparison and lets the cascade build dense lookup
tables.

## Tokenizer notes

- The tokenizer is fully byte-driven; it never allocates a `String` for
  tag names — it slices into the input buffer and interns into
  `TagName` once per tag.
- Numeric and named character references (`&#x2014;`, `&mdash;`) are
  resolved here, not in the tree builder.
- Script and style content uses raw text mode — the tokenizer does not
  reinterpret `<` inside `<script>` until it sees `</script>`.

## Tree builder quirks

- **Foster parenting** — text and elements that appear inside `<table>`
  but outside a cell get reparented to before the table, matching WHATWG
  behaviour. Without this, real-world pages with stray whitespace inside
  tables render with unexpected extra inline content.
- **Implicit closing** — `<p>` and friends auto-close on certain start
  tags (e.g. opening a `<div>` inside a `<p>`).
- **Optional `<head>` / `<body>`** — both tags are inserted on demand if
  the document jumps straight into content.
- **`<template>`** is parsed but its contents are not currently exposed
  to layout (open issue).

## Mutation

The DOM is mutated in two situations:

1. **During parsing** — the tree builder appends nodes as tokens arrive.
2. **From JavaScript** — `js_dom.rs` mutates the DOM through
   `Document::create_element`, `append_child`, `set_attribute`,
   `set_text_content`, `replace_child`, etc.

After any mutation the affected subtree is marked dirty. The next call
to `relayout_if_dirty()` rebuilds layout for those subtrees only — the
DOM itself is not rebuilt.

## Why an arena?

- **No reference cycles.** Parent / child are just integers, so we never
  worry about `Rc<RefCell<>>` cycles or weak pointers.
- **Cache friendly.** All nodes live in one `Vec`, so traversals stay
  close to the prefetcher.
- **Cheap snapshots.** Cloning a `NodeId` is a `usize` copy. We exploit
  this in the JS bindings, which only ever pass `NodeId` across the
  Rust↔QuickJS boundary.
- **Reusable storage.** When a navigation replaces the document, we can
  reuse the existing `Vec<Node>` rather than dropping and reallocating.

## Tests

- `src/html/tokenizer/tests.rs` — token-level edge cases (entity
  decoding, raw text mode, `<![CDATA[`, comment recovery).
- `src/html/tree_builder/tests.rs` — insertion modes, foster parenting,
  malformed input, the WHATWG "deep nesting no panic" property test.
- `tests/browser_integration.rs` — end-to-end parses (heading hierarchy,
  nested lists, tables) that double as smoke tests for the DOM.
