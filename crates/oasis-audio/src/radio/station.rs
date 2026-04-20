//! Station data model and registry.
//!
//! Stations are internet radio endpoints with metadata (name, URL, genre,
//! format, bitrate). The registry is serialized as TOML for VFS storage.

use serde::{Deserialize, Serialize};

/// A single internet radio station.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Station {
    /// Display name.
    pub name: String,
    /// Stream URL (HTTP). Empty for archive-type stations.
    #[serde(default)]
    pub url: String,
    /// Genre tag (e.g. "ambient", "electronic").
    pub genre: String,
    /// Audio format ("mp3" or "aac").
    #[serde(default = "default_format")]
    pub format: String,
    /// Bitrate in kbps (0 if unknown).
    #[serde(default)]
    pub bitrate: u32,
    /// Whether this station is a user favorite.
    #[serde(default)]
    pub favorite: bool,
    /// Source type: "archive" (Internet Archive) or "icecast" (Icecast/Shoutcast).
    #[serde(default = "default_source_type")]
    pub source_type: String,
    /// Internet Archive collection identifier (only for archive-type stations).
    #[serde(default)]
    pub collection: String,
}

fn default_source_type() -> String {
    "archive".into()
}

fn default_format() -> String {
    "mp3".into()
}

/// Collection of stations, serializable as TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationRegistry {
    #[serde(rename = "station")]
    pub stations: Vec<Station>,
}

impl StationRegistry {
    /// Parse a registry from TOML text.
    pub fn from_toml(toml_str: &str) -> Result<Self, String> {
        toml::from_str(toml_str).map_err(|e| format!("invalid stations.toml: {e}"))
    }

