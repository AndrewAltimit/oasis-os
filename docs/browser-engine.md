# Browser Engine (`oasis-browser`)

This document inventories what the embedded browser engine actually
supports. It's the long-form version of the one-line entry in
`CLAUDE.md`. If you're adding a feature, extending a pass, or trying to
figure out "does the engine already do X?", this is the index to read.

Forward-looking gaps live in [`browser-backlog.md`](browser-backlog.md).
Contributor guide: [`design.md`](design.md) §Browser. ADRs on the
arena DOM and backend traits: [`adr/001-arena-based-dom.md`](adr/001-arena-based-dom.md),
[`adr/003-backend-trait-design.md`](adr/003-backend-trait-design.md).

## Network stack

- **HTTP/1.1 + HTTP/2** — ALPN-negotiated `h2` over rustls. Full HPACK
  (RFC 7541) with static + dynamic table, integer codec, string
  literals, and Huffman decoder (verified against RFC Appendix C).
  Sync frame layer (RFC 9113) handles HEADERS/CONTINUATION reassembly,
  DATA flow control via `WINDOW_UPDATE`, PING/PONG, graceful GOAWAY,
  and `PUSH_PROMISE` refusal. One request per connection — no
  multiplexing, but unblocks every CDN that hard-requires `h2` for the
  initial GET.
- **Cookies, gzip, CSP.** Scripts/styles/connect-src enforced;
  img-src relaxed for practicality.

## HTML parser

- Full WHATWG tokenizer + tree builder: adoption agency algorithm,
  foster parenting, `<template>` with full DocumentFragment scope
  isolation (form + insertion mode), foreign content subset
  (SVG/MathML), parser error reporting.
- Vendored-subset `html5lib-tests` harness for conformance.

## CSS cascade & selectors

- Viewport-aware `@media` / `@supports` queries — window dimensions
  threaded into `Stylesheet::parse_with_viewport` so desktop
  breakpoints no longer collapse to the 480x272 default.
- `var()` custom properties, `calc()`.
- `html { font-size: 62.5% }` resolution — the html element only uses
  the CSS "medium" 16 px baseline; `rem` units track html's computed
  font-size via a thread-local cell.
- Absolute-size font keywords (`x-small`..`xx-large`) anchored to the
  CSS 2.1 §15.7 16 px default, so `font-size: x-small` matches real
  browsers (~10 px) instead of collapsing against the 8 px PSP-tuned
  `ROOT_FONT_SIZE`.
- CSS length units: `px`, `em`, `rem`, `pt`, `ex`, `ch` (the latter
  two resolve to `0.5em`).
- CSS Nesting (`&` selector with parse-time desugaring).
- `:has()` relational pseudo-class with `>`/`+`/`~` leading
  combinators, scope-bounded inner combinators.
- Selectors Level 4 `:not(a, b, c)` list form.
- `@layer` cascade layers — statement / named-block / anonymous-block
  forms, cross-stylesheet name merging, factored into the cascade
  sort with `!important` inversion.
- Modern color functions: `hsl/hsla`, `oklch/oklab`, `color(srgb |
  srgb-linear | display-p3)`, `color-mix(in srgb | oklch | oklab |
  hsl | srgb-linear)`, `light-dark()` with `color-scheme` tracking.
- CSS logical properties — parse-time rewrite to LTR physical
  equivalents for margin / padding / border / inset / block-size /
  inline-size longhands and shorthands.

## Layout

- **Block / inline / flex / grid / table.** Float `left` / `right` per
  CSS 2.1 §10.3.5 — floats keep their declared width, auto margins
  compute to 0, and float descendants shift with their parent
  post-placement so `float: right` sidebars land on the right edge
  with their children in the right place.
- **Table layout** honours explicit pixel widths on `<td>`/`<th>`
  (pinned — not rescaled when the table has slack) and percent
  widths (`<td width="25%">`) via a pre-pass in `distribute_widths`
  that reserves each percent column's share before auto columns
  fight over the remainder. Replaced children (`<input>`, `<img>`,
  etc.) contribute their intrinsic dimensions to cell preferred
  widths via `replaced_dimensions` instead of collapsing the cell
  to 0 px. A cell with only inline / replaced children is wrapped
  in an anonymous block so it gets a real inline formatting
  context.
- **Presentational HTML attributes → CSS:** `bgcolor`, `align`,
  `valign`, `nowrap`, `cellspacing`, `cellpadding` (table-level,
  propagated to descendant cells via an ancestor walk),
  `width`/`height` on `td`/`th`/`img`/`input`, `border` on
  `<table>`, `size` on `<input>` (→ width), `<br clear="left|
  right|all">` (→ CSS `clear`). `<center>` centres both inline
  content (via `text-align`) and block children (via
  `margin-left/right: auto`).
- **`text-wrap`** — `balance` binary-searches narrowest equal-line-
  count width, `pretty` avoids orphans, `stable` parsed.
- **`aspect-ratio`** for non-replaced blocks and replaced elements —
  width-driven height; CSS ratio overrides intrinsic.
- Nested scroll containers (`overflow: auto/scroll` with per-element
  scroll offsets).
- Sticky element scroll caching via `PushSticky` / `PopSticky`
  display items.
