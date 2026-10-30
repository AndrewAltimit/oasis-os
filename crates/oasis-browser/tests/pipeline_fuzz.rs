//! Property-based "fuzz" of the whole parse -> style -> layout -> paint
//! pipeline with adversarial markup and CSS values.
//!
//! The generators build structurally plausible documents (nested
//! elements from a tag palette that covers every layout mode: block,
//! inline, flex, grid, table, list, replaced, form controls, SVG,
//! canvas) and decorate them with inline styles drawn from a palette of
//! hostile values: huge and negative lengths, `NaN`-producing `calc()`,
//! enormous `repeat()` counts, degenerate transforms, deep nesting. The
//! only property checked is the one a browser must never violate on
//! untrusted input: no panic, no hang, and every painted coordinate
//! stays finite.

use std::collections::HashMap;

use oasis_browser::internals::{
    CascadeContext, LayoutBox, PaintViewport, Stylesheet, TextMeasurer, Tokenizer, TreeBuilder,
    build_layout_tree, default_stylesheet, paint_page, style_tree,
};
use oasis_test_backend::MockSdiCore;
use proptest::prelude::*;

struct FixedMeasurer;

impl TextMeasurer for FixedMeasurer {
    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        (text.chars().count() as u32).saturating_mul(u32::from(font_size).div_ceil(2).max(1))
    }
}

const TAGS: &[&str] = &[
    "div", "span", "p", "pre", "b", "a", "ul", "ol", "li", "table", "tr", "td", "th", "thead",
    "caption", "img", "input", "textarea", "select", "option", "button", "h1", "svg", "canvas",
    "details", "summary", "br", "hr", "label", "fieldset", "legend", "dl", "dt", "dd", "section",
    "sub", "sup", "ruby", "rt", "wbr", "video", "iframe", "colgroup", "col",
];

const DECLS: &[&str] = &[
    "display:flex",
    "display:grid",
    "display:inline-block",
    "display:table",
    "display:table-cell",
    "display:list-item",
    "display:none",
    "display:contents",
    "display:inline-flex",
    "flex-direction:column",
    "flex-wrap:wrap",
    "flex:1e30 1e30 0",
    "flex-basis:-5px",
    "order:-2147483648",
    "width:1e30px",
    "width:-1e30px",
    "width:99999999999em",
    "height:1e38px",
    "min-width:1e30px",
    "max-width:-1px",
    "width:calc(100% / 0)",
    "width:calc(1e308px * 1e308)",
    "width:calc(0px / 0)",
    "padding:1e30px",
    "margin:-1e30px",
    "margin:auto",
    "border:1e30px solid red",
    "border-width:-3px",
    "border-radius:1e30px",
    "font-size:1e30px",
    "font-size:0",
    "font-size:-10px",
    "line-height:1e30",
    "line-height:-5",
    "letter-spacing:1e30px",
    "word-spacing:-1e30px",
    "text-indent:-1e30px",
    "white-space:pre",
    "white-space:nowrap",
    "word-break:break-all",
    "overflow:scroll",
    "overflow:hidden",
    "position:absolute",
    "position:fixed",
    "position:relative",
    "position:sticky",
    "top:-1e30px",
    "left:1e30px",
    "right:50%",
    "bottom:auto",
    "z-index:2147483647",
    "z-index:-2147483648",
    "float:left",
    "float:right",
    "clear:both",
    "columns:100000",
    "column-count:2147483647",
    "column-gap:1e30px",
    "grid-template-columns:repeat(100000, 1fr)",
    "grid-template-columns:repeat(auto-fill, minmax(0px, 1fr))",
    "grid-template-columns:repeat(auto-fill, 0px)",
    "grid-template-rows:repeat(4, 1e30px)",
    "grid-column:1 / -1",
    "grid-column:span 2147483647",
    "grid-row:-2147483648 / 2147483647",
    "grid-area:1 / 1 / 99999 / 99999",
    "gap:1e30px",
    "transform:scale(1e30)",
    "transform:scale(0)",
    "transform:rotate(1e30deg)",
    "transform:matrix(0,0,0,0,0,0)",
    "transform:matrix3d(0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0)",
    "transform:perspective(0px) rotateY(90deg)",
    "transform:translate3d(1e30px, -1e30px, 1e30px)",
    "perspective:0",
    "transform-style:preserve-3d",
    "opacity:-1",
    "opacity:1e30",
    "box-shadow:1e30px 1e30px 1e30px 1e30px black",
    "text-shadow:1e30px 0 1e30px red",
    "filter:blur(1e30px)",
    "outline:1e30px solid blue",
    "list-style-type:upper-roman",
    "counter-reset:c 2147483647",
    "counter-increment:c 2147483647",
    "table-layout:fixed",
    "border-collapse:collapse",
    "border-spacing:1e30px",
    "aspect-ratio:0 / 0",
    "aspect-ratio:1e30 / 1e-30",
    "object-fit:cover",
    "text-align:justify",
    "direction:rtl",
    "writing-mode:vertical-rl",
    "tab-size:2147483647",
    "background:linear-gradient(1e30deg, red, blue)",
    "background-size:0 0",
    "background-position:1e30px -1e30px",
    "transition:all 1e30s",
    "animation:x -1s infinite",
    "content:counter(c)",
    "--v:var(--v)",
    "width:var(--v, 1e30px)",
];

