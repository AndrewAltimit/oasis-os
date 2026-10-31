//! Internet Archive catalog: data types and JSON parsing.
//!
//! Shared between SDL and WASM backends. Contains no network I/O — only
//! pure data transformation for IA API responses.

/// A single track from an Internet Archive item.
#[derive(Debug, Clone)]
pub struct ArchiveTrack {
    /// IA item identifier.
    pub item_id: String,
    /// MP3 filename within the item.
    pub filename: String,
    /// Display title.
    pub title: String,
    /// Artist/creator.
    pub creator: String,
}

/// Percent-encode a filename for use in HTTP paths/URLs.
///
/// Encodes characters that are unsafe in URL path segments: space, `#`, `?`,
/// `&`, `%`, `+`, control bytes (CR / LF in a filename would otherwise end
/// the request line) and bytes >= 127.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b' ' => out.push_str("%20"),
            b'#' => out.push_str("%23"),
            b'?' => out.push_str("%3F"),
            b'&' => out.push_str("%26"),
            b'%' => out.push_str("%25"),
            b'+' => out.push_str("%2B"),
            ..b' ' | 0x7F.. => {
                out.push('%');
                out.push(char::from(HEX[b as usize >> 4]));
                out.push(char::from(HEX[b as usize & 0xF]));
            },
            _ => out.push(char::from(b)),
        }
    }
    out
}

const HEX: [u8; 16] = *b"0123456789ABCDEF";

/// Whether `id` is a well-formed Internet Archive item identifier: ASCII
/// letters, digits, `.`, `_` and `-`, not starting with `.` (so never `..`).
fn is_item_identifier(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A browsable catalog of tracks from an IA collection.
pub struct ArchiveCatalog {
    /// IA collection identifier.
    pub collection: String,
    /// All discovered tracks.
    pub tracks: Vec<ArchiveTrack>,
    /// Current playback position.
    pub current: usize,
}

impl ArchiveCatalog {
    /// Create a new catalog for the given collection.
    pub fn new(collection: &str) -> Self {
        Self {
            collection: collection.to_string(),
            tracks: Vec::new(),
            current: 0,
        }
    }

    /// Parse the IA advanced search API JSON response.
    ///
    /// Returns a list of `(identifier, title, creator)` tuples.
    pub fn parse_search_response(json: &str) -> Vec<(String, String, String)> {
        let parsed: serde_json::Value = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        let docs = match parsed.get("response").and_then(|r| r.get("docs")) {
            Some(serde_json::Value::Array(arr)) => arr,
            _ => return Vec::new(),
        };
        let mut results = Vec::new();
        for doc in docs {
            let identifier = doc
                .get("identifier")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = doc
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let creator = doc
                .get("creator")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string();
            // The identifier is spliced into request paths unencoded, so
            // anything but an IA identifier (`[A-Za-z0-9._-]`) is dropped.
            if is_item_identifier(&identifier) {
                results.push((identifier, title, creator));
            }
        }
        results
    }

    /// Parse the IA metadata/files JSON response, filtering to MP3 files.
    ///
    /// Returns `ArchiveTrack` entries for each MP3 file in the item.
    pub fn parse_files_response(json: &str, item_id: &str, creator: &str) -> Vec<ArchiveTrack> {
        let parsed: serde_json::Value = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        let files = match parsed.get("result") {
            Some(serde_json::Value::Array(arr)) => arr,
            _ => return Vec::new(),
        };
        let mut tracks = Vec::new();
        for file in files {
            let name = file.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let format = file.get("format").and_then(|v| v.as_str()).unwrap_or("");
            // Only include MP3 files.
            if !format.contains("MP3") {
                continue;
            }
            let title_str = file.get("title").and_then(|v| v.as_str()).unwrap_or(name);
            tracks.push(ArchiveTrack {
                item_id: item_id.to_string(),
                filename: name.to_string(),
                title: title_str.to_string(),
                creator: creator.to_string(),
            });
        }
        tracks
    }

    /// Advance to the next track, wrapping around to the beginning.
    ///
    /// Returns `None` if the catalog is empty.
    pub fn next_track(&mut self) -> Option<&ArchiveTrack> {
        if self.tracks.is_empty() {
            return None;
        }
        self.current = (self.current + 1) % self.tracks.len();
        Some(&self.tracks[self.current])
    }

    /// Get the current track without advancing.
    pub fn current_track(&self) -> Option<&ArchiveTrack> {
        self.tracks.get(self.current)
    }

    /// Shuffle the track order using a simple Fisher-Yates with frame counter seed.
    pub fn shuffle(&mut self, seed: u64) {
        let len = self.tracks.len();
        if len <= 1 {
            return;
        }
        let mut rng = seed;
        for i in (1..len).rev() {
            // Simple LCG for deterministic shuffle.
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let j = (rng >> 33) as usize % (i + 1);
            self.tracks.swap(i, j);
        }
        self.current = 0;
    }

    /// Return the download path for a track: `/download/{item_id}/{filename}`.
    pub fn download_path(track: &ArchiveTrack) -> String {
        format!(
            "/download/{}/{}",
            track.item_id,
            percent_encode(&track.filename)
        )
    }

    /// Return the full download URL for a track.
    pub fn download_url(track: &ArchiveTrack) -> String {
        format!(
            "https://archive.org/download/{}/{}",
            track.item_id,
            percent_encode(&track.filename)
        )
    }

    /// Return the search API URL for a collection.
    pub fn search_url(collection: &str) -> String {
        format!(
            "https://archive.org/advancedsearch.php?\
             q=collection:{collection}+AND+mediatype:audio\
             &fl=identifier,title,creator&sort=random&rows=50&output=json"
        )
    }

    /// Return the files metadata API path for an item.
    pub fn files_api_path(item_id: &str) -> String {
        format!("/metadata/{item_id}/files")
    }
}

#[cfg(test)]
mod tests;
