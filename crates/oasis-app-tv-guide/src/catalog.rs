//! Internet Archive video catalog: data types and JSON parsing.
//!
//! Pure data transformation for IA metadata API responses — no network I/O.
//! Follows the same pattern as `oasis_audio::radio::archive::ArchiveCatalog`.

use std::fmt;

/// Recognized video format families from Internet Archive metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoFormat {
    /// MPEG-4 container (IA format string contains "mpeg4").
    Mpeg4,
    /// H.264 base (IA format string contains "h.264" but not "IA" suffix).
    H264,
    /// H.264 IA derivative (IA format string "h.264 IA" or similar).
    H264Ia,
    /// Generic MP4 (IA format string contains "mp4" but not "mpeg4"/"h.264").
    Mp4,
    /// Unrecognized / non-video format.
    Other(String),
}

impl VideoFormat {
    /// Parse an IA format string into a `VideoFormat` variant.
    pub fn parse(format: &str) -> Self {
        let lower = format.to_ascii_lowercase();
        if lower.contains("h.264") {
            if lower.contains("ia") {
                Self::H264Ia
            } else {
                Self::H264
            }
        } else if lower.contains("mpeg4") {
            Self::Mpeg4
        } else if lower.contains("mp4") {
            Self::Mp4
        } else {
            Self::Other(format.to_string())
        }
    }

    /// Whether this format represents a playable video file.
    pub fn is_video(&self) -> bool {
        !matches!(self, Self::Other(_))
    }

    /// Whether this is an H.264 format (either base or IA derivative).
    pub fn is_h264(&self) -> bool {
        matches!(self, Self::H264 | Self::H264Ia)
    }
}

impl fmt::Display for VideoFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mpeg4 => f.write_str("MPEG4"),
            Self::H264 => f.write_str("h.264"),
            Self::H264Ia => f.write_str("h.264 IA"),
            Self::Mp4 => f.write_str("mp4"),
            Self::Other(s) => f.write_str(s),
        }
    }
}

