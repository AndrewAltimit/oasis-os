//! Parser for text/gemini documents.
//!
//! Converts raw text/gemini markup into a structured sequence of
//! [`GeminiLine`] variants suitable for rendering or further
//! processing.

/// A parsed line from a text/gemini document.
#[derive(Debug, Clone, PartialEq)]
pub enum GeminiLine {
    /// Regular text paragraph.
    Text(String),
    /// Link: `=> URL optional display text`
    Link {
        url: String,
        display: Option<String>,
    },
    /// Heading level 1: `# text`
    Heading1(String),
    /// Heading level 2: `## text`
    Heading2(String),
    /// Heading level 3: `### text`
    Heading3(String),
    /// Unordered list item: `* text`
    ListItem(String),
    /// Blockquote: `> text`
    Quote(String),
    /// Preformatted text (between ``` markers).
    Preformatted {
        alt_text: String,
        lines: Vec<String>,
    },
    /// Empty line.
    Empty,
}

/// A parsed Gemini document.
#[derive(Debug, Clone)]
pub struct GeminiDocument {
    /// The parsed lines that make up the document.
    pub lines: Vec<GeminiLine>,
}

impl GeminiDocument {
    /// Parse a text/gemini document from raw text.
    pub fn parse(input: &str) -> Self {
        let mut lines = Vec::new();
        let mut in_preformatted = false;
        let mut pre_alt = String::new();
        let mut pre_lines: Vec<String> = Vec::new();

        for line in input.lines() {
            if let Some(rest) = line.strip_prefix("```") {
                if in_preformatted {
                    // End preformatted block.
                    lines.push(GeminiLine::Preformatted {
                        alt_text: pre_alt.clone(),
                        lines: pre_lines.clone(),
                    });
                    pre_lines.clear();
                    pre_alt.clear();
                    in_preformatted = false;
                } else {
                    // Start preformatted block.
                    pre_alt = rest.trim().to_string();
                    in_preformatted = true;
                }
                continue;
            }

            if in_preformatted {
                pre_lines.push(line.to_string());
                continue;
            }

            // Parse regular lines.
            if line.starts_with("=>") {
                lines.push(parse_link_line(line));
            } else if let Some(rest) = line.strip_prefix("### ") {
                lines.push(GeminiLine::Heading3(rest.to_string()));
            } else if let Some(rest) = line.strip_prefix("## ") {
                lines.push(GeminiLine::Heading2(rest.to_string()));
            } else if let Some(rest) = line.strip_prefix("# ") {
                lines.push(GeminiLine::Heading1(rest.to_string()));
            } else if let Some(rest) = line.strip_prefix("* ") {
                lines.push(GeminiLine::ListItem(rest.to_string()));
            } else if let Some(rest) = line.strip_prefix('>') {
                let text = rest.trim_start().to_string();
                lines.push(GeminiLine::Quote(text));
            } else if line.trim().is_empty() {
                lines.push(GeminiLine::Empty);
            } else {
                lines.push(GeminiLine::Text(line.to_string()));
            }
        }

        // Handle unclosed preformatted block.
        if in_preformatted && !pre_lines.is_empty() {
            lines.push(GeminiLine::Preformatted {
                alt_text: pre_alt,
                lines: pre_lines,
            });
        }

        GeminiDocument { lines }
    }

    /// Extract the document title (first heading, if any).
    pub fn title(&self) -> Option<&str> {
        for line in &self.lines {
            match line {
                GeminiLine::Heading1(t) | GeminiLine::Heading2(t) | GeminiLine::Heading3(t) => {
                    return Some(t);
                },
                _ => {},
            }
        }
        None
    }

