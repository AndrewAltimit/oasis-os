//! Blocking Internet Archive fetch helpers (radio catalogs, TV catalogs,
//! archive track connections). Run on background threads spawned by the
//! radio and TV controllers; never on the frame loop.

use oasis_core::net::StdNetworkBackend;

use crate::app_state;

/// Check that a host belongs to the archive.org family.
///
/// Redirect following in `connect_archive_source` honors arbitrary
/// `Location:` hosts from the server — without this guard a malicious
/// or misconfigured response could steer us at an internal address
/// (SSRF). TLS cert validation does not help here: the attacker could
/// own a perfectly valid cert for their host.
fn is_archive_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "archive.org" || h.ends_with(".archive.org")
}

/// Largest Internet Archive API response read (headers + body). The
/// search and files responses are a few hundred KB at most; a server that
/// keeps sending must not grow the fetch thread's buffer without bound.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Whether `target` can go into an HTTP request line as is: a non-empty
/// origin-form path with no whitespace or control characters. Anything
/// else could end the request line early or inject header lines (CRLF).
fn is_safe_request_target(target: &str) -> bool {
    target.starts_with('/') && !target.bytes().any(|b| b <= b' ' || b == 0x7f)
}

/// Append `data` to `response`, failing once the response would exceed
/// [`MAX_RESPONSE_BYTES`].
fn append_capped(response: &mut Vec<u8>, data: &[u8]) -> std::result::Result<(), String> {
    if response.len().saturating_add(data.len()) > MAX_RESPONSE_BYTES {
        return Err(format!(
            "response larger than {} MiB",
            MAX_RESPONSE_BYTES / (1024 * 1024)
        ));
    }
    response.extend_from_slice(data);
    Ok(())
}

