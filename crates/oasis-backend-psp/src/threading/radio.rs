//! Internet radio connection handler (I/O thread).
//!
//! Two paths:
//! - **icecast** — raw TCP + HTTP/1.0 with ICY metadata interleaving. Hands
//!   a socket fd to the audio thread (legacy SomaFM-style streams).
//! - **archive** — Internet Archive collections over HTTPS. Performs a
//!   search → first item → first MP3 lookup, then streams the MP3 chunks
//!   into `RADIO_DATA_QUEUE` for the audio thread to consume. Mirrors the
//!   desktop / WASM `archive` flow.

use core::sync::atomic::Ordering;

use super::{
    IO_RESP_QUEUE, IoResponse, RADIO_CANCEL, RADIO_DATA_QUEUE, RADIO_STOPPED,
    find_header_end, parse_icy_metaint, parse_radio_url,
};
use super::tls_http::TlsHttpReader;

/// Diagnostic logger that won't allocate during decode-hot paths. Mirrors
/// `io_log` from `threading/mod.rs` but local to keep this module loosely
/// coupled.
fn radio_log(msg: &str) {
    // SAFETY: file I/O via raw sceIo* with valid pointers.
    unsafe {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            psp::sys::sceIoWrite(fd, msg.as_ptr() as *const _, msg.len());
            psp::sys::sceIoWrite(fd, b"\n".as_ptr() as *const _, 1);
            psp::sys::sceIoClose(fd);
        }
    }
}

/// Read the body of a TLS HTTP response into a `String`, capped at `max_bytes`.
///
/// The metadata endpoint for collections like `the_shadow_season_*` can run
/// to several hundred KB (one entry per file × dozens of episodes), so we
/// stop early as soon as we have enough to find the first MP3. Streaming
/// reads at 2 s recv timeout per syscall means a 256 KB body could take
/// over a minute to drain — the cap bounds that to a few seconds.
fn read_tls_body_capped(
    reader: &mut TlsHttpReader,
    max_bytes: usize,
) -> Result<String, String> {
    let mut body = Vec::with_capacity(max_bytes.min(32 * 1024));
    let mut buf = [0u8; 4096];
    while body.len() < max_bytes {
        let n = reader.read_data(&mut buf)?;
        if n <= 0 {
            break;
        }
        let len = n as usize;
        let room = max_bytes - body.len();
        let take = len.min(room);
        body.extend_from_slice(&buf[..take]);
        if take < len {
            break;
        }
    }
    String::from_utf8(body).map_err(|e| format!("body utf8: {e}"))
}

/// Find the first `identifier` field in an Internet Archive
/// `advancedsearch.php` response. Returns the value without parsing the
/// full JSON tree (works on the same stripped-down basis as the TV Guide
/// `parse_files_lightweight`).
fn find_first_identifier(json: &str) -> Option<&str> {
    // The search endpoint nests results under `response.docs[]` with each
    // doc containing an `identifier`. Just locate the first occurrence of
    // `"identifier":"…"` since that's the field we care about.
    let needle = "\"identifier\":\"";
    let start = json.find(needle)? + needle.len();
    let end = json[start..].find('"')?;
    Some(&json[start..start + end])
}

/// Find the first MP3 file in an Internet Archive `metadata/<item>/files`
/// response. Returns its `name` (relative path within the item).
fn find_first_mp3_filename(json: &str) -> Option<String> {
    // The files endpoint returns `{"result":[{file_objects}…]}`. We scan
    // each file object, picking the first one whose `format` field
    // contains "MP3" (covers "VBR MP3", "128Kbps MP3", etc.).
    let result_marker = "\"result\":[";
    let result_start = json.find(result_marker).map(|p| p + result_marker.len())?;
    let rest = &json[result_start..];

    let mut pos = 0usize;
    while pos < rest.len() {
        let obj_start = rest[pos..].find('{').map(|p| pos + p)?;
        // Find matching `}` accounting for nested structures and string
        // literals (escape sequences honored).
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escape = false;
        let mut obj_end = obj_start;
        for (i, b) in rest[obj_start..].bytes().enumerate() {
            if escape {
                escape = false;
                continue;
            }
            match b {
                b'\\' if in_string => escape = true,
                b'"' => in_string = !in_string,
                b'{' if !in_string => depth += 1,
                b'}' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        obj_end = obj_start + i + 1;
                        break;
                    }
                },
                _ => {},
            }
        }
        if depth != 0 {
            return None;
        }
        let obj = &rest[obj_start..obj_end];
        pos = obj_end;

        // Extract `format` and `name` using the same flat string approach
        // as `extract_json_str` in `io_handlers.rs`.
        let name = extract_json_str(obj, "name");
        if name.is_empty() {
            continue;
        }
        let format = extract_json_str(obj, "format");
        let is_mp3 = format.to_ascii_uppercase().contains("MP3")
            || name.to_ascii_lowercase().ends_with(".mp3");
        if is_mp3 {
            return Some(name.to_string());
        }
    }
    None
}

