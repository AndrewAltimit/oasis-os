# Testing

`oasis-browser` ships with three layers of automated checks: unit
tests in-module, integration tests under `tests/`, and benchmarks
under `benches/`. There is also a fuzz harness under `fuzz/` for the
parsers.

## Layers

### Unit tests

Per-module `#[cfg(test)] mod tests` blocks. These are the bulk of the
suite (~2,200 tests across the engine's four crates with `javascript`
on) and are the right place to add a regression test for a single
function or one CSS property arm. The parser and loader crates test on
their own, without building layout, paint or QuickJS:

```bash
cargo test -p oasis-html   # incl. tests/html5lib_tree_construction.rs
cargo test -p oasis-css
cargo test -p oasis-http
cargo test -p oasis-browser --features javascript
```

Hot spots:

| Area | Test module |
| --- | --- |
| Tokenizer | `crates/oasis-html/src/tokenizer/tests.rs` |
| Tree builder | `crates/oasis-html/src/tree_builder/tests.rs` |
| CSS parser | `crates/oasis-css/src/parser/tests.rs` |
| CSS cascade | `crates/oasis-css/src/cascade/tests.rs` |
| `apply_declaration` (per-property) | `crates/oasis-css/src/values/apply/mod.rs` (`mod tests`) |
| Block layout | `src/layout/block/tests.rs` |
| Text cache | `src/layout/text_cache.rs` (`mod tests`) |
| Loader cache | `crates/oasis-http/src/cache.rs` (`mod tests`) |
| Cookies | `crates/oasis-http/src/cookies.rs` (`mod tests`) |
| Navigation history | `src/nav.rs` (`mod tests`) |

Several modules also have `mod prop` blocks containing
[`proptest`](https://docs.rs/proptest) property tests. These run
hundreds of randomised cases to catch crashes and structural
violations. Add property tests for invariants ("history length equals
navigations", "cascade never panics on arbitrary CSS") rather than
specific values.

### Integration tests

`tests/browser_integration.rs` contains end-to-end tests that drive
the full pipeline (HTML → CSS → layout → paint) on small synthetic
pages. They are slower than unit tests but catch interactions the
units miss. Examples:

- `simple_page_paragraph_block_with_positive_height`
- `heading_hierarchy_font_sizes`
- `nested_structure_dom_tree_and_style_inheritance`
- `link_styling_color_and_underline`
- `table_layout_cells_laid_out`
- `nested_lists_display_and_indentation`
- `complex_page_with_gradients_completes_within_budget` (a soft perf
  budget — fails if a known-good page suddenly takes 10× longer)

Add an integration test when a feature spans multiple modules and a
unit test alone would not exercise the full path.

### Benchmarks

`benches/` is criterion-based:

| Bench | Measures |
| --- | --- |
| `oasis-css/benches/css_cascade.rs` | cascade throughput on a large stylesheet |
| `oasis-html/benches/html_parsing.rs` | tokenizer + tree builder throughput |
| `layout_engine.rs` | layout build + relayout throughput |
| `paint.rs` | display list record + replay |

Run with:

```bash
cargo bench -p oasis-browser
cargo bench -p oasis-css --bench css_cascade
cargo bench -p oasis-html --bench html_parsing
```

Benchmarks are not run in CI by default — they exist to catch
regressions when you suspect one. Compare numbers against `main`
before/after a change.

### Fuzzing

`fuzz/` contains `cargo-fuzz` targets for the HTML and CSS parsers
(through `oasis_browser::internals`). CI fuzzes the copies in
`crates/oasis-core/fuzz` (`.github/workflows/fuzz.yml`), which also
cover the Gemini and HTTP response parsers. Run with:

```bash
cd crates/oasis-browser/fuzz
cargo +nightly fuzz run fuzz_html_tokenizer
cargo +nightly fuzz run fuzz_css_parser
```

The fuzzers should not panic, hit `unreachable!()`, or trigger UB
under MIRI.

## What to run before pushing

```bash
cargo fmt --all
cargo clippy -p oasis-browser -- -D warnings
cargo test  -p oasis-browser
```

For a full repo check (mirrors CI):

```bash
docker compose --profile ci run --rm rust-ci cargo fmt --all -- --check
docker compose --profile ci run --rm rust-ci cargo clippy --workspace --all-targets -- -D warnings
docker compose --profile ci run --rm rust-ci cargo test  --workspace
```

## Manual smoke test

The fastest way to confirm that nothing visually regressed is to
launch the desktop binary and click into the Browser app:

```bash
cargo run --release -p oasis-app
```

CI also runs the headless `screenshot-tests` binary (`oasis-app`) and
diffs its frames against `screenshots/baseline/` with
`scripts/compare-screenshots.sh`. After an intentional visual change,
regenerate with
`cargo run -p oasis-app --bin screenshot-tests --release -- --bless`
(with `OASIS_FIXED_TIME` / `OASIS_FIXED_FRAME` set as in
`.github/workflows/main-ci.yml`) and copy `screenshots/tests/` over the
baseline.

## Adding a new test

1. **Unit test first.** Drop a `#[test] fn ...` in the module where
   the code lives.
2. **Integration test second.** Only escalate to
   `tests/browser_integration.rs` when the unit-level setup gets
   noisy or when the feature spans modules.
3. **Property test for invariants.** If you can express the
   correctness condition as "for all inputs, X holds", reach for
   `proptest`.
4. **Benchmark for hot paths.** When optimising layout, paint, or
   cascade code, capture a baseline before/after with criterion.
