use super::*;

#[test]
fn frames_complete_post() {
    let mut buf = b"POST /mcp HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello".to_vec();
    match try_frame(&mut buf) {
        Framing::Ready(req) => {
            assert_eq!(req.head.method, "POST");
            assert_eq!(req.head.path, "/mcp");
            assert_eq!(req.body, b"hello");
            assert!(req.head.keep_alive);
            assert!(buf.is_empty());
        },
        _ => panic!("expected Ready"),
    }
}

#[test]
fn pending_until_body_complete() {
    // Headers arrive but body is short.
    let mut buf = b"POST /mcp HTTP/1.1\r\nContent-Length: 5\r\n\r\nhel".to_vec();
    assert!(matches!(try_frame(&mut buf), Framing::Pending));
    // Buffer untouched.
    assert_eq!(
        buf.len(),
        b"POST /mcp HTTP/1.1\r\nContent-Length: 5\r\n\r\nhel".len()
    );
    // Rest of the body arrives.
    buf.extend_from_slice(b"lo");
    assert!(matches!(try_frame(&mut buf), Framing::Ready(_)));
}

#[test]
fn pending_until_headers_complete() {
    let mut buf = b"POST /mcp HTTP/1.1\r\nContent-Len".to_vec();
    assert!(matches!(try_frame(&mut buf), Framing::Pending));
}

#[test]
fn parses_bearer_and_connection_close() {
    let mut buf = b"POST /mcp HTTP/1.1\r\nAuthorization: Bearer sekret\r\nConnection: close\r\nContent-Length: 0\r\n\r\n".to_vec();
    match try_frame(&mut buf) {
        Framing::Ready(req) => {
            assert_eq!(req.head.auth_bearer.as_deref(), Some("sekret"));
            assert!(!req.head.keep_alive);
            assert!(req.body.is_empty());
        },
        _ => panic!("expected Ready"),
    }
}

#[test]
fn rejects_chunked() {
    let mut buf = b"POST /mcp HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    assert!(matches!(try_frame(&mut buf), Framing::Error(411)));
}

#[test]
fn two_pipelined_requests() {
    let mut buf = b"POST /mcp HTTP/1.1\r\nContent-Length: 1\r\n\r\naPOST /mcp HTTP/1.1\r\nContent-Length: 1\r\n\r\nb".to_vec();
    let r1 = try_frame(&mut buf);
    assert!(matches!(r1, Framing::Ready(ref r) if r.body == b"a"));
    let r2 = try_frame(&mut buf);
    assert!(matches!(r2, Framing::Ready(ref r) if r.body == b"b"));
}

#[test]
fn rejects_duplicate_content_length() {
    let mut buf = b"POST /mcp HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\na".to_vec();
    assert!(matches!(try_frame(&mut buf), Framing::Error(400)));
}

#[test]
fn rejects_malformed_content_length() {
    for bad in ["abc", "-1", "+5", "1 2", ""] {
        let mut buf = format!("POST /mcp HTTP/1.1\r\nContent-Length: {bad}\r\n\r\n").into_bytes();
        assert!(
            matches!(try_frame(&mut buf), Framing::Error(400)),
            "Content-Length {bad:?} should be rejected"
        );
    }
}

#[test]
fn parses_host_origin_content_type() {
    let mut buf = b"POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:7345\r\n\
        Origin: http://localhost\r\nContent-Type: application/json\r\n\r\n"
        .to_vec();
    match try_frame(&mut buf) {
        Framing::Ready(req) => {
            assert_eq!(req.head.host.as_deref(), Some("127.0.0.1:7345"));
            assert_eq!(req.head.origin.as_deref(), Some("http://localhost"));
            assert_eq!(req.head.content_type.as_deref(), Some("application/json"));
        },
        _ => panic!("expected Ready"),
    }
}

fn expects_continue(head: &[u8]) -> bool {
    parse_head(head).is_ok_and(|h| h.expect_continue)
}

#[test]
fn detects_expect_continue() {
    assert!(expects_continue(
        b"POST /mcp HTTP/1.1\r\nexpect: 100-Continue\r\nContent-Length: 9\r\n\r\n"
    ));
    assert!(!expects_continue(
        b"POST /mcp HTTP/1.1\r\nContent-Length: 9\r\n\r\n"
    ));
    // Only a header line counts, not the request target.
    assert!(!expects_continue(
        b"POST /expect:100-continue HTTP/1.1\r\n\r\n"
    ));
}

fn status_of(raw: &[u8]) -> Option<u16> {
    match try_frame(&mut raw.to_vec()) {
        Framing::Error(code) => Some(code),
        _ => None,
    }
}

#[test]
fn header_cap_holds_when_terminator_arrives_in_one_read() {
    // The 431 check used to run only while `CRLFCRLF` was missing, so a
    // 60 KiB header block delivered whole was accepted.
    let mut raw = b"POST /mcp HTTP/1.1\r\n".to_vec();
    for i in 0..2000 {
        raw.extend_from_slice(format!("X-Pad-{i}: aaaaaaaaaaaaaaaaaaaa\r\n").as_bytes());
    }
    raw.extend_from_slice(b"Content-Length: 0\r\n\r\n");
    assert!(raw.len() > MAX_HEADER_BYTES);
    assert_eq!(status_of(&raw), Some(431));
}

#[test]
fn any_transfer_encoding_is_rejected() {
    for te in ["chunked", "gzip, chunked", "identity", "CHUNKED"] {
        let raw = format!(
            "POST /mcp HTTP/1.1\r\nTransfer-Encoding: {te}\r\nContent-Length: 2\r\n\r\n{{}}"
        );
        assert_eq!(status_of(raw.as_bytes()), Some(411), "{te}");
    }
}

#[test]
fn malformed_header_lines_are_rejected() {
    for bad in [
        "Content-Length : 0",
        " Content-Length: 0",
        "\tfolded continuation",
        "no colon here",
        ": empty name",
    ] {
        let raw = format!("POST /mcp HTTP/1.1\r\nHost: x\r\n{bad}\r\n\r\n");
        assert_eq!(status_of(raw.as_bytes()), Some(400), "{bad:?}");
    }
}

#[test]
fn request_line_and_version_are_checked() {
    assert_eq!(status_of(b"POST /mcp HTTP/2.0\r\n\r\n"), Some(505));
    assert_eq!(status_of(b"POST /mcp\r\n\r\n"), Some(400));
    assert_eq!(status_of(b"POST /mcp HTTP/1.1 extra\r\n\r\n"), Some(400));
    let mut ok = b"POST /mcp HTTP/1.0\r\n\r\n".to_vec();
    assert!(matches!(try_frame(&mut ok), Framing::Ready(ref r) if !r.head.keep_alive));
}

#[test]
fn bearer_scheme_is_case_insensitive() {
    for scheme in ["Bearer", "bearer", "BEARER"] {
        let raw = format!("POST /mcp HTTP/1.1\r\nAuthorization: {scheme} t0k\r\n\r\n");
        match try_frame(&mut raw.into_bytes()) {
            Framing::Ready(req) => assert_eq!(req.head.auth_bearer.as_deref(), Some("t0k")),
            _ => panic!("expected Ready"),
        }
    }
}

#[test]
fn response_has_headers() {
    let resp = build_response(200, true, &[], Some("application/json"), b"{}");
    let text = String::from_utf8_lossy(&resp);
    assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(text.contains("Content-Length: 2\r\n"));
    assert!(text.contains("Content-Type: application/json\r\n"));
    assert!(text.contains("Connection: keep-alive\r\n"));
    assert!(text.ends_with("\r\n\r\n{}"));
}
