# CSS pipeline

This document covers how CSS is *processed*. For the list of which
properties we actually support, see [`css-coverage.md`](css-coverage.md).

## Stages

```text
bytes
  │
  ▼
css::tokenizer    ──▶ Token stream (idents, numbers, strings, blocks, ...)
  │
  ▼
css::parser       ──▶ Stylesheet { rules: Vec<Rule>, ... }
  │
  ▼
css::cascade      ──▶ for each DOM node: ComputedStyle
  │
  ▼
css::values::apply ─▶ resolve relative units (em / rem / %), inheritance, var()
```

Files of interest:

| Stage | File |
| --- | --- |
| Tokenizer | `src/css/tokenizer.rs` |
| Parser | `src/css/parser/mod.rs`, `parser/types.rs` |
| Selectors | `src/css/selectors.rs` |
| Shorthand expansion | `src/css/shorthand/{background,border,box_model,flex,font,gradient,list}.rs` |
| Cascade | `src/css/cascade/mod.rs`, `matching.rs`, `var_resolve.rs` |
| Computed style | `src/css/values/computed.rs` |
| Apply (parsed → computed) | `src/css/values/apply.rs` |
| Resolve helpers | `src/css/values/resolve.rs` |
| Default UA stylesheet | `src/css/default.rs` |
| Animations / transitions | `src/css/animation.rs`, `src/css/transition.rs` |

## Parser

The parser is a hand-written recursive-descent parser over the
tokenizer output. It produces a `Stylesheet` consisting of:

```rust
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub keyframes: Vec<KeyframesAtRule>,
    pub media_blocks: Vec<MediaBlock>,
    pub supports_blocks: Vec<SupportsBlock>,
    // ...
}

pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    pub origin: Origin, // UA, User, Author, Inline
}

pub struct Declaration {
    pub property: String,
    pub value: CssValue,
    pub important: bool,
    pub property_id: PropertyId, // interned for fast dispatch
}
```

`PropertyId` (in `parser/types.rs`) is an integer enum that interns the
~75 hottest property names. The cascade dispatches on `PropertyId` first
and only falls back to string comparison for cold properties (the
`Other` variant).

## Shorthand expansion

Shorthands like `margin: 1px 2px`, `border: 1px solid red`, or
`background: url(x.png) no-repeat center / cover` are expanded into
their longhand declarations *during parsing* by the modules under
`src/css/shorthand/`. By the time a declaration reaches the cascade,
every property is a longhand. This keeps the cascade simple — it never
needs to know about shorthands.

The `font` shorthand is the trickiest because order matters and several
sub-properties are optional. See `shorthand/font.rs` for the state
machine.

## Cascade

The cascade resolves "given a DOM node, which declarations win?" Its
inputs are:

1. The parsed `Stylesheet`s (UA defaults, author CSS, inline `style="…"`).
2. The DOM tree.
3. The current `MediaViewport` (viewport size, color scheme, prefers-…).

The algorithm:

1. **Index selectors.** `SelectorIndex` (in `cascade/matching.rs`)
   buckets rules by their rightmost simple selector (id, class, tag, or
   universal). When matching against a node, only the relevant buckets
   are tested. This is the single biggest perf win in the cascade.
2. **Match.** For each candidate rule, walk the selector right-to-left
   against the DOM, honoring combinators (`>`, `+`, `~`, descendant)
   and pseudo-classes (`:hover`, `:focus`, `:nth-child`, `:not`, ...).
3. **Sort.** Matched declarations are sorted by `(Origin, !important,
   Specificity, source order)`. The classic CSS cascade order.
4. **Apply.** Each declaration is fed to
   `ComputedStyle::apply_declaration(property, value, parent_font_size)`,
   which resolves units (`em`, `rem`, `%`, `vw`, `vh`, `calc()`),
   normalises colors, and writes to the corresponding field.
5. **Inherit.** After applying, any inheritable property that was not
   explicitly set falls through to the parent's value via
   `ComputedStyle::inherit(parent)`.

Specificity is the standard `(a, b, c)` tuple — id selectors, class /
attr / pseudo-class, type / pseudo-element. Inline `style=""` always
beats stylesheet rules unless the stylesheet uses `!important`.

## `var()` and `calc()`

- **`var(--name, fallback)`** is resolved by `cascade/var_resolve.rs`
  which walks up the DOM looking for an ancestor that defined
  `--name` in its `custom_properties` map. Custom properties always
  inherit, so the lookup is very cheap.
- **`calc(...)`** is parsed eagerly by the value parser into a small
  expression tree, then evaluated when the surrounding length is
  resolved (so `calc(100% - 16px)` knows the containing-block width at
  resolve time).

## `@media` and `@supports`

`@media` blocks live in their own `Vec<MediaBlock>` on the
`Stylesheet`. The cascade evaluates each block's condition against the
current `MediaViewport` (width, height, color-scheme, hover, pointer,
prefers-reduced-motion) and folds matching blocks into the rule list
before specificity sorting.

`@supports` is evaluated similarly, but the condition checks against
the `PropertyId::from_name(...)` table — anything that maps to `Other`
or fails parsing is treated as unsupported.

`@container` queries are **not** implemented yet.

## Animations and transitions

- `transition` declarations are parsed into `Vec<Transition>` on
  `ComputedStyle`. When a property changes due to a state flip
  (typically `:hover`), `src/css/transition.rs` interpolates the old →
  new value over `duration_ms` using the configured `TimingFunction`.
  27 numeric properties are auto-interpolated.
- `@keyframes` rules are parsed into `KeyframesAtRule` and linked to
  elements via the `animation-name` property. `src/css/animation.rs`
  drives the per-frame value updates from `BrowserWidget::tick`.

## Caching

- `MatchedRules` (the per-node list of matched declarations) is cached
  on each DOM node. Cache invalidation happens when:
  - A subtree is marked dirty by mutation.
  - A pseudo-class state flip (`:hover`, `:focus`) affects a node whose
    selectors include that pseudo-class.
  - A `var(--…)` referenced by the node changes.
- The cascade itself is single-threaded.

## Tests

- `src/css/parser/tests.rs` — tokenisation and rule-list parsing.
- `src/css/cascade/tests.rs` — specificity, ordering, inheritance.
- `src/css/values/apply.rs` (`mod tests`) — per-property parsing tests.
  Add one here for every new property you add.
- A `proptest` block (`prop_tests::cascade_arbitrary_css_no_panic`)
  feeds random CSS into the cascade to catch crashes.