/// Percent-encode an Internet Archive identifier (collection or item id)
/// for a URL path or query. Real identifiers (`[A-Za-z0-9._-]`) pass
/// through unchanged; anything else (spaces, `&`, CR/LF from a crafted
/// station entry or search response) is escaped.
fn encode_ia_id(id: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(id.len());
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Perform a blocking HTTPS GET and return the response body as a string.
///
/// Used for Internet Archive API calls (small JSON responses).
fn https_get_body(
    net_backend: &mut oasis_core::net::StdNetworkBackend,
    tls_provider: &oasis_core::net::RustlsTlsProvider,
    host: &str,
    path: &str,
) -> std::result::Result<String, String> {
    use oasis_core::backend::NetworkBackend;
    use oasis_core::net::TlsProvider;

    log::debug!("HTTPS GET https://{host}{path}");
    if !is_safe_request_target(path) || !is_archive_host(host) {
        return Err(format!("refusing request target {host:?} {path:?}"));
    }

    let tcp = net_backend
        .connect(host, 443)
        .map_err(|e| format!("connect: {e}"))?;

    log::debug!("HTTPS: TCP connected to {host}:443");

    // This is a minimal HTTP/1.1 blocking client. The shared TLS config
    // advertises both `h2` and `http/1.1` so the browser can negotiate
    // HTTP/2 with CDNs that require it; if we used the default `connect_tls`
    // here, archive.org would select `h2` and our `\r\n\r\n` parser would
    // trip on HTTP/2 frames. Force `http/1.1` only.
    let tls_conn = tls_provider
        .connect_tls_with_alpn(tcp, host, &[b"http/1.1"])
        .map_err(|e| format!("TLS: {e}"))?;
    let mut stream = tls_conn.stream;

    log::debug!("HTTPS: TLS handshake complete (alpn={:?})", tls_conn.alpn);

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: OASIS_OS/0.1\r\n\
         Connection: close\r\nAccept: */*\r\n\r\n"
    );
    // One absolute deadline covers the request write and the response:
    // a peer that stops reading or goes silent (WouldBlock forever on the
    // non-blocking socket) must not pin this thread.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let req_bytes = request.as_bytes();
    let mut written = 0;
    while written < req_bytes.len() {
        match stream.write(&req_bytes[written..]) {
            Ok(0) => return Err("write: peer closed".to_string()),
            Ok(n) => written += n,
            Err(e) if e.is_would_block() => {
                if std::time::Instant::now() > deadline {
                    return Err("timeout writing HTTP request".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            },
            Err(e) => return Err(format!("write: {e}")),
        }
    }

    // Read full response (runs on background thread; polls WouldBlock
    // until the deadline).
    let mut buf = [0u8; 8192];
    let mut response = Vec::new();
    loop {
        if std::time::Instant::now() > deadline {
            return Err("timeout reading HTTP response".to_string());
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => append_capped(&mut response, &buf[..n])?,
            Err(e) if e.is_would_block() => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            },
            Err(e) => {
                if !response.is_empty() {
                    break;
                }
                return Err(format!("read: {e}"));
            },
        }
    }

    log::debug!("HTTPS: received {} bytes from {host}{path}", response.len());

    // Split headers from body on raw bytes to avoid UTF-8 lossy offset issues.
    let header_end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "no header/body separator in response".to_string())?;
    let header_bytes = &response[..header_end];
    let header_text = String::from_utf8_lossy(header_bytes);

    // Parse status code from first line.
    if let Some(first_line) = header_text.lines().next()
        && let Some(code_str) = first_line.split_whitespace().nth(1)
        && let Ok(code) = code_str.parse::<u16>()
        && code >= 400
    {
        return Err(format!("HTTP {code}"));
    }

    let body_bytes = &response[header_end + 4..];

    // Decode chunked transfer encoding if present.
    let is_chunked = header_text.lines().any(|l| {
        l.to_ascii_lowercase().starts_with("transfer-encoding:")
            && l.to_ascii_lowercase().contains("chunked")
    });
    let final_body = if is_chunked {
        decode_chunked(body_bytes)
    } else {
        body_bytes.to_vec()
    };
    Ok(String::from_utf8_lossy(&final_body).into_owned())
}

/// Decode HTTP chunked transfer encoding on raw bytes.
///
/// Format: `<hex-size>\r\n<data>\r\n` repeated, terminated by `0\r\n\r\n`.
fn decode_chunked(input: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut pos = 0;
    loop {
        // Skip optional leading \r\n.
        while pos < input.len() && (input[pos] == b'\r' || input[pos] == b'\n') {
            pos += 1;
        }
        if pos >= input.len() {
            break;
        }
        // Read chunk size (hex).
        let size_start = pos;
        while pos < input.len() && input[pos] != b'\r' && input[pos] != b'\n' {
            pos += 1;
        }
        let size_str = std::str::from_utf8(&input[size_start..pos]).unwrap_or("");
        // Chunk size may include extensions after `;` — strip them.
        let hex = size_str.split(';').next().unwrap_or("").trim();
        let chunk_size = match usize::from_str_radix(hex, 16) {
            Ok(0) => break, // Final chunk.
            Ok(n) => n,
            Err(_) => break, // Malformed — return what we have.
        };
        // Skip \r\n after size line.
        if pos < input.len() && input[pos] == b'\r' {
            pos += 1;
        }
        if pos < input.len() && input[pos] == b'\n' {
            pos += 1;
        }
        // Extract chunk data. A crafted size near `usize::MAX` must not
        // overflow (that panicked the fetch thread): malformed, stop.
        let Some(end) = pos.checked_add(chunk_size) else {
            break;
        };
        let end = end.min(input.len());
        result.extend_from_slice(&input[pos..end]);
        pos = end;
    }
    result
}

/// Connect to archive.org over TLS and create an ArchiveSource for the given track.
///
/// Follows up to 3 HTTP redirects per attempt (archive.org returns a 302 to
/// a CDN node like `dn720703.ca.archive.org`). If the CDN responds with a
/// transient 5xx (common — archive.org's CDN intermittently returns 500 on
/// valid files), we retry from archive.org itself: a fresh 302 is typically
/// routed to a different CDN node, which usually succeeds. Polls the source
/// until response headers are parsed before returning.
pub(crate) fn connect_archive_source(
    tls_provider: &oasis_core::net::RustlsTlsProvider,
    track: &oasis_audio::radio::ArchiveTrack,
) -> std::result::Result<Box<dyn oasis_audio::radio::RadioSource + Send>, String> {
    use oasis_audio::radio::RadioSource;
    use oasis_audio::radio::source::SourceState;
    use oasis_core::backend::NetworkBackend;
    use oasis_core::net::TlsProvider;

    let orig_host = "archive.org".to_string();
    let orig_path = oasis_audio::radio::ArchiveCatalog::download_path(track);
    let title = track.title.clone();
    let creator = track.creator.clone();

    log::info!("Connecting to archive source: {orig_host}{orig_path}");
    if !is_safe_request_target(&orig_path) {
        return Err(format!("refusing request target {orig_path:?}"));
    }

    const CDN_RETRIES: usize = 3;
    let mut last_err = String::new();

    'attempt: for attempt in 0..CDN_RETRIES {
        let mut target = oasis_audio::radio::StreamUrl {
            https: true,
            host: orig_host.clone(),
            port: 443,
            path: orig_path.clone(),
        };
        if attempt > 0 {
            log::warn!(
                "Retrying archive source (attempt {}/{CDN_RETRIES}) after: {last_err}",
                attempt + 1,
            );
            // Brief pause before retry so we don't hammer a struggling CDN.
            std::thread::sleep(std::time::Duration::from_millis(500));
        }

        'redirect: for _redirect_num in 0..3 {
            // SSRF guard: only follow redirects that stay within the
            // archive.org domain. The initial `orig_host` is hard-coded,
            // but CDN redirects are honoured verbatim, so a compromised
            // or misbehaving response must not be able to steer us at
            // an internal address.
            if !is_archive_host(&target.host) {
                last_err = format!("redirect to non-archive host rejected: {}", target.host);
                continue 'attempt;
            }
            if !is_safe_request_target(&target.path) {
                last_err = format!("redirect to unsafe path rejected: {:?}", target.path);
                continue 'attempt;
            }
            let host = target.host.clone();
            let mut net_backend = StdNetworkBackend::new();
            let tcp = match net_backend.connect(&host, target.port) {
                Ok(t) => t,
                Err(e) => {
                    // TCP failure on a CDN node is transient — fall back
                    // to `'attempt` so archive.org can re-route us.
                    last_err = format!("connect: {e}");
                    continue 'attempt;
                },
            };
            // Force HTTP/1.1 ALPN: ArchiveSource speaks HTTP/1.1 and the shared
            // TLS config also offers `h2` for the browser, so without this the
            // server may hand us an h2 stream that the source can't parse.
            let stream = match tls_provider.connect_tls_with_alpn(tcp, &host, &[b"http/1.1"]) {
                Ok(s) => s.stream,
                Err(e) => {
                    last_err = format!("TLS: {e}");
                    continue 'attempt;
                },
            };

            let mut source = oasis_audio::radio::ArchiveSource::new(
                stream,
                &target.host_header(),
                &target.path,
                &title,
                &creator,
            );

            // Poll until headers are fully parsed (data arrives, or error/redirect).
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                if std::time::Instant::now() > deadline {
                    last_err = "timeout waiting for response headers".into();
                    continue 'attempt;
                }
                match source.poll() {
                    Ok(Some(chunk)) => {
                        source.push_back_chunk(chunk);
                        log::info!("Archive source connected, audio data flowing");
                        // Wrap in a ThreadedSource so steady-state socket
                        // reads happen on a pump thread with a readahead
                        // queue instead of the frame loop.
                        return Ok(Box::new(oasis_audio::radio::ThreadedSource::spawn_from(
                            Box::new(source),
                        )));
                    },
                    Ok(None) => match source.state() {
                        SourceState::Ended => {
                            last_err = "connection closed before headers".into();
                            continue 'attempt;
                        },
                        SourceState::Error => {
                            last_err = "source error during header parsing".into();
                            continue 'attempt;
                        },
                        _ => {
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        },
                    },
                    Err(e) => {
                        let msg = format!("{e}");
                        // OasisError::Backend("redirect:...") formats as
                        // "backend error: redirect:..." — strip the prefix.
                        let inner = msg.strip_prefix("backend error: ").unwrap_or(&msg);
                        if let Some(url) = inner.strip_prefix("redirect:") {
                            // Every archive.org host serves HTTPS, and this
                            // client only speaks TLS: upgrade an `http://`
                            // CDN Location instead of refusing it as a
                            // downgrade. Relative Locations resolve against
                            // the current host.
                            let url = match url.get(..7) {
                                Some(p) if p.eq_ignore_ascii_case("http://") => {
                                    format!("https://{}", &url[7..])
                                },
                                _ => url.to_string(),
                            };
                            match target.resolve_redirect(&url) {
                                Ok(next) => {
                                    target = next;
                                    continue 'redirect;
                                },
                                Err(e) => {
                                    last_err = format!("bad redirect: {e}");
                                    continue 'attempt;
                                },
                            }
                        }
                        // HTTP 4xx is the caller's problem (bad URL, auth,
                        // etc.) — don't retry those. Everything else is
                        // potentially transient: HTTP 5xx from a flaky
                        // CDN node, TLS alerts, TCP RSTs, or connection
                        // resets mid-header-parse all deserve a re-roll
                        // through archive.org to pick up a different CDN
                        // assignment.
                        if inner.starts_with("HTTP 4") {
                            return Err(msg);
                        }
                        last_err = msg;
                        continue 'attempt;
                    },
                }
            }
        }

        last_err = "too many redirects".into();
    }

    Err(format!(
        "archive CDN unreachable after {CDN_RETRIES} attempts: {last_err}"
    ))
}

/// Fetch catalog and connect to first track on a background thread.
///
/// The identifier may be either an Internet Archive collection (in which case
/// we run an `advancedsearch.php` query for audio items under it) or a single
/// item holding many MP3 files (e.g. `OTRR_This_Is_Your_FBI_Singles`). If the
/// collection search returns no items we fall back to treating the identifier
/// as an item id and pull files from `/metadata/<id>/files` directly.
///
/// Creates its own `StdNetworkBackend` (cheap) so no shared state is needed.
pub(crate) fn fetch_catalog_blocking(
    collection: &str,
    seed: u64,
    tls: &oasis_core::net::RustlsTlsProvider,
) -> std::result::Result<app_state::CatalogFetchResult, String> {
    let mut net = StdNetworkBackend::new();

    log::info!("Fetching catalog for '{collection}'");

    let search_path = format!(
        "/advancedsearch.php?\
         q=collection:{}+AND+mediatype:audio\
         &fl=identifier,title,creator\
         &sort=random&rows=50&output=json",
        encode_ia_id(collection)
    );
    let body = https_get_body(&mut net, tls, "archive.org", &search_path)
        .map_err(|e| format!("search API: {e}"))?;

    let items = oasis_audio::radio::ArchiveCatalog::parse_search_response(&body);
    log::info!("Search returned {} items", items.len());

    let mut catalog = oasis_audio::radio::ArchiveCatalog::new(collection);

    // Collection path: fetch files for up to 5 items returned by search.
    for (item_id, _title, creator) in items.iter().take(5) {
        let fp = oasis_audio::radio::ArchiveCatalog::files_api_path(&encode_ia_id(item_id));
        match https_get_body(&mut net, tls, "archive.org", &fp) {
            Ok(fb) => {
                let tracks =
                    oasis_audio::radio::ArchiveCatalog::parse_files_response(&fb, item_id, creator);
                log::info!("Item '{item_id}': {} MP3 tracks", tracks.len());
                catalog.tracks.extend(tracks);
            },
            Err(e) => {
                log::warn!("Files API for '{item_id}': {e}");
            },
        }
    }

    // Single-item fallback: treat `collection` itself as an IA item id. This
    // is what makes stations like "This Is Your FBI" (an item, not a
    // collection) work.
    if catalog.tracks.is_empty() {
        log::info!("Collection search empty — trying '{collection}' as a single item");
        let fp = oasis_audio::radio::ArchiveCatalog::files_api_path(&encode_ia_id(collection));
        match https_get_body(&mut net, tls, "archive.org", &fp) {
            Ok(fb) => {
                let tracks = oasis_audio::radio::ArchiveCatalog::parse_files_response(
                    &fb, collection, "Unknown",
                );
                log::info!("Item '{collection}': {} MP3 tracks", tracks.len());
                catalog.tracks.extend(tracks);
            },
            Err(e) => {
                log::warn!("Files API for '{collection}': {e}");
            },
        }
    }

    if catalog.tracks.is_empty() {
        return Err("no MP3 files found".to_string());
    }

    catalog.shuffle(seed);

    // Try up to CATALOG_TRACK_RETRIES tracks in the shuffled catalog — if
    // one item's files are on a flaky CDN node, the next item is likely on
    // a different one. Without this, a single bad shuffle (or a file that
    // was de-listed) fails the whole station.
    const CATALOG_TRACK_RETRIES: usize = 5;
    let mut last_err = String::new();
    for _ in 0..CATALOG_TRACK_RETRIES.min(catalog.tracks.len()) {
        let Some(track) = catalog.current_track().cloned() else {
            break;
        };
        match connect_archive_source(tls, &track) {
            Ok(source) => return Ok(app_state::CatalogFetchResult { catalog, source }),
            Err(e) => {
                log::warn!("Track '{}' unreachable: {e}; trying next", track.filename);
                last_err = e;
                catalog.next_track();
            },
        }
    }
    Err(format!("no playable tracks in catalog: {last_err}"))
}

/// Fetch video catalogs for all TV channels on a background thread.
pub(crate) fn fetch_tv_catalogs_blocking(
    channels: &[oasis_apps::tv_guide::Channel],
    tls: &oasis_core::net::RustlsTlsProvider,
) -> std::result::Result<Vec<Option<oasis_apps::tv_guide::ChannelCatalog>>, String> {
    use oasis_apps::tv_guide::catalog::ChannelCatalog;

    log::info!("TV fetch_tv_catalogs_blocking: {} channels", channels.len());

    let mut net = oasis_core::net::StdNetworkBackend::new();
    let mut results = Vec::new();

    for channel in channels {
        log::debug!(
            "TV: fetching CH {} '{}' ({} sources)",
            channel.number,
            channel.call_sign,
            channel.source.len(),
        );
        let mut catalog = ChannelCatalog::new(channel.number);

        for source in &channel.source {
            let files_path = ChannelCatalog::files_api_path(&encode_ia_id(&source.item_id));
            match https_get_body(&mut net, tls, "archive.org", &files_path) {
                Ok(body) => {
                    log::debug!(
                        "TV: source '{}' response: {} bytes",
                        source.item_id,
                        body.len(),
                    );
                    let episodes = ChannelCatalog::parse_files_response(
                        &body,
                        &source.item_id,
                        source.subfolder.as_deref(),
                    );
                    log::info!(
                        "TV item '{}': {} video episodes",
                        source.item_id,
                        episodes.len(),
                    );
                    catalog.add_episodes(episodes);
                },
                Err(e) => {
                    log::warn!("TV files API for '{}': {e}", source.item_id);
                },
            }
        }

        if catalog.episodes.is_empty() {
            log::debug!("TV: CH {} has no episodes", channel.number);
            results.push(None);
        } else {
            log::debug!(
                "TV: CH {} loaded {} episodes ({:.0}s total)",
                channel.number,
                catalog.episodes.len(),
                catalog.total_duration_secs,
            );
            results.push(Some(catalog));
        }
    }

    let loaded = results.iter().filter(|c| c.is_some()).count();
    log::info!(
        "TV fetch_tv_catalogs_blocking done: {loaded}/{} channels loaded",
        results.len(),
    );

    Ok(results)
}

/// Connect to a single archive track on a background thread.
pub(crate) fn connect_archive_track_sync(
    tls: &oasis_core::net::RustlsTlsProvider,
    track: &oasis_audio::radio::ArchiveTrack,
) -> std::result::Result<app_state::TrackFetchResult, String> {
    let source = connect_archive_source(tls, track)?;
    Ok(app_state::TrackFetchResult { source })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunked_decodes_and_stops_on_malformed_sizes() {
        assert_eq!(
            decode_chunked(b"3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n"),
            b"abcde"
        );
        // A size that overflows `pos + size` used to panic the fetch
        // thread; it is malformed now (what came before is kept).
        assert_eq!(decode_chunked(b"1\r\nx\r\nffffffffffffffff\r\nyz"), b"x");
        assert_eq!(decode_chunked(b"zz\r\nabc"), b"");
    }

    #[test]
    fn response_size_is_capped() {
        let mut response = vec![0u8; MAX_RESPONSE_BYTES - 4];
        assert!(append_capped(&mut response, &[1; 4]).is_ok());
        assert_eq!(response.len(), MAX_RESPONSE_BYTES);
        let err = append_capped(&mut response, &[1]).unwrap_err();
        assert!(err.contains("larger than"), "{err}");
        assert_eq!(response.len(), MAX_RESPONSE_BYTES);
    }

    #[test]
    fn identifiers_are_encoded_and_unsafe_targets_refused() {
        assert_eq!(
            encode_ia_id("OTRR_This-Is.Your_FBI"),
            "OTRR_This-Is.Your_FBI"
        );
        assert_eq!(
            encode_ia_id("x HTTP/1.1\r\nHost: evil"),
            "x%20HTTP%2F1.1%0D%0AHost%3A%20evil"
        );
        assert!(is_safe_request_target("/metadata/item/files"));
        assert!(is_safe_request_target(&format!(
            "/advancedsearch.php?q=collection:{}",
            encode_ia_id("a b\r\n")
        )));
        for bad in ["", "metadata", "/a b", "/a\r\nX: y", "/a\0", "/a\x7f"] {
            assert!(!is_safe_request_target(bad), "{bad:?}");
        }
        // Refused before any connection is attempted.
        let tls = oasis_core::net::RustlsTlsProvider::new();
        let mut net = StdNetworkBackend::new();
        let err = https_get_body(&mut net, &tls, "archive.org", "/x\r\nEvil: 1").unwrap_err();
        assert!(err.contains("refusing"), "{err}");
        let err = https_get_body(&mut net, &tls, "127.0.0.1", "/x").unwrap_err();
        assert!(err.contains("refusing"), "{err}");
    }
}
