//! Gemini protocol framing: status codes, response parsing, requests.
//!
//! Gemini is a lightweight protocol with mandatory TLS, single-line
//! requests, and a simple text-based content format (text/gemini).
//! The transport lives in [`crate::gemini_fetch`]; the `text/gemini`
//! document parser and renderer live in `oasis-browser`'s `gemini`
//! module, which re-exports these items.

/// Gemini response status codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeminiStatus {
    /// 10-19: request input from user.
    Input(u8),
    /// 20-29: success, body follows.
    Success(u8),
    /// 30-39: redirect to new URL.
    Redirect(u8),
    /// 40-49: temporary failure.
    TemporaryFailure(u8),
    /// 50-59: permanent failure.
    PermanentFailure(u8),
    /// 60-69: client certificate required.
    ClientCert(u8),
}

impl GeminiStatus {
    /// Classify a numeric status code into the appropriate variant.
    pub fn from_code(code: u8) -> Self {
        match code {
            10..=19 => GeminiStatus::Input(code),
            20..=29 => GeminiStatus::Success(code),
            30..=39 => GeminiStatus::Redirect(code),
            40..=49 => GeminiStatus::TemporaryFailure(code),
            50..=59 => GeminiStatus::PermanentFailure(code),
            60..=69 => GeminiStatus::ClientCert(code),
            _ => GeminiStatus::PermanentFailure(59),
        }
    }

    /// Returns `true` for status codes in the 20-29 (success) range.
    pub fn is_success(&self) -> bool {
        matches!(self, GeminiStatus::Success(_))
    }

    /// Returns `true` for status codes in the 30-39 (redirect) range.
    pub fn is_redirect(&self) -> bool {
        matches!(self, GeminiStatus::Redirect(_))
    }
}

/// A parsed Gemini response.
#[derive(Debug, Clone)]
pub struct GeminiResponse {
    /// Status code category.
    pub status: GeminiStatus,
    /// MIME type for success, URL for redirect, message for errors.
    pub meta: String,
    /// Response body (present only on success).
    pub body: Option<String>,
}

/// Parse a raw Gemini response (status line + optional body).
///
/// The response format is: `<STATUS><SPACE><META>\r\n[body]`
/// where STATUS is a two-digit code.
pub fn parse_response(data: &[u8]) -> Option<GeminiResponse> {
    let text = std::str::from_utf8(data).ok()?;
    let first_line_end = text.find("\r\n")?;
    let status_line = &text[..first_line_end];

    // `get`, not indexing: a non-ASCII status line must not panic on a
    // char boundary.
    let code = status_line.get(..2)?;
    if !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let code: u8 = code.parse().ok()?;
    let meta = status_line
        .get(2..)
        .map(|m| m.strip_prefix(' ').unwrap_or(m))
        .unwrap_or_default()
        .to_string();

    let status = GeminiStatus::from_code(code);

    let body = if status.is_success() && text.len() > first_line_end + 2 {
        Some(text[first_line_end + 2..].to_string())
    } else {
        None
    };

    Some(GeminiResponse { status, meta, body })
}

/// Build a Gemini request string (just the URL terminated by CRLF).
pub fn build_request(url: &str) -> Vec<u8> {
    format!("{}\r\n", url).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_success_response() {
        let data = b"20 text/gemini\r\n# Hello\r\nWorld";
        let resp = parse_response(data).unwrap();
        assert_eq!(resp.status, GeminiStatus::Success(20));
        assert_eq!(resp.meta, "text/gemini");
        assert_eq!(resp.body.as_deref(), Some("# Hello\r\nWorld"));
    }

    #[test]
    fn parse_redirect_response() {
        let data = b"31 gemini://example.com/new\r\n";
        let resp = parse_response(data).unwrap();
        assert!(resp.status.is_redirect());
        assert_eq!(resp.meta, "gemini://example.com/new");
        assert!(resp.body.is_none());
    }

    #[test]
    fn parse_error_response() {
        let data = b"51 Not found\r\n";
        let resp = parse_response(data).unwrap();
        assert_eq!(resp.status, GeminiStatus::PermanentFailure(51));
        assert_eq!(resp.meta, "Not found");
        assert!(resp.body.is_none());
    }

    /// Regression: the status line was byte-sliced (`[..2]`, `[3..]`),
    /// panicking when a multi-byte character straddled either cut.
    #[test]
    fn non_ascii_status_line_does_not_panic() {
        for raw in [
            "aé\r\n",
            "é\r\n",
            "20é\r\n",
            "2é x\r\n",
            "+1 x\r\n",
            "2\r\n",
        ] {
            let resp = parse_response(raw.as_bytes());
            assert!(resp.is_none() || raw.starts_with("20"), "{raw:?}");
        }
        let resp = parse_response("20 tëxt/gemini\r\nbody".as_bytes()).unwrap();
        assert_eq!(resp.meta, "tëxt/gemini");
        assert_eq!(parse_response(b"20\r\n").unwrap().meta, "");
    }

    #[test]
    fn build_request_string() {
        let req = build_request("gemini://example.com/");
        assert_eq!(req, b"gemini://example.com/\r\n");
    }
}
