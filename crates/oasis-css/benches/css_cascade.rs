//! Benchmarks for CSS parsing and cascade matching.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oasis_css::cascade::{CascadeContext, style_tree};
use oasis_css::parser::Stylesheet;
use oasis_html::tokenizer::Tokenizer;
use oasis_html::tree_builder::TreeBuilder;

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
                b.iter(|| style_tree(doc, &[stylesheet], &[], &CascadeContext::default()));
            },
        );
    }

    group.finish();
}

/// Like [`generate_css`], but every value goes through custom properties
/// (the pattern design-token CSS such as Wikipedia's Codex uses).
fn generate_css_with_vars(n: usize) -> String {
    let mut css = String::from(
        ":root { --fg: #202122; --pad: 4px; --gap: 2px; --size: 14px; --bg: #f8f9fa; }\n",
    );
    for i in 0..n {
        css.push_str(&format!(
            ".class-{i} {{ color: var(--fg); padding: calc(var(--pad) + {}px); \
             margin: var(--gap) var(--gap); font-size: var(--size, 12px); \
             background: var(--bg); }}\n",
            i % 20,
        ));
    }
    css
}

fn bench_cascade_vars(c: &mut Criterion) {
    let mut group = c.benchmark_group("css_cascade_vars");

    {
        let (n_rules, n_elements) = (100, 500);
        let css = generate_css_with_vars(n_rules);
        let html = generate_html_with_classes(n_elements);
        let label = format!("{n_rules}r_{n_elements}e");

        let stylesheet = Stylesheet::parse(&css);
        let doc = TreeBuilder::build(Tokenizer::new(&html).tokenize());

        group.bench_with_input(
            BenchmarkId::new("style_tree", &label),
            &(&doc, &stylesheet),
            |b, (doc, stylesheet)| {
                b.iter(|| style_tree(doc, &[stylesheet], &[], &CascadeContext::default()));
            },
        );
    }

    group.finish();
}

/// A large, realistic page: a site stylesheet with type, class, id,
/// attribute, pseudo-class, descendant and child selectors (most of which
/// do not match any given element, as on real sites), over a deeply
/// nested document of nav lists, articles, tables and forms.
fn generate_large_page() -> (String, String) {
    let mut css = String::from(
        "* { box-sizing: border-box; }\n\
         html, body { margin: 0; padding: 0; font-family: sans-serif; }\n\
         a { color: #0645ad; text-decoration: none; }\n\
         a:hover { text-decoration: underline; }\n\
         ul li { margin: 2px 0; }\n\
         nav > ul > li > a { padding: 4px 8px; }\n\
         article p:first-child { font-weight: bold; }\n\
         table tr:nth-child(2n) td { background: #f0f0f0; }\n\
         input[type=\"text\"] { border: 1px solid #ccc; }\n\
         #main .content h2 { font-size: 20px; }\n",
    );
    for i in 0..400 {
        css.push_str(&format!(
            ".mod-{i} .item-{} > span {{ color: #{:02x}{:02x}{:02x}; }}\n\
             #sect-{i} p.lead {{ margin: {}px; }}\n\
             div.box-{i}:hover a[href] {{ padding: {}px; }}\n",
            i % 37,
            i % 256,
            (i * 3) % 256,
            (i * 7) % 256,
            i % 9,
            i % 5,
        ));
    }
    let mut html = String::from(
        "<html><head></head><body><nav><ul>\
         <li><a href=\"/\">Home</a></li><li><a href=\"/a\">About</a></li>\
         <li><a href=\"/b\">Blog</a></li></ul></nav><div id=\"main\">",
    );
    for s in 0..150 {
        html.push_str(&format!(
            "<section id=\"sect-{s}\" class=\"mod-{} content\"><h2>Section {s}</h2>\
             <article><p class=\"lead\">Lead <a href=\"#x\">link</a></p>\
             <p>Body <span>text</span> and <em>more</em></p>\
             <ul><li class=\"item-{}\"><span>one</span></li><li>two</li></ul></article>\
             <div class=\"box-{s}\"><table><tr><td>a</td><td>b</td></tr>\
             <tr><td>c</td><td>d</td></tr></table>\
             <form><input type=\"text\"><button>Go</button></form></div></section>",
            s % 400,
            s % 37,
        ));
    }
    html.push_str("</div></body></html>");
    (css, html)
}

fn bench_cascade_large_page(c: &mut Criterion) {
    let mut group = c.benchmark_group("css_cascade_large");
    group.sample_size(20);
    let (css, html) = generate_large_page();
    let stylesheet = Stylesheet::parse(&css);
    let doc = TreeBuilder::build(Tokenizer::new(&html).tokenize());
    group.bench_function("style_tree/1210r_page", |b| {
        b.iter(|| style_tree(&doc, &[&stylesheet], &[], &CascadeContext::default()));
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_stylesheet_parse,
    bench_cascade,
    bench_cascade_vars,
    bench_cascade_large_page
);
criterion_main!(benches);