- Soft hyphen (U+00AD) line breaking with visible hyphens.
- Bidi text direction detection (Hebrew / Arabic / Syriac).

## 2D + 3D transforms

- Full 2D CSS transforms plus 3D transform functions (`rotateX/Y/Z`,
  `translate3d`, `scale3d`, `rotate3d`, `matrix3d`, `perspective()`)
  evaluated through a 4x4 `Matrix3d`.
- Pure 2D transforms flatten orthographically via
  `AffineTransform2D::from_css_transforms`.
- 3D transforms under an ancestor `perspective:` property route
  through a screen-space projection path that builds the full
  `T(vp) * Persp(d) * T(-vp) * T(origin) * local * T(-origin)` matrix
  and derives a 3-corner-fit affine via
  `Matrix3d::project_screen_rect_affine`.
- `transform-style: preserve-3d` propagates the parent's 4x4 matrix
  to descendants via `PaintContext.preserved_3d`.
- `transform-origin` (X/Y/Z) and `perspective-origin` parsed into
  structured types, resolved via shared helpers.
- `backface-visibility: hidden` culls back-facing subtrees via
  surface-normal Z.

## Animations & transitions

- Hover-triggered CSS transitions — 27 numeric properties
  auto-interpolate on state change.
- `::before` / `::after` pseudo-elements.
- `text-overflow: ellipsis`.
- Z-index stacking contexts, CSS 2.1 appendix E paint order.

## Canvas, SVG, compositor

- **Canvas 2D path API** — `beginPath`, `bezierCurveTo`,
  `quadraticCurveTo`, `fill`, `stroke`, `save`, `restore`.
- **SVG** — paths with fill-rule / linecap / linejoin, `<g>` group
  transform composition.
- **Light compositor** — display list with batched rect + text
  submission via `SdiBatch::submit_rect_batch` /
  `submit_text_batch`, vertical/horizontal strip merging, occluded
  rect elimination, clip intersection optimization, granular
  animation dirty tracking.

## Forms

- Text input, password masking, checkbox, radio button, textarea,
  select with dropdown overlay, submit/reset buttons.
- `<label for="">` click association, Tab focus navigation.
- Direct click on `<input>` / `<button>` / `<textarea>` focuses or
  submits (the click handler walks up from the hit node, so clicks
  on wrapper spans like `<span class="lsbb"><input…>` still land on
  the input).
- Physical keyboard typing: `TextInput(ch)` / `Backspace` /
  `Button::Confirm` route to the focused form element through
  `dispatch_form_key`, and in-flight values sync back to the DOM
  `value` attribute on each keystroke so the next relayout paints
  the typed text.
- Form GET / POST submission; Enter on any text field submits the
  owning form.
- Forms are rebuilt from the DOM on every page load via
  `populate_forms_from_dom` (walks every `<form>` and registers
  inputs, selects, textareas, and submit buttons with the
  `FormManager`).

## Web fonts (`@font-face`)

- Full CSS parsing of `font-family` / `src` / `font-weight` ranges /
  `font-style` / `font-display` / `unicode-range` descriptors.
- `FontFamily` as ordered name stack with generic fallbacks.
- `fontdue` TTF/OTF rasterizer behind the `web-fonts` feature.
- `FontRegistry` with CSS font matching, `FontAwareTextMeasurer` for
  layout, glyph texture cache for rendering, lazy font loading on
  first tick.

## JavaScript DOM bindings

Handled by the `javascript` feature. Engine details are in
[`javascript-engine.md`](javascript-engine.md). The bindings exposed
to page scripts include:

- `document.getElementById`, `createElement`, `createTextNode`,
  `querySelector`, `querySelectorAll`.
- `textContent`, attributes, `innerHTML`, `classList`, `style`
  property.
- `fetch()`, `setTimeout` / `setInterval`.
- `localStorage` (persistent across navigations) / `sessionStorage`.
- `document.cookie` getter/setter, `history.pushState` /
  `replaceState`, `window.location` with assign/replace/reload.
- Event dispatch with three-phase capture/target/bubble via
  `__oasis_dispatch_with_bubbling`: click, keydown, keyup, mousedown,
  mouseup, mousemove. `addEventListener` options `{once, capture,
  passive}`, detail properties `clientX`/`clientY`/`key`/`code`,
  `stopPropagation`/`preventDefault`.

## Browser chrome (UI around the page)

- URL bar with caret positioned via `bitmap_measure_text` (same
  measurer used by content paint), click-to-select-all URL, bookmarks
  via a "B" button that navigates to `vfs://bookmarks` (served inline
  from `nav::bookmarks_page_html()`).
- Back / Forward / Home / Bookmark buttons, 28 px tall chrome, 14 px
  labels vertically centered.
- Reader mode, link navigation.

## Related docs

- [`browser-backlog.md`](browser-backlog.md) — remaining work and
  recently shipped epics.
- [`compositor-overhaul-plan.md`](compositor-overhaul-plan.md) —
  compositor deep dive.
- [`javascript-engine.md`](javascript-engine.md) — JS engine and PSP
  cross-compile specifics.
- [`psp-architecture.md`](psp-architecture.md) — PSP-specific
  constraints on the browser (GU buffer, TLS, video).
- [`adr/001-arena-based-dom.md`](adr/001-arena-based-dom.md) — why
  the DOM uses arena allocation.
