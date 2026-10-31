# Browser Engine (`oasis-browser`)

This document inventories what the embedded browser engine actually
supports. It's the long-form version of the one-line entry in
`CLAUDE.md`. If you're adding a feature, extending a pass, or trying to
figure out "does the engine already do X?", this is the index to read.

Forward-looking gaps live in [`browser-backlog.md`](browser-backlog.md).
Contributor guide: [`design.md`](design.md) §Browser. ADRs on the
arena DOM and backend traits: [`adr/001-arena-based-dom.md`](adr/001-arena-based-dom.md),
[`adr/003-backend-trait-design.md`](adr/003-backend-trait-design.md).

## Crates

The engine is split so the parsers and the loader build and test
without layout, paint or QuickJS:

| Crate | Contents | Depends on |
| --- | --- | --- |
| `oasis-html` | tokenizer, tree builder, arena DOM, `Hook<F>` embedder hooks | `log` |
| `oasis-css` | CSS tokenizer, parser, cascade, computed values, animations / transitions | `oasis-html`, `oasis-types` |
| `oasis-http` | `Url`, HTTP/1.1 + HTTP/2, Gemini framing, cookies, cache, CSP, IO thread, VFS loader | `oasis-net`, `oasis-vfs`, `oasis-types` |
| `oasis-js` | QuickJS-NG engine (`JsEngine`), console, timers, `fetch` plumbing, Web Storage | `rquickjs` |
| `oasis-browser` | layout, paint, `BrowserWidget`, forms, JS DOM bindings, `.gmi` parser + HTML translation, images, fonts, SVG, canvas | all of the above (`oasis-js` behind `javascript`) |

Inside `oasis-browser` the larger subsystems are directories:
`layout/` (block, inline, flex, grid, table, multicol, positioning),
`paint/record/` (the display-list recorder: box walk in `mod.rs`,
`decorations`, `text`, `replaced`, form `controls`),
`paint/display_list/` (items, optimizer, replay), `svg/` (`parse`,
`path`, `paint`), `widget/pipeline/` (`navigation`, `document`,
`stylesheets`, `scripts`, `gemini_page`), `widget/input/` (`click`,
`focus`, `forms`, `history`, `hover`, `js_events`, `scroll`) and
`js_dom/`.

`oasis-browser` re-exports `oasis_http` as `loader` and keeps
`internals::*` for benches, fuzzers and the PSP hook setters, so
embedders compile unchanged. Features forward: `javascript` →
`oasis-css/scripting`, `web-fonts` → `oasis-css/web-fonts`,
`parallel-style` → `oasis-css/parallel-style`, `psp` → `oasis-http/psp`.
`oasis-core`'s `http` / `browse` terminal commands use `oasis-http`
directly.

## Network stack

- **HTTP/1.1 + HTTP/2** — ALPN-negotiated `h2` over rustls. Full HPACK
  (RFC 7541) with static + dynamic table, integer codec, string
  literals, and a canonical-code Huffman decoder (verified against RFC
  Appendix C); header values that are not UTF-8 decode lossily.
  Sync frame layer (RFC 9113) handles HEADERS/CONTINUATION reassembly,
  DATA flow control via `WINDOW_UPDATE` (crediting padding too),
  PING/PONG, graceful GOAWAY, and `PUSH_PROMISE` refusal; request bodies
  (up to the 64 KiB default send window) go out as 16 KiB DATA frames.
  One request per connection — no multiplexing, but unblocks every CDN
  that hard-requires `h2` for the initial GET. DoS caps: frames over the
  advertised 1 MiB `MAX_FRAME_SIZE` are refused (GOAWAY
  `FRAME_SIZE_ERROR`) before their payload is allocated, a HEADERS +
  CONTINUATION block over 64 KiB gets GOAWAY `ENHANCE_YOUR_CALM`, as do
  more than 1024 frames without response progress (PING / SETTINGS /
  empty-DATA floods), and HPACK output is capped at the advertised
  64 KiB `MAX_HEADER_LIST_SIZE` (GOAWAY `COMPRESSION_ERROR`).
