//! Benchmarks for CSS parsing and cascade matching.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oasis_browser::css::cascade::style_tree;
use oasis_browser::css::parser::Stylesheet;
use oasis_browser::html::tokenizer::Tokenizer;
use oasis_browser::html::tree_builder::TreeBuilder;

/// Generate a CSS stylesheet with `n` rules.
fn generate_css(n: usize) -> String {
    let mut css = String::with_capacity(n * 80);
    for i in 0..n {
        css.push_str(&format!(
            ".class-{i} {{ color: #{i:02x}{i:02x}{i:02x}; padding: {i}px; margin: {}px; \
             font-size: {}px; background: #{:02x}{:02x}{:02x}; }}\n",
            i % 20,
            10 + i % 20,
            (i * 7) % 256,
            (i * 13) % 256,
            (i * 17) % 256,
        ));
    }
    css
}

/// Generate an HTML document with `n` elements, each with a class that matches
/// one of the CSS rules.
fn generate_html_with_classes(n: usize) -> String {
    let mut html = String::from("<html><head></head><body>\n");
    for i in 0..n {
        html.push_str(&format!(
            "<div class=\"class-{}\"><p>Element {i}</p></div>\n",
            i % 100,
        ));
    }
    html.push_str("</body></html>");
    html
}

fn bench_stylesheet_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("css_parse");

    for n_rules in [50, 100, 500] {
        let css = generate_css(n_rules);
        let label = format!("{n_rules}_rules");

        group.bench_with_input(BenchmarkId::new("parse", &label), &css, |b, css| {
            b.iter(|| Stylesheet::parse(css));
        });
    }

    group.finish();
}

fn bench_cascade(c: &mut Criterion) {
    let mut group = c.benchmark_group("css_cascade");

    for (n_rules, n_elements) in [(50, 200), (100, 500), (100, 1000)] {
        let css = generate_css(n_rules);
        let html = generate_html_with_classes(n_elements);
        let label = format!("{n_rules}r_{n_elements}e");

        let stylesheet = Stylesheet::parse(&css);
        let mut tokenizer = Tokenizer::new(&html);
        let tokens = tokenizer.tokenize();
        let doc = TreeBuilder::build(tokens);

        group.bench_with_input(
            BenchmarkId::new("style_tree", &label),
            &(&doc, &stylesheet),
            |b, (doc, stylesheet)| {
                b.iter(|| style_tree(doc, &[stylesheet], &[]));
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_stylesheet_parse, bench_cascade);
criterion_main!(benches);
