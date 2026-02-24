# Browser Rendering Improvement Plan v2

## Current State

After the Phase 1-8 improvements (table/float/positioning wiring, text measurement,
border styles, overflow clipping, `<style>` collection, cascade optimization), the browser
has correct structural layout but still renders poorly for several root-cause reasons:

1. **Anonymous block boxes lose parent styling** -- mixed block/inline children get wrapped
   in anonymous blocks with `ComputedStyle::default()` (8px font, black text, left align),
   discarding the parent's inherited font-size, color, line-height, and text-align.
2. **Ordered list counters always show "1."** -- `resolve_list_marker()` hardcodes
   `ListMarker::Decimal(1)` for every `<li>`.
3. **Inline elements have no visual chrome** -- `<code>`, `<mark>`, `<a>` backgrounds,
   borders, and padding are never painted. `paint_inline_content()` only draws text.
4. **No word-breaking** -- long URLs and unbroken strings overflow containers.
5. **UA stylesheet has no visual styling** for `<blockquote>`, `<pre>`, `<code>`,
   `<mark>`, `<details>`, `<summary>`, `<sub>`, `<sup>`, `<small>` beyond font changes.
6. **No `border-radius` in page CSS** -- only chrome UI uses rounded corners.
7. **No `box-shadow`** -- all elements are flat with no depth cues.
8. **No `opacity`** -- no transparency control for content elements.
9. **`paint_line_box()` is dead code** -- the proper fragment-based painting path exists
   but is marked `#[allow(dead_code)]` and never called.
10. **Baseline approximation** (`line_height * 0.8`) produces misaligned mixed-size text.

---

## Phase 1: Fix Critical Rendering Bugs (HIGH IMPACT, LOW EFFORT)

These are bugs that break basic rendering correctness.

### 1A: Anonymous Block Style Inheritance

**File:** `crates/oasis-browser/src/layout/block.rs`

**Problem:** `make_anonymous_block()` (line ~503) creates anonymous boxes with
`ComputedStyle::default()`, losing the parent element's inherited properties. When
`layout_inline()` reads `parent.style.line_height`, `parent.style.text_align`,
`parent.style.color`, and `parent.style.font_size`, it gets defaults (8px, left, black)
instead of the actual parent's values.

**Fix:** Pass the parent's `ComputedStyle` into `wrap_anonymous()` and
`make_anonymous_block()`, copying inherited properties (color, font-size, font-weight,
font-style, font-family, line-height, text-align, text-decoration, text-transform,
letter-spacing, word-spacing, white-space, visibility, list-style-type) while keeping
display as Block.

```rust
fn make_anonymous_block(children: Vec<LayoutBox>, parent_style: &ComputedStyle) -> LayoutBox {
    let mut style = parent_style.clone();
    style.display = Display::Block;
    // Reset non-inherited properties to defaults.
    style.margin_top = Dimension::Px(0.0);
    style.margin_bottom = Dimension::Px(0.0);
    // ... other non-inherited resets ...
    LayoutBox {
        box_type: BoxType::Anonymous,
        style,
        ..
    }
}
```

Update `wrap_anonymous()` to accept a `&ComputedStyle` parameter, and update its call
sites in `build_box_for_node()` (line ~366) and `build_layout_tree()` (line ~72).

### 1B: Ordered List Counter Numbering

**File:** `crates/oasis-browser/src/layout/block.rs`

**Problem:** `resolve_list_marker()` (line ~406) returns `ListMarker::Decimal(1)` for
every list item. All ordered lists render as "1. 1. 1. 1." instead of "1. 2. 3. 4.".

**Fix:** Move list counter assignment from `resolve_list_marker()` to the parent layout
pass (`layout_block_children`). When laying out children of a `<ul>`/`<ol>`, track a
counter and assign sequential numbers to `ListItem { marker: Decimal(n) }` boxes.

```rust
// In layout_block_children, track list counter:
let mut list_counter: usize = 1;
for child in &mut parent.children {
    if let BoxType::ListItem { ref mut marker } = child.box_type {
        if let ListMarker::Decimal(_) = marker {
            *marker = ListMarker::Decimal(list_counter);
            list_counter += 1;
        }
    }
    // ... existing layout code ...
}
```

