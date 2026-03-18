//! User-agent default stylesheet.
//!
//! Contains the CSS 2.1 default rules for HTML elements. This is the
//! baseline stylesheet that all documents start with before any author
//! or skin stylesheets are applied.

use std::sync::LazyLock;

use super::parser::Stylesheet;

/// Cached UA stylesheet for builds with JavaScript enabled.
#[cfg(feature = "javascript")]
static UA_SHEET_JS: LazyLock<Stylesheet> = LazyLock::new(|| Stylesheet::parse(UA_CSS));

/// Cached UA stylesheet for builds without JavaScript.
#[cfg(not(feature = "javascript"))]
static UA_SHEET_NO_JS: LazyLock<Stylesheet> = LazyLock::new(|| {
    let css = format!("{UA_CSS}\nnoscript {{ display: block !important; }}");
    Stylesheet::parse(&css)
});

/// Get the user-agent default stylesheet.
///
/// Returns a reference to a lazily-initialized static stylesheet,
/// avoiding re-parsing on every navigation and hover restyle.
pub fn default_stylesheet() -> &'static Stylesheet {
    #[cfg(feature = "javascript")]
    {
        &UA_SHEET_JS
    }
    #[cfg(not(feature = "javascript"))]
    {
        &UA_SHEET_NO_JS
    }
}

/// User-agent stylesheet following CSS 2.1 defaults with visual styling
/// for semantic elements.
const UA_CSS: &str = r#"
/* -- Block-level elements ------------------------------------------- */
html, body, div, main, section, article, nav, aside,
header, footer, figure, figcaption, address, fieldset, form,
hgroup, search, dialog {
    display: block;
}

body {
    margin: 8px;
}

/* -- Paragraphs ----------------------------------------------------- */
p {
    display: block;
    margin-top: 1em;
    margin-bottom: 1em;
}

/* -- Headings ------------------------------------------------------- */
h1 {
    display: block;
    font-size: 2em;
    font-weight: bold;
    margin-top: 0.67em;
    margin-bottom: 0.67em;
}
h2 {
    display: block;
    font-size: 1.5em;
    font-weight: bold;
    margin-top: 0.83em;
    margin-bottom: 0.83em;
}
h3 {
    display: block;
    font-size: 1.17em;
    font-weight: bold;
    margin-top: 1em;
    margin-bottom: 1em;
}
h4 {
    display: block;
    font-size: 1em;
    font-weight: bold;
    margin-top: 1.33em;
    margin-bottom: 1.33em;
}
h5 {
    display: block;
    font-size: 0.83em;
    font-weight: bold;
    margin-top: 1.67em;
    margin-bottom: 1.67em;
}
h6 {
    display: block;
    font-size: 0.67em;
    font-weight: bold;
    margin-top: 2.33em;
    margin-bottom: 2.33em;
}

/* -- Lists ---------------------------------------------------------- */
ul, ol {
    display: block;
    margin-top: 1em;
    margin-bottom: 1em;
    padding-left: 40px;
}
ul li {
    display: list-item;
    list-style-type: disc;
}
ol li {
    display: list-item;
    list-style-type: decimal;
}
li {
    display: list-item;
}
dl {
    display: block;
    margin-top: 1em;
    margin-bottom: 1em;
}
dt { display: block; font-weight: bold; }
dd { display: block; margin-left: 40px; }

/* -- Blockquote ----------------------------------------------------- */
blockquote {
    display: block;
    margin-top: 1em;
    margin-bottom: 1em;
    margin-left: 10px;
    padding-left: 10px;
    border-left-width: 3px;
    border-left-style: solid;
    border-left-color: #808080;
}

/* -- Preformatted & Code -------------------------------------------- */
pre {
    display: block;
    white-space: pre;
    font-family: monospace;
    margin-top: 1em;
    margin-bottom: 1em;
    padding-top: 6px;
    padding-bottom: 6px;
    padding-left: 8px;
    padding-right: 8px;
    background-color: rgba(128, 128, 128, 25);
    border-width: 1px;
    border-style: solid;
    border-color: rgba(128, 128, 128, 50);
}
code, kbd, samp, var {
    font-family: monospace;
    background-color: rgba(128, 128, 128, 25);
}

/* -- Inline text semantics ------------------------------------------ */
mark {
    background-color: rgba(255, 255, 0, 128);
    color: #000000;
}
small { font-size: 0.83em; }
sub { font-size: 0.83em; }
sup { font-size: 0.83em; }
abbr { text-decoration: underline; }
dfn { font-style: italic; }

/* -- Details/Summary ------------------------------------------------ */
details {
    display: block;
    margin-top: 1em;
    margin-bottom: 1em;
}
summary {
    display: block;
    font-weight: bold;
}
summary::before {
    content: "▶ ";
}
details[open] > summary::before {
    content: "▼ ";
}

/* -- Horizontal rule ------------------------------------------------ */
hr {
    display: block;
    margin-top: 8px;
    margin-bottom: 8px;
    border-top-width: 1px;
    border-top-style: solid;
    border-top-color: #808080;
}

