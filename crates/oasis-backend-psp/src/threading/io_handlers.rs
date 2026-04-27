//! I/O thread: file reads, JPEG decoding, HTTP requests, TV catalog parsing.

use super::{IO_CMD_QUEUE, IO_RESP_QUEUE, IoCmd, IoResponse, TvCatalogRequest, io_log};
use crate::filesystem::decode_jpeg;

// ---------------------------------------------------------------------------
// I/O thread main loop
// ---------------------------------------------------------------------------

/// Dedicated I/O thread: file reads, JPEG decoding, and radio connections.
pub(super) fn io_thread_fn() {
    loop {
        match IO_CMD_QUEUE.pop() {
            Some(IoCmd::LoadTexture { path, max_w, max_h }) => {
                handle_load_texture(path, max_w, max_h);
            },
            Some(IoCmd::ReadFile { path }) => {
                handle_read_file(path);
            },
            Some(IoCmd::HttpGet { url, tag }) => {
                handle_http_get(url, tag);
            },
            Some(IoCmd::RadioConnect { url }) => {
                super::radio::handle_radio_connect(url);
            },
            Some(IoCmd::RadioArchive { collection }) => {
                super::radio::handle_radio_archive(collection);
            },
            Some(IoCmd::TvCatalogFetchBatch { requests }) => {
                handle_tv_catalog_batch(requests);
            },
            Some(IoCmd::VideoDownload { url, dest, tag }) => {
                super::video_download::handle_video_download(url, dest, tag);
            },
            Some(IoCmd::Shutdown) => break,
            None => {
                // Sleep when idle to avoid spinning.
                psp::thread::sleep_ms(10);
            },
        }
    }
}

// ---------------------------------------------------------------------------
// File I/O handlers
// ---------------------------------------------------------------------------

fn handle_load_texture(path: String, max_w: i32, max_h: i32) {
    match psp::io::read_to_vec(&path) {
        Ok(data) => match decode_jpeg(&data, max_w, max_h) {
            Some((w, h, rgba)) => {
                let _ = IO_RESP_QUEUE.push(IoResponse::TextureReady {
                    path,
                    width: w,
                    height: h,
                    rgba,
                });
            },
            None => {
                let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                    path,
                    msg: "JPEG decode failed".into(),
                });
            },
        },
        Err(e) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                path,
                msg: format!("file read failed: {e}"),
            });
        },
    }
}

fn handle_read_file(path: String) {
    match psp::io::read_to_vec(&path) {
        Ok(data) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::FileReady { path, data });
        },
        Err(e) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                path,
                msg: format!("file read failed: {e}"),
            });
        },
    }
}

fn handle_http_get(url: String, tag: u32) {
    // Check connectivity without showing a dialog (must not call
    // ensure_net_init_pub from background thread -- freezes EBOOT).
    if !psp::net::is_connected() {
        let _ = IO_RESP_QUEUE.push(IoResponse::Error {
            path: url,
            msg: "not connected to WiFi".to_string(),
        });
        return;
    }

    let mut url_bytes: Vec<u8> = url.as_bytes().to_vec();
    url_bytes.push(0);

    match psp::http::HttpClient::new() {
        Ok(client) => {
            match client.request(psp::sys::HttpMethod::Get, &url_bytes)
                .timeout(15_000) // 15 second timeout
                .send()
            {
                Ok(resp) => {
                    let _ = IO_RESP_QUEUE.push(IoResponse::HttpDone {
                        tag,
                        status_code: resp.status_code,
                        body: resp.body,
                    });
                },
                Err(e) => {
                    let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                        path: url,
                        msg: format!("HTTP GET: {e}"),
                    });
                },
            }
        },
        Err(e) => {
            let _ = IO_RESP_QUEUE.push(IoResponse::Error {
                path: url,
                msg: format!("HTTP init: {e}"),
            });
        },
    }
}

// ---------------------------------------------------------------------------
// TV catalog handler (I/O thread -- JSON parse off main thread)
// ---------------------------------------------------------------------------