    /// Serialize the registry to TOML text.
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| format!("serialize error: {e}"))
    }

    /// Return the default curated station list (Internet Archive collections).
    pub fn defaults() -> Self {
        Self {
            stations: vec![
                Station {
                    name: "Old Time Radio".into(),
                    url: String::new(),
                    genre: "drama".into(),
                    format: "mp3".into(),
                    bitrate: 0,
                    favorite: true,
                    source_type: "archive".into(),
                    collection: "oldtimeradio".into(),
                },
                Station {
                    name: "LibriVox Audiobooks".into(),
                    url: String::new(),
                    genre: "audiobooks".into(),
                    format: "mp3".into(),
                    bitrate: 0,
                    favorite: false,
                    source_type: "archive".into(),
                    collection: "librivoxaudio".into(),
                },
                Station {
                    name: "Netlabel Music".into(),
                    url: String::new(),
                    genre: "music".into(),
                    format: "mp3".into(),
                    bitrate: 0,
                    favorite: true,
                    source_type: "archive".into(),
                    collection: "netlabels".into(),
                },
                Station {
                    name: "78rpm Records".into(),
                    url: String::new(),
                    genre: "vintage".into(),
                    format: "mp3".into(),
                    bitrate: 0,
                    favorite: false,
                    source_type: "archive".into(),
                    collection: "78rpm".into(),
                },
                Station {
                    name: "This Is Your FBI".into(),
                    url: String::new(),
                    genre: "true crime".into(),
                    format: "mp3".into(),
                    bitrate: 0,
                    favorite: false,
                    source_type: "archive".into(),
                    collection: "OTRR_This_Is_Your_FBI_Singles".into(),
                },
            ],
        }
    }

    /// Return all unique genres, sorted.
    pub fn genres(&self) -> Vec<String> {
        let mut genres: Vec<String> = self.stations.iter().map(|s| s.genre.clone()).collect();
        genres.sort();
        genres.dedup();
        genres
    }

    /// Return stations matching a genre.
    pub fn by_genre(&self, genre: &str) -> Vec<&Station> {
        self.stations
            .iter()
            .filter(|s| s.genre.eq_ignore_ascii_case(genre))
            .collect()
    }

    /// Return only favorite stations.
    pub fn favorites(&self) -> Vec<&Station> {
        self.stations.iter().filter(|s| s.favorite).collect()
    }

    /// Toggle the favorite flag on a station by index.
    pub fn toggle_favorite(&mut self, index: usize) -> bool {
        if let Some(station) = self.stations.get_mut(index) {
            station.favorite = !station.favorite;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_has_stations() {
        let reg = StationRegistry::defaults();
        assert!(!reg.stations.is_empty());
        assert!(reg.stations.len() >= 4);
    }

    #[test]
    fn toml_round_trip() {
        let reg = StationRegistry::defaults();
        let toml_str = reg.to_toml().unwrap();
        let parsed = StationRegistry::from_toml(&toml_str).unwrap();
        assert_eq!(parsed.stations.len(), reg.stations.len());
        assert_eq!(parsed.stations[0].name, reg.stations[0].name);
    }

    #[test]
    fn genres_returns_sorted_unique() {
        let reg = StationRegistry::defaults();
        let genres = reg.genres();
        assert!(!genres.is_empty());
        // Check sorted.
        let mut sorted = genres.clone();
        sorted.sort();
        assert_eq!(genres, sorted);
        // Check unique.
        let mut deduped = genres.clone();
        deduped.dedup();
        assert_eq!(genres, deduped);
    }

    #[test]
    fn by_genre_filters() {
        let reg = StationRegistry::defaults();
        let drama = reg.by_genre("drama");
        assert!(!drama.is_empty());
        for s in &drama {
            assert_eq!(s.genre, "drama");
        }
    }

    #[test]
    fn by_genre_case_insensitive() {
        let reg = StationRegistry::defaults();
        let upper = reg.by_genre("DRAMA");
        let lower = reg.by_genre("drama");
        assert_eq!(upper.len(), lower.len());
    }

    #[test]
    fn favorites_filters() {
        let reg = StationRegistry::defaults();
        let favs = reg.favorites();
        for s in &favs {
            assert!(s.favorite);
        }
    }

    #[test]
    fn toggle_favorite() {
        let mut reg = StationRegistry::defaults();
        let was_fav = reg.stations[0].favorite;
        assert!(reg.toggle_favorite(0));
        assert_eq!(reg.stations[0].favorite, !was_fav);
        assert!(reg.toggle_favorite(0));
        assert_eq!(reg.stations[0].favorite, was_fav);
    }

    #[test]
    fn toggle_favorite_out_of_bounds() {
        let mut reg = StationRegistry::defaults();
        assert!(!reg.toggle_favorite(999));
    }

    #[test]
    fn from_toml_invalid() {
        assert!(StationRegistry::from_toml("not valid toml {{{").is_err());
    }

    #[test]
    fn from_toml_custom() {
        let toml_str = r#"
[[station]]
name = "Test Radio"
url = "http://example.com/stream"
genre = "test"
format = "mp3"
bitrate = 64
"#;
        let reg = StationRegistry::from_toml(toml_str).unwrap();
        assert_eq!(reg.stations.len(), 1);
        assert_eq!(reg.stations[0].name, "Test Radio");
        assert!(!reg.stations[0].favorite);
        // Backward compat: missing source_type defaults to "archive".
        assert_eq!(reg.stations[0].source_type, "archive");
    }

    #[test]
    fn from_toml_icecast_station() {
        let toml_str = r#"
[[station]]
name = "Custom Icecast"
url = "http://example.com/stream"
genre = "electronic"
format = "mp3"
bitrate = 128
source_type = "icecast"
"#;
        let reg = StationRegistry::from_toml(toml_str).unwrap();
        assert_eq!(reg.stations[0].source_type, "icecast");
        assert!(reg.stations[0].collection.is_empty());
    }

    #[test]
    fn from_toml_archive_station() {
        let toml_str = r#"
[[station]]
name = "Old Time Radio"
genre = "drama"
source_type = "archive"
collection = "oldtimeradio"
"#;
        let reg = StationRegistry::from_toml(toml_str).unwrap();
        assert_eq!(reg.stations[0].source_type, "archive");
        assert_eq!(reg.stations[0].collection, "oldtimeradio");
        assert!(reg.stations[0].url.is_empty());
        assert_eq!(reg.stations[0].format, "mp3");
    }

    #[test]
    fn defaults_are_archive_type() {
        let reg = StationRegistry::defaults();
        for s in &reg.stations {
            assert_eq!(s.source_type, "archive");
            assert!(!s.collection.is_empty());
        }
    }
}