/* -- Text formatting ------------------------------------------------ */
b, strong { font-weight: bold; }
i, em, cite { font-style: italic; }
u, ins { text-decoration: underline; }
s, del { text-decoration: line-through; }

/* -- Links ---------------------------------------------------------- */
a {
    color: #0066cc;
    text-decoration: underline;
}

/* -- Tables --------------------------------------------------------- */
table {
    display: table;
    border-collapse: collapse;
}
caption {
    display: block;
    text-align: center;
    font-weight: bold;
    padding-top: 4px;
    padding-bottom: 4px;
}
thead { display: block; }
tbody { display: block; }
tfoot { display: block; }
colgroup { display: none; }
col { display: none; }
tr { display: table-row; }
td, th {
    display: table-cell;
    padding-top: 2px;
    padding-bottom: 2px;
    padding-left: 4px;
    padding-right: 4px;
    border-top-width: 1px;
    border-right-width: 1px;
    border-bottom-width: 1px;
    border-left-width: 1px;
    border-top-style: solid;
    border-right-style: solid;
    border-bottom-style: solid;
    border-left-style: solid;
    border-top-color: #ccc;
    border-right-color: #ccc;
    border-bottom-color: #ccc;
    border-left-color: #ccc;
}
th {
    font-weight: bold;
    text-align: center;
}

/* -- Form elements -------------------------------------------------- */
br, img, input, button, select, textarea {
    display: inline;
}
option {
    display: none;
}
fieldset {
    display: block;
    margin-top: 0;
    margin-bottom: 0;
    padding-top: 4px;
    padding-bottom: 4px;
    padding-left: 8px;
    padding-right: 8px;
    border-width: 1px;
    border-style: solid;
    border-color: #808080;
}
legend {
    display: block;
    font-weight: bold;
    padding-left: 4px;
    padding-right: 4px;
}
label { display: inline; }

/* -- Deprecated elements -------------------------------------------- */
center {
    display: block;
    text-align: center;
}

/* -- Hidden elements ------------------------------------------------ */
head, script, style, link, meta, title, noscript, template {
    display: none;
}

input[type="hidden"] {
    display: none;
}

/* -- Focus ring ----------------------------------------------------- */
:focus {
    outline-width: 2px;
    outline-style: solid;
    outline-color: #0066cc;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ua_stylesheet_parses_without_error() {
        let ss = default_stylesheet();
        assert!(!ss.rules.is_empty());
    }

    #[test]
    fn ua_stylesheet_has_body_margin() {
        let ss = default_stylesheet();
        let body_rule = ss.rules.iter().find(|r| {
            r.selectors.selectors.iter().any(|sel| {
                sel.parts.iter().any(|(compound, _)| {
                    compound.parts.iter().any(|simple| {
                        matches!(
                            simple,
                            super::super::parser::SimpleSelector::Type(t)
                                if t == "body"
                        )
                    })
                })
            })
        });
        assert!(body_rule.is_some(), "UA stylesheet should have body rule");
    }

    #[test]
    fn ua_stylesheet_hides_head() {
        let ss = default_stylesheet();
        let head_rule = ss.rules.iter().find(|r| {
            r.selectors.selectors.iter().any(|sel| {
                sel.parts.iter().any(|(compound, _)| {
                    compound.parts.iter().any(|simple| {
                        matches!(
                            simple,
                            super::super::parser::SimpleSelector::Type(t)
                                if t == "head"
                        )
                    })
                })
            })
        });
        assert!(head_rule.is_some(), "UA stylesheet should have head rule");
        let decls = &head_rule.unwrap().declarations;
        assert!(
            decls.iter().any(|d| d.property == "display"
                && matches!(
                    &d.value,
                    super::super::parser::CssValue::Keyword(k)
                        if k == "none"
                )),
            "head should have display:none"
        );
    }

    #[test]
    fn ua_stylesheet_sets_h1_font_size() {
        let ss = default_stylesheet();
        let h1_rule = ss.rules.iter().find(|r| {
            r.selectors.selectors.iter().any(|sel| {
                sel.parts.iter().any(|(compound, _)| {
                    compound.parts.iter().any(|simple| {
                        matches!(
                            simple,
                            super::super::parser::SimpleSelector::Type(t)
                                if t == "h1"
                        )
                    })
                })
            })
        });
        assert!(h1_rule.is_some(), "UA stylesheet should have h1 rule");
        let decls = &h1_rule.unwrap().declarations;
        assert!(
            decls.iter().any(|d| d.property == "font-size"),
            "h1 should set font-size"
        );
    }

    #[test]
    fn ua_stylesheet_table_display() {
        let ss = default_stylesheet();
        let table_rule = ss.rules.iter().find(|r| {
            r.selectors.selectors.iter().any(|sel| {
                sel.parts.len() == 1
                    && sel.parts[0].0.parts.iter().any(|simple| {
                        matches!(
                            simple,
                            super::super::parser::SimpleSelector::Type(t)
                                if t == "table"
                        )
                    })
            })
        });
        assert!(table_rule.is_some(), "UA stylesheet should have table rule");
    }
}
