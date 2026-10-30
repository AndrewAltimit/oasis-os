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
  initial GET. DoS caps: frames over the advertised 1 MiB
  `MAX_FRAME_SIZE` are refused (GOAWAY `FRAME_SIZE_ERROR`) before their
  payload is allocated, a HEADERS + CONTINUATION block over 64 KiB gets
  GOAWAY `ENHANCE_YOUR_CALM`, and HPACK output is capped at the
  advertised 64 KiB `MAX_HEADER_LIST_SIZE` (GOAWAY `COMPRESSION_ERROR`).
- **HTTP/1.1 framing** (`loader/http1.rs`) is shared by the desktop and
  PSP clients: header lines are parsed (not substring-sniffed) to pick
  `Content-Length` / chunked framing, and the chunked terminator is
  found by walking the chunk sizes.
- **URLs** — network-scheme URLs are normalized at parse time: host
  lower-cased, `user:pw@` split into `Url::userinfo` (never sent),
  default port elided, `\` treated as `/`. `Url::origin()` is the one
  origin used by CSP `'self'`, `fetch()` CORS and Web Storage.
- **Cookies** (`loader/cookies.rs`, desktop only) — session-scoped
  RFC 6265 jar: host-only cookies, `Domain=` validated (no IP parents, a
  "PSL-lite" public-suffix guard), `Max-Age` / `Expires` expiry and
  deletion, `Secure` (sent over and accepted from `https:` only),
  `__Secure-` / `__Host-` prefixes, control-character and 4 KiB size
  checks, 50-per-domain / 3000-total caps. `SameSite` is parsed but not
  enforced. Cookies are computed and stored **per redirect hop**, so a
  cross-site redirect never carries the first site's cookies and a hop's
  `Set-Cookie` lands under the host that sent it. `document.cookie` is a
  separate per-page map that never reaches the network.
- **Redirect header policy** — cache validators go on the first hop
  only; `Cookie` / `Authorization` are dropped once a hop leaves the
  original origin; `Referer` follows `strict-origin-when-cross-origin`
  per hop (origin only cross-origin, nothing on an `https:` → `http:`
  downgrade); body headers go with the body on a POST → GET rewrite.
- **CSP** (`loader/csp.rs`) — every `Content-Security-Policy` header is
  enforced (report-only and `<meta>` policies are ignored):
  `script-src(-elem)` gates inline `<script>` (matching `nonce`,
  `'sha256-…'` hash, or `'unsafe-inline'` when the list has no nonce /
  hash / `'strict-dynamic'`); `script-src-attr` gates inline event
  handlers (`'unsafe-inline'` only); `style-src(-elem)` gates inline
  `<style>` (nonce / hash / `'unsafe-inline'`) and `<link
  rel=stylesheet>` / `@import` URLs; `style-src-attr` gates
  parser-inserted `style=""`; `connect-src` gates script `fetch()`.
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

- Full WHATWG tokenizer + tree builder: adoption agency algorithm,
  foster parenting, `<template>` with full DocumentFragment scope
  isolation (form + insertion mode), foreign content subset
  (SVG/MathML), parser error reporting.
- Vendored-subset `html5lib-tests` harness for conformance.

## CSS cascade & selectors

- Linked stylesheets (`<link rel=stylesheet>`, network or `vfs://`)
  follow their `@import` rules (nested, resolved against the sheet URL,
  cascading before the importing sheet; print-only imports skipped; at
  most 16 imported URLs per page). `@import` in inline `<style>` is not
  followed.

- Viewport-aware `@media` / `@supports` queries — window dimensions
  threaded into `Stylesheet::parse_with_viewport` so desktop
  breakpoints no longer collapse to the 480x272 default.
- `var()` custom properties, `calc()`. `var()` is substituted at
  computed-value time anywhere in a value (`calc(var(--gap) * 2)`,
  `rgb(var(--r), 0, 0)`, gradients, shorthands) and re-parsed; names
  are case-sensitive, cycles and missing references without fallback
  are invalid at computed-value time (`unset` semantics).
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
- Right-to-left selector matching with backtracking, so `A > B C`
  matches through an outer `B` when the nearest `B` doesn't fit
  (Servo-style failure kinds keep it linear in practice). Each
  selector-vs-element match has a 16k-step traversal budget, shared
  with `:has()` subtree scans; running out counts as no match.
- `:has()` relational pseudo-class with `>`/`+`/`~` leading
  combinators, scope-bounded inner combinators.
- Selectors Level 4 `:not(a, b, c)` list form.
- `@layer` cascade layers — statement / named-block / anonymous-block
  forms, cross-stylesheet name merging, factored into the cascade
  sort with `!important` inversion.
- Modern color functions: `hsl/hsla`, `oklch/oklab`, `color(srgb |
  srgb-linear | display-p3)`, `color-mix(in srgb | oklch | oklab |
  hsl | srgb-linear)`, `light-dark()` with `color-scheme` tracking.
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
  to the rightmost child border-box edge. Float placement returns
  BFC-local coordinates but the caller adds the containing block's
  `content.x`, so a float inside a padded parent lands at the parent's
  content edge instead of overhanging at x=0 (the old.reddit
  `.midcol` vote-column bug).
- **CSS Grid.** Paren-aware track lists (`repeat(3, minmax(0, 1fr))`,
  `repeat(2, 1fr 2fr)`, `%` / `em` / `min-content` / `max-content` /
  `fit-content()` tracks), `repeat(auto-fill | auto-fit, ...)` resolved
  from the container width (auto-fit collapses empty tracks), line
  placement with `span`, negative lines and `a / b` syntax, named
  areas, and row/column auto-flow including `dense`. Subgrid is not
  supported. See `crates/oasis-browser/docs/layout.md`.
- **Inter-element whitespace.** A text node containing only whitespace
  between two inline siblings (`<a>x</a>\n<a>y</a>`) now emits a
  single `space_width` fragment into the line flow, per CSS 2.1 §16.6.
  Previously the split-into-words pass returned an empty vec on the
  `" "` collapsed result and swallowed the gap, producing `xy`
  instead of `x y` on every inline-sibling chain.
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
  the typed text. Clicking a field puts the caret at the end of its
  value.
- One form state with scripts: typed values are mirrored into the
  script-side DOM too (so `input.value` reads them and a later script
  DOM mutation doesn't wipe them), `input` events fire after the edit,
  and values scripts write (`input.value = ...`, hidden inputs
  included) are adopted by the `FormManager`, so they are edited and
  submitted.
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
  render passes. Each inline script, event dispatch and per-frame
  timer tick runs under `BrowserConfig::js_max_exec_ms` (2 s desktop,
  500 ms PSP; timers still due when a tick's budget runs out wait for
  the next frame), and the QuickJS heap is capped at
  `BrowserConfig::js_memory_limit_bytes` (64 MiB desktop, 8 MiB PSP).
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
  recently shipped epics.
- [`compositor-overhaul-plan.md`](archive/compositor-overhaul-plan.md) —
  compositor deep dive.
- [`javascript-engine.md`](javascript-engine.md) — JS engine and PSP
  cross-compile specifics.
- [`psp-architecture.md`](psp-architecture.md) — PSP-specific
  constraints on the browser (GU buffer, TLS, video).
- [`adr/001-arena-based-dom.md`](adr/001-arena-based-dom.md) — why
  the DOM uses arena allocation.
