//! Minimal synchronous HTTP/2 client (RFC 9113) — just enough to
//! complete a single request on a freshly-opened TLS connection where
//! ALPN selected `h2`.
//!
//! Scope:
//!
//! * One request per connection. No multiplexing.
//! * Request bodies up to the caller's limit, streamed out respecting
//!   the initial flow-control window.
//! * Response bodies up to 8 MB (same cap as the HTTP/1.1 path),
//!   streamed back as DATA frames. Flow control is handled by
//!   opening a generous initial window and sending `WINDOW_UPDATE`
//!   after every DATA frame to keep traffic flowing.
//! * Full HPACK decode for server headers; encoder uses the simple
//!   literal-without-indexing path (see the `hpack` sibling module).
//! * Gracefully declines server push by RST_STREAM'ing any
//!   `PUSH_PROMISE`.
//!
//! HTTP/2 multiplexing, prioritization, and ALTSVC are out of scope —
//! they add complexity without meaningfully improving "can this page
//! load on our browser" compatibility.

use std::io::{Read, Write};

use oasis_types::error::{OasisError, Result};

use super::Url;
use super::hpack::{DynamicTable, decode_block, encode_literal};
use super::http::HttpResponse;

// --- Constants -------------------------------------------------------

/// HTTP/2 connection preface, RFC 9113 §3.4.
const CLIENT_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// Maximum response body we're willing to buffer.
const MAX_BODY_SIZE: usize = 8 * 1024 * 1024;

/// Generous initial stream window so we don't stall on normal pages.
const INITIAL_WINDOW_SIZE: u32 = 1_048_576;

/// Maximum frame payload we're willing to accept from the peer.
const MAX_FRAME_SIZE: u32 = 1 << 20; // 1 MiB

/// Header table size we advertise to the peer.
const HEADER_TABLE_SIZE: usize = 4096;

// --- Frame types (RFC 9113 §11.2) ------------------------------------

const FRAME_DATA: u8 = 0x0;
const FRAME_HEADERS: u8 = 0x1;
const FRAME_PRIORITY: u8 = 0x2;
const FRAME_RST_STREAM: u8 = 0x3;
const FRAME_SETTINGS: u8 = 0x4;
const FRAME_PUSH_PROMISE: u8 = 0x5;
const FRAME_PING: u8 = 0x6;
const FRAME_GOAWAY: u8 = 0x7;
const FRAME_WINDOW_UPDATE: u8 = 0x8;
const FRAME_CONTINUATION: u8 = 0x9;

// Flags
const FLAG_END_STREAM: u8 = 0x1;
const FLAG_ACK: u8 = 0x1;
const FLAG_END_HEADERS: u8 = 0x4;
const FLAG_PADDED: u8 = 0x8;
const FLAG_PRIORITY: u8 = 0x20;

// Settings identifiers (RFC 9113 §6.5.2)
const SETTINGS_HEADER_TABLE_SIZE: u16 = 0x1;
const SETTINGS_ENABLE_PUSH: u16 = 0x2;
const SETTINGS_MAX_CONCURRENT_STREAMS: u16 = 0x3;
const SETTINGS_INITIAL_WINDOW_SIZE: u16 = 0x4;
const SETTINGS_MAX_FRAME_SIZE: u16 = 0x5;
const SETTINGS_MAX_HEADER_LIST_SIZE: u16 = 0x6;

// Error codes (RFC 9113 §7)
const ERROR_CODE_NO_ERROR: u32 = 0x0;
const ERROR_CODE_PROTOCOL_ERROR: u32 = 0x1;
const ERROR_CODE_FLOW_CONTROL_ERROR: u32 = 0x3;
const ERROR_CODE_FRAME_SIZE_ERROR: u32 = 0x6;
const ERROR_CODE_REFUSED_STREAM: u32 = 0x7;
const ERROR_CODE_CANCEL: u32 = 0x8;

// --- Frame header ----------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct FrameHeader {
    length: u32,
    frame_type: u8,
    flags: u8,
    stream_id: u32,
}

fn read_exact<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<()> {
    let mut read = 0;
    while read < buf.len() {
        match r.read(&mut buf[read..]) {
            Ok(0) => {
                return Err(OasisError::Backend(
                    "HTTP/2: unexpected EOF in frame".into(),
                ));
            },
            Ok(n) => read += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(OasisError::Backend(format!("HTTP/2 read: {e}").into()));
            },
        }
    }
    Ok(())
}

fn read_frame_header<R: Read>(r: &mut R) -> Result<FrameHeader> {
    let mut hdr = [0u8; 9];
    read_exact(r, &mut hdr)?;
    let length = ((hdr[0] as u32) << 16) | ((hdr[1] as u32) << 8) | (hdr[2] as u32);
    let frame_type = hdr[3];
    let flags = hdr[4];
    let stream_id = (((hdr[5] & 0x7f) as u32) << 24)
        | ((hdr[6] as u32) << 16)
        | ((hdr[7] as u32) << 8)
        | (hdr[8] as u32);
    if length > MAX_FRAME_SIZE {
        return Err(OasisError::Backend(
            format!("HTTP/2 frame too large: {length}").into(),
        ));
    }
    Ok(FrameHeader {
        length,
        frame_type,
        flags,
        stream_id,
    })
}

fn write_frame_header<W: Write>(
    w: &mut W,
    length: u32,
    frame_type: u8,
    flags: u8,
    stream_id: u32,
) -> Result<()> {
    let hdr = [
        ((length >> 16) & 0xff) as u8,
        ((length >> 8) & 0xff) as u8,
        (length & 0xff) as u8,
        frame_type,
        flags,
        ((stream_id >> 24) & 0x7f) as u8,
        ((stream_id >> 16) & 0xff) as u8,
        ((stream_id >> 8) & 0xff) as u8,
        (stream_id & 0xff) as u8,
    ];
    w.write_all(&hdr)
        .map_err(|e| OasisError::Backend(format!("HTTP/2 write: {e}").into()))
}

fn write_all<W: Write>(w: &mut W, data: &[u8]) -> Result<()> {
    w.write_all(data)
        .map_err(|e| OasisError::Backend(format!("HTTP/2 write: {e}").into()))
}

// --- Public entry point ---------------------------------------------

