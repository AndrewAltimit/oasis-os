# Browser Engine Backlog

Forward-looking gap analysis for `oasis-browser`. Open work only —
shipped epics are summarised in a single "Recently shipped" section
and otherwise tracked via `git log`.

Last updated: 2026-04-17

## Recently shipped (pointers only)

The big compatibility and architecture epics are done. See git log
for the detailed commit history; each bullet names the merge branch
so you can `git show` for specifics.

- **old.reddit.com rendering + address-bar polish**
  (`feat/browser-old-reddit-rendering`). Three fixes that together
  turn the listing and comments pages from an illegible overlap mess
  into a recognisable old.reddit layout. (1) CSS `ex` and `ch` length
  units are now parsed (both resolve to `0.5em`, the standard
  pre-typometric heuristic) — old.reddit's vote column uses
  `width: 4.1ex` and the post rank uses `width: 2.2ex`, so treating
  those as invalid was collapsing the whole midcol gutter to zero.
  (2) Absolute-size font keywords (`x-small` / `small` / `medium` /
  `large` / `x-large` / `xx-large`) now anchor to the CSS 2.1 §15.7
  16 px baseline instead of the PSP-tuned `ROOT_FONT_SIZE` (8 px), so
  `font-size: x-small` renders at ~10 px as real browsers show
  instead of 5 px. (3) Floated blocks no longer inherit the
  normal-flow over-constrained rule (CSS 2.1 §10.3.3) that absorbs
  leftover width into `margin-right`; per §10.3.5 floats keep their
  declared width and auto margins compute to 0, so `.side { float:
  right; width: 300px }` inside a 1280 px container now places at the
  right edge instead of at `x=0`. Float descendants also shift with
  their parent post-placement (same subtree-delta trick the centered
  `margin: 0 auto` path already used). Address-bar polish rolled in
  on the same branch: caret uses `bitmap_measure_text` instead of
  the hardcoded 8-px-per-char assumption, click-to-focus selects the
  whole URL so the next keystroke replaces it (Firefox/Chrome
  behaviour), a new "B" button next to "H" navigates to
  `vfs://bookmarks` (served inline from `nav::bookmarks_page_html`),
  chrome height bumped 20→28 px for usable tap targets. Real-world
  test-page shortcuts (wikipedia / old.reddit / google) added to the
  browser homepage at `vfs://sites/home/index.html`.
- **`@font-face` / web fonts** (`feat/browser-font-face`). Full
  `@font-face` CSS parsing (family, src url/local, font-weight
  ranges, font-style, font-display, unicode-range). `FontFamily`
  extended from 3-variant enum to ordered name stack with generic
  fallbacks. `fontdue` TTF/OTF rasterizer behind `web-fonts` feature.
  Font registry with CSS font matching, font-aware text measurement,
  glyph texture cache, lazy font loading on first tick with
  `web_font_id` resolution on `ComputedStyle`.
- **HTTP/2** (`feat/browser-http2`). ALPN-negotiated `h2` on the
  existing rustls TLS path. Full HPACK (RFC 7541) — static + dynamic
  table, integer codec, string literals, Huffman decoder, verified
  against RFC Appendix C examples. Sync frame layer (RFC 9113) with
  HEADERS/CONTINUATION reassembly, DATA flow control via
  `WINDOW_UPDATE`, PING/PONG, graceful GOAWAY, and `PUSH_PROMISE`
  refusal. One request per connection — no multiplexing, but
  unblocks every CDN that hard-requires `h2` for the initial GET.
- **Image decoding error recovery + network error UX**
  (`feat/browser-image-error-recovery`). `catch_unwind` wraps all
  format-specific decoders; decode failures produce a broken-image
  placeholder. Error pages categorize failures (DNS, timeout, TLS,
  redirect loop) with styled explanations and suggested actions.
  `mask-size`/`mask-position`/`mask-repeat` wired for URL masks.
- **Compositor overhaul** (`feat/browser-compositor-overhaul`).
  `mix-blend-mode`, `backdrop-filter`, `filter:`, `isolation:
  isolate`, `will-change:`, and `mask-*` (including URL-backed
  masks) all route through a single `PushCompositingLayer` /
  `PopCompositingLayer` pair backed by the `SdiRenderTarget` trait.
  Backends without render-target support fall back to `PushLayer`.
- **3D transforms** (`feat/browser-3d-transforms` + follow-ups).
  4x4 `Matrix3d`, screen-space perspective projection, `transform-
  style: preserve-3d` with Z-sorted children, `backface-visibility`,
  `transform-origin: Z`, and a trapezoidal background path for
  steep perspective angles.
- **Real-world compatibility measurement**
  (`feat/browser-realworld-compat-epic`). Display-list golden
  harness for visual regression, hard wall-clock layout budgets
  gating `cargo test`, criterion corpus bench group, and a local-
  only triage binary for bucket-sorting arbitrary HTML snapshots.
- **WHATWG HTML conformance** (`feat/browser-whatwg-epic-completion`
  + earlier `feat/browser-whatwg-conformance`). Full adoption agency
  algorithm, foster parenting, `<template>` with full DocumentFragment
  scope isolation (form + insertion mode), foreign content subset
  (SVG/MathML), parser error reporting, and a vendored-subset
  `html5lib-tests` harness.
