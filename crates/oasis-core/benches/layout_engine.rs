//! Benchmarks for the layout engine.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oasis_core::browser::SimpleTextMeasurer;
use oasis_core::browser::css::cascade::style_tree;
use oasis_core::browser::css::parser::Stylesheet;
use oasis_core::browser::html::tokenizer::Tokenizer;
use oasis_core::browser::html::tree_builder::TreeBuilder;
use oasis_core::browser::layout::block::build_layout_tree;

/// Generate HTML with `n` block-level divs.
fn generate_blocks(n: usize) -> String {
    let mut html = String::from("<html><head></head><body>\n");
    for i in 0..n {
        html.push_str(&format!(
            "<div style=\"padding: 4px; margin: 2px;\"><p>Block {i} with some content text.</p></div>\n",
        ));
    }
    html.push_str("</body></html>");
    html
}

/// Generate HTML with an NxN table.
fn generate_table(rows: usize, cols: usize) -> String {
    let mut html = String::from("<html><head></head><body><table>\n");
    for r in 0..rows {
        html.push_str("<tr>");
        for c in 0..cols {
            html.push_str(&format!("<td>R{r}C{c}</td>"));
        }
        html.push_str("</tr>\n");
    }
    html.push_str("</table></body></html>");
    html
}

/// Parse HTML+CSS and return the components needed for layout.
fn prepare_for_layout(
    html: &str,
    css: &str,
) -> (
    oasis_core::browser::html::dom::Document,
    Vec<Option<oasis_core::browser::css::values::ComputedStyle>>,
) {
    let mut tokenizer = Tokenizer::new(html);
    let tokens = tokenizer.tokenize();
    let doc = TreeBuilder::build(tokens);
    let stylesheet = Stylesheet::parse(css);
    let styles = style_tree(&doc, &[&stylesheet], &[]);
    (doc, styles)
}

fn bench_block_layout(c: &mut Criterion) {
    let mut group = c.benchmark_group("layout_blocks");

    let css = "div { display: block; } p { display: block; }";
    let measurer = SimpleTextMeasurer;

    for n in [100, 500, 1000] {
        let html = generate_blocks(n);
        let (doc, styles) = prepare_for_layout(&html, css);
        let label = format!("{n}_blocks");

        group.bench_with_input(
            BenchmarkId::new("build_layout_tree", &label),
            &(&doc, &styles),
            |b, (doc, styles)| {
                b.iter(|| build_layout_tree(doc, styles, &measurer, 480.0, 272.0));
            },
        );
    }

    group.finish();
}

fn bench_table_layout(c: &mut Criterion) {
    let mut group = c.benchmark_group("layout_table");

    let css = "table { display: table; } tr { display: table-row; } td { display: table-cell; padding: 2px; }";
    let measurer = SimpleTextMeasurer;

    for (rows, cols) in [(10, 10), (20, 20), (50, 10)] {
        let html = generate_table(rows, cols);
        let (doc, styles) = prepare_for_layout(&html, css);
        let label = format!("{rows}x{cols}");

        group.bench_with_input(
            BenchmarkId::new("build_layout_tree", &label),
            &(&doc, &styles),
            |b, (doc, styles)| {
                b.iter(|| build_layout_tree(doc, styles, &measurer, 480.0, 272.0));
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_block_layout, bench_table_layout);
criterion_main!(benches);
