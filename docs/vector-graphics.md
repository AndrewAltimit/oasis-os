# Vector Graphics

`oasis-vector` is a resolution-independent drawing layer used for dashboard
icons, sidebar chrome, and animated background overlays. It defines a flat
`VectorOp` enum, a `VectorScene` container, an animation clock, and a renderer
that dispatches each op to the appropriate `SdiBackend` extension method.

There is no GPU pipeline of its own — the backend's existing rect / shape /
polygon / text primitives do the work. The crate's value is having a single
data-driven representation that every backend (SDL3, WASM, UE5, PSP) can
consume identically.

## Public types

| Type | Source | Notes |
| --- | --- | --- |
| `VectorOp` | `op.rs:15` | Flat enum of drawing operations. Serializable, no trait objects. |
| `VectorScene` | `scene.rs:17` | Width, height, `Vec<VectorOp>` paint list. |
| `IconDef` | `icons.rs:18` | Named icon: `ops`, `width`, `height`, helpers `recolor`, `as_op`, `as_op_alpha`. |
| `IconCategory` | `icon_set.rs:33` | Semantic category derived from app titles via `from_app_title`. |
| `AnimClock` | `anim.rs:13` | Frame counter + monotonic seconds, oscillators, easings. |
| `BackgroundLayer`, `BackgroundScene` | `background.rs` | Data-driven decorative layers (grids, spheres, radar, waves, shaders). |

## VectorOp reference

The op set is intentionally narrow — there are no general Beziers; curves are
approximated with arcs and polygons.

- Rectangles: `FillRect`, `StrokeRect`, `FillRoundedRect`, `StrokeRoundedRect`.
- Polygons: `FillPolygon`, `StrokePolygon` (vector of integer points; convex
  fills assumed).
