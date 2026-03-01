//! Mock catalog data for offline testing and CI.
//!
//! Provides deterministic test fixtures so TV Guide tests don't need network
//! access. Gated behind `#[cfg(any(test, feature = "test-data"))]`.

use super::catalog::{ChannelCatalog, VideoEpisode};
use super::channel::Channel;

/// Create a mock channel catalog with synthetic episodes.
///
/// Each episode has a predictable title ("Episode 1", "Episode 2", ...)
/// and a duration of 1800s (30 minutes) by default.
pub fn mock_channel_catalog(channel_num: u32, episode_count: usize) -> ChannelCatalog {
    let mut catalog = ChannelCatalog::new(channel_num);
    let episodes: Vec<VideoEpisode> = (0..episode_count)
        .map(|i| VideoEpisode {
            item_id: format!("mock-item-{channel_num}-{i}"),
            filename: format!("episode_{i:02}.mp4"),
            title: format!("Episode {}", i + 1),
            duration_secs: 1800.0,
            width: 640,
            height: 480,
            size_bytes: 50_000_000,
            format: "MPEG4".into(),
            original: None,
        })
        .collect();
    catalog.add_episodes(episodes);
    catalog
}

/// Create mock catalogs for a set of channels.
///
/// Returns one `Option<ChannelCatalog>` per channel with 5 episodes each.
pub fn mock_all_catalogs(channels: &[Channel]) -> Vec<Option<ChannelCatalog>> {
    channels
        .iter()
        .map(|ch| Some(mock_channel_catalog(ch.number, 5)))
        .collect()
}

/// Realistic Internet Archive `/metadata/{item}/files` JSON response
/// for use in parser tests.
pub const SAMPLE_IA_FILES_JSON: &str = r#"{
    "result": [
        {
            "name": "Season1/Pilot.mp4",
            "format": "MPEG4",
            "length": "1423.7",
            "width": "640",
            "height": "480",
            "size": "73400320"
        },
        {
            "name": "Season1/Episode_02_The_Return.mp4",
            "format": "h.264 IA",
            "length": "1312.4",
            "width": "1280",
            "height": "720",
            "size": "104857600"
        },
        {
            "name": "Season1/Episode_03_Final.mp4",
            "format": "MPEG4",
            "length": "1567.9",
            "width": "640",
            "height": "480",
            "size": "89128960"
        },
        {
            "name": "cover.jpg",
            "format": "JPEG",
            "size": "51200"
        },
        {
            "name": "metadata.xml",
            "format": "Metadata",
            "size": "2048"
        },
        {
            "name": "Season1/bonus_behind_scenes.avi",
            "format": "AVI",
            "length": "600",
            "size": "30000000"
        }
    ]
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_catalog_episode_count() {
        let catalog = mock_channel_catalog(2, 3);
        assert_eq!(catalog.episodes.len(), 3);
        assert_eq!(catalog.channel_number, 2);
        assert!((catalog.total_duration_secs - 5400.0).abs() < 0.01);
    }

    #[test]
    fn mock_catalog_titles_sequential() {
        let catalog = mock_channel_catalog(1, 4);
        assert_eq!(catalog.episodes[0].title, "Episode 1");
        assert_eq!(catalog.episodes[3].title, "Episode 4");
    }

    #[test]
    fn mock_all_catalogs_matches_channels() {
        let channels = vec![
            Channel {
                number: 2,
                call_sign: "TEST".to_string(),
                name: "Test".to_string(),
                genre: "test".to_string(),
                location: None,
                source: vec![],
            },
            Channel {
                number: 5,
                call_sign: "TST2".to_string(),
                name: "Test 2".to_string(),
                genre: "test".to_string(),
                location: None,
                source: vec![],
            },
        ];
        let catalogs = mock_all_catalogs(&channels);
        assert_eq!(catalogs.len(), 2);
        assert!(catalogs[0].is_some());
        assert!(catalogs[1].is_some());
        assert_eq!(catalogs[0].as_ref().unwrap().channel_number, 2);
        assert_eq!(catalogs[1].as_ref().unwrap().channel_number, 5);
    }

    #[test]
    fn sample_json_parses_videos_only() {
        let episodes =
            ChannelCatalog::parse_files_response(SAMPLE_IA_FILES_JSON, "test-item", None);
        // Should only include the 3 MPEG4/h.264 files, not JPEG/Metadata/AVI.
        assert_eq!(episodes.len(), 3);
    }

    #[test]
    fn sample_json_with_subfolder() {
        let episodes = ChannelCatalog::parse_files_response(
            SAMPLE_IA_FILES_JSON,
            "test-item",
            Some("Season1"),
        );
        assert_eq!(episodes.len(), 3);
        assert_eq!(episodes[0].title, "Pilot");
    }
}