Also check if the `<ol>` element has a `start` attribute and initialize the counter
accordingly.

### 1C: Whitespace Between Inline Elements

**File:** `crates/oasis-browser/src/layout/block.rs`

**Problem:** `build_box_for_node()` (line ~372) drops whitespace-only text nodes:
```rust
NodeKind::Text(text) => {
    if text.trim().is_empty() {
        return None;  // <-- drops inter-element whitespace
    }
```

This means `<em>hello</em> <strong>world</strong>` renders as "helloworld" with no space
between the words, because the space text node between `</em>` and `<strong>` is dropped.

**Fix:** Only drop whitespace-only text nodes when they're between block-level siblings.
Keep them when they're between inline siblings (which is the common case). Check if the
node's parent is a block-level element and if the adjacent siblings are also block-level
before dropping.

### Acceptance Criteria
- Text inside `<div>Some text<p>paragraph</p>more text</div>` renders with correct
  parent font-size and color
- Ordered lists render as "1. 2. 3. 4."
- `<em>hello</em> <strong>world</strong>` has a space between "hello" and "world"

---

## Phase 2: Inline Element Visual Styling (HIGH IMPACT, MEDIUM EFFORT)

Inline elements like `<code>`, `<mark>`, `<a>`, `<em>` currently render as plain text
with no visual distinction (no background, no border, no padding).

### 2A: Paint Inline Backgrounds and Borders

**File:** `crates/oasis-browser/src/paint.rs`

**Problem:** `paint_inline_content()` (line ~418) only calls `paint_text()`. It never
paints the inline element's background-color, border, or padding. An inline `<code>` with
`background-color: #f0f0f0; padding: 2px 4px; border-radius: 3px` renders identically
to plain text.