/// Extract a JSON string field from an object. Local copy of the same
/// helper used by `io_handlers::parse_files_lightweight` to keep this
/// module self-contained.
fn extract_json_str<'a>(obj: &'a str, key: &str) -> &'a str {
    let needle = format!("\"{}\":\"", key);
    if let Some(start) = obj.find(&needle) {
        let val_start = start + needle.len();
        if let Some(end) = obj[val_start..].find('"') {
            return &obj[val_start..val_start + end];
        }
    }
    ""
}

/// Percent-encode a path segment for use in an HTTP URL. Mirrors
/// `oasis_audio::radio::archive::percent_encode` (kept local so this
/// module stays compile-light).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b' ' => out.push_str("%20"),
            b'#' => out.push_str("%23"),
            b'?' => out.push_str("%3F"),
            b'&' => out.push_str("%26"),
            b'%' => out.push_str("%25"),
            b'+' => out.push_str("%2B"),
            0x80.. => {
                out.push('%');
                out.push(char::from(HEX[(b >> 4) as usize]));
                out.push(char::from(HEX[(b & 0xF) as usize]));
            },
            _ => out.push(char::from(b)),
        }
    }
    out
}
const HEX: [u8; 16] = *b"0123456789ABCDEF";

/// Resolve an Internet Archive collection to a single MP3 stream and pump
/// the bytes into `RADIO_DATA_QUEUE` for the audio thread. Stops cleanly
/// when `RADIO_CANCEL` is set, EOF arrives, or a fatal HTTPS error occurs.
pub(super) fn handle_radio_archive(collection: String) {
    RADIO_CANCEL.store(false, Ordering::Release);
    RADIO_STOPPED.store(false, Ordering::Release);

    // Drain anything left over from a previous stream so the audio thread
    // doesn't decode stale bytes after a station change.
    while RADIO_DATA_QUEUE.pop().is_some() {}

    radio_log(&format!("[IO-RADIO] resolving archive collection: {collection}"));

    if !psp::net::is_connected() {
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: "not connected to WiFi".to_string(),
        });
        RADIO_STOPPED.store(true, Ordering::Release);
        return;
    }

    // Step 1: pick a random item from the collection. The `sort=random`
    // hint asks the API to shuffle for us; rows=1 keeps the response tiny.
    let search_url = format!(
        "https://archive.org/advancedsearch.php?\
         q=collection:{collection}+AND+mediatype:audio\
         &fl=identifier&sort=random&rows=1&output=json"
    );
    radio_log(&format!("[IO-RADIO] search GET: {search_url}"));
    let mut search_reader = match TlsHttpReader::open(&search_url) {
        Ok((r, _)) => r,
        Err(e) => {
            let msg = format!("search: {e}");
            radio_log(&format!("[IO-RADIO] {msg}"));
            let _ = IO_RESP_QUEUE.push(IoResponse::RadioError { msg });
            RADIO_STOPPED.store(true, Ordering::Release);
            return;
        },
    };
    // Search response with rows=1 is small (~1 KB); 8 KB is plenty.
    let search_body = match read_tls_body_capped(&mut search_reader, 8 * 1024) {
        Ok(b) => b,
        Err(e) => {
            search_reader.cleanup();
            let msg = format!("search read: {e}");
            radio_log(&format!("[IO-RADIO] {msg}"));
            let _ = IO_RESP_QUEUE.push(IoResponse::RadioError { msg });
            RADIO_STOPPED.store(true, Ordering::Release);
            return;
        },
    };
    search_reader.cleanup();
    let item_id = match find_first_identifier(&search_body) {
        Some(id) => id.to_string(),
        None => {
            radio_log("[IO-RADIO] no items in search result");
            let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
                msg: format!("collection {collection} has no audio items"),
            });
            RADIO_STOPPED.store(true, Ordering::Release);
            return;
        },
    };
    radio_log(&format!("[IO-RADIO] item: {item_id}"));

    // Step 2: list the files for that item and pick an MP3.
    let files_url = format!("https://archive.org/metadata/{item_id}/files");
    radio_log(&format!("[IO-RADIO] files GET: {files_url}"));
    let mut files_reader = match TlsHttpReader::open(&files_url) {
        Ok((r, _)) => r,
        Err(e) => {
            let msg = format!("files: {e}");
            radio_log(&format!("[IO-RADIO] {msg}"));
            let _ = IO_RESP_QUEUE.push(IoResponse::RadioError { msg });
            RADIO_STOPPED.store(true, Ordering::Release);
            return;
        },
    };
    // Files response can be 100s of KB on multi-episode items. Cap at 32 KB:
    // the first MP3 entry is reliably within the first few KB.
    let files_body = match read_tls_body_capped(&mut files_reader, 32 * 1024) {
        Ok(b) => b,
        Err(e) => {
            files_reader.cleanup();
            let msg = format!("files read: {e}");
            radio_log(&format!("[IO-RADIO] {msg}"));
            let _ = IO_RESP_QUEUE.push(IoResponse::RadioError { msg });
            RADIO_STOPPED.store(true, Ordering::Release);
            return;
        },
    };
    files_reader.cleanup();
    let mp3_name = match find_first_mp3_filename(&files_body) {
        Some(n) => n,
        None => {
            radio_log("[IO-RADIO] no MP3 in item files");
            let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
                msg: format!("no MP3 in {item_id}"),
            });
            RADIO_STOPPED.store(true, Ordering::Release);
            return;
        },
    };
    radio_log(&format!("[IO-RADIO] mp3: {mp3_name}"));

    // Step 3: stream the MP3 file. Push chunks into RADIO_DATA_QUEUE for
    // the audio thread to consume. Send `RadioConnected` with an empty
    // initial buffer + icy_metaint=0 so the audio thread spins up its
    // queue-fed decoder.
    let mp3_url = format!(
        "https://archive.org/download/{item_id}/{}",
        percent_encode(&mp3_name)
    );
    radio_log(&format!("[IO-RADIO] streaming GET: {mp3_url}"));
    let mut mp3_reader = match TlsHttpReader::open(&mp3_url) {
        Ok((r, _)) => r,
        Err(e) => {
            let msg = format!("mp3: {e}");
            radio_log(&format!("[IO-RADIO] {msg}"));
            let _ = IO_RESP_QUEUE.push(IoResponse::RadioError { msg });
            RADIO_STOPPED.store(true, Ordering::Release);
            return;
        },
    };

    // Notify the main / audio thread that streaming is starting. The
    // audio thread will drive RADIO_DATA_QUEUE.
    let _ = IO_RESP_QUEUE.push(IoResponse::RadioConnected {
        fd: -1, // sentinel: queue-fed source
        icy_metaint: 0,
        initial_data: Vec::new(),
    });

    // Pump bytes from TLS into the queue. Each chunk is a heap-allocated
    // Vec; the audio thread takes ownership.
    let mut buf = [0u8; 16 * 1024];
    loop {
        if RADIO_CANCEL.load(Ordering::Acquire) {
            radio_log("[IO-RADIO] cancel requested, stopping stream");
            break;
        }
        match mp3_reader.read_data(&mut buf) {
            Ok(0) => {
                radio_log("[IO-RADIO] mp3 EOF");
                break;
            },
            Ok(n) if n > 0 => {
                let chunk = buf[..n as usize].to_vec();
                // Spin until queue has space (audio thread is consuming).
                let mut item = chunk;
                while !RADIO_CANCEL.load(Ordering::Acquire) {
                    match RADIO_DATA_QUEUE.push(item) {
                        Ok(()) => break,
                        Err(returned) => {
                            item = returned;
                            psp::thread::sleep_ms(5);
                        },
                    }
                }
            },
            Ok(_) => {
                // Negative result: treat as EOF.
                break;
            },
            Err(e) => {
                radio_log(&format!("[IO-RADIO] read error: {e}"));
                break;
            },
        }
    }

    mp3_reader.cleanup();
    RADIO_STOPPED.store(true, Ordering::Release);
    radio_log("[IO-RADIO] handle_radio_archive exit");
}