- **HTTP/1.1 framing** (`oasis-http/src/http1.rs`) is shared by the desktop and
  PSP clients: header lines are parsed (not substring-sniffed) to pick
  `Content-Length` / chunked framing, and the chunked terminator is
  found by walking the chunk sizes. Interim `1xx` heads (`103 Early
  Hints`) are skipped (they count against the 16 KiB header cap), and
  `204` / `304` responses and `HEAD` answers end with their head even
  when they carry a `Content-Length`. The desktop client gives one
  response at most 60 s (each read 15 s), sets a write timeout, and
  tries every resolved address in turn.
- **URLs** — network-scheme URLs are normalized at parse time: host
  lower-cased, `user:pw@` split into `Url::userinfo` (never sent),
  default port elided, `\` treated as `/`. Relative and absolute-path
  references are resolved with RFC 3986 dot-segment removal (`..` never
  climbs above `/`, directory references keep their trailing `/`).
  `Url::origin()` is the one origin used by CSP `'self'`, `fetch()` CORS
  and Web Storage. `data:` URLs (`loader::parse_data_uri`) percent-decode
  to raw bytes (`+` stays `+`); scheme, media type and `;base64` are
  case-insensitive.
- **Body caps** — response bodies are capped at 8 MB on desktop (2 MB on
  PSP), and gzip / deflate / brotli bodies are decoded through the same
  limit on HTTP/1.1 and HTTP/2, so a decompression bomb errors out.
- **Cookies** (`oasis-http/src/cookies.rs`, desktop only) — session-scoped
  RFC 6265 jar: host-only cookies, `Domain=` validated (no IP parents, a
  "PSL-lite" public-suffix guard), `Max-Age` / `Expires` expiry and
  deletion, `Secure` (sent over and accepted from `https:` only),
  `__Secure-` / `__Host-` prefixes, control-character and 4 KiB size
  checks, 50-per-domain / 3000-total caps; plain http can neither
  overwrite nor delete a `Secure` cookie. `SameSite` is enforced on send
  (`cookies::SameSiteContext`, from `ResourceRequest::initiator`):
  cross-site subresources get no `Strict` / `Lax` cookies and cross-site
  top-level navigations get `Lax` ones only with GET / HEAD; sites are
  PSL-lite registrable domains, a missing `SameSite` behaves as `None`,
  and GET navigations count as user-initiated (link clicks and typed
  URLs share one path). Cookies are computed and stored **per redirect
  hop**, so a cross-site redirect never carries the first site's
  cookies and a hop's `Set-Cookie` lands under the host that sent it.
  `document.cookie` is a separate per-page map (4 KiB per pair, 50
  pairs, `max-age=0` / past-`expires` deletion) that never reaches the
  network.
- **Redirect header policy** — cache validators go on the first hop
  only; `Cookie` / `Authorization` are dropped once a hop leaves the
  original origin; `Referer` follows `strict-origin-when-cross-origin`
  per hop (origin only cross-origin, nothing on an `https:` → `http:`
  downgrade); 301 / 302 / 303 rewrite POST to GET (body headers go with
  the body) while 307 / 308 keep method and body. A network response
  may only redirect to `http:` / `https:` (Gemini: `gemini:`).
- **Request hygiene** — `Url::request_target` percent-encodes controls,
  space and non-ASCII in the path and query (h1 request line, h2
  `:path`, PSP), and `validate_request` rejects non-token methods /
  header names and CR / LF / NUL in header values or the host before
  connecting. Keep-alive connections are pooled only after a response
  that ended at its framed length.
- **CSP** (`oasis-http/src/csp.rs`) — every `Content-Security-Policy` header is
  enforced (report-only and `<meta>` policies are ignored):
  `script-src(-elem)` gates inline `<script>` (matching `nonce`,
  `'sha256-…'` hash, or `'unsafe-inline'` when the list has no nonce /
  hash / `'strict-dynamic'`); `script-src-attr` gates inline event
  handlers one by one (`'unsafe-inline'`, or `'unsafe-hashes'` plus the
  hash of that handler's text); `style-src(-elem)` gates inline
  `<style>` (nonce / hash / `'unsafe-inline'`) and `<link
  rel=stylesheet>` / `@import` URLs; `style-src-attr` gates
  parser-inserted `style=""`; `connect-src` gates script `fetch()`
  (every redirect hop, and `data:` URLs). The policy is stored with the
  cached page and restored on back / forward re-renders and `304`s.
  Scripts and handlers inside `<template>` never run.
  `img-src` is parsed but not enforced, and external `<script src>` is
  never loaded. Scheme (`https:`, `data:`), host (`*.` wildcard, port,
  path), `'self'` and `*` sources follow CSP3 matching. PSP carries the
  CSP header too, but has no cookie jar.
- **`vfs://` isolation** — pages with a network origin cannot load
  `vfs://` images, stylesheets, `@import`s or fonts
  (`loader::subresource_allowed`); their `http(s)` subresources may still
  come from the offline `/sites/<host>/` cache.
