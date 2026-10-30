//! Stream URL parsing and redirect-following connect for Icecast sources.
//!
//! Icecast / Shoutcast directories frequently hand out URLs that answer
//! with a 3xx (`http://` → `https://` upgrades, load-balancer hops to a
//! relay node). [`connect_icecast`] connects, waits for the response head
//! and follows up to [`MAX_REDIRECTS`] redirects under a strict policy:
//! same-scheme or `http` → `https` only, never `https` → `http` (that would
//! silently drop TLS on a stream the user asked for over HTTPS).
//!
//! The transport is supplied by the caller (`connect` closure: TCP + TLS
//! as appropriate), so this module stays backend-agnostic.

use std::time::{Duration, Instant};

use oasis_types::backend::NetworkStream;

use super::source::{IcecastSource, RadioSource, SourceState};

/// Maximum number of redirects followed before giving up.
pub const MAX_REDIRECTS: usize = 5;

/// How long to wait for a response head on each hop.
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(15);

/// A parsed `http://` / `https://` stream URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamUrl {
    /// `true` for `https://`.
    pub https: bool,
    /// Host name or address (no port, no brackets).
    pub host: String,
    /// TCP port (scheme default when the URL has none).
    pub port: u16,
    /// Path plus query, always starting with `/`.
    pub path: String,
}

