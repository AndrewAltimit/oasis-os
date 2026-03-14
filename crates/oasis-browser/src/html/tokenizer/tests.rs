//! Tests for the HTML tokenizer.

#[cfg(test)]
mod tests {
    use crate::html::tokenizer::*;

    /// Helper: tokenize and strip the trailing Eof.
    fn tok(input: &str) -> Vec<Token> {
        let mut t = Tokenizer::new(input);
        let mut tokens = t.tokenize();
        if matches!(tokens.last(), Some(Token::Eof)) {
            tokens.pop();
        }
        tokens
    }

    // -- basic tags ---------------------------------------------------------

    #[test]
    fn basic_paragraph() {
        let tokens = tok("<p>Hello</p>");
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "p".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[1], Token::Character("Hello".into()));
        assert_eq!(tokens[2], Token::EndTag(EndTagToken { name: "p".into() }));
    }

    #[test]
    fn self_closing_br() {
        let tokens = tok("<br/>");
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "br".into(),
                attributes: vec![],
                self_closing: true,
            })
        );
    }

    #[test]
    fn self_closing_img_with_attr() {
        let tokens = tok(r#"<img src="test.png"/>"#);
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "img".into(),
                attributes: vec![Attribute {
                    name: "src".into(),
                    value: "test.png".into(),
                }],
                self_closing: true,
            })
        );
    }

    // -- attributes ---------------------------------------------------------

    #[test]
    fn double_quoted_attribute() {
        let tokens = tok(r#"<a href="http://example.com">link</a>"#);
        assert_eq!(tokens.len(), 3);
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.name, "a");
        assert_eq!(tag.attributes.len(), 1);
        assert_eq!(tag.attributes[0].name, "href");
        assert_eq!(tag.attributes[0].value, "http://example.com");
    }

    #[test]
    fn single_quoted_attribute() {
        let tokens = tok("<div class='main'>");
        assert_eq!(tokens.len(), 1);
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.attributes[0].name, "class");
        assert_eq!(tag.attributes[0].value, "main");
    }

    #[test]
    fn unquoted_attribute() {
        let tokens = tok("<input type=text>");
        assert_eq!(tokens.len(), 1);
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.attributes[0].name, "type");
        assert_eq!(tag.attributes[0].value, "text");
    }

    #[test]
    fn boolean_attribute() {
        let tokens = tok("<input disabled>");
        assert_eq!(tokens.len(), 1);
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.attributes.len(), 1);
        assert_eq!(tag.attributes[0].name, "disabled");
        assert_eq!(tag.attributes[0].value, "");
    }

    #[test]
    fn multiple_attributes() {
        let tokens = tok(r#"<input type="text" name="q" value="search">"#);
        assert_eq!(tokens.len(), 1);
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.attributes.len(), 3);
        assert_eq!(tag.attributes[0].name, "type");
        assert_eq!(tag.attributes[0].value, "text");
        assert_eq!(tag.attributes[1].name, "name");
        assert_eq!(tag.attributes[1].value, "q");
        assert_eq!(tag.attributes[2].name, "value");
        assert_eq!(tag.attributes[2].value, "search");
    }

    // -- character references -----------------------------------------------

    #[test]
    fn named_char_ref_amp() {
        let tokens = tok("a&amp;b");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("a&b".into()));
    }

    #[test]
    fn named_char_ref_lt_gt() {
        let tokens = tok("&lt;div&gt;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("<div>".into()));
    }

    #[test]
    fn decimal_char_ref() {
        let tokens = tok("&#60;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("<".into()));
    }

    #[test]
    fn hex_char_ref_lower() {
        let tokens = tok("&#x3c;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("<".into()));
    }

    #[test]
    fn hex_char_ref_upper() {
        let tokens = tok("&#x3C;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("<".into()));
    }

    #[test]
    fn char_ref_in_attribute() {
        let tokens = tok(r#"<a href="?a=1&amp;b=2">x</a>"#);
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.attributes[0].value, "?a=1&b=2");
    }

    #[test]
    fn named_char_ref_nbsp() {
        let tokens = tok("hello&nbsp;world");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("hello\u{00A0}world".into()));
    }

    // -- comments -----------------------------------------------------------

    #[test]
    fn basic_comment() {
        let tokens = tok("<!-- comment -->");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Comment(" comment ".into()));
    }

    #[test]
    fn empty_comment() {
        let tokens = tok("<!---->");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Comment("".into()));
    }

    #[test]
    fn comment_with_dashes() {
        let tokens = tok("<!-- a -- b -->");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Comment(" a -- b ".into()));
    }

    // -- doctype ------------------------------------------------------------

    #[test]
    fn doctype_html() {
        let tokens = tok("<!DOCTYPE html>");
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::Doctype(DoctypeToken {
                name: Some("html".into()),
                force_quirks: false,
            })
        );
    }

    #[test]
    fn doctype_case_insensitive() {
        let tokens = tok("<!doctype HTML>");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(&tokens[0], Token::Doctype(_)), "expected doctype");
        let Token::Doctype(dt) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(dt.name, Some("html".into()));
    }

    // -- nested tags --------------------------------------------------------

    #[test]
    fn nested_tags() {
        let tokens = tok("<div><p>text</p></div>");
        assert_eq!(tokens.len(), 5);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "div".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(
            tokens[1],
            Token::StartTag(StartTagToken {
                name: "p".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[2], Token::Character("text".into()));
        assert_eq!(tokens[3], Token::EndTag(EndTagToken { name: "p".into() }));
        assert_eq!(tokens[4], Token::EndTag(EndTagToken { name: "div".into() }));
    }

    // -- malformed input ----------------------------------------------------

    #[test]
    fn unclosed_tag() {
        let tokens = tok("<p>hello");
        assert_eq!(tokens.len(), 2);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "p".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[1], Token::Character("hello".into()));
    }

    #[test]
    fn bare_less_than() {
        let tokens = tok("a < b");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("a < b".into()));
    }

    #[test]
    fn empty_input() {
        let tokens = tok("");
        assert!(tokens.is_empty());
    }

    // -- script content (RAWTEXT) -------------------------------------------

    #[test]
    fn script_content() {
        let tokens = tok("<script>var x = 1 < 2;</script>");
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "script".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[1], Token::Character("var x = 1 < 2;".into()));
        assert_eq!(
            tokens[2],
            Token::EndTag(EndTagToken {
                name: "script".into(),
            })
        );
    }

    #[test]
    fn style_content() {
        let tokens = tok("<style>body { color: red; }</style>");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[1], Token::Character("body { color: red; }".into()));
    }

    // -- RCDATA -------------------------------------------------------------

    #[test]
    fn title_with_char_ref() {
        let tokens = tok("<title>Page &amp; Title</title>");
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "title".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[1], Token::Character("Page & Title".into()));
        assert_eq!(
            tokens[2],
            Token::EndTag(EndTagToken {
                name: "title".into(),
            })
        );
    }

    #[test]
    fn textarea_rcdata() {
        let tokens = tok("<textarea>some &lt;text&gt;</textarea>");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[1], Token::Character("some <text>".into()));
    }

    // -- mixed content ------------------------------------------------------

    #[test]
    fn mixed_content() {
        // "Hello <b>world</b> and <i>friends</i>!" produces 9 tokens:
        // Character, StartTag, Character, EndTag, Character, StartTag,
        // Character, EndTag, Character.
        let tokens = tok("Hello <b>world</b> and <i>friends</i>!");
        assert_eq!(tokens.len(), 9);
        assert_eq!(tokens[0], Token::Character("Hello ".into()));
        assert_eq!(
            tokens[1],
            Token::StartTag(StartTagToken {
                name: "b".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[2], Token::Character("world".into()));
        assert_eq!(tokens[3], Token::EndTag(EndTagToken { name: "b".into() }));
        assert_eq!(tokens[4], Token::Character(" and ".into()));
        assert_eq!(
            tokens[5],
            Token::StartTag(StartTagToken {
                name: "i".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[6], Token::Character("friends".into()));
        assert_eq!(tokens[7], Token::EndTag(EndTagToken { name: "i".into() }));
        assert_eq!(tokens[8], Token::Character("!".into()));
    }

    // -- void elements ------------------------------------------------------

    #[test]
    fn void_elements() {
        let tokens = tok("<br><hr><img src=\"a.png\">");
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "br".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(
            tokens[1],
            Token::StartTag(StartTagToken {
                name: "hr".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(
            tokens[2],
            Token::StartTag(StartTagToken {
                name: "img".into(),
                attributes: vec![Attribute {
                    name: "src".into(),
                    value: "a.png".into(),
                }],
                self_closing: false,
            })
        );
    }

    // -- full document ------------------------------------------------------

    #[test]
    fn full_document() {
        let html = concat!(
            "<!DOCTYPE html>",
            "<html><head><title>Test</title></head>",
            "<body><p>Hello</p></body></html>",
        );
        let tokens = tok(html);
        // DOCTYPE, <html>, <head>, <title>, "Test",
        // </title>, </head>, <body>, <p>, "Hello",
        // </p>, </body>, </html>
        assert_eq!(tokens.len(), 13);
        assert_eq!(
            tokens[0],
            Token::Doctype(DoctypeToken {
                name: Some("html".into()),
                force_quirks: false,
            })
        );
    }

    // -- edge cases ---------------------------------------------------------

    #[test]
    fn tag_name_case_insensitive() {
        let tokens = tok("<DIV>x</DIV>");
        assert_eq!(tokens.len(), 3);
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.name, "div");
        assert!(matches!(&tokens[2], Token::EndTag(_)), "expected end tag");
        let Token::EndTag(tag) = &tokens[2] else {
            unreachable!()
        };
        assert_eq!(tag.name, "div");
    }

    #[test]
    fn attribute_name_case_insensitive() {
        let tokens = tok(r#"<div CLASS="x">"#);
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.attributes[0].name, "class");
    }

    #[test]
    fn bogus_comment_from_question_mark() {
        let tokens = tok("<?xml version=\"1.0\"?>");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(&tokens[0], Token::Comment(_)), "expected comment");
        let Token::Comment(_) = &tokens[0] else {
            unreachable!()
        };
        // Good -- treated as bogus comment.
    }

    #[test]
    fn self_closing_with_space() {
        let tokens = tok("<br />");
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "br".into(),
                attributes: vec![],
                self_closing: true,
            })
        );
    }

    #[test]
    fn unknown_entity_passthrough() {
        let tokens = tok("&foobar;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("&foobar;".into()));
    }

    #[test]
    fn numeric_ref_zero_becomes_replacement() {
        let tokens = tok("&#0;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("\u{FFFD}".into()));
    }

    #[test]
    fn multiple_char_refs_coalesce() {
        let tokens = tok("&lt;&gt;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("<>".into()));
    }

    #[test]
    fn script_with_attributes() {
        let tokens = tok(r#"<script type="text/javascript">alert(1)</script>"#);
        assert_eq!(tokens.len(), 3);
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.name, "script");
        assert_eq!(tag.attributes[0].name, "type");
        assert_eq!(tag.attributes[0].value, "text/javascript");
        assert_eq!(tokens[1], Token::Character("alert(1)".into()));
    }

    #[test]
    fn bare_ampersand_not_ref() {
        let tokens = tok("a & b");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("a & b".into()));
    }

    // -- robustness / edge cases ----------------------------------------

    #[test]
    fn unclosed_tag_at_eof() {
        // Unclosed start tag should not panic; tokenizer may drop it.
        let tokens = tok("<div");
        let _ = tokens;
    }

    #[test]
    fn unclosed_attribute_value_at_eof() {
        // Incomplete attribute value at EOF should not panic.
        let tokens = tok(r#"<div class="open"#);
        let _ = tokens;
    }

    #[test]
    fn deeply_nested_tags() {
        // 200 levels of nesting -- tokenizer should handle without stack overflow.
        let open: String = (0..200).map(|_| "<div>").collect();
        let close: String = (0..200).map(|_| "</div>").collect();
        let html = format!("{open}leaf{close}");
        let tokens = tok(&html);
        assert!(tokens.len() >= 401); // 200 open + 1 text + 200 close
    }

    #[test]
    fn very_long_attribute_value() {
        let val = "x".repeat(10_000);
        let html = format!(r#"<div data="{val}"></div>"#);
        let tokens = tok(&html);
        assert_eq!(tokens.len(), 2); // start + end
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.attributes[0].value.len(), 10_000);
    }

    #[test]
    fn very_long_tag_name() {
        let name = "a".repeat(5_000);
        let html = format!("<{name}>text</{name}>");
        let tokens = tok(&html);
        assert_eq!(tokens.len(), 3);
    }

    #[test]
    fn extremely_long_text_content() {
        let text = "w ".repeat(50_000);
        let html = format!("<p>{text}</p>");
        let tokens = tok(&html);
        assert!(tokens.len() >= 2);
    }

    #[test]
    fn null_bytes_in_content() {
        let tokens = tok("<p>before\0after</p>");
        assert!(!tokens.is_empty());
    }

    #[test]
    fn null_bytes_in_tag_name() {
        let tokens = tok("<di\0v>text</di\0v>");
        // Should not panic; exact behavior is implementation-defined.
        assert!(!tokens.is_empty());
    }

    #[test]
    fn many_attributes_same_tag() {
        let attrs: String = (0..100).map(|i| format!(r#" a{i}="v{i}""#)).collect();
        let html = format!("<div{attrs}>x</div>");
        let tokens = tok(&html);
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.attributes.len(), 100);
    }

    #[test]
    fn unquoted_attribute_value() {
        let tokens = tok("<div class=foo></div>");
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.attributes[0].value, "foo");
    }

    #[test]
    fn single_quoted_attribute_edge() {
        let tokens = tok("<div class='bar'></div>");
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.attributes[0].value, "bar");
    }

    #[test]
    fn multiple_unclosed_tags() {
        let tokens = tok("<div><span><b>text");
        assert!(tokens.len() >= 4); // 3 opens + text
    }

    #[test]
    fn mismatched_closing_tags() {
        // </span> when only <div> is open -- should not panic.
        let tokens = tok("<div>text</span></div>");
        assert!(!tokens.is_empty());
    }

    #[test]
    fn empty_tag() {
        let tokens = tok("<>text</>");
        // Empty tag names -- should not panic.
        assert!(!tokens.is_empty());
    }

    #[test]
    fn angle_brackets_in_text() {
        let tokens = tok("<p>1 < 2 and 3 > 1</p>");
        assert!(!tokens.is_empty());
    }

    #[test]
    fn only_whitespace() {
        let tokens = tok("   \n\t\r\n   ");
        // May produce character tokens or nothing; should not panic.
        let _ = tokens;
    }

    #[test]
    fn empty_input_edge_case() {
        let tokens = tok("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn cdata_like_content() {
        let tokens = tok("<p><![CDATA[some data]]></p>");
        // Not real CDATA in HTML; should not panic.
        assert!(!tokens.is_empty());
    }

    #[test]
    fn nested_comments() {
        let tokens = tok("<!-- outer <!-- inner --> rest -->");
        // HTML does not support nested comments; should not panic.
        assert!(!tokens.is_empty());
    }

    #[test]
    fn comment_with_many_dashes() {
        let tokens = tok("<!-- -- --- ---->");
        let _ = tokens; // Should not panic.
    }

    #[test]
    fn doctype_token() {
        let tokens = tok("<!DOCTYPE html><html></html>");
        assert!(!tokens.is_empty());
    }

    #[test]
    fn consecutive_self_closing_tags() {
        let tokens = tok("<br/><hr/><img/>");
        assert_eq!(tokens.len(), 3);
    }

    // -- real-world HTML compliance tests --------------------------------

    #[test]
    fn malformed_named_entity_passthrough() {
        // Unknown named entity should be passed through verbatim.
        let tokens = tok("&notareal;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("&notareal;".into()));
    }

    #[test]
    fn numeric_ref_out_of_unicode_range() {
        // &#99999999; is beyond max Unicode codepoint (0x10FFFF).
        // Should produce replacement character U+FFFD.
        let tokens = tok("&#99999999;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("\u{FFFD}".into()));
    }

    #[test]
    fn hex_ref_out_of_unicode_range() {
        // &#x110000; is above U+10FFFF, should produce U+FFFD.
        let tokens = tok("&#x110000;");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("\u{FFFD}".into()));
    }

    #[test]
    fn script_containing_script_string() {
        // Nested "<script>" string inside script content should not
        // start a new script element. Only "</script>" closes it.
        let tokens = tok(r#"<script>var x = "<script>";</script>"#);
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens[0],
            Token::StartTag(StartTagToken {
                name: "script".into(),
                attributes: vec![],
                self_closing: false,
            })
        );
        assert_eq!(tokens[1], Token::Character("var x = \"<script>\";".into()));
        assert_eq!(
            tokens[2],
            Token::EndTag(EndTagToken {
                name: "script".into(),
            })
        );
    }

    #[test]
    fn attribute_value_with_equals_and_ampersand() {
        // Query strings with = and & in attribute values.
        let tokens = tok(r#"<a href="page?a=1&b=2">link</a>"#);
        assert_eq!(tokens.len(), 3);
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.name, "a");
        assert_eq!(tag.attributes[0].name, "href");
        assert_eq!(tag.attributes[0].value, "page?a=1&b=2");
    }

    #[test]
    fn multiple_whitespace_between_attributes() {
        // Excessive whitespace (spaces, tabs, newlines) between attrs.
        let tokens = tok("<div   class=\"a\"  \n\t  id=\"b\"  >");
        assert_eq!(tokens.len(), 1);
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.name, "div");
        assert_eq!(tag.attributes.len(), 2);
        assert_eq!(tag.attributes[0].name, "class");
        assert_eq!(tag.attributes[0].value, "a");
        assert_eq!(tag.attributes[1].name, "id");
        assert_eq!(tag.attributes[1].value, "b");
    }

    #[test]
    fn uppercase_tag_names_lowered() {
        // Mixed-case tags should be normalized to lowercase.
        let tokens = tok("<DIV class=\"test\"><SPAN>x</SPAN></DIV>");
        assert_eq!(tokens.len(), 5);
        assert!(
            matches!(&tokens[0], Token::StartTag(_)),
            "expected div start tag"
        );
        let Token::StartTag(tag) = &tokens[0] else {
            unreachable!()
        };
        assert_eq!(tag.name, "div");
        assert_eq!(tag.attributes[0].value, "test");
        assert!(
            matches!(&tokens[1], Token::StartTag(_)),
            "expected span start tag"
        );
        let Token::StartTag(tag) = &tokens[1] else {
            unreachable!()
        };
        assert_eq!(tag.name, "span");
        assert!(
            matches!(&tokens[3], Token::EndTag(_)),
            "expected span end tag"
        );
        let Token::EndTag(tag) = &tokens[3] else {
            unreachable!()
        };
        assert_eq!(tag.name, "span");
    }

    #[test]
    fn multiple_malformed_entities_in_text() {
        // Mix of valid and invalid entities in one text run.
        let tokens = tok("a&amp;b&fake;c&#60;d");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Character("a&b&fake;c<d".into()));
    }

    #[test]
    fn style_element_rawtext() {
        // Style content should preserve everything, including <tags>.
        let tokens = tok("<style>div > p { color: red; }</style>");
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens[1],
            Token::Character("div > p { color: red; }".into())
        );
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// Tokenizing arbitrary ASCII input never panics.
            #[test]
            fn tokenize_never_panics(input in "[ -~]{0,120}") {
                let mut t = Tokenizer::new(&input);
                let _ = t.tokenize();
            }

            /// Tokenizing arbitrary UTF-8 input never panics.
            #[test]
            fn tokenize_unicode_never_panics(input in ".{0,60}") {
                let mut t = Tokenizer::new(&input);
                let _ = t.tokenize();
            }

            /// A plain text input produces at least one Character token.
            #[test]
            fn plain_text_produces_character_token(
                s in "[a-zA-Z0-9 ]{1,40}",
            ) {
                let mut t = Tokenizer::new(&s);
                let tokens = t.tokenize();
                let has_char = tokens.iter().any(|tok| {
                    matches!(tok, Token::Character(txt) if !txt.is_empty())
                });
                prop_assert!(
                    has_char,
                    "plain text should produce a Character token",
                );
            }

            /// A simple open tag tokenizes to at least one StartTag.
            #[test]
            fn simple_tag_produces_start_tag(
                tag in "[a-z]{1,10}",
            ) {
                let html = format!("<{tag}>");
                let mut t = Tokenizer::new(&html);
                let tokens = t.tokenize();
                let has_start = tokens.iter().any(|tok| match tok {
                    Token::StartTag(st) => st.name == tag,
                    _ => false,
                });
                prop_assert!(
                    has_start,
                    "should produce a StartTag for the tag name",
                );
            }

            /// Matching open/close tags produce both start and end tokens.
            #[test]
            fn matching_tags_produce_both(
                tag in "[a-z]{1,8}",
            ) {
                let html = format!("<{tag}></{tag}>");
                let mut t = Tokenizer::new(&html);
                let tokens = t.tokenize();
                let has_start = tokens.iter().any(|tok| match tok {
                    Token::StartTag(st) => st.name == tag,
                    _ => false,
                });
                let has_end = tokens.iter().any(|tok| match tok {
                    Token::EndTag(et) => et.name == tag,
                    _ => false,
                });
                prop_assert!(has_start, "should have StartTag");
                prop_assert!(has_end, "should have EndTag");
            }

            /// Numeric character references produce Character tokens.
            #[test]
            fn numeric_char_ref(codepoint in 32u32..127) {
                let html = format!("&#{codepoint};");
                let mut t = Tokenizer::new(&html);
                let tokens = t.tokenize();
                let has_char = tokens.iter().any(|tok| {
                    matches!(tok, Token::Character(txt) if !txt.is_empty())
                });
                prop_assert!(
                    has_char,
                    "numeric ref should produce a Character token",
                );
            }

            /// Deeply nested tags don't panic.
            #[test]
            fn deep_nesting_no_panic(depth in 1usize..30) {
                let open: String = (0..depth).map(|_| "<div>").collect();
                let close: String = (0..depth).map(|_| "</div>").collect();
                let html = format!("{open}text{close}");
                let mut t = Tokenizer::new(&html);
                let _ = t.tokenize();
            }

            /// Self-closing tags produce a StartTag with self_closing set.
            #[test]
            fn self_closing_tag(tag in "[a-z]{1,8}") {
                let html = format!("<{tag}/>");
                let mut t = Tokenizer::new(&html);
                let tokens = t.tokenize();
                let has_self_closing = tokens.iter().any(|tok| match tok {
                    Token::StartTag(st) => st.name == tag && st.self_closing,
                    _ => false,
                });
                prop_assert!(
                    has_self_closing,
                    "should produce a self-closing StartTag",
                );
            }
        }
    }
}
