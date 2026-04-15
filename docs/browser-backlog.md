# Browser Engine Backlog

Gap analysis and follow-up work for `oasis-browser`, organized by epic.
Each epic is scoped as a standalone PR or short series. Items marked with
effort estimates assume one focused engineer-week.

This document is the live roadmap for closing the gap between our
from-scratch engine and a launch-ready embedded browser. Items are
grouped by area; ranking guidance is at the bottom.

Last updated: 2026-04-15 (3D transforms scaffolding tracked in `feat/browser-3d-transforms`).

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

## Epic: 3D transforms

**Effort:** 1–2 weeks. Standalone from compositor, similar
cross-cutting nature.

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

**Follow-ups (not yet shipped):**

- **Perspective projection.** `perspective` (the container property)
  and `perspective(d)` (the transform function) both parse and feed
  into the 4×4 pipeline, but `flatten_to_affine` performs only an
  orthographic drop of the Z column/row. A true perspective-correct
  paint needs either (a) a non-affine quad path (project the four
  corners with the perspective divide and feed them to
  `fill_polygon`, with a matching path for text/borders), or (b) a
  GPU-side projection matrix on backends that have one.
  `perspective-origin` is still stored as an opaque string — needs
  the same structured pre-resolve treatment `transform-origin` got.
- **`transform-style: preserve-3d`.** Parsed and stored, still
  ignored. Needs to (1) skip the per-element flatten so descendants
  inherit the parent's 4×4, (2) re-sort children by transformed Z
  inside the preserved subtree, and (3) flatten only at the next
  `Flat` boundary.
- **`transform-origin: Z`.** The 2D origin is plumbed through
  `from_css_transforms_3d`, but Z is hard-coded to 0. Trivial extension
  once `transform-origin` parsing learns a third component.

**Backend impact:** desktop and WASM rasterize the flattened 2D
affine today via the existing `fill_polygon` path. UE5 and PSP also
inherit the flatten transparently. Once perspective is wired up, the
non-affine quad path becomes the cross-cutting backend question —
PSP GU has `sceGumPerspective`, but the backend trait would need a
new "submit a textured trapezoid with perspective-correct UVs"
primitive.

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

## Epic: WHATWG HTML conformance

**Effort:** ~1 week. Needs one external test-suite integration.

- **Integrate `html5lib-tests`** (~20k standard tests from the WHATWG
  working group). Add as `tests/html5lib.rs`, allowlist failures we
  can't fix, gate CI on no-regression.
- **Known gaps worth fixing:**
  - ~~Foster parenting is subtly wrong — inserts at the wrong position.~~
    Fixed on `feat/browser-whatwg-conformance`. `foster_parent` now
    uses the new `Document::insert_before` helper to place the new
    node immediately before the foster-parented `<table>` in the
    table's parent (per WHATWG §13.2.6.1). The InTable "anything else"
    branch also defers to InBody when the current open element is no
    longer in a table context, so a foster-parented `<div>` correctly
    receives its own children instead of foster-parenting them too.
  - Adoption agency algorithm is simplified. Handles common formatting
    cases; fails on the adversarial `<b><p></b></p>` reorderings from
    the WHATWG spec examples.
  - ~~No `<template>` element / DocumentFragment support.~~ — minimal
    support shipped on `feat/browser-whatwg-conformance`.
    `TagName::Template` is now a real variant; `<template>` parses as
    a regular element in both InHead and InBody modes; the InHead
    fallback dispatches via InBody when the current open element is a
    `Template` so children parse normally instead of implicitly
    closing `<head>`. The UA stylesheet already had
    `template { display: none }`, so contents are inert at paint.
    DocumentFragment isolation (the spec's "template contents owner")
    is still a follow-up — children currently inherit form/scope from
    the enclosing tree, which is the same simplification we use for
    SVG/MathML foreign content.
  - No SVG/MathML foreign content handling.
  - ~~No parser error reporting — we silently drop malformed input.~~ —
    `log::trace!` calls now fire on the most common tree-builder
    parse errors: stray doctype in body, stray `<html>`/`<head>`/
    `<body>`, stray table-structure tags outside a table, stray
    `</table>`, and any token that triggers foster parenting. Filters
    on the `oasis_browser::html::tree_builder` log target surface
    these without spamming general output.
- Full frameset support is **not** a goal — document it as a deliberate
  non-goal.

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
2. **`html5lib-tests` integration** (catches tree-builder weirdness in
   one shot, no speculative design needed).
3. ~~**CSS long-tail subset: `:has()` + `@layer` + `@container` +
   CSS nesting**~~ — all shipped. `:has()` and `@layer` on
   `feat/browser-has-selector`; CSS nesting on `feat/css-nesting`;
   `@container` on `feat/browser-container-queries`.
4. **Compositor overhaul** (high effort but unlocks mix-blend-mode,
   backdrop-filter, mask, isolation, filter, will-change in one
   architectural change).
5. **3D transforms** (lower user impact — most real sites degrade
   gracefully without them).
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
