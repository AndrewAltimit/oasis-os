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
    ///
    /// Both directions rebuild the page through [`Self::load_html`]
    /// (without a history entry), so every per-document cache — the
    /// selector index over the page's author sheets, container lookups,
    /// external sheets, the script engine, forms — is replaced together.
    /// Swapping in only the reader DOM left the selector index of the
    /// original page in place, and the next hover restyle indexed sheets
    /// that no longer existed.
    pub fn toggle_reader_mode(&mut self) {
        if !self.config.features.reader_mode {
            return;
        }

        self.reader_mode = !self.reader_mode;
        self.nav.update_reader_mode(self.reader_mode);

        let url = self.nav.current_url().unwrap_or("about:reader").to_string();
        if self.reader_mode {
            if let Some(article) = self.document.as_ref().and_then(reader::extract_article) {
                self.reader_html = Some(article.html.clone());
                self.skip_nav_push = true;
                // The reader page is ours (an element whitelist, no
                // scripts): the page's CSP must not block its `<style>`.
                self.reader_saved_csp = self.page_csp.take();
                self.load_html(&article.html, &url);
                self.selected_link = -1;
                self.nav.update_title(&format!("Reader: {}", article.title));
            }
        } else {
            // Restore the original page (and its CSP) from the cache.
            self.reader_html = None;
            if let Some(csp) = self.reader_saved_csp.take() {
                self.page_csp = Some(csp);
            }
            if let Some(entry) = self.cache.get(&url) {
                let body = entry.response.body.clone();
                let ct = entry.response.content_type;
                if ct == ContentType::Html || ct == ContentType::PlainText {
                    let text = html_for_text_body(ct, &body);
                    self.skip_nav_push = true;
                    self.load_html(&text, &url);
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
        // Escaped like every other line: a `# </title><script>` heading
        // must not become markup.
        push_escaped(&mut html, title);
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
