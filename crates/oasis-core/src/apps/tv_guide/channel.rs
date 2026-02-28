//! TV channel configuration with TOML serialization.

use serde::{Deserialize, Serialize};

/// A single Internet Archive source for a channel's video library.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelSource {
    /// IA item identifier.
    pub item_id: String,
    /// Optional subfolder within the item to filter files.
    #[serde(default)]
    pub subfolder: Option<String>,
    /// Media type (always "video" for TV guide).
    #[serde(default = "default_media_type")]
    pub media_type: String,
}

fn default_media_type() -> String {
    "video".to_string()
}

/// A TV channel definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Channel {
    /// Channel number (displayed in the grid).
    pub number: u32,
    /// Short call sign (e.g. "RETRO", "TECH").
    pub call_sign: String,
    /// Human-readable channel name.
    pub name: String,
    /// Genre tag.
    pub genre: String,
    /// One or more IA item sources for this channel's content.
    pub source: Vec<ChannelSource>,
}

/// Top-level config holding all channels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelConfig {
    pub channel: Vec<Channel>,
}

impl ChannelConfig {
    /// Parse channel config from TOML text.
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// Serialize config to TOML text.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}

/// Default 5-channel configuration.
pub const DEFAULT_CHANNELS_TOML: &str = r#"[[channel]]
number = 2
call_sign = "RETRO"
name = "Retro Cartoons"
genre = "cartoons"

[[channel.source]]
item_id = "adventures-of-sonic-the-hedgehog-01-x-44-the-mystery-of-the-missing-hi-tops_202402"
subfolder = "AOSTH Episodes (+ Special and Pilot)"
media_type = "video"

[[channel]]
number = 5
call_sign = "TECH"
name = "Tech & Bytes"
genre = "technology"

[[channel.source]]
item_id = "bits-and-bytes-yt"
subfolder = "Bits-and-Bytes"
media_type = "video"

[[channel]]
number = 8
call_sign = "GAME"
name = "Gaming"
genre = "gaming"

[[channel.source]]
item_id = "disney-bootlegs-jon-tron"
media_type = "video"

[[channel]]
number = 11
call_sign = "WILD"
name = "Game Shows"
genre = "game_shows"

[[channel.source]]
item_id = "003-1986-05-16"
media_type = "video"

[[channel]]
number = 13
call_sign = "DOCS"
name = "Documentaries"
genre = "documentary"

[[channel.source]]
item_id = "youtube-mTtMCoJrGxk"
media_type = "video"

[[channel.source]]
item_id = "youtube-goO-cqm0yho"
media_type = "video"

[[channel.source]]
item_id = "youtube-QVqvv-BbhmU"
media_type = "video"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_channels() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        assert_eq!(config.channel.len(), 5);
        assert_eq!(config.channel[0].number, 2);
        assert_eq!(config.channel[0].call_sign, "RETRO");
        assert_eq!(config.channel[0].source.len(), 1);
        assert_eq!(config.channel[4].number, 13);
        assert_eq!(config.channel[4].source.len(), 3);
    }

    #[test]
    fn roundtrip_toml() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let serialized = config.to_toml().unwrap();
        let reparsed = ChannelConfig::from_toml(&serialized).unwrap();
        assert_eq!(config, reparsed);
    }

    #[test]
    fn channel_source_defaults() {
        let toml = r#"
            [[channel]]
            number = 1
            call_sign = "TEST"
            name = "Test Channel"
            genre = "test"
            [[channel.source]]
            item_id = "test-item"
        "#;
        let config: ChannelConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.channel[0].source[0].media_type, "video");
        assert!(config.channel[0].source[0].subfolder.is_none());
    }

    #[test]
    fn empty_channels() {
        let toml = "channel = []";
        let config: ChannelConfig = toml::from_str(toml).unwrap();
        assert!(config.channel.is_empty());
    }
}