- **Redirects** — page loads follow up to `BrowserConfig::max_redirects`
  (default 5) redirects; script `fetch()` keeps the default.
- **Background I/O thread** — network loads never block the UI thread.
  Dropping the widget mid-load returns immediately: queued requests are
  abandoned and a request still in flight finishes on the detached
  worker, which then exits.

## HTML parser

- WHATWG tokenizer + tree builder (§13.2): every insertion mode except
  the frameset and "in template" ones (template contents parse "in
  body", with DocumentFragment-style isolation of the form pointer and
  insertion mode), including in caption / column group / select /
  select in table / head noscript. The active formatting list has
  markers (cells, captions, templates, `applet` / `object` /
  `marquee`), reconstruction and the adoption agency follow the spec,
  and foster parenting runs the in-body rules at the "appropriate place
  for inserting a node". Quirks mode is not modelled (`<p><table>`
  always closes the paragraph).
- Foreign content: the builder tracks each element's namespace (the DOM
  does not store it). SVG `foreignObject` / `desc` / `title`, MathML
  `annotation-xml encoding="text/html"` and the MathML text elements
  are integration points whose content parses as HTML; SVG tag and
  attribute names get their camelCase back; HTML breakout pops to the
  nearest HTML element or integration point.
- `TreeBuilder::parse_with(html, ParseOptions { scripting })` lets the
  tree builder drive the tokenizer, as the spec does: raw-text / RCDATA
  / script-data / PLAINTEXT states only for HTML elements, CDATA
  sections only in foreign content, and `<noscript>` raw text only with
  scripting on. `load_html` passes the `javascript` feature, so builds
  without JS render `<noscript>` content. The pre-tokenized
  `TreeBuilder::build(Tokenizer::new(..).tokenize())` path remains
  (`innerHTML`, benches) and picks states by tag name. Script data
  implements the escaped / double-escaped states
  (`<!-- document.write("<script>…</script>") -->` stays one script).
- Input preprocessing: CR LF / CR become LF and NUL becomes U+FFFD.
  Character references: numeric ones map C1 controls through
  Windows-1252 (`&#150;` is `–`), `&#;` stays literal. Named references
  come from a ~300-entry table (`entities.rs`: all of HTML 4 plus common
  HTML5 names such as `&check;` and the ASCII punctuation names), not
  the full HTML list. Without a `;` only the legacy Latin-1 names match,
  as the longest prefix (`&copy2024` is `©2024`, `&mdash` stays
  literal), and in attributes not even those when an alphanumeric or
  `=` follows (`?a=1&copy=2`).
- Hostile-input bounds: at most 256 attributes per tag (duplicates
  dropped, first wins), 100 000 nodes, open elements and DOM depth both
  capped at `MAX_NESTING_DEPTH` (256; `MAX_DOM_DEPTH` = 257 including
  children appended at the cap), and a 200 000-attribute budget for the
  formatting-element clones the adoption agency and reconstruction make.
  End tags without a dedicated rule follow the spec walk, so a stray
  `</div>` or `</span>` cannot close an enclosing table cell.
