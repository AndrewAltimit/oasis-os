# Browser Engine Backlog

Forward-looking gap analysis for `oasis-browser` and its parser / loader
crates (`oasis-html`, `oasis-css`, `oasis-http`, `oasis-js`). Open work
only: what ships is catalogued in [`browser-engine.md`](browser-engine.md),
and the history of each epic lives in `git log` (search the commit
subjects, e.g. `git log --oneline --grep=browser`).

Last updated: 2026-10-30

## Missing CSS features

Low-impact. Deliberately deferred until real-world corpus pressure
surfaces breakage.

- **View Transitions API** (`view-transition-*`).
- **Anchor Positioning** (CSS Anchor Positioning Module Level 1).
- **Subgrid** (`grid-template-*: subgrid` is not parsed).
- **`scroll-timeline` / `animation-timeline`**.

## Known gaps (verified against the code)

Real, reproducible holes in features that are otherwise shipped. Each is
small enough for a focused PR.

### Layout and paint

- **`line-clamp` is parsed but never applied.** `-webkit-line-clamp` /
  `line-clamp` populate `ComputedStyle::line_clamp`
  (`oasis-css/src/values/apply/mod.rs`), but nothing in `layout/` reads
  it, so clamped text renders every line and no ellipsis is drawn.
- **`position: sticky` never releases at its containing block.**
  `sticky_offset` (`paint/display_list/mod.rs`, used by both the
  recorder and replay) pins against the viewport `top` / `bottom` only;
  a sticky header keeps sticking after its parent has scrolled away.
- **Grid line-name references.** `[name]` groups in track lists are
  skipped by the tokenizer (`parse_plain_tracks`), so placements like
  `grid-column: content-start / content-end` only resolve when the names
  come from `grid-template-areas`.
- **Grid `align-items` / `align-self`.** Items are stretched to the cell
  width, but block-axis alignment is ignored: items are not stretched to
  the row height and `align-items: center | end` has no effect.
- **Rotated boxes rotate only their background.** Under a rotate /
  scale / skew / 3D transform the recorder emits the background as a
  `FillPolygon` quad, but borders, outlines, text and images keep
  following the translation only (`paint/record/`).
- **`backdrop-filter` is not sampled.** It only isolates the element into
  a compositing layer; the backdrop is never read or filtered.
- **`rowspan="0"` spans to the end of the table**, not the end of its row
  group (`layout/table.rs` has no row-group boundaries).
- **Oversized compositing layers are clipped.** A layer longer than
  4352 px on a side gets an offscreen target covering only the part that
  can reach the surface (`bound_layer_axis` in
  `paint/display_list/replay.rs`); a `mask-image` on such a layer is
  positioned against the clipped rect.
- **SVG `currentColor` paints black.** `svg::parse_svg_color` has no
  inherited CSS `color` to resolve against.

### Parsers

- **No quirks-mode CSS.** The parser picks the quirks mode from the
  DOCTYPE (`Document::quirks_mode`) and applies its one tree difference,
  but no quirks-mode CSS rules (table font inheritance, body height,
  unitless lengths) apply. `innerHTML` always parses as quirks mode
  (its wrapper has no DOCTYPE), so `<p><table>` there keeps the table
  inside the paragraph even in a standards-mode page.
- **Template contents are children, not a DocumentFragment.** They
  parse with the "in template" mode, but there is no `template.content`:
  JS and CSS see them as the element's children.
- **The DOM stores no namespaces.** The tree builder tracks them while
  parsing, but CSS (`svg|rect`) and JS (`namespaceURI`) cannot see them,
  and an SVG `<title>` is indistinguishable from the HTML one.
- **`innerHTML` uses the pre-tokenized path** (`js_dom/node_api.rs`), not
  fragment parsing: states are chosen by tag name and there is no
  context element.
- **Named character references** come from a ~300-entry table (HTML 4
  plus common HTML5 names), not the full 2 231-name HTML list.

### Network and security

- `<meta http-equiv="Content-Security-Policy">` is ignored.
- `'sha384-…'` / `'sha512-…'` CSP hashes are recognised but never match.
- Script-written `style=""` attributes bypass `style-src-attr`, and
  `style=""` is checked as a whole (so `'unsafe-hashes'` style hashes
  don't match).
- `'unsafe-eval'` is not enforced (`eval` / `new Function` /
  `setTimeout(string)` always work).
- PSP has no cookie jar (`load_from_network` on PSP carries CSP but not
  cookies).
- HTTPS has no connection reuse (fresh TCP + TLS per request; h2 sends
  GOAWAY after one stream).
- `SameSite` leftovers: not applied when *storing*, a missing `SameSite`
  is not defaulted to `Lax`, same-site is schemeless, and GET navigations
  count as user-initiated (link clicks and typed URLs share
  `navigate_vfs_inner`, so a cross-site link still sends `Strict`
  cookies).
- The global cookie cap evicts the oldest cookie of any site, so one
  site with many subdomains can push out others' sessions.
- Redirect chains A → B → A re-send `Authorization` on the return hop.
- A stale JS wrapper whose freed slot was reused aliases the new node (no
  generation counter; the tree itself stays well-formed).

## Follow-ups

- **PSP GU perspective.** 3D transforms are projected on the CPU
  (`FillPolygon` quads with the full 4x4 screen matrix); the
  `sceGumPerspective` hardware path is not wired.
- **GPU-side layer filters.** Filter / mask / non-Normal blend layers are
  read back and composited on the CPU (`filter_chain::cpu_blend_composite`);
  correct, but a GPU path would be faster.

## Launch polish

- **Font rendering quality across skins**: kerning, hinting, subpixel
  positioning. Especially on PSP, which uses system TrueType fonts via
  `psp::font`.
- **Accessibility**: ARIA roles are parsed but not exposed to anything.

## Out of scope / non-goals

Documented so we stop relitigating them.

- **V8-level JS performance on PSP.** QuickJS-NG is two orders of
  magnitude slower than V8; that is fine for the target use cases.
- **Service workers, WebRTC, Web Audio API, IndexedDB.** Too much surface
  area for an embedded engine.
- **Full HTML5 frameset support.** The web has moved on.
- **SVG animation (SMIL).**
- **CSS Houdini.**
