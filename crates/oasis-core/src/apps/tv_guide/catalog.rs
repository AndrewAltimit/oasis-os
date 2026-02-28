//! Internet Archive video catalog: data types and JSON parsing.
//!
//! Pure data transformation for IA metadata API responses — no network I/O.
//! Follows the same pattern as `oasis_audio::radio::archive::ArchiveCatalog`.

/// A single video episode from an Internet Archive item.
#[derive(Debug, Clone)]
pub struct VideoEpisode {
    /// IA item identifier.
    pub item_id: String,
    /// Video filename within the item.
    pub filename: String,
    /// Display title.
    pub title: String,
    /// Duration in seconds (from IA metadata `length` field).
    pub duration_secs: f64,
    /// Video width in pixels.
    pub width: u32,
    /// Video height in pixels.
    pub height: u32,
    /// File size in bytes.
    pub size_bytes: u64,
}

/// A channel's video library — all episodes available for scheduling.
#[derive(Debug, Clone)]
pub struct ChannelCatalog {
    /// Channel number (used as PRNG seed for deterministic shuffle).
    pub channel_number: u32,
    /// All discovered episodes.
    pub episodes: Vec<VideoEpisode>,
    /// Sum of all episode durations (cached for schedule math).
    pub total_duration_secs: f64,
}

impl ChannelCatalog {
    /// Create a new empty catalog for a channel.
    pub fn new(channel_number: u32) -> Self {
        Self {
            channel_number,
            episodes: Vec::new(),
            total_duration_secs: 0.0,
        }
    }

    /// Add episodes and recompute total duration.
    pub fn add_episodes(&mut self, episodes: Vec<VideoEpisode>) {
        for ep in episodes {
            self.total_duration_secs += ep.duration_secs;
            self.episodes.push(ep);
        }
    }

    /// Parse the IA `/metadata/{item_id}/files` JSON response for video files.
    ///
    /// Filters to MP4/h.264 files and extracts duration, dimensions, and size.
    /// If `subfolder` is provided, only includes files whose name starts with
    /// that prefix (followed by `/`).
    pub fn parse_files_response(
        json: &str,
        item_id: &str,
        subfolder: Option<&str>,
    ) -> Vec<VideoEpisode> {
        let parsed: serde_json::Value = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        let files = match parsed.get("result") {
            Some(serde_json::Value::Array(arr)) => arr,
            _ => return Vec::new(),
        };
        let mut episodes = Vec::new();
        for file in files {
            let name = file.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let format = file.get("format").and_then(|v| v.as_str()).unwrap_or("");

            // Only include MP4/h.264 video files.
            if !is_video_format(format) {
                continue;
            }

            // Subfolder filter: only include files in the specified directory.
            if let Some(sf) = subfolder
                && !name.starts_with(sf)
            {
                continue;
            }

            let duration = parse_duration(file);
            // Skip files without a usable duration.
            if duration <= 0.0 {
                continue;
            }

            let title = derive_title(name, subfolder);
            let width = file
                .get("width")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let height = file
                .get("height")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let size_bytes = file
                .get("size")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            episodes.push(VideoEpisode {
                item_id: item_id.to_string(),
                filename: name.to_string(),
                title,
                duration_secs: duration,
                width,
                height,
                size_bytes,
            });
        }
        episodes
    }

    /// Return the files metadata API path for an item.
    pub fn files_api_path(item_id: &str) -> String {
        format!("/metadata/{item_id}/files")
    }

    /// Return the full download URL for a video episode.
    pub fn download_url(episode: &VideoEpisode) -> String {
        format!(
            "https://archive.org/download/{}/{}",
            episode.item_id,
            oasis_audio::radio::archive::percent_encode(&episode.filename)
        )
    }

    /// Return the IA embed player URL for an item.
    pub fn embed_url(item_id: &str) -> String {
        format!("https://archive.org/embed/{item_id}")
    }

    /// Return the thumbnail URL for an item.
    pub fn thumbnail_url(item_id: &str) -> String {
        format!("https://archive.org/services/img/{item_id}")
    }
}

/// Check if a format string indicates a video file (MP4/h.264).
fn is_video_format(format: &str) -> bool {
    let f = format.to_ascii_lowercase();
    f.contains("mpeg4") || f.contains("h.264") || f.contains("mp4")
}

/// Parse the duration field from an IA file entry.
///
/// The `length` field can be a string like "1234.56" or a number.
fn parse_duration(file: &serde_json::Value) -> f64 {
    if let Some(v) = file.get("length") {
        if let Some(n) = v.as_f64() {
            return n;
        }
        if let Some(s) = v.as_str() {
            return s.parse().unwrap_or(0.0);
        }
    }
    0.0
}

