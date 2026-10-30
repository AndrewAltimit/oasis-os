//! HTML escaping for generated markup (serialization, internal pages,
//! reader mode, Gemini / plain-text conversion).

/// Append `text` to `out` with `&`, `<`, `>` and `"` escaped.
///
/// The result is safe as an HTML text node and inside a double-quoted
/// attribute value, which is everything the engine generates.
pub fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

/// [`push_escaped`] into a new string.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    push_escaped(&mut out, text);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup_and_quotes() {
        assert_eq!(
            escape(r#"<a href="x">Tom & 'Jerry'</a>"#),
            "&lt;a href=&quot;x&quot;&gt;Tom &amp; 'Jerry'&lt;/a&gt;"
        );
        assert_eq!(escape(""), "");
    }
}