- Circles: `FillCircle`, `StrokeCircle`.
- Arcs: `FillArc`, `StrokeArc` (`cx`, `cy`, `radius`, `start_angle`,
  `end_angle` in radians, clockwise from 3 o'clock).
- Lines: `Line`, `DashedLine`.
- Triangles: `FillTriangle`.
- Gradients: `RectGradient`, `PolygonGradient`.
- Text: `Text { string, x, y, font_size, color }`.
- `Group { ops, translate: (i32, i32), opacity: u8 }` — hierarchical
  container for nested transform and alpha.

Coordinates are integer pixels. Angles are radians, clockwise from the
positive X axis. Alpha is 0–255.

`VectorOp::scale(factor: f32)` (`op.rs:215`) multiplies every coordinate and
stroke width by `factor` so an icon designed at 22×22 can render at any
display size without re-authoring.

## Scene composition

`VectorScene` keeps a flat ops list in paint order (back to front). To compose
two scenes use `VectorScene::embed(x, y, other)` — it wraps `other.ops` in a
translated `Group` and appends them. There is no retained scene graph; every
frame rebuilds whatever ops the caller wants to draw.

## Built-in icon catalog

All in `crates/oasis-vector/src/icons.rs` and `icon_set.rs`.

### Altimit dashboard set (22×22)

The original PSP-style dashboard icons. Current factories include:

- `icon_the_world` — outer + inner squares; rotates when active via
  `icon_the_world_animated(angle)`.
- `icon_mailer` — envelope with V-flap.
- `icon_news` — bold "N" polygon.
- `icon_accessory` — pen-nib pentagon, centre line, dot.
- `icon_audio` — play triangle outline; pulses with
  `icon_audio_animated(alpha)`.
- `icon_data` — memory card silhouette with contact lines and an LED dot
  that blinks via `icon_data_animated(led_visible)`.

### Semantic icon sets (24×24)

Three coordinated sets keyed by `IconCategory` (the live enum lists every
category; common ones include browser, files, audio, tv, radio, settings,
video, home, network, power, gallery, weather, terminal, and a generic
fallback):

- `outline_icon(category, color)` — 2 px stroke, transparent body.
- `solid_icon(category, color)` — filled, high-contrast.
- `pixel_icon(category, color)` — 32×32 with baked-in window border, title
  band, and body. Pair with `icon_container = "none"` in the skin so the
  chrome doesn't double up.

### Background and overlay helpers

`icons.rs:476–622` defines:

- `wireframe_sphere(radius, color)` and an `_animated(angle)` variant.
- `active_indicator(height, color)` — the thin sidebar bar.
- `glass_polygon(points, color, alpha)` — translucent shape.
- `grid_overlay(w, h, spacing, color)` — sparse grid.
- `radar_sweep(cx, cy, radius, sweep_angle, rotation, color)` — filled
  wedge.
- `eq_bar(x, y, width, height, color)` — audio visualiser bar.
- `altimit_sidebar(...)` — full sidebar layout: 6 icons + labels + LED.

## Animation model

`AnimClock` (`anim.rs:13`) is the single time source:

- `frame: u32` — wrapping frame counter, advanced by `tick_frame()`.
- `time_s: f32` — monotonic seconds, advanced by `tick_dt(ms)` for variable
  timestep or implicitly by `tick_frame` at 1/60 s per tick.

Oscillators (`anim.rs:42`):

- `sine(freq, phase) -> f32` — `sin(TAU * freq * time + phase)`, range -1..1.
- `sine_norm(freq, phase) -> f32` — same, mapped to 0..1.
- `sawtooth(period_s, phase) -> f32` — wrapping 0..1 ramp.

Easing curves (`anim.rs:67`):

- `entrance_scale(elapsed_ms, duration_ms)` — ease-out cubic, 0→1.
- `entrance_alpha(elapsed_ms, duration_ms)` — ease-out quadratic, 0–255.
- `entrance_slide_y(elapsed_ms, duration_ms, distance)` — ease-out cubic.

Per-frame helpers (`anim.rs:129`):

- `float_offset(frame, slot, amplitude, speed)` — sine bobbing,
  per-slot phase.
- `pulse_alpha(frame, speed, min_alpha)` — oscillating alpha.
- `blink_visible(frame, interval)` — 2/3 on, 1/3 off duty cycle.
- `rotate_point`, `rotate_rect` — rotation around a centre.

The model is **frame-driven** in the sense that animations are pure functions
of `(frame, params)` — there is no tweening engine and no event queue. Drop a
frame and animations don't desync because next frame's evaluation uses the
new counter value.

## Rendering: SdiVector trait integration

`render::render_scene(backend, &scene)` (`render.rs:17`) dispatches each op to
the backend trait it belongs to:

- `SdiCore` for `fill_rect`, `draw_text`.
- `SdiShapes` for `fill_circle`, `stroke_circle`, `draw_line`.
- `SdiVector` for `fill_polygon`, `stroke_polygon`.
- `SdiClipTransform` for `push_translate` / `pop_translate` (Group nesting).
- `SdiGradients` for the gradient ops.

Group opacity is multiplied into each child op's alpha by `apply_alpha`
(`render.rs:44`). Empty translates `(0, 0)` are skipped to avoid pushing a
no-op transform.

`render_scene_at(backend, scene, x, y, alpha)` is the common wrapper — it
positions the scene and applies a final alpha multiplier. Use it for
dashboard icons; use `render_scene` directly for full-viewport overlays.

## Where it's used

- `oasis-core/src/dashboard/vector_icons.rs` — `icon_for_app` picks the
  preset (altimit / outline / solid / pixel) based on theme config and
  invokes the right factory; `altimit_icon` layers per-icon animations.
- `oasis-core/src/vector_overlay.rs` — `render_vector_background` builds an
  `AnimClock`, calls `BackgroundScene::build_ops`, and renders the result
  full-frame.
- `oasis-skin/src/active_theme/derive.rs` — derives `BackgroundLayer` lists
  from TOML `[[background_layers]]` blocks during skin load.

## Adding a new icon

1. Open `crates/oasis-vector/src/icons.rs`. Add a factory returning
   `IconDef` with the right `width` / `height` and a `Vec<VectorOp>`.
2. Stick to the supported primitives. Use `FillPolygon` to approximate
   curves; pick a step count that looks acceptable at 1× scale (8–16 sides
   for a circle is usually fine since the renderer also scales).
3. Choose a grid: 22×22 for Altimit, 24×24 for outline / solid, 32×32 for
   pixel.
4. Add a unit test next to existing ones (around `icons.rs:690`) asserting
   name, dimensions, and op count.
5. Register in the dispatcher: `icon_for_app` for app-keyed icons,
   `outline_icon` / `solid_icon` / `pixel_icon` match arms for new
   `IconCategory` entries.
6. If you want it animated, add an `<icon>_animated(...)` variant that takes
   the relevant frame-driven parameter and uses helpers from `anim.rs`
   instead of constants.