    /// Extract all links from the document.
    pub fn links(&self) -> Vec<(&str, Option<&str>)> {
        self.lines
            .iter()
            .filter_map(|line| {
                if let GeminiLine::Link { url, display } = line {
                    Some((url.as_str(), display.as_deref()))
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Parse a link line: `=> URL [display text]`
fn parse_link_line(line: &str) -> GeminiLine {
    let rest = line[2..].trim_start();
    if rest.is_empty() {
        return GeminiLine::Link {
            url: String::new(),
            display: None,
        };
    }

    // URL ends at first whitespace.
    let (url, display) = if let Some(pos) = rest.find(|c: char| c.is_whitespace()) {
        let url = rest[..pos].to_string();
        let display = rest[pos..].trim().to_string();
        let display = if display.is_empty() {
            None
        } else {
            Some(display)
        };
        (url, display)
    } else {
        (rest.to_string(), None)
    };

    GeminiLine::Link { url, display }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text_lines() {
        let doc = GeminiDocument::parse("Hello world\nSecond line");
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[0], GeminiLine::Text("Hello world".into()));
        assert_eq!(doc.lines[1], GeminiLine::Text("Second line".into()));
    }

    #[test]
    fn parse_links_with_and_without_display() {
        let input = "=> gemini://example.com/ Example Site\n\
                      => gemini://bare.link/";
        let doc = GeminiDocument::parse(input);
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(
            doc.lines[0],
            GeminiLine::Link {
                url: "gemini://example.com/".into(),
                display: Some("Example Site".into()),
            }
        );
        assert_eq!(
            doc.lines[1],
            GeminiLine::Link {
                url: "gemini://bare.link/".into(),
                display: None,
            }
        );
    }

    #[test]
    fn parse_headings_all_levels() {
        let input = "# Heading 1\n## Heading 2\n### Heading 3";
        let doc = GeminiDocument::parse(input);
        assert_eq!(doc.lines.len(), 3);
        assert_eq!(doc.lines[0], GeminiLine::Heading1("Heading 1".into()));
        assert_eq!(doc.lines[1], GeminiLine::Heading2("Heading 2".into()));
        assert_eq!(doc.lines[2], GeminiLine::Heading3("Heading 3".into()));
    }

    #[test]
    fn parse_list_items() {
        let input = "* First item\n* Second item";
        let doc = GeminiDocument::parse(input);
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[0], GeminiLine::ListItem("First item".into()));
        assert_eq!(doc.lines[1], GeminiLine::ListItem("Second item".into()));
    }

    #[test]
    fn parse_blockquotes() {
        let input = "> This is a quote\n>";
        let doc = GeminiDocument::parse(input);
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[0], GeminiLine::Quote("This is a quote".into()));
        assert_eq!(doc.lines[1], GeminiLine::Quote(String::new()));
    }

    #[test]
    fn parse_preformatted_blocks() {
        let input = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```";
        let doc = GeminiDocument::parse(input);
        assert_eq!(doc.lines.len(), 1);
        assert_eq!(
            doc.lines[0],
            GeminiLine::Preformatted {
                alt_text: "rust".into(),
                lines: vec![
                    "fn main() {".into(),
                    "    println!(\"hi\");".into(),
                    "}".into(),
                ],
            }
        );
    }

    #[test]
    fn parse_empty_lines() {
        let input = "text\n\nmore text";
        let doc = GeminiDocument::parse(input);
        assert_eq!(doc.lines.len(), 3);
        assert_eq!(doc.lines[1], GeminiLine::Empty);
    }

    #[test]
    fn mixed_document_all_line_types() {
        let input = "\
# Welcome
Hello world

=> gemini://example.com/ Visit
* item one
> a quote
```code
print()
```

## Sub heading";
        let doc = GeminiDocument::parse(input);
        assert_eq!(doc.lines.len(), 9);
        assert!(matches!(doc.lines[0], GeminiLine::Heading1(_)));
        assert!(matches!(doc.lines[1], GeminiLine::Text(_)));
        assert!(matches!(doc.lines[2], GeminiLine::Empty));
        assert!(matches!(doc.lines[3], GeminiLine::Link { .. }));
        assert!(matches!(doc.lines[4], GeminiLine::ListItem(_)));
        assert!(matches!(doc.lines[5], GeminiLine::Quote(_)));
        assert!(matches!(doc.lines[6], GeminiLine::Preformatted { .. }));
        assert!(matches!(doc.lines[7], GeminiLine::Empty));
        assert!(matches!(doc.lines[8], GeminiLine::Heading2(_)));
    }

    #[test]
    fn extract_title() {
        let doc = GeminiDocument::parse("Hello\n# My Title\nMore");
        assert_eq!(doc.title(), Some("My Title"));
    }

    #[test]
    fn extract_title_none_when_no_headings() {
        let doc = GeminiDocument::parse("Just text here.");
        assert_eq!(doc.title(), None);
    }

    #[test]
    fn extract_links() {
        let input = "Text\n=> /a Link A\n=> /b\nMore text";
        let doc = GeminiDocument::parse(input);
        let links = doc.links();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0], ("/a", Some("Link A")));
        assert_eq!(links[1], ("/b", None));
    }
}