/// Execute a single HTTP/2 request on an already-open TLS stream.
///
/// The stream must have just completed the TLS handshake with ALPN
/// having selected `h2`. This function writes the HTTP/2 connection
/// preface, completes the SETTINGS exchange, sends the request, and
/// reads frames until the server closes the request stream. On return,
/// the stream is in an indeterminate state — the caller should not
/// attempt to reuse it for another request without re-running the full
/// preface, so most callers drop the stream after a single request.
pub fn h2_request<S: Read + Write>(
    stream: &mut S,
    method: &str,
    url: &Url,
    body: Option<&[u8]>,
    extra_headers: &[(&str, &str)],
) -> Result<HttpResponse> {
    // 1. Send preface + initial SETTINGS.
    write_all(stream, CLIENT_PREFACE)?;
    send_initial_settings(stream)?;

    // 2. Bound request-body size. HTTP/2's default initial send window
    //    is 64 KiB (RFC 9113 §6.9.2); anything bigger would need us to
    //    track the peer's WINDOW_UPDATE frames, which isn't worth the
    //    complexity for a single-request client.
    if let Some(data) = body
        && data.len() > 65_535
    {
        return Err(OasisError::Backend(
            "HTTP/2: request body exceeds default send window".into(),
        ));
    }

    // 3. Build and send HEADERS frame for stream 1. We fragment into
    //    CONTINUATION frames if the block exceeds the default
    //    MAX_FRAME_SIZE (16 KiB is HTTP/2's floor and also the peer's
    //    initial limit before SETTINGS arrives).
    let header_block = encode_request_headers(method, url, body.map(|b| b.len()), extra_headers);
    let end_stream = body.is_none();
    write_header_block(stream, &header_block, 1, end_stream)?;

    // 4. If there's a body, send it as a single DATA frame with
    //    END_STREAM. The 16 KiB floor applies here too — no real
    //    server advertises a smaller MAX_FRAME_SIZE, so we don't
    //    bother fragmenting body DATA for the POST case.
    if let Some(data) = body {
        write_frame_header(stream, data.len() as u32, FRAME_DATA, FLAG_END_STREAM, 1)?;
        write_all(stream, data)?;
    }

    // 5. Drive the read loop.
    read_response(stream)
}

/// Write a HEADERS frame, splitting into CONTINUATION frames if the
/// block is larger than the HTTP/2 default `MAX_FRAME_SIZE` (16 KiB).
/// This keeps us conformant even if a server advertises the spec
/// minimum.
fn write_header_block<W: Write>(
    w: &mut W,
    block: &[u8],
    stream_id: u32,
    end_stream: bool,
) -> Result<()> {
    const DEFAULT_MAX_FRAME_SIZE: usize = 16_384;
    let es_flag = if end_stream { FLAG_END_STREAM } else { 0 };

    if block.len() <= DEFAULT_MAX_FRAME_SIZE {
        let flags = FLAG_END_HEADERS | es_flag;
        write_frame_header(w, block.len() as u32, FRAME_HEADERS, flags, stream_id)?;
        write_all(w, block)?;
        return Ok(());
    }

    // First chunk as HEADERS (with END_STREAM if requested, but not
    // END_HEADERS), remaining chunks as CONTINUATION, final CONTINUATION
    // with END_HEADERS.
    let (first, rest) = block.split_at(DEFAULT_MAX_FRAME_SIZE);
    write_frame_header(w, first.len() as u32, FRAME_HEADERS, es_flag, stream_id)?;
    write_all(w, first)?;

    let mut cursor = rest;
    while !cursor.is_empty() {
        let take = cursor.len().min(DEFAULT_MAX_FRAME_SIZE);
        let (chunk, tail) = cursor.split_at(take);
        let flags = if tail.is_empty() { FLAG_END_HEADERS } else { 0 };
        write_frame_header(w, chunk.len() as u32, FRAME_CONTINUATION, flags, stream_id)?;
        write_all(w, chunk)?;
        cursor = tail;
    }
    Ok(())
}

// --- Settings --------------------------------------------------------

fn send_initial_settings<W: Write>(w: &mut W) -> Result<()> {
    // Advertise our receive preferences. Each setting is 2 bytes id + 4 bytes value.
    let mut payload = Vec::with_capacity(24);
    push_setting(&mut payload, SETTINGS_ENABLE_PUSH, 0);
    push_setting(
        &mut payload,
        SETTINGS_INITIAL_WINDOW_SIZE,
        INITIAL_WINDOW_SIZE,
    );
    push_setting(&mut payload, SETTINGS_MAX_FRAME_SIZE, MAX_FRAME_SIZE);
    push_setting(
        &mut payload,
        SETTINGS_HEADER_TABLE_SIZE,
        HEADER_TABLE_SIZE as u32,
    );
    push_setting(&mut payload, SETTINGS_MAX_HEADER_LIST_SIZE, 64 * 1024);
    // Only one concurrent stream (we only ever open stream 1).
    push_setting(&mut payload, SETTINGS_MAX_CONCURRENT_STREAMS, 1);

    write_frame_header(w, payload.len() as u32, FRAME_SETTINGS, 0, 0)?;
    write_all(w, &payload)?;

    // Also open the connection-level flow-control window to match
    // INITIAL_WINDOW_SIZE so we don't immediately stall.
    let delta = INITIAL_WINDOW_SIZE.saturating_sub(65_535);
    if delta > 0 {
        send_window_update(w, 0, delta)?;
    }
    Ok(())
}

fn push_setting(buf: &mut Vec<u8>, id: u16, value: u32) {
    buf.extend_from_slice(&id.to_be_bytes());
    buf.extend_from_slice(&value.to_be_bytes());
}

fn send_settings_ack<W: Write>(w: &mut W) -> Result<()> {
    write_frame_header(w, 0, FRAME_SETTINGS, FLAG_ACK, 0)
}

fn send_window_update<W: Write>(w: &mut W, stream_id: u32, delta: u32) -> Result<()> {
    write_frame_header(w, 4, FRAME_WINDOW_UPDATE, 0, stream_id)?;
    write_all(w, &delta.to_be_bytes())
}

fn send_rst_stream<W: Write>(w: &mut W, stream_id: u32, error_code: u32) -> Result<()> {
    write_frame_header(w, 4, FRAME_RST_STREAM, 0, stream_id)?;
    write_all(w, &error_code.to_be_bytes())
}

fn send_goaway<W: Write>(w: &mut W, last_stream: u32, error_code: u32) -> Result<()> {
    let mut payload = Vec::with_capacity(8);
    payload.extend_from_slice(&last_stream.to_be_bytes());
    payload.extend_from_slice(&error_code.to_be_bytes());
    write_frame_header(w, payload.len() as u32, FRAME_GOAWAY, 0, 0)?;
    write_all(w, &payload)
}

