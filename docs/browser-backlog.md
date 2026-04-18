# Browser Engine Backlog

Forward-looking gap analysis for `oasis-browser`. Open work only —
shipped epics are summarised in a single "Recently shipped" section
and otherwise tracked via `git log`.

Last updated: 2026-04-18

## Recently shipped (pointers only)

The big compatibility and architecture epics are done. See git log
for the detailed commit history; each bullet names the merge branch
so you can `git show` for specifics.

- **old.reddit.com interactivity + inline whitespace + float
  positioning** (`feat/browser-reddit-rendering`). Screenshot-driven
  iteration on old.reddit's listing and comments pages surfaced four
  gaps, each of which is a general engine improvement rather than a
  reddit-specific hack:

  1. **JS DOM mutations now trigger re-cascade + relayout.** A shared
     `Rc<Cell<bool>>` dirty flag is flipped by every mutating JS
     binding (`setAttribute`, `classList.add/remove/toggle`, inline
     `style.*`, `innerHTML`, `appendChild`, etc.). After every JS
     click / key / input event the widget clones the JS-shared
     document back into `self.document`, re-runs `style_tree` against
     cached sheets, rebuilds the link map, and marks layout + repaint
     dirty. Before this, `onclick="this.classList.add('foo')"` updated
     the DOM but the next paint still used the stale layout — reddit
     comment collapse, upvote visual feedback, and every
     click-to-toggle-class page was inert.

  2. **`preventDefault` / `return false` suppresses link follow-up.**
     `__oasis_dispatch_with_bubbling` returns `evt._defaultPrevented`,
     and the inline-handler wrapper calls `event.preventDefault()`
     when the body returns `false` (HTML spec). The Rust click path
     skips the `link_map` follow-up when the JS handler prevented
     default, so `onclick="return togglecomment(this)"` actually
     toggles the class instead of also navigating to `href="#"`.

  3. **Site-compat JS shims.** `install_site_compat_shims` registers
     globals — `togglecomment` / `hidecomment` / `unhidecomment` /
     `morechildren` / `togglevote` — that would otherwise only exist
     inside reddit's 1 MB `reddit.js` bundle. Each shim is guarded by
     `typeof ... === 'undefined'` so a real page script wins. The
     togglers walk to the nearest `.comment` ancestor via `parentNode`
     and flip a `collapsed` class that the page's existing CSS rules
     already hide/show; `togglevote` switches `.up` ↔ `.upmod` (or
     `.down` ↔ `.downmod`) on vote arrows. Lets reddit threads stay
     interactive even when the full script bundle doesn't load.

  4. **Float absolute-x fix.** `place_float` returns BFC-local
     coordinates (origin at the containing block's content edge) but
     `layout_block_children` was using the value verbatim as the
     float's absolute `content.x`, so a float inside a padded parent
     landed at `x=0` instead of the parent's content edge. Two visible
     symptoms: (a) hit-testing failed on the float's descendants
     because the parent's AABB check short-circuited the recursion
     (old.reddit vote arrows unreachable to clicks), (b) the wikipedia
     infobox and reddit link-post vote column overhung their card on
     the left. Fix is one `content_x +` addition; four other fixtures'
     goldens shifted inward as corrections.

  5. **Float `width: auto` shrink-to-fit** (CSS 2.1 §10.3.5). Was a
     documented TODO — floats with `width: auto` filled the entire
     containing width instead of computing `min(max-content,
     available)`. Now lays out children at the available width and
     clamps `content.width` down to the rightmost child border-box
     edge. Mirrors the existing inline-block shrink-to-fit approach.

  6. **Inter-element whitespace preservation** (CSS 2.1 §16.6). A
     text node containing only whitespace between inline siblings
     collapsed to `" "` and then `split_line_into_words` returned an
     empty vec because both sides of the `split(' ')` were empty
     strings. Every `<a>foo</a>\n<a>bar</a>` chain lost its space —
     reddit taglines rendered `[-]withoutboats412 points13 hours
     ago`, the rust docs nav read `stdcorealloctest`, wikipedia
     portal condensed whole prose runs. Fix: emit a zero-text,
     `trailing_space: true` placeholder when the input is
     whitespace-only; `make_text_fragments` contributes one
     `space_width` to the line, and `trim_line_boundary_spaces` still
     strips it at line edges. Eight goldens regenerated; each diff
     shows inserted spaces between inline siblings and corresponds to
     a legitimate reading improvement.

  **Supporting work.** `screenshot-tests` gained a `--size WxH` flag
  so desktop-width fixtures can be captured at 1024×768+ instead of
  the PSP-native 480×272. `test-fixtures/html/reddit_{comments,
  listing}.html` are symlinked to the browser tests' fixtures so the
  display-list visual-regression harness and the PNG screenshot
  binary render identical HTML. The comments fixture now includes
  realistic `.submitter` / `.moderator` / `.edited-timestamp` /
  `.morechildren` patterns, `.md blockquote` / `code` / `p` styling,
  and `<a class="expand" onclick="return togglecomment(this)">[–]</a>`
  anchors on every comment tagline. Three new interactive tests
  (`reddit_expand_click_collapses_subtree`,
  `reddit_vote_arrow_toggles_class`,
  `reddit_morechildren_click_does_not_navigate`) cover the full
  click → JS dispatch → DOM mutation → preventDefault → relayout
  loop, plus four unit tests pinning the new layout / text
  invariants.

- **google.com rendering + form interactivity + click hit-testing**
  (`feat/browser-google-rendering`). The OASIS-UA variant of the
  Google homepage (the 79 KB table-based legacy fallback served to
  non-mainstream user-agents) now renders with a real 25/50/25
  three-column layout, sized search input, clickable submit
  buttons, and — the part that made the "it's just a picture"
  comparison especially painful before — the search box is
  genuinely interactive: clicking focuses it, typing updates the
  displayed text, and Enter navigates to `/search?q=...`.

  **Table / layout fixes:** `measure_box_widths` in
  `layout/table.rs` now reads replaced elements' intrinsic
  dimensions via the newly-exported `replaced_dimensions`,
  so `<input>` in a `<td>` no longer collapses the cell to 0 px.
  `<td width="25%">` reserves its share via a new percent-
  constraint pass in `distribute_widths`, and explicit-pixel
  columns stay pinned instead of being rescaled.
  `layout_cell_content` wraps inline / replaced children in an
  anonymous block so an inputs-only cell gets a real inline
  formatting context. New presentational-hint mappings for
  `valign` (→ `vertical-align` on tr/td/th), table-level
  `cellpadding` (propagated to descendant cells via an ancestor
  walk), `<br clear="…">` (→ CSS `clear`), and `<center> > *`
  getting `margin-left/right: auto` so shrink-wrapped tables
  inside `<center>` end up centred.

  **Interactivity fixes:** `populate_forms_from_dom` runs on
  every page load — previously nothing wrote to
  `form_manager.forms`, so every `<input>` looked unowned to
  the click handler. `handle_form_element_click` walks up from
  the hit node looking for an `<input>`/`<button>`/`<textarea>`
  ancestor (Google's submit button is wrapped in
  `<span class="lsbb">`). Focused-element routing in
  `handle_input` delivers `TextInput(ch)` / `Backspace` /
  `Button::Confirm` to the form manager instead of the
  page-zoom shortcut or the link-activation path.
  `sync_form_values_to_dom` writes `form_manager` state back
  onto the `<input value="…">` attribute so the next relayout
  actually paints the typed characters — without it the
  layout's `ReplacedContent::TextInput { value }` is re-read
  from the original HTML on every dirty-layout tick.

  **Click hit-testing was off by 28 px.** Paint draws the
  layout tree offset by `url_bar_height` (plus scroll) but every
  `hit_test` call in `widget_input.rs` passed raw screen coords.
  Added `screen_to_layout` and applied it at all five hit-test
  sites — clicks on inputs, labels, `<details>` summaries, and
  the JS click/mouse dispatchers used to land on whatever was
  28 px above the real target.

  **Failed `background-image: url(…)` no longer tiles a red-X
  grid.** A new `broken_image_urls` set on `BrowserWidget`
  tracks URLs whose fetch or decode failed; the CSS-background
  assignment path skips them, so Google's `.lsb` button sprite
  (referencing a 404'd `nav_logo229.png`) falls through to the
  background-colour instead of repeating a 24×24 placeholder
  across every submit button.

  **Internal-link regression fix bundled in:** homepage hrefs
  were absolute paths like `/sites/home/about.html` which, on
  `vfs://sites/home/…`, resolved per RFC 3986 to
  `vfs://sites/sites/home/…` (authority "sites" + absolute path
  preserved) and then the missing VFS file fell through to
  `load_from_network`, which rejected `vfs:` with the
  misleading "unsupported network scheme" error. Homepage hrefs
  are now relative, and `VfsThenNetwork` no longer escapes to
  network for `vfs:`/`about:`/`data:` schemes.

  **Test coverage:** new `google_homepage.html` fixture + 480×272
  and 800×600 goldens wired into `visual_regression`, and three
  end-to-end form interaction tests (`form_click_focuses_input`,
  `form_typing_updates_value_and_reflects_in_dom`,
  `form_enter_submits`) that would have caught each bug above.
  Knock-on: HackerNews' 30-px rank / 14-px votelinks columns
  are now honoured (frontpage height dropped ~900 px);
  Wikipedia infobox cells flow inline content through a real
  IFC instead of the single-line approximation stub.
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
