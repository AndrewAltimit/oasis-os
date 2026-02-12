# OASIS_OS Fuzz Tests

Fuzz testing for OASIS_OS parsers using `cargo-fuzz` (libFuzzer).

## Prerequisites

```bash
# Install cargo-fuzz (requires nightly)
cargo install cargo-fuzz
```

## Available Targets

| Target | Module | Input | What it exercises |
|--------|--------|-------|-------------------|
| `html_tokenizer` | `browser::html::tokenizer` | UTF-8 HTML | HTML tokenization: tags, attributes, entities, comments |
| `css_parser` | `browser::css::parser` | UTF-8 CSS | CSS stylesheet + inline style parsing |
| `gemini_parser` | `browser::gemini::parser` | UTF-8 text/gemini | Gemini document line parsing |
| `http_response` | `browser::loader::http` | Raw bytes | HTTP response parsing: status, headers, body |
| `skin_toml` | `skin::Skin` | UTF-8 TOML | Skin manifest/layout/features TOML parsing |
| `pbp_parser` | `pbp` | Raw bytes | PSP PBP container format parsing |

## Running

```bash
cd crates/oasis-core

# Run a specific target
cargo +nightly fuzz run html_tokenizer

# Run with a time limit (e.g., 60 seconds)
cargo +nightly fuzz run html_tokenizer -- -max_total_time=60

# Run with more parallelism
cargo +nightly fuzz run html_tokenizer -- -jobs=4 -workers=4

# List all targets
cargo +nightly fuzz list
```

## Corpus

Seed corpus files are in `fuzz/corpus/{target}/`. The fuzzer uses these as
starting points and builds up a corpus of interesting inputs over time.

To add a seed file, just drop it in the appropriate corpus directory:
```bash
cp my_test.html fuzz/corpus/html_tokenizer/
```

## Interpreting Results

- **No crashes after extended run** = good (the parser handles arbitrary input)
- **Crash found** = a panic or OOM on some input. The crashing input is saved
  to `fuzz/artifacts/{target}/`. Fix the parser and re-run.
- **Timeout** = the parser is taking too long on some input (possible infinite
  loop). Review `fuzz/artifacts/{target}/` for the offending input.

## Goals

These fuzz targets verify that parsers:
1. Never panic on arbitrary input
2. Never enter infinite loops
3. Never consume unbounded memory
4. Return graceful errors for malformed input