/// Derive a display title from a filename, stripping path prefix and extension.
fn derive_title(filename: &str, subfolder: Option<&str>) -> String {
    let mut name = filename;
    // Strip subfolder prefix (e.g. "Subfolder/Episode.mp4" -> "Episode.mp4").
    if let Some(sf) = subfolder {
        name = name
            .strip_prefix(sf)
            .unwrap_or(name)
            .trim_start_matches('/');
    }
    // Strip last path segment's extension.
    let basename = name.rsplit('/').next().unwrap_or(name);
    basename
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(basename)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_files_json() -> &'static str {
        r#"{
            "result": [
                {
                    "name": "Videos/episode01.mp4",
                    "format": "MPEG4",
                    "length": "1234.5",
                    "width": "640",
                    "height": "480",
                    "size": "52428800"
                },
                {
                    "name": "cover.jpg",
                    "format": "JPEG"
                },
                {
                    "name": "Videos/episode02.mp4",
                    "format": "h.264 IA",
                    "length": "987.2",
                    "width": "1280",
                    "height": "720",
                    "size": "104857600"
                },
                {
                    "name": "Videos/bonus.avi",
                    "format": "AVI",
                    "length": "300"
                },
                {
                    "name": "metadata.xml",
                    "format": "Metadata"
                }
            ]
        }"#
    }

    #[test]
    fn parse_files_filters_video() {
        let episodes = ChannelCatalog::parse_files_response(sample_files_json(), "item-1", None);
        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[0].filename, "Videos/episode01.mp4");
        assert_eq!(episodes[0].title, "episode01");
        assert!((episodes[0].duration_secs - 1234.5).abs() < 0.01);
        assert_eq!(episodes[0].width, 640);
        assert_eq!(episodes[0].height, 480);
        assert_eq!(episodes[0].size_bytes, 52428800);
        assert_eq!(episodes[1].filename, "Videos/episode02.mp4");
    }

    #[test]
    fn parse_files_with_subfolder() {
        let episodes =
            ChannelCatalog::parse_files_response(sample_files_json(), "item-1", Some("Videos"));
        assert_eq!(episodes.len(), 2);
    }

    #[test]
    fn parse_files_subfolder_filters() {
        let json = r#"{
            "result": [
                {
                    "name": "Season1/ep1.mp4",
                    "format": "MPEG4",
                    "length": "100"
                },
                {
                    "name": "Season2/ep1.mp4",
                    "format": "MPEG4",
                    "length": "200"
                }
            ]
        }"#;
        let episodes = ChannelCatalog::parse_files_response(json, "item", Some("Season1"));
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].title, "ep1");
    }

    #[test]
    fn parse_files_empty() {
        let json = r#"{"result": []}"#;
        let episodes = ChannelCatalog::parse_files_response(json, "x", None);
        assert!(episodes.is_empty());
    }

    #[test]
    fn parse_files_invalid_json() {
        let episodes = ChannelCatalog::parse_files_response("not json", "x", None);
        assert!(episodes.is_empty());
    }

    #[test]
    fn parse_files_numeric_length() {
        let json = r#"{
            "result": [
                {
                    "name": "video.mp4",
                    "format": "MPEG4",
                    "length": 300.5
                }
            ]
        }"#;
        let episodes = ChannelCatalog::parse_files_response(json, "item", None);
        assert_eq!(episodes.len(), 1);
        assert!((episodes[0].duration_secs - 300.5).abs() < 0.01);
    }

    #[test]
    fn parse_files_skips_zero_duration() {
        let json = r#"{
            "result": [
                {
                    "name": "video.mp4",
                    "format": "MPEG4",
                    "length": "0"
                }
            ]
        }"#;
        let episodes = ChannelCatalog::parse_files_response(json, "item", None);
        assert!(episodes.is_empty());
    }

    #[test]
    fn add_episodes_updates_total() {
        let mut catalog = ChannelCatalog::new(1);
        assert_eq!(catalog.total_duration_secs, 0.0);
        catalog.add_episodes(vec![
            VideoEpisode {
                item_id: "a".into(),
                filename: "ep1.mp4".into(),
                title: "Ep 1".into(),
                duration_secs: 100.0,
                width: 640,
                height: 480,
                size_bytes: 1000,
            },
            VideoEpisode {
                item_id: "a".into(),
                filename: "ep2.mp4".into(),
                title: "Ep 2".into(),
                duration_secs: 200.0,
                width: 640,
                height: 480,
                size_bytes: 2000,
            },
        ]);
        assert_eq!(catalog.episodes.len(), 2);
        assert!((catalog.total_duration_secs - 300.0).abs() < 0.01);
    }

    #[test]
    fn download_url_format() {
        let ep = VideoEpisode {
            item_id: "my-item".into(),
            filename: "My Video #1.mp4".into(),
            title: "My Video #1".into(),
            duration_secs: 100.0,
            width: 640,
            height: 480,
            size_bytes: 1000,
        };
        let url = ChannelCatalog::download_url(&ep);
        assert!(url.starts_with("https://archive.org/download/my-item/"));
        assert!(url.contains("%23")); // # encoded
        assert!(!url.contains('#'));
    }

    #[test]
    fn embed_url_format() {
        assert_eq!(
            ChannelCatalog::embed_url("test-item"),
            "https://archive.org/embed/test-item"
        );
    }

    #[test]
    fn thumbnail_url_format() {
        assert_eq!(
            ChannelCatalog::thumbnail_url("test-item"),
            "https://archive.org/services/img/test-item"
        );
    }

    #[test]
    fn derive_title_strips_extension() {
        assert_eq!(derive_title("episode.mp4", None), "episode");
    }

    #[test]
    fn derive_title_strips_subfolder() {
        assert_eq!(
            derive_title("Season1/Episode 01.mp4", Some("Season1")),
            "Episode 01"
        );
    }

    #[test]
    fn derive_title_no_extension() {
        assert_eq!(derive_title("readme", None), "readme");
    }

    #[test]
    fn is_video_format_variants() {
        assert!(is_video_format("MPEG4"));
        assert!(is_video_format("h.264 IA"));
        assert!(is_video_format("h.264"));
        assert!(is_video_format("mp4"));
        assert!(!is_video_format("VBR MP3"));
        assert!(!is_video_format("JPEG"));
        assert!(!is_video_format("Metadata"));
    }
}
