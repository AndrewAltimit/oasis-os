# Paint & compositor

There is one renderer. `record.rs` walks the `LayoutBox` tree and emits
a flat `DisplayList` of draw items; the list is compacted, optimized and
replayed against the `SdiBackend`. There is no GPU-side scene graph, no
tile cache and no threaded compositor: every frame is a list replay into
a backend-owned framebuffer, with offscreen render targets for the
effects that need them.

The same path serves everything that paints:

- `BrowserWidget::paint` (`widget/paint.rs`) records when layout,
  focus or a large scroll invalidates the cached list, and otherwise
  replays the cached list with a scroll delta or a dirty rectangle.
- `paint::paint` (exported as `internals::paint_page`) records and
  replays once. The visual-regression goldens, the corpus test, the
  paint benchmark, the triage example and `paint/tests.rs` all use it,
  so they guard the shipped renderer.

## Files

```text
src/paint/
├── mod.rs              PaintViewport, LinkRegion, build_display_list(), paint()
├── record.rs           layout tree → DisplayItems (paint order, transforms, clips, layers)
├── display_list/
│   ├── mod.rs          DisplayItem enum + DisplayList container
│   ├── optimize.rs     compact() / optimize(): zero-size removal, strip merging, occlusion
│   ├── replay.rs       replay() / replay_dirty(): clip/opacity/layer/sticky stacks, batching
│   └── mask.rs         mask-* rasterizers applied when a compositing layer pops
├── filter_chain.rs     per-pixel filter chain + CPU blend for compositing layers
├── filters.rs          per-color filter approximations used while recording
├── stacking.rs         stacking-context / compositing-layer / positioned predicates
├── transforms.rs       transform-origin, perspective-origin, 3D detection
├── clip_path.rs        clip-path bounding rect, rect intersection
├── background.rs       background-image tiling, gradient-stop sampling
├── markers.rs          ordered list-marker formatting (incl. @counter-style)
├── replaced.rs         object-fit geometry
└── tests.rs            record + replay tests against MockBackend
```

`build_display_list(layout, viewport, links, &mut list, nested_scroll)`
records into `list` and runs `compact()` + `optimize()`;
`DisplayList::replay(backend, dx, dy, base_clip)` and
`replay_dirty(backend, dirty, dx, dy, base_clip)` play it back.

## Display items

```rust
pub enum DisplayItem {
    FillRect { x, y, w, h, color, node_id },
    FillRoundedRect { x, y, w, h, radius, color, node_id },
    FillPolygon { points: [(i32, i32); 4], color, node_id }, // transformed quads
    DrawText { text, x, y, font_size, color, bold, italic, width, node_id, web_font_id },
    Blit { texture, x, y, w, h },
    BlitSub { texture, src_*, dst_* },                        // atlas sprites
    Gradient { x, y, w, h, style },
    BorderEdge { x, y, w, h, color, style, horizontal },
    Shadow { x, y, w, h, blur, spread, offset_x, offset_y, color, radius },
    Svg { element: Rc<SvgElement>, x, y, width, height },
    Canvas { state: Rc<RefCell<CanvasState>>, x, y, w, h },
    PushClip { x, y, w, h }, PopClip,
    PushLayer { opacity }, PopLayer,                          // opacity fast path
    PushCompositingLayer { bounds, opacity, blend, filters, mask },
    PopCompositingLayer,
    PushSticky { natural_y, box_height, top_px, bottom_px, visible_viewport_h },
    PopSticky,
}
```

`node_id` tags let `patch_node_colors` recolor a hovered element in
place without re-recording.

## Record pass

`record_box` follows CSS 2.1 appendix E for each box: box shadow,
background, borders, outline, then its content — negative z-index
stacking contexts, normal-flow children, positioned `z-index: auto`
children, then non-negative stacking contexts sorted by z-index. Inline
boxes record their text runs (with shadows and `text-decoration` in its
`solid` / `double` / `dashed` / `dotted` / `wavy` style); list items
record their marker right-aligned against the content edge; replaced
boxes record images (object-fit), form controls, `<hr>`, and SVG /
canvas items.

Per box the recorder also:

- culls boxes outside the (scroll-buffered) viewport unless their
  descendants may overflow into it;