impl StreamUrl {
    /// Parse an absolute `http://` or `https://` URL. Returns `None` for any
    /// other scheme, an empty host, or a bad port.
    pub fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        let (https, rest) = if let Some(r) = strip_prefix_ci(url, "https://") {
            (true, r)
        } else {
            (false, strip_prefix_ci(url, "http://")?)
        };
        let (authority, path) = match rest.find(['/', '?']) {
            Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], rest[i..].to_string()),
            Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
            None => (rest, "/".to_string()),
        };
        // Drop any fragment; it is never sent to the server.
        let path = match path.split_once('#') {
            Some((p, _)) => p.to_string(),
            None => path,
        };
        // Userinfo is not supported (and would be an SSRF/phishing vector).
        if authority.contains('@') {
            return None;
        }
        let default_port = if https { 443 } else { 80 };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => {
                (h.to_string(), p.parse().ok()?)
            },
            _ => (authority.to_string(), default_port),
        };
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        if host.is_empty() {
            return None;
        }
        Some(Self {
            https,
            host,
            port,
            path,
        })
    }

    /// `Host` header value: the host, plus the port when it is not the
    /// scheme default.
    pub fn host_header(&self) -> String {
        let default = if self.https { 443 } else { 80 };
        if self.port == default {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// Resolve a `Location` header against this URL and apply the redirect
    /// policy: absolute `http(s)://` targets and absolute paths (`/x`) are
    /// accepted; `https` → `http` downgrades are refused.
    pub fn resolve_redirect(&self, location: &str) -> Result<Self, String> {
        let location = location.trim();
        let target = if location.starts_with("//") {
            // Scheme-relative: keep our scheme.
            let scheme = if self.https { "https:" } else { "http:" };
            Self::parse(&format!("{scheme}{location}"))
        } else if location.starts_with('/') {
            Some(Self {
                path: location.to_string(),
                ..self.clone()
            })
        } else {
            Self::parse(location)
        }
        .ok_or_else(|| format!("unsupported redirect target: {location}"))?;
        if self.https && !target.https {
            return Err(format!("refusing https -> http redirect to {location}"));
        }
        Ok(target)
    }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

/// Connect to an Icecast/Shoutcast stream, following redirects.
///
/// `connect` opens the transport for each hop (plain TCP for `http`, TCP +
/// TLS for `https`). Returns a source whose response head has been parsed
/// (2xx), with any audio bytes that arrived alongside the head preserved
/// for the first `poll()`.
pub fn connect_icecast<F>(url: &str, connect: F) -> Result<IcecastSource, String>
where
    F: FnMut(&StreamUrl) -> Result<Box<dyn NetworkStream>, String>,
{
    connect_icecast_with(url, MAX_REDIRECTS, HEADER_TIMEOUT, connect)
}

/// [`connect_icecast`] with explicit redirect and header-timeout limits.
pub fn connect_icecast_with<F>(
    url: &str,
    max_redirects: usize,
    header_timeout: Duration,
    mut connect: F,
) -> Result<IcecastSource, String>
where
    F: FnMut(&StreamUrl) -> Result<Box<dyn NetworkStream>, String>,
{
    let mut target = StreamUrl::parse(url).ok_or_else(|| format!("invalid stream URL: {url}"))?;
    for _hop in 0..=max_redirects {
        let stream = connect(&target)?;
        let mut src = IcecastSource::new(stream, &target.host_header(), &target.path);
        let deadline = Instant::now() + header_timeout;
        loop {
            match src.poll() {
                Ok(Some(chunk)) => {
                    src.push_back_chunk(chunk);
                    return Ok(src);
                },
                Ok(None) if src.headers_parsed() => return Ok(src),
                Ok(None) => match src.state() {
                    SourceState::Ended => return Err("connection closed before headers".into()),
                    SourceState::Error => return Err("stream error before headers".into()),
                    _ => {
                        if Instant::now() >= deadline {
                            src.disconnect();
                            return Err("timeout waiting for response headers".into());
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    },
                },
                Err(e) => match src.redirect_location() {
                    Some(loc) => {
                        let next = target.resolve_redirect(loc)?;
                        log::info!("radio: redirect {} -> {loc}", target.host);
                        target = next;
                        break;
                    },
                    None => {
                        let msg = format!("{e}");
                        let msg = msg.strip_prefix("backend error: ").unwrap_or(&msg);
                        return Err(msg.to_string());
                    },
                },
            }
        }
    }
    Err(format!("too many redirects (> {max_redirects})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_urls() {
        let u = StreamUrl::parse("http://ice.example.com/live").unwrap();
        assert_eq!(
            u,
            StreamUrl {
                https: false,
                host: "ice.example.com".into(),
                port: 80,
                path: "/live".into(),
            }
        );
        let u = StreamUrl::parse("HTTPS://ice.example.com:8443").unwrap();
        assert!(u.https);
        assert_eq!((u.port, u.path.as_str()), (8443, "/"));
        assert_eq!(u.host_header(), "ice.example.com:8443");
        let u = StreamUrl::parse("http://h?x=1#frag").unwrap();
        assert_eq!(u.path, "/?x=1");
        let u = StreamUrl::parse("http://[::1]:8000/s").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("::1", 8000));
    }

    #[test]
    fn parse_rejects_bad_urls() {
        assert!(StreamUrl::parse("ftp://h/x").is_none());
        assert!(StreamUrl::parse("http:///x").is_none());
        assert!(StreamUrl::parse("http://h:notaport/").is_none());
        assert!(StreamUrl::parse("http://user@h/").is_none());
    }

    #[test]
    fn redirect_policy() {
        let http = StreamUrl::parse("http://a.example/s").unwrap();
        let https = StreamUrl::parse("https://a.example/s").unwrap();
        // Upgrade and same-scheme are fine.
        assert!(http.resolve_redirect("https://b.example/s").unwrap().https);
        assert!(!http.resolve_redirect("http://b.example/s").unwrap().https);
        assert!(https.resolve_redirect("https://b.example/s").is_ok());
        // Downgrade is refused, including via a scheme-relative URL's base.
        assert!(https.resolve_redirect("http://b.example/s").is_err());
        // Relative forms keep scheme, host and port.
        let r = https.resolve_redirect("/other?q=1").unwrap();
        assert_eq!(
            (r.host.as_str(), r.path.as_str(), r.https),
            ("a.example", "/other?q=1", true)
        );
        let r = https.resolve_redirect("//c.example:9000/x").unwrap();
        assert_eq!(
            (r.host.as_str(), r.port, r.https),
            ("c.example", 9000, true)
        );
        // Unsupported schemes are refused.
        assert!(http.resolve_redirect("ftp://c.example/").is_err());
    }

    // ---------------------------------------------------------------
    // Loopback servers
    // ---------------------------------------------------------------

    /// Accept one connection per response from `make(port)`, read its
    /// request head, write the response and (unless `hold` is set) close.
    /// Returns the port and the request heads.
    fn serve(
        hold: Option<std::sync::mpsc::Receiver<()>>,
        make: impl FnOnce(u16) -> Vec<Vec<u8>>,
    ) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let responses = make(port);
        let handle = std::thread::spawn(move || {
            let mut heads = Vec::new();
            for response in responses {
                let (mut conn, _) = listener.accept().unwrap();
                let mut req = Vec::new();
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = conn.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                heads.push(String::from_utf8_lossy(&req).into_owned());
                conn.write_all(&response).unwrap();
                if let Some(ref hold) = hold {
                    // Keep the connection open and silent.
                    let _ = hold.recv();
                }
            }
            heads
        });
        (port, handle)
    }

    /// Plain-TCP transport (ignores `https`: the loopback servers do not
    /// speak TLS, but the redirect policy only looks at the URL scheme).
    fn tcp(target: &StreamUrl) -> Result<Box<dyn NetworkStream>, String> {
        use oasis_types::backend::NetworkBackend;
        oasis_net::StdNetworkBackend::new()
            .connect(&target.host, target.port)
            .map_err(|e| e.to_string())
    }

    fn drain(src: &mut IcecastSource) -> Vec<u8> {
        let mut out = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match src.poll() {
                Ok(Some(c)) => out.extend_from_slice(&c.data),
                Ok(None) if src.state() == SourceState::Ended => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(2)),
                Err(e) => panic!("stream error: {e}"),
            }
        }
        out
    }

    fn audio_response(len: usize) -> Vec<u8> {
        let mut r = b"HTTP/1.0 200 OK\r\nContent-Type: audio/mpeg\r\n\r\n".to_vec();
        r.extend(std::iter::repeat_n(0x5A, len));
        r
    }

    #[test]
    fn follows_redirect_chain_to_audio() {
        // Absolute 302, then a relative 301, then the audio.
        let (port, server) = serve(None, |port| {
            vec![
                format!("HTTP/1.0 302 Found\r\nLocation: http://127.0.0.1:{port}/relay\r\n\r\n")
                    .into_bytes(),
                b"HTTP/1.1 301 Moved\r\nLocation: /final?x=1\r\nContent-Length: 0\r\n\r\n".to_vec(),
                audio_response(300),
            ]
        });
        let mut src = connect_icecast(&format!("http://127.0.0.1:{port}/live"), tcp).unwrap();
        // Every audio byte survives, including any that arrived with the head.
        assert_eq!(drain(&mut src), vec![0x5A; 300]);
        let heads = server.join().unwrap();
        assert_eq!(heads.len(), 3);
        assert!(heads[0].starts_with("GET /live HTTP/1.0"), "{}", heads[0]);
        assert!(heads[1].starts_with("GET /relay HTTP/1.0"), "{}", heads[1]);
        assert!(
            heads[2].starts_with("GET /final?x=1 HTTP/1.0"),
            "{}",
            heads[2]
        );
        assert!(
            heads[2].contains(&format!("Host: 127.0.0.1:{port}")),
            "{}",
            heads[2]
        );
    }

    #[test]
    fn redirect_loop_is_bounded() {
        let (port, server) = serve(None, |port| {
            let hop =
                format!("HTTP/1.0 302 Found\r\nLocation: http://127.0.0.1:{port}/again\r\n\r\n");
            vec![hop.into_bytes(); 3]
        });
        let url = format!("http://127.0.0.1:{port}/live");
        let err = connect_icecast_with(&url, 2, HEADER_TIMEOUT, tcp).unwrap_err();
        assert!(err.contains("too many redirects"), "{err}");
        assert_eq!(server.join().unwrap().len(), 3);
    }

    #[test]
    fn https_to_http_redirect_is_refused() {
        let (port, server) = serve(None, |port| {
            vec![
                format!("HTTP/1.0 302 Found\r\nLocation: http://127.0.0.1:{port}/plain\r\n\r\n")
                    .into_bytes(),
            ]
        });
        let mut hops = Vec::new();
        let err = connect_icecast(&format!("https://127.0.0.1:{port}/live"), |t| {
            hops.push(t.clone());
            tcp(t)
        })
        .unwrap_err();
        assert!(err.contains("https -> http"), "{err}");
        assert_eq!(
            hops.len(),
            1,
            "the downgrade target must never be connected"
        );
        server.join().unwrap();
    }

    #[test]
    fn http_error_is_reported_not_followed() {
        let (port, server) = serve(None, |_| vec![b"HTTP/1.0 404 Not Found\r\n\r\n".to_vec()]);
        let err = connect_icecast(&format!("http://127.0.0.1:{port}/x"), tcp).unwrap_err();
        assert_eq!(err, "HTTP 404");
        server.join().unwrap();
    }

    #[test]
    fn silent_server_times_out_waiting_for_headers() {
        // The server reads the request and then says nothing, keeping the
        // socket open: the connect must give up instead of hanging.
        let (release, hold) = std::sync::mpsc::channel();
        let (port, server) = serve(Some(hold), |_| vec![Vec::new()]);
        let started = Instant::now();
        let url = format!("http://127.0.0.1:{port}/live");
        let err = connect_icecast_with(&url, 0, Duration::from_millis(300), tcp).unwrap_err();
        assert!(err.contains("timeout"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
        release.send(()).unwrap();
        server.join().unwrap();
    }
}