- Vendored-subset `html5lib-tests` harness for conformance
  (`tests/fixtures/html5lib/*.dat`, each case run through both entry
  points).

## CSS cascade & selectors

- Linked stylesheets (`<link rel=stylesheet>`, network or `vfs://`)
  follow their `@import` rules (nested, resolved against the sheet URL,
  cascading before the importing sheet; print-only imports skipped; at
  most 16 imported URLs per page). `@import` in inline `<style>` is not
  followed.

- Tokenizer follows CSS Syntax for escapes (`\f101` in strings,
  `.md\:flex` in selectors), non-ASCII identifiers and unquoted
  `url(images/a.png)` / `url(data:…;base64,…)`.
- Viewport-aware `@media` / `@supports` queries — window dimensions
  threaded into `Stylesheet::parse_with_viewport` so desktop
  breakpoints no longer collapse to the 480x272 default. `em` / `rem`
  breakpoints use the 16 px initial font size. At-rules nest inside
  each other (`@media { @supports { … } }`).
- `var()` custom properties, `calc()`. `var()` is substituted at
  computed-value time anywhere in a value (`calc(var(--gap) * 2)`,
  `rgb(var(--r), 0, 0)`, gradients, shorthands) and re-parsed; names
  are case-sensitive, cycles and missing references without fallback
  are invalid at computed-value time (`unset` semantics). Expansion is
  bounded: 64 KiB per value and 1 MiB per element's custom-property
  map.
- Hostile-stylesheet bounds: rule bodies and at-rule blocks nest at
  most 32 deep, `calc()` parentheses 32 deep, a flattened nested rule
  carries at most 1024 selectors of at most 64 compounds, `repeat()`
  grids at most 1000 tracks, and 50 000 rules per sheet.
- `html { font-size: 62.5% }` resolution — the html element only uses
  the CSS "medium" 16 px baseline; `rem` units track html's computed
  font-size via a thread-local cell.
- Absolute-size font keywords (`x-small`..`xx-large`) anchored to the
  CSS 2.1 §15.7 16 px default, so `font-size: x-small` matches real
  browsers (~10 px) instead of collapsing against the 8 px PSP-tuned
  `ROOT_FONT_SIZE`.
- CSS length units: `px`, `em`, `rem`, `pt`, `ex`, `ch` (the latter
  two resolve to `0.5em`).
- CSS Nesting (`&` selector with parse-time desugaring); declarations
  directly inside a nested `@media` apply to the parent selector.
- Right-to-left selector matching with backtracking, so `A > B C`
  matches through an outer `B` when the nearest `B` doesn't fit
  (Servo-style failure kinds keep it linear in practice). Each
  selector-vs-element match has a 16k-step traversal budget, shared
  with `:has()` subtree scans and charged for sibling-list scans, plus
  a 128k-node allowance for the sibling scans of structural
  pseudo-classes (`:nth-child()`, `:last-of-type`, ...); running out
  counts as no match for the whole selector, `:not()` included.
- `:has()` relational pseudo-class with `>`/`+`/`~` leading
  combinators, scope-bounded inner combinators.
