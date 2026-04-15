# Browser Engine Backlog

Forward-looking gap analysis for `oasis-browser`. Open work only —
shipped epics are summarised in a single "Recently shipped" section
and otherwise tracked via `git log`.

Last updated: 2026-04-15

---

## Recently shipped (pointers only)

The big compatibility and architecture epics are done. See git log
for the detailed commit history; each bullet names the merge branch
so you can `git show` for specifics.

- **Compositor overhaul** (`feat/browser-compositor-overhaul`).
  `mix-blend-mode`, `backdrop-filter`, `filter:`, `isolation:
  isolate`, `will-change:`, and `mask-*` (including URL-backed
  masks) all route through a single `PushCompositingLayer` /
  `PopCompositingLayer` pair backed by the `SdiRenderTarget` trait.
  Backends without render-target support fall back to `PushLayer`.
- **3D transforms** (`feat/browser-3d-transforms` + follow-ups).
  4×4 `Matrix3d`, screen-space perspective projection, `transform-
  style: preserve-3d` with Z-sorted children, `backface-visibility`,
  `transform-origin: Z`, and a trapezoidal background path for
  steep perspective angles.
- **Real-world compatibility measurement**
  (`feat/browser-realworld-compat-epic`). Display-list golden
  harness for visual regression, hard wall-clock layout budgets
  gating `cargo test`, criterion corpus bench group, and a local-
  only triage binary for bucket-sorting arbitrary HTML snapshots.
- **WHATWG HTML conformance** (`feat/browser-whatwg-epic-completion`
  + earlier `feat/browser-whatwg-conformance`). Full §13.2.6.4.7
  adoption agency algorithm, foster parenting, `<template>` with
  form-scope isolation, foreign content subset (SVG/MathML), parser
  error reporting, and a vendored-subset `html5lib-tests` harness.
- **CSS long tail** (`feat/browser-has-selector`,
  `feat/browser-container-queries`, `feat/css-nesting`). `:has()`,
  `@layer`, `@container`, CSS nesting, `@scope`, `@property`,
  `@counter-style` (parse-only), `color-mix()` / `oklch()` /
  `color()` / `light-dark()`, logical properties, `aspect-ratio`
  for non-replaced blocks, `text-wrap` parsing, `field-sizing:
  content`, `accent-color`, `caret-color`.
- **PSP JavaScript** (`feat/psp-quickjs`). QuickJS-NG via `rquickjs`
  on real PSP hardware, same DOM bindings as desktop/WASM/UE5.

---

## Epic: Missing CSS features (remaining)

Low-impact. Deliberately deferred until real-world corpus pressure
surfaces breakage. Skipping these does not block launch.

- **View Transitions API** (`view-transition-*`).
- **Anchor Positioning** (CSS Anchor Positioning Module Level 1).
- **Subgrid**.
- **`scroll-timeline` / `animation-timeline`**.

---

## Follow-ups from shipped epics

Small, well-scoped items pulled out of already-shipped epics. Each
is small enough to handle as a drive-by on a related PR.

### Compositor / mask

- **`mask-size` / `mask-position` / `mask-repeat` for URL masks.**
  URL masks currently stretch to layer bounds with nearest-neighbor
  sampling. Wire `background_image_tiles` through `MaskParams` so
  positioned / tiled / sized masks work. The stretched-to-fit path
  already covers the rounded-avatar / logo-badge case.
- **Real GPU read-modify-write for layer filters.** The filter
  + mask pop path drops the CSS blend mode (becomes `Normal`)
  because it reads back, composites via a plain alpha-over blit,
  then throws the texture away. A GPU-side path would keep blend
  mode + filter + opacity composable.
- **Nested `@container` rules: AND-combining.** Current behavior
  is innermost-wins. Pathological circular dependencies also not
  resolved (single-pass relayout).
- **`@container style(...)` queries** parse but always evaluate
  false.

### 3D transforms