const ATTRS: &[&str] = &[
    "colspan=\"65535\"",
    "rowspan=\"65535\"",
    "colspan=\"-1\"",
    "rowspan=\"0\"",
    "span=\"100000\"",
    "width=\"4294967295\"",
    "height=\"-1\"",
    "size=\"2147483647\"",
    "cols=\"0\"",
    "rows=\"4294967296\"",
    "start=\"-2147483648\"",
    "value=\"2147483647\"",
    "reversed",
    "open",
    "hidden",
    "dir=\"rtl\"",
    "type=\"range\"",
    "type=\"checkbox\"",
    "multiple",
    "src=\"data:image/png;base64,AAAA\"",
    "viewBox=\"0 0 0 0\"",
];

const TEXTS: &[&str] = &[
    "",
    "hello world",
    "\u{0}\u{fffd}",
    "\u{202e}rtl override\u{202c}",
    "\u{ad}\u{ad}\u{ad}",
    "a\u{301}\u{302}\u{303}",
    "\t\t\t\n\n\n",
    "&#x110000;&#0;&#xD800;",
    "verylongwordwithoutanybreakopportunityatallwhatsoeverxxxxxxxxxxxxxxxxx",
    "\u{1f600}\u{200d}\u{1f600}",
    "&amp;&lt;&gt;&nbsp;&notanentity;",
];

#[derive(Debug, Clone)]
enum Node {
    Text(usize),
    Elem {
        tag: usize,
        decls: Vec<usize>,
        attrs: Vec<usize>,
        children: Vec<Node>,
    },
}

fn arb_node() -> impl Strategy<Value = Node> {
    let leaf = (0..TEXTS.len()).prop_map(Node::Text);
    leaf.prop_recursive(6, 48, 6, |inner| {
        (
            0..TAGS.len(),
            prop::collection::vec(0..DECLS.len(), 0..4),
            prop::collection::vec(0..ATTRS.len(), 0..2),
            prop::collection::vec(inner, 0..6),
        )
            .prop_map(|(tag, decls, attrs, children)| Node::Elem {
                tag,
                decls,
                attrs,
                children,
            })
    })
}

