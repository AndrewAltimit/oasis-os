//! Benchmarks for the layout engine.
//!
//! Four groups:
//!
//! - `layout_blocks` / `layout_table` — synthetic stress tests that
//!   measure raw layout throughput as a function of element count.
//! - `layout_nested_flex` — flex containers nested 4 to 16 deep, each
//!   level a content-sized item beside a growing one (row), or a
//!   growing item in a definite-height column. Layout cost must grow
//!   linearly with depth (it was ~depth^3 for rows, 2^depth before).
//! - `layout_corpus` — real-world fixtures from
//!   `crates/oasis-browser/tests/fixtures/`, measured end-to-end
//!   (`parse + cascade + layout`) at two viewport sizes (800×600
//!   desktop, 480×272 PSP). Pair this with the hard wall-clock budget
//!   in `tests/layout_budget.rs` — the bench catches 20% regressions,
//!   the budget test catches catastrophic ones.

use std::path::PathBuf;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oasis_browser::SimpleTextMeasurer;
use oasis_browser::internals::{
    CascadeContext, Stylesheet, Tokenizer, TreeBuilder, build_layout_tree, default_stylesheet,
    style_tree,
};

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
    oasis_browser::internals::Document,
    Vec<Option<oasis_browser::internals::ComputedStyle>>,
) {
    let mut tokenizer = Tokenizer::new(html);
    let tokens = tokenizer.tokenize();
    let doc = TreeBuilder::build(tokens);
    let stylesheet = Stylesheet::parse(css);
    let styles = style_tree(&doc, &[&stylesheet], &[], &CascadeContext::default());
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
                b.iter(|| {
                    build_layout_tree(
                        doc,
                        styles,
                        &measurer,
                        480.0,
                        272.0,
                        None,
                        &std::collections::HashMap::new(),
                    )
                });
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
                b.iter(|| {
                    build_layout_tree(
                        doc,
                        styles,
                        &measurer,
                        480.0,
                        272.0,
                        None,
                        &std::collections::HashMap::new(),
                    )
                });
            },
        );
    }

    group.finish();
}

/// Real-world corpus fixtures benched at desktop + PSP viewport sizes.
///
/// Each entry measures the full `parse + cascade + build_layout_tree`
/// pipeline against the fixture. Criterion will fire on a > ~20%
/// regression compared to the saved baseline, so this is where paint
/// / layout rewrites should get their pre-merge pass.
fn bench_corpus_layout(c: &mut Criterion) {
    const FIXTURES: &[&str] = &[
        "wikipedia_article.html",
        "news_homepage.html",
        "blog_post.html",
        "hackernews_frontpage.html",
        "github_readme.html",
        "rust_std_docs.html",
        "forum_thread.html",
        "commerce_product.html",
        "substack_post.html",
        "adversarial_malformed.html",
        "rtl_bidi_stress.html",
        "responsive_grid.html",
    ];

    let measurer = SimpleTextMeasurer;
    let ua = default_stylesheet();
    let mut group = c.benchmark_group("layout_corpus");

    for name in FIXTURES {
        let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
            .iter()
            .collect();
        let html = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("missing bench fixture {}: {e}", path.display()));

        for (w, h, label) in [
            (800.0f32, 600.0f32, "800x600"),
            (480.0f32, 272.0f32, "480x272"),
        ] {
            let bench_id = format!("{}_{}", name.trim_end_matches(".html"), label);
            let ua_ref = &ua;
            group.bench_with_input(
                BenchmarkId::new("pipeline", &bench_id),
                &(html.clone(), w, h),
                |b, (html, w, h)| {
                    b.iter(|| {
                        let tokens = Tokenizer::new(html).tokenize();
                        let doc = TreeBuilder::build(tokens);
                        let sheets: Vec<&Stylesheet> = vec![ua_ref];
                        let styles = style_tree(&doc, &sheets, &[], &CascadeContext::default());
                        build_layout_tree(
                            &doc,
                            &styles,
                            &measurer,
                            *w,
                            *h,
                            None,
                            &std::collections::HashMap::new(),
                        )
                    });
                },
            );
        }
    }

    group.finish();
}

/// `depth` nested flex levels of `open` ... `close` around a text leaf.
fn generate_nested(open: &str, close: &str, depth: usize) -> String {
    format!(
        "<html><head></head><body>{}leaf text{}</body></html>",
        open.repeat(depth),
        close.repeat(depth)
    )
}

fn bench_nested_flex(c: &mut Criterion) {
    let mut group = c.benchmark_group("layout_nested_flex");
    let css = "div { display: block; } .row { display: flex; } \
               .grow { flex-grow: 1; padding: 1px; } \
               .col { display: flex; flex-direction: column; height: 400px; } \
               .one { flex: 1; }";
    let measurer = SimpleTextMeasurer;
    let patterns = [
        (
            "row_grow",
            "<div class=\"row\"><span>a b</span><div class=\"grow\">",
            "</div></div>",
        ),
        ("row_auto", "<div class=\"row\">", "</div>"),
        (
            "column_flex1",
            "<div class=\"col\"><div class=\"one\">",
            "</div></div>",
        ),
    ];
    for (name, open, close) in patterns {
        for depth in [4usize, 8, 12, 16] {
            let html = generate_nested(open, close, depth);
            let (doc, styles) = prepare_for_layout(&html, css);
            group.bench_with_input(
                BenchmarkId::new(name, depth),
                &(&doc, &styles),
                |b, (doc, styles)| {
                    b.iter(|| {
                        build_layout_tree(
                            doc,
                            styles,
                            &measurer,
                            800.0,
                            600.0,
                            None,
                            &std::collections::HashMap::new(),
                        )
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_block_layout,
    bench_table_layout,
    bench_nested_flex,
    bench_corpus_layout
);
criterion_main!(benches);