**Fix:** Before painting text in `paint_inline_content()`, check the inline box's style
for non-transparent background and non-zero border, and paint them around the text
content area. For inline elements that span multiple lines, this needs per-fragment
painting (the content rect already covers the fragment's dimensions from `lines_to_children()`).

```rust
fn paint_inline_content(...) -> Result<()> {
    // Paint inline background if non-transparent.
    let bg = layout_box.style.background_color;
    if bg.a > 0 {
        let content = &layout_box.dimensions.content;
        let x = (content.x + offset_x as f32) as i32;
        let y = (content.y - ctx.scroll_y + offset_y as f32) as i32;
        let pad = 2; // inline padding visual adjustment
        backend.fill_rect(
            x - pad, y,
            content.width as u32 + pad as u32 * 2,
            content.height as u32,
            bg,
        )?;
    }

    // Paint text content.
    if let Some(ref text) = layout_box.text { ... }
    ...
}
```

### 2B: Enhance UA Stylesheet for Inline Elements

**File:** `crates/oasis-browser/src/css/cascade.rs`

Add visual defaults to the UA stylesheet for elements that should be visually distinct:

```css
code, kbd, samp {
    font-family: monospace;
    background-color: rgba(128, 128, 128, 30);
    /* Note: padding requires inline padding support (2A) */
}

mark {
    background-color: rgba(255, 255, 0, 128);
    color: #000000;
}

blockquote {
    display: block;
    margin-top: 1em;
    margin-bottom: 1em;
    margin-left: 40px;
    padding-left: 10px;
    border-left-width: 3px;
    border-left-style: solid;
    border-left-color: #808080;
}

pre {
    display: block;
    white-space: pre;
    font-family: monospace;
    margin-top: 1em;
    margin-bottom: 1em;
    padding: 8px;
    background-color: rgba(128, 128, 128, 20);
    border-width: 1px;
    border-style: solid;
    border-color: rgba(128, 128, 128, 40);
}

small { font-size: 0.83em; }
sub { font-size: 0.83em; }
sup { font-size: 0.83em; }

dt { font-weight: bold; }

details { display: block; margin-top: 1em; margin-bottom: 1em; }
summary { display: block; font-weight: bold; }
```

### 2C: Correct `<ol>` Default List Style

**File:** `crates/oasis-browser/src/css/cascade.rs`

Currently both `<ul>` and `<ol>` items get `list-style-type: disc` because the UA
stylesheet only styles `li` generically. Add:

```css
ol > li { list-style-type: decimal; }
ul > li { list-style-type: disc; }
```

Or handle in the cascade by checking parent tag.

### Acceptance Criteria
- `<code>inline code</code>` shows a visible background highlight
- `<mark>highlighted</mark>` shows yellow background
- `<blockquote>` has a visible left border
- `<pre>` has background + border distinguishing it from body text
- `<ol>` uses decimal markers, `<ul>` uses disc markers

---

## Phase 3: Text Overflow & Word Breaking (MEDIUM-HIGH IMPACT, LOW EFFORT)

Long unbroken strings (URLs, code, hashes) overflow their container, producing
horizontally clipped or invisible text.

### 3A: Parse `word-break` and `overflow-wrap` CSS Properties

**File:** `crates/oasis-browser/src/css/values.rs`

Add to `ComputedStyle`:
```rust
pub word_break: WordBreak,       // normal | break-all
pub overflow_wrap: OverflowWrap, // normal | break-word | anywhere
```

**File:** `crates/oasis-browser/src/css/parser.rs`

Parse `word-break` and `overflow-wrap` properties.

### 3B: Emergency Word Breaking in Inline Layout

**File:** `crates/oasis-browser/src/layout/inline.rs`

When a single word fragment is wider than the available line width, break it at the
container edge:

```rust
// In the line-breaking loop:
if !current_line.try_add(fragment) {
    // If the fragment doesn't fit on an *empty* line, break the word.
    if current_line.is_empty() {
        let broken = break_word(fragment, available_width, measurer);
        for piece in broken {
            if !current_line.try_add(&piece) {
                lines.push(current_line);
                current_line = LineBox::new(available_width);
                current_line.try_add(&piece);
            }
        }
        continue;
    }
    // Normal wrap: push current line, start new one.
    lines.push(current_line);
    current_line = LineBox::new(available_width);
    current_line.try_add(fragment);
}
```

The `break_word()` function splits a text fragment character-by-character at the
available width boundary.

### 3C: UA Default for `<pre>` Overflow

Add `overflow-wrap: break-word` to `<pre>` in the UA stylesheet so pre-formatted text
wraps instead of overflowing the viewport.

### Acceptance Criteria
- A 100-character URL in a paragraph wraps at the container edge
- `<pre>` blocks with long lines wrap instead of overflowing
- `word-break: break-all` in author CSS breaks all words at container edge

---

## Phase 4: `border-radius` for Page Content (MEDIUM IMPACT, MEDIUM EFFORT)

Currently `border-radius` only works for the browser chrome (URL bar, buttons). Page
content can't use rounded corners.

### 4A: Parse `border-radius` CSS Property

**File:** `crates/oasis-browser/src/css/values.rs`

Add to `ComputedStyle`:
```rust
pub border_radius: f32, // simplified: single radius for all 4 corners
```

**File:** `crates/oasis-browser/src/css/parser.rs`

Parse `border-radius` property (shorthand for all 4 corners). Initial version: single
value applied to all corners. Future: per-corner values.

### 4B: Paint Rounded Backgrounds and Borders

**File:** `crates/oasis-browser/src/paint.rs`

In `paint_background()` and `paint_borders()`, when `border_radius > 0`, use
`backend.fill_rounded_rect()` and `backend.stroke_rounded_rect()` instead of
`backend.fill_rect()`.

### Acceptance Criteria
- `<div style="border-radius: 8px; background: #eee; padding: 10px">` renders with
  rounded corners
- `<code>` with UA `border-radius: 3px` has subtle rounded corners

---

## Phase 5: `box-shadow` (MEDIUM IMPACT, MEDIUM EFFORT)

Box shadows add visual depth and are used extensively in modern web design for cards,
buttons, and panels.

### 5A: Parse `box-shadow` CSS Property

**File:** `crates/oasis-browser/src/css/values.rs`

Add to `ComputedStyle`:
```rust
pub box_shadow: Option<BoxShadow>,

pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}
```

**File:** `crates/oasis-browser/src/css/parser.rs`

Parse `box-shadow: offset-x offset-y blur spread color` syntax.

### 5B: Paint Box Shadows

**File:** `crates/oasis-browser/src/paint.rs`

In `paint_box()`, before painting the background (step 1), paint the shadow. For the
initial implementation, approximate blur with a series of concentric rectangles at
decreasing opacity (no true Gaussian blur -- that would require a framebuffer which we
don't have in the `SdiBackend` trait).

```rust
fn paint_box_shadow(shadow: &BoxShadow, border_box: &Rect, backend, offset_x, offset_y, ctx) {
    let steps = (shadow.blur as i32).max(1);
    for i in 0..steps {
        let alpha = ((shadow.color.a as f32) * (1.0 - i as f32 / steps as f32)) as u8;
        let expand = shadow.spread + i as f32;
        let color = Color::rgba(shadow.color.r, shadow.color.g, shadow.color.b, alpha);
        backend.fill_rect(
            bx + shadow.offset_x as i32 - expand as i32,
            by + shadow.offset_y as i32 - expand as i32,
            bw + expand as u32 * 2,
            bh + expand as u32 * 2,
            color,
        )?;
    }
}
```

### Acceptance Criteria
- `box-shadow: 2px 2px 4px rgba(0,0,0,0.3)` renders a visible shadow behind the element
- Shadow appears behind element, not on top
- Elements without box-shadow are not affected (no performance regression)

---

## Phase 6: `opacity` Property (LOW-MEDIUM IMPACT, LOW EFFORT)

### 6A: Parse `opacity` CSS Property

**File:** `crates/oasis-browser/src/css/values.rs`

Add to `ComputedStyle`:
```rust
pub opacity: f32, // 0.0 (fully transparent) to 1.0 (fully opaque), default 1.0
```

### 6B: Apply Opacity During Paint

**File:** `crates/oasis-browser/src/paint.rs`

When `opacity < 1.0`, scale the alpha channel of all colors painted for that box (background,
border, text). This is an approximation -- true CSS opacity creates a compositing layer,
but scaling alpha is sufficient for the embedded browser use case.

```rust
fn apply_opacity(color: Color, opacity: f32) -> Color {
    Color::rgba(color.r, color.g, color.b, (color.a as f32 * opacity) as u8)
}
```

### Acceptance Criteria
- `opacity: 0.5` makes element appear semi-transparent
- `opacity: 0` makes element invisible
- `opacity: 1` (default) has no effect

---

## Phase 7: Better Home Page & Demo Content (LOW EFFORT, HIGH USER-VISIBLE IMPACT)

The built-in home page is minimal HTML with no styling. Since this is the first thing
users see, improving it demonstrates the browser's capabilities and provides a better
first impression.

### 7A: Enhance Home Page HTML

**File:** `crates/oasis-app/src/vfs_setup.rs`

Replace the current sparse home page with a styled page that exercises the browser's
features: headings, paragraphs, styled `<code>` blocks, a table, a list, blockquote,
links, and color. Include an inline `<style>` block.

### 7B: Add a CSS Feature Test Page

Add `vfs://sites/home/features.html` that exercises all supported CSS features with
labeled examples (like a browser compatibility test). This serves as both a demo and a
regression test.

### Acceptance Criteria
- Home page looks visually polished with proper headings, styled code, and color
- Feature test page renders all supported CSS properties correctly

---

## Phase 8: Improved Baseline Alignment (LOW-MEDIUM IMPACT, MEDIUM EFFORT)

### 8A: Compute Proper Baseline from Font Metrics

**File:** `crates/oasis-browser/src/layout/inline.rs`

Replace the crude `line.baseline = line.height * 0.8` (line 64) with a proper baseline
computation. The bitmap font's ascent is approximately 75% of the glyph cell height.

```rust
fn compute_baseline(font_size: f32) -> f32 {
    // Bitmap font: ascender ≈ 6/8 of em, descender ≈ 2/8 of em
    font_size * 0.75
}
```

### 8B: Vertical Alignment of Mixed-Size Inline Content

When inline fragments on the same line have different font sizes (e.g., normal text next
to `<small>` or `<sup>`), align them on a shared baseline rather than on the top of the
line box.

### Acceptance Criteria
- Mixed-size text on the same line aligns at baseline
- `<sup>` and `<sub>` elements visually shift above/below baseline

---

## Phase 9: `::before` and `::after` Pseudo-Elements (LOW IMPACT, MEDIUM EFFORT)

Many stylesheets use `::before` and `::after` for decorative content, bullet
customization, and clearfix patterns.

### 9A: Parse Pseudo-Element Selectors

**File:** `crates/oasis-browser/src/css/parser.rs`

Recognize `::before` and `::after` in selectors. Store the pseudo-element type alongside
the selector so the cascade can match them.

### 9B: Parse `content` CSS Property

**File:** `crates/oasis-browser/src/css/values.rs`

Add:
```rust
pub content: Option<String>, // The generated content string
```

Parse `content: "text"`, `content: ""`, `content: none`.

### 9C: Generate Pseudo-Element Boxes

**File:** `crates/oasis-browser/src/layout/block.rs`

In `build_box_for_node()`, after building an element's children, check if the cascade
produced `::before` or `::after` styles with a `content` value. If so, insert an inline
`LayoutBox` at the beginning (::before) or end (::after) of the children list.

### Acceptance Criteria
- `p::before { content: ">> "; color: gray; }` renders ">> " before every paragraph
- `div::after { content: ""; display: block; clear: both; }` works as a clearfix
- `content: none` suppresses pseudo-element generation

---

## Phase 10: `:hover` and `:visited` Pseudo-Classes (LOW IMPACT, MEDIUM EFFORT)

### 10A: Track Mouse Position for `:hover`

**File:** `crates/oasis-browser/src/lib.rs`

Track the mouse cursor position within the browser widget. During paint, record which
DOM elements are under the cursor. On the next style/layout cycle, mark those elements
as hovered.

**File:** `crates/oasis-browser/src/css/cascade.rs`

In selector matching, evaluate `:hover` against the set of hovered node IDs.

### 10B: Track Visited URLs for `:visited`

**File:** `crates/oasis-browser/src/nav.rs`

The `NavigationController` already has a history. Expose the set of visited URLs.

**File:** `crates/oasis-browser/src/css/cascade.rs`

In selector matching, evaluate `:visited` by checking if the `<a>`'s href is in the
visited URL set. Per CSS spec, restrict `:visited` to color properties only (prevents
history sniffing).

### Acceptance Criteria
- `a:hover { color: red; }` changes link color when mouse hovers
- `a:visited { color: purple; }` changes color for previously visited links
- `:visited` only affects color, not layout properties

---

## Implementation Order & Rationale

| Phase | Impact | Effort | Description |
|-------|--------|--------|-------------|
| 1     | **Critical** | Low | Fix 3 bugs that break basic rendering correctness |
| 2     | High | Medium | Inline visual styling -- biggest visual improvement |
| 3     | Med-High | Low | Word breaking prevents text overflow |
| 4     | Medium | Medium | border-radius adds modern visual polish |
| 5     | Medium | Medium | box-shadow adds visual depth |
| 6     | Low-Med | Low | opacity for transparency effects |
| 7     | High (UX) | Low | Better home page as first impression |
| 8     | Low-Med | Medium | Baseline alignment for mixed text |
| 9     | Low | Medium | ::before/::after for CSS patterns |
| 10    | Low | Medium | :hover/:visited for interactivity |

Phases 1-3 should be done first as they fix correctness issues and have the highest
rendering impact for the lowest effort. Phases 4-7 add visual polish. Phases 8-10 are
refinements.

---

## Out of Scope

Explicitly not planned (diminishing returns for an embedded 480x272 browser):

- JavaScript execution
- CSS Grid layout
- CSS animations/transitions/transforms
- Form submission and interactive inputs
- Web fonts / @font-face
- SVG rendering
- `<canvas>` / `<video>` / `<audio>` rendering
- Service Workers / WebSocket
- Multi-tab browsing
- Accessibility tree (ARIA)
- CSS `calc()`, `var()`, custom properties
- `background-image` (would need image-in-CSS pipeline)
