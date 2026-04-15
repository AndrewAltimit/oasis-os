# Browser Engine Backlog

Gap analysis and follow-up work for `oasis-browser`, organized by epic.
Each epic is scoped as a standalone PR or short series. Items marked with
effort estimates assume one focused engineer-week.

This document is the live roadmap for closing the gap between our
from-scratch engine and a launch-ready embedded browser. Items are
grouped by area; ranking guidance is at the bottom.

Last updated: 2026-04-15 (WHATWG HTML conformance epic fully shipped on
`feat/browser-whatwg-epic-completion` — full adoption agency
algorithm, simplified foreign content (SVG / MathML) with the
canonical breakout list, template form-scope isolation, and a
vendored html5lib-tests-format harness seeded with 9 fixtures).

---

## ✅ Done: PSP JavaScript integration (QuickJS-NG + DOM)

Completed on `feat/psp-quickjs` (2026-04-14). All PSP targets now share
the same `rquickjs-engine` backend as desktop/WASM/UE5 — the earlier
`boa_engine` PSP backend has been removed entirely. Scripts tags,
`document.getElementById`, `textContent`, event dispatch, `fetch`,
`localStorage`, and the rest of the DOM surface run against a real DOM
on real PSP hardware (verified end-to-end with an HTTP-served test
page mutating three DOM nodes).

**Key decisions and fixes, for the historical record:**

- **Switched PSP from `boa_engine` back to QuickJS-NG via `rquickjs`.**
  The original "pure-Rust JS on PSP" plan used `boa` to avoid a MIPS
  C toolchain. Once pspdev was installed (`/opt/pspdev`), wiring the
  `cc` crate at `psp-gcc` via `CC_mipsel_sony_psp_std` was
  straightforward, and QuickJS is ~10× faster than boa on Allegrex.
- **Linker routed through `psp-ld` instead of `rust-lld`.** GCC 15 /
  binutils 2.43 emit `.symtab` headers with a `sh_info` layout that
  `rust-lld` strictly rejects (`invalid binding: 0`). Switched the
  Rust target's linker to `/opt/pspdev/bin/psp-ld` via
  `linker-flavor=gnu` under `-Z unstable-options`, re-playing the
  rust-psp target-spec pre-link args (`--emit-relocs`, `--nmagic`,
  inline PRX link script) under the `gnu` flavor. Added a tiny
  supplementary link script (`tools/psp-linkscript.ld`) to synthesise
  `_gp` at ALIGN(16)+0x7ff0; the rust-psp target script omits `_gp`
  because rust-lld computes it internally.
- **Plan B shims instead of pspdev's newlib.** To stay dependency-free
  we don't link `libc.a` / `libm.a` / `libcglue.a` — those archives are
  compiled as eabi32 / msingle-float / abicalls and can't be merged
  with Rust's o32 / mdouble-float / non-abicalls code. Instead
  `src/quickjs_shim.rs` provides the ~40 libc/libm symbols QuickJS
  references (math via the `libm` crate, string/memory routines,
  `calloc`/`realloc` forwarding to libpsp's malloc/free with header
  decoding, non-variadic stdio stubs, a 256-byte `_impure_ptr` static,
  RTC-backed `clock_gettime` family).
- **FPU ABI: C must be `-msingle-float`, not `-mdouble-float`.** The
  real-hardware blocker that ate five days of debugging. PSP Allegrex
  has a single-precision FPU only; rust-psp's target spec declares
  `"features": "+single-float"`. We were building QuickJS C with
  `-mdouble-float`, which PPSSPP silently fixed up but which crashed
  on real hardware inside `JS_Eval`'s dtoa path on the first double-
  precision helper call. Fixed by flipping `CFLAGS_mipsel_sony_psp_std`
  to `-msingle-float`. All six JSTEST probes pass post-fix.
- **Lazy init.** `BrowserState::widget` in
  `crates/oasis-backend-psp/src/app_states.rs` stores
  `Option<BrowserWidget>`; the JS engine is only constructed when the
  user actually launches the browser app. Boot-time cost is zero.