fn send_ping_ack<W: Write>(w: &mut W, opaque: &[u8; 8]) -> Result<()> {
    write_frame_header(w, 8, FRAME_PING, FLAG_ACK, 0)?;
    write_all(w, opaque)
}

// --- Request header encoding ----------------------------------------

fn encode_request_headers(
    method: &str,
    url: &Url,
    body_len: Option<usize>,
    extra_headers: &[(&str, &str)],
) -> Vec<u8> {
    let mut out = Vec::new();

    // Pseudo-headers must come first, in this order.
    encode_literal(&mut out, ":method", method);
    encode_literal(&mut out, ":scheme", &url.scheme);

    let path = if let Some(ref q) = url.query {
        format!("{}?{}", url.path, q)
    } else {
        url.path.clone()
    };
    // Some servers reject empty :path; normalize "" to "/".
    let path_ref = if path.is_empty() { "/" } else { path.as_str() };
    encode_literal(&mut out, ":path", path_ref);

    let default_port = if url.scheme == "https" { 443 } else { 80 };
    let authority = match url.port {
        Some(p) if p != default_port => format!("{}:{}", url.host, p),
        _ => url.host.clone(),
    };
    encode_literal(&mut out, ":authority", &authority);

    // Regular headers. HTTP/2 requires header names to be lowercase.
    let mut saw_ua = false;
    let mut saw_accept = false;
    let mut saw_accept_encoding = false;
    let mut saw_content_type = false;
    let mut saw_content_length = false;
    for (name, value) in extra_headers {
        let lower = name.to_ascii_lowercase();
        // Skip connection-specific headers that are banned in HTTP/2
        // (RFC 9113 §8.2.2).
        if is_connection_specific(&lower) {
            continue;
        }
        match lower.as_str() {
            "user-agent" => saw_ua = true,
            "accept" => saw_accept = true,
            "accept-encoding" => saw_accept_encoding = true,
            "content-type" => saw_content_type = true,
            "content-length" => saw_content_length = true,
            _ => {},
        }
        encode_literal(&mut out, &lower, value);
    }
    if !saw_ua {
        encode_literal(&mut out, "user-agent", "OASIS/1.0");
    }
    if !saw_accept {
        encode_literal(&mut out, "accept", "*/*");
    }
    if !saw_accept_encoding {
        encode_literal(&mut out, "accept-encoding", "gzip, deflate, br");
    }
    if let Some(len) = body_len {
        if !saw_content_type {
            encode_literal(
                &mut out,
                "content-type",
                "application/x-www-form-urlencoded",
            );
        }
        if !saw_content_length {
            // Content-length is optional in HTTP/2 when END_STREAM
            // terminates the body, but strict API gateways reject
            // body-carrying requests without it.
            encode_literal(&mut out, "content-length", &len.to_string());
        }
    }

    out
}

fn is_connection_specific(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "transfer-encoding"
            | "upgrade"
            | "keep-alive"
            | "proxy-connection"
            | "host"
            // `te` is allowed only with value "trailers"; we just drop it.
            | "te"
    )
}

// --- Response read loop ---------------------------------------------