fn handle_tv_catalog_batch(requests: Vec<TvCatalogRequest>) {
    io_log(&format!("[IO-TV] batch: {} requests", requests.len()));

    // Check connectivity without showing a dialog. The WiFi dialog must
    // only be shown from the main thread (it uses GU rendering). Calling
    // ensure_net_init_pub() here would try to show the dialog from the
    // I/O thread, which freezes the EBOOT.
    if !psp::net::is_connected() {
        io_log("[IO-TV] not connected, skipping catalog fetch");
        return;
    }

    let client = match psp::http::HttpClient::new() {
        Ok(c) => c,
        Err(e) => {
            io_log(&format!("[IO-TV] HTTP init failed: {e}"));
            return;
        },
    };

    for req in &requests {
        io_log(&format!("[IO-TV] fetching ch={} {}", req.ch_idx, req.url));

        let mut url_bytes: Vec<u8> = req.url.as_bytes().to_vec();
        url_bytes.push(0);

        let resp = match client
            .request(psp::sys::HttpMethod::Get, &url_bytes)
            .timeout(15_000)
            .send()
        {
            Ok(r) => r,
            Err(e) => {
                io_log(&format!("[IO-TV] GET failed ch={}: {e}", req.ch_idx));
                continue;
            },
        };

        io_log(&format!(
            "[IO-TV] ch={} status={} len={}",
            req.ch_idx,
            resp.status_code,
            resp.body.len()
        ));

        if resp.status_code < 200 || resp.status_code >= 300 {
            continue;
        }

        if resp.body.len() < 256 {
            let preview = String::from_utf8_lossy(&resp.body);
            io_log(&format!("[IO-TV] body: {preview}"));
        }

        // Convert to String and drop the original body to reduce peak memory.
        let body_len = resp.body.len();
        let json = String::from_utf8_lossy(&resp.body).into_owned();
        drop(resp);
        io_log(&format!(
            "[IO-TV] parsing ch={} ({body_len} bytes)...",
            req.ch_idx
        ));
        let episodes = parse_files_lightweight(&json, &req.item_id, req.subfolder.as_deref());
        io_log(&format!(
            "[IO-TV] ch={} parsed {} episodes",
            req.ch_idx,
            episodes.len()
        ));

        let _ = IO_RESP_QUEUE.push(IoResponse::TvCatalogReady {
            ch_idx: req.ch_idx,
            episodes,
        });
    }

    io_log("[IO-TV] batch complete");
}

// ---------------------------------------------------------------------------
// Lightweight JSON parser for archive.org metadata
// ---------------------------------------------------------------------------

/// Extract a JSON string value for the given key from a JSON object substring.
/// Returns the unescaped value or empty string if not found.
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

/// Lightweight archive.org `/metadata/ITEM/files` parser.
///
/// Scans the JSON for file objects without building a full DOM tree.
/// Extracts only MP4/h.264 video entries, matching the same filtering
/// as `ChannelCatalog::parse_files_response` but with O(1) heap overhead.
fn parse_files_lightweight(
    json: &str,
    item_id: &str,
    subfolder: Option<&str>,
) -> Vec<oasis_core::apps::tv_guide::VideoEpisode> {
    // Find the "result" array.
    let result_start = match json.find("\"result\":[") {
        Some(pos) => pos + "\"result\":[".len(),
        None => match json.find("\"result\": [") {
            Some(pos) => pos + "\"result\": [".len(),
            None => return Vec::new(),
        },
    };

    let mut episodes = Vec::new();
    let rest = &json[result_start..];

    // Pre-compute subfolder prefix outside the loop.
    let sf_prefix: Option<String> = subfolder.map(|sf| format!("{sf}/"));

    // Iterate over objects in the array by finding matched { }.
    let mut pos = 0;
    while pos < rest.len() {
        let obj_start = match rest[pos..].find('{') {
            Some(p) => pos + p,
            None => break,
        };
        // Find the matching closing brace. Skip nested braces by tracking
        // depth, ignoring braces inside JSON string literals.
        let mut depth = 0i32;
        let mut obj_end = obj_start;
        let mut in_string = false;
        let mut escape = false;
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
            break; // Malformed JSON.
        }
        let obj = &rest[obj_start..obj_end];
        pos = obj_end;

        let name = extract_json_str(obj, "name");
        if name.is_empty() {
            continue;
        }

        // Quick filter: skip non-video files early (before extracting other fields).
        let format_str = extract_json_str(obj, "format");
        let is_video = format_str.eq_ignore_ascii_case("h.264")
            || format_str.eq_ignore_ascii_case("mpeg4")
            || format_str.eq_ignore_ascii_case("h.264 ia")
            || name.ends_with(".mp4");
        if !is_video {
            continue;
        }

        // Subfolder filter.
        if let Some(ref prefix) = sf_prefix {
            if !name.starts_with(prefix.as_str()) {
                continue;
            }
        }

        // Parse duration — skip files without one.
        let length_str = extract_json_str(obj, "length");
        let duration: f64 = length_str.parse().unwrap_or(0.0);
        if duration <= 0.0 {
            continue;
        }

        let width: u32 = extract_json_str(obj, "width").parse().unwrap_or(0);
        let height: u32 = extract_json_str(obj, "height").parse().unwrap_or(0);
        let size_bytes: u64 = extract_json_str(obj, "size").parse().unwrap_or(0);
        let original = extract_json_str(obj, "original");

        // Derive title from filename.
        let display_name = if let Some(ref prefix) = sf_prefix {
            name.strip_prefix(prefix.as_str()).unwrap_or(name)
        } else {
            name
        };
        let title = display_name
            .strip_suffix(".mp4")
            .or_else(|| display_name.strip_suffix(".MP4"))
            .unwrap_or(display_name)
            .replace('_', " ");

        episodes.push(oasis_core::apps::tv_guide::VideoEpisode {
            item_id: item_id.to_string(),
            filename: name.to_string(),
            title,
            duration_secs: duration,
            width,
            height,
            size_bytes,
            format: format_str.into(),
            original: if original.is_empty() {
                None
            } else {
                Some(original.to_string())
            },
        });

        // Cap at 50 episodes per channel to limit memory.
        if episodes.len() >= 50 {
            break;
        }
    }

    episodes
}
