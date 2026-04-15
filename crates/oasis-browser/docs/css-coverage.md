# CSS Coverage

State of CSS property support in `oasis-browser`. Update this file whenever
properties are added or removed in `src/css/values/apply.rs`.

**As of 2026-04-12: ~120 properties parsed and stored on `ComputedStyle`.**

The cascade pipeline lives in `src/css/`:

- `parser/` — tokenizer + selector / declaration parser, `PropertyId` enum (fast-path interning for the ~75 hottest properties).
- `cascade/` — specificity sorting, `@media` / `@supports`, `var()` / `calc()` resolution.
- `shorthand/` — `background`, `border`, `box`, `flex`, `font`, `gradient`, `list` shorthand expanders.
- `values/types.rs` — all CSS value enums and structs (`Display`, `BorderStyle`, `BlendMode`, …).
- `values/computed.rs` — `ComputedStyle` (final per-element state) + `Default` + `inherit()`.
- `values/apply.rs` — the giant `apply_declaration` match (~280 arms) that resolves a parsed `CssValue` into the right `ComputedStyle` field.
- `values/resolve.rs` — `as_keyword`, `resolve_length`, `resolve_dimension`, `resolve_color`, `resolve_color_or_current`, `resolve_font_size`, etc.

## Supported properties

### Box model
`display` · `visibility` · `box-sizing` · `width` · `height` · `min-width` · `min-height` · `max-width` · `max-height` · `aspect-ratio` · `margin` (+ four sides) · `padding` (+ four sides) · `border` (width / color / style, + four sides) · `border-radius` (+ four corners) · `outline` (width / color / style / offset) · `inset` (shorthand for top/right/bottom/left)

### Positioning & overflow
`position` · `top` · `right` · `bottom` · `left` · `z-index` · `float` · `clear` · `overflow` / `overflow-x` / `overflow-y` · `clip-path` · `resize`

### Text
`color` · `font-size` · `font-weight` · `font-style` · `font-family` · `font-variant` · `font-stretch` · `font-kerning` · `font-feature-settings` · `line-height` · `letter-spacing` · `word-spacing` · `text-align` · `text-align-last` · `text-justify` · `text-indent` · `text-transform` · `text-decoration` (+ `-line` / `-color` / `-style` / `-thickness`) · `text-underline-offset` · `text-underline-position` · `text-shadow` · `text-overflow` · `text-rendering` · `white-space` · `word-break` · `overflow-wrap` (`word-wrap`) · `hyphens` · `tab-size` · `direction` · `vertical-align` · `line-clamp` (`-webkit-line-clamp`)

### Backgrounds & images
`background-color` · `background-image` (url + linear/radial gradients) · `background-size` · `background-position` · `background-repeat` · `background-clip` · `background-origin` · `background-blend-mode` · `object-fit` · `object-position` · `image-rendering`

### Flexbox
`flex` · `flex-direction` · `flex-wrap` · `flex-grow` · `flex-shrink` · `flex-basis` · `justify-content` · `align-items` · `align-self` · `align-content` · `place-items` · `place-content` · `order` · `gap` · `row-gap` · `column-gap`

### Grid
`grid-template-columns` · `grid-template-rows` · `grid-template-areas` · `grid-auto-rows` · `grid-auto-columns` · `grid-auto-flow` · `grid-area` · `grid-column` (+ `-start` / `-end`) · `grid-row` (+ `-start` / `-end`) · `grid-gap` / `grid-row-gap` / `grid-column-gap` · `justify-self` · `justify-items`

### Tables & lists
`border-collapse` · `border-spacing` · `table-layout` · `list-style-type` · `list-style-position`

### Multi-column
`columns` · `column-count` · `column-width`

### Visual effects
`opacity` · `box-shadow` · `filter` (blur / brightness / contrast / grayscale / invert / opacity / saturate / sepia / hue-rotate) · `backdrop-filter` · `mix-blend-mode` · `isolation` · `content-visibility`

### Transforms (2D + 3D)
`transform` (translate / scale / rotate / skew / matrix / translate3d / translateZ / scale3d / scaleZ / rotateX / rotateY / rotateZ / rotate3d / matrix3d / perspective) · `transform-origin` · `transform-style` · `perspective` · `perspective-origin` · `backface-visibility`

### Animation & transitions
`transition` · `animation` (+ `-name` / `-duration` / `-timing-function` / `-delay` / `-iteration-count` / `-direction` / `-fill-mode` / `-play-state`) · `will-change`

