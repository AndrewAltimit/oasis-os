# Browser Rendering Improvement Plan

## Current State

The browser crate (`oasis-browser`) is ~28k LOC with hand-rolled HTML/CSS/layout engines.
It has solid foundations: WHATWG-based tokenizer/tree builder, CSS cascade with specificity,
block/inline/flex layout, and arena-based DOM. However, rendering quality is poor because
three major layout modules (table, float, positioning) are implemented but **not wired in**,
and several CSS properties are parsed but silently ignored during paint.

This plan prioritizes fixes by real-world rendering impact: most web pages rely on tables,
floats, and positioned elements, so wiring those in comes first. Each phase is independently
shippable and testable.

---

## Phase 1: Wire In Table Layout (HIGH IMPACT)

**Goal**: Tables render with proper column widths and row heights instead of as flat block stacks.

Tables are everywhere on the web (data tables, legacy layouts, email HTML). Currently
`layout/table.rs` (1,217 LOC) is complete but marked `#[allow(dead_code)]` and never called.

### Tasks

1. **Remove `#[allow(dead_code)]` from `layout/table.rs`** and fix any resulting compilation issues
2. **Wire `layout_table()` into `build_layout_tree()`** in `layout/block.rs`:
   - When a node has `display: table`, delegate to `layout_table()` instead of block layout
   - Route `display: table-row` and `display: table-cell` children through table layout
3. **Wire table painting into `paint.rs`**:
   - Render cell backgrounds, borders (respect `border-collapse`)
   - Render border-spacing gaps
4. **Handle implicit table elements**:
   - Bare `<td>` without `<tr>` → wrap in anonymous table-row
   - Bare `<tr>` without `<table>` → wrap in anonymous table
5. **Tests**:
   - Simple 2x2 table with text content
   - Table with colspan/rowspan
   - Border-collapse vs border-separate
   - Nested tables
   - Table with percentage widths

### Acceptance Criteria
- `<table>` renders with visible columns and rows
- `border-collapse: collapse` merges adjacent borders
- `colspan`/`rowspan` spans cells correctly

---

## Phase 2: Wire In Float Layout (HIGH IMPACT)

**Goal**: Floated elements position correctly and inline content wraps around them.

Floats are used on the majority of pre-flexbox websites for multi-column layouts, image
positioning, and pull-quotes. `layout/float.rs` (485 LOC) is complete but dead code.

### Tasks

1. **Remove `#[allow(dead_code)]` from `layout/float.rs`** and fix compilation issues
2. **Integrate `FloatContext` into block layout**:
   - Create `FloatContext` at block formatting context boundaries
   - When a child has `float: left|right`, call `place_float()` instead of normal flow
   - Pass `FloatContext` to `layout_inline()` for available-width queries
3. **Adjust inline layout for float avoidance**:
   - In `layout/inline.rs`, query `FloatContext::available_width()` for each line
   - Shift line box start position to avoid left floats
   - Reduce line box end position to avoid right floats
4. **Implement `clear` property**:
   - When a block has `clear: left|right|both`, advance y-position past cleared floats
5. **Handle float-containing blocks**:
   - Block height should encompass floated children (clearfix behavior)
   - BFC roots contain their floats
6. **Tests**:
   - Float left with text wrapping
   - Float right with text wrapping
   - Adjacent left floats (horizontal stacking)
   - Clear after floats
   - Float within a containing block (height containment)
   - Classic two-column float layout

### Acceptance Criteria
- `float: left` moves element to left edge, text wraps on right
- `float: right` mirrors the behavior
- `clear: both` drops below all floats
- Floated images display inline with wrapping text

---

## Phase 3: Wire In Positioned Layout (MEDIUM-HIGH IMPACT)

**Goal**: `position: relative/absolute/fixed` elements render at correct positions.

Positioned elements are used for dropdowns, tooltips, overlays, sticky headers, and many
layout patterns. `layout/positioning.rs` (833 LOC) is complete but dead code.

### Tasks

1. **Remove `#[allow(dead_code)]` from `layout/positioning.rs`** and fix compilation issues
2. **Integrate relative positioning**:
   - After normal-flow layout, apply `top/left/right/bottom` offsets to `position: relative` boxes
   - Relative positioning does not affect subsequent siblings (offset only)
3. **Integrate absolute positioning**:
   - Remove `position: absolute` elements from normal flow during tree building
   - After parent layout completes, resolve `top/left/right/bottom` against containing block
   - Containing block = nearest `position: relative|absolute|fixed` ancestor (or viewport)
