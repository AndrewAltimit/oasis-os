# Testing

`oasis-browser` ships with three layers of automated checks: unit
tests in-module, integration tests under `tests/`, and benchmarks
under `benches/`. There is also a fuzz harness under `fuzz/` for the
parsers.

## Layers

### Unit tests

Per-module `#[cfg(test)] mod tests` blocks. These are the bulk of the
suite (~1500 tests) and are the right place to add a regression test
for a single function or one CSS property arm.

Hot spots:

| Area | Test module |
| --- | --- |
| Tokenizer | `src/html/tokenizer/tests.rs` |
| Tree builder | `src/html/tree_builder/tests.rs` |
| CSS parser | `src/css/parser/tests.rs` |
| CSS cascade | `src/css/cascade/tests.rs` |
| `apply_declaration` (per-property) | `src/css/values/apply.rs` (`mod tests`) |
| Block layout | `src/layout/block/tests.rs` |
| Text cache | `src/layout/text_cache.rs` (`mod tests`) |
| Loader cache | `src/loader/cache.rs` (`mod tests`) |
| Cookies | `src/loader/cookies.rs` (`mod tests`) |
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
| `css_cascade.rs` | cascade throughput on a large stylesheet |
| `html_parsing.rs` | tokenizer + tree builder throughput |
| `layout_engine.rs` | layout build + relayout throughput |
| `paint.rs` | display list record + replay |

Run with:

```bash
cargo bench -p oasis-browser
```

Benchmarks are not run in CI by default — they exist to catch
regressions when you suspect one. Compare numbers against `main`
before/after a change.

### Fuzzing

`fuzz/` contains `cargo-fuzz` targets for the HTML and CSS parsers.
Run with:

```bash
cd crates/oasis-browser
cargo +nightly fuzz run html_tokenizer
cargo +nightly fuzz run css_parser
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
docker compose --profile ci run --rm rust-ci cargo clippy --workspace -- -D warnings
docker compose --profile ci run --rm rust-ci cargo test  --workspace
```

## Manual smoke test

The fastest way to confirm that nothing visually regressed is to
launch the desktop binary and click into the Browser app:

```bash
cargo build --release -p oasis-app
LD_LIBRARY_PATH="$(pwd)/target/release:$LD_LIBRARY_PATH" ./target/release/oasis-app
```

The repo also ships an automated screenshot test under
`scripts/screenshot-test.sh` (used in CI) that runs the app headlessly,
captures a frame, and diffs it against the golden image. Update the
golden image with `cargo run -p oasis-app --bin oasis-screenshot` after
intentional visual changes.

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