- Selectors Level 4 `:not(a, b, c)` list form.
- `@layer` cascade layers — statement / named-block / anonymous-block
  forms, cross-stylesheet name merging, factored into the cascade
  sort with `!important` inversion. Nested layers are qualified
  (`@layer a { @layer b }` is `a.b`) and ordered as a tree (CSS Cascade
  5): sublayers sort before their parent's own rules, and a sublayer
  declared late stays inside its parent's slot (`@layer a, b;
  @layer a.c` orders `a.c < a < b`), across stylesheets too.
- CSS-wide keywords: `inherit` takes the parent's value for
  non-inherited properties too (`background-color`, `width`, `border`,
  `display`, ...), and on shorthands (`border`, `background`, `font`,
  `outline`, `list-style`, `border-radius`, `flex`) reaches every
  longhand; `all: unset | initial | inherit` resets every modelled
  property but `direction` (`revert` is still ignored). The `flex` shorthand implements every form of Flexbox 1
  §7.1.1 (`auto`, `none`, `initial`, `<grow> <shrink>? || <basis>`).
- Attribute selectors: all six operators, the `i` / `s` case flags, and
  HTML's case-insensitive attribute values (`[type=checkbox]` matches
  `type="CHECKBOX"`); empty `^=` / `$=` / `*=` / `~=` values and
  malformed selectors match nothing.
- Colours: `#rgb[a]` / `#rrggbb[aa]`, all 148 CSS named colours,
  `rgb/rgba`, `hsl/hsla`, `oklch/oklab`, `color(srgb | srgb-linear |
  display-p3)`, `color-mix(in srgb | oklch | oklab | hsl |
  srgb-linear)`, `light-dark()` with `color-scheme` tracking. One parser
  serves stylesheets, `bgcolor`, SVG presentation attributes and canvas
  `fillStyle` / `strokeStyle` (`oasis_css::parse_color`).
- CSS logical properties: inline-axis logicals (margin / padding /
  border / inset `-inline-start|end`) are resolved against `direction`
  at cascade time, block-axis ones map to top / bottom, and
  `block-size` / `inline-size` map to height / width. `text-align:
  start | end` likewise follow `direction`.

## Layout

- **Block / inline / flex / grid / table.** Float `left` / `right` per
  CSS 2.1 §10.3.5 — floats keep their declared width, auto margins
  compute to 0, and float descendants shift with their parent
  post-placement so `float: right` sidebars land on the right edge
  with their children in the right place. `width: auto` floats use
  one-pass shrink-to-fit (`min(max-content, available)`): children
  lay out at the containing width, then `content.width` clamps down
  to the rightmost child border-box edge. A float inside a padded
  parent lands at the parent's content edge. A float never starts above
  an earlier float's top (§9.5.1 rule 5).
- **Flexbox** (`layout/flex.rs`): direction, wrap, `order`, grow /
  shrink, `flex-basis` (or `width` / `height`), `gap`, `justify-content`,
  `align-items` / `align-self`, `align-content` and main-axis auto
  margins. Content-sized items (`width: auto`, no basis) use the same
  fit-content measurement as floats. Columns are as tall as their items
  unless the container has a definite height, which `flex-grow` then
  fills; a single-line container gives its line the container's cross
  size. Inline children (a bare text run, `<a>` / `<span>`) are wrapped
  into anonymous items that carry their flex properties; whitespace-only
  runs are dropped. An item is laid out again only when flexing changed
  its size, then translated into place, so nesting stays linear.
- **CSS Grid.** Paren-aware track lists (`repeat(3, minmax(0, 1fr))`,
  `repeat(2, 1fr 2fr)`, `%` / `em` / `min-content` / `max-content` /
  `fit-content()` tracks), `repeat(auto-fill | auto-fit, ...)` resolved
  from the container width (auto-fit collapses empty tracks), line
  placement with `span`, negative lines and `a / b` syntax, named
  areas, and row/column auto-flow including `dense`. Subgrid is not
  supported. See `crates/oasis-browser/docs/layout.md`.
- **Inter-element whitespace.** A text node containing only whitespace
  between two inline siblings (`<a>x</a>\n<a>y</a>`) emits a single
  `space_width` fragment into the line flow, per CSS 2.1 §16.6.
- **Multi-column** (`column-count` / `column-width` / `column-gap`):
  children are distributed across balanced columns; a box that resolves
  to a single column is laid out as a normal block.