/// Connect to an internet radio stream via raw TCP + HTTP.
///
/// Sends an HTTP GET with `Icy-MetaData: 1`, reads headers to extract
/// `icy-metaint`, then passes the connected socket fd to the audio thread.
pub(super) fn handle_radio_connect(url: String) {
    use std::ffi::c_void;

    // Check connectivity without showing a dialog (must not call
    // ensure_net_init_pub from background thread -- freezes EBOOT).
    if !psp::net::is_connected() {
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: "not connected to WiFi".to_string(),
        });
        return;
    }

    // Parse URL.
    let (host, port, path) = match parse_radio_url(&url) {
        Some(v) => v,
        None => {
            let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
                msg: format!("bad URL: {url}"),
            });
            return;
        },
    };

    // DNS resolve.
    let mut host_bytes: Vec<u8> = host.as_bytes().to_vec();
    host_bytes.push(0);
    let addr = match psp::net::resolve_hostname(&host_bytes) {
        Ok(a) => a,
        Err(e) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
                msg: format!("DNS: {e}"),
            });
            return;
        },
    };

    // Create TCP socket.
    // SAFETY: AF_INET=2, SOCK_STREAM=1, protocol=0.
    let fd = unsafe { psp::sys::sceNetInetSocket(2, 1, 0) };
    if fd < 0 {
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: "socket() failed".into(),
        });
        return;
    }

    // Connect.
    let sa = crate::network::make_sockaddr_in_pub(addr.0, port);
    // SAFETY: Connect to the resolved address.
    let ret = unsafe {
        psp::sys::sceNetInetConnect(fd, &sa, core::mem::size_of::<psp::sys::sockaddr>() as u32)
    };
    if ret < 0 {
        // SAFETY: fd is a valid socket descriptor; closing on connect failure.
        unsafe { psp::sys::sceNetInetClose(fd) };
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: format!("connect {}:{} failed", host, port),
        });
        return;
    }

    // Send HTTP GET with ICY metadata request.
    let request = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nIcy-MetaData: 1\r\n\
         User-Agent: OASIS_OS/1.0\r\nAccept: */*\r\n\r\n",
        path, host,
    );
    let req_bytes = request.as_bytes();
    // SAFETY: Send the HTTP request over the connected socket.
    let sent = unsafe {
        psp::sys::sceNetInetSend(fd, req_bytes.as_ptr() as *const c_void, req_bytes.len(), 0)
    };
    if sent <= 0 {
        // SAFETY: fd is a valid socket descriptor; closing on send failure.
        unsafe { psp::sys::sceNetInetClose(fd) };
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: "send failed".into(),
        });
        return;
    }

    // Read response headers (up to 4KB).
    let mut hdr_buf = vec![0u8; 4096];
    let mut hdr_len = 0usize;
    let mut attempts = 0;
    while hdr_len < hdr_buf.len() && attempts < 200 {
        // SAFETY: Blocking recv for header data.
        let n = unsafe {
            psp::sys::sceNetInetRecv(
                fd,
                hdr_buf.as_mut_ptr().add(hdr_len) as *mut c_void,
                (hdr_buf.len() - hdr_len).min(512),
                0,
            )
        };
        if n > 0 {
            hdr_len += n as usize;
            // Check for end of headers.
            if hdr_len >= 4 {
                let search_start = if hdr_len > n as usize + 3 {
                    hdr_len - n as usize - 3
                } else {
                    0
                };
                let haystack = &hdr_buf[search_start..hdr_len];
                if find_header_end(haystack).is_some() {
                    break;
                }
            }
        } else if n == 0 {
            break; // Connection closed.
        } else {
            attempts += 1;
            psp::thread::sleep_ms(20);
        }
    }

    // Validate that we received a complete header (with \r\n\r\n terminator).
    let header_end = if hdr_len > 0 {
        find_header_end(&hdr_buf[..hdr_len])
    } else {
        None
    };

    let Some(header_end) = header_end else {
        // SAFETY: fd is a valid socket descriptor; closing on incomplete headers.
        unsafe { psp::sys::sceNetInetClose(fd) };
        let _ = IO_RESP_QUEUE.push(IoResponse::RadioError {
            msg: "incomplete headers".into(),
        });
        return;
    };

    // Parse icy-metaint from headers.
    let hdr_str = String::from_utf8_lossy(&hdr_buf[..hdr_len]);
    let icy_metaint = parse_icy_metaint(&hdr_str);

    // Extract any leftover audio data after the header boundary.
    let initial_data = hdr_buf[header_end..hdr_len].to_vec();

    // Set non-blocking for streaming.
    // PSP socket constants: SOL_SOCKET=0xFFFF, SO_NONBLOCK=0x0080
    // (see network.rs for full documentation of PSP-specific values).
    let nb: i32 = 1;
    // SAFETY: SO_NONBLOCK is a PSP-specific socket option.
    unsafe {
        psp::sys::sceNetInetSetsockopt(
            fd,
            crate::network::PSP_SOL_SOCKET,
            crate::network::PSP_SO_NONBLOCK,
            &nb as *const i32 as *const c_void,
            core::mem::size_of::<i32>() as u32,
        );
    }

    let _ = IO_RESP_QUEUE.push(IoResponse::RadioConnected {
        fd,
        icy_metaint,
        initial_data,
    });
}