fn read_response<S: Read + Write>(stream: &mut S) -> Result<HttpResponse> {
    let mut table = DynamicTable::new(HEADER_TABLE_SIZE);
    let mut header_buf: Vec<u8> = Vec::new();
    let mut waiting_continuation = false;

    let mut response_headers: Option<Vec<(String, String)>> = None;
    let mut body: Vec<u8> = Vec::new();
    let mut stream_closed = false;

    // Deadline to bound the read loop in case the server misbehaves.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);

    // Keep reading while the request stream is still open, or while
    // we're still stitching CONTINUATION frames together (END_STREAM
    // can arrive on the HEADERS frame before END_HEADERS completes
    // the block, and we must still read the CONTINUATIONs).
    while !stream_closed || waiting_continuation {
        if std::time::Instant::now() > deadline {
            return Err(OasisError::Backend("HTTP/2 response timed out".into()));
        }

        let fh = read_frame_header(stream)?;
        if waiting_continuation && fh.frame_type != FRAME_CONTINUATION {
            let _ = send_goaway(stream, 1, ERROR_CODE_PROTOCOL_ERROR);
            return Err(OasisError::Backend(
                "HTTP/2: expected CONTINUATION frame".into(),
            ));
        }

        // Read payload.
        let mut payload = vec![0u8; fh.length as usize];
        read_exact(stream, &mut payload)?;

        match fh.frame_type {
            FRAME_SETTINGS => {
                // SETTINGS, like PING, is connection-level and MUST
                // carry stream_id == 0 (RFC 9113 §6.5).
                if fh.stream_id != 0 {
                    let _ = send_goaway(stream, 0, ERROR_CODE_PROTOCOL_ERROR);
                    return Err(OasisError::Backend(
                        "HTTP/2: SETTINGS on non-zero stream".into(),
                    ));
                }
                if fh.flags & FLAG_ACK != 0 {
                    // Server ACK'd our settings. RFC 9113 §6.5:
                    // "Receipt of a SETTINGS frame with the ACK flag
                    //  set and a length field value other than 0 MUST
                    //  be treated as a connection error of type
                    //  FRAME_SIZE_ERROR."
                    if !payload.is_empty() {
                        let _ = send_goaway(stream, 0, ERROR_CODE_FRAME_SIZE_ERROR);
                        return Err(OasisError::Backend(
                            "HTTP/2: SETTINGS ACK with non-empty payload".into(),
                        ));
                    }
                } else {
                    if !payload.len().is_multiple_of(6) {
                        let _ = send_goaway(stream, 0, ERROR_CODE_FRAME_SIZE_ERROR);
                        return Err(OasisError::Backend(
                            "HTTP/2: malformed SETTINGS frame".into(),
                        ));
                    }
                    // Walk the 6-byte entries. We only care about a
                    // handful; everything else we accept silently.
                    // RFC 9113 §6.5.2 gives the legal bounds and
                    // requires FLOW_CONTROL_ERROR / PROTOCOL_ERROR on
                    // out-of-range values.
                    let mut i = 0;
                    while i + 6 <= payload.len() {
                        let id = u16::from_be_bytes([payload[i], payload[i + 1]]);
                        let val = u32::from_be_bytes([
                            payload[i + 2],
                            payload[i + 3],
                            payload[i + 4],
                            payload[i + 5],
                        ]);
                        i += 6;
                        match id {
                            SETTINGS_INITIAL_WINDOW_SIZE => {
                                if val > 0x7fff_ffff {
                                    let _ = send_goaway(
                                        stream,
                                        0,
                                        ERROR_CODE_FLOW_CONTROL_ERROR,
                                    );
                                    return Err(OasisError::Backend(
                                        "HTTP/2: FLOW_CONTROL_ERROR on INITIAL_WINDOW_SIZE".into(),
                                    ));
                                }
                            },
                            SETTINGS_MAX_FRAME_SIZE => {
                                if !(16_384..=16_777_215).contains(&val) {
                                    let _ = send_goaway(
                                        stream,
                                        0,
                                        ERROR_CODE_PROTOCOL_ERROR,
                                    );
                                    return Err(OasisError::Backend(
                                        "HTTP/2: PROTOCOL_ERROR on MAX_FRAME_SIZE".into(),
                                    ));
                                }
                            },
                            SETTINGS_ENABLE_PUSH => {
                                if val > 1 {
                                    let _ = send_goaway(
                                        stream,
                                        0,
                                        ERROR_CODE_PROTOCOL_ERROR,
                                    );
                                    return Err(OasisError::Backend(
                                        "HTTP/2: PROTOCOL_ERROR on ENABLE_PUSH".into(),
                                    ));
                                }
                            },
                            _ => {},
                        }
                    }
                    send_settings_ack(stream)?;
                }
            },
            FRAME_HEADERS => {
                if fh.stream_id == 0 {
                    // HEADERS on the connection stream is a protocol
                    // error (RFC 9113 §6.2). RST_STREAM on 0 is itself
                    // illegal, so we must escalate to a connection
                    // error via GOAWAY.
                    let _ = send_goaway(stream, 0, ERROR_CODE_PROTOCOL_ERROR);
                    return Err(OasisError::Backend("HTTP/2: HEADERS on stream 0".into()));
                }
                if fh.stream_id != 1 {
                    // Stray frame on an unknown stream — reset it.
                    send_rst_stream(stream, fh.stream_id, ERROR_CODE_CANCEL)?;
                    continue;
                }
                let block = strip_headers_padding_and_priority(&payload, fh.flags)?;
                header_buf.extend_from_slice(block);

                if fh.flags & FLAG_END_HEADERS != 0 {
                    let headers = decode_block(&header_buf, &mut table, HEADER_TABLE_SIZE)?;
                    response_headers = Some(headers);
                    header_buf.clear();
                    waiting_continuation = false;
                } else {
                    waiting_continuation = true;
                }

                if fh.flags & FLAG_END_STREAM != 0 {
                    stream_closed = true;
                }
            },
            FRAME_CONTINUATION => {
                if fh.stream_id != 1 || !waiting_continuation {
                    let _ = send_goaway(stream, 1, ERROR_CODE_PROTOCOL_ERROR);
                    return Err(OasisError::Backend(
                        "HTTP/2: unexpected CONTINUATION".into(),
                    ));
                }
                header_buf.extend_from_slice(&payload);
                if fh.flags & FLAG_END_HEADERS != 0 {
                    let headers = decode_block(&header_buf, &mut table, HEADER_TABLE_SIZE)?;
                    response_headers = Some(headers);
                    header_buf.clear();
                    waiting_continuation = false;
                }
            },
            FRAME_DATA => {
                if fh.stream_id == 0 {
                    // DATA on stream 0 is a connection-level protocol
                    // error (RFC 9113 §6.1). Same escalation as above.
                    let _ = send_goaway(stream, 0, ERROR_CODE_PROTOCOL_ERROR);
                    return Err(OasisError::Backend("HTTP/2: DATA on stream 0".into()));
                }
                if fh.stream_id != 1 {
                    send_rst_stream(stream, fh.stream_id, ERROR_CODE_CANCEL)?;
                    continue;
                }
                let data = strip_data_padding(&payload, fh.flags)?;
                if body.len() + data.len() > MAX_BODY_SIZE {
                    send_rst_stream(stream, 1, ERROR_CODE_CANCEL)?;
                    send_goaway(stream, 1, ERROR_CODE_CANCEL)?;
                    return Err(OasisError::Backend(
                        "HTTP/2 response body exceeds 8 MB limit".into(),
                    ));
                }
                body.extend_from_slice(data);

                // Replenish the flow-control window (both connection
                // and stream) so the server can keep sending.
                if !data.is_empty() {
                    send_window_update(stream, 0, data.len() as u32)?;
                    send_window_update(stream, 1, data.len() as u32)?;
                }

                if fh.flags & FLAG_END_STREAM != 0 {
                    stream_closed = true;
                }
            },
            FRAME_WINDOW_UPDATE => {
                if payload.len() != 4 {
                    let _ = send_goaway(stream, 0, ERROR_CODE_FRAME_SIZE_ERROR);
                    return Err(OasisError::Backend(
                        "HTTP/2: WINDOW_UPDATE payload not 4 bytes".into(),
                    ));
                }
                // We don't track our send window precisely — request
                // bodies are capped at 65k above, so updates are moot.
            },
            FRAME_PING => {
                // RFC 9113 §6.7 — PING is a connection-level frame and
                // MUST carry stream_id == 0. Anything else is a
                // connection error.
                if fh.stream_id != 0 {
                    let _ = send_goaway(stream, 0, ERROR_CODE_PROTOCOL_ERROR);
                    return Err(OasisError::Backend(
                        "HTTP/2: PING on non-zero stream".into(),
                    ));
                }
                if payload.len() != 8 {
                    let _ = send_goaway(stream, 0, ERROR_CODE_FRAME_SIZE_ERROR);
                    return Err(OasisError::Backend("HTTP/2: malformed PING".into()));
                }
                if fh.flags & FLAG_ACK == 0 {
                    let mut op = [0u8; 8];
                    op.copy_from_slice(&payload);
                    send_ping_ack(stream, &op)?;
                }
            },
            FRAME_RST_STREAM => {
                // RFC 9113 §6.4 — RST_STREAM on stream 0 is a
                // connection error. We cannot reply with RST_STREAM on
                // 0 (also illegal); escalate to GOAWAY.
                if fh.stream_id == 0 {
                    let _ = send_goaway(stream, 0, ERROR_CODE_PROTOCOL_ERROR);
                    return Err(OasisError::Backend("HTTP/2: RST_STREAM on stream 0".into()));
                }
                if payload.len() != 4 {
                    let _ = send_goaway(stream, 0, ERROR_CODE_FRAME_SIZE_ERROR);
                    return Err(OasisError::Backend(
                        "HTTP/2: RST_STREAM payload not 4 bytes".into(),
                    ));
                }
                if fh.stream_id == 1 {
                    let code =
                        u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                    return Err(OasisError::Backend(
                        format!("HTTP/2 stream reset by peer, code={code}").into(),
                    ));
                }
            },
            FRAME_GOAWAY => {
                // If the request stream was already fully received we're
                // fine; otherwise bail out.
                if !stream_closed {
                    let code = if payload.len() >= 8 {
                        u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]])
                    } else {
                        0
                    };
                    return Err(OasisError::Backend(
                        format!("HTTP/2 GOAWAY from peer, code={code}").into(),
                    ));
                }
            },
            FRAME_PUSH_PROMISE => {
                // RFC 9113 §6.6 — we sent ENABLE_PUSH=0, so this is a
                // protocol error. Reject cleanly. The promised stream
                // ID lives after any PADDED byte, so strip padding
                // before indexing.
                let body_slice = if fh.flags & FLAG_PADDED != 0 {
                    if payload.is_empty() {
                        return Err(OasisError::Backend(
                            "HTTP/2: PUSH_PROMISE PADDED with empty frame".into(),
                        ));
                    }
                    let pad_len = payload[0] as usize;
                    if 1 + pad_len > payload.len() {
                        return Err(OasisError::Backend(
                            "HTTP/2: PUSH_PROMISE pad_len too large".into(),
                        ));
                    }
                    &payload[1..payload.len() - pad_len]
                } else {
                    &payload[..]
                };
                // A PUSH_PROMISE that doesn't carry the 4-byte promised
                // stream ID is malformed — treat as a connection-level
                // protocol error (RFC 9113 §6.6).
                if body_slice.len() < 4 {
                    let _ = send_goaway(stream, 0, ERROR_CODE_PROTOCOL_ERROR);
                    return Err(OasisError::Backend(
                        "HTTP/2: PUSH_PROMISE too short for promised stream ID".into(),
                    ));
                }
                let promised = u32::from_be_bytes([
                    body_slice[0] & 0x7f,
                    body_slice[1],
                    body_slice[2],
                    body_slice[3],
                ]);
                if promised == 0 {
                    send_goaway(stream, 0, ERROR_CODE_PROTOCOL_ERROR)?;
                    return Err(OasisError::Backend(
                        "HTTP/2: PUSH_PROMISE with promised stream 0".into(),
                    ));
                }
                send_rst_stream(stream, promised, ERROR_CODE_REFUSED_STREAM)?;
            },
            FRAME_PRIORITY => {
                // Deprecated in RFC 9113 but still legal. Ignore.
            },
            _ => {
                // Unknown frame types must be ignored (RFC 9113 §4.1).
            },
        }
    }

    let headers = response_headers
        .ok_or_else(|| OasisError::Backend("HTTP/2: stream ended without headers".into()))?;

    // Extract :status, strip pseudo-headers from the rest.
    let mut status_code: u16 = 0;
    let mut regular: Vec<(String, String)> = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        if let Some(stripped) = name.strip_prefix(':') {
            if stripped == "status" {
                status_code = value.parse().unwrap_or(0);
            }
            // Other pseudo-headers (RFC 9113 §8.3.1 lists none for
            // responses) are dropped.
            continue;
        }
        regular.push((name, value));
    }

    if status_code == 0 {
        return Err(OasisError::Backend(
            "HTTP/2: response missing :status".into(),
        ));
    }

    // Send a courteous GOAWAY so the peer knows we're done with the
    // connection. Ignore errors — the response is already complete.
    let _ = send_goaway(stream, 1, ERROR_CODE_NO_ERROR);

    // Decode content-encoding using the shared HTTP/1.1 helper by
    // going through `parse_response`. Easier: reuse `decode_body`.
    let body = super::http::decode_body_public(&regular, body)?;

    Ok(HttpResponse {
        status_code,
        headers: regular,
        body,
    })
}