- opens a `PushSticky` group for `position: sticky`;
- pushes a clip for `overflow: hidden | scroll | auto` and for
  `clip-path` (the shape's bounding box), and applies the box's nested
  scroll offset to its descendants;
- opens an opacity layer, or a compositing layer for
  `mix-blend-mode`, `isolation`, box-level `filter`, `mask-*`,
  `will-change` and `backdrop-filter`.

`visibility: hidden` skips the box's own painting but not its
children. SVG and canvas are ordinary items, so they stack under later
overlays (dropdowns, modals, sticky headers), clip, scroll inside
nested scroll containers and honour visibility. A canvas item shares
its `CanvasState` with the JS bindings, so a cached list replays the
canvas's current commands.

## Transforms

An element's transform applies to its own box and to its descendants.
Translate-only transforms move the integer offset. Rotations, scales
and skews become an `AffineTransform2D` built around the screen-space
transform origin; backgrounds under a non-trivial transform record a
`FillPolygon` quad. Borders, text and images under a rotation are not
rotated (they move with the translation only).

3D transforms under a `perspective:` ancestor, inside a
`transform-style: preserve-3d` context, or on a preserve-3d element
take the screen-space path: the recorder builds
`T(vp) * Persp(d) * T(-vp) * T(origin) * local * T(-origin)` (the
vanishing point is the perspective-origin resolved against the
ancestor's border box), composes it with any inherited preserve-3d
matrix, and projects background corners through the full 4x4 matrix
so steep rotations paint a true trapezoid. Corners that project behind
the camera drop the fill. Inside a preserve-3d context, children are
painted back to front by projected Z; children with an explicit
`z-index` opt out (CSS Transforms 2 §6.1) and keep the stacking tiers.
`backface-visibility: hidden` culls a subtree whose effective matrix
faces away.

## Compaction and optimization

After recording, `compact()` drops zero-size items and merges
horizontally adjacent same-color `FillRect`s; `optimize()` merges
vertical strips and drops opaque `FillRect`s fully covered by a later
opaque fill at the same clip / sticky / layer depth (never inside a
translucent layer). Neither pass merges across a compositing-layer
boundary, and SVG / canvas items break adjacency, so nothing is merged
or culled across them.

## Replay

Replay walks the items in order and keeps:

- a clip stack, intersecting each `PushClip` with its parent (or the
  base clip, the widget's content viewport) and restoring the parent on
  `PopClip`;
- an opacity stack, multiplied into item colors;
- a layer stack: a `PushCompositingLayer` binds an offscreen render
  target when the backend supports one, and the pop composites it back
  with its blend mode and opacity, running the filter chain and mask on
  CPU readback first where `supports_render_target_readback()`. Without
  render targets (PSP) a compositing layer degrades to plain opacity;
- a sticky stack that recomputes each sticky group's offset for the
  replay scroll position, so scrolling can replay a cached list.

Consecutive `FillRect`s are submitted through
`SdiBatch::submit_rect_batch` and consecutive same-style `DrawText`s
through `submit_text_batch`. SVG and canvas items call
`svg::paint_svg` / `canvas::paint_canvas`; under a translucent opacity
layer they draw into a render target composited at the layer opacity
when the backend supports render targets, and opaque otherwise.

`replay_dirty` replays only the items whose bounds intersect a dirty
rectangle (clip, layer and sticky markers are always replayed).

## Widget caching

`BrowserWidget::paint` records with the viewport extended by
`scroll.buffer_zone`, so small scrolls replay the cached list with a
translation and shift the link map instead of re-recording. It
re-records when layout changed, the list is stale or empty, a full
repaint was requested, or the scroll delta leaves the buffer zone.
Visual-only animations (colors, `visibility`) patch item colors by node
id and replay only the dirty rectangles.

## Known limitations

- Rotated / scaled / 3D-transformed boxes only transform their
  background; borders, text and images follow the translation only.
- `clip-path` clips to the shape's bounding box (no stencil).
- `backdrop-filter` isolates the element into a compositing layer, but
  the backdrop is not sampled or filtered.
- Box-level `filter` colors are approximated per color while recording
  and, on render-target backends with readback, the per-pixel chain
  also runs on the layer, so color-only filters on the element's own
  background and text apply twice there.

## Tests and benches

- `src/paint/tests.rs` — record + replay against `MockBackend`
  (stacking order, transforms, 3D, SVG / canvas order and scrolling,
  decoration styles).
- `src/paint/display_list/*` — optimizer and replay unit tests.
- `tests/visual_regression.rs` — draw-call goldens for the corpus at
  800x600 and 480x272 (`UPDATE_GOLDENS=1` to re-bless).
- `tests/browser_corpus.rs` — corpus smoke tests through the same path.
- `benches/paint.rs` — record + replay on synthetic pages into a null
  backend.