fn render(node: &Node, out: &mut String) {
    match node {
        Node::Text(i) => out.push_str(TEXTS[*i]),
        Node::Elem {
            tag,
            decls,
            attrs,
            children,
        } => {
            let tag = TAGS[*tag];
            out.push('<');
            out.push_str(tag);
            if !decls.is_empty() {
                out.push_str(" style=\"");
                for d in decls {
                    out.push_str(DECLS[*d]);
                    out.push(';');
                }
                out.push('"');
            }
            for a in attrs {
                out.push(' ');
                out.push_str(ATTRS[*a]);
            }
            out.push('>');
            for c in children {
                render(c, out);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        },
    }
}

fn check_finite(b: &LayoutBox) -> Result<(), String> {
    let d = &b.dimensions.content;
    for v in [d.x, d.y, d.width, d.height] {
        if !v.is_finite() {
            return Err(format!(
                "non-finite layout coordinate {v} in {:?}",
                b.box_type
            ));
        }
    }
    b.children.iter().try_for_each(check_finite)
}

/// Run one document through the pipeline.
///
/// With `PIPELINE_FUZZ_DUMP=<file>` set, each input is written to that
/// file before it runs, so a case that aborts the process (out of
/// memory, stack overflow) is left behind for reproduction.
fn run(html: &str, sheet: Option<&str>, width: f32, height: f32) -> Result<(), String> {
    if let Ok(dump) = std::env::var("PIPELINE_FUZZ_DUMP") {
        let _ = std::fs::write(
            &dump,
            format!("{width}\n{}\n----\n{html}", sheet.unwrap_or_default()),
        );
    }
    let doc = TreeBuilder::build(Tokenizer::new(html).tokenize());
    let ua = default_stylesheet();
    let author = sheet.map(Stylesheet::parse);
    let mut sheets: Vec<&Stylesheet> = vec![&ua];
    if let Some(s) = author.as_ref() {
        sheets.push(s);
    }
    let styles = style_tree(&doc, &sheets, &[], &CascadeContext::default());
    let layout = build_layout_tree(
        &doc,
        &styles,
        &FixedMeasurer,
        width,
        height,
        None,
        &HashMap::new(),
    );
    check_finite(&layout)?;
    let mut backend = MockSdiCore::new(width as u32, height as u32);
    let vp = PaintViewport {
        scroll_y: 0.0,
        scroll_x: 0.0,
        x: 0,
        y: 0,
        width,
        height,
        visible_height: height,
        focused_node: None,
        counter_styles: Vec::new(),
    };
    paint_page(&layout, &mut backend, vp, &HashMap::new())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Inputs the generators found, each of which used to abort the process
/// or panic. Kept as fixed cases so they stay fixed without relying on
/// the random search finding them again.
#[test]
fn known_hostile_inputs() {
    let cases: &[(&str, &str)] = &[
        // Dense rows x cols occupancy grid: ~4 GB.
        (
            "<table><tr><td colspan=65535 rowspan=65535>x<td>y<tr><td>z</table>",
            "",
        ),
        // `min-width: 2000px` on a cell used to be read as colspan=2.
        ("<table><tr><td style='min-width:1e30px'>a<td>b</table>", ""),
        // One column bucket per declared column: ~50 GB.
        (
            "<p style='column-count:2147483647;width:calc(100% / 0)'>word</p>",
            "",
        ),
        // Line-box capacity from width / available width: 69 GB.
        (
            "<button style='white-space:pre;width:1e30px'>\u{ad}\u{ad}\u{ad}</button>",
            "button { text-align: justify; line-height: 1e30; width: 99999999999em }",
        ),
        // Two billion spaces per tab.
        (
            "<pre>\t\t\t</pre>",
            "pre { font-size: 1e30px; tab-size: 2147483647 }",
        ),
        // Integer overflow in border edge placement (debug panic).
        (
            "<summary><pre></pre></summary>",
            "summary > *, :is(summary) { width: 99999999999em; float: right }",
        ),
        // Offscreen layer the size of the (saturated) box.
        (
            "<div style='opacity:.5;filter:blur(2px);width:1e30px;height:1e30px'>x</div>",
            "",
        ),
        // Byte-offset slicing of non-ASCII colours.
        ("<table bgcolor='#\u{e9}1'><tr><td>x</table>", ""),
        (
            "<svg width=10 height=10><rect width=5 height=5 fill='#\u{e9}1'/></svg>",
            "",
        ),
    ];
    for &(html, sheet) in cases {
        // Some of these were only reached on particular hash orders, so
        // run each a few times.
        for _ in 0..4 {
            let sheet = Some(sheet).filter(|s| !s.is_empty());
            run(html, sheet, 480.0, 272.0).unwrap_or_else(|e| panic!("{e}: {html}"));
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn hostile_inline_styles_never_panic(root in arb_node(), w in prop::sample::select(
        vec![0.0f32, 1.0, 320.0, 800.0]
    )) {
        let mut html = String::from("<!doctype html><body>");
        render(&root, &mut html);
        if let Err(e) = run(&html, None, w, 600.0) {
            prop_assert!(false, "{e}\n{html}");
        }
    }

    #[test]
    fn hostile_stylesheets_never_panic(
        root in arb_node(),
        rules in prop::collection::vec(
            (0..TAGS.len(), prop::collection::vec(0..DECLS.len(), 1..5)),
            0..8,
        ),
    ) {
        let mut sheet = String::new();
        for (tag, decls) in &rules {
            sheet.push_str(TAGS[*tag]);
            sheet.push_str(" > * , :is(");
            sheet.push_str(TAGS[*tag]);
            sheet.push_str(") {");
            for d in decls {
                sheet.push_str(DECLS[*d]);
                sheet.push(';');
            }
            sheet.push('}');
        }
        let mut html = String::new();
        render(&root, &mut html);
        if let Err(e) = run(&html, Some(&sheet), 480.0, 272.0) {
            prop_assert!(false, "{e}\n{sheet}\n{html}");
        }
    }

    /// Byte soup straight into the tokenizer / tree builder / CSS parser.
    #[test]
    fn arbitrary_text_never_panics(s in "\\PC{0,400}") {
        let _ = run(&s, Some(&s), 480.0, 272.0);
    }
}
