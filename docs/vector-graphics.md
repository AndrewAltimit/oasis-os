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
| `VectorOp` | `op.rs` | Flat enum of drawing operations. Serializable, no trait objects. |
| `VectorScene` | `scene.rs` | Width, height, `Vec<VectorOp>` paint list. |
| `IconDef` | `icons.rs` | Named icon: `ops`, `width`, `height`, helpers `recolor`, `as_op`. |
| `IconCategory` | `icon_set/mod.rs` | Semantic category derived from app titles via `from_app_title`. |
| `AnimClock` | `anim.rs` | Frame counter + monotonic seconds, oscillators, easings. |
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

`VectorOp::scale(factor: f32)` (`op.rs`) multiplies every coordinate,
size, radius, font size and rectangle stroke width by `factor` so an icon
designed at 22×22 can render at any display size without re-authoring.
Line, circle, arc and polygon stroke widths (and dash patterns) are left
as authored. Radii, rect stroke widths and font sizes never scale below 1.

Positions are snapped as **absolute** pixels: a point `p` (including its
enclosing groups' translation) lands on `round(p * factor)`, and a
rectangle keeps the snapped edges `round(x * factor)` and
`round((x + w) * factor)` instead of scaling its width separately. Parts
that touch in design space — the abutting rects of a pixel icon, a body
and its outline, ops in differently translated groups — therefore still
touch at any fractional scale; each scaled pixel shows the design pixel
its centre samples. Radii round; font sizes and rect stroke widths round
down.

## Scene composition

`VectorScene` keeps a flat ops list in paint order (back to front). To compose
two scenes use `VectorScene::embed(x, y, other)` — it wraps `other.ops` in a
translated `Group` and appends them. There is no retained scene graph; every
frame rebuilds whatever ops the caller wants to draw.

## Built-in icon catalog

All in `crates/oasis-vector/src/icons.rs` and `icon_set/` (`outline.rs`,
`solid.rs`, `pixel.rs`; shared helpers and `ops_bounds` in `mod.rs`).

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
video, home, network, power, gallery, weather, terminal, packages, monitor,
calculator, paint, text editor, games, and a generic fallback; every
built-in app maps to a non-generic category):

- `outline_icon(category, color)` — 2 px stroke, transparent body.
- `solid_icon(category, color)` — filled, high-contrast.
- `pixel_icon(category, color)` — 32×32 with baked-in window border, title
  band, and body. Pair with `icon_container = "none"` in the skin so the
  chrome doesn't double up.

### Status-bar glyphs

`status_glyphs.rs` holds the tiny status-bar indicators, recoloured with
the skin's `bar.battery_color`:

- `battery(level, charging, color)` — outline + nub with `level` of
  `BATTERY_LEVELS` (5) cells; charging dims the cells under a bolt.
  `battery_level(percent)` maps 0-100% to a level.
- `ac_plug(color)` — two-prong plug (wall power, no battery).
- `wifi(connected, color)` — two arcs over a dot, dimmed when
  disconnected.

`oasis-core`'s `StatusBar` reserves invisible "glyph slot" SDI objects for
them (placement + state key, so dirty tracking stays exact) and the shell
paints them after the SDI pass with `statusbar::render_status_glyphs`.
`StatusBar::indicator_style = IndicatorStyle::Text` restores the legacy
`AC` / `75% [|||| ]` text.

### Background and overlay helpers

`icons.rs` defines `grid_overlay(w, h, spacing, color)` — the sparse
Altimit grid (same lines as the `grid` background layer). The animated
background elements (wireframe sphere, radar sweep, rings, EQ bars,
waves, ...) live in the crate-private `backgrounds.rs` and are driven by
`BackgroundScene`.

## Animation model

`AnimClock` (`anim.rs`) is the single time source:

- `frame()` — wrapping frame counter, advanced by `tick_frame()`.
- `time_s()` — monotonic seconds, advanced by `tick_dt(ms)` for variable
  timestep or implicitly by `tick_frame` at 1/60 s per tick.
