# Layout

The layout engine turns a styled DOM (`Document` + per-node
`ComputedStyle`) into a `LayoutBox` tree with absolute positions and
sizes. It is a single, recursive top-down walk. Where a container
needs an item's intrinsic width (a content-sized flex item, an
intrinsic grid track) it asks `intrinsic::max_content_width`, which
computes it from the box tree without laying the item out and caches
it per box for the current layout pass.

## Files

```text
src/layout/
├── mod.rs            module list
├── box_model.rs      LayoutBox, BoxType, Dimensions, Rect, EdgeSizes
├── block/            block / inline-block / list-item layout
│   ├── mod.rs
│   ├── tree_builder.rs  entry points: build_layout_tree, relayout_tree
│   ├── float.rs      float context, BFC root tracking
│   └── tests.rs
├── inline.rs         inline run building, line boxes
├── intrinsic.rs      max-content widths without layout, per-pass cache
├── text.rs           word breaking, soft hyphen, bidi run detection
├── text_cache.rs     CachingMeasurer + SharedTextCache
├── flex.rs           CSS flexbox: main / cross axis, grow / shrink, gap
├── grid.rs           CSS grid: auto-repeat, placement, auto-flow, track sizing
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
    pub sizing: SizingState,            // flex / grid forced sizes, max-content cache
    pub inline_source: Option<Box<Vec<LayoutBox>>>, // inline content an IFC was built from
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

Anonymous boxes are inserted as needed: inline children of a
`display: flex` / `grid` container become anonymous `Block` items (one
per inline element, carrying its flex / grid properties; one per text
run; whitespace-only runs dropped); a `<td>` outside a `<tr>` gets
wrapped in an anonymous `TableRow`.

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
  `flex-basis`, `gap`, `order`. A definite basis (in the item's
  `box-sizing`) is the base size; items without one size to fit-content,
  their max-content width (`intrinsic.rs`) clamped to the space
  available. A column container without a definite height is as tall as
  its items (and ignores their basis). A row item is laid out once, at
  its resolved size, passed as `LayoutBox::sizing.forced_width` (never
  written into the style); a column item again only if flexing changed
  its height. Positioning translates the laid-out subtree, so nesting
  is linear in depth (`benches/layout_engine.rs` `layout_nested_flex`).
  It does not implement `align-tracks` / `justify-tracks`.
- **Grid** (`grid.rs`) supports `grid-template-{columns,rows,areas}`,
  `grid-auto-{columns,rows}`, `grid-auto-flow: [row | column] || dense`,
  `gap`, and explicit / implicit grids. Track lists are parsed by
  `oasis-css/src/values/grid.rs` with a paren-aware tokenizer: nested
  `repeat(3, minmax(0, 1fr))`, multi-track `repeat(2, 1fr 2fr)`, and
  `px` / `%` / `em` / `rem` / `fr` / `auto` / `min-content` /
  `max-content` / `fit-content()` / `minmax()` tracks.
  `repeat(auto-fill | auto-fit, ...)` is resolved at layout time from
  the container size (the largest repetition count whose tracks and
  gutters fit, minimum 1); `auto-fit` then collapses empty repeated
  tracks. Placement accepts line numbers (negative lines count from the
  end of the explicit grid), `span n`, the `a / b` slash syntax on
  `grid-row` / `grid-column` / `grid-area`, and area names (`grid-area:
  main`, or `main-start` / `main-end` lines). Track sizing follows the
  shape of §11 (base sizes, maximize, flexible tracks with minimum
  freezing, auto-track stretch); `fr` rows resolve against a definite
  container `height`. Intrinsic tracks size from the items' max-content
  contributions (min-content is not distinguished); items are stretched
  to their cell through `sizing.forced_width`. Subgrid,
  `[line-name]` references and `align-items` stretch of grid items are
  **not** implemented.
- **Table** (`table.rs`) implements `border-collapse: separate` and the
  HTML table model with auto-sizing columns. `border-collapse: collapse`
  is parsed and stored but the painter still draws separate borders.

## Relayout

`relayout_tree` lays an existing tree out again in place (the widget
does after images decode, so canvas state and textures survive). It is a
full layout and must give exactly the geometry of a fresh
`build_layout_tree`; layout therefore keeps nothing between passes that
it does not recompute: inline layout replaces `children` with line
fragments but lays out from `inline_source`, and flex / grid sizes go
through `sizing`, not the style. `src/relayout_tests.rs` checks every
corpus fixture (same-width relayout, a detour through another width,
image sizes changing). There is no partial dirty-subtree relayout; the
one that existed skipped repositioning clean subtrees, floats and
`clear`.

## Tests

- `src/layout/block/tests.rs` — margin collapsing, float context,
  positioning.
- `src/layout/text_cache.rs` — caching invariants.
- `tests/browser_integration.rs` — full pipeline regression tests for
  block, table, list, and gradient pages.
- `src/relayout_tests.rs` — relayout vs fresh layout over the corpus.
- `benches/layout_engine.rs` — layout build throughput, nested flex.