- **Binary impact.** EBOOT grew from 6.26 MB (no `javascript` feature)
  → 6.79 MB (with DOM bindings). Runtime heap drops from 8.76 MB →
  8.44 MB free after browser open. Both fit comfortably in the PSP's
  24 MB user partition.

**Expected perf ceiling.** QuickJS on Allegrex is still two orders of
magnitude slower than V8 on desktop — inert pages with small
bootstrap scripts will work fine; React SPAs will crawl. This is the
same honest framing as before, just with QuickJS's real numbers
instead of boa's.

**Follow-up tickets** (filed separately in the launch-polish section
below):

- Space-collapsing bug: JS-mutated `textContent` containing ASCII
  spaces renders without visible spaces on PSP (`"hello from QuickJS"`
  renders as `"hellofromQuickJS"`). Likely a PSP bitmap font / text
  measurement quirk in the browser paint path, not a JS issue — the
  same mutation renders correctly on desktop. Scope: investigate
  whether `glyph_advance(' ')` returns 0 in the PSP font table, or
  whether the text layout step collapses whitespace after
  JS-triggered re-layout.
- `js_dom.rs` bootstrap bloat: two large JS string constants
  (`JS_DOM_BOOTSTRAP` ~530 lines, `JS_CANVAS_BOOTSTRAP` ~144 lines)
  eagerly get compiled into `.rodata` even on PSP where canvas is
  unused. Consider feature-gating the canvas bootstrap behind a
  separate flag to trim ~20 KB on PSP.

---

## ✅ Done: 3D transforms

**Effort:** 1–2 weeks. Standalone from compositor, similar
cross-cutting nature. **Complete** — scaffolding shipped on
`feat/browser-3d-transforms`, three follow-ups shipped on
`feat/browser-3d-transforms-followups`.

- ~~Add `AffineTransform3D` alongside the existing `AffineTransform2D`.~~
  Shipped on `feat/browser-3d-transforms` as `Matrix3d` (4×4
  column-major matrix in `crates/oasis-browser/src/transform.rs`).
  Provides `identity`, `translate`, `scale`, `rotate_x/y/z`,
  `rotate_axis`, `perspective`, `multiply`, `apply_homogeneous`,
  `apply_point_3d`, `from_2d_affine`, `from_css_transforms_3d`, and
  `flatten_to_affine`. The existing
  `AffineTransform2D::from_css_transforms` is now a thin wrapper:
  `Matrix3d::from_css_transforms_3d(...).flatten_to_affine()`.
- ~~`translate3d` / `rotate3d` / `rotateX` / `rotateY` / `rotateZ` /
  `scale3d` — parsed today but flattened to 2D affine.~~ — parsed on
  `feat/browser-3d-transforms`. `TransformFunction` gained
  `Translate3d`, `TranslateZ`, `Scale3d`, `ScaleZ`, `RotateX`,
  `RotateY`, `RotateZ`, `Rotate3d`, `Matrix3d`, and `Perspective`
  variants. The CSS parser handles `translate3d()`, `translateZ()`,
  `scale3d()`, `scaleZ()`, `rotateX/Y/Z()`, `rotate3d(x,y,z,deg)`,
  `matrix3d(...16 values)`, and the `perspective(d)` function.
  Evaluation runs through the new `Matrix3d` 4×4 pipeline, then
  flattens orthographically — `rotateX(60deg)` becomes a vertical
  squash by `cos(60°)`, `rotateY(60deg)` a horizontal squash, etc.
  This is visually correct under orthographic projection but loses
  the perspective trapezoid (see follow-ups below).
- ~~`backface-visibility: hidden` — parsed; needs paint-time normal
  check to cull back-facing quads.~~ — shipped on
  `feat/browser-3d-transforms`. `paint_box` builds a `Matrix3d` from
  the element's transforms when `backface_visibility == Hidden`,
  computes the surface-normal Z of the
  `(0,0,0)→(w,0,0)→(0,h,0)` triangle, and skips painting the entire
  subtree when it's negative. Front-face culling test cases for
  `rotateY(60deg)` (front) vs. `rotateY(120deg)` (back) live in
  `transform.rs`.