- `AnimClock::from_frame(n)` — a clock at frame `n` of a fixed 60 fps
  timeline (what the shell's vector overlay builds every frame).

Time is accumulated in `f64` internally: an `f32` accumulator stops
resolving the 1/60 s step after a few days of uptime (at 2^18 s its spacing
is 1/32 s, so animations ran at double speed). The oscillators and `angle`
reduce their phase in `f64` before narrowing.

Oscillators (`anim.rs`):

- `angle(rad_per_s, phase) -> f32` — constant-speed rotation angle, reduced
  modulo a full turn (used by rotating background layers).
- `sine(freq, phase) -> f32` — `sin(TAU * freq * time + phase)`, range -1..1.
- `sine_norm(freq, phase) -> f32` — same, mapped to 0..1.
- `sawtooth(period_s, phase) -> f32` — wrapping 0..1 ramp (negative
  phases wrap forward).

Easing curves (`anim.rs`):

- `entrance_scale(elapsed_ms, duration_ms)` — ease-out cubic, 0→1
  (`oasis_types::easing::ease_out_cubic`).
- `entrance_alpha(elapsed_ms, duration_ms)` — ease-out quadratic, 0–255.
- `entrance_slide_y(elapsed_ms, duration_ms, distance)` — ease-out cubic.

Per-frame helpers (`anim.rs`):

- `float_offset(frame, slot, amplitude, speed)` — sine bobbing,
  per-slot phase.
- `pulse_alpha(frame, speed, min_alpha)` — oscillating alpha.
- `blink_visible(frame, interval)` — 2/3 on, 1/3 off duty cycle
  (`interval` 0 or 1 = always visible; at least one frame on otherwise).
- `rotate_point`, `rotate_rect` — rotation around a centre (results are
  rounded, so a half turn of `(10, 0)` lands on `(-10, 0)`).

The model is **frame-driven** in the sense that animations are pure functions
of `(frame, params)` — there is no tweening engine and no event queue. Drop a
frame and animations don't desync because next frame's evaluation uses the
new counter value.

## Rendering: SdiVector trait integration

`render::render_scene(backend, &scene)` (`render.rs`) dispatches each op to
the backend trait it belongs to:

- `SdiCore` for `fill_rect`, `draw_text`.
- `SdiShapes` for `fill_circle`, `stroke_circle`, `draw_line`.
- `SdiVector` for `fill_polygon`, `stroke_polygon`.
- `SdiClipTransform` for `push_translate` / `pop_translate` (Group nesting).
- `SdiGradients` for the gradient ops.

Group opacity is multiplied into each child op's alpha by `apply_alpha`
(`render.rs`), including every stop of `RectGradient` / `PolygonGradient`. Empty translates `(0, 0)` are skipped to avoid pushing a
no-op transform.

`render_scene_at(backend, scene, x, y, alpha)` is the common wrapper — it
positions the scene and applies a final alpha multiplier. Use it for
dashboard icons; use `render_scene` directly for full-viewport overlays.

## Where it's used

- `oasis-core/src/dashboard/vector_icons.rs` — `icon_for_app` picks the
  preset (altimit / outline / solid / pixel) based on theme config and
  invokes the right factory; `altimit_icon` maps apps by `IconCategory`
  (browser → the world, audio → audio, files → data, text editor →
  accessory, other categories → their `solid` glyph, uncategorised apps →
  the legacy by-position cycle) and layers per-icon animations.
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
4. Add a unit test next to existing ones (in the `tests` module of `icons.rs`) asserting
   name, dimensions, and op count. `icon_set::ops_bounds` gives an op
   list's conservative pixel bounds; the icon-set tests assert every glyph
   stays inside its design box.
5. Register in the dispatcher: `icon_for_app` for app-keyed icons,
   `outline_icon` / `solid_icon` / `pixel_icon` match arms for new
   `IconCategory` entries.
6. If you want it animated, add an `<icon>_animated(...)` variant that takes
   the relevant frame-driven parameter and uses helpers from `anim.rs`
   instead of constants.