fn strip_headers_padding_and_priority(payload: &[u8], flags: u8) -> Result<&[u8]> {
    let mut start = 0usize;
    let mut end = payload.len();
    if flags & FLAG_PADDED != 0 {
        if payload.is_empty() {
            return Err(OasisError::Backend(
                "HTTP/2: PADDED with empty frame".into(),
            ));
        }
        let pad_len = payload[0] as usize;
        start = 1;
        if pad_len >= payload.len() {
            return Err(OasisError::Backend("HTTP/2: pad_len too large".into()));
        }
        end -= pad_len;
    }
    if flags & FLAG_PRIORITY != 0 {
        // 5 bytes priority info: stream dep (4) + weight (1).
        if end < start + 5 {
            return Err(OasisError::Backend(
                "HTTP/2: truncated PRIORITY in HEADERS".into(),
            ));
        }
        start += 5;
    }
    Ok(&payload[start..end])
}

fn strip_data_padding(payload: &[u8], flags: u8) -> Result<&[u8]> {
    if flags & FLAG_PADDED == 0 {
        return Ok(payload);
    }
    if payload.is_empty() {
        return Err(OasisError::Backend(
            "HTTP/2: PADDED with empty frame".into(),
        ));
    }
    let pad_len = payload[0] as usize;
    if 1 + pad_len > payload.len() {
        return Err(OasisError::Backend("HTTP/2: pad_len too large".into()));
    }
    Ok(&payload[1..payload.len() - pad_len])
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A [`Read`]+[`Write`] pair of in-memory buffers that lets tests
    /// script an HTTP/2 dialog.
    struct MemStream {
        read: std::io::Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl MemStream {
        fn new(server_bytes: Vec<u8>) -> Self {
            Self {
                read: std::io::Cursor::new(server_bytes),
                written: Vec::new(),
            }
        }
    }

    impl Read for MemStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.read.read(buf)
        }
    }

    impl Write for MemStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Build a frame (header + payload) as a single byte vector.
    fn build_frame(frame_type: u8, flags: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
        let length = payload.len() as u32;
        let mut out = Vec::with_capacity(9 + payload.len());
        out.push(((length >> 16) & 0xff) as u8);
        out.push(((length >> 8) & 0xff) as u8);
        out.push((length & 0xff) as u8);
        out.push(frame_type);
        out.push(flags);
        out.push(((stream_id >> 24) & 0x7f) as u8);
        out.push(((stream_id >> 16) & 0xff) as u8);
        out.push(((stream_id >> 8) & 0xff) as u8);
        out.push((stream_id & 0xff) as u8);
        out.extend_from_slice(payload);
        out
    }

    fn make_response_headers() -> Vec<u8> {
        // Literal-without-indexing :status 200 + content-type text/html.
        let mut out = Vec::new();
        encode_literal(&mut out, ":status", "200");
        encode_literal(&mut out, "content-type", "text/html");
        out
    }

    #[test]
    fn frame_header_roundtrip() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame_header(&mut buf, 42, FRAME_HEADERS, FLAG_END_STREAM, 1).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let fh = read_frame_header(&mut cursor).unwrap();
        assert_eq!(fh.length, 42);
        assert_eq!(fh.frame_type, FRAME_HEADERS);
        assert_eq!(fh.flags, FLAG_END_STREAM);
        assert_eq!(fh.stream_id, 1);
    }

    #[test]
    fn end_to_end_simple_response() {
        // Server script:
        //  - SETTINGS (empty)
        //  - SETTINGS ACK (for our client SETTINGS, but our loop is
        //    tolerant of ordering)
        //  - HEADERS on stream 1 with END_HEADERS + END_STREAM, holding
        //    :status 200 and content-type text/html
        //  - DATA frame on stream 1 with END_STREAM carrying "<hi/>"
        //
        // Our driver reads frames until END_STREAM, so the DATA frame
        // alone with END_STREAM is what closes the stream; the HEADERS
        // frame should *not* carry END_STREAM in that case.
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        server.extend(build_frame(FRAME_SETTINGS, FLAG_ACK, 0, &[]));
        let hdr_block = make_response_headers();
        server.extend(build_frame(FRAME_HEADERS, FLAG_END_HEADERS, 1, &hdr_block));
        server.extend(build_frame(FRAME_DATA, FLAG_END_STREAM, 1, b"<hi/>"));

        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let resp = h2_request(&mut stream, "GET", &url, None, &[]).unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.body, b"<hi/>");
        // Confirm the client wrote the preface.
        assert!(stream.written.starts_with(CLIENT_PREFACE));
    }

    #[test]
    fn end_stream_on_headers_ok() {
        // Server closes the stream on the HEADERS frame itself (empty body).
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        let hdr_block = make_response_headers();
        server.extend(build_frame(
            FRAME_HEADERS,
            FLAG_END_HEADERS | FLAG_END_STREAM,
            1,
            &hdr_block,
        ));

        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let resp = h2_request(&mut stream, "GET", &url, None, &[]).unwrap();
        assert_eq!(resp.status_code, 200);
        assert!(resp.body.is_empty());
    }

    #[test]
    fn ping_is_acked() {
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        server.extend(build_frame(FRAME_PING, 0, 0, &[0u8; 8]));
        let hdr_block = make_response_headers();
        server.extend(build_frame(
            FRAME_HEADERS,
            FLAG_END_HEADERS | FLAG_END_STREAM,
            1,
            &hdr_block,
        ));

        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let resp = h2_request(&mut stream, "GET", &url, None, &[]).unwrap();
        assert_eq!(resp.status_code, 200);
        // Among the written bytes there should be a PING ACK frame.
        // The simplest check: look for the PING type byte with ACK flag.
        let mut found = false;
        let w = &stream.written;
        // Skip the 24-byte connection preface before walking frames.
        let mut i = CLIENT_PREFACE.len();
        while i + 9 <= w.len() {
            let length = ((w[i] as usize) << 16) | ((w[i + 1] as usize) << 8) | (w[i + 2] as usize);
            let ftype = w[i + 3];
            let flags = w[i + 4];
            if ftype == FRAME_PING && flags & FLAG_ACK != 0 {
                found = true;
                break;
            }
            i += 9 + length;
        }
        assert!(found, "client did not ACK PING");
    }

    #[test]
    fn rst_stream_is_error() {
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        // RST_STREAM on stream 1 with REFUSED_STREAM.
        server.extend(build_frame(FRAME_RST_STREAM, 0, 1, &[0, 0, 0, 7]));

        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let err = h2_request(&mut stream, "GET", &url, None, &[]).unwrap_err();
        assert!(err.to_string().contains("stream reset"));
    }

    #[test]
    fn continuation_frames_reassembled() {
        // Split the header block across HEADERS + CONTINUATION.
        let full = make_response_headers();
        let split = full.len() / 2;
        let (a, b) = full.split_at(split);

        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        // END_STREAM goes on HEADERS (per RFC 9113); CONTINUATION
        // only carries END_HEADERS.
        server.extend(build_frame(FRAME_HEADERS, FLAG_END_STREAM, 1, a));
        server.extend(build_frame(FRAME_CONTINUATION, FLAG_END_HEADERS, 1, b));

        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let resp = h2_request(&mut stream, "GET", &url, None, &[]).unwrap();
        assert_eq!(resp.status_code, 200);
    }

    #[test]
    fn post_body_sent_as_data_frame() {
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        let hdr_block = make_response_headers();
        server.extend(build_frame(
            FRAME_HEADERS,
            FLAG_END_HEADERS | FLAG_END_STREAM,
            1,
            &hdr_block,
        ));

        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/api").unwrap();
        let body = b"key=value";
        let resp = h2_request(&mut stream, "POST", &url, Some(body), &[]).unwrap();
        assert_eq!(resp.status_code, 200);

        // The written bytes should contain a DATA frame with our body.
        let w = &stream.written;
        let mut i = CLIENT_PREFACE.len();
        let mut found_data = false;
        while i + 9 <= w.len() {
            let length = ((w[i] as usize) << 16) | ((w[i + 1] as usize) << 8) | (w[i + 2] as usize);
            let ftype = w[i + 3];
            if ftype == FRAME_DATA {
                let payload = &w[i + 9..i + 9 + length];
                assert_eq!(payload, body);
                found_data = true;
                break;
            }
            i += 9 + length;
        }
        assert!(found_data, "client did not send DATA frame for POST body");
    }

    #[test]
    fn request_headers_include_pseudo_and_defaults() {
        let url = crate::loader::Url::parse("https://example.com/path?q=1").unwrap();
        let block = encode_request_headers("GET", &url, None, &[]);

        let mut table = DynamicTable::new(4096);
        let decoded = decode_block(&block, &mut table, 4096).unwrap();

        // Pseudo-headers must come first, in fixed order.
        assert_eq!(decoded[0], (":method".into(), "GET".into()));
        assert_eq!(decoded[1], (":scheme".into(), "https".into()));
        assert_eq!(decoded[2], (":path".into(), "/path?q=1".into()));
        assert_eq!(decoded[3], (":authority".into(), "example.com".into()));

        // Default user-agent, accept, accept-encoding were appended.
        let has = |name: &str| decoded.iter().any(|(n, _)| n == name);
        assert!(has("user-agent"));
        assert!(has("accept"));
        assert!(has("accept-encoding"));
    }

    #[test]
    fn connection_specific_headers_dropped() {
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let block = encode_request_headers(
            "GET",
            &url,
            None,
            &[
                ("Connection", "keep-alive"),
                ("Transfer-Encoding", "chunked"),
                ("Host", "example.com"),
                ("Cookie", "session=abc"),
            ],
        );
        let mut table = DynamicTable::new(4096);
        let decoded = decode_block(&block, &mut table, 4096).unwrap();
        assert!(!decoded.iter().any(|(n, _)| n == "connection"));
        assert!(!decoded.iter().any(|(n, _)| n == "transfer-encoding"));
        assert!(!decoded.iter().any(|(n, _)| n == "host"));
        // Non-banned extra headers are preserved.
        assert!(
            decoded
                .iter()
                .any(|(n, v)| n == "cookie" && v == "session=abc")
        );
    }

    #[test]
    fn post_request_emits_content_length() {
        let url = crate::loader::Url::parse("https://example.com/api").unwrap();
        let block = encode_request_headers("POST", &url, Some(42), &[]);
        let mut table = DynamicTable::new(4096);
        let decoded = decode_block(&block, &mut table, 4096).unwrap();
        let cl = decoded
            .iter()
            .find(|(n, _)| n == "content-length")
            .map(|(_, v)| v.as_str());
        assert_eq!(cl, Some("42"));
    }

    #[test]
    fn explicit_content_length_header_not_duplicated() {
        let url = crate::loader::Url::parse("https://example.com/api").unwrap();
        let block = encode_request_headers("POST", &url, Some(3), &[("Content-Length", "99")]);
        let mut table = DynamicTable::new(4096);
        let decoded = decode_block(&block, &mut table, 4096).unwrap();
        let cls: Vec<&str> = decoded
            .iter()
            .filter(|(n, _)| n == "content-length")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(cls, vec!["99"]);
    }

    #[test]
    fn headers_on_stream_zero_is_connection_error() {
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        // HEADERS on stream 0 — must trigger a connection-level error,
        // not RST_STREAM on stream 0.
        server.extend(build_frame(FRAME_HEADERS, FLAG_END_HEADERS, 0, &[]));
        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let err = h2_request(&mut stream, "GET", &url, None, &[]).unwrap_err();
        assert!(err.to_string().contains("stream 0"));
        // Verify a GOAWAY was emitted on stream 0, not a RST_STREAM.
        let w = &stream.written;
        let mut i = CLIENT_PREFACE.len();
        let mut saw_goaway = false;
        let mut saw_rst_on_zero = false;
        while i + 9 <= w.len() {
            let length = ((w[i] as usize) << 16) | ((w[i + 1] as usize) << 8) | (w[i + 2] as usize);
            let ftype = w[i + 3];
            let sid = (((w[i + 5] & 0x7f) as u32) << 24)
                | ((w[i + 6] as u32) << 16)
                | ((w[i + 7] as u32) << 8)
                | (w[i + 8] as u32);
            if ftype == FRAME_GOAWAY {
                saw_goaway = true;
            }
            if ftype == FRAME_RST_STREAM && sid == 0 {
                saw_rst_on_zero = true;
            }
            i += 9 + length;
        }
        assert!(saw_goaway, "client did not send GOAWAY on stream 0 error");
        assert!(
            !saw_rst_on_zero,
            "client illegally sent RST_STREAM on stream 0"
        );
    }

    #[test]
    fn data_on_stream_zero_is_connection_error() {
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        server.extend(build_frame(FRAME_DATA, 0, 0, b"bad"));
        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let err = h2_request(&mut stream, "GET", &url, None, &[]).unwrap_err();
        assert!(err.to_string().contains("stream 0"));
    }

    #[test]
    fn settings_max_frame_size_out_of_range_rejected() {
        // Server advertises MAX_FRAME_SIZE = 1 (below the 16 KiB floor).
        let mut settings_payload = Vec::new();
        push_setting(&mut settings_payload, SETTINGS_MAX_FRAME_SIZE, 1);

        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &settings_payload));
        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let err = h2_request(&mut stream, "GET", &url, None, &[]).unwrap_err();
        assert!(err.to_string().contains("MAX_FRAME_SIZE"));
    }

    #[test]
    fn large_header_block_fragments_into_continuation() {
        // Build a header block larger than 16 KiB (the default HTTP/2
        // MAX_FRAME_SIZE floor) by adding a big Cookie header.
        let big_cookie = "k=".to_string() + &"x".repeat(20_000);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();

        // Script the server: reply immediately with a small response.
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        let hdr_block = make_response_headers();
        server.extend(build_frame(
            FRAME_HEADERS,
            FLAG_END_HEADERS | FLAG_END_STREAM,
            1,
            &hdr_block,
        ));

        let mut stream = MemStream::new(server);
        h2_request(
            &mut stream,
            "GET",
            &url,
            None,
            &[("cookie", big_cookie.as_str())],
        )
        .unwrap();

        // Walk the written frames; there should be at least one
        // HEADERS followed by one or more CONTINUATION frames.
        let w = &stream.written;
        let mut i = CLIENT_PREFACE.len();
        let mut saw_headers_without_end = false;
        let mut saw_continuation_with_end = false;
        while i + 9 <= w.len() {
            let length = ((w[i] as usize) << 16) | ((w[i + 1] as usize) << 8) | (w[i + 2] as usize);
            let ftype = w[i + 3];
            let flags = w[i + 4];
            if ftype == FRAME_HEADERS && flags & FLAG_END_HEADERS == 0 {
                saw_headers_without_end = true;
            }
            if ftype == FRAME_CONTINUATION && flags & FLAG_END_HEADERS != 0 {
                saw_continuation_with_end = true;
            }
            i += 9 + length;
        }
        assert!(
            saw_headers_without_end,
            "expected an initial HEADERS frame without END_HEADERS"
        );
        assert!(
            saw_continuation_with_end,
            "expected a terminal CONTINUATION frame with END_HEADERS"
        );
    }

    #[test]
    fn ping_on_non_zero_stream_is_connection_error() {
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        // PING on stream 1 — protocol error per RFC 9113 §6.7.
        server.extend(build_frame(FRAME_PING, 0, 1, &[0u8; 8]));
        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let err = h2_request(&mut stream, "GET", &url, None, &[]).unwrap_err();
        assert!(err.to_string().contains("PING on non-zero stream"));
    }

    #[test]
    fn rst_stream_on_stream_zero_is_connection_error() {
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        // RST_STREAM on stream 0 — protocol error per RFC 9113 §6.4.
        server.extend(build_frame(FRAME_RST_STREAM, 0, 0, &[0, 0, 0, 1]));
        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let err = h2_request(&mut stream, "GET", &url, None, &[]).unwrap_err();
        assert!(err.to_string().contains("RST_STREAM on stream 0"));
    }

    #[test]
    fn push_promise_short_payload_is_connection_error() {
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        // PUSH_PROMISE with only 3 payload bytes (needs 4 for stream ID).
        server.extend(build_frame(FRAME_PUSH_PROMISE, 0, 1, &[0, 0, 0]));
        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let err = h2_request(&mut stream, "GET", &url, None, &[]).unwrap_err();
        assert!(err.to_string().contains("PUSH_PROMISE too short"));
    }

    #[test]
    fn settings_ack_with_payload_is_connection_error() {
        let mut server = Vec::new();
        // SETTINGS ACK with non-empty payload — FRAME_SIZE_ERROR.
        server.extend(build_frame(FRAME_SETTINGS, FLAG_ACK, 0, &[0u8; 6]));
        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let err = h2_request(&mut stream, "GET", &url, None, &[]).unwrap_err();
        assert!(
            err.to_string()
                .contains("SETTINGS ACK with non-empty payload")
        );
    }

    #[test]
    fn settings_on_non_zero_stream_is_connection_error() {
        let mut server = Vec::new();
        // SETTINGS on stream 1 — protocol error per RFC 9113 §6.5.
        server.extend(build_frame(FRAME_SETTINGS, 0, 1, &[]));
        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let err = h2_request(&mut stream, "GET", &url, None, &[]).unwrap_err();
        assert!(err.to_string().contains("SETTINGS on non-zero stream"));
    }

    #[test]
    fn window_update_wrong_length_is_connection_error() {
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        // WINDOW_UPDATE with 3 bytes instead of exactly 4.
        server.extend(build_frame(FRAME_WINDOW_UPDATE, 0, 0, &[0, 0, 1]));
        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let err = h2_request(&mut stream, "GET", &url, None, &[]).unwrap_err();
        assert!(err.to_string().contains("WINDOW_UPDATE"));
    }

    #[test]
    fn ping_ack_with_wrong_length_is_connection_error() {
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        // PING ACK with 4 bytes instead of exactly 8.
        server.extend(build_frame(FRAME_PING, FLAG_ACK, 0, &[0u8; 4]));
        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let err = h2_request(&mut stream, "GET", &url, None, &[]).unwrap_err();
        assert!(err.to_string().contains("malformed PING"));
    }

    #[test]
    fn padded_data_frame_handled() {
        let mut server = Vec::new();
        server.extend(build_frame(FRAME_SETTINGS, 0, 0, &[]));
        let hdr_block = make_response_headers();
        server.extend(build_frame(FRAME_HEADERS, FLAG_END_HEADERS, 1, &hdr_block));
        // Padded payload: [pad_len=3] "body" [0,0,0]
        let mut padded = vec![3u8];
        padded.extend_from_slice(b"body");
        padded.extend_from_slice(&[0u8, 0, 0]);
        server.extend(build_frame(
            FRAME_DATA,
            FLAG_END_STREAM | FLAG_PADDED,
            1,
            &padded,
        ));
        let mut stream = MemStream::new(server);
        let url = crate::loader::Url::parse("https://example.com/").unwrap();
        let resp = h2_request(&mut stream, "GET", &url, None, &[]).unwrap();
        assert_eq!(resp.body, b"body");
    }

    /// Live end-to-end probe against `www.wikipedia.org`. Hidden
    /// behind `#[ignore]` so CI doesn't flake on network failures —
    /// run with `cargo test -p oasis-browser wikipedia_live -- --ignored`.
    #[test]
    #[ignore]
    fn wikipedia_live_h2() {
        use crate::loader::http::http_get;
        use oasis_net::tls_rustls::RustlsTlsProvider;
        use oasis_types::tls::TlsProvider;

        let provider = RustlsTlsProvider::new();
        let tls: &dyn TlsProvider = &provider;
        let url = crate::loader::Url::parse("https://www.wikipedia.org/").unwrap();
        let resp =
            http_get(&url, Some(tls)).unwrap_or_else(|e| panic!("wikipedia fetch failed: {e}"));
        let body = String::from_utf8_lossy(&resp.body);
        assert!(
            body.len() > 10_000,
            "body suspiciously small ({} bytes): {:?}",
            body.len(),
            &body[..body.len().min(400)]
        );
        assert!(
            body.contains("<html") || body.contains("<!DOCTYPE") || body.contains("<!doctype"),
            "unexpected body prefix: {:?}",
            &body[..body.len().min(400)]
        );
        // Real Wikipedia content signal.
        assert!(
            body.contains("Wikipedia"),
            "body did not contain 'Wikipedia'"
        );
    }

    /// Similar live probe against `github.com`, another major CDN that
    /// hard-requires HTTP/2.
    #[test]
    #[ignore]
    fn github_live_h2() {
        use crate::loader::http::http_get;
        use oasis_net::tls_rustls::RustlsTlsProvider;
        use oasis_types::tls::TlsProvider;

        let provider = RustlsTlsProvider::new();
        let tls: &dyn TlsProvider = &provider;
        let url = crate::loader::Url::parse("https://github.com/").unwrap();
        let resp = http_get(&url, Some(tls)).unwrap_or_else(|e| panic!("github fetch failed: {e}"));
        assert_eq!(resp.status, 200);
        assert!(
            resp.body.len() > 5_000,
            "body suspiciously small: {} bytes",
            resp.body.len()
        );
    }
}
