//! Gemini pages and reader mode.

use super::*;

impl BrowserWidget {
    /// Load and render a Gemini document.
    pub fn load_gemini(&mut self, source: &str, url: &str) {
        let doc = gemini::parser::GeminiDocument::parse(source);
        let title = doc.title().unwrap_or("Gemini page").to_string();

        // Convert to HTML and render through the HTML pipeline.
        let html = gemini_to_html(&doc);
        self.load_html(&html, url);

        // Override the title with the Gemini document title.
        self.nav.update_title(&title);
    }

    /// Toggle reader mode on/off.
    pub fn toggle_reader_mode(&mut self) {
        if !self.config.features.reader_mode {
            return;
        }

        self.reader_mode = !self.reader_mode;
        self.nav.update_reader_mode(self.reader_mode);

        if self.reader_mode {
            // Extract article and re-render.
            if let Some(doc) = &self.document
                && let Some(article) = reader::extract_article(doc)
            {
                self.reader_html = Some(article.html.clone());
                // Re-parse the reader HTML.
                let url = self.nav.current_url().unwrap_or("about:reader").to_string();
                let tokens = html::tokenizer::Tokenizer::new(&article.html).tokenize();
                let reader_doc = html::tree_builder::TreeBuilder::build(tokens);
                let ua_sheet = css::default::default_stylesheet();
                let reader_ctx = css::cascade::CascadeContext {
                    hover_node: None,
                    visited_urls: Some(&self.visited_urls),
                    focused_node: None,
                    containers: None,
                    global_layers: None,
                };
                let styles = css::cascade::style_tree(&reader_doc, &[ua_sheet], &[], &reader_ctx);
                let href_map = Self::build_link_map(&reader_doc);
                self.cached_author_sheets = Vec::new();
                self.cached_author_sheet_positions = Vec::new();
                self.cached_inline_styles = Vec::new();
                self.external_stylesheets = Vec::new();
                self.external_stylesheet_positions = Vec::new();
                self.clear_node_references();
                self.document = Some(reader_doc);
                self.styles = styles;
                self.href_map = href_map;
                self.layout_root = None;
                self.link_map.clear();
                self.scroll.reset();
                self.selected_link = -1;
                self.nav.update_title(&format!("Reader: {}", article.title));
                let _ = url; // suppress unused warning
            }
        } else {
            // Restore original page by re-navigating.
            self.reader_html = None;
            if let Some(url) = self.nav.current_url() {
                let url = url.to_string();
                // Re-parse original from cache if available.
                if let Some(entry) = self.cache.get(&url) {
                    let body = entry.response.body.clone();
                    let ct = entry.response.content_type;
                    if ct == ContentType::Html || ct == ContentType::PlainText {
                        let text = html_for_text_body(ct, &body);
                        self.load_html(&text, &url);
                    }
                }
            }
        }
    }
}

/// Convert a parsed Gemini document to HTML for
/// rendering through the HTML pipeline.
fn gemini_to_html(doc: &gemini::parser::GeminiDocument) -> String {
    let mut html = String::from("<html><head><title>");
    if let Some(title) = doc.title() {
        html.push_str(title);
    }
    html.push_str("</title></head><body>");

    for line in &doc.lines {
        match line {
            gemini::parser::GeminiLine::Text(text) => {
                html.push_str("<p>");
                push_escaped(&mut html, text);
                html.push_str("</p>");
            },
            gemini::parser::GeminiLine::Link { url, display } => {
                html.push_str("<p><a href=\"");
                push_escaped(&mut html, url);
                html.push_str("\">");
                let label = display.as_deref().unwrap_or(url.as_str());
                push_escaped(&mut html, label);
                html.push_str("</a></p>");
            },
            gemini::parser::GeminiLine::Heading1(text) => {
                html.push_str("<h1>");
                push_escaped(&mut html, text);
                html.push_str("</h1>");
            },
            gemini::parser::GeminiLine::Heading2(text) => {
                html.push_str("<h2>");
                push_escaped(&mut html, text);
                html.push_str("</h2>");
            },
            gemini::parser::GeminiLine::Heading3(text) => {
                html.push_str("<h3>");
                push_escaped(&mut html, text);
                html.push_str("</h3>");
            },
            gemini::parser::GeminiLine::ListItem(text) => {
                html.push_str("<li>");
                push_escaped(&mut html, text);
                html.push_str("</li>");
            },
            gemini::parser::GeminiLine::Quote(text) => {
                html.push_str("<blockquote>");
                push_escaped(&mut html, text);
                html.push_str("</blockquote>");
            },
            gemini::parser::GeminiLine::Preformatted { lines, alt_text: _ } => {
                html.push_str("<pre>");
                for (i, pre_line) in lines.iter().enumerate() {
                    if i > 0 {
                        html.push('\n');
                    }
                    push_escaped(&mut html, pre_line);
                }
                html.push_str("</pre>");
            },
            gemini::parser::GeminiLine::Empty => {
                html.push_str("<br>");
            },
        }
    }

    html.push_str("</body></html>");
    html
}
