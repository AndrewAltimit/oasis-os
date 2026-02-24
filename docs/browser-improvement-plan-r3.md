# Browser Rendering Quality Improvement Plan (Round 3)

## Comprehensive 10-Phase Plan

**Branch:** `feat/browser-improvements`
**Goal:** Make the OASIS browser engine render real-world pages (Google, Wikipedia, news sites, documentation) correctly and legibly.

---

## Current State Assessment

### What Works Well
- HTML5 parser with 50+ tags, entity decoding, tree construction
- CSS cascade with specificity, inheritance, 60+ properties, CSS variables
- Block, inline, table, flexbox, float, and positioned layout
- Text alignment (left/center/right/justify), text-decoration, text-indent
- Border rendering (solid/dashed/dotted/double), border-radius, box-shadow
- Image decoding (BMP/PNG/JPEG), progressive loading, memory budget
- Overflow:hidden clipping, opacity, z-index stacking
- Auto-margin centering, box-sizing, min/max dimensions
- Scroll support, link navigation, reader mode

### Key Rendering Problems Identified

From screenshots of google.html and wikipedia.html:

1. **No visual bold/italic** -- `font-weight: bold` and `font-style: italic` are parsed but never rendered. All text looks the same weight.
2. **Non-ASCII characters render as `?`** -- Cyrillic, CJK, accented chars, Unicode symbols all show as question marks (Wikipedia's "Русский" → "???????").
3. **Font size scaling is blocky** -- The 8x8 bitmap font scales by integer multiples only (8, 16, 24, 32...). A CSS `font-size: 13px` rounds to 8px, `font-size: 20px` rounds to 16px. This means most real-world font sizes look wrong.
4. **No `line-height` CSS rendering** -- Line height is computed but text doesn't actually space lines correctly for different line-height values (e.g., `line-height: 27px` on the Google navbar).
5. **`float: right` text positioning** -- The "Sign in" link on Google's navbar uses `float: right` but the `<span>` container is likely not positioned correctly.
6. **Missing `::placeholder` pseudo-element** -- Search inputs show empty boxes instead of placeholder text.
7. **No `cursor` CSS property visual** -- No cursor shape changes (not critical for PSP but matters for desktop).
8. **Limited `background-image`** -- No gradient support, no `url()` image backgrounds.
9. **No `text-transform` in rendering** -- `text-transform: uppercase/lowercase/capitalize` is computed but may not be applied during painting.
10. **Margin collapsing edge cases** -- Parent-child margin collapsing is incomplete.
11. **No `max-width` on root content** -- Pages that rely on `max-width` for centered layouts may overflow.
12. **Form element rendering is basic** -- Inputs are rendered as simple rectangles, buttons don't look like buttons.

---

## Phase 1: Bold & Italic Text Rendering

**Impact: HIGH** -- Affects every page with `<b>`, `<strong>`, `<em>`, `<i>`, `<h1>`-`<h6>`

### Problem
`font_weight` and `font_style` are stored in `ComputedStyle` but never passed to the paint layer. The `draw_text()` backend API takes only `(text, x, y, font_size, color)` with no weight/style parameter. All text renders at normal weight.

### Solution

**1a. Faux-bold via double-strike rendering**
Since we use a bitmap font, true font weight isn't possible. Instead, render bold text by drawing each glyph twice with a 1px horizontal offset (a classic CGA/EGA technique):
- In `paint.rs` `paint_text_run()`: check `style.font_weight == Bold`
- If bold: call `draw_text()` at (x, y) AND (x+1, y) -- this thickens each stroke
- For large scales (font-size >= 16): offset by `scale` pixels instead of 1

**1b. Faux-italic via skew rendering**
For italic, shift each row of the glyph by a fraction:
- Add a `draw_text_italic()` method to `SdiBackend` (or an `italic: bool` param)
- In the SDL backend: for each glyph row, apply a 2px rightward shift for the top rows that decreases toward the bottom (creates ~12-degree slant)
- Alternative simpler approach: draw at (x + row_offset, y) where row_offset = (7 - row) / 3

**1c. Pass font style through paint pipeline**
- `paint_text_run()` already receives `&ComputedStyle` -- extract `font_weight` and `font_style`
- Add `draw_text_styled()` to `SdiBackend` trait with `bold: bool, italic: bool` params
- Update SDL, UE5, and PSP backends

### Files to modify
- `crates/oasis-browser/src/paint.rs` -- paint_text_run
- `crates/oasis-types/src/backend.rs` -- SdiBackend trait
- `crates/oasis-backend-sdl/src/lib.rs` -- draw_text implementation
- `crates/oasis-backend-ue5/src/lib.rs` -- draw_text implementation

### Test fixtures
- `test-fixtures/html/basic_text.html` already has bold/italic -- screenshot should show visible difference

---

## Phase 2: Extended Character Set (Latin Extended + Common Symbols)

**Impact: HIGH** -- Fixes "???????" rendering for accented characters, copyright symbol, em-dash, etc.

### Problem
The bitmap font covers ASCII 0x20-0x7E only (95 glyphs). Any character outside this range (accented Latin, Cyrillic, CJK, Unicode symbols) renders as '?'. This breaks Wikipedia's language section, copyright symbols, em-dashes, smart quotes, etc.

### Solution

**2a. Latin-1 Supplement (U+00A0-U+00FF) -- 96 glyphs**
Add glyphs for the most common non-ASCII characters:
- `©` (copyright), `®` (registered), `°` (degree), `±` (plus-minus)
- `À-Ö`, `Ø-ö`, `ø-ÿ` (accented Latin -- French, German, Spanish, Portuguese, etc.)
- `«»` (guillemets), `¡¿` (inverted punctuation)
- `×÷` (math operators), `µ` (micro)

**2b. Common Unicode symbols (cherry-picked)**
Add the most-used Unicode code points that appear on real web pages:
- `—` (em-dash U+2014), `–` (en-dash U+2013)
- `'` `'` `"` `"` (smart quotes U+2018-201D)
- `…` (ellipsis U+2026)
- `•` (bullet U+2022), `◦` (white bullet U+25E6), `▪` (square U+25AA)
- `←→↑↓` (arrows U+2190-2193)
- `€` (euro U+20AC), `£` (pound U+00A3), `¥` (yen U+00A5)
- `≤≥≠≈` (math relations)
- `✓✗` (check/cross marks)

**2c. Cyrillic basic (U+0410-U+044F) -- 64 glyphs**
Common Cyrillic letters for Russian text rendering.

**2d. Update fallback handling**
Instead of replacing unknown chars with `?`, use a Unicode tofu box `□` (U+25A1) that makes it clear a glyph is missing rather than appearing as a question mark.

### Files to modify
- `crates/oasis-types/src/bitmap_font.rs` -- extend FONT_DATA table, update glyph() lookup
- `crates/oasis-backend-sdl/src/font.rs` -- glyph table
- `crates/oasis-browser/src/layout/text.rs` -- remove '?' fallback for known ranges

### Scope
~200 new 8x8 glyph bitmaps. Each is 8 bytes, so total data growth is ~1.6KB.

---

## Phase 3: Font Size Interpolation & Line Height

**Impact: HIGH** -- Fixes text being wrong size on almost every page

### Problem
The bitmap font is 8x8 and scales only by integer multiples: 8px, 16px, 24px, 32px, etc. Real CSS uses arbitrary sizes like 11px, 13px, 14px, 18px, 20px, 48px. Currently:
- 11px → renders as 8px (scale=1)
- 13px → renders as 8px (scale=1)
- 14px → renders as 8px (scale=1)
- 20px → renders as 16px (scale=2)
- 48px → renders as 48px (scale=6) -- correct by luck

This makes small text all the same size and medium text has jarring size jumps.

### Solution

**3a. Sub-pixel scaling for text rendering**
Instead of `scale = font_size / 8` (integer division), use floating-point:
- Pre-render each glyph to a small texture at the exact pixel size needed
- Or: use nearest-neighbor integer scaling but with the correct pixel count:
  - 13px font: render each glyph into a 13x13 box by scaling the 8x8 grid
  - Use bresenham-style scaling: for each output row, pick the nearest source row

**3b. Glyph cache**
Cache rendered glyphs as textures indexed by (char, font_size):
- Avoids re-rendering for repeated characters
- LRU eviction when cache exceeds memory budget
- Pre-populate for the 2-3 most common sizes on page load

**3c. Line height fix**
Ensure `line-height` CSS property actually controls vertical spacing between lines:
- In `layout/inline.rs`: line height should be the max of computed `line-height` values for all fragments on the line
- When line-height > font-size, center text vertically within the line box
- This fixes the Google navbar where `line-height: 27px` should make the nav links vertically centered

**3d. Text measurement accuracy**
Update `bitmap_measure_text()` to use the same floating-point scaling so that measured width matches rendered width exactly.

### Files to modify
- `crates/oasis-backend-sdl/src/lib.rs` -- draw_text with fractional scaling
- `crates/oasis-types/src/backend.rs` -- bitmap_measure_text
- `crates/oasis-browser/src/layout/inline.rs` -- line height computation
- `crates/oasis-browser/src/layout/text.rs` -- text measurement

---

## Phase 4: Form Element Rendering

**Impact: MEDIUM** -- Google and Wikipedia both have search inputs and buttons

### Problem
Form elements (`<input>`, `<button>`, `<select>`) are rendered as flat rectangles with minimal styling. They don't look like interactive form elements.

### Solution

**4a. Text input rendering**
- Draw with inset border (light bottom-right, dark top-left) for 3D appearance
- Show `placeholder` attribute text in gray when value is empty
- Show `value` attribute text in the input
- Respect `width`, `height`, `padding`, `border`, `border-radius`, `background-color`, `color`, `font-size` CSS

**4b. Button rendering**
- Draw with raised 3D border appearance (dark bottom-right, light top-left)
- Center the `value` text within the button
- Respect `background-color`, `color`, `padding`, `border`, `border-radius`
- Add subtle gradient or shading for depth

**4c. Select/dropdown rendering**
- Draw as a rectangular box with down-arrow indicator
- Show selected option text

**4d. Textarea rendering**
- Multi-line text display area with scroll indicator

### Files to modify
- `crates/oasis-browser/src/paint.rs` -- form element paint functions
- `crates/oasis-browser/src/layout/inline.rs` -- replaced element dimension computation
- `crates/oasis-browser/src/layout/box_model.rs` -- form element intrinsic sizes
- `crates/oasis-browser/src/css/cascade.rs` -- default user-agent styles for form elements

---

## Phase 5: Margin Collapsing & Block Layout Correctness

**Impact: MEDIUM** -- Fixes vertical spacing between elements

### Problem
Vertical margin collapsing between adjacent siblings works but parent-child collapsing does not. This causes incorrect spacing in many layouts:
- A `<div>` with a `<p>` inside should collapse the `<p>`'s top margin with the `<div>`'s top margin
- Empty blocks should collapse their top and bottom margins
- Margins through empty blocks should collapse

### Solution

**5a. Parent-child margin collapsing**
Per CSS 2.1 §8.3.1:
- If a parent has no top border, padding, or inline content, its top margin collapses with the first child's top margin
- Similarly for bottom margins with the last child
- Implement by passing the first/last child margin through to the parent

**5b. Empty block collapsing**
- An element with no height, padding, or border collapses its own top and bottom margins into a single margin

**5c. Collapsing inhibition**
Margins don't collapse through:
- Elements with `overflow` != `visible`
- Floated elements
- Absolutely positioned elements
- Inline-block elements
- Elements with border or padding

### Files to modify
- `crates/oasis-browser/src/layout/block.rs` -- margin collapsing logic

---

## Phase 6: Background & Visual Effects

**Impact: MEDIUM** -- Improves visual quality significantly

### Problem
Background rendering is limited to solid colors. Many pages use gradients, background images, and multiple backgrounds for visual design.

### Solution

**6a. Linear gradients**
Parse `linear-gradient()` in CSS:
- Support `to top/right/bottom/left`, angle values
- Support color stops with positions
- Render by filling each horizontal scanline with interpolated color
- For the browser viewport size, this is inexpensive

**6b. Background-image: url()**
- Resolve URL against base URL
- Load image via the image pipeline
- Tile or position according to `background-repeat`, `background-position`, `background-size`
- Initially support `no-repeat` and `cover`/`contain`

**6c. Text shadow**
- Parse `text-shadow` CSS property
- Render by drawing text twice: once with shadow offset/color, then the actual text on top

**6d. Multiple borders per element**
- Ensure each border edge (top/right/bottom/left) can have independent width, style, and color
- Currently partially working -- verify full correctness

### Files to modify
- `crates/oasis-browser/src/css/parser.rs` -- gradient parsing
- `crates/oasis-browser/src/css/values.rs` -- gradient value types
- `crates/oasis-browser/src/paint.rs` -- gradient/shadow rendering

---

## Phase 7: Improved Table Layout

**Impact: MEDIUM** -- Tables are common on Wikipedia, documentation, data pages

### Problem
Table layout works for simple cases but has issues:
- `border-collapse: collapse` needs fully merged borders between adjacent cells
- `colspan` width distribution could be more accurate
- Table `width: 100%` should fill the containing block
- Vertical alignment within cells (especially `vertical-align: middle`)
- `<caption>` positioning
- `<thead>`/`<tfoot>` should render at fixed positions during scroll (future)

### Solution

**7a. Border-collapse rendering**
- When `border-collapse: collapse`, draw a single border between adjacent cells
- Use the CSS border conflict resolution algorithm (wider wins, then style priority)

**7b. Table width: 100% and auto**
- `width: 100%` tables should fill the containing block
- Auto-width tables should shrink-wrap to content

**7c. Cell vertical alignment**
- `vertical-align: top/middle/bottom/baseline` within table cells
- Currently cells are top-aligned by default -- add middle/bottom support

**7d. Caption and col/colgroup**
- Render `<caption>` above/below the table
- Apply `<col>` width attributes to columns

### Files to modify
- `crates/oasis-browser/src/layout/table.rs` -- all table layout improvements
- `crates/oasis-browser/src/paint.rs` -- collapsed border rendering

---

## Phase 8: CSS Shorthand Expansion & Missing Properties

**Impact: MEDIUM** -- Many real-world pages use shorthand CSS

### Problem
While many shorthands are expanded, there are gaps and edge cases:
- `font` shorthand (e.g., `font: bold 14px/1.5 sans-serif`)
- `background` shorthand with multiple values
- `border` shorthand with all three components
- `flex` shorthand (e.g., `flex: 1 0 auto`)
- `list-style` shorthand
- Missing properties: `word-break`, `overflow-wrap`, `text-overflow: ellipsis`

### Solution

**8a. `font` shorthand parsing**
Parse `font: [style] [weight] size[/line-height] family`:
- `font: bold 14px/1.5 sans-serif` → font-weight: bold, font-size: 14px, line-height: 1.5, font-family: sans-serif

**8b. `text-overflow: ellipsis`**
When text overflows a container with `overflow: hidden`:
- Truncate the last visible text fragment
- Append "..." at the truncation point
- Critical for nav bars and constrained layouts

**8c. `word-break` and `overflow-wrap`**
- `word-break: break-all` -- allow breaking within any word
- `overflow-wrap: break-word` -- break words that overflow their container
- Currently emergency breaking exists but isn't CSS-controlled

**8d. `white-space: nowrap` with overflow**
Ensure `nowrap` text that overflows is properly clipped by `overflow: hidden`.

### Files to modify
- `crates/oasis-browser/src/css/parser.rs` -- shorthand expansion
- `crates/oasis-browser/src/css/values.rs` -- new property fields
- `crates/oasis-browser/src/layout/inline.rs` -- text overflow handling

---

## Phase 9: User-Agent Stylesheet & Default Styles

**Impact: HIGH** -- Fixes baseline rendering for every page

### Problem
The browser needs a proper user-agent (UA) stylesheet that provides default styling for all HTML elements. Currently, defaults are scattered as hardcoded values in the cascade module. A proper UA stylesheet ensures:
- `<h1>` through `<h6>` have correct default sizes and margins
- `<p>`, `<blockquote>`, `<pre>` have correct margins
- Lists have proper indentation and markers
- Tables have sensible defaults
- Form elements look appropriate
- `<a>` has blue color and underline
- `<b>`, `<strong>` default to bold; `<em>`, `<i>` to italic
- `<code>`, `<pre>` use monospace
- `<hr>` has proper default appearance
- Semantic elements (`<nav>`, `<header>`, `<footer>`, `<article>`, `<section>`) are display:block

### Solution

**9a. Implement UA stylesheet as actual CSS**
Create a built-in CSS string that's prepended to every page's style cascade with lowest specificity:
```css
html, body { display: block; margin: 0; }
h1 { font-size: 32px; font-weight: bold; margin: 21px 0; }
h2 { font-size: 24px; font-weight: bold; margin: 19px 0; }
/* ... etc for all elements ... */
```

**9b. Replace hardcoded defaults**
Remove scattered default values from `cascade.rs` and `box_model.rs` in favor of the UA stylesheet rules.

**9c. Add `!important` handling for UA defaults**
Ensure UA stylesheet has lower priority than author styles but provides correct inheritance.

### Files to modify
- New: `crates/oasis-browser/src/css/ua_stylesheet.rs`
- `crates/oasis-browser/src/css/cascade.rs` -- integrate UA stylesheet
- `crates/oasis-browser/src/css/values.rs` -- default value cleanup

---

## Phase 10: Full-Page Screenshot & Visual Regression Infrastructure

**Impact: MEDIUM** -- Essential for validating all the above improvements

### Problem
Current screenshots only capture the 480x272 viewport. Pages that scroll (Wikipedia, long documents) can't be fully validated. Also, there's no automated way to detect rendering regressions.

### Solution

**10a. Full-page scroll screenshot**
Add a `--full-page` flag to `screenshot-tests`:
- After layout, read `content_height` from the layout tree
- Create a framebuffer sized `(viewport_width, content_height)`
- Render the page at `scroll_y = 0` into the full-height buffer
- Save as a tall PNG

**10b. Real-world test fixtures**
Create simplified HTML fixtures that represent real-world pages:
- A Wikipedia article page (with sidebar, infobox table, headings, paragraphs, lists, references)
- A news article (headline, byline, image, multi-column text)
- A documentation page (nav sidebar, code blocks, tables, anchored headings)
- A forum/discussion page (nested comments, avatars, timestamps)

**10c. Pixel-diff regression testing**
Extend `--check` mode:
- Compare actual vs golden screenshots using pixel-diff
- Report percentage of pixels that differ
- Threshold for pass/fail (e.g., <0.1% difference)
- Generate diff images highlighting changed pixels

**10d. HTML report with before/after**
Extend `--report` mode:
- Side-by-side comparison of golden vs actual
- Diff overlay highlighting changes
- Filter by pass/fail status

### Files to modify
- `crates/oasis-app/src/screenshot_tests.rs` -- full-page mode, pixel diff
- New test fixtures in `test-fixtures/html/`

---

## Implementation Priority & Dependencies

```
Phase 9 (UA Stylesheet) ─────────────────────── can start immediately
Phase 1 (Bold/Italic)   ─────────────────────── can start immediately
Phase 2 (Extended Chars) ─────────────────────── can start immediately
Phase 3 (Font Scaling)   ─── depends on Phase 1 (bold rendering affects scaling)
Phase 4 (Form Elements)  ─── depends on Phase 9 (UA defaults for forms)
Phase 5 (Margin Collapse)─────────────────────── can start immediately
Phase 6 (Backgrounds)    ─────────────────────── can start immediately
Phase 7 (Table Layout)   ─────────────────────── can start immediately
Phase 8 (CSS Shorthands) ─────────────────────── can start immediately
Phase 10 (Screenshots)   ─── should be done FIRST or in parallel to validate
```

### Recommended Order
1. **Phase 10** (screenshots first -- validates everything)
2. **Phase 9** (UA stylesheet -- fixes baseline rendering)
3. **Phase 1** (bold/italic -- most visible improvement)
4. **Phase 2** (extended chars -- fixes Wikipedia etc.)
5. **Phase 3** (font scaling -- fixes text sizes)
6. **Phase 5** (margin collapsing -- fixes spacing)
7. **Phase 4** (form elements -- improves Google etc.)
8. **Phase 8** (CSS shorthands -- improves compatibility)
9. **Phase 6** (backgrounds/gradients -- visual polish)
10. **Phase 7** (table improvements -- data page support)

---

## Success Criteria

After all 10 phases:
- **Google homepage**: Logo colors correct, search box with rounded border visible, buttons rendered as buttons, nav bar with proper spacing, "Sign in" floated right, footer centered with correct font sizes
- **Wikipedia homepage**: All text visible (including accented characters), headings bold, search input with button styled, language links with correct font sizes, footer legible
- **Generic article page**: Headings visually distinct from body, paragraphs properly spaced, links blue and underlined, images positioned correctly, lists indented with markers, tables with borders and aligned content
- **Full-page scrolling**: Content below the fold renders correctly, page doesn't cut off
