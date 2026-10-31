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
| Tokenizer | `crates/oasis-css/src/tokenizer.rs` |
| Parser | `crates/oasis-css/src/parser/mod.rs`, `parser/types.rs` |
| Selectors | `crates/oasis-css/src/selectors.rs` |
| Shorthand expansion | `crates/oasis-css/src/shorthand/{background,border,box_model,flex,font,gradient,list}.rs` |
| Cascade | `crates/oasis-css/src/cascade/mod.rs`, `matching.rs`, `var_resolve.rs` |
| Computed style | `crates/oasis-css/src/values/computed.rs` |
| Apply (parsed → computed) | `crates/oasis-css/src/values/apply/mod.rs` |
| Resolve helpers | `crates/oasis-css/src/values/resolve.rs` |
| Default UA stylesheet | `crates/oasis-css/src/default.rs` |
| Animations / transitions | `crates/oasis-css/src/animation.rs`, `crates/oasis-css/src/transition.rs` |

## Parser

The parser is a hand-written recursive-descent parser over the
tokenizer output. It produces a `Stylesheet` consisting of:

```rust
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub keyframes: Vec<KeyframesRule>,
    pub layers: Vec<String>, // cascade layers, in cascade order
    // ... @counter-style, @property, @font-face, ...
}

pub struct Rule {
    pub selectors: SelectorList,
    pub declarations: Vec<Declaration>,
    pub layer: Option<u16>,                   // index into `layers`
    pub container: Option<ContainerCondition>, // enclosing `@container`
    pub scope: Option<ScopeCondition>,         // enclosing `@scope`
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
`crates/oasis-css/src/shorthand/`. By the time a declaration reaches the cascade,
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

1. **Index selectors.** `SelectorIndex` (in `oasis-css/src/cascade/index.rs`)
   files each rule under one key of its rightmost (subject) compound
   selector, the most specific one (id > class > tag, else universal).
   When matching against a node, only the relevant buckets are tested.
   This is the single biggest perf win in the cascade.
2. **Filter.** An ancestor Bloom filter (`cascade/ancestor_filter.rs`)
   holds the ids, classes and tags of the current element's ancestors;
   a descendant / child selector whose required ancestor parts are
   missing is rejected without walking the ancestor chain.
3. **Match.** For each remaining rule, walk the selector right-to-left
   against the DOM, honoring combinators (`>`, `+`, `~`, descendant)
   and pseudo-classes (`:hover`, `:focus`, `:nth-child`, `:has()`, and
   `:is()` / `:where()` / `:not()` with complex selectors).
4. **Sort.** Matched declarations (borrowed from the stylesheet, not
   cloned) are sorted by origin-and-importance tier, cascade layer,
   specificity and source order. The tiers, lowest first: normal
   user-agent < presentational hints < normal author < normal inline <
   important author < important inline < important user-agent. The
   built-in UA sheet is its own origin, so any author rule, layered or
   not, beats it; within the author origin unlayered rules beat layered
   ones (reversed for `!important`). `revert` rolls back to the UA
   value, `revert-layer` to the previous layer.
5. **Apply.** Each declaration is fed to
   `ComputedStyle::apply_declaration(property, value, parent_font_size)`,
   which resolves units (`em`, `rem`, `%`, `vw`, `vh`, `calc()`),
   normalises colors, and writes to the corresponding field.
6. **Inherit.** After applying, any inheritable property that was not
   explicitly set falls through to the parent's value via
   `ComputedStyle::inherit(parent)`.

Specificity is the standard `(a, b, c)` tuple — id selectors, class /
attr / pseudo-class, type / pseudo-element. Inline `style=""` always
beats stylesheet rules unless the stylesheet uses `!important`.

## `var()` and `calc()`

- **`var(--name, fallback)`**: any declaration whose value contains
  `var(` anywhere (inside `calc()`, `rgb()`, gradients, `color-mix()`,
  shorthands, ...) is stored by the parser as
  `CssValue::Unresolved(raw_text)` and is not shorthand-expanded. At
  computed-value time `cascade/var_resolve.rs` substitutes the
  element's custom properties textually (nested fallbacks, chained
  references), and the result is re-parsed with the property's normal
  parser plus shorthand expansion (memoised per thread by property +
  substituted text). A declaration that can't be resolved (missing
  property without fallback, cycle) is *invalid at computed-value
  time* and behaves as `unset`: inherited properties take the parent's
  value, others their initial value.
- **Custom properties** are case-sensitive (`--Foo` ≠ `--foo`). Their
  own `var()` references are substituted on the element that declares
  them, so children inherit computed values; every member of a
  reference cycle becomes invalid (guaranteed-invalid), while a
  referrer outside the cycle can still use its fallback.
- **`calc(...)`** is parsed eagerly by the value parser into a small
  expression tree, then evaluated when the surrounding length is
  resolved (so `calc(100% - 16px)` knows the containing-block width at
  resolve time).

## `@media`, `@supports`, `@container`

`@media` is evaluated while parsing: `Stylesheet::parse_with_viewport`
checks each block's condition against a `MediaViewport` (width, height,
color-scheme, hover, pointer, prefers-reduced-motion) and keeps only
the rules of matching blocks. The browser re-parses and re-cascades
when a resize flips a condition.

`@supports` is also decided at parse time, against the list of
properties `apply_declaration` knows (`parser/supports.rs`); any other
property, or a condition that fails to parse, is treated as unsupported.

`@container` rules keep their condition on `Rule::container`; the
cascade evaluates it against the nearest matching container ancestor in
the layout snapshot of the previous pass (no snapshot: the rule does
not apply).

## Animations and transitions

- `transition` declarations are parsed into `Vec<Transition>` on
  `ComputedStyle`. When a property changes due to a state flip
  (typically `:hover`), `crates/oasis-css/src/transition.rs` interpolates the old →
  new value over `duration_ms` using the configured `TimingFunction`.
  27 numeric properties are auto-interpolated.
- `@keyframes` rules are parsed into `KeyframesRule` and linked to
  elements via the `animation-name` property. `crates/oasis-css/src/animation.rs`
  drives the per-frame value updates from `BrowserWidget::tick`.

## Caching and threading

- The browser widget keeps the parsed author sheets and the
  `SelectorIndex` between cascades. A `:hover` / `:focus` flip
  re-cascades only the affected nodes (the old and new target's
  ancestors and their descendants, parents first) and repaints without
  relayout when only visual properties changed.
- `var()` substitutions are memoised per thread by property +
  substituted text (see above).
- The cascade is single-threaded unless the `parallel-style` feature
  (rayon) is on.

## Tests

- `crates/oasis-css/src/parser/tests.rs` — tokenisation and rule-list parsing.
- `crates/oasis-css/src/cascade/tests.rs` — specificity, ordering, inheritance.
- `crates/oasis-css/src/values/apply/mod.rs` (`mod tests`) — per-property parsing tests.
  Add one here for every new property you add.
- A `proptest` block (`prop_tests::cascade_arbitrary_css_no_panic`)
  feeds random CSS into the cascade to catch crashes.