- **CSS long tail** (`feat/browser-has-selector`,
  `feat/browser-container-queries`, `feat/css-nesting`,
  `feat/browser-backlog-batch`). `:has()`, `@layer`, `@container`
  (incl. nested AND-combining + `style(...)` queries), CSS nesting,
  `@scope`, `@property`, `@counter-style` (full system support
  driving list markers), `color-mix()` / `oklch()` / `color()` /
  `light-dark()`, RTL-aware logical properties (direction-resolved
  at cascade time), `aspect-ratio` for replaced + non-replaced
  elements, `text-wrap: balance` / `pretty`, `field-sizing:
  content`, `accent-color`, `caret-color`.
- **PSP JavaScript** (`feat/psp-quickjs`). QuickJS-NG via `rquickjs`
  on real PSP hardware, same DOM bindings as desktop/WASM/UE5.
- **Follow-up batch** (`feat/browser-backlog-batch`). Preserve-3d
  z-index opt-outs, template scope isolation for table/select,
  RTL/bidi + responsive grid corpus fixtures, triage `--parallel`
  flag, benchmark CI gate.
- **Wikipedia rendering fixes** (`feat/browser-wikipedia-rendering`).
  Six production bugs that broke every page using modern CSS
  patterns, exposed by treating www.wikipedia.org as the canonical
  test case. `Stylesheet::parse()` was using the 480x272 default
  viewport for `@media` evaluation, so every desktop window silently
  got the mobile breakpoints of any page with `@media (max-width:
  ...)` — fixed by threading the real window viewport through
  `widget_pipeline::collect_style_sheets`. Absolute-positioned boxes
  got auto-margin absorption from the block-flow constraint solver
  (a 124.8-wide box inside a 436.8-wide container ended up with
  `margin-right: 312` and painted hundreds of pixels off-screen);
  the over-constrained-margin rule now short-circuits for
  `position: absolute/fixed`. `apply_absolute_position` moved the
  box itself but not its descendants, so `<strong>`/`<small>` inside
  a positioned `<div>` kept their pre-positioning coordinates —
  fixed by shifting the whole subtree by the delta. `margin: 0 auto`
  horizontal centering computed the auto-margin but never updated
  `content.x`, because `calculate_block_width` resolved auto margins
  *after* `layout_block_children` had baked the pre-resolution
  margin into x; fixed by snapshotting x before the recursive layout
  call and shifting the subtree by the post-layout delta in both
  `layout_block_children` and `layout_children_incremental`. The
  `<html>` element now seeds its parent-font-size with the CSS
  "medium" baseline (16px) so Wikipedia-style `html { font-size:
  62.5% }` resolves to 10px as authors intend, rather than 5px
  against our 8px engine default. `rem` resolution now reads a
  thread-local cell that the cascade sets once the html element has
  been styled, so `1.4rem` on Wikipedia's body is 14px instead of
  11.2px — thread-local (not atomic) so parallel-test cascades for
  desktop and PSP viewports don't race. Image improvements on the
  same branch: multi-layer `background-image` picks the URL layer
  (Wikipedia's `linear-gradient(transparent,transparent), url(...)`
  sprite pattern used to drop the sprite silently); `image/svg+xml`
  (including data URIs) is detected by a textual probe and produces
  a transparent RGBA placeholder sized from the SVG's
  `width`/`height`/`viewBox` so layout reserves the right space and
  the broken-image `×` stops appearing for sprite elements; PNG
  indexed-palette decode is now enabled via
  `Transformations::EXPAND`; broken-image alt text scales with the
  element's computed font-size and is clipped to the box.
- **Follow-up batch 12** (`feat/browser-backlog-batch-12`). CPU
  blend-mode compositing for filter/mask pop path (all 16 CSS blend
  modes), `FillPolygon` display list item with 4-corner perspective
  projection for 3D-transformed backgrounds in the recording path,
  PSP bitmap font proportional advance fix (space-collapsing bug).
  SVG `<defs>` pipeline: `<linearGradient>`, `<radialGradient>`,
  `<pattern>` definitions with `fill="url(#id)"` resolution,
  presentation attribute inheritance from `<g>` groups, `<text>`
  with `text-anchor`/`letter-spacing`/`font-weight`/`opacity`,
  `<tspan>` children, and gradient/pattern fill rendering.

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

- ~~**Real GPU read-modify-write for layer filters.**~~ **Shipped
  (`feat/browser-backlog-batch-12`).** The filter + mask pop path
  now reads destination pixels and applies all 16 CSS blend modes
  on CPU via `cpu_blend_composite` in `filter_chain.rs`. Non-Normal
  blend modes are fully preserved through the filter/mask readback
  path. A future GPU-side path could still replace this for
  performance, but correctness is no longer degraded.

### 3D transforms

- ~~**Real perspective rendering on GPU backends.**~~ **Partially
  shipped (`feat/browser-backlog-batch-12`).** The display list
  recording path now computes and propagates the full 4x4
  `ambient_screen_matrix` through 3D-transformed subtrees and
  emits `FillPolygon` items with true 4-corner perspective
  projection for backgrounds. PSP GU `sceGumPerspective` hardware
  path remains a future follow-up.

### PSP

- ~~**Space-collapsing in JS-mutated text nodes.**~~ **Shipped
  (`feat/browser-backlog-batch-12`).** Root cause: PSP
  `draw_text_bitmap` used constant `GLYPH_WIDTH = 8` for cursor
  advance instead of proportional `glyph_advance_scaled()`. Fixed
  by switching to per-character proportional advances from
  `oasis_types::bitmap_font`, matching `bitmap_measure_text`.

---

## Launch-polish items

These don't show up as CSS properties but bite users first. File
as individual issues — not a single epic.

- **Font rendering quality across skins** — kerning, hinting,
  subpixel positioning. Especially on PSP where we have system
  TrueType fonts via `psp::font`.
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