impl From<&str> for VideoFormat {
    fn from(s: &str) -> Self {
        Self::parse(s)
    }
}

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
    /// Parsed video format from IA metadata.
    pub format: VideoFormat,
    /// Original filename this file derives from (IA `original` key).
    /// Present on derivative files; `None` on originals.
    pub original: Option<String>,
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
            let format_str = file.get("format").and_then(|v| v.as_str()).unwrap_or("");
            let format = VideoFormat::parse(format_str);

            // Only include MP4/h.264 video files.
            if !format.is_video() {
                continue;
            }

            // Subfolder filter: only include files in the specified directory.
            // Use "sf/" prefix to avoid matching sibling dirs (e.g. "Season1" vs "Season10").
            if let Some(sf) = subfolder {
                let prefix = format!("{sf}/");
                if !name.starts_with(&prefix) {
                    continue;
                }
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

            let original = file
                .get("original")
                .and_then(|v| v.as_str())
                .map(String::from);

            episodes.push(VideoEpisode {
                item_id: item_id.to_string(),
                filename: name.to_string(),
                title,
                duration_secs: duration,
                width,
                height,
                size_bytes,
                format,
                original,
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

/// Select the smallest suitable video file for constrained playback.
///
/// Prefers IA "h.264" derivatives (always smaller/optimized), filters to files
/// under `max_bytes`, and among candidates prefers width >= `min_width` (for
/// watchability on small screens like PSP 480x272). Falls back to the smallest
/// file if nothing meets the threshold.
pub fn select_smallest_for(
    episodes: &[VideoEpisode],
    max_bytes: u64,
    min_width: u32,
) -> Option<&VideoEpisode> {
    if episodes.is_empty() {
        return None;
    }

    // Partition into h.264 derivatives and others.
    let is_h264_deriv = |ep: &&VideoEpisode| ep.format.is_h264() && ep.original.is_some();

    // First try: h.264 derivatives under max_bytes with acceptable width.
    let mut best: Option<&VideoEpisode> = episodes
        .iter()
        .filter(is_h264_deriv)
        .filter(|ep| ep.size_bytes <= max_bytes && ep.width >= min_width)
        .min_by_key(|ep| ep.size_bytes);

    // Second: h.264 derivatives under max_bytes (any width).
    if best.is_none() {
        best = episodes
            .iter()
            .filter(is_h264_deriv)
            .filter(|ep| ep.size_bytes <= max_bytes)
            .min_by_key(|ep| ep.size_bytes);
    }

    // Third: any file under max_bytes with acceptable width.
    if best.is_none() {
        best = episodes
            .iter()
            .filter(|ep| ep.size_bytes <= max_bytes && ep.width >= min_width)
            .min_by_key(|ep| ep.size_bytes);
    }

    // Fourth: any file under max_bytes.
    if best.is_none() {
        best = episodes
            .iter()
            .filter(|ep| ep.size_bytes <= max_bytes)
            .min_by_key(|ep| ep.size_bytes);
    }

    // Last resort: smallest file regardless of size.
    if best.is_none() {
        best = episodes.iter().min_by_key(|ep| ep.size_bytes);
    }

    best
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
        assert_eq!(episodes[0].format, VideoFormat::Mpeg4);
        assert!(episodes[0].original.is_none());
        assert_eq!(episodes[1].filename, "Videos/episode02.mp4");
        assert_eq!(episodes[1].format, VideoFormat::H264Ia);
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
                format: "MPEG4".into(),
                original: None,
            },
            VideoEpisode {
                item_id: "a".into(),
                filename: "ep2.mp4".into(),
                title: "Ep 2".into(),
                duration_secs: 200.0,
                width: 640,
                height: 480,
                size_bytes: 2000,
                format: "MPEG4".into(),
                original: None,
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
            format: "MPEG4".into(),
            original: None,
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
    fn video_format_parsing() {
        assert_eq!(VideoFormat::parse("MPEG4"), VideoFormat::Mpeg4);
        assert_eq!(VideoFormat::parse("h.264 IA"), VideoFormat::H264Ia);
        assert_eq!(VideoFormat::parse("h.264"), VideoFormat::H264);
        assert_eq!(VideoFormat::parse("mp4"), VideoFormat::Mp4);
        assert!(VideoFormat::parse("MPEG4").is_video());
        assert!(VideoFormat::parse("h.264 IA").is_video());
        assert!(VideoFormat::parse("h.264").is_video());
        assert!(VideoFormat::parse("mp4").is_video());
        assert!(!VideoFormat::parse("VBR MP3").is_video());
        assert!(!VideoFormat::parse("JPEG").is_video());
        assert!(!VideoFormat::parse("Metadata").is_video());
    }

    #[test]
    fn video_format_is_h264() {
        assert!(VideoFormat::H264.is_h264());
        assert!(VideoFormat::H264Ia.is_h264());
        assert!(!VideoFormat::Mpeg4.is_h264());
        assert!(!VideoFormat::Mp4.is_h264());
        assert!(!VideoFormat::Other("AVI".into()).is_h264());
    }

    #[test]
    fn video_format_display() {
        assert_eq!(VideoFormat::Mpeg4.to_string(), "MPEG4");
        assert_eq!(VideoFormat::H264.to_string(), "h.264");
        assert_eq!(VideoFormat::H264Ia.to_string(), "h.264 IA");
        assert_eq!(VideoFormat::Mp4.to_string(), "mp4");
        assert_eq!(VideoFormat::Other("AVI".into()).to_string(), "AVI");
    }

    #[test]
    fn parse_files_format_and_original() {
        let json = r#"{
            "result": [
                {
                    "name": "original.mp4",
                    "format": "MPEG4",
                    "length": "100",
                    "size": "50000000"
                },
                {
                    "name": "original.ia.mp4",
                    "format": "h.264 IA",
                    "original": "original.mp4",
                    "length": "100",
                    "size": "10000000"
                }
            ]
        }"#;
        let episodes = ChannelCatalog::parse_files_response(json, "item", None);
        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[0].format, VideoFormat::Mpeg4);
        assert!(episodes[0].original.is_none());
        assert_eq!(episodes[1].format, VideoFormat::H264Ia);
        assert_eq!(episodes[1].original.as_deref(), Some("original.mp4"));
    }

    fn make_ep(
        filename: &str,
        format: &str,
        original: Option<&str>,
        size: u64,
        width: u32,
    ) -> VideoEpisode {
        VideoEpisode {
            item_id: "test".into(),
            filename: filename.into(),
            title: filename.into(),
            duration_secs: 100.0,
            width,
            height: 240,
            size_bytes: size,
            format: format.into(),
            original: original.map(String::from),
        }
    }

    #[test]
    fn select_smallest_prefers_h264_derivative() {
        let eps = vec![
            make_ep("big.mp4", "MPEG4", None, 50_000_000, 640),
            make_ep("small.mp4", "h.264 IA", Some("big.mp4"), 10_000_000, 320),
        ];
        let best = select_smallest_for(&eps, 60_000_000, 320).unwrap();
        assert_eq!(best.filename, "small.mp4");
    }

    #[test]
    fn select_smallest_respects_max_bytes() {
        let eps = vec![
            make_ep("huge.mp4", "MPEG4", None, 100_000_000, 640),
            make_ep("medium.mp4", "h.264 IA", Some("huge.mp4"), 30_000_000, 320),
        ];
        // With 20MB cap, neither h264 fits under cap; falls back to smallest overall.
        let best = select_smallest_for(&eps, 20_000_000, 320).unwrap();
        assert_eq!(best.filename, "medium.mp4");
    }

    #[test]
    fn select_smallest_prefers_adequate_width() {
        let eps = vec![
            make_ep("tiny.mp4", "h.264 IA", Some("a.mp4"), 5_000_000, 160),
            make_ep("good.mp4", "h.264 IA", Some("a.mp4"), 8_000_000, 320),
        ];
        let best = select_smallest_for(&eps, 20_000_000, 320).unwrap();
        assert_eq!(best.filename, "good.mp4");
    }

    #[test]
    fn select_smallest_falls_back_to_narrow() {
        let eps = vec![make_ep(
            "tiny.mp4",
            "h.264 IA",
            Some("a.mp4"),
            5_000_000,
            160,
        )];
        // Only narrow file available; still selected.
        let best = select_smallest_for(&eps, 20_000_000, 320).unwrap();
        assert_eq!(best.filename, "tiny.mp4");
    }

    #[test]
    fn select_smallest_empty() {
        assert!(select_smallest_for(&[], 20_000_000, 320).is_none());
    }

    #[test]
    fn select_smallest_all_over_cap_returns_smallest() {
        let eps = vec![
            make_ep("a.mp4", "MPEG4", None, 50_000_000, 640),
            make_ep("b.mp4", "MPEG4", None, 30_000_000, 640),
        ];
        let best = select_smallest_for(&eps, 10_000_000, 320).unwrap();
        assert_eq!(best.filename, "b.mp4");
    }

    // -----------------------------------------------------------------------
    // PSP-specific video selection tests
    //
    // The PSP backend calls `select_smallest_for(&episodes, 20_000_000, 320)`
    // -- 20MB max, 320px minimum width for 480x272 screen. These tests
    // exercise the exact parameter combination used on PSP hardware.
    // -----------------------------------------------------------------------

    /// PSP constraint: 20MB max, 320px min width. h.264 derivatives preferred.
    #[test]
    fn select_smallest_psp_constraints_h264_preferred() {
        let eps = vec![
            make_ep("original.mp4", "MPEG4", None, 50_000_000, 640),
            make_ep(
                "derivative.mp4",
                "h.264 IA",
                Some("original.mp4"),
                15_000_000,
                320,
            ),
        ];
        let best = select_smallest_for(&eps, 20_000_000, 320).unwrap();
        assert_eq!(best.filename, "derivative.mp4");
    }

    /// PSP constraint: when only originals exist and all exceed 20MB,
    /// the function should still return the smallest as last resort.
    #[test]
    fn select_smallest_psp_constraints_all_over_20mb() {
        let eps = vec![
            make_ep("large.mp4", "MPEG4", None, 80_000_000, 640),
            make_ep("medium.mp4", "MPEG4", None, 40_000_000, 320),
        ];
        let best = select_smallest_for(&eps, 20_000_000, 320).unwrap();
        assert_eq!(best.filename, "medium.mp4");
    }

    /// PSP constraint: prefer 320px+ width even if a smaller file exists.
    #[test]
    fn select_smallest_psp_prefers_320px_width() {
        let eps = vec![
            make_ep("tiny.mp4", "h.264 IA", Some("a.mp4"), 5_000_000, 160),
            make_ep("good.mp4", "h.264 IA", Some("a.mp4"), 12_000_000, 320),
        ];
        let best = select_smallest_for(&eps, 20_000_000, 320).unwrap();
        assert_eq!(best.filename, "good.mp4");
    }

    /// PSP constraint: if no 320px+ file fits under 20MB, fall back to
    /// narrower h.264 derivative.
    #[test]
    fn select_smallest_psp_fallback_to_narrow_h264() {
        let eps = vec![
            make_ep("wide.mp4", "h.264 IA", Some("a.mp4"), 25_000_000, 640),
            make_ep("narrow.mp4", "h.264 IA", Some("a.mp4"), 8_000_000, 160),
        ];
        let best = select_smallest_for(&eps, 20_000_000, 320).unwrap();
        assert_eq!(best.filename, "narrow.mp4");
    }

    /// PSP constraint: with mixed format types, h.264 derivative under
    /// 20MB with adequate width should win over larger originals.
    #[test]
    fn select_smallest_psp_mixed_formats() {
        let eps = vec![
            make_ep("original_hd.mp4", "MPEG4", None, 100_000_000, 1920),
            make_ep(
                "derivative_sd.mp4",
                "h.264 IA",
                Some("original_hd.mp4"),
                18_000_000,
                640,
            ),
            make_ep(
                "derivative_lo.mp4",
                "h.264 IA",
                Some("original_hd.mp4"),
                6_000_000,
                320,
            ),
            make_ep("small_original.mp4", "MPEG4", None, 12_000_000, 320),
        ];
        let best = select_smallest_for(&eps, 20_000_000, 320).unwrap();
        // Should pick the smallest h.264 derivative with >= 320px.
        assert_eq!(best.filename, "derivative_lo.mp4");
    }

    /// PSP constraint: single episode just under 20MB.
    #[test]
    fn select_smallest_psp_single_episode_under_cap() {
        let eps = vec![make_ep(
            "episode.mp4",
            "h.264 IA",
            Some("orig.mp4"),
            19_999_999,
            480,
        )];
        let best = select_smallest_for(&eps, 20_000_000, 320).unwrap();
        assert_eq!(best.filename, "episode.mp4");
    }

    /// PSP constraint: single episode exactly at 20MB (edge case).
    #[test]
    fn select_smallest_psp_exactly_at_cap() {
        let eps = vec![make_ep(
            "exact.mp4",
            "h.264 IA",
            Some("orig.mp4"),
            20_000_000,
            320,
        )];
        let best = select_smallest_for(&eps, 20_000_000, 320).unwrap();
        assert_eq!(best.filename, "exact.mp4");
    }
}