4. **Integrate fixed positioning**:
   - Like absolute, but containing block is always the viewport
   - Fixed elements don't scroll (offset by scroll_y during paint)
5. **Z-index stacking in paint**:
   - Collect positioned elements into stacking contexts
   - Paint in z-index order: negative z-index → normal flow → positive z-index
6. **Tests**:
   - `position: relative` with top/left offset
   - `position: absolute` within a relative container
   - `position: fixed` stays on screen during scroll
   - Z-index ordering of overlapping elements
   - Absolute element sized to containing block with `left: 0; right: 0`

### Acceptance Criteria
- `position: relative` offsets elements without disrupting flow
- `position: absolute` positions against nearest positioned ancestor
- `position: fixed` stays visible during scrolling
- Z-index controls paint order of overlapping elements

---

## Phase 4: Improve Inline Layout & Text Rendering (MEDIUM IMPACT)

**Goal**: Inline content (text, inline-block, images) renders with correct baselines,
vertical alignment, and proper line-height handling.

### Tasks

1. **Fix baseline alignment**:
   - Current baseline approximation (`line_height * 0.8`) is inaccurate
   - Derive baseline from actual font metrics (ascent from `glyph_advance` data)
   - Align inline fragments on shared baseline within each line box
2. **Implement `vertical-align`** (at least: baseline, top, middle, bottom, sub, super):
   - Adjust fragment y-offset within line box per vertical-align value
3. **Improve inline-block sizing**:
   - Inline-block elements should use their content dimensions as atomic inline units
   - Respect explicit width/height on inline-block
4. **Fix inline margin/padding/border rendering**:
   - Currently inline elements don't render their own padding/border
   - Add inline-level background + border painting (split across line breaks)
5. **Improve text-decoration rendering**:
   - Underline: draw 1px line at baseline + 1
   - Line-through: draw 1px line at midpoint
   - Overline: draw 1px line at top
6. **Implement `word-break: break-all`**:
   - Allow breaking within words when no space-break point fits
   - Prevents overflow of long URLs/strings
7. **Tests**:
   - Mixed inline elements (text + images + inline-blocks) alignment
   - Underline/line-through rendering verification
   - Long word breaking with `word-break: break-all`
   - Inline padding/border across line break

### Acceptance Criteria
- Inline images align on text baseline by default
- `vertical-align: middle` centers inline content
- Underline decoration renders at correct position
- Long unbroken strings don't overflow their container

---

## Phase 5: Box Model & Visual Polish (MEDIUM IMPACT)

**Goal**: Backgrounds, borders, rounded corners, and the box model render accurately.

### Tasks

1. **Implement `border-radius`** (rounded corners):
   - Parse `border-radius` shorthand and individual corners
   - Render rounded rectangles in paint (quarter-circle approximation at each corner)
   - Clip content to rounded border
2. **Implement `background-image: url()`**:
   - Load image via resource loader
   - Tile or position per `background-repeat`, `background-position`, `background-size`
   - At minimum: cover, contain, no-repeat with position
3. **Implement `opacity`**:
   - Alpha-blend the element's paint output
   - Creates a stacking context
4. **Implement `overflow: hidden` clipping**:
   - Clip children that extend beyond the content box
   - This is critical for many layouts (image containers, card components)
5. **Implement `box-shadow`**:
   - Parse `box-shadow: offset-x offset-y blur spread color`
   - Render shadow behind element (solid approximation for blur)
6. **Improve border rendering**:
   - Render `dashed` borders (alternating filled/empty segments)
   - Render `dotted` borders (square dots along edge)
   - Render `double` borders (two parallel lines)
7. **Tests**:
   - Border-radius on div
   - Overflow hidden clipping children
   - Box-shadow rendering
   - Dashed/dotted border visual checks

### Acceptance Criteria
- Rounded corners visible on elements with `border-radius`
- `overflow: hidden` clips child content
- Dashed and dotted borders visually distinct from solid

---

## Phase 6: Improve CSS Cascade & Selectors (LOW-MEDIUM IMPACT)

**Goal**: Better CSS rule matching for real-world stylesheets.

### Tasks

1. **Implement `::before` and `::after` pseudo-elements**:
   - Generate anonymous inline boxes with `content` property value
   - Support `content: ""`, `content: "text"`, `content: attr(...)`, `content: counter(...)`
   - Insert before/after element's children in layout tree