- ~~**Perspective projection.**~~ — shipped on
  `feat/browser-3d-transforms`. `PaintContext` now tracks an
  inherited `PerspectiveContext` (distance + screen-space vanishing
  point) which is pushed when descending into children of an element
  with the `perspective` CSS property. When a descendant has 3D
  transforms under an active perspective context, paint takes a
  screen-space path: it builds the full 4×4
  `T(vp) * Persp(d) * T(-vp) * T(origin) * local * T(-origin)`
  matrix in screen coordinates, projects the box's three reference
  corners through it with per-vertex perspective divide, and derives
  a screen-space affine that maps the original screen rect to the
  projected screen quad via `Matrix3d::project_screen_rect_affine`.
  The 4th corner is the parallelogram completion `p1 + p2 - p0` —
  exact for orthographic rotations, an approximation for steep
  perspective angles. `perspective-origin` is now parsed into a
  structured `PerspectiveOrigin { x, y, x_pct, y_pct }` (replaces
  the opaque `String` storage) and resolved against the
  perspective-establishing element's content box.
- ~~**`transform-style: preserve-3d`.**~~ — shipped on
  `feat/browser-3d-transforms`. `PaintContext` carries an ambient
  `preserved_3d: Option<Matrix3d>` that propagates a parent's full
  screen-space matrix to descendants when the parent has
  `transform-style: preserve-3d`. Children compose their own local
  3D matrix into the preserved ambient matrix instead of flattening
  at the parent boundary, so a `translateZ(50px)` child under a
  `rotateY(30deg)` preserve-3d ancestor with a `perspective: 800px`
  great-grandparent now actually moves toward/away from the viewer
  and gets perspective-divided correctly. The default `flat`
  flushes `preserved_3d` to `None` for descendants, matching the
  spec semantics ("the element renders as a flattened 2D image
  inside its 3D parent"). Z-sorting of children inside a preserved
  subtree is still a follow-up — paint order remains DOM order.
- ~~**`transform-origin: Z`.**~~ — shipped on
  `feat/browser-3d-transforms`. `TransformOrigin` gained a `z`
  field, the parser accepts the three-token form
  (e.g. `transform-origin: 25% 75% 40px`), and
  `Matrix3d::from_css_transforms_3d` plumbs the Z component through
  the pre/post translate pair so rotations pivot around an
  arbitrary 3D point.
- ~~**Reviewer-flagged: `transform-origin` resolution.**~~ — the
  earlier scaffolding hardcoded the box centre in
  `compute_transform_matrix` and the backface-visibility check.
  Now both consult `style.transform_origin` via a shared
  `resolve_transform_origin` helper that handles the `x_pct`/`y_pct`
  percentage forms.

**Follow-ups shipped on `feat/browser-3d-transforms-followups`:**

- ~~**Z-sorting inside preserve-3d subtrees.**~~ — shipped.
  `paint_box` now detects when the element is a `transform-style:
  preserve-3d` container whose screen-space matrix was computed
  (i.e. it went through the 3D screen path) and, for that child
  walk, flattens the CSS 2.1 stacking-context tiers into a single
  `normal_children` list. The children are then sorted
  back-to-front by the projected Z of their layout-center point,
  computed via the new `preserve3d_child_z` helper that composes
  `parent_screen_matrix * child_local_matrix` and reads `z/w`
  after the perspective divide. Siblings with negative `z-index`
  still carry their usual semantics outside preserve-3d;
  explicit z-index opt-outs inside preserve-3d are considered a
  follow-up (the spec itself is fuzzy on the interaction).
- ~~**Trapezoidal background for steep perspective.**~~ — shipped.
  `PaintContext` now tracks an inherited
  `ambient_screen_matrix: Option<Matrix3d>` — the nearest
  3D-transformed ancestor's full screen-space 4×4 matrix.
  `paint_background` checks that matrix first: when present (and
  the element has `border-radius: 0`) it projects all four
  padding-box corners individually via `apply_point_3d`, producing
  a true trapezoid under steep rotations like
  `rotateY(75deg) perspective(200px)`. The 3-corner-fit affine in
  `project_screen_rect_affine` is still used for the element's own
  descendant matrix composition (which is an affine operation by
  definition); only background painting now bypasses it.
- ~~**Existing 2D transform double-translation bug.**~~ — fixed.
  The flat orthographic path now builds its 2D affine in screen
  space (pivot at `(sx + ox_local, sy + oy_local)`) so
  `ctx.transform` composed with that matrix rotates each box
  around its correct screen pivot. Correspondingly, for
  non-translation-only matrices we no longer add
  `child_matrix.e/.f` to `tx_offset_x/y` — the screen-space
  matrix already carries all the translation, and the old addition
  was injecting the rotation-pivot compensation as a stray offset
  on children. The translate-only fast path is unchanged
  (`is_translation_only` matrices still flow through the offset
  shift so plain `translate(...)` doesn't have to round-trip
  through `fill_polygon`).

Regression tests for all three are in `paint::tests` under
`crates/oasis-browser/src/paint/mod.rs`:
`rotate_around_box_center_produces_symmetric_quad`,
`rotated_parent_does_not_shift_child_offset`,
`preserve_3d_children_z_sorted_back_to_front`, and
`steep_perspective_produces_trapezoidal_quad`.

**Backend impact:** desktop and WASM rasterize the flattened 2D
affine today via the existing `fill_polygon` path. UE5 and PSP also
inherit the flatten transparently. The screen-space projection is
backend-agnostic — it produces standard affines that any backend
can consume. Trapezoidal painting (the follow-up above) would need
a non-affine quad primitive on backends that don't already have one;
PSP GU has `sceGumPerspective` for true perspective rendering.

---

## Epic: Real-world compatibility measurement

**Effort:** ongoing grind. **Highest-leverage item on the list.**

This PR (`feat/browser-improvements`) added 4 fixtures in
`tests/fixtures/`. That's the floor, not the target.

**Corpus expansion (20–50 fixtures):**

- Wikipedia article (real HTML pulled from a stable revision, not
  synthetic).
- Hacker News front page.
- GitHub README rendered output.
- A docs site (Rust `std` docs, MDN reference page).
- A forum (phpBB or Discourse snapshot).
- A news masthead (NYT-style multi-column grid).
- A commerce product page.
- A blog platform post (Medium, Substack).

Strip each to a reasonable size and check in under `tests/fixtures/`.

**Visual regression harness.** Single highest-leverage item:

- Render each corpus fixture to PNG via the SDL backend.
- Check golden PNGs into the repo (one per fixture).
- CI gate on pixel delta > threshold.
- Add as a new CI step after the existing `screenshot regression` job.
- This catches ~90% of paint regressions automatically.

**Layout performance budgets:**

- "Wikipedia frontpage lays out in <500ms on desktop, <2s on PSP."
- Wire into the existing `cargo bench` infrastructure under
  `benches/layout_engine.rs`.
- CI gate on regression > 20%.

**Triage tooling (not in CI):**

- Crawler script: point at a curated list of top-500 sites, record which
  ones panic/error/hang, bucketed by failure mode. Local tool for
  triage, not CI.

---

## ✅ Done: WHATWG HTML conformance

**Effort:** ~1 week (actual). Shipped on
`feat/browser-whatwg-epic-completion` — the remaining items from the
original epic all landed in one pass.

- ~~**Integrate `html5lib-tests`** (~20k standard tests from the WHATWG
  working group).~~ — shipped on `feat/browser-whatwg-epic-completion`
  as a **vendored-subset** harness. `crates/oasis-browser/tests/html5lib_tree_construction.rs`
  parses the upstream `.dat` format (tree-construction dialect: `#data`
  / `#errors` / `#new-errors` / `#document` / `#document-fragment` /
  `#script-on|off`) and diffs our tree builder output against the
  pipe-indented expected dump. Fixtures live under
  `tests/fixtures/html5lib/tree_construction_basic.dat` (9 cases
  covering the features this epic touched: basic tree shape, the
  `<p>a<b>b<i>c</b>d</i>e</p>` spec example for adoption agency,
  `<b><div>...</div></b>` furthest-block handling, `<svg>` / `<math>`
  subtrees, `<template>` hoisting to `<head>`, list + table implicit
  structure). We pull in a curated subset rather than the full ~20k
  upstream repo because (a) many upstream tests exercise features we
  deliberately don't implement — full namespaced SVG with camelCase
  fixup, MathML integration points, the adversarial 8+-iteration
  adoption agency cases, plaintext mode, etc. — and (b) we don't want
  to make CI depend on an external download. The harness is
  extensible: drop more `.dat` files into the fixtures directory and
  list them in `FIXTURE_FILES`.
- ~~Foster parenting is subtly wrong — inserts at the wrong position.~~
  (Already fixed earlier on `feat/browser-whatwg-conformance`.)
- ~~Adoption agency algorithm is simplified.~~ — **full WHATWG
  §13.2.6.4.7 algorithm shipped** on
  `feat/browser-whatwg-epic-completion`. `close_formatting_element`
  now runs the outer 8-iteration / inner 64-iteration rebuild loop,
  computes the "furthest block" via a new `TagName::is_special()`
  helper, and reparents children via a clone-and-insert pass on each
  iteration. The common adversarial cases all work:
  `<p>a<b>b<i>c</b>d</i>e</p>` produces `<p>a<b>b<i>c</i></b><i>d</i>e</p>`
  (with text hoisted correctly), `<b><div>x</div></b>` keeps
  `<b><div>x</div></b>` as-is (the `</div>` closes cleanly before the
  `</b>` reaches adoption agency), and `<b><p>...</b></p>` runs the
  full move-furthest-block-to-common-ancestor path. A new
  `Document::detach_node` helper (unlinks without freeing, unlike
  `remove_child`) supports the reparenting.
- ~~No `<template>` element / DocumentFragment support.~~ — earlier
  minimal support is now **upgraded with form-scope isolation**. A
  `template_form_stack: Vec<Option<NodeId>>` on `TreeBuilder` saves the
  enclosing `form_element` pointer on `<template>` open and restores
  it on close, so a `<form>` inside a `<template>` inside an outer
  `<form>` actually parses instead of being silently dropped by our
  nested-form guard. The InHead fallback was also corrected to check
  the *entire* open elements stack for a `<template>` ancestor (not
  just the current node), which unblocks parsing `<template><p>x</p></template>`
  where the `</p>` arrives while we're still in InHead. BeforeHead
  also now recognises `<template>` as a head-content tag that triggers
  an implicit `<head>` + InHead switch rather than falling through to
  an implicit `<body>`. Real DocumentFragment isolation for other
  scope types (table, select) is still a follow-up.
- ~~No SVG/MathML foreign content handling.~~ — shipped on
  `feat/browser-whatwg-epic-completion` as a simplified subset of
  WHATWG §13.2.6.5. `TreeBuilder::foreign_depth` counts open
  foreign-content elements; while > 0, tokens are dispatched through
  `handle_foreign_content` instead of the HTML insertion modes.
  Generic start tags become literal elements (no `close_p_if_in_scope`,
  no `reconstruct_formatting`, no void-element fixup), `self_closing`
  is honored so `<circle />` works, and end tags pop to the matching
  element without adoption agency. HTML **breakout** is implemented
  against the canonical spec list (`b`, `big`, `blockquote`, `body`,
  `br`, `center`, `code`, `dd`, `div`, `dl`, `dt`, `em`, `embed`,
  `h1`…`h6`, `head`, `hr`, `i`, `img`, `li`, `listing`, `menu`,
  `meta`, `nobr`, `ol`, `p`, `pre`, `ruby`, `s`, `small`, `span`,
  `strong`, `strike`, `sub`, `sup`, `table`, `tt`, `u`, `ul`, `var`)
  — seeing one of these tags inside `<svg>`/`<math>` pops the
  foreign subtree off the open stack and reprocesses the token via
  the HTML path. Tag names are stored lowercased (the tokenizer
  already lowercases and we don't track namespaces), so SVG
  camelCase identifiers like `<foreignObject>` / `<textPath>` don't
  round-trip — this is an intentional simplification.
- ~~No parser error reporting — we silently drop malformed input.~~
  (Already fixed earlier on `feat/browser-whatwg-conformance`.)
- Full frameset support remains a **deliberate non-goal**.

---

## Epic: Missing CSS features (the long tail)

**Effort:** varies per item. We currently implement ~120 properties;
Blink/WebKit ship ~600. Most of the gap is niche. These are the ones
that cause real breakage on modern sites:

**High-impact, should prioritize:**

- ~~`:has()` selector~~ — shipped on `feat/browser-has-selector`. Parses
  relative-selector lists (`> child`, `+ sib`, `~ sib`, descendant),
  matches candidates against each relative selector, specificity takes
  the max of the inner selectors. Ancestor-walking combinators inside
  the inner selector are scope-bounded to the subject's subtree, so
  `article:has(.a .b)` can't match via an `.a` that lives above the
  article.
- ~~`@container` queries~~ — shipped on `feat/browser-container-queries`.
  Parses `@container [name?] (min-width|max-width|width|min-height|
  max-height|height: Npx)` (plus `inline-size` / `block-size` aliases),
  joined with `and`. Conditions are stored on each contained `Rule` and
  evaluated at cascade time against the nearest matching container
  ancestor. `container-type` (`normal` / `inline-size` / `size`),
  `container-name`, and the `container: <name> [/ <type>]` shorthand are
  all parsed into `ComputedStyle`. The pipeline does a second
  cascade+layout pass after the first layout when any rule is
  container-gated, populating a `ContainerLookup` snapshot of every
  query container's content-box size; pages without `@container` skip
  the work entirely. Limitations: nested `@container` rules use
  innermost-wins instead of AND-combining; style queries
  (`@container style(...)`) are parsed but always evaluate false; we
  don't currently iterate the relayout to a fixpoint, so a single pass
  catches the common "container resizes its descendants based on the
  first laid-out width" case but not pathological circular dependencies.
- ~~`@layer`~~ — shipped on `feat/browser-has-selector`. Supports
  statement form (`@layer a, b, c;`), named block form
  (`@layer a { ... }`), and anonymous block form (`@layer { ... }`).
  Cascade sort factors layer order between origin and specificity;
  `!important` reverses layer priority per spec. Known limitation:
  layer names are sheet-local (not merged across multiple stylesheets)
  — cross-stylesheet ordering still falls through to source order.
- ~~CSS nesting (`& .foo { }`)~~ — shipped on `feat/css-nesting` (parse-time
  desugaring: Cartesian-expands parent × child selector lists, substitutes
  `&` inline, supports nested `@media`, no compositor/matcher changes).
- ~~`color-mix()`, `oklch()`, `color()`, `light-dark()` functions~~ —
  shipped on `feat/browser-has-selector`. `hsl/hsla`, `oklch/oklab`,
  `color(srgb | srgb-linear | display-p3 …)`, `color-mix(in srgb, …)`,
  and `light-dark()` all parse to our existing sRGB `CssColor`.
  `color-mix` interpolates in linear sRGB (not in the requested color
  space for non-`srgb` arguments yet). `light-dark()` always returns
  the light-mode argument since we don't track a color-scheme context
  at parse time.
- ~~Logical properties~~ — shipped on `feat/browser-has-selector`.
  Parse-time rewrite of `margin-inline-*`, `padding-block-*`,
  `inset-inline-*`, `border-inline/block-*-{width,color,style}`, and
  `inline-size`/`block-size` (plus min/max variants) to their LTR
  physical equivalents. `margin-inline` / `padding-block` /
  `inset-inline` / `inset-block` shorthands expand with the usual
  one-value / two-value forms. RTL is still not supported anywhere in
  the engine so the rewrite is always LTR.
- ~~`text-wrap: balance` / `pretty`~~ — parsed and stored on
  `ComputedStyle` on `feat/browser-has-selector`. `wrap` / `nowrap`
  behave correctly; `balance` / `pretty` / `stable` fall through to
  `wrap` because the layout-side balancing algorithm is a follow-up.
- ~~`:is()` / `:where()` — check if already supported; audit.~~
  **Already done.** Parsed in
  `crates/oasis-browser/src/css/parser/selectors.rs:166-168` and
  matched in `crates/oasis-browser/src/css/cascade/matching.rs` via
  the `Is` / `Where` arms of `matches_simple`.
- ~~`aspect-ratio` — audit, may already be supported.~~
  Parsed and stored before; **now wired into block layout on
  `feat/browser-has-selector`**: non-replaced block elements with
  `height: auto` derive their content height from the resolved
  content width and the ratio. Explicit height always wins; when
  `width` is also `auto` we let children drive the height as before.
  Replaced-element aspect-ratio sizing (img, video) is still a
  follow-up.

**Medium-impact:**

- ~~`@property` — typed custom property registration.~~ — shipped on
  `feat/browser-container-queries`. Parses `@property --name { syntax;
  inherits; initial-value }` into `Stylesheet.properties`. Cascade
  seeds the `initial-value` into each element's custom-properties map
  before pass 1 (so `var(--name)` resolves even when no rule sets it),
  and respects `inherits: false` by stripping the property after the
  inherit-from-parent step. `syntax` is parsed but not validated.
- ~~`field-sizing: content`.~~ — shipped on
  `feat/browser-container-queries`. New `FieldSizing` enum on
  `ComputedStyle`. The inline layout pass measures the input's actual
  `value` (or `placeholder`) width and uses that instead of the
  `size`-attribute × char-width product. `<textarea>` content-sizing
  walks lines for height too.
- ~~`@scope` — shipping in Chrome.~~ — shipped on
  `feat/browser-container-queries`. Parses `@scope (root) [to (limit)]?
  { ... }` and tags inner rules with a `ScopeCondition`. Cascade
  filters scope-gated rules by walking the DOM up from each candidate
  element: a limit ancestor fails the rule fast; the first matching
  root ancestor passes it. A bare `@scope { ... }` (no root) applies
  anywhere not under a limit boundary. Selectors in the scope clause
  are re-parsed via `parse_selector_string` per element check (cheap
  for the typical case of one or two scopes per page; we can cache
  later if real corpora hit it hard).
- ~~`counter-style` — rarely breaks rendering but worth parsing.~~ —
  shipped on `feat/browser-container-queries` as parse-only. Parses
  `@counter-style name { system; symbols; additive-symbols; range;
  prefix; suffix; pad; negative; fallback; speak-as }` into
  `Stylesheet.counter_styles` so authors can ship the descriptor
  block without warnings. List-item rendering still uses the built-in
  styles only — wiring custom counter styles into `<ol>` markers is a
  follow-up.

**Low-impact (skip until someone complains):**

- View Transitions API (`view-transition-*`).
- Anchor Positioning (CSS Anchor Positioning Module Level 1).
- Subgrid.
- `scroll-timeline`, `animation-timeline`.

**Already parsed but not painted — audit needed:**

- ~~`accent-color`, `caret-color`.~~ — both wired up on
  `feat/browser-container-queries`. `accent-color` tints the checked
  background of `<input type="checkbox">` and the dot of
  `<input type="radio">` (Blink-style); the checkmark flips to white
  when the box is filled with the accent for contrast. `caret-color`
  draws a 1-pixel caret in focused `<input>` and `<textarea>` form
  controls (record path), positioned after the value text on text
  inputs and after the last visible line on textareas, with fallback
  to `style.color`. PaintViewport now carries `focused_node` so the
  recorder can tell which input has focus.
- ~~`will-change`.~~ — broadened on `feat/browser-container-queries`.
  The boolean is now `will_change_promotes_layer` and accepts any of
  `transform`, `opacity`, `filter`, `scroll-position`, or `contents`,
  including `Multiple` value lists like `will-change: top, transform`.
  The compositor already promotes any element with the flag to its
  own stacking context AND a real compositing layer (see
  `creates_compositing_layer` / `creates_stacking_context` in
  `paint/mod.rs`), so the layer-creation half of the backlog item
  is now end-to-end.

---

## Launch-polish items

These don't show up as CSS properties but bite users first. Not a
single epic — file as individual issues.

- **Font rendering quality across skins** — kerning, hinting, subpixel
  positioning. Especially on PSP where we have system TrueType fonts
  via `psp::font`.
- **PSP space-collapsing in JS-mutated text nodes.** When JavaScript
  sets `textContent` to a string containing ASCII spaces (e.g.
  `"hello from QuickJS"`), the PSP browser renders it without any
  visible spaces (`"hellofromQuickJS"`). The same mutation renders
  correctly on desktop. Not a JS bug — likely either `glyph_advance`
  returns 0 for U+0020 in one of the PSP bitmap font tables
  (`oasis-backend-psp/src/font.rs`), or the text layout step collapses
  whitespace after JS-triggered re-layout. Reproduce: `browse
  http://<pc>/test.html` where test.html contains an inline script
  that assigns a space-containing string to an element's
  `textContent`. Screencap will show the space-free rendering.
- **Image decoding error recovery** — corrupt JPEG/PNG currently
  crashes the decode path. Should degrade to a placeholder.
- **Network error UX** — timeout, DNS fail, TLS error should produce a
  useful error page, not a blank screen.
- **HTTP/2 support** — we only speak HTTP/1.1. Many modern CDNs require
  h2. Blocks access to some sites entirely.
- **`@font-face` / web fonts** — completely missing. Fallback to system
  fonts works but looks wrong on branded pages.
- **Accessibility** — ARIA roles are parsed but not exposed to anything.
  Low priority for launch but should at least have a plan.

---

## Ranking by ROI for "launch in 1–2 months"

If the constraint is a short runway to public launch, the priority
order is:

1. **Visual regression harness** (biggest leverage per hour of work,
   smallest risk). Catches regressions automatically forever.
2. ~~**`html5lib-tests` integration**~~ — shipped as a vendored-subset
   harness on `feat/browser-whatwg-epic-completion` (see the WHATWG
   HTML conformance epic above).
3. ~~**CSS long-tail subset: `:has()` + `@layer` + `@container` +
   CSS nesting**~~ — all shipped. `:has()` and `@layer` on
   `feat/browser-has-selector`; CSS nesting on `feat/css-nesting`;
   `@container` on `feat/browser-container-queries`.
4. **Compositor overhaul** (high effort but unlocks mix-blend-mode,
   backdrop-filter, mask, isolation, filter, will-change in one
   architectural change).
5. ~~**3D transforms**~~ — shipped (scaffolding on
   `feat/browser-3d-transforms`, follow-ups on
   `feat/browser-3d-transforms-followups`).
6. **Launch polish items** (parallel stream, file individually).

PSP JavaScript integration was on this list previously; it shipped
on `feat/psp-quickjs` (see the Done section at the top) and is no
longer blocking.

---

## Out of scope / non-goals

Document these explicitly so we stop relitigating them:

- **V8-level JS performance on PSP.** Not happening. We ship QuickJS-NG
  on PSP (see the Done epic at the top); it's two orders of magnitude
  slower than V8 and that's fine for our target use cases.
- **Service workers, WebRTC, Web Audio API, IndexedDB.** Too much
  surface area for an embedded engine. If something needs these, it's
  not our target use case.
- **Full HTML5 frameset support.** Deliberate non-goal — the web has
  moved on.
- **SVG animation (SMIL).** Parse basic SVG paths only; complex SVG
  rendering is out of scope.
- **CSS Houdini.** Too new, no ecosystem demand.