- **Z-index opt-outs inside `preserve-3d` subtrees.** Explicit
  `z-index` on a child inside a preserved 3D container currently
  flattens into the Z-sort instead of leaving the preserved plane.
- **Real perspective rendering on GPU backends.** PSP GU has
  `sceGumPerspective` for true perspective projection; the
  software path uses a 3-corner-fit affine.

### WHATWG HTML

- **Full DocumentFragment scope isolation for table + select**
  (currently form-scope isolation only).
- **SVG camelCase identifier round-trip** (`<foreignObject>`,
  `<textPath>`). Tag names are stored lowercased; expose a
  namespace-aware representation if real pages need it.

### CSS long tail

- **Custom counter styles wired to `<ol>` markers.**
  `@counter-style` parses into `Stylesheet.counter_styles` today
  but list-item rendering still uses the built-ins only.
- **`text-wrap: balance` / `pretty` layout-side algorithm.** Parsed
  and stored; fall through to `wrap` at layout time.
- **Replaced-element `aspect-ratio`** (`<img>`, `<video>`).
  Non-replaced blocks derive height from width × ratio; replaced
  elements don't yet.
- **Cross-stylesheet `@layer` name merging.** Layer names are
  sheet-local; cross-stylesheet ordering falls through to source
  order.
- **Color-space-aware `color-mix`.** Currently interpolates in
  linear sRGB regardless of the requested color space.
- **`light-dark()` color-scheme tracking.** Always returns the
  light-mode argument.
- **RTL support** anywhere in the engine. Logical properties
  rewrite to physical LTR at parse time.

### Real-world compatibility

- **RTL / bidi stress fixture** in the corpus.
- **`@media` responsive grid fixture** in the corpus.
- **Bench baseline as a CI gate.** Currently manual save via
  `cargo bench -- --save-baseline main`.
- **Triage tool `--parallel` flag** via rayon.

### PSP

- **Space-collapsing in JS-mutated text nodes.** `textContent`
  containing ASCII spaces renders without visible spaces on PSP
  only. Likely a `glyph_advance(' ')` returning 0 in the PSP
  bitmap font table (`oasis-backend-psp/src/font.rs`) or the text
  layout step collapsing whitespace after JS-triggered relayout.
- **`js_dom.rs` bootstrap bloat.** `JS_DOM_BOOTSTRAP` and
  `JS_CANVAS_BOOTSTRAP` are large string constants; feature-gate
  the canvas half to trim ~20 KB on PSP where canvas is unused.

---

## Launch-polish items

These don't show up as CSS properties but bite users first. File
as individual issues — not a single epic.

- **Font rendering quality across skins** — kerning, hinting,
  subpixel positioning. Especially on PSP where we have system
  TrueType fonts via `psp::font`.
- **Image decoding error recovery** — corrupt JPEG/PNG currently
  crashes the decode path. Should degrade to a placeholder.
- **Network error UX** — timeout, DNS fail, TLS error should
  produce a useful error page, not a blank screen.
- **HTTP/2 support** — we only speak HTTP/1.1. Many modern CDNs
  require h2. Blocks access to some sites entirely.
- **`@font-face` / web fonts** — completely missing. Fallback to
  system fonts works but looks wrong on branded pages.
- **Accessibility** — ARIA roles are parsed but not exposed to
  anything. Low priority for launch but should at least have a
  plan.

---

## Out of scope / non-goals

Documented so we stop relitigating them.

- **V8-level JS performance on PSP.** We ship QuickJS-NG on PSP;
  it's two orders of magnitude slower than V8 and that's fine for
  the target use cases.
- **Service workers, WebRTC, Web Audio API, IndexedDB.** Too much
  surface area for an embedded engine.
- **Full HTML5 frameset support.** Deliberate non-goal — the web
  has moved on.
- **SVG animation (SMIL).** Parse basic SVG paths only; complex
  SVG rendering is out of scope.
- **CSS Houdini.** Too new, no ecosystem demand.