2. **Implement `@media` queries** (basic):
   - `@media screen`, `@media (max-width: Xpx)`, `@media (min-width: Xpx)`
   - Evaluate against viewport dimensions (480x272 or configurable)
3. **Implement CSS `inherit` and `initial` keywords**:
   - `inherit`: explicitly copy parent's computed value
   - `initial`: reset to CSS spec initial value
4. **Implement `:visited` pseudo-class**:
   - Track visited URLs in navigation history
   - Match `:visited` selector for visited links
   - Restrict to color properties only (privacy)
5. **Tests**:
   - `::before` with content string
   - `@media (max-width: 480px)` matches PSP viewport
   - `inherit` overrides non-inherited property

### Acceptance Criteria
- `::before`/`::after` content renders inline
- Media queries adapt styles for viewport size
- `inherit` keyword works on any property

---

## Phase 7: HTML Parsing Robustness (LOW IMPACT, HIGH QUALITY)

**Goal**: Handle more real-world HTML patterns without breaking layout.

### Tasks

1. **Improve auto-close heuristics**:
   - `<p>` auto-closes at block-level start tags (per WHATWG)
   - `<li>` auto-closes at next `<li>`
   - `<dt>`/`<dd>` auto-close at next `<dt>`/`<dd>`
   - `<option>` auto-closes at next `<option>`
2. **Handle `<style>` in `<body>`**:
   - Currently only `<head>` stylesheets are processed
   - Collect and apply `<style>` blocks found in `<body>`
3. **Improve entity handling**:
   - Add remaining HTML5 named character references (2,000+ entities)
   - Handle malformed entity references gracefully
4. **Handle `<meta charset>` and encoding**:
   - Detect charset from `<meta>` tag
   - Convert non-UTF-8 content (at minimum: Latin-1/ISO-8859-1)
5. **Tests**:
   - Unclosed `<p>` followed by `<div>`
   - `<style>` block in body
   - Malformed entity references

### Acceptance Criteria
- Unclosed tags auto-close per WHATWG rules
- Inline `<style>` blocks in body are applied
- Non-UTF-8 pages display without mojibake

---

## Phase 8: Performance & Incremental Rendering (LOW IMPACT)

**Goal**: Improve rendering speed and responsiveness for complex pages.

### Tasks

1. **Selector indexing**:
   - Build hash maps for ID selectors and class selectors
   - Skip rules that can't possibly match (fast reject)
2. **Layout caching**:
   - Cache computed dimensions for unchanged subtrees
   - Only relayout dirty nodes (extend existing incremental system)
3. **Viewport-based rendering**:
   - Skip layout for elements known to be far off-screen
   - Render visible area first, then extend
4. **Image lazy-loading**:
   - Only decode/load images near the viewport
   - Show placeholder dimensions while loading
5. **Tests**:
   - Performance benchmark: layout of 1000-element page
   - Incremental relayout of single modified node

### Acceptance Criteria
- Selector matching is measurably faster on pages with 100+ rules
- Scrolling through a long page doesn't relayout the entire DOM

---

## Implementation Order Rationale

| Phase | Impact | Effort | Rationale |
|-------|--------|--------|-----------|
| 1. Tables | High | Medium | Code exists, needs wiring. Tables appear on most pages. |
| 2. Floats | High | Medium | Code exists, needs wiring. Classic layouts depend on floats. |
| 3. Positioning | Med-High | Medium | Code exists, needs wiring. Headers, dropdowns, modals. |
| 4. Inline/Text | Medium | Medium | Fixes visual quality of body text (most visible element). |
| 5. Visual Polish | Medium | High | border-radius, shadows, opacity elevate visual quality. |
| 6. CSS Cascade | Low-Med | Medium | ::before/::after and @media used by modern sites. |
| 7. HTML Robustness | Low | Low | Prevents broken layouts from malformed HTML. |
| 8. Performance | Low | Medium | Only matters once rendering is correct. |

Phases 1-3 are pure wiring of existing code and will produce the largest rendering improvement
for the least effort. Phases 4-5 fix visual quality. Phases 6-8 are polish and robustness.

---

## Out of Scope

These are explicitly not planned (diminishing returns for an embedded browser):

- JavaScript execution
- CSS animations/transitions
- CSS Grid layout
- Form submission
- Web fonts
- SVG rendering
- WebSocket/Service Workers
- Accessibility tree (ARIA)
- Print stylesheets