- **Table layout**: `colspan` / `rowspan` follow HTML's integer parsing
  and limits (1000 / 65534; `rowspan="0"` spans to the last row), and a
  span is clamped to the rows that exist. A table has at most 1000
  columns in total (cells past that are dropped). Occupancy is tracked per
  column, so huge spans cost nothing. Explicit pixel widths on `<td>`/`<th>`
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
- **Hostile-input bounds.** Layout runs in `f32`; once it finishes,
  every box's geometry saturates at ±2^25 px (`MAX_LAYOUT_PX`, Chrome's
  `LayoutUnit` range) so paint's integer arithmetic cannot overflow on
  `width: 1e30px`. Allocations sized by CSS values are bounded by the
  content instead (line boxes per fragment, column buckets per child,
  offscreen layers to the reachable surface; see
  `paint/display_list/replay.rs`). Work driven by CSS / markup values is
  capped too: grid auto-placement runs under a cell-inspection budget,
  background / mask tiles per box (4096, larger tiles past that),
  repeating-gradient repetitions (one per pixel), `blur()` sigma (100 px,
  running-sum passes), SVG `<pattern>` nesting (2), path points (65 536,
  coordinates clamped to ±1e7), even-odd triangulation (2048 points) and
  stroke stepping (clipped to ±16 384 px). Image decoders reject
  oversized headers before allocating (BMP, GIF frames, WebP via
  `image::Limits`). `tests/pipeline_fuzz.rs` drives the
  whole parse → style → layout → paint pipeline with generated hostile
  markup and CSS (raise `PROPTEST_CASES` for a longer run).

## 2D + 3D transforms

- Full 2D CSS transforms plus 3D transform functions (`rotateX/Y/Z`,
  `translate3d`, `scale3d`, `rotate3d`, `matrix3d`, `perspective()`)
  evaluated through a 4x4 `Matrix3d`.
- A transform applies to the element's own box (background, borders,
  overflow clip, compositing bounds, link hit region) and to its
  descendants. Translations move the box; rotations / scales / skews
  flatten to an `AffineTransform2D` around the screen-space
  transform origin, and backgrounds under them paint as transformed
  quads (borders, text and images follow the translation only).
- 3D transforms under an ancestor `perspective:` property route
  through a screen-space projection path that builds the full
  `T(vp) * Persp(d) * T(-vp) * T(origin) * local * T(-origin)` matrix
  and derives a 3-corner-fit affine via
  `Matrix3d::project_screen_rect_affine`.
- `transform-style: preserve-3d` propagates the parent's 4x4 matrix
  to descendants, whose backgrounds project all four corners through
  it (a true trapezoid); children are painted back to front by
  projected Z, except those with an explicit `z-index`.
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
  `quadraticCurveTo`, `fill`, `stroke`, `save`, `restore`. Commands are
  recorded and replayed every paint, so the list is bounded: a
  `clearRect` covering the canvas drops everything before it (the usual
  clear-and-redraw animation loop stays constant-size), and commands
  (16 384), path points (65 536 per path, 2^20 in total) and `save()`
  depth (1024) are capped.
- **SVG** (`svg/`) — `<rect>`, `<circle>`, `<ellipse>`, `<line>`,
  `<polygon>`, `<polyline>`, `<path>` (all commands, Bezier flattening)
  with fill-rule / linecap / linejoin; `<g>` transform composition and
  presentation-attribute inheritance; `<defs>` linear / radial gradients
  and patterns via `url(#id)`; `<text>` / `<tspan>` with `text-anchor`.
  Colours use the CSS parser; `currentColor` paints black.
- **One renderer: display list record + replay** (`paint/`). The
  recorder walks the layout tree in CSS 2.1 paint order and emits
  display items — including `Svg` / `Canvas` items, so inline SVG and
  `<canvas>` stack under later overlays, clip, scroll inside nested
  scroll containers and honour `visibility`. The widget caches the
  list and replays it with a scroll delta or dirty rectangles; tests,
  the visual-regression goldens and the paint bench run the same
  record + replay path (`internals::paint_page`). See
  [`crates/oasis-browser/docs/paint.md`](../crates/oasis-browser/docs/paint.md).
- **Light compositor** — batched rect + text submission via
  `SdiBatch::submit_rect_batch` / `submit_text_batch`,
  vertical/horizontal strip merging, occluded rect elimination,
  clip intersection, granular animation dirty tracking, and
  render-target layers for `mix-blend-mode`, `isolation`, box-level
  `filter` and `mask-*` (CPU filter / mask / blend passes need
  render-target readback; PSP falls back to plain opacity).
