# Layout

The layout engine turns a styled DOM (`Document` + per-node
`ComputedStyle`) into a `LayoutBox` tree with absolute positions and
sizes. It is a single, recursive top-down walk — there is no separate
"compute intrinsic widths" pass like Gecko or Blink. Instead, every
formatting context shares one helper that resolves dimensions on the
fly.

## Files

```text
src/layout/
├── mod.rs            entry points: build_layout_tree, layout_block_incremental
├── box_model.rs      LayoutBox, BoxType, Dimensions, Rect, EdgeSizes
├── block/            block / inline-block / list-item layout
│   ├── mod.rs
│   ├── float.rs      float context, BFC root tracking
│   └── tests.rs
├── inline.rs         inline run building, line boxes
├── text.rs           word breaking, soft hyphen, bidi run detection
├── text_cache.rs     CachingMeasurer + SharedTextCache
├── flex.rs           CSS flexbox: main / cross axis, grow / shrink, gap
├── grid.rs           CSS grid: explicit + implicit tracks, area resolution
├── table.rs          table-wrapper / table-row / table-cell, auto width
├── multicol.rs       multi-column balancing (partial)
└── replaced.rs       intrinsic-size resolution for <img>, <video>, etc.
```

## `LayoutBox`

```rust
pub struct LayoutBox {
    pub box_type: BoxType,
    pub style: Rc<ComputedStyle>,
    pub dimensions: Dimensions,         // border / padding / margin / content rect
    pub children: Vec<LayoutBox>,
    pub dom_node: Option<NodeId>,       // back-reference for hit testing
    pub dirty: bool,                    // incremental layout flag
    // ... line boxes, float info, scroll offset, etc.
}

pub enum BoxType {
    Block,
    Inline,
    InlineBlock,
    Flex,
    Grid,
    TableWrapper,
    TableRow,
    TableCell,
    ListItem { marker: ListMarker },
    Replaced(ReplacedContent),
    Anonymous,
}
```

`Dimensions` contains the standard CSS box rectangles: `content`,
`padding`, `border`, `margin`, plus their resolved edge sizes. The
painter operates exclusively on these rectangles — it never inspects
`ComputedStyle` directly during paint replay.

## Build pass

`build_layout_tree(document, viewport)` walks the DOM and constructs
the `LayoutBox` tree by dispatching on each element's `display`:

- `block` / `list-item` → block layout
- `inline` / `inline-block` → inline run inside the parent block
- `flex` / `inline-flex` → flex container
- `grid` / `inline-grid` → grid container
- `table` / `table-row` / `table-cell` → table layout
- `none` → skipped entirely (no `LayoutBox` is created)

Anonymous boxes are inserted as needed: an inline that ends up directly
inside a `display: flex` is wrapped in an anonymous `Block`; a
`<td>` outside a `<tr>` gets wrapped in an anonymous `TableRow`.

## Block layout

Block layout in `block/mod.rs` follows CSS 2.1:

- Width is resolved against the containing block.
- Height is resolved by stacking children, honoring margin collapsing.
- Floats are tracked in a `FloatContext` (`block/float.rs`) and
  influence sibling line boxes.
- `position: relative` shifts the painted box without affecting
  siblings; `position: absolute` and `position: fixed` are laid out
  against the nearest positioned ancestor (or the viewport).
- `position: sticky` is implemented via a paint-time offset cache
  (`PushSticky` / `PopSticky` display items) so the layout pass does
  not need to know the current scroll position.

## Inline layout

`inline.rs` collects consecutive inline-level boxes into line boxes,
breaking on:

- Hard line breaks (`<br>`).
- Word boundaries when the next word would overflow the line.
- Emergency breaks inside long words when `overflow-wrap: break-word`
  or `word-break: break-all` is set.
- Soft hyphens (U+00AD) — the line break renders a visible `-` glyph.

The line breaker is bidi-aware to the level needed for Hebrew / Arabic
/ Syriac runs (it detects the predominant direction and reverses run
order at line break time). It is **not** a full UAX #9 implementation.

## Text measurement

Width is the dominant cost in inline layout. Two layers of caching keep
it cheap:

1. **`CachingMeasurer`** (`text_cache.rs`) wraps the backend's
   `TextMeasurer` and memoises `(text_hash, font_size) → width` in a
   `HashMap`.
2. **`SharedTextCache`** persists the inner `HashMap` between layout
   passes so successive scrolls / resizes / hover toggles do not
   re-measure unchanged runs. The cache is cleared only on a zoom
   change (which invalidates every measurement).

The cache is keyed by a 64-bit hash of the text plus a `u16` font size,
which makes lookups effectively free for steady-state pages.

## Flex, grid, table

- **Flex** (`flex.rs`) implements the CSS Flexible Box Module Level 1:
  `flex-direction`, `flex-wrap`, `justify-content`, `align-items`,
  `align-self`, `align-content`, `flex-grow`, `flex-shrink`,
  `flex-basis`, `gap`, `order`. It does not implement
  `align-tracks` / `justify-tracks`.
- **Grid** (`grid.rs`) supports `grid-template-{columns,rows,areas}`,
  `grid-auto-{columns,rows,flow}`, named lines, `repeat()`, `minmax()`,
  `fr` units, `gap`, and explicit / implicit grids. Subgrid is **not**
  implemented.
- **Table** (`table.rs`) implements `border-collapse: separate` and the
  HTML table model with auto-sizing columns. `border-collapse: collapse`
  is parsed and stored but the painter still draws separate borders.

## Incremental layout

`layout_block_incremental` only re-runs layout for subtrees whose
`dirty` flag is set. Mutations from the cascade or DOM mark dirty
upwards; sibling boxes that depend on the dirty box (because they share
a containing block whose width changed) are also marked.

The current implementation is **conservative** — when in doubt, it
walks the whole subtree. The point is to avoid relayout on
hover / scroll / focus changes, which it does well.

## Tests

- `src/layout/block/tests.rs` — margin collapsing, float context,
  positioning.
- `src/layout/text_cache.rs` — caching invariants.
- `tests/browser_integration.rs` — full pipeline regression tests for
  block, table, list, and gradient pages.
- `benches/layout_engine.rs` — layout build / relayout throughput.