### Scrolling
`scroll-behavior` · `scroll-snap-type` · `scroll-snap-align` · `scroll-snap-stop` · `overscroll-behavior` (+ `-x` / `-y`)

### Interaction
`cursor` · `pointer-events` · `user-select` · `touch-action` · `caret-color` · `accent-color` · `appearance`

### Generated content & counters
`content` (+ `::before` / `::after`) · `counter-reset` · `counter-increment`

### Miscellaneous
`color-scheme` · CSS custom properties (`--*`) with `var()` resolution

## Known gaps

These are recognised by the parser but **not yet stored or applied**:

- **Logical properties** — `margin-block-*`, `margin-inline-*`, `padding-block-*`, `padding-inline-*`, `inset-block-*`, `inset-inline-*`, `border-block-*`, `border-inline-*`. Today only the physical equivalents work.
- **Containment** — `contain`, `contain-intrinsic-size`. (`content-visibility` is stored but containment is not enforced.)
- **Text emphasis** — `text-emphasis`, `text-emphasis-color`, `text-emphasis-style`, `text-emphasis-position`.
- **`font-variant-*` longhands** — `font-variant-numeric`, `font-variant-caps`, `font-variant-ligatures`, etc. Only the legacy `font-variant: small-caps` shorthand is parsed.
- **Masking** — `mask`, `mask-image`, `mask-mode`, `mask-position`, `mask-size`, `mask-repeat`, `mask-clip`, `mask-origin`, `mask-composite`.
- **Scroll padding/margin** — `scroll-padding-*`, `scroll-margin-*`.
- **`@container` queries** — only `@media` and `@supports` are wired into the cascade.
- **`scroll-snap-align` multi-value** — only single-keyword values (`start`, `end`, `center`, `none`) are parsed; the two-value form (`start center`) silently retains the default.
- **`justify-items` initial value** — uses `Stretch` instead of spec `legacy` (no `Legacy` variant in the enum; pragmatic choice for the embedded engine).
- **`text-decoration-thickness` percentage values** — the spec allows `<length-percentage>` but percentages are silently dropped; `resolve_length` has no containing-block context to resolve them. Only `<length>`, `<number>`, `calc()`, `auto`, and `from-font` are handled.
- **`inset` shorthand multi-value** — only the 1-value form is expanded; 2–4 value forms (e.g. `inset: 10px 20px`) pass a multi-value `CssValue` to `resolve_dimension`, which falls through to `Dimension::Auto` for all four sides.
- **`color-mix()`, `color()`, `lab()`, `lch()`, `oklab()`, `oklch()`** — only `rgb()`, `rgba()`, `hsl()`, `hsla()`, `#rrggbb`, and named colors are parsed.

## Storage vs. rendering

Many of the newer properties (`backdrop-filter`, `mix-blend-mode`, `clip-path`,
`perspective`, `perspective-origin`, `transform-style`, `scroll-snap-*`,
`content-visibility`, …) are parsed and stored on `ComputedStyle` so cascading
and `getComputedStyle()` work, but the **paint pipeline doesn't yet honour
them**. They're useful as groundwork — the painter can opt in to a property
without re-touching the parser. When wiring one of these into the compositor,
search for the field name in `src/paint/` and `src/css/values/computed.rs`.

Notable: 3D transform *functions* (`rotateX`, `rotate3d`, `translate3d`,
`matrix3d`, `perspective(d)`, …) **are** painted — they go through the new
`Matrix3d` pipeline in `src/transform.rs` and are flattened orthographically
to a 2D affine for the existing paint path. `backface-visibility: hidden` is
also honored. The unimplemented half is the `perspective` *property* on
ancestor containers, which would let the flatten produce a true perspective
trapezoid instead of a parallelogram.

## How to add a property

1. **`PropertyId` enum** — `src/css/parser/types.rs` (only for hot properties; everything else falls through to `Other`).
2. **Value type** — add an enum/struct in `src/css/values/types.rs` if the property has a constrained set of values.
3. **`ComputedStyle` field** — `src/css/values/computed.rs`. Add the field, set its CSS initial value in `Default`, and (if inheritable) carry it through `inherit()`.
4. **Parse arm** — `src/css/values/apply.rs::apply_declaration`. Use the `as_keyword` / `resolve_length` / `resolve_color` helpers and any small parser helper at the bottom of the file.
5. **Test** — at least one positive parse test in the `mod tests` block in `apply.rs`.
6. **Update this file** — add the property to the relevant section above and bump the count in the header.
