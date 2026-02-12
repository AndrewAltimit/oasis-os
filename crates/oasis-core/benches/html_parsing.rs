//! Benchmarks for the HTML tokenizer and DOM tree builder.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oasis_core::browser::html::tokenizer::Tokenizer;
use oasis_core::browser::html::tree_builder::TreeBuilder;

/// Generate a synthetic HTML document of approximately `target_bytes` size.
fn generate_html(target_bytes: usize) -> String {
    let header = "<html><head><title>Benchmark</title></head><body>\n";
    let footer = "</body></html>";
    let overhead = header.len() + footer.len();

    let paragraph = "<div class=\"content\"><h2>Section</h2>\
        <p>Lorem ipsum dolor sit amet, consectetur adipiscing elit. \
        Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.</p>\
        <ul><li>Item one</li><li>Item two</li><li>Item three</li></ul></div>\n";

    let repeats = (target_bytes.saturating_sub(overhead)) / paragraph.len() + 1;
    let mut html = String::with_capacity(target_bytes + 256);
    html.push_str(header);
    for _ in 0..repeats {
        html.push_str(paragraph);
        if html.len() >= target_bytes {
            break;
        }
    }
    html.push_str(footer);
    html
}

fn bench_tokenizer(c: &mut Criterion) {
    let mut group = c.benchmark_group("html_tokenizer");

    for size in [10_000, 50_000, 100_000] {
        let html = generate_html(size);
        let label = format!("{size}B");

        group.bench_with_input(BenchmarkId::new("tokenize", &label), &html, |b, html| {
            b.iter(|| {
                let mut tokenizer = Tokenizer::new(html);
                tokenizer.tokenize()
            });
        });
    }

    group.finish();
}

fn bench_tree_builder(c: &mut Criterion) {
    let mut group = c.benchmark_group("html_tree_builder");

    for size in [10_000, 50_000, 100_000] {
        let html = generate_html(size);
        let label = format!("{size}B");

        // Pre-tokenize so we only measure tree building.
        let mut tokenizer = Tokenizer::new(&html);
        let tokens = tokenizer.tokenize();

        group.bench_with_input(
            BenchmarkId::new("build", &label),
            &tokens,
            |b, tokens| {
                b.iter(|| TreeBuilder::build(tokens.clone()));
            },
        );
    }

    group.finish();
}

fn bench_full_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("html_full_parse");

    for size in [10_000, 50_000, 100_000] {
        let html = generate_html(size);
        let label = format!("{size}B");

        group.bench_with_input(BenchmarkId::new("tokenize+build", &label), &html, |b, html| {
            b.iter(|| {
                let mut tokenizer = Tokenizer::new(html);
                let tokens = tokenizer.tokenize();
                TreeBuilder::build(tokens)
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_tokenizer, bench_tree_builder, bench_full_parse);
criterion_main!(benches);
