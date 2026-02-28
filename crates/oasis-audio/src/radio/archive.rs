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
/// `&`, `%`, `+`, and bytes >127.
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
            0x80.. => {
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
            if !identifier.is_empty() {
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
mod tests {
    use super::*;

    fn sample_search_json() -> &'static str {
        r#"{
            "response": {
                "numFound": 2,
                "docs": [
                    {
                        "identifier": "item-001",
                        "title": "Old Radio Show",
                        "creator": "Radio Corp"
                    },
                    {
                        "identifier": "item-002",
                        "title": "Jazz Hour",
                        "creator": "Jazz Inc"
                    }
                ]
            }
        }"#
    }

    fn sample_files_json() -> &'static str {
        r#"{
            "result": [
                {
                    "name": "show01.mp3",
                    "format": "VBR MP3",
                    "title": "Episode 1"
                },
                {
                    "name": "cover.jpg",
                    "format": "JPEG"
                },
                {
                    "name": "show02.mp3",
                    "format": "128Kbps MP3",
                    "title": "Episode 2"
                }
            ]
        }"#
    }

    #[test]
    fn parse_search_response_valid() {
        let results = ArchiveCatalog::parse_search_response(sample_search_json());
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "item-001");
        assert_eq!(results[0].1, "Old Radio Show");
        assert_eq!(results[0].2, "Radio Corp");
        assert_eq!(results[1].0, "item-002");
    }

    #[test]
    fn parse_search_response_empty() {
        let json = r#"{"response": {"numFound": 0, "docs": []}}"#;
        let results = ArchiveCatalog::parse_search_response(json);
        assert!(results.is_empty());
    }

    #[test]
    fn parse_search_response_invalid_json() {
        let results = ArchiveCatalog::parse_search_response("not json");
        assert!(results.is_empty());
    }

    #[test]
    fn parse_search_response_missing_fields() {
        let json = r#"{"response": {"docs": [{"identifier": "x"}]}}"#;
        let results = ArchiveCatalog::parse_search_response(json);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, ""); // title missing
        assert_eq!(results[0].2, "Unknown"); // creator missing
    }

    #[test]
    fn parse_files_response_filters_mp3() {
        let tracks =
            ArchiveCatalog::parse_files_response(sample_files_json(), "item-001", "Radio Corp");
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].filename, "show01.mp3");
        assert_eq!(tracks[0].title, "Episode 1");
        assert_eq!(tracks[0].creator, "Radio Corp");
        assert_eq!(tracks[1].filename, "show02.mp3");
    }

    #[test]
    fn parse_files_response_empty() {
        let json = r#"{"result": []}"#;
        let tracks = ArchiveCatalog::parse_files_response(json, "x", "y");
        assert!(tracks.is_empty());
    }

    #[test]
    fn parse_files_response_invalid_json() {
        let tracks = ArchiveCatalog::parse_files_response("bad", "x", "y");
        assert!(tracks.is_empty());
    }

    #[test]
    fn next_track_wraps_around() {
        let mut catalog = ArchiveCatalog::new("test");
        catalog.tracks = vec![
            ArchiveTrack {
                item_id: "a".into(),
                filename: "1.mp3".into(),
                title: "T1".into(),
                creator: "C".into(),
            },
            ArchiveTrack {
                item_id: "a".into(),
                filename: "2.mp3".into(),
                title: "T2".into(),
                creator: "C".into(),
            },
        ];
        catalog.current = 0;

        let t = catalog.next_track().unwrap();
        assert_eq!(t.title, "T2");
        assert_eq!(catalog.current, 1);

        let t = catalog.next_track().unwrap();
        assert_eq!(t.title, "T1");
        assert_eq!(catalog.current, 0);
    }

    #[test]
    fn next_track_empty_catalog() {
        let mut catalog = ArchiveCatalog::new("test");
        assert!(catalog.next_track().is_none());
    }

    #[test]
    fn current_track_returns_correct() {
        let mut catalog = ArchiveCatalog::new("test");
        assert!(catalog.current_track().is_none());

        catalog.tracks.push(ArchiveTrack {
            item_id: "a".into(),
            filename: "1.mp3".into(),
            title: "T1".into(),
            creator: "C".into(),
        });
        assert_eq!(catalog.current_track().unwrap().title, "T1");
    }

    #[test]
    fn shuffle_changes_order() {
        let mut catalog = ArchiveCatalog::new("test");
        for i in 0..20 {
            catalog.tracks.push(ArchiveTrack {
                item_id: format!("item-{i}"),
                filename: format!("{i}.mp3"),
                title: format!("Track {i}"),
                creator: "C".into(),
            });
        }
        let original: Vec<String> = catalog.tracks.iter().map(|t| t.title.clone()).collect();
        catalog.shuffle(42);
        let shuffled: Vec<String> = catalog.tracks.iter().map(|t| t.title.clone()).collect();
        // Very unlikely to be identical with 20 items.
        assert_ne!(original, shuffled);
        assert_eq!(catalog.current, 0);
    }

    #[test]
    fn shuffle_single_item() {
        let mut catalog = ArchiveCatalog::new("test");
        catalog.tracks.push(ArchiveTrack {
            item_id: "a".into(),
            filename: "1.mp3".into(),
            title: "T1".into(),
            creator: "C".into(),
        });
        catalog.shuffle(0);
        assert_eq!(catalog.tracks.len(), 1);
    }

    #[test]
    fn download_path_format() {
        let track = ArchiveTrack {
            item_id: "item-123".into(),
            filename: "audio.mp3".into(),
            title: "T".into(),
            creator: "C".into(),
        };
        assert_eq!(
            ArchiveCatalog::download_path(&track),
            "/download/item-123/audio.mp3"
        );
    }

    #[test]
    fn download_url_format() {
        let track = ArchiveTrack {
            item_id: "item-123".into(),
            filename: "audio.mp3".into(),
            title: "T".into(),
            creator: "C".into(),
        };
        assert_eq!(
            ArchiveCatalog::download_url(&track),
            "https://archive.org/download/item-123/audio.mp3"
        );
    }

    #[test]
    fn search_url_format() {
        let url = ArchiveCatalog::search_url("oldtimeradio");
        assert!(url.contains("collection:oldtimeradio"));
        assert!(url.contains("mediatype:audio"));
        assert!(url.contains("output=json"));
    }

    #[test]
    fn files_api_path_format() {
        assert_eq!(
            ArchiveCatalog::files_api_path("item-123"),
            "/metadata/item-123/files"
        );
    }

    #[test]
    fn percent_encode_spaces() {
        assert_eq!(percent_encode("hello world.mp3"), "hello%20world.mp3");
    }

    #[test]
    fn percent_encode_special_chars() {
        assert_eq!(percent_encode("file#1.mp3"), "file%231.mp3");
        assert_eq!(percent_encode("a?b&c+d%e"), "a%3Fb%26c%2Bd%25e");
    }

    #[test]
    fn percent_encode_safe_string() {
        assert_eq!(percent_encode("simple.mp3"), "simple.mp3");
        assert_eq!(percent_encode("a-b_c.mp3"), "a-b_c.mp3");
    }

    #[test]
    fn percent_encode_unicode() {
        let encoded = percent_encode("caf\u{00e9}.mp3");
        assert!(
            encoded.contains("%"),
            "expected percent-encoded output: {encoded}"
        );
        assert!(!encoded.contains('\u{00e9}'));
    }

    #[test]
    fn download_path_encodes_filename() {
        let track = ArchiveTrack {
            item_id: "item-123".into(),
            filename: "my song #1.mp3".into(),
            title: "T".into(),
            creator: "C".into(),
        };
        let path = ArchiveCatalog::download_path(&track);
        assert_eq!(path, "/download/item-123/my%20song%20%231.mp3");
    }

    #[test]
    fn download_url_encodes_filename() {
        let track = ArchiveTrack {
            item_id: "item-123".into(),
            filename: "track one.mp3".into(),
            title: "T".into(),
            creator: "C".into(),
        };
        let url = ArchiveCatalog::download_url(&track);
        assert_eq!(url, "https://archive.org/download/item-123/track%20one.mp3");
    }
}
