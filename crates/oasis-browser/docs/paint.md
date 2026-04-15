# Paint & compositor

The painter is intentionally tiny. It walks a `LayoutBox` tree, emits a
linear `DisplayList` of draw commands, and replays it through
`SdiBackend`. There is no GPU-side scene graph, no tile cache, no
threaded compositor — every frame is a list replay against a
backend-owned framebuffer.

## Files

```text
src/paint/
├── mod.rs          PaintViewport, link region tracking, replay loop
├── display_list.rs DisplayItem enum + DisplayList container
├── record.rs       walk LayoutBox tree → emit DisplayItems
├── background.rs   solid + gradient backgrounds, background-image, repeat
├── borders.rs      border-style rendering (solid, dashed, dotted, double)
├── text.rs         text run painting, decoration lines, shadows
├── replaced.rs     <img>, <video>, <hr>, <input> visual replaceds
├── shadow.rs       box-shadow + text-shadow
└── markers.rs      list-item bullets and numbers
```

The recording entry point is `record_display_list(&LayoutBox,
viewport)` and the replay entry point is
`DisplayList::replay(backend, dirty_rects)`.

## Display list

```rust
pub enum DisplayItem {
    FillRect { rect, color },
    FillRoundedRect { rect, radii, color },
    StrokeRoundedRect { rect, radii, width, color, style },
    DrawText { x, y, text, font, color, decoration },
    DrawTextRun { batch_index },          // submitted via SdiBatch::submit_text_batch
    Blit { texture, src, dst },
    BlitSub { texture, src_rect, dst_rect },
    DrawGradient { rect, gradient },
    PushClip { rect, radii },
    PopClip,
    PushSticky { offset_y },
    PopSticky,
    SetBlendMode(BlendMode),              // partial — see css-coverage.md
    SubmitRectBatch { batch_index },      // submitted via SdiBatch::submit_rect_batch
}
```

Items are stored in a single flat `Vec<DisplayItem>` per
`DisplayList`. The list is recorded once per layout and replayed
unchanged on every paint until the layout dirties again.

## Record pass

`record.rs` walks the `LayoutBox` tree in CSS 2.1 paint order:

1. Background of the current box.
2. Borders + outline.
3. Block-level descendants in document order.
4. Inline-level descendants (text runs, inline replaceds).
5. List markers.
6. Outlines (drawn after children so they sit on top).

Each box is its own atomic chunk of the display list — there is no
state machine that crosses box boundaries except for `PushClip` /
`PopClip` and `PushSticky` / `PopSticky`.

## Stacking contexts and z-index

`record.rs` honors CSS 2.1 appendix E paint order. Positioned elements
with `z-index` form a stacking context, and their descendants are
collected separately and merged into the parent's display list at the
end of the parent's paint pass. `z-index: auto` does **not** create a
stacking context — only an explicit numeric value (including `0`) does.

The implementation is straightforward: `record_stacking_context()`
recurses, collects items into a sub-list, sorts by `z-index`, then
splices the sub-list back into the parent.

## Replay & dirty rects

`DisplayList::replay(backend, dirty_rects)` iterates the items in
order. Each item:

1. Computes its bounding rectangle (cached on the item where possible).
2. Tests against the union of dirty rects. If it does not intersect any
   dirty rect, it is skipped entirely.
3. Otherwise, it issues the corresponding `SdiBackend` call.

Dirty rects are tracked at the `BrowserWidget` level. Anything that
mutates layout or marks an animation frame appends a rect; the painter
unions them, replays the relevant items, then clears the list.

## Batching

`SdiBackend` exposes two batch APIs the painter uses heavily:

- **`submit_rect_batch(rects, color)`** — many same-color fills issued
  in one call. The painter coalesces consecutive `FillRect` items with
  the same color into a single batch.
- **`submit_text_batch(batch)`** — many glyphs of the same font / size
  issued in one call. The painter coalesces consecutive `DrawText`
  items into per-font batches.

Batches are built during recording, not during replay, so a backend
that overrides `submit_rect_batch` with a single GPU draw call gets the
benefit automatically.

## Light compositor

`paint/mod.rs` runs a small set of optimisations against the recorded
list before the first replay:

- **Vertical / horizontal strip merging** — merges adjacent same-color
  fills along an axis.
- **Occluded rect elimination** — drops fills that are fully covered by
  a later opaque fill in the same clip region.
- **Clip intersection optimisation** — collapses nested clips into a
  single intersected rect when possible.

These passes are O(n) in the size of the list and run once per
recording.

## Sticky positioning

Sticky elements are recorded once at their base position. The painter
tracks the current scroll offset and, when it encounters a `PushSticky`
item, applies a per-axis offset to all items until the matching
`PopSticky`. This avoids re-recording the entire display list on every
scroll tick.

## What is not in the painter

A surprising amount of CSS is parsed and cascaded but not yet honored
by the painter. See the **Storage vs. rendering** section in
[`css-coverage.md`](css-coverage.md). Notable: `clip-path`, `mask-*`,
`backdrop-filter`, `mix-blend-mode`, `content-visibility`, and full
`scroll-snap-*` behaviour.

The 3D transform stack **is** painted end-to-end via the new
screen-space projection path:

- 3D transform functions (`rotateX/Y/Z`, `rotate3d`, `translate3d`,
  `scale3d`, `matrix3d`, `perspective(d)`) compose through the
  `Matrix3d` 4×4 pipeline in `transform.rs`.
- `perspective` and `perspective-origin` on a parent establish a
  `PerspectiveContext` in `PaintContext` that descendants pick up.
  When a 3D-transformed descendant is painted, the painter builds
  the full `T(vp) * Persp(d) * T(-vp) * T(origin) * local *
  T(-origin)` matrix in screen space and projects the box's three
  reference corners through it (with per-vertex perspective divide)
  via `Matrix3d::project_screen_rect_affine`. The result is a
  parallelogram approximation of the true trapezoid — exact for the
  top-left/top-right/bottom-left corners.
- `transform-style: preserve-3d` propagates the parent's full
  screen-space matrix to descendants via `PaintContext.preserved_3d`
  so a child's `translateZ(50px)` actually moves toward the viewer
  inside an ancestor's 3D rendering context. `flat` flushes the
  ambient matrix, matching the spec.
- `backface-visibility: hidden` is honored via the surface-normal Z
  test on the transformed front face.

Pure 2D transforms keep the existing orthographic flatten via
`AffineTransform2D::from_css_transforms`. The screen-space 3D path
also skips the long-standing `child_matrix.e/.f → tx_offset_x/y`
double-translation bug in the 2D path (gated by `needs_screen_path`)
— the existing 2D path still has that bug for backwards compat.

When wiring one of these into the painter:

1. Find the field on `ComputedStyle` (it is already there).
2. Add a new `DisplayItem` variant if the property changes per-element
   pixel output.
3. Emit it from `record.rs` at the appropriate point in the paint
   order.
4. Honor it in `replay()`. Most properties only need a backend call —
   if the backend cannot do it (e.g. `mix-blend-mode` on the SDL3
   software path), guard the call behind a capability flag.

## Tests and benches

- `tests/browser_integration.rs` — `complex_page_with_gradients_completes_within_budget`
  is the closest thing we have to a paint regression test (it asserts
  the whole pipeline completes under a wall-clock budget).
- `benches/paint.rs` — record + replay benchmarks against synthetic
  pages.