- **Images** — PNG / JPEG / GIF / BMP (+ WebP with `webp`). Images
  larger than `BrowserConfig::max_image_dimension` / `max_image_pixels`
  (desktop 4096 px / 2 Mpx, PSP 1024 px / 1 Mpx) are downscaled —
  JPEGs by DCT scaling during decode — and keep their natural layout
  size.

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
  the typed text. Clicking a field puts the caret at the end of its
  value.
- One form state with scripts: typed values are mirrored into the
  script-side DOM too (so `input.value` reads them and a later script
  DOM mutation doesn't wipe them), `input` events fire after the edit,
  and values scripts write (`input.value = ...`, hidden inputs
  included) are adopted by the `FormManager`, so they are edited and
  submitted.
- Form GET / POST submission (a GET replaces the action URL's query and
  keeps its fragment). Enter in a text field (or Confirm on a submit
  button) submits the owning form after the same validation a click
  runs; Enter in a `<textarea>` inserts a newline.
- Forms are rebuilt from the DOM on every page load via
  `populate_forms_from_dom` (walks every `<form>` and registers
  inputs, selects, textareas, and submit buttons with the
  `FormManager`).

## Web fonts (`@font-face`)

- Full CSS parsing of `font-family` / `src` / `font-weight` ranges /
  `font-style` / `font-display` / `unicode-range` descriptors.
- `FontFamily` as ordered name stack with generic fallbacks.
- `fontdue` TTF/OTF rasterizer behind the `web-fonts` feature.
- `FontRegistry` with CSS font matching, glyph texture cache for
  rendering, lazy font loading on first tick. `FontAwareTextMeasurer`
  exists but layout does not use it yet: web-font text is measured with
  bitmap-font metrics and drawn with the font's own advances.

## JavaScript DOM bindings

Handled by the `javascript` feature. Engine details are in
[`javascript-engine.md`](javascript-engine.md). The bindings exposed
to page scripts include:

- `document.getElementById`, `createElement`, `createTextNode`,
  `querySelector`, `querySelectorAll`.
- `textContent`, attributes, `innerHTML`, `classList`, `style`
  property. `innerHTML` / `outerHTML` serialize the text of raw-text
  elements (`<style>`, `<script>`, ...) verbatim and drop tag /
  attribute names a script created that markup cannot represent.
- `fetch()` (Promise-based, same-origin / private-network policy — see
  [`oasis-js.md`](oasis-js.md#storage-and-fetch)), `setTimeout` /
  `setInterval`.
- `localStorage` (persistent across navigations) / `sessionStorage`,
  partitioned per origin with a 5 MiB quota.
- `document.cookie` getter/setter, `history.pushState` /
  `replaceState`, `window.location` with assign/replace/reload.
- Event dispatch with three-phase capture/target/bubble via
  `__oasis_dispatch_with_bubbling`: click, keydown, keyup, mousedown,
  mouseup, mousemove. `addEventListener` options `{once, capture,
  passive}`, detail properties `clientX`/`clientY`/`key`/`code`,
  `stopPropagation`/`preventDefault`.
- **DOM mutation → layout reinvalidation.** Every mutating binding
  (`setAttribute`, `removeAttribute`, `settext`, `appendChild`,
  `removeChild`, `insertBefore`, `innerHTML`, `classList.add/remove/
  toggle`, `style.*` setters, `settitle`) flips a shared
  `Rc<Cell<bool>>` dirty flag. After every JS click / key dispatch
  the widget clones the JS-shared document back into its authoritative
  slot, re-runs `css::cascade::style_tree` against cached sheets,
  rebuilds the link map, and marks layout + repaint dirty. Without
  this, a handler that flipped a class would update the DOM but paint
  the pre-mutation layout until the next navigation.
  `classList.add` of a token the element already has (and similar
  no-ops) skip the flag so redundant reassignments don't force a
  re-cascade.
- **Robustness limits.** Script insertions that would nest a node
  deeper than the parser's own limit (`MAX_DOM_DEPTH`, 257 levels)
  throw `HierarchyRequestError`, and `innerHTML` flattens past it, so
  a script-built chain can't overflow the UI stack in the recursive
  render passes; text / comment nodes never take children. Scripts may
  grow the document to `MAX_SCRIPT_DOM_NODES` (200k) live nodes, past
  which node-creating calls throw `QuotaExceededError` (the arena sits
  outside the JS heap cap). Ids of freed nodes are rejected, so a
  wrapper kept across `innerHTML = ''` can't corrupt the tree. Each
  inline script, event dispatch and per-frame
  timer tick runs under `BrowserConfig::js_max_exec_ms` (2 s desktop,
  500 ms PSP; timers still due when a tick's budget runs out wait for
  the next frame), and the QuickJS heap is capped at
  `BrowserConfig::js_memory_limit_bytes` (64 MiB desktop, 8 MiB PSP).
  The console buffer, which lives outside that heap, keeps the last
  1000 messages of at most 4 KiB each.
  See [`oasis-js.md` §Watchdog](oasis-js.md#watchdog).
- **preventDefault-aware link follow-up.** The inline-handler wrapper
  calls `event.preventDefault()` when the body returns `false` (the
  HTML spec), and `__oasis_dispatch_with_bubbling` returns the event's
  `_defaultPrevented`. The Rust click handler skips link navigation
  and `<summary>` toggle when the JS handler preventDefault'd, so
  `onclick="return togglecomment(this)"` collapses the comment
  instead of following the `href="#"`.
- **Site-compat JS shims.** `install_site_compat_shims` defines
  `togglecomment` / `hidecomment` / `unhidecomment` / `morechildren` /
  `togglevote` globals when the page's own script bundle doesn't
  provide them (guarded by `typeof ... === 'undefined'`). The toggler
  shims walk to the nearest `.comment` ancestor and flip a
  `collapsed` class; `togglevote` switches `.up` ↔ `.upmod` (or
  `.down` ↔ `.downmod`) on vote arrows. Lets old.reddit threads stay
  interactive when `reddit.js` fails to load (or simply isn't
  available, as in the engine's offline fixtures).

## Browser chrome (UI around the page)

- URL bar with caret positioned via `bitmap_measure_text` (same
  measurer used by content paint), click-to-select-all URL, bookmarks
  via a "B" button that navigates to `vfs://bookmarks` (served inline
  from `nav::bookmarks_page_html()`).
- Back / Forward / Home / Bookmark buttons, 28 px tall chrome, 14 px
  labels vertically centered. There is no Reload button; reload is a
  key (below) or `browse reload`.
- `vfs://history` serves the history page like `vfs://bookmarks`
  (titles and URLs HTML-escaped).
- Keyboard (`BrowserWidget::handle_key`, delivered by the shell before
  the key's gamepad twin): F5 / Ctrl+R reload (no new history entry),
  Alt+Left / Alt+Right back / forward, Ctrl+L / F6 focus the URL bar,
  Page Up / Page Down and Home / End scroll while nothing is being
  typed. Escape (Cancel) discards a URL-bar edit or leaves a focused
  text field; otherwise the shell closes the browser window.
- The shell titles the window `<page title> - Browser` (following
  `document.title`) and re-themes the chrome of an open browser on a
  skin swap (`apply_chrome_theme`).
- Reader mode, link navigation.

## Related docs

- [`browser-backlog.md`](browser-backlog.md) — remaining work and
  known gaps.
- [`compositor-overhaul-plan.md`](archive/compositor-overhaul-plan.md) —
  compositor deep dive.
- [`javascript-engine.md`](javascript-engine.md) — JS engine and PSP
  cross-compile specifics.
- [`psp-architecture.md`](psp-architecture.md) — PSP-specific
  constraints on the browser (GU buffer, TLS, video).
- [`adr/001-arena-based-dom.md`](adr/001-arena-based-dom.md) — why
  the DOM uses arena allocation.
